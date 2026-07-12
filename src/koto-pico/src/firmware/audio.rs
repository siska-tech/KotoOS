//! PicoCalc audio backend on the KotoAudio runtime (KOTO-0146 / KOTO-0165).
//!
//! CPU0 owns the host-call boundary and enqueues bounded audio work. CPU1 owns
//! the ported `koto-audio` [`DefaultAudioService`]: it drains commands, renders
//! fixed mixer blocks through the service, and mixes in the raw-PCM stream.
//! Sample pacing is hardware-owned: a DMA channel paced by DMA timer 0 streams
//! duty words into the PWM compare register at exactly 16 kHz (the KOTO-0114
//! `probe_audio` pattern), so neither VM/render frame timing nor a service
//! render burst can disturb sample output.
//!
//! # KOTO-0165: the legacy audio implementation is gone
//!
//! The former in-tree engine — the runtime KotoMML parser, the `.kwt` wavetable
//! loader, the hand-rolled square/pulse/triangle/saw/noise synth, the mirrored
//! `PicoBgmScore` tables, and the tone-fallback path — was removed. Every
//! sequence cue now plays through the `koto-audio` crate itself (the same
//! runtime the SIM bridge uses), fed from the compiled cue tables in
//! [`super::audio_cues`] / [`super::audio_cues_generated`]. Only two inputs
//! reach this module:
//!
//! * **Sequence cues**: `&'static` [`PolyphonicSequence`] (looping BGM) and
//!   [`Sequence`] (one-shot SFX) references resolved by
//!   [`super::audio_cues::primary_audio_route`].
//! * **Raw PCM**: the bounded `audio_submit_i16` stream, mixed additively with
//!   the service output exactly like the SIM folds the bridge into `SimAudio`.
//!
//! # Memory
//!
//! The service (~3 KiB) lives in its own `StaticCell` (CPU1-only after init),
//! the shared rings live in a `critical_section` cell sized by the asserts
//! below, and the DMA duty ring adds 1 KiB of aligned `.bss`. The removed
//! legacy state (two by-value `PicoBgmScore` slots and the 4.5 KiB
//! `AudioAssetScratch` SD buffers) covers most of that; the measured net cost
//! of the port is ~6 KiB of `.bss`, dominated by the 8 KiB CPU1 stack
//! (KOTO-0148 headroom notes updated in the issue doc).

use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use critical_section::Mutex;
use embassy_rp::{
    clocks::clk_sys_freq,
    multicore::{spawn_core1, Stack},
    pac,
    pac::dma::vals::{DataSize, TreqSel},
    peripherals::CORE1,
    pwm::{Config as PwmConfig, Pwm, SetDutyCycle},
    Peri,
};
use embassy_time::{block_for, Duration, Instant};
use koto_audio::{
    AudioBackend, AudioPolicy, BackendError, BackendReport, BackendResult, BackendState,
    DefaultAudioService, MixerBlock, PolyphonicSequence, Sequence, DEFAULT_MIXER_BLOCK_FRAMES,
};
use portable_atomic::AtomicU32;
use static_cell::StaticCell;

/// Output sample rate. Matches `AudioLimits::v0_default()` and the sample rate
/// the koto-audio built-in drum tables are authored at, so drums play at pitch.
const PCM_OUTPUT_SAMPLE_RATE_HZ: u32 = 16_000;
/// Fixed service mixer block length (frames per `tick()`).
const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES;
/// Raw `audio_submit_i16` ring capacity (128 ms at 16 kHz).
const PCM_RING_CAPACITY: usize = 2048;
/// Rendered service output ring: three mixer blocks (~24 ms) of read-ahead.
const OUT_RING_CAPACITY: usize = BLOCK_FRAMES * 3;
const AUDIO_COMMAND_CAPACITY: usize = 16;
/// CPU1 worker stack. The koto-audio mixer keeps ~1.3 KiB of locals in one
/// `tick()` frame (`[i64; 128]` accumulator + output block), on top of the
/// worker/service call chain; the old 4 KiB budget was sized for the removed
/// ≤2 KiB legacy synth and left no headroom for that.
///
/// KOTO-0186: under fat LTO (KOTO-0176) the service was inlined into the worker
/// and `apply_command`'s `StopAll` arm rebuilt `SourceLifecycle`/`Mixer` **by
/// value**, materializing a ~5 KiB temporary that pushed the `run` → `StopAll`
/// chain to ~8.9 KiB and overflowed this 8 KiB stack into `APP_STATIC_SHADOW`.
/// The durable fix made that reset construct in place (koto-audio
/// `service.rs::reset`), so the deepest arm no longer carries a whole-struct
/// temporary and 8 KiB holds. The `core1_stack_free_min` canary on the
/// `phase=173` line now measures the real remaining margin instead of leaving
/// it to guesswork; re-size here only against that measured low-water mark.
pub const AUDIO_CORE1_STACK_BYTES: usize = 8192;
const PCM_PWM_DIVIDER: u8 = 6;
const PCM_PWM_TOP: u16 = 250;
const IDLE_SILENCE_DUTY: u16 = 0;

