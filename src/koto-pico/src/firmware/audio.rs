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
//! The former in-tree engine — the runtime KotoMML parser, custom wavetable
//! loader, the hand-rolled square/pulse/triangle/saw/noise synth, the mirrored
//! `PicoBgmScore` tables, and the tone-fallback path — was removed. Every
//! sequence cue now plays through the `koto-audio` crate itself (the same
//! runtime the SIM bridge uses). Package cues arrive as pointer-free runtime
//! images compiled from the SD-resident Native KMML and staged through PSRAM.
//! Only two inputs
//! reach this module:
//!
//! * **Sequence cues**: built-in static sequences and owned runtime cue images.
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
    cell::{RefCell, UnsafeCell},
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::firmware::audio_residency::{
    AudioResidencyOwner, ResidencyState, ResidencyToken, TransitionError,
};
use critical_section::Mutex;
use embassy_rp::{
    clocks::clk_sys_freq,
    multicore::{spawn_core1, Stack},
    pac,
    pac::dma::vals::{DataSize, TreqSel},
    peripherals::CORE1,
    pwm::{Config as PwmConfig, Pwm},
    Peri,
};
use embassy_time::{block_for, Duration, Instant};
use koto_audio::{
    runtime_cue_max_encoded_len, AudioBackend, AudioLimits, AudioPolicy, BackendError,
    BackendReport, BackendResult, BackendState, DecodeResult, DefaultAudioService, MixerBlock,
    MixerVolume, OwnedClipPlayer, PolyphonicSequence, RuntimeCuePlayer, Sequence,
    DEFAULT_MIXER_BLOCK_FRAMES,
};
use portable_atomic::{AtomicBool, AtomicU32};

/// Output sample rate. Matches `AudioLimits::v0_default()` and the sample rate
/// the koto-audio built-in drum tables are authored at, so drums play at pitch.
const PCM_OUTPUT_SAMPLE_RATE_HZ: u32 = 16_000;
/// Fixed service mixer block length (frames per `tick()`).
const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES;
/// Raw `audio_submit_i16` ring capacity (256 ms at 16 kHz). Long KPA streams
/// need enough lead to cover an SD file open + seek + multi-sector read without
/// exposing storage latency to the hardware-paced DMA output, while keeping the
/// RP2040 SRAM increase bounded.
const PCM_RING_CAPACITY: usize = 4096;
/// Rendered service output ring: three mixer blocks (~24 ms) of read-ahead.
const OUT_RING_CAPACITY: usize = BLOCK_FRAMES * 3;
const AUDIO_COMMAND_CAPACITY: usize = 16;
pub const RUNTIME_BGM_EVENTS_PER_TRACK: usize = 272;
pub const RUNTIME_SFX_EVENTS_PER_TRACK: usize = 32;
pub const RUNTIME_CUE_IMAGE_CAPACITY: usize =
    runtime_cue_max_encoded_len::<RUNTIME_BGM_EVENTS_PER_TRACK>();
/// Maximum complete KACL package image copied to CPU1.
pub const RUNTIME_CLIP_IMAGE_CAPACITY: usize = 8192;
const RUNTIME_SFX_PLAYERS: usize = 3;
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

/// Bounded audio work CPU0 hands to the CPU1 worker. Fixed host cues carry
/// static references; package cues use the owned runtime-image staging slot.
#[derive(Clone, Copy)]
enum AudioCommand {
    None,
    PlayBgm(&'static PolyphonicSequence<'static>),
    PlaySfx(&'static Sequence<'static>),
    PlayRuntimeBgm,
    PlayRuntimeSfx,
    PlayRuntimeClip,
    StopBgm,
    StopAll,
    QuiesceRich(u32),
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

/// Permanently resident state required by PCM16/SLDPCM4 package streaming.
struct StreamAudioShared {
    /// Raw `audio_submit_i16` PCM from CPU0.
    raw: PcmRing<PCM_RING_CAPACITY>,
}

impl StreamAudioShared {
    const fn new() -> Self {
        Self {
            raw: PcmRing::new(),
        }
    }

    fn reset(&mut self) {
        self.raw.clear();
    }
}

/// State used only by the full sequence/cue/owned-clip audio service.
struct RichAudioShared {
    /// Service-rendered output blocks awaiting the PWM cadence.
    out: PcmRing<OUT_RING_CAPACITY>,
    commands: AudioCommandQueue,
    high_priority: AudioCommand,
    runtime_image_len: usize,
    runtime_image_busy: bool,
}

impl RichAudioShared {
    const fn new() -> Self {
        Self {
            out: PcmRing::new(),
            commands: AudioCommandQueue::new(),
            high_priority: AudioCommand::None,
            runtime_image_len: 0,
            runtime_image_busy: false,
        }
    }

    fn reset(&mut self) {
        self.out.clear();
        self.commands.clear();
        self.high_priority = AudioCommand::None;
        self.runtime_image_len = 0;
        self.runtime_image_busy = false;
    }
}

#[repr(transparent)]
struct StreamAudioArena(UnsafeCell<StreamAudioShared>);

// Safety: ordinary access is serialized by the cross-core critical section.
// Direct TLS workspace access is possible only after the CPU1/DMA offline ACK
// and remains exclusive until the generation-owned handle is returned.
unsafe impl Sync for StreamAudioArena {}

#[unsafe(no_mangle)]
static AUDIO_STREAM_SHARED: StreamAudioArena =
    StreamAudioArena(UnsafeCell::new(StreamAudioShared::new()));

fn with_stream_audio<R>(use_shared: impl FnOnce(&mut StreamAudioShared) -> R) -> R {
    critical_section::with(|_| use_shared(unsafe { &mut *AUDIO_STREAM_SHARED.0.get() }))
}

static AUDIO_STATS: AudioAtomicStats = AudioAtomicStats::new();
static AUDIO_RESIDENCY_OWNER: Mutex<RefCell<AudioResidencyOwner>> =
    Mutex::new(RefCell::new(AudioResidencyOwner::new()));
static WORKER_RICH_ACTIVE: AtomicBool = AtomicBool::new(true);
static RICH_SERVICE_READY: AtomicBool = AtomicBool::new(false);
static WORKER_RICH_OFFLINE_GENERATION: AtomicU32 = AtomicU32::new(0);
static WORKER_RICH_ONLINE_GENERATION: AtomicU32 = AtomicU32::new(0);
static WORKER_STREAM_ACTIVE: AtomicBool = AtomicBool::new(true);
static WORKER_STREAM_OFFLINE_GENERATION: AtomicU32 = AtomicU32::new(0);
static WORKER_STREAM_ONLINE_GENERATION: AtomicU32 = AtomicU32::new(0);
static TLS_AUDIO_WORKSPACE_CLAIMED: AtomicBool = AtomicBool::new(false);
const WORKER_CONTROL_NONE: u32 = 0;
const WORKER_CONTROL_QUIESCE_RICH: u32 = 1;
const WORKER_CONTROL_ACTIVATE_RICH: u32 = 2;
const WORKER_CONTROL_QUIESCE_STREAM: u32 = 3;
const WORKER_CONTROL_ACTIVATE_STREAM: u32 = 4;
static WORKER_CONTROL: AtomicU32 = AtomicU32::new(WORKER_CONTROL_NONE);
static WORKER_CONTROL_GENERATION: AtomicU32 = AtomicU32::new(0);
/// CPU0 marks the host-managed KPA stream active after its first successful
/// refill. CPU1 uses this only to distinguish an expected empty raw ring from
/// a streaming starvation gap in the `underruns` diagnostic.
static PCM_STREAM_ACTIVE: AtomicBool = AtomicBool::new(false);

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

#[repr(transparent)]
struct RichSlot<T>(UnsafeCell<MaybeUninit<T>>);

// `&'static self -> &'static mut T` goes through the `UnsafeCell`, the
// sanctioned interior-mutability escape hatch; exclusive access is the
// residency owner's serialization contract (see `RichAudioArena`), which the
// `mut_from_ref` lint (deny-by-default since clippy 1.96) cannot see.
#[allow(clippy::mut_from_ref)]
impl<T> RichSlot<T> {
    const fn new() -> Self {
        Self(UnsafeCell::new(MaybeUninit::uninit()))
    }

