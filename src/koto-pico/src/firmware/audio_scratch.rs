//! Permanently resident CPU0 scratch for package audio streaming.
//!
//! Runtime cue loading uses the switchable rich-audio residency directly. This
//! storage contains only the encoded refill and decoded PCM views required in
//! `WifiStreamAudio`.

use core::{
    cell::{Cell, UnsafeCell},
    slice,
};

/// Refill half of the 256 ms PCM ring at a time.
pub(crate) const STREAM_REFILL_FRAMES: usize = 2048;
/// The decode view is reused four times per refill.
pub(crate) const STREAM_DECODE_FRAMES: usize = 512;
pub(crate) const STREAM_PCM16_BYTES: usize = STREAM_REFILL_FRAMES * 2;
pub(crate) const STREAM_SLD4_BYTES: usize = STREAM_REFILL_FRAMES / 2;

const STREAM_DECODE_BYTES: usize = STREAM_DECODE_FRAMES * core::mem::size_of::<i16>();
pub(crate) const STREAM_BYTES: usize = STREAM_PCM16_BYTES + STREAM_DECODE_BYTES;
const GUARD_VALUE: u32 = 0xa5c3_5a3c;

const _: () = assert!(STREAM_PCM16_BYTES % core::mem::align_of::<i16>() == 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum ScratchMode {
    Idle,
    Stream,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AudioScratchStats {
    pub(crate) load_acquisitions: u32,
    pub(crate) stream_acquisitions: u32,
    pub(crate) rejected_acquisitions: u32,
    pub(crate) corruption_failures: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AudioRegressionStageStats {
    pub(crate) pcm16_stream_starts: u32,
    pub(crate) sld4_stream_starts: u32,
    pub(crate) cold_cue_loads: u32,
    pub(crate) cue_cache_hits: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScratchBusy;

/// The guards and explicit alignment make the layout visible in the ELF while
/// retaining zero-filled, in-place initialization in `.bss`.
#[repr(C, align(4))]
struct AlignedStorage {
    leading_guard: u32,
    trailing_guard: u32,
    /// Keep the raw bytes last: their end is the end of the ELF symbol and is
    /// therefore contiguous with `AUDIO_STREAM_SHARED`'s PCM samples.
    bytes: [u8; STREAM_BYTES],
}

impl AlignedStorage {
    const fn new() -> Self {
        Self {
            // Remain all-zero for in-place `.bss` initialization. The product
            // session arms both guards before its first acquisition.
            leading_guard: 0,
            trailing_guard: 0,
            bytes: [0; STREAM_BYTES],
        }
    }
}

const _: () = assert!(core::mem::align_of::<AlignedStorage>() >= core::mem::align_of::<i16>());
const _: () = assert!(
    core::mem::size_of::<AlignedStorage>() <= STREAM_BYTES + 2 * core::mem::size_of::<u32>()
);

#[repr(C, align(4))]
struct Cpu0AudioScratch {
    mode: Cell<ScratchMode>,
    load_acquisitions: Cell<u32>,
    stream_acquisitions: Cell<u32>,
    rejected_acquisitions: Cell<u32>,
    corruption_failures: Cell<u32>,
    pcm16_stream_starts: Cell<u32>,
    sld4_stream_starts: Cell<u32>,
    cold_cue_loads: Cell<u32>,
    cue_cache_hits: Cell<u32>,
    /// Last field so `storage.bytes` ends exactly at the static-symbol end.
    storage: UnsafeCell<AlignedStorage>,
}

// Safety: the product frame loop is the only caller and runs these accessors
// on CPU0. CPU1 receives owned copies through AudioShared and never accesses
// this static. The runtime mode guard rejects accidental nested CPU0 access.
unsafe impl Sync for Cpu0AudioScratch {}

impl Cpu0AudioScratch {
    const fn new() -> Self {
        Self {
            mode: Cell::new(ScratchMode::Idle),
            load_acquisitions: Cell::new(0),
            stream_acquisitions: Cell::new(0),
            rejected_acquisitions: Cell::new(0),
            corruption_failures: Cell::new(0),
            pcm16_stream_starts: Cell::new(0),
            sld4_stream_starts: Cell::new(0),
            cold_cue_loads: Cell::new(0),
            cue_cache_hits: Cell::new(0),
            storage: UnsafeCell::new(AlignedStorage::new()),
        }
    }

    fn try_with<R>(
        &self,
        mode: ScratchMode,
        use_bytes: impl FnOnce(&mut [u8; STREAM_BYTES]) -> R,
    ) -> Result<R, ScratchBusy> {
        if self.mode.get() != ScratchMode::Idle {
            self.rejected_acquisitions
                .set(self.rejected_acquisitions.get().saturating_add(1));
            return Err(ScratchBusy);
        }
        self.mode.set(mode);
        match mode {
            ScratchMode::Stream => self
                .stream_acquisitions
                .set(self.stream_acquisitions.get().saturating_add(1)),
            ScratchMode::Idle => {}
        }

        let storage = unsafe { &mut *self.storage.get() };
        let guard_failed =
            storage.leading_guard != GUARD_VALUE || storage.trailing_guard != GUARD_VALUE;
        let result = use_bytes(&mut storage.bytes);
        if guard_failed
            || storage.leading_guard != GUARD_VALUE
            || storage.trailing_guard != GUARD_VALUE
        {
            self.corruption_failures
                .set(self.corruption_failures.get().saturating_add(1));
        }
        self.mode.set(ScratchMode::Idle);
        Ok(result)
    }

    fn stats(&self) -> AudioScratchStats {
        AudioScratchStats {
            load_acquisitions: self.load_acquisitions.get(),
            stream_acquisitions: self.stream_acquisitions.get(),
            rejected_acquisitions: self.rejected_acquisitions.get(),
            corruption_failures: self.corruption_failures.get(),
        }
    }

    fn reset_diagnostics(&self) {
        debug_assert_eq!(self.mode.get(), ScratchMode::Idle);
        // Safety: reset runs on CPU0 before the app session can acquire either
        // view. Only the guard metadata is initialized here; the 8 KiB-class
        // storage remains in-place and is never constructed on the stack.
        unsafe {
            (*self.storage.get()).leading_guard = GUARD_VALUE;
            (*self.storage.get()).trailing_guard = GUARD_VALUE;
        }
        self.load_acquisitions.set(0);
        self.stream_acquisitions.set(0);
        self.rejected_acquisitions.set(0);
        self.corruption_failures.set(0);
        self.pcm16_stream_starts.set(0);
        self.sld4_stream_starts.set(0);
        self.cold_cue_loads.set(0);
        self.cue_cache_hits.set(0);
    }

    fn regression_stages(&self) -> AudioRegressionStageStats {
        AudioRegressionStageStats {
            pcm16_stream_starts: self.pcm16_stream_starts.get(),
            sld4_stream_starts: self.sld4_stream_starts.get(),
            cold_cue_loads: self.cold_cue_loads.get(),
            cue_cache_hits: self.cue_cache_hits.get(),
        }
    }
}

#[unsafe(no_mangle)]
static AUDIO_STREAM_SCRATCH: Cpu0AudioScratch = Cpu0AudioScratch::new();

/// KOTO-0245: raw view of the CPU0 stream scratch, used during an HTTPS fetch
/// to hold the TLS receive-record buffer, HTTP decoder, and plaintext staging;
/// its remaining tail extends the 8 KiB PCM crypto stack. During a fetch the stream is
/// quiesced (the caller holds the TLS/audio workspace claim), so the CPU0
/// refill that normally owns this storage cannot run and nothing else aliases
/// it.
///
/// # Safety
/// The caller must invoke this only on CPU0 while stream audio is quiesced for
/// the whole lifetime of the returned borrow, and must not let the ordinary
/// scratch accessors run concurrently.
#[cfg(feature = "app_fetch_https")]
pub unsafe fn tls_workspace_bytes() -> &'static mut [u8] {
    unsafe { &mut (*AUDIO_STREAM_SCRATCH.storage.get()).bytes }
}

/// Bytes from the end of the raw scratch array to the next ELF symbol. The
/// field order above deliberately makes this zero, so the extended stack never
/// overwrites scratch guards, mode, or counters while it is parked.
#[cfg(all(feature = "mcu-rp2040", feature = "app_fetch_https"))]
pub const TLS_SCRATCH_TRAILING_BYTES: usize = core::mem::size_of::<Cpu0AudioScratch>()
    - core::mem::offset_of!(Cpu0AudioScratch, storage)
    - core::mem::offset_of!(AlignedStorage, bytes)
    - STREAM_BYTES;

/// Reconstructs the scratch guards, ownership mode, and diagnostics after the
/// extended TLS crypto stack is no longer borrowed.
///
/// # Safety
/// No scratch borrow may remain, and stream audio must still be quiesced.
#[cfg(all(feature = "mcu-rp2040", feature = "app_fetch_https"))]
pub unsafe fn restore_after_tls_stack() {
    unsafe {
        let storage = &mut *AUDIO_STREAM_SCRATCH.storage.get();
        core::ptr::write_volatile(&mut storage.leading_guard, 0);
        for byte in &mut storage.bytes {
            core::ptr::write_volatile(byte, 0);
        }
        core::ptr::write_volatile(&mut storage.trailing_guard, 0);
    }
    AUDIO_STREAM_SCRATCH.mode.set(ScratchMode::Idle);
    AUDIO_STREAM_SCRATCH.load_acquisitions.set(0);
    AUDIO_STREAM_SCRATCH.stream_acquisitions.set(0);
    AUDIO_STREAM_SCRATCH.rejected_acquisitions.set(0);
    AUDIO_STREAM_SCRATCH.corruption_failures.set(0);
    AUDIO_STREAM_SCRATCH.pcm16_stream_starts.set(0);
    AUDIO_STREAM_SCRATCH.sld4_stream_starts.set(0);
    AUDIO_STREAM_SCRATCH.cold_cue_loads.set(0);
    AUDIO_STREAM_SCRATCH.cue_cache_hits.set(0);
}

pub(crate) fn try_with_stream<R>(
    use_stream: impl FnOnce(&mut [u8], &mut [i16]) -> R,
) -> Result<R, ScratchBusy> {
    AUDIO_STREAM_SCRATCH.try_with(ScratchMode::Stream, |bytes| {
        let (encoded, decoded_bytes) = bytes.split_at_mut(STREAM_PCM16_BYTES);
        // Safety: AlignedStorage is 4-byte aligned, the byte view begins after
        // a u32 guard, and STREAM_PCM16_BYTES is i16-aligned. The const size
        // assertion above proves the decoded view remains inside `bytes`.
        let decoded = unsafe {
            slice::from_raw_parts_mut(
                decoded_bytes.as_mut_ptr().cast::<i16>(),
                STREAM_DECODE_FRAMES,
            )
        };
        use_stream(encoded, decoded)
    })
}

pub(crate) fn stats() -> AudioScratchStats {
    AUDIO_STREAM_SCRATCH.stats()
}

pub(crate) fn reset_diagnostics() {
    AUDIO_STREAM_SCRATCH.reset_diagnostics();
}

pub(crate) fn regression_stages() -> AudioRegressionStageStats {
    AUDIO_STREAM_SCRATCH.regression_stages()
}

pub(crate) fn record_load_acquisition() {
    AUDIO_STREAM_SCRATCH.load_acquisitions.set(
        AUDIO_STREAM_SCRATCH
            .load_acquisitions
            .get()
            .saturating_add(1),
    );
}

pub(crate) fn record_pcm16_stream_start() {
    AUDIO_STREAM_SCRATCH.pcm16_stream_starts.set(
        AUDIO_STREAM_SCRATCH
            .pcm16_stream_starts
            .get()
            .saturating_add(1),
    );
}

pub(crate) fn record_sld4_stream_start() {
    AUDIO_STREAM_SCRATCH.sld4_stream_starts.set(
        AUDIO_STREAM_SCRATCH
            .sld4_stream_starts
            .get()
            .saturating_add(1),
    );
}

pub(crate) fn record_cold_cue_load() {
    AUDIO_STREAM_SCRATCH
        .cold_cue_loads
        .set(AUDIO_STREAM_SCRATCH.cold_cue_loads.get().saturating_add(1));
}

pub(crate) fn record_cue_cache_hit() {
    AUDIO_STREAM_SCRATCH
        .cue_cache_hits
        .set(AUDIO_STREAM_SCRATCH.cue_cache_hits.get().saturating_add(1));
}

#[cfg(test)]
pub(crate) fn corrupt_trailing_guard_for_test() {
    // Safety: test-only fault injection runs synchronously while the scratch is
    // idle and deliberately mutates only the guard metadata.
    unsafe {
        (*AUDIO_STREAM_SCRATCH.storage.get()).trailing_guard = 1;
    }
}