// --- DMA-paced output (KOTO-0114 pattern) -----------------------------------
//
// DMA timer 0 requests one word per output sample; the channel streams duty
// words from the aligned ring into the PWM slice-5 compare register with
// hardware address wrapping. LCD SPI owns embassy `DMA_CH0` and the koto-psram
// fast read owns `DMA_CH1`/PAC 0-1, so audio takes the top PAC channel.
const AUDIO_DMA_CH: usize = 11;
/// One duty word per sample: 256 samples = 16 ms of hardware-owned lead.
const DMA_RING_SAMPLES: usize = 256;
/// 256 words * 4 bytes = 1 KiB naturally aligned ring for DMA read wrapping.
const DMA_RING_BYTES_LOG2: u8 = 10;
/// Transfers per arm period (~3 days at 16 kHz before a re-arm). A multiple of
/// the ring length so the base-restarted read address stays aligned with the
/// writer's modulo position across re-arms.
const DMA_ARM_COUNT: u32 = u32::MAX & !(DMA_RING_SAMPLES as u32 - 1);
/// Worker fill-pass cadence. Each pass tops the ring back up to a full ring of
/// lead, so the pass budget only needs to stay well under the 16 ms lead.
const WORKER_PASS_PERIOD_US: u64 = 1_000;
/// A pass longer than half the ring lead is flagged in `worker_late`.
const WORKER_LATE_PASS_US: u64 = 8_000;

/// The DMA-read duty-word ring. Written only by the CPU1 worker, read only by
/// the DMA engine; `repr(align)` satisfies the hardware ring-wrap alignment.
#[repr(C, align(1024))]
struct AlignedDmaRing([u32; DMA_RING_SAMPLES]);

static mut AUDIO_DMA_RING: AlignedDmaRing = AlignedDmaRing([0; DMA_RING_SAMPLES]);

struct PcmRing<const N: usize> {
    samples: [i16; N],
    read_idx: usize,
    write_idx: usize,
    len: usize,
}