    unsafe fn init(&'static self, value: T) -> &'static mut T {
        let slot = self.0.get();
        unsafe {
            slot.write(MaybeUninit::new(value));
            &mut *slot.cast::<T>()
        }
    }

    /// Copies `template` into the slot without materializing a `T` on the
    /// caller's stack (KOTO-0252): [`Self::init`] receives its value by value,
    /// so a player-sized argument is built on the calling frame first — the
    /// ~18 KiB BGM temporary set the shell-path stack low-water mark. Here the
    /// source stays wherever it lives (XIP flash for the idle-player
    /// templates) and the copy is one direct `memcpy` into the arena.
    unsafe fn init_from(&'static self, template: &T) -> &'static mut T
    where
        T: Copy,
    {
        let slot = self.0.get().cast::<T>();
        unsafe {
            core::ptr::copy_nonoverlapping(template, slot, 1);
            &mut *slot
        }
    }
    unsafe fn assume_init_mut(&'static self) -> &'static mut T {
        unsafe { &mut *self.0.get().cast::<T>() }
    }
}

pub const RICH_AUDIO_RESIDENCY_BYTES: usize = 36 * 1024;
const RICH_AUDIO_STORAGE_BYTES: usize = core::mem::size_of::<Mutex<RefCell<RichAudioShared>>>()
    + core::mem::size_of::<UnsafeCell<[u8; RUNTIME_CUE_IMAGE_CAPACITY]>>()
    + core::mem::size_of::<RichSlot<PicoAudioService>>()
    + core::mem::size_of::<RichSlot<RuntimeCuePlayer<RUNTIME_BGM_EVENTS_PER_TRACK>>>()
    + core::mem::size_of::<
        RichSlot<[RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK>; RUNTIME_SFX_PLAYERS]>,
    >()
    + core::mem::size_of::<RichSlot<OwnedClipPlayer<RUNTIME_CLIP_IMAGE_CAPACITY>>>();
const RICH_AUDIO_RESERVE_BYTES: usize = RICH_AUDIO_RESIDENCY_BYTES - RICH_AUDIO_STORAGE_BYTES;

/// CPU1-owned rich-service storage, kept contiguous for RP2040 residency reuse.
#[repr(C, align(8))]
struct RichAudioResidency {
    shared: Mutex<RefCell<RichAudioShared>>,
    runtime_image: UnsafeCell<[u8; RUNTIME_CUE_IMAGE_CAPACITY]>,
    service: RichSlot<PicoAudioService>,
    runtime_bgm: RichSlot<RuntimeCuePlayer<RUNTIME_BGM_EVENTS_PER_TRACK>>,
    runtime_sfx: RichSlot<[RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK>; RUNTIME_SFX_PLAYERS]>,
    runtime_clip: RichSlot<OwnedClipPlayer<RUNTIME_CLIP_IMAGE_CAPACITY>>,
    reserve: [u8; RICH_AUDIO_RESERVE_BYTES],
}

impl RichAudioResidency {
    const fn new() -> Self {
        Self {
            shared: Mutex::new(RefCell::new(RichAudioShared::new())),
            runtime_image: UnsafeCell::new([0; RUNTIME_CUE_IMAGE_CAPACITY]),
            service: RichSlot::new(),
            runtime_bgm: RichSlot::new(),
            runtime_sfx: RichSlot::new(),
            runtime_clip: RichSlot::new(),
            reserve: [0; RICH_AUDIO_RESERVE_BYTES],
        }
    }
}

#[repr(transparent)]
struct RichAudioArena(UnsafeCell<RichAudioResidency>);

// Safety: the residency owner serializes whole-arena reuse. Field access is
// allowed only in FullAudio, and CPU1 releases every field reference before an
// Offline acknowledgement permits the alternate owner to reuse these bytes.
unsafe impl Sync for RichAudioArena {}

#[unsafe(no_mangle)]
static AUDIO_RICH_RESIDENCY: RichAudioArena =
    RichAudioArena(UnsafeCell::new(RichAudioResidency::new()));

fn rich_residency() -> &'static RichAudioResidency {
    unsafe { &*AUDIO_RICH_RESIDENCY.0.get() }
}

fn rich_shared() -> &'static Mutex<RefCell<RichAudioShared>> {
    &rich_residency().shared
}

fn rich_runtime_image() -> *mut [u8; RUNTIME_CUE_IMAGE_CAPACITY] {
    rich_residency().runtime_image.get()
}
/// Rebuilds the rich-audio residency fields in the arena.
///
/// KOTO-0251: every slot is constructed inside its own `#[inline(never)]`
/// frame. The previous single-frame shape materialized *all* by-value
/// temporaries at once (service, cue players, 8 KiB clip player, plus
/// slot-sized `MaybeUninit` rewrites) on the caller's stack — KOTO-0172's
/// by-value-ctor lesson — and on hardware that transient punched through
/// `_stack_end` into the `.bss` tail (zeroing embassy's clock bookkeeping)
/// once the wifi-config image grew the statics. Splitting the constructors
/// bounds the transient depth to the largest single value instead of the sum,
/// on boot and on every post-Wi-Fi reconstruction alike. The redundant
/// `RichSlot::new()` pre-writes are dropped: the slots hold no `Drop` types
/// and each `init` overwrites its slot completely.
unsafe fn initialize_rich_residency() {
    init_rich_shared();
    let service_ready = init_rich_service();
    init_rich_runtime_bgm();
    for index in 0..RUNTIME_SFX_PLAYERS {
        init_rich_runtime_sfx_player(index);
    }
    init_rich_runtime_clip();
    RICH_SERVICE_READY.store(service_ready, Ordering::Release);
}

#[inline(never)]
fn init_rich_shared() {
    let residency = AUDIO_RICH_RESIDENCY.0.get();
    unsafe {
        core::ptr::addr_of_mut!((*residency).shared)
            .write(Mutex::new(RefCell::new(RichAudioShared::new())));
    }
}

#[inline(never)]
fn init_rich_service() -> bool {
    let residency = rich_residency();
    PicoAudioService::new(AudioPolicy::v0_default(), PwmBlockSink::new())
        .ok()
        .map(|service| unsafe { residency.service.init(service) })
        .is_some()
}

/// Idle-player templates, const-built into `.rodata` (XIP flash) so the
/// rebuild path copies flash -> arena directly (KOTO-0252). Passing
/// `T::new(..)` to `RichSlot::init` staged each player on the constructing
/// frame first; the ~18 KiB BGM temporary was the shell-path stack low-water
/// mark (`phase=176 at=shell`), reached from page-exit teardown on every
/// post-Wi-Fi rich-audio reconstruction.
static IDLE_BGM_PLAYER: RuntimeCuePlayer<RUNTIME_BGM_EVENTS_PER_TRACK> =
    RuntimeCuePlayer::new(PCM_OUTPUT_SAMPLE_RATE_HZ);
static IDLE_SFX_PLAYER: RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK> =
    RuntimeCuePlayer::new(PCM_OUTPUT_SAMPLE_RATE_HZ);
static IDLE_CLIP_PLAYER: OwnedClipPlayer<RUNTIME_CLIP_IMAGE_CAPACITY> = OwnedClipPlayer::new();

#[inline(never)]
fn init_rich_runtime_bgm() {
    let residency = rich_residency();
    unsafe {
        residency.runtime_bgm.init_from(&IDLE_BGM_PLAYER);
    }
}

/// One SFX player per frame: the whole-array temporary tripled the transient.
#[inline(never)]
fn init_rich_runtime_sfx_player(index: usize) {
    let residency = rich_residency();
    let array = residency
        .runtime_sfx
        .0
        .get()
        .cast::<[RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK>; RUNTIME_SFX_PLAYERS]>();
    unsafe {
        core::ptr::copy_nonoverlapping(
            &IDLE_SFX_PLAYER,
            core::ptr::addr_of_mut!((*array)[index]),
            1,
        );
    }
}

#[inline(never)]
fn init_rich_runtime_clip() {
    let residency = rich_residency();
    unsafe {
        residency.runtime_clip.init_from(&IDLE_CLIP_PLAYER);
    }
}

// Residency budgets: the permanent stream cell is dominated by the 8 KiB raw
// ring. The rich cell owns the service output, commands, and runtime staging.
const _: () = assert!(core::mem::size_of::<StreamAudioShared>() <= 9 * 1024);
const _: () = assert!(core::mem::align_of::<RichAudioArena>() >= 8);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<RichAudioArena>() == RICH_AUDIO_RESIDENCY_BYTES);
#[cfg(target_pointer_width = "32")]
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
    arena_guard_failures: AtomicU32,
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
            arena_guard_failures: AtomicU32::new(0),
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
        self.arena_guard_failures.store(0, Ordering::Relaxed);
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
            let mut shared = rich_shared().borrow_ref_mut(cs);
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
    TemporaryUnavailable,
}