impl<const N: usize> PcmRing<N> {
    const fn new() -> Self {
        Self {
            samples: [0; N],
            read_idx: 0,
            write_idx: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.read_idx = 0;
        self.write_idx = 0;
        self.len = 0;
    }

    fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, sample: i16) -> bool {
        if self.len >= N {
            return false;
        }
        self.samples[self.write_idx] = sample;
        self.write_idx = (self.write_idx + 1) % N;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<i16> {
        if self.len == 0 {
            return None;
        }
        let sample = self.samples[self.read_idx];
        self.read_idx = (self.read_idx + 1) % N;
        self.len -= 1;
        Some(sample)
    }
}

/// Bounded audio work CPU0 hands to the CPU1 worker. Sequence commands carry
/// `&'static` references into the compiled cue tables, so enqueueing never
/// copies event data.
#[derive(Clone, Copy)]
enum AudioCommand {
    None,
    PlayBgm(&'static PolyphonicSequence<'static>),
    PlaySfx(&'static Sequence<'static>),
    StopBgm,
    StopAll,
}

struct AudioCommandQueue {
    commands: [AudioCommand; AUDIO_COMMAND_CAPACITY],
    read_idx: usize,
    write_idx: usize,
    len: usize,
}

impl AudioCommandQueue {
    const fn new() -> Self {
        Self {
            commands: [AudioCommand::None; AUDIO_COMMAND_CAPACITY],
            read_idx: 0,
            write_idx: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.read_idx = 0;
        self.write_idx = 0;
        self.len = 0;
    }

    fn push(&mut self, command: AudioCommand) -> bool {
        if self.len >= self.commands.len() {
            return false;
        }
        self.commands[self.write_idx] = command;
        self.write_idx = (self.write_idx + 1) % self.commands.len();
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<AudioCommand> {
        if self.len == 0 {
            return None;
        }
        let command = self.commands[self.read_idx];
        self.commands[self.read_idx] = AudioCommand::None;
        self.read_idx = (self.read_idx + 1) % self.commands.len();
        self.len -= 1;
        Some(command)
    }
}

/// Everything shared between the CPU0 facade, the service backend sink, and the
/// CPU1 worker, behind one critical-section cell.
struct AudioShared {
    /// Raw `audio_submit_i16` PCM from CPU0.
    raw: PcmRing<PCM_RING_CAPACITY>,
    /// Service-rendered output blocks awaiting the PWM cadence.
    out: PcmRing<OUT_RING_CAPACITY>,
    commands: AudioCommandQueue,
    high_priority: AudioCommand,
}

impl AudioShared {
    const fn new() -> Self {
        Self {
            raw: PcmRing::new(),
            out: PcmRing::new(),
            commands: AudioCommandQueue::new(),
            high_priority: AudioCommand::None,
        }
    }

    fn reset(&mut self) {
        self.raw.clear();
        self.out.clear();
        self.commands.clear();
        self.high_priority = AudioCommand::None;
    }
}

static AUDIO_SHARED: Mutex<RefCell<AudioShared>> = Mutex::new(RefCell::new(AudioShared::new()));
static AUDIO_STATS: AudioAtomicStats = AudioAtomicStats::new();

// --- Core1 stack canary (KOTO-0186) -----------------------------------------
//
// The core0 main stack has the KOTO-0170 `phase=176` low-water canary, but the
// core1 audio-worker stack (`AUDIO_CORE1_STACK`, a fixed `.bss` region with no
// linker guard below it) had none — the LTO overflow that scribbled
// `APP_STATIC_SHADOW` was invisible until it was heard. This is the same
// paint-and-scan idea applied to that region: core0 paints the whole stack with
// `CANARY_WORD` *before* `spawn_core1` (core1 is not yet running, so the region
// is exclusively owned), and later reads it back — the worker never touches the
// bytes below its deepest frame, so the first painted word still holding the
// pattern, scanned up from the base, marks the low-water mark. `used` is then
// the worst-case worker frame depth and `free_min` the margin left before the
// stack would grow into whatever `.bss` sits below it.
const CORE1_CANARY_WORD: u32 = 0x6F74_6F6B; // "koto", LE ASCII (matches core0).
/// Base (lowest address) of the painted core1 stack region. Zero until painted.
static CORE1_STACK_BASE: AtomicUsize = AtomicUsize::new(0);
/// One-past-the-top of the painted core1 stack region. Zero until painted.
static CORE1_STACK_TOP: AtomicUsize = AtomicUsize::new(0);

/// Paints the core1 worker stack with [`CORE1_CANARY_WORD`]. Must run on core0
/// before `spawn_core1` hands the stack to the worker; at that point core1 is
/// idle so the whole region is dead memory this core owns exclusively.
fn paint_core1_stack(stack: &mut Stack<AUDIO_CORE1_STACK_BYTES>) {
    let base = (stack as *mut Stack<AUDIO_CORE1_STACK_BYTES> as usize + 3) & !3;
    let top = (stack as *mut Stack<AUDIO_CORE1_STACK_BYTES> as usize
        + core::mem::size_of::<Stack<AUDIO_CORE1_STACK_BYTES>>())
        & !3;
    let mut word = base as *mut u32;
    while (word as usize) < top {
        // Volatile so the fill is not elided against the later cross-core scan
        // of the same untyped memory.
        unsafe {
            word.write_volatile(CORE1_CANARY_WORD);
            word = word.add(1);
        }
    }
    CORE1_STACK_BASE.store(base, Ordering::Release);
    CORE1_STACK_TOP.store(top, Ordering::Release);
}

/// Scans the painted core1 stack from the base upward for the worker's session
/// low-water mark and returns the untouched margin below it, in bytes. `None`
/// until [`paint_core1_stack`] has run. The scan is a read-only walk of memory
/// the worker never writes below its deepest frame, so it needs no
/// synchronization with the running worker.
fn core1_stack_free_min() -> Option<usize> {
    let base = CORE1_STACK_BASE.load(Ordering::Acquire);
    let top = CORE1_STACK_TOP.load(Ordering::Acquire);
    if base == 0 || top <= base {
        return None;
    }
    let mut word = base as *const u32;
    while (word as usize) < top {
        if unsafe { word.read_volatile() } != CORE1_CANARY_WORD {
            break;
        }
        word = unsafe { word.add(1) };
    }
    Some(word as usize - base)
}

/// The ported koto-audio service shape used on device.
type PicoAudioService = DefaultAudioService<'static, PwmBlockSink>;

/// CPU1-owned service storage (KOTO-0148: retained state in its own StaticCell,
/// out of the CPU1 stack and the main-task future).
static AUDIO_SERVICE: StaticCell<PicoAudioService> = StaticCell::new();

// Shared-cell and service budgets: the raw ring dominates (4 KiB, unchanged
// from the pre-KOTO-0165 worker); the service replaces the two removed
// by-value `PicoBgmScore` command slots and the legacy players.
const _: () = assert!(core::mem::size_of::<AudioShared>() <= 6 * 1024);
const _: () = assert!(core::mem::size_of::<PicoAudioService>() <= 4 * 1024);

struct AudioAtomicStats {
    samples_submitted: AtomicU32,
    samples_played: AtomicU32,
    drops: AtomicU32,
    underruns: AtomicU32,
    unsupported_count: AtomicU32,
    command_drops: AtomicU32,
    bgm_starts: AtomicU32,
    bgm_stops: AtomicU32,
    active_bgm_voices: AtomicU32,
    active_sfx_voices: AtomicU32,
    mixer_saturations: AtomicU32,
    worker_late: AtomicU32,
    worker_max_jitter_us: AtomicU32,
    /// Monotonic CPU1 worker-loop pass counter (KOTO-0186). A live worker bumps
    /// this every `run()` pass (~1 ms); if it stops advancing across two
    /// `phase=173` samples the worker is wedged or dead, so worker liveness is a
    /// number on UART instead of an inferred-from-the-buzz mystery. It is *not*
    /// zeroed by `reset()` (a `StopAll` mid-session must not look like a stall).
    worker_heartbeat: AtomicU32,
}

impl AudioAtomicStats {
    const fn new() -> Self {
        Self {
            samples_submitted: AtomicU32::new(0),
            samples_played: AtomicU32::new(0),
            drops: AtomicU32::new(0),
            underruns: AtomicU32::new(0),
            unsupported_count: AtomicU32::new(0),
            command_drops: AtomicU32::new(0),
            bgm_starts: AtomicU32::new(0),
            bgm_stops: AtomicU32::new(0),
            active_bgm_voices: AtomicU32::new(0),
            active_sfx_voices: AtomicU32::new(0),
            mixer_saturations: AtomicU32::new(0),
            worker_late: AtomicU32::new(0),
            worker_max_jitter_us: AtomicU32::new(0),
            worker_heartbeat: AtomicU32::new(0),
        }
    }

    fn reset(&self) {
        self.samples_submitted.store(0, Ordering::Relaxed);
        self.samples_played.store(0, Ordering::Relaxed);
        self.drops.store(0, Ordering::Relaxed);
        self.underruns.store(0, Ordering::Relaxed);
        self.unsupported_count.store(0, Ordering::Relaxed);
        self.command_drops.store(0, Ordering::Relaxed);
        self.bgm_starts.store(0, Ordering::Relaxed);
        self.bgm_stops.store(0, Ordering::Relaxed);
        self.active_bgm_voices.store(0, Ordering::Relaxed);
        self.active_sfx_voices.store(0, Ordering::Relaxed);
        self.mixer_saturations.store(0, Ordering::Relaxed);
        self.worker_late.store(0, Ordering::Relaxed);
        self.worker_max_jitter_us.store(0, Ordering::Relaxed);
    }

    fn inc(counter: &AtomicU32, by: u32) {
        counter.fetch_add(by, Ordering::Relaxed);
    }

    fn set_max(counter: &AtomicU32, value: u32) {
        let mut current = counter.load(Ordering::Relaxed);
        while value > current {
            match counter.compare_exchange_weak(
                current,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }
}

/// The koto-audio backend boundary on device: each mixed block is queued into
/// the shared output ring, which the worker's PWM cadence drains. The worker
/// only ticks the service when a whole block fits, so `QueueFull` here means a
/// logic bug rather than expected backpressure.
struct PwmBlockSink {
    state: BackendState,
}

impl PwmBlockSink {
    const fn new() -> Self {
        Self {
            state: BackendState::Stopped,
        }
    }
}

impl AudioBackend<BLOCK_FRAMES> for PwmBlockSink {
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if self.state != BackendState::Running {
            return Err(BackendError::NotRunning);
        }
        let fitted = critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            if shared.out.len() + BLOCK_FRAMES > OUT_RING_CAPACITY {
                return false;
            }
            for &sample in block.as_pcm16_mono() {
                let _ = shared.out.push(sample);
            }
            true
        });
        if fitted {
            Ok(BackendReport::submitted_block())
        } else {
            Err(BackendError::QueueFull)
        }
    }

    fn suspend(&mut self) -> BackendResult {
        self.state = BackendState::Suspended;
        Ok(BackendReport::default())
    }

    fn resume(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn query_state(&self) -> BackendState {
        self.state
    }

    fn reset(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }
}

pub enum PcmSubmitError {
    BadArgument,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioBackendStats {
    pub samples_submitted: u32,
    pub samples_played: u32,
    pub drops: u32,
    pub underruns: u32,
    pub unsupported_count: u32,
    pub buffer_level: u32,
    pub buffer_capacity: u32,
    pub command_drops: u32,
    pub bgm_starts: u32,
    pub bgm_stops: u32,
    pub active_bgm_voices: u32,
    pub active_sfx_voices: u32,
    pub mixer_saturations: u32,
    pub worker_late: u32,
    pub worker_max_jitter_us: u32,
    /// Monotonic CPU1 worker-loop pass counter (KOTO-0186); a frozen value
    /// across two samples means the worker is wedged or dead.
    pub worker_heartbeat: u32,
    /// Untouched bytes below the deepest observed core1 worker frame
    /// (KOTO-0186); `u32::MAX` means the canary was not painted.
    pub core1_stack_free_min: u32,
}

/// CPU0 facade over the CPU1 audio worker.
pub struct PicoAudioBackend;

impl PicoAudioBackend {
    pub fn spawn_cpu1(
        core1: Peri<'static, CORE1>,
        stack: &'static mut Stack<AUDIO_CORE1_STACK_BYTES>,
        pwm: Pwm<'static>,
    ) -> Self {
        critical_section::with(|cs| AUDIO_SHARED.borrow_ref_mut(cs).reset());
        AUDIO_STATS.reset();
        // KOTO-0186: paint the worker stack while core1 is still idle so the
        // cross-core `core1_stack_free_min` scan can measure the worst-case
        // worker frame depth after the LTO in-place-reset fix.
        paint_core1_stack(stack);
        // Build the ~3 KiB service HERE on the CPU0 main stack and move it into
        // its StaticCell before spawning: constructing it on the CPU1 stack
        // would blow the whole core1 stack budget by itself. Construction only
        // fails on an invalid policy, which `v0_default()` cannot produce; the
        // `None` fallback keeps CPU1 in a silent, panic-free loop.
        let service = PicoAudioService::new(AudioPolicy::v0_default(), PwmBlockSink::new())
            .ok()
            .map(|service| AUDIO_SERVICE.init(service));
        spawn_core1(core1, stack, move || -> ! {
            run_audio_worker(pwm, service)
        });
        Self
    }

    pub const fn backend_name(&self) -> &'static str {
        "cpu1_pwm_koto_audio_gp26_gp27"
    }

    pub const fn sample_rate_hz(&self) -> u32 {
        PCM_OUTPUT_SAMPLE_RATE_HZ
    }

    pub fn submit_pcm_i16(
        &mut self,
        sample_rate_hz: u32,
        frames: i32,
        channels: i32,
        samples: &[u8],
    ) -> Result<i32, PcmSubmitError> {
        if sample_rate_hz != self.sample_rate_hz() {
            self.record_unsupported();
            return Err(PcmSubmitError::Unsupported);
        }
        if frames <= 0 || !matches!(channels, 1 | 2) {
            return Err(PcmSubmitError::BadArgument);
        }
        let expected_len = (frames as usize)
            .saturating_mul(channels as usize)
            .saturating_mul(2);
        if samples.len() != expected_len {
            return Err(PcmSubmitError::BadArgument);
        }

        let channel_count = channels as usize;
        let mut accepted_frames = 0u32;
        let mut dropped_frames = 0u32;
        critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            for frame_idx in 0..frames as usize {
                let offset = frame_idx * channel_count * 2;
                let mono = if channel_count == 1 {
                    i16::from_le_bytes([samples[offset], samples[offset + 1]])
                } else {
                    let l = i16::from_le_bytes([samples[offset], samples[offset + 1]]) as i32;
                    let r = i16::from_le_bytes([samples[offset + 2], samples[offset + 3]]) as i32;
                    ((l + r) / 2) as i16
                };
                if shared.raw.push(mono) {
                    accepted_frames += 1;
                } else {
                    dropped_frames = dropped_frames.saturating_add(1);
                }
            }
        });

        AudioAtomicStats::inc(&AUDIO_STATS.samples_submitted, accepted_frames);
        AudioAtomicStats::inc(&AUDIO_STATS.drops, dropped_frames);
        Ok(accepted_frames as i32)
    }

    pub fn submit_pcm_mono_i16(
        &mut self,
        sample_rate_hz: u32,
        samples: &[i16],
    ) -> Result<i32, PcmSubmitError> {
        if sample_rate_hz != self.sample_rate_hz() {
            self.record_unsupported();
            return Err(PcmSubmitError::Unsupported);
        }

        let mut accepted = 0u32;
        let mut dropped = 0u32;
        critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            for &sample in samples {
                if shared.raw.push(sample) {
                    accepted += 1;
                } else {
                    dropped = dropped.saturating_add(1);
                }
            }
        });
        AudioAtomicStats::inc(&AUDIO_STATS.samples_submitted, accepted);
        AudioAtomicStats::inc(&AUDIO_STATS.drops, dropped);
        Ok(accepted as i32)
    }

    /// Starts (or replaces) the looping BGM sequence for a routed cue.
    pub fn play_bgm_cue(&mut self, sequence: &'static PolyphonicSequence<'static>) {
        self.enqueue_command(AudioCommand::PlayBgm(sequence));
    }

    /// Plays a one-shot SFX sequence for a routed cue.
    pub fn play_sfx_cue(&mut self, sequence: &'static Sequence<'static>) {
        self.enqueue_command(AudioCommand::PlaySfx(sequence));
    }

    pub fn stop_bgm(&mut self) {
        critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            // StopAll outranks StopBgm in the single high-priority slot.
            if !matches!(shared.high_priority, AudioCommand::StopAll) {
                shared.high_priority = AudioCommand::StopBgm;
            }
        });
    }

    pub fn stop(&mut self) {
        critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            shared.high_priority = AudioCommand::StopAll;
        });
    }

    pub fn service(&mut self) {}

    pub fn record_samples_submitted(&mut self, frames: u32) {
        AudioAtomicStats::inc(&AUDIO_STATS.samples_submitted, frames);
    }

    pub fn record_drop(&mut self) {
        AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
    }

    pub fn record_unsupported(&mut self) {
        AudioAtomicStats::inc(&AUDIO_STATS.unsupported_count, 1);
    }

    pub fn stats(&self) -> AudioBackendStats {
        let buffer_level =
            critical_section::with(|cs| AUDIO_SHARED.borrow_ref(cs).raw.len() as u32);
        AudioBackendStats {
            samples_submitted: AUDIO_STATS.samples_submitted.load(Ordering::Relaxed),
            samples_played: AUDIO_STATS.samples_played.load(Ordering::Relaxed),
            drops: AUDIO_STATS.drops.load(Ordering::Relaxed),
            underruns: AUDIO_STATS.underruns.load(Ordering::Relaxed),
            unsupported_count: AUDIO_STATS.unsupported_count.load(Ordering::Relaxed),
            buffer_level,
            buffer_capacity: PCM_RING_CAPACITY as u32,
            command_drops: AUDIO_STATS.command_drops.load(Ordering::Relaxed),
            bgm_starts: AUDIO_STATS.bgm_starts.load(Ordering::Relaxed),
            bgm_stops: AUDIO_STATS.bgm_stops.load(Ordering::Relaxed),
            active_bgm_voices: AUDIO_STATS.active_bgm_voices.load(Ordering::Relaxed),
            active_sfx_voices: AUDIO_STATS.active_sfx_voices.load(Ordering::Relaxed),
            mixer_saturations: AUDIO_STATS.mixer_saturations.load(Ordering::Relaxed),
            worker_late: AUDIO_STATS.worker_late.load(Ordering::Relaxed),
            worker_max_jitter_us: AUDIO_STATS.worker_max_jitter_us.load(Ordering::Relaxed),
            worker_heartbeat: AUDIO_STATS.worker_heartbeat.load(Ordering::Relaxed),
            core1_stack_free_min: core1_stack_free_min()
                .and_then(|free| u32::try_from(free).ok())
                .unwrap_or(u32::MAX),
        }
    }

    fn enqueue_command(&mut self, command: AudioCommand) {
        let accepted = critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            shared.commands.push(command)
        });
        if !accepted {
            AudioAtomicStats::inc(&AUDIO_STATS.command_drops, 1);
            AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
        }
    }
}