#[must_use]
pub struct WifiResidencyArena {
    generation: u32,
}

/// Exclusive RP2040 storage loan for one TLS connection future.
/// Dropping this handle without returning it intentionally leaves audio
/// unavailable; restoration must pass through the zeroizing release method.
#[cfg(feature = "mcu-rp2040")]
#[must_use]
pub struct TlsAudioWorkspace {
    generation: u32,
}

#[cfg(feature = "mcu-rp2040")]
impl TlsAudioWorkspace {
    pub const CAPACITY: usize = PCM_RING_CAPACITY * core::mem::size_of::<i16>();

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        unsafe {
            core::slice::from_raw_parts_mut(
                (*AUDIO_STREAM_SHARED.0.get())
                    .raw
                    .samples
                    .as_mut_ptr()
                    .cast(),
                Self::CAPACITY,
            )
        }
    }

    pub fn try_start_future<'storage, F>(
        &'storage mut self,
        future: F,
    ) -> Result<
        crate::firmware::arena_future::ArenaFuture<'storage>,
        crate::firmware::arena_future::ArenaFutureError,
    >
    where
        F: core::future::Future<Output = ()> + 'storage,
    {
        crate::firmware::arena_future::ArenaFuture::try_new(self.bytes(), future)
    }

    /// Zero-initializes the whole 8 KiB PCM loan and returns it as the
    /// dedicated TLS crypto stack. The P-256 CertVerify peak (~5.1 KiB) plus
    /// nested interrupt frames must fit here: a stack whose peak reached the
    /// base would push interrupt frames BELOW it into the live CYW43/net-stack
    /// arena, wedging the transport (KOTO-0245 wire diagnosis). The adjacent
    /// lower region is now fetch-local audio scratch rather than the net
    /// stack. The crypto implementation must retain explicit interrupt
    /// headroom within this fixed SRAM-neutral loan.
    pub fn crypto_stack(&mut self) -> &mut [u8] {
        let bytes = self.bytes();
        for byte in bytes.iter_mut() {
            byte.write(0);
        }
        // SAFETY: every byte was just initialized above.
        unsafe { &mut *(core::ptr::from_mut(bytes) as *mut [u8]) }
    }

    /// Reads back the crypto-stack bytes (after the exchange, before release)
    /// so the high-water mark can be recovered. The region was initialized by
    /// [`Self::crypto_stack`].
    pub fn crypto_stack_readback(&mut self) -> &[u8] {
        let bytes = self.bytes();
        // SAFETY: initialized by the preceding `crypto_stack` call.
        unsafe { &*(core::ptr::from_ref(bytes) as *const [u8]) }
    }
}

/// KOTO-0245 receive-record buffer. The controlled server handshake flight is
/// 767 bytes; 1,792 bytes retains bounded overhead while freeing more of the
/// adjacent audio scratch for the extended crypto stack. Oversized records
/// fail closed as `Tls`.
#[cfg(feature = "mcu-rp2040")]
pub const TLS_RECORD_RX_BYTES: usize = 1792;
/// Transmit-record buffer for the bounded GET head; served from the quiesced
/// audio DMA ring.
#[cfg(feature = "mcu-rp2040")]
pub const TLS_RECORD_TX_BYTES: usize = 1024;

#[cfg(feature = "mcu-rp2040")]
const _: () = assert!(TLS_RECORD_TX_BYTES <= DMA_RING_SAMPLES * core::mem::size_of::<u32>());

/// Lends the quiesced audio DMA ring as the TLS transmit-record buffer. Sound
/// only while stream audio is quiesced (workspace claimed): the DMA is aborted
/// and nothing else touches this storage during a fetch.
///
/// # Safety
/// Caller holds the TLS/audio workspace claim (stream quiesced) for the whole
/// borrow, on CPU0.
#[cfg(all(feature = "mcu-rp2040", feature = "app_fetch_https"))]
pub unsafe fn tls_record_tx_bytes() -> &'static mut [u8] {
    let ring = unsafe { &mut *core::ptr::addr_of_mut!(AUDIO_DMA_RING) };
    // SAFETY: [u32; N] reinterpreted as bytes; the DMA is aborted during a
    // fetch so this is exclusively ours. Truncated to the TX record size.
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::from_mut(&mut ring.0).cast::<u8>(),
            TLS_RECORD_TX_BYTES,
        )
    };
    bytes.fill(0);
    bytes
}