/// CPU1 entry: run the fill loop against the service CPU0 already parked in
/// its StaticCell. Only worker-loop frames live on the core1 stack.
///
/// Output pacing is **hardware-owned** (KOTO-0114 pattern, validated by
/// `probe_audio`): a DMA channel paced by DMA timer 0 streams precomputed duty
/// words from [`AUDIO_DMA_RING`] into the PWM slice-5 compare register at
/// exactly 16 kHz. The worker only *fills* the ring ahead of the DMA read
/// position on a coarse ~1 ms pass, so a multi-millisecond service render burst
/// never disturbs sample timing (the CPU-paced first cut audibly did).
fn run_audio_worker(mut pwm: Pwm<'static>, service: Option<&'static mut PicoAudioService>) -> ! {
    configure_pcm_output(&mut pwm);
    let Some(service) = service else {
        let _ = pwm.set_duty_cycle(IDLE_SILENCE_DUTY);
        loop {
            block_for(Duration::from_millis(100));
        }
    };
    let _ = service.start();

    let ring = unsafe { &mut *core::ptr::addr_of_mut!(AUDIO_DMA_RING) };
    let silence = duty_word(0);
    for slot in ring.0.iter_mut() {
        *slot = silence;
    }
    setup_dma_pacing_timer();
    arm_audio_dma(ring);

    let mut worker = PicoAudioWorker {
        _pwm: pwm,
        service,
        pending_sources: 0,
        underrun_latched: false,
        // The pre-filled silence ring means the writer already leads the DMA
        // reader by one full ring.
        write_pos: DMA_RING_SAMPLES as u64,
        sent_base: 0,
    };
    worker.run()
}

struct PicoAudioWorker<'d> {
    /// Held only to keep the PWM slice configured and alive; the compare
    /// register is written by DMA, not by this handle.
    _pwm: Pwm<'d>,
    service: &'static mut PicoAudioService,
    /// Active + queued service sources after the last command/tick.
    pending_sources: u32,
    underrun_latched: bool,
    /// Total duty words written into the DMA ring since start.
    write_pos: u64,
    /// Samples consumed by DMA in previous arm periods (see [`arm_audio_dma`]).
    sent_base: u64,
}

impl<'d> PicoAudioWorker<'d> {
    fn run(&mut self) -> ! {
        loop {
            let pass_start = Instant::now();

            // Liveness beat: bumped once per pass so core0 can see the worker is
            // alive (KOTO-0186). Placed at the top of the loop so it advances
            // even if a later stage would wedge on the next pass.
            AudioAtomicStats::inc(&AUDIO_STATS.worker_heartbeat, 1);

            self.drain_commands();
            self.render_ahead();
            self.fill_dma_ring();

            let elapsed_us = pass_start.elapsed().as_micros();
            // Diagnostics: max pass duration approximates the worst render
            // burst; a pass longer than half the ring lead is flagged late.
            AudioAtomicStats::set_max(
                &AUDIO_STATS.worker_max_jitter_us,
                elapsed_us.min(u64::from(u32::MAX)) as u32,
            );
            if elapsed_us > WORKER_LATE_PASS_US {
                AudioAtomicStats::inc(&AUDIO_STATS.worker_late, 1);
            }
            block_for(Duration::from_micros(
                WORKER_PASS_PERIOD_US.saturating_sub(elapsed_us),
            ));
        }
    }

    fn drain_commands(&mut self) {
        let high_command = critical_section::with(|cs| {
            let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
            let high_command = shared.high_priority;
            shared.high_priority = AudioCommand::None;
            high_command
        });
        self.apply_command(high_command);
        // The queue is bounded (16), so this pass-local drain is bounded too.
        while let Some(command) =
            critical_section::with(|cs| AUDIO_SHARED.borrow_ref_mut(cs).commands.pop())
        {
            self.apply_command(command);
        }
    }