/// Claims the globally fenced PCM workspace without retaining a
/// `PicoAudioBackend` reference in the network future. The residency owner and
/// one-owner CAS are checked at the instant ownership moves.
#[cfg(feature = "mcu-rp2040")]
pub(crate) fn claim_shared_tls_audio_workspace() -> Result<TlsAudioWorkspace, TransitionError> {
    let generation = critical_section::with(|cs| {
        let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
        (owner.state() == ResidencyState::TlsExclusive).then(|| owner.token().generation())
    })
    .ok_or(TransitionError::InvalidState)?;
    TLS_AUDIO_WORKSPACE_CLAIMED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| TransitionError::InvalidState)?;
    Ok(TlsAudioWorkspace { generation })
}

/// Returns a network-owned workspace, overwrites every byte, and starts the
/// normal CPU1 stream reconstruction. The audio facade synchronizes to the new
/// residency generation on its next service pass.
#[cfg(feature = "mcu-rp2040")]
pub(crate) fn release_shared_tls_audio_workspace(
    mut workspace: TlsAudioWorkspace,
) -> Result<u32, TransitionError> {
    let valid = TLS_AUDIO_WORKSPACE_CLAIMED.load(Ordering::Acquire)
        && critical_section::with(|cs| {
            let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
            owner.state() == ResidencyState::TlsExclusive
                && owner.token().generation() == workspace.generation
        });
    if !valid {
        return Err(TransitionError::StaleToken);
    }
    crate::firmware::arena_future::zeroize_arena(workspace.bytes());
    with_stream_audio(StreamAudioShared::reset);
    let token = critical_section::with(|cs| {
        AUDIO_RESIDENCY_OWNER
            .borrow_ref_mut(cs)
            .begin_stream_restore_after_tls()
    })?;
    TLS_AUDIO_WORKSPACE_CLAIMED.store(false, Ordering::Release);
    WORKER_CONTROL_GENERATION.store(token.generation(), Ordering::Relaxed);
    WORKER_CONTROL.store(WORKER_CONTROL_ACTIVATE_STREAM, Ordering::Release);
    Ok(token.generation())
}