    /// Keeps the service output ring topped up. Bounded to **one** mixer block
    /// per pass: a render burst is the dominant pass cost, and a pass must
    /// never outlast the 16 ms DMA ring lead or the reader replays stale
    /// slots (`underruns`). One 8 ms block per ~1 ms pass is still 8× real
    /// time, and a drained out-ring refills in 3 passes (~3 ms). The previous
    /// cap of 3 let one pass stack three bursts back-to-back — under KotoRun
    /// SMASH load (BGM + overlapping SFX sources, plus core0's heavy frame
    /// thrashing the shared XIP cache) that produced 25.8 ms passes and the
    /// phase=173 `worker_late`/`underruns` climb.
    fn render_ahead(&mut self) {
        let mut rendered = 0;
        while self.pending_sources > 0 && rendered < 1 {
            let out_level = critical_section::with(|cs| AUDIO_SHARED.borrow_ref(cs).out.len());
            if out_level + BLOCK_FRAMES > OUT_RING_CAPACITY {
                break;
            }
            let _ = self.service.tick();
            while self.service.poll_audio_event().is_some() {}
            self.refresh_source_stats();
            rendered += 1;
        }
    }

    /// Advances the writer up to one full ring ahead of the DMA read position,
    /// mixing the service output and the raw PCM stream into duty words.
    fn fill_dma_ring(&mut self) {
        let channel = pac::DMA.ch(AUDIO_DMA_CH);
        let remaining = channel.trans_count().read();
        if remaining == 0 {
            // The huge arm count ran out (days of audio); account for it and
            // re-trigger. The ring-wrapped read address carries on in place.
            self.sent_base += u64::from(DMA_ARM_COUNT);
            let ring = unsafe { &*core::ptr::addr_of!(AUDIO_DMA_RING) };
            arm_audio_dma(ring);
            return;
        }
        let sent = self.sent_base + u64::from(DMA_ARM_COUNT - remaining);

        if self.write_pos < sent {
            // DMA overtook the writer (a pass stalled longer than the ring):
            // it replayed stale slots. Resync and count one underrun.
            if !self.underrun_latched {
                AudioAtomicStats::inc(&AUDIO_STATS.underruns, 1);
                self.underrun_latched = true;
            }
            self.write_pos = sent;
        }

        let ring = unsafe { &mut *core::ptr::addr_of_mut!(AUDIO_DMA_RING) };
        let mut raw_played = 0u32;
        // Pop shared-ring samples in chunks under one critical section instead
        // of one section per sample. The section is the process-wide dual-core
        // spinlock (embassy's time driver and CPU0's audio facade contend on
        // it), so up to 256 acquisitions per pass was real pass-time under
        // KotoRun SMASH load (phase=173 `worker_late`). Chunking keeps each
        // hold bounded (≤ FILL_CHUNK ring pops) while cutting acquisitions to
        // a handful per pass. The per-sample mixing/underrun semantics below
        // are transition-equivalent to the old per-sample loop: within one
        // chunk the `out` ring yields all its samples before running dry, so
        // the latch sees the same some→none edges.
        const FILL_CHUNK: usize = 32;
        while self.write_pos < sent + DMA_RING_SAMPLES as u64 {
            let needed =
                ((sent + DMA_RING_SAMPLES as u64 - self.write_pos) as usize).min(FILL_CHUNK);
            let mut seq_buf = [0i16; FILL_CHUNK];
            let mut raw_buf = [0i16; FILL_CHUNK];
            let (seq_len, raw_len) = critical_section::with(|cs| {
                let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
                let mut seq_len = 0;
                while seq_len < needed {
                    match shared.out.pop() {
                        Some(sample) => {
                            seq_buf[seq_len] = sample;
                            seq_len += 1;
                        }
                        None => break,
                    }
                }
                let mut raw_len = 0;
                while raw_len < needed {
                    match shared.raw.pop() {
                        Some(sample) => {
                            raw_buf[raw_len] = sample;
                            raw_len += 1;
                        }
                        None => break,
                    }
                }
                (seq_len, raw_len)
            });

            if seq_len > 0 {
                self.underrun_latched = false;
            }
            if seq_len < needed && self.pending_sources > 0 && !self.underrun_latched {
                AudioAtomicStats::inc(&AUDIO_STATS.underruns, 1);
                self.underrun_latched = true;
            }

            for index in 0..needed {
                let seq = if index < seq_len { seq_buf[index] } else { 0 };
                let mut mixed = i32::from(seq);
                if index < raw_len {
                    mixed = mixed.saturating_add(i32::from(raw_buf[index]));
                }
                let clamped = mixed.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                if clamped != mixed {
                    AudioAtomicStats::inc(&AUDIO_STATS.mixer_saturations, 1);
                }

                ring.0[(self.write_pos % DMA_RING_SAMPLES as u64) as usize] =
                    duty_word(clamped as i16);
                self.write_pos += 1;
            }
            raw_played += raw_len as u32;
        }
        if raw_played > 0 {
            AudioAtomicStats::inc(&AUDIO_STATS.samples_played, raw_played);
        }
    }