impl WifiResidencyArena {
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        unsafe {
            core::slice::from_raw_parts_mut(
                AUDIO_RICH_RESIDENCY.0.get().cast::<MaybeUninit<u8>>(),
                RICH_AUDIO_RESIDENCY_BYTES,
            )
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioRequestError {
    TemporaryUnavailable,
    StaleHandle,
    QueueFull,
}

#[derive(Clone, Copy)]
pub(crate) enum LoadedAudioImage {
    Cue { len: usize, bgm: bool },
    Clip { len: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RichImageError {
    TemporaryUnavailable,
    Busy,
}

pub(crate) fn try_load_rich_audio_image(
    load: impl FnOnce(&mut [u8; RUNTIME_CUE_IMAGE_CAPACITY]) -> Option<LoadedAudioImage>,
) -> Result<bool, RichImageError> {
    let rich_available = critical_section::with(|cs| {
        let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
        owner.rich_audio_available(owner.token())
    });
    if !rich_available {
        return Err(RichImageError::TemporaryUnavailable);
    }
    let claimed = critical_section::with(|cs| {
        let mut shared = rich_shared().borrow_ref_mut(cs);
        if shared.runtime_image_busy {
            return false;
        }
        shared.runtime_image_busy = true;
        true
    });
    if !claimed {
        return Err(RichImageError::Busy);
    }

    // The busy claim prevents CPU0 from lending this slot again. CPU1 can only
    // read it after the command is queued below, after this mutable view ends.
    let image = unsafe { &mut *rich_runtime_image() };
    let loaded = load(image);

    let queued = critical_section::with(|cs| {
        let mut shared = rich_shared().borrow_ref_mut(cs);
        let command = match loaded {
            Some(LoadedAudioImage::Cue { len, bgm })
                if len > 0 && len <= RUNTIME_CUE_IMAGE_CAPACITY =>
            {
                shared.runtime_image_len = len;
                if bgm {
                    AudioCommand::PlayRuntimeBgm
                } else {
                    AudioCommand::PlayRuntimeSfx
                }
            }
            Some(LoadedAudioImage::Clip { len })
                if len > 0 && len <= RUNTIME_CLIP_IMAGE_CAPACITY =>
            {
                shared.runtime_image_len = len;
                AudioCommand::PlayRuntimeClip
            }
            _ => {
                shared.runtime_image_len = 0;
                shared.runtime_image_busy = false;
                return false;
            }
        };
        if shared.commands.push(command) {
            true
        } else {
            shared.runtime_image_len = 0;
            shared.runtime_image_busy = false;
            false
        }
    });
    if !queued {
        AudioAtomicStats::inc(&AUDIO_STATS.command_drops, 1);
        AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
    }
    Ok(queued)
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
    pub residency_state: ResidencyState,
    pub residency_generation: u32,
    pub worker_offline_generation: u32,
    pub worker_online_generation: u32,
    pub worker_rich_active: bool,
    pub transition_failures: u32,
    pub arena_guard_failures: u32,
}

/// CPU0 facade over the CPU1 audio worker.
pub struct PicoAudioBackend {
    residency_token: ResidencyToken,
}

impl PicoAudioBackend {
    pub fn spawn_cpu1(
        core1: Peri<'static, CORE1>,
        stack: &'static mut Stack<AUDIO_CORE1_STACK_BYTES>,
        pwm: Pwm<'static>,
    ) -> Self {
        with_stream_audio(StreamAudioShared::reset);
        critical_section::with(|cs| {
            *AUDIO_RESIDENCY_OWNER.borrow_ref_mut(cs) = AudioResidencyOwner::new();
        });
        TLS_AUDIO_WORKSPACE_CLAIMED.store(false, Ordering::Release);
        AUDIO_STATS.reset();
        // KOTO-0186: paint the worker stack while core1 is still idle so the
        // cross-core `core1_stack_free_min` scan can measure the worst-case
        // worker frame depth after the LTO in-place-reset fix.
        paint_core1_stack(stack);
        // Construct the ~3 KiB service on CPU0 so it never consumes the
        // bounded CPU1 worker stack. The same routine reconstructs these
        // fields after the Wi-Fi owner releases the arena.
        unsafe { initialize_rich_residency() };
        spawn_core1(core1, stack, move || -> ! { run_audio_worker(pwm) });
        let residency_token =
            critical_section::with(|cs| AUDIO_RESIDENCY_OWNER.borrow_ref(cs).token());
        Self { residency_token }
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
        if !self.stream_request_available() {
            return Err(PcmSubmitError::TemporaryUnavailable);
        }
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
        with_stream_audio(|shared| {
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
        if !self.stream_request_available() {
            return Err(PcmSubmitError::TemporaryUnavailable);
        }
        if sample_rate_hz != self.sample_rate_hz() {
            self.record_unsupported();
            return Err(PcmSubmitError::Unsupported);
        }

        let mut accepted = 0u32;
        let mut dropped = 0u32;
        with_stream_audio(|shared| {
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

    /// Number of mono frames the host streamer can enqueue without dropping.
    pub fn pcm_free_frames(&self) -> usize {
        if !self.stream_request_available() {
            return 0;
        }
        with_stream_audio(|shared| PCM_RING_CAPACITY.saturating_sub(shared.raw.len()))
    }

    /// Tells the CPU1 diagnostics whether an empty raw ring is a stream gap.
    pub fn set_pcm_stream_active(&mut self, active: bool) {
        PCM_STREAM_ACTIVE.store(active, Ordering::Relaxed);
    }

    /// Starts (or replaces) the looping BGM sequence for a routed cue.
    pub fn play_bgm_cue(
        &mut self,
        sequence: &'static PolyphonicSequence<'static>,
    ) -> Result<(), AudioRequestError> {
        self.enqueue_rich_command(AudioCommand::PlayBgm(sequence))
    }

    /// Plays a one-shot SFX sequence for a routed cue.
    pub fn play_sfx_cue(
        &mut self,
        sequence: &'static Sequence<'static>,
    ) -> Result<(), AudioRequestError> {
        self.enqueue_rich_command(AudioCommand::PlaySfx(sequence))
    }

    /// Copies one PSRAM-loaded pointer-free cue image to the CPU1 staging slot.
    pub fn play_runtime_cue(&mut self, image: &[u8], bgm: bool) -> bool {
        if !self.rich_request_available()
            || image.is_empty()
            || image.len() > RUNTIME_CUE_IMAGE_CAPACITY
        {
            self.record_drop();
            return false;
        }
        let accepted = critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            if shared.runtime_image_busy {
                return false;
            }
            let runtime_image = unsafe { &mut *rich_runtime_image() };
            runtime_image[..image.len()].copy_from_slice(image);
            shared.runtime_image_len = image.len();
            shared.runtime_image_busy = true;
            if !shared.commands.push(if bgm {
                AudioCommand::PlayRuntimeBgm
            } else {
                AudioCommand::PlayRuntimeSfx
            }) {
                shared.runtime_image_busy = false;
                shared.runtime_image_len = 0;
                return false;
            }
            true
        });
        if !accepted {
            AudioAtomicStats::inc(&AUDIO_STATS.command_drops, 1);
            self.record_drop();
        }
        accepted
    }

    /// Copies one runtime-ready KACL image into CPU1-owned playback storage.
    pub fn play_runtime_clip(&mut self, image: &[u8]) -> bool {
        if !self.rich_request_available()
            || image.is_empty()
            || image.len() > RUNTIME_CLIP_IMAGE_CAPACITY
        {
            self.record_drop();
            return false;
        }
        let accepted = critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            if shared.runtime_image_busy {
                return false;
            }
            let runtime_image = unsafe { &mut *rich_runtime_image() };
            runtime_image[..image.len()].copy_from_slice(image);
            shared.runtime_image_len = image.len();
            shared.runtime_image_busy = true;
            if !shared.commands.push(AudioCommand::PlayRuntimeClip) {
                shared.runtime_image_busy = false;
                shared.runtime_image_len = 0;
                return false;
            }
            true
        });
        if !accepted {
            AudioAtomicStats::inc(&AUDIO_STATS.command_drops, 1);
            self.record_drop();
        }
        accepted
    }

    pub fn stop_bgm(&mut self) {
        if !self.rich_request_available() {
            return;
        }
        critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            // StopAll outranks StopBgm in the single high-priority slot.
            if !matches!(shared.high_priority, AudioCommand::StopAll) {
                shared.high_priority = AudioCommand::StopBgm;
            }
        });
    }

    pub fn stop(&mut self) {
        PCM_STREAM_ACTIVE.store(false, Ordering::Relaxed);
        if !self.rich_request_available() {
            return;
        }
        critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            shared.high_priority = AudioCommand::StopAll;
        });
    }

    pub fn begin_wifi_quiesce(&mut self) -> Result<u32, TransitionError> {
        let token = critical_section::with(|cs| {
            let mut owner = AUDIO_RESIDENCY_OWNER.borrow_ref_mut(cs);
            owner.begin_wifi()
        })?;
        self.residency_token = token;
        critical_section::with(|cs| {
            rich_shared().borrow_ref_mut(cs).commands.clear();
        });
        WORKER_CONTROL_GENERATION.store(token.generation(), Ordering::Relaxed);
        WORKER_CONTROL.store(WORKER_CONTROL_QUIESCE_RICH, Ordering::Release);
        Ok(token.generation())
    }

    pub fn service(&mut self) {
        // A network-owned TLS workspace starts restoration without borrowing
        // this facade. Adopt only that explicitly published owner generation;
        // all other transitions remain token-driven by facade methods.
        if let Some(token) = critical_section::with(|cs| {
            let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
            (owner.state() == ResidencyState::RestoringStreamAfterTls
                && owner.token().generation() != self.residency_token.generation())
            .then(|| owner.token())
        }) {
            self.residency_token = token;
        }
        let offline_acknowledged = WORKER_RICH_OFFLINE_GENERATION.load(Ordering::Acquire)
            == self.residency_token.generation();
        let online_acknowledged = WORKER_RICH_ONLINE_GENERATION.load(Ordering::Acquire)
            == self.residency_token.generation();
        let stream_offline_acknowledged = WORKER_STREAM_OFFLINE_GENERATION.load(Ordering::Acquire)
            == self.residency_token.generation();
        let stream_online_acknowledged = WORKER_STREAM_ONLINE_GENERATION.load(Ordering::Acquire)
            == self.residency_token.generation();
        critical_section::with(|cs| {
            let mut owner = AUDIO_RESIDENCY_OWNER.borrow_ref_mut(cs);
            if owner.state() == ResidencyState::QuiescingAudio && offline_acknowledged {
                let _ = owner.mark_audio_offline(self.residency_token);
            } else if owner.state() == ResidencyState::Offline && online_acknowledged {
                let _ = owner.activate_full_audio(self.residency_token);
            } else if owner.state() == ResidencyState::QuiescingStreamForTls
                && stream_offline_acknowledged
            {
                let _ = owner.activate_tls_exclusive(self.residency_token);
            } else if owner.state() == ResidencyState::RestoringStreamAfterTls
                && stream_online_acknowledged
            {
                let _ = owner.activate_stream_after_tls(self.residency_token);
            }
        });
    }

    /// Requests the RP2040 worker/DMA ownership fence required before TLS may
    /// reuse permanent stream-audio storage. Completion is observable as
    /// [`ResidencyState::TlsExclusive`] after repeated [`Self::service`] calls.
    #[cfg(feature = "mcu-rp2040")]
    pub fn begin_tls_audio_quiesce(&mut self) -> Result<u32, TransitionError> {
        let token = critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER
                .borrow_ref_mut(cs)
                .begin_tls_exclusive()
        })?;
        self.residency_token = token;
        PCM_STREAM_ACTIVE.store(false, Ordering::Release);
        WORKER_CONTROL_GENERATION.store(token.generation(), Ordering::Relaxed);
        WORKER_CONTROL.store(WORKER_CONTROL_QUIESCE_STREAM, Ordering::Release);
        Ok(token.generation())
    }

    /// Claims the stopped PCM sample region for exactly one TLS future.
    /// Callers must wait for `TlsExclusive`; transitional states never expose
    /// the bytes even if the worker has started quiescing.
    #[cfg(feature = "mcu-rp2040")]
    pub fn claim_tls_audio_workspace(&mut self) -> Result<TlsAudioWorkspace, TransitionError> {
        let valid = critical_section::with(|cs| {
            let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
            owner.state() == ResidencyState::TlsExclusive
                && owner.token().generation() == self.residency_token.generation()
        });
        if !valid {
            return Err(TransitionError::InvalidState);
        }
        claim_shared_tls_audio_workspace()
    }

    /// Drops the TLS ownership epoch, overwrites every loaned byte, and only
    /// then asks CPU1 to reconstruct the PCM/DMA path. Rust's borrow of
    /// `workspace.bytes()` prevents this call while an [`ArenaFuture`](crate::firmware::arena_future::ArenaFuture)
    /// still occupies the storage.
    #[cfg(feature = "mcu-rp2040")]
    pub fn release_tls_audio_workspace(
        &mut self,
        workspace: TlsAudioWorkspace,
    ) -> Result<u32, TransitionError> {
        if workspace.generation != self.residency_token.generation()
            || self.residency_state() != ResidencyState::TlsExclusive
        {
            return Err(TransitionError::StaleToken);
        }
        let generation = release_shared_tls_audio_workspace(workspace)?;
        self.residency_token =
            critical_section::with(|cs| AUDIO_RESIDENCY_OWNER.borrow_ref(cs).token());
        Ok(generation)
    }

    pub fn activate_wifi_stream_audio(&mut self) -> Result<WifiResidencyArena, TransitionError> {
        critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER
                .borrow_ref_mut(cs)
                .activate_wifi(self.residency_token)
        })?;
        Ok(WifiResidencyArena {
            generation: self.residency_token.generation(),
        })
    }

    pub fn begin_full_audio_quiesce(
        &mut self,
        mut arena: WifiResidencyArena,
    ) -> Result<WifiResidencyArena, TransitionError> {
        // `arena.generation` is the Wi-Fi lifecycle generation installed when
        // rich audio first lent these bytes to the radio. An HTTPS transaction
        // legitimately advances the audio owner generation twice while that
        // same (linear, non-Copy) arena remains owned by WifiRuntime: once for
        // TLS exclusion and once for stream restoration. Consequently the
        // lifecycle generation must not be compared with the current audio
        // transition token here. `begin_full_audio` still proves that the
        // global owner has reached WifiStreamAudio, and consuming the unique
        // arena handle proves that the Wi-Fi runtime returned the bytes.
        let token = critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER.borrow_ref_mut(cs).begin_full_audio()
        })?;
        self.residency_token = token;
        // Rebase the returned arena onto the new reverse-transition token so
        // `complete_wifi_quiesce` retains its stale-token fence.
        arena.generation = token.generation();
        Ok(arena)
    }

    /// Completes the Wi-Fi teardown after its runner and all arena users have
    /// joined. The worker remains on permanent PCM/DMA state until it acquires
    /// the reconstructed rich fields and publishes the matching generation.
    pub fn complete_wifi_quiesce(
        &mut self,
        arena: WifiResidencyArena,
    ) -> Result<(), TransitionError> {
        if arena.generation != self.residency_token.generation() {
            AudioAtomicStats::inc(&AUDIO_STATS.arena_guard_failures, 1);
            return Err(TransitionError::StaleToken);
        }
        critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER
                .borrow_ref_mut(cs)
                .mark_wifi_offline(self.residency_token)
        })?;
        unsafe { initialize_rich_residency() };
        WORKER_CONTROL_GENERATION.store(self.residency_token.generation(), Ordering::Relaxed);
        WORKER_CONTROL.store(WORKER_CONTROL_ACTIVATE_RICH, Ordering::Release);
        Ok(())
    }

    pub fn residency_state(&self) -> ResidencyState {
        critical_section::with(|cs| AUDIO_RESIDENCY_OWNER.borrow_ref(cs).state())
    }

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
        let buffer_level = if TLS_AUDIO_WORKSPACE_CLAIMED.load(Ordering::Acquire) {
            0
        } else {
            with_stream_audio(|shared| shared.raw.len() as u32)
        };
        let residency = critical_section::with(|cs| *AUDIO_RESIDENCY_OWNER.borrow_ref(cs));
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
            residency_state: residency.state(),
            residency_generation: residency.token().generation(),
            worker_offline_generation: WORKER_RICH_OFFLINE_GENERATION.load(Ordering::Acquire),
            worker_online_generation: WORKER_RICH_ONLINE_GENERATION.load(Ordering::Acquire),
            worker_rich_active: WORKER_RICH_ACTIVE.load(Ordering::Acquire),
            transition_failures: residency.transition_failures(),
            arena_guard_failures: AUDIO_STATS.arena_guard_failures.load(Ordering::Relaxed),
        }
    }

    fn enqueue_rich_command(&mut self, command: AudioCommand) -> Result<(), AudioRequestError> {
        let availability = critical_section::with(|cs| {
            let owner = *AUDIO_RESIDENCY_OWNER.borrow_ref(cs);
            if owner.rich_audio_available(self.residency_token) {
                Ok(())
            } else if owner.token().generation() != self.residency_token.generation() {
                Err(AudioRequestError::StaleHandle)
            } else {
                Err(AudioRequestError::TemporaryUnavailable)
            }
        });
        availability?;
        let accepted = critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            shared.commands.push(command)
        });
        if !accepted {
            AudioAtomicStats::inc(&AUDIO_STATS.command_drops, 1);
            AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
            return Err(AudioRequestError::QueueFull);
        }
        Ok(())
    }

    fn rich_request_available(&self) -> bool {
        critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER
                .borrow_ref(cs)
                .rich_audio_available(self.residency_token)
        })
    }

    fn stream_request_available(&self) -> bool {
        critical_section::with(|cs| {
            AUDIO_RESIDENCY_OWNER
                .borrow_ref(cs)
                .stream_audio_available(self.residency_token)
        })
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
fn run_audio_worker(mut pwm: Pwm<'static>) -> ! {
    let residency = rich_residency();
    let mut service = RICH_SERVICE_READY
        .load(Ordering::Acquire)
        .then(|| unsafe { residency.service.assume_init_mut() });
    let runtime_bgm = unsafe { residency.runtime_bgm.assume_init_mut() };
    let runtime_sfx = unsafe { residency.runtime_sfx.assume_init_mut() };
    let runtime_clip = unsafe { residency.runtime_clip.assume_init_mut() };
    configure_pcm_output(&mut pwm);
    if let Some(service) = service.as_deref_mut() {
        let _ = service.start();
    }

    let ring = unsafe { &mut *core::ptr::addr_of_mut!(AUDIO_DMA_RING) };
    let silence = duty_word(0);
    for slot in ring.0.iter_mut() {
        *slot = silence;
    }
    setup_dma_pacing_timer();
    arm_audio_dma(ring);
    WORKER_RICH_ACTIVE.store(service.is_some(), Ordering::Release);
    WORKER_RICH_OFFLINE_GENERATION.store(0, Ordering::Release);
    WORKER_STREAM_ACTIVE.store(true, Ordering::Release);
    WORKER_STREAM_OFFLINE_GENERATION.store(0, Ordering::Release);
    WORKER_STREAM_ONLINE_GENERATION.store(0, Ordering::Release);

    let mut worker = PicoAudioWorker {
        _pwm: pwm,
        service,
        runtime_bgm: Some(runtime_bgm),
        runtime_sfx: Some(runtime_sfx),
        runtime_clip: Some(runtime_clip),
        runtime_sfx_cursor: 0,
        pending_sources: 0,
        underrun_latched: false,
        raw_underrun_latched: false,
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
    service: Option<&'static mut PicoAudioService>,
    runtime_bgm: Option<&'static mut RuntimeCuePlayer<RUNTIME_BGM_EVENTS_PER_TRACK>>,
    runtime_sfx:
        Option<&'static mut [RuntimeCuePlayer<RUNTIME_SFX_EVENTS_PER_TRACK>; RUNTIME_SFX_PLAYERS]>,
    runtime_sfx_cursor: usize,
    runtime_clip: Option<&'static mut OwnedClipPlayer<RUNTIME_CLIP_IMAGE_CAPACITY>>,
    /// Active + queued service sources after the last command/tick.
    pending_sources: u32,
    underrun_latched: bool,
    raw_underrun_latched: bool,
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

            self.drain_transition_command();
            if WORKER_RICH_ACTIVE.load(Ordering::Acquire) {
                self.drain_commands();
                if WORKER_RICH_ACTIVE.load(Ordering::Acquire) {
                    self.render_ahead();
                }
            }
            if WORKER_STREAM_ACTIVE.load(Ordering::Acquire) {
                self.fill_dma_ring();
            }

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

    fn drain_transition_command(&mut self) {
        let control = WORKER_CONTROL.load(Ordering::Acquire);
        if control == WORKER_CONTROL_NONE {
            return;
        }
        let generation = WORKER_CONTROL_GENERATION.load(Ordering::Relaxed);
        WORKER_CONTROL.store(WORKER_CONTROL_NONE, Ordering::Release);
        if control == WORKER_CONTROL_QUIESCE_RICH {
            self.apply_command(AudioCommand::QuiesceRich(generation));
        } else if control == WORKER_CONTROL_ACTIVATE_RICH {
            self.activate_rich(generation);
        } else if control == WORKER_CONTROL_QUIESCE_STREAM {
            self.quiesce_stream(generation);
        } else if control == WORKER_CONTROL_ACTIVATE_STREAM {
            self.activate_stream(generation);
        }
    }

    fn quiesce_stream(&mut self, generation: u32) {
        PCM_STREAM_ACTIVE.store(false, Ordering::Release);
        pac::DMA
            .chan_abort()
            .modify(|w| w.set_chan_abort(1 << AUDIO_DMA_CH));
        let channel = pac::DMA.ch(AUDIO_DMA_CH);
        while channel.ctrl_trig().read().busy() {}

        // Leave both PWM outputs at the silent midpoint after DMA has stopped.
        let midpoint = (PCM_PWM_TOP / 2).max(1);
        pac::PWM.ch(5).cc().write(|w| {
            w.set_a(midpoint);
            w.set_b(midpoint);
        });
        with_stream_audio(StreamAudioShared::reset);
        self.write_pos = 0;
        self.sent_base = 0;
        self.raw_underrun_latched = false;
        WORKER_STREAM_ACTIVE.store(false, Ordering::Release);
        WORKER_STREAM_OFFLINE_GENERATION.store(generation, Ordering::Release);
    }

    fn activate_stream(&mut self, generation: u32) {
        with_stream_audio(StreamAudioShared::reset);
        let ring = unsafe { &mut *core::ptr::addr_of_mut!(AUDIO_DMA_RING) };
        let silence = duty_word(0);
        for slot in ring.0.iter_mut() {
            *slot = silence;
        }
        self.write_pos = DMA_RING_SAMPLES as u64;
        self.sent_base = 0;
        self.raw_underrun_latched = false;
        setup_dma_pacing_timer();
        arm_audio_dma(ring);
        WORKER_STREAM_ACTIVE.store(true, Ordering::Release);
        WORKER_STREAM_ONLINE_GENERATION.store(generation, Ordering::Release);
    }

    fn activate_rich(&mut self, generation: u32) {
        let residency = rich_residency();
        self.service = RICH_SERVICE_READY
            .load(Ordering::Acquire)
            .then(|| unsafe { residency.service.assume_init_mut() });
        self.runtime_bgm = Some(unsafe { residency.runtime_bgm.assume_init_mut() });
        self.runtime_sfx = Some(unsafe { residency.runtime_sfx.assume_init_mut() });
        self.runtime_clip = Some(unsafe { residency.runtime_clip.assume_init_mut() });
        self.runtime_sfx_cursor = 0;
        self.pending_sources = 0;
        self.underrun_latched = false;
        if let Some(service) = self.service.as_deref_mut() {
            let _ = service.start();
        }
        WORKER_RICH_ACTIVE.store(true, Ordering::Release);
        WORKER_RICH_ONLINE_GENERATION.store(generation, Ordering::Release);
    }

    fn drain_commands(&mut self) {
        let high_command = critical_section::with(|cs| {
            let mut shared = rich_shared().borrow_ref_mut(cs);
            let high_command = shared.high_priority;
            shared.high_priority = AudioCommand::None;
            high_command
        });
        self.apply_command(high_command);
        // The queue is bounded (16), so this pass-local drain is bounded too.
        while let Some(command) =
            critical_section::with(|cs| rich_shared().borrow_ref_mut(cs).commands.pop())
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
            let out_level = critical_section::with(|cs| rich_shared().borrow_ref(cs).out.len());
            if out_level + BLOCK_FRAMES > OUT_RING_CAPACITY {
                break;
            }
            let Some(service) = self.service.as_deref_mut() else {
                return;
            };
            let _ = service.tick();
            while service.poll_audio_event().is_some() {}
            self.refresh_source_stats();
            rendered += 1;
        }
    }

    /// Advances the writer up to one full ring ahead of the DMA read position,
    /// mixing the service output and the raw PCM stream into duty words.
    fn fill_dma_ring(&mut self) {
        let channel = pac::DMA.ch(AUDIO_DMA_CH);
        let remaining = dma_transfer_count(&channel);
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
            let rich_active = WORKER_RICH_ACTIVE.load(Ordering::Acquire);
            let (seq_len, raw_len) = critical_section::with(|cs| {
                let stream = unsafe { &mut *AUDIO_STREAM_SHARED.0.get() };
                let mut seq_len = 0;
                if rich_active {
                    let mut rich = rich_shared().borrow_ref_mut(cs);
                    while seq_len < needed {
                        match rich.out.pop() {
                            Some(sample) => {
                                seq_buf[seq_len] = sample;
                                seq_len += 1;
                            }
                            None => break,
                        }
                    }
                }
                let mut raw_len = 0;
                while raw_len < needed {
                    match stream.raw.pop() {
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
            if raw_len > 0 {
                self.raw_underrun_latched = false;
            }
            if raw_len < needed
                && PCM_STREAM_ACTIVE.load(Ordering::Relaxed)
                && !self.raw_underrun_latched
            {
                AudioAtomicStats::inc(&AUDIO_STATS.underruns, 1);
                self.raw_underrun_latched = true;
            }

            for index in 0..needed {
                let seq = if index < seq_len { seq_buf[index] } else { 0 };
                let mut mixed = i32::from(seq);
                if let Some(player) = self.runtime_bgm.as_deref_mut() {
                    if let DecodeResult::Sample(sample) = player.next_sample() {
                        mixed = mixed.saturating_add(scale_runtime_sample(sample, 150));
                    }
                }
                if let Some(players) = self.runtime_sfx.as_deref_mut() {
                    for player in players {
                        if let DecodeResult::Sample(sample) = player.next_sample() {
                            mixed = mixed.saturating_add(scale_runtime_sample(sample, 200));
                        }
                    }
                }
                if let Some(player) = self.runtime_clip.as_deref_mut() {
                    if let DecodeResult::Sample(sample) = player.next_sample() {
                        mixed = mixed.saturating_add(scale_runtime_sample(sample, 200));
                    }
                }
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
                if self
                    .service
                    .as_deref_mut()
                    .is_some_and(|service| service.play_bgm_sequence(*sequence).is_ok())
                {
                    AudioAtomicStats::inc(&AUDIO_STATS.bgm_starts, 1);
                } else {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
                self.refresh_source_stats();
            }
            AudioCommand::PlaySfx(sequence) => {
                if !self
                    .service
                    .as_deref_mut()
                    .is_some_and(|service| service.play_sequence(*sequence).is_ok())
                {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
                self.refresh_source_stats();
            }
            AudioCommand::PlayRuntimeBgm => {
                let ok = critical_section::with(|cs| {
                    let mut shared = rich_shared().borrow_ref_mut(cs);
                    let len = shared.runtime_image_len;
                    let result = self.runtime_bgm.as_deref_mut().is_some_and(|player| {
                        player
                            .play_image(&unsafe { &*rich_runtime_image() }[..len])
                            .is_ok()
                    });
                    shared.runtime_image_len = 0;
                    shared.runtime_image_busy = false;
                    result
                });
                if ok {
                    AudioAtomicStats::inc(&AUDIO_STATS.bgm_starts, 1);
                } else {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
            }
            AudioCommand::PlayRuntimeSfx => {
                let Some(players) = self.runtime_sfx.as_deref_mut() else {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                    return;
                };
                let slot = players
                    .iter()
                    .position(|player| !player.is_playing())
                    .unwrap_or(self.runtime_sfx_cursor);
                let ok = critical_section::with(|cs| {
                    let mut shared = rich_shared().borrow_ref_mut(cs);
                    let len = shared.runtime_image_len;
                    let result =
                        players[slot].play_image(&unsafe { &*rich_runtime_image() }[..len]);
                    shared.runtime_image_len = 0;
                    shared.runtime_image_busy = false;
                    result.is_ok()
                });
                self.runtime_sfx_cursor = (slot + 1) % players.len();
                if !ok {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
            }
            AudioCommand::PlayRuntimeClip => {
                let ok = critical_section::with(|cs| {
                    let mut shared = rich_shared().borrow_ref_mut(cs);
                    let len = shared.runtime_image_len;
                    let result = self.runtime_clip.as_deref_mut().is_some_and(|player| {
                        player
                            .play_image(
                                &unsafe { &*rich_runtime_image() }[..len],
                                AudioLimits::v0_default(),
                            )
                            .is_ok()
                    });
                    shared.runtime_image_len = 0;
                    shared.runtime_image_busy = false;
                    result
                });
                if !ok {
                    AudioAtomicStats::inc(&AUDIO_STATS.drops, 1);
                }
            }
            AudioCommand::StopBgm => {
                if let Some(service) = self.service.as_deref_mut() {
                    let _ = service.stop_bgm();
                }
                if let Some(player) = self.runtime_bgm.as_deref_mut() {
                    player.stop();
                }
                AudioAtomicStats::inc(&AUDIO_STATS.bgm_stops, 1);
                self.refresh_source_stats();
            }
            AudioCommand::StopAll => {
                // Full bounded reset: sources, mixer, events, and both rings.
                if let Some(service) = self.service.as_deref_mut() {
                    let _ = service.reset();
                    let _ = service.start();
                }
                if let Some(player) = self.runtime_bgm.as_deref_mut() {
                    player.stop();
                }
                if let Some(players) = self.runtime_sfx.as_deref_mut() {
                    for player in players {
                        player.stop();
                    }
                }
                if let Some(player) = self.runtime_clip.as_deref_mut() {
                    player.stop();
                }
                critical_section::with(|cs| {
                    unsafe { &mut *AUDIO_STREAM_SHARED.0.get() }.raw.clear();
                    rich_shared().borrow_ref_mut(cs).out.clear();
                });
                AudioAtomicStats::inc(&AUDIO_STATS.bgm_stops, 1);
                self.refresh_source_stats();
            }
            AudioCommand::QuiesceRich(generation) => {
                if let Some(service) = self.service.as_deref_mut() {
                    let _ = service.reset();
                }
                if let Some(player) = self.runtime_bgm.as_deref_mut() {
                    player.stop();
                }
                if let Some(players) = self.runtime_sfx.as_deref_mut() {
                    for player in players {
                        player.stop();
                    }
                }
                if let Some(player) = self.runtime_clip.as_deref_mut() {
                    player.stop();
                }
                critical_section::with(|cs| rich_shared().borrow_ref_mut(cs).reset());
                self.pending_sources = 0;
                if let Some(service) = self.service.take() {
                    unsafe { core::ptr::drop_in_place(service) };
                }
                if let Some(player) = self.runtime_bgm.take() {
                    unsafe { core::ptr::drop_in_place(player) };
                }
                if let Some(players) = self.runtime_sfx.take() {
                    unsafe { core::ptr::drop_in_place(players) };
                }
                if let Some(player) = self.runtime_clip.take() {
                    unsafe { core::ptr::drop_in_place(player) };
                }
                RICH_SERVICE_READY.store(false, Ordering::Release);
                AUDIO_STATS.active_bgm_voices.store(0, Ordering::Relaxed);
                AUDIO_STATS.active_sfx_voices.store(0, Ordering::Relaxed);
                WORKER_RICH_ACTIVE.store(false, Ordering::Release);
                WORKER_RICH_OFFLINE_GENERATION.store(generation, Ordering::Release);
            }
        }
    }

    fn refresh_source_stats(&mut self) {
        let Some(service) = self.service.as_deref() else {
            self.pending_sources = 0;
            AUDIO_STATS.active_bgm_voices.store(0, Ordering::Relaxed);
            AUDIO_STATS.active_sfx_voices.store(0, Ordering::Relaxed);
            return;
        };
        let snapshot = service.counter_snapshot();
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

fn scale_runtime_sample(sample: i16, gain: u16) -> i32 {
    (i32::from(sample) * i32::from(MixerVolume::new(gain).get())) / 256
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
    set_dma_transfer_count(&channel, DMA_ARM_COUNT);
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

#[cfg(feature = "mcu-rp2040")]
#[inline]
fn dma_transfer_count(channel: &pac::dma::Channel) -> u32 {
    channel.trans_count().read()
}

#[cfg(feature = "mcu-rp235xa")]
#[inline]
fn dma_transfer_count(channel: &pac::dma::Channel) -> u32 {
    channel.trans_count().read().count()
}

#[cfg(feature = "mcu-rp2040")]
#[inline]
fn set_dma_transfer_count(channel: &pac::dma::Channel, count: u32) {
    channel.trans_count().write_value(count);
}

#[cfg(feature = "mcu-rp235xa")]
#[inline]
fn set_dma_transfer_count(channel: &pac::dma::Channel, count: u32) {
    channel.trans_count().write(|w| w.set_count(count));
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