    fn apply_command(&mut self, command: AudioCommand) {
        match command {
            AudioCommand::None => {}
            AudioCommand::PlayBgm(sequence) => {
                if self.service.play_bgm_sequence(*sequence).is_ok() {
                    AudioAtomicStats::inc(&AUDIO_STATS.bgm_starts, 1);
                } else {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
                self.refresh_source_stats();
            }
            AudioCommand::PlaySfx(sequence) => {
                if self.service.play_sequence(*sequence).is_err() {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
                self.refresh_source_stats();
            }
            AudioCommand::StopBgm => {
                let _ = self.service.stop_bgm();
                AudioAtomicStats::inc(&AUDIO_STATS.bgm_stops, 1);
                self.refresh_source_stats();
            }
            AudioCommand::StopAll => {
                // Full bounded reset: sources, mixer, events, and both rings.
                let _ = self.service.reset();
                let _ = self.service.start();
                critical_section::with(|cs| {
                    let mut shared = AUDIO_SHARED.borrow_ref_mut(cs);
                    shared.raw.clear();
                    shared.out.clear();
                });
                AudioAtomicStats::inc(&AUDIO_STATS.bgm_stops, 1);
                self.refresh_source_stats();
            }
        }
    }

    fn refresh_source_stats(&mut self) {
        let snapshot = self.service.counter_snapshot();
        self.pending_sources =
            u32::from(snapshot.active_source_count) + u32::from(snapshot.queued_source_count);
        AUDIO_STATS.active_bgm_voices.store(
            u32::from(snapshot.active_bgm_source_count),
            Ordering::Relaxed,
        );
        AUDIO_STATS.active_sfx_voices.store(
            u32::from(snapshot.active_sfx_source_count),
            Ordering::Relaxed,
        );
    }
}

/// A precomputed PWM compare word for both output channels (A|B) of slice 5.
fn duty_word(sample: i16) -> u32 {
    let duty = i16_to_pwm_duty(sample, PCM_PWM_TOP);
    u32::from(duty) | (u32::from(duty) << 16)
}

/// Programs DMA timer 0 to request one transfer per output sample:
/// `rate = clk_sys * X / Y`. Both factors must fit `u16`; if the reduced
/// fraction does not (an unusual clk_sys), halving both approximates the rate
/// within a fraction of a hertz rather than failing.
fn setup_dma_pacing_timer() {
    let system_hz = clk_sys_freq();
    let divisor = gcd(system_hz, PCM_OUTPUT_SAMPLE_RATE_HZ);
    let mut x = PCM_OUTPUT_SAMPLE_RATE_HZ / divisor;
    let mut y = system_hz / divisor;
    while y > u32::from(u16::MAX) {
        x = (x / 2).max(1);
        y /= 2;
    }
    pac::DMA.timer(0).write(|w| {
        w.set_x(x as u16);
        w.set_y(y as u16);
    });
}

/// Arms (or re-arms) the audio DMA channel: read the duty-word ring with
/// hardware address wrapping, write the fixed PWM slice-5 compare register,
/// one word per DMA-timer request. `DMA_ARM_COUNT` lasts days at 16 kHz; on
/// exhaustion `fill_dma_ring` re-arms and the wrapped read address continues
/// in place.
fn arm_audio_dma(ring: &AlignedDmaRing) {
    let channel = pac::DMA.ch(AUDIO_DMA_CH);
    if !channel.ctrl_trig().read().busy() {
        channel.read_addr().write_value(ring.0.as_ptr() as u32);
        channel
            .write_addr()
            .write_value(pac::PWM.ch(5).cc().as_ptr() as u32);
    }
    channel.trans_count().write_value(DMA_ARM_COUNT);
    channel.ctrl_trig().write(|w| {
        w.set_data_size(DataSize::SIZE_WORD);
        w.set_incr_read(true);
        w.set_incr_write(false);
        w.set_ring_size(DMA_RING_BYTES_LOG2);
        w.set_ring_sel(false);
        w.set_chain_to(AUDIO_DMA_CH as u8);
        w.set_treq_sel(TreqSel::TIMER0);
        w.set_irq_quiet(true);
        w.set_en(true);
    });
}

const fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn configure_pcm_output(pwm: &mut Pwm<'_>) {
    let mut config = PwmConfig::default();
    config.divider = PCM_PWM_DIVIDER.into();
    config.top = PCM_PWM_TOP;
    let midpoint = (PCM_PWM_TOP / 2).max(1);
    config.compare_a = midpoint;
    config.compare_b = midpoint;
    pwm.set_config(&config);
}

fn i16_to_pwm_duty(sample: i16, top: u16) -> u16 {
    let centered = i32::from(top / 2);
    let scaled = i32::from(sample) * i32::from(top.saturating_sub(2)) / (2 * i32::from(i16::MAX));
    (centered + scaled).clamp(1, i32::from(top.saturating_sub(1))) as u16
}
