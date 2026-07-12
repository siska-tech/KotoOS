use crate::hal::{HalError, PsramHal};
use crate::runtime::{CodeSource, CodeTileTransition};

/// Transfer granularity for block-oriented PSRAM access.
///
/// Callers stage bytecode and assets in SRAM buffers sized in multiples of this
/// constant so backends can issue efficient DMA bursts. The value is a prototype
/// default; the embedded backend may tune it once real timing data exists.
pub const PSRAM_BLOCK_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PsramError {
    /// The backing PSRAM reported itself as unavailable.
    Unavailable,
    /// The requested transfer falls outside the configured capacity.
    OutOfRange,
    /// A block transfer buffer was not exactly [`PSRAM_BLOCK_SIZE`] bytes.
    BlockSizeMismatch,
    /// The underlying HAL transfer failed.
    Hal(HalError),
}

/// Core-facing PSRAM block API.
///
/// This wrapper deliberately exposes only copy-based transfers between PSRAM and
/// caller-provided SRAM buffers. It never returns a slice or pointer into PSRAM,
/// keeping the RP2040 rule (no memory map / XIP / direct deref) enforceable at
/// the type level. All transfers are range-checked against `capacity` before the
/// request reaches the backend.
pub struct PsramBlocks<H> {
    hal: H,
    capacity: u32,
}

impl<H: PsramHal> PsramBlocks<H> {
    /// Wrap a `PsramHal` backend exposing `capacity` bytes of storage.
    ///
    /// Returns [`PsramError::Unavailable`] if the backend has no PSRAM populated.
    pub fn try_new(hal: H, capacity: u32) -> Result<Self, PsramError> {
        if !hal.available() {
            return Err(PsramError::Unavailable);
        }
        Ok(Self { hal, capacity })
    }

    /// Total addressable PSRAM in bytes.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Number of whole [`PSRAM_BLOCK_SIZE`] blocks that fit in `capacity`.
    pub fn block_count(&self) -> u32 {
        self.capacity / PSRAM_BLOCK_SIZE as u32
    }

    /// Copy `dst.len()` bytes starting at `address` into the SRAM buffer `dst`.
    pub fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), PsramError> {
        self.check_range(address, dst.len())?;
        self.hal.read(address, dst).map_err(PsramError::Hal)
    }

    /// Copy the SRAM buffer `src` into PSRAM starting at `address`.
    pub fn write(&mut self, address: u32, src: &[u8]) -> Result<(), PsramError> {
        self.check_range(address, src.len())?;
        self.hal.write(address, src).map_err(PsramError::Hal)
    }

    /// Mutable access to the underlying HAL backend (diagnostic/feature paths only).
    pub fn backend_mut(&mut self) -> &mut H {
        &mut self.hal
    }

    /// Copy `dst.len()` bytes starting at `address` for a [`PsramCodeWindow`]
    /// refill, allowing the backend to use its opt-in code-fetch read path. The
    /// default backend behavior is identical to [`PsramBlocks::read`].
    pub fn read_code_window(&mut self, address: u32, dst: &mut [u8]) -> Result<(), PsramError> {
        self.check_range(address, dst.len())?;
        self.hal
            .read_code_window(address, dst)
            .map_err(PsramError::Hal)
    }

    /// Read a single block by index into an exactly block-sized SRAM buffer.
    pub fn read_block(&mut self, index: u32, dst: &mut [u8]) -> Result<(), PsramError> {
        let address = self.block_address(index, dst.len())?;
        self.hal.read(address, dst).map_err(PsramError::Hal)
    }

    /// Write a single block by index from an exactly block-sized SRAM buffer.
    pub fn write_block(&mut self, index: u32, src: &[u8]) -> Result<(), PsramError> {
        let address = self.block_address(index, src.len())?;
        self.hal.write(address, src).map_err(PsramError::Hal)
    }

    /// Validate a byte range and return its exclusive end offset.
    fn check_range(&self, address: u32, len: usize) -> Result<u32, PsramError> {
        let len = u32::try_from(len).map_err(|_| PsramError::OutOfRange)?;
        let end = address.checked_add(len).ok_or(PsramError::OutOfRange)?;
        if end > self.capacity {
            return Err(PsramError::OutOfRange);
        }
        Ok(end)
    }

    /// Validate a block index plus buffer length and return its byte address.
    fn block_address(&self, index: u32, len: usize) -> Result<u32, PsramError> {
        if len != PSRAM_BLOCK_SIZE {
            return Err(PsramError::BlockSizeMismatch);
        }
        let address = index
            .checked_mul(PSRAM_BLOCK_SIZE as u32)
            .ok_or(PsramError::OutOfRange)?;
        self.check_range(address, len)?;
        Ok(address)
    }
}

/// Small SRAM working window over PSRAM-resident program code (KOTO-0127).
///
/// The KOTO-0125 launch path held an entire program in an SRAM bytecode buffer, so
/// any app larger than that buffer failed to launch; real games carry 20–73 KiB of
/// code, far past a sensible RP2040 SRAM bytecode budget. This window keeps the
/// code segment resident in PSRAM and serves the VM one 4-byte code word at a time
/// from a small SRAM cache refilled through [`PsramBlocks`] (FR-RT-2 / FR-RT-5), so
/// a program never has to fit in SRAM.
///
/// The window *tiles* the code: word `i` is served from the tile
/// `[base, base + tile_words)` where `base` is `i` rounded down to a tile
/// multiple. Sequential execution (the common case) only refills when it crosses a
/// tile boundary; a branch or call outside the cached tiles refills once and then
/// runs from cache again. An app whose whole code fits one tile loads on the
/// first miss and never refills, matching pre-PSRAM behavior for small apps.
///
/// Two cache shapes share this type (KOTO-0173):
///
/// - [`Self::new`] — one tile spanning the whole `window` buffer (the historical
///   single-tile shape; the host/sim/test call sites keep it).
/// - [`Self::new_two_tile`] — the buffer split into **two resident tiles** with
///   MRU/LRU replacement, so a `main`<->helper ping-pong between two far-apart
///   code regions loads each tile once and then runs entirely from cache,
///   instead of refilling on every crossing. Sequential fetches still check the
///   MRU slot first, so the straight-line hot path costs the same compares as
///   the single-tile shape.
///
/// History: the first 2-tile attempt (KOTO-0131) coincided with a KotoBlocks
/// launch hang and was reverted (KOTO-0134). KOTO-0170/0172 later measured the
/// real cause of that era's hangs — the invisible main-stack ceiling that the
/// +8 KiB buffer growth crossed — and widened the margin to ~81 KiB with a
/// permanent canary, so KOTO-0173 re-lands the cache with the budget accounted.
pub struct PsramCodeWindow<'a, H> {
    psram: &'a mut PsramBlocks<H>,
    window: &'a mut [u8],
    base_addr: u32,
    code_words: u32,
    /// Tile size in code words: the whole `window` for [`Self::new`], half of it
    /// for [`Self::new_two_tile`]. Tile indices in the diagnostics are
    /// `base_word / tile_words`.
    tile_words: u32,
    /// Resident tile slots actually in use (1 or 2). Slot 1 stays permanently
    /// empty in the single-tile shape, so the lookup path needs no branch on it.
    slots: usize,
    /// Per-slot first cached code word.
    slot_base_word: [u32; 2],
    /// Per-slot valid word count (`0` = empty/invalidated).
    slot_len_words: [u32; 2],
    /// Most-recently-used slot; the other slot is the refill victim (LRU).
    mru_slot: usize,
    /// Window refills since the last [`CodeSource::reset_fetch_metrics`] — the
    /// per-frame code-fetch thrash counter (KOTO-0134).
    refills: u32,
    /// Bitmask of tile indices refilled since the last reset (bit `t` = tile
    /// `t`). Tiles beyond bit 31 are not tracked, which only undercounts an
    /// already-pathological case. Its population count is the distinct-tile count.
    tiles_touched: u32,
    /// Per-frame refill count by tile index (KOTO-0136 triage); tiles `>= 32` fold
    /// into the last bucket (an already-pathological large-code case).
    tile_refills: [u16; CODE_TILE_BUCKETS],
    /// Bounded top-`CODE_TRANS_SLOTS` table of tile→tile refill transitions this
    /// frame, and the last refilled tile (`-1` = none yet) to pair against.
    transitions: [CodeTileTransition; CODE_TRANS_SLOTS],
    last_tile: i32,
    /// Monotonic microsecond clock for refill timing (KOTO-0132 phase 1). The device
    /// installs an embassy-time reader via [`PsramCodeWindow::set_refill_clock`]; the
    /// default ([`zero_us`]) returns 0, so the host/sim/test paths — which do not
    /// time PSRAM and may not even use this window — carry no clock dependency and
    /// leave the accumulators at 0 with no measurable overhead.
    now_us: fn() -> u64,
    /// Microseconds spent in [`PsramCodeWindow::refill`]'s synchronous PSRAM transfer
    /// since the last reset (saturating), the slowest single refill, and the bytes
    /// those refills moved — the per-frame refill-cost diagnostic (KOTO-0132).
    cw_refill_us_total: u32,
    cw_refill_us_max: u32,
    cw_refill_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PsramCodeWindowDebugState {
    pub base_addr: u32,
    pub code_words: u32,
    pub window_base_word: u32,
    pub window_len_words: u32,
    pub window_capacity_words: u32,
}

/// Default no-op clock for [`PsramCodeWindow`] refill timing (KOTO-0132 phase 1):
/// reports 0 µs so the host/sim/test paths carry no clock dependency and the timing
/// accumulators stay zero until the device installs a real clock.
fn zero_us() -> u64 {
    0
}

/// Tile-refill histogram buckets (KOTO-0136). 32 matches the [`tiles_touched`]
/// bitmask range; tiles at or beyond it fold into the last bucket.
const CODE_TILE_BUCKETS: usize = 32;
/// Bounded tile→tile transition table size (KOTO-0136). A hot ping-pong has 2-3
/// dominant pairs, so a small table captures it; a many-tile walk overflows it
/// (visible as low, even per-bucket counts in the histogram instead).
const CODE_TRANS_SLOTS: usize = 8;

impl<'a, H: PsramHal> PsramCodeWindow<'a, H> {
    /// Wrap a PSRAM-resident code segment of `code_words` words based at
    /// `base_addr`, using `window` as a **single-tile** SRAM cache. `window`
    /// must hold at least one word (4 bytes); its length rounded down to a
    /// multiple of 4 is the tile size, so a larger window means fewer refills.
    pub fn new(
        psram: &'a mut PsramBlocks<H>,
        window: &'a mut [u8],
        base_addr: u32,
        code_words: u32,
    ) -> Self {
        Self::with_slots(psram, window, base_addr, code_words, 1)
    }

    /// Wrap the code segment with `window` split into **two resident tiles**
    /// (KOTO-0173): each tile is half the buffer, replacement is MRU/LRU. This
    /// is the device shape — two far-apart hot regions (e.g. `main` in a high
    /// tile, helpers in tile 0) each keep their tile resident instead of
    /// evicting each other on every call/return.
    pub fn new_two_tile(
        psram: &'a mut PsramBlocks<H>,
        window: &'a mut [u8],
        base_addr: u32,
        code_words: u32,
    ) -> Self {
        Self::with_slots(psram, window, base_addr, code_words, 2)
    }

    fn with_slots(
        psram: &'a mut PsramBlocks<H>,
        window: &'a mut [u8],
        base_addr: u32,
        code_words: u32,
        slots: usize,
    ) -> Self {
        let tile_words = ((window.len() / 4) / slots) as u32;
        Self {
            psram,
            window,
            base_addr,
            code_words,
            tile_words,
            slots,
            slot_base_word: [0; 2],
            slot_len_words: [0; 2],
            mru_slot: 0,
            refills: 0,
            tiles_touched: 0,
            tile_refills: [0; CODE_TILE_BUCKETS],
            transitions: [CodeTileTransition::default(); CODE_TRANS_SLOTS],
            last_tile: -1,
            now_us: zero_us,
            cw_refill_us_total: 0,
            cw_refill_us_max: 0,
            cw_refill_bytes: 0,
        }
    }

    /// Install a monotonic microsecond clock for refill timing (KOTO-0132 phase 1).
    /// The device passes an embassy-time reader (`|| Instant::now().as_micros()`);
    /// without this the refill-timing accumulators stay zero. The clock persists
    /// across [`CodeSource::reset_fetch_metrics`].
    pub fn set_refill_clock(&mut self, now_us: fn() -> u64) {
        self.now_us = now_us;
    }

    /// Record a refill of `tile` in the per-frame histogram and transition table
    /// (KOTO-0136). The histogram folds tiles `>= CODE_TILE_BUCKETS` into the last
    /// bucket; the transition table is bounded — a new pair past `CODE_TRANS_SLOTS`
    /// distinct pairs is dropped (a many-tile walk shows up in the flat histogram).
    fn account_tile(&mut self, tile: u32) {
        let bucket = (tile as usize).min(CODE_TILE_BUCKETS - 1);
        self.tile_refills[bucket] = self.tile_refills[bucket].saturating_add(1);
        if self.last_tile >= 0 && self.last_tile as u32 != tile {
            let from = self.last_tile.min(255) as u8;
            let to = tile.min(255) as u8;
            if let Some(slot) = self
                .transitions
                .iter_mut()
                .find(|t| t.count != 0 && t.from == from && t.to == to)
            {
                slot.count = slot.count.saturating_add(1);
            } else if let Some(slot) = self.transitions.iter_mut().find(|t| t.count == 0) {
                *slot = CodeTileTransition { from, to, count: 1 };
            }
        }
        self.last_tile = tile as i32;
    }

    /// True when `index` is inside `slot`'s cached tile.
    #[inline(always)]
    fn slot_hit(&self, slot: usize, index: u32) -> bool {
        let len = self.slot_len_words[slot];
        len != 0 && index >= self.slot_base_word[slot] && index < self.slot_base_word[slot] + len
    }

    /// Byte offset of `slot`'s tile inside the window buffer.
    #[inline(always)]
    fn slot_byte_offset(&self, slot: usize) -> usize {
        slot * (self.tile_words as usize) * 4
    }

    /// Snapshot current window addressing state for launch/read diagnostics.
    /// Reports the most-recently-used tile; capacity is the tile size (equal to
    /// the whole buffer in the single-tile shape).
    pub fn debug_state(&self) -> PsramCodeWindowDebugState {
        PsramCodeWindowDebugState {
            base_addr: self.base_addr,
            code_words: self.code_words,
            window_base_word: self.slot_base_word[self.mru_slot],
            window_len_words: self.slot_len_words[self.mru_slot],
            window_capacity_words: self.tile_words,
        }
    }

    /// The most-recently-used cached tile's bytes (`0` until the first
    /// successful refill).
    pub fn current_window_bytes(&self) -> &[u8] {
        let start = self.slot_byte_offset(self.mru_slot);
        let len = (self.slot_len_words[self.mru_slot] as usize) * 4;
        &self.window[start..start + len]
    }

    /// Mutable access to the underlying PSRAM blocks for platform diagnostics.
    pub fn psram_mut(&mut self) -> &mut PsramBlocks<H> {
        self.psram
    }

    /// Refill the LRU slot with the tile containing `index` and make it the MRU
    /// slot. Returns `false` if the PSRAM transfer fails (only the victim slot
    /// is invalidated — the other resident tile stays servable) or the window
    /// cannot hold a word.
    fn refill(&mut self, index: u32) -> bool {
        let tile_words = self.tile_words;
        if tile_words == 0 {
            return false;
        }
        let base_word = (index / tile_words) * tile_words;
        let count = tile_words.min(self.code_words - base_word);
        let byte_len = (count as usize) * 4;
        let addr = self.base_addr + base_word * 4;
        let victim = if self.slots == 2 {
            1 - self.mru_slot
        } else {
            0
        };
        let dst_start = self.slot_byte_offset(victim);
        // Bracket the synchronous PSRAM transfer with the installed clock (KOTO-0132
        // phase 1): on the device this is the single blocking refill cost the VM pays
        // on a tile miss; on host/sim `now_us` is the no-op clock so `elapsed_us` is 0.
        let started_us = (self.now_us)();
        let read = self
            .psram
            .read_code_window(addr, &mut self.window[dst_start..dst_start + byte_len]);
        let elapsed_us =
            u32::try_from((self.now_us)().wrapping_sub(started_us)).unwrap_or(u32::MAX);
        if read.is_err() {
            self.slot_len_words[victim] = 0;
            return false;
        }
        self.cw_refill_us_total = self.cw_refill_us_total.saturating_add(elapsed_us);
        self.cw_refill_us_max = self.cw_refill_us_max.max(elapsed_us);
        self.cw_refill_bytes = self.cw_refill_bytes.saturating_add(byte_len as u32);
        self.slot_base_word[victim] = base_word;
        self.slot_len_words[victim] = count;
        self.mru_slot = victim;
        // Account the refill for the per-frame thrash diagnostic (KOTO-0134): one
        // tile transfer, and which tile, so a `main`<->helper ping-pong shows many
        // refills across few distinct tiles.
        self.refills = self.refills.saturating_add(1);
        let tile = base_word / tile_words;
        if tile < 32 {
            self.tiles_touched |= 1 << tile;
        }
        self.account_tile(tile);
        true
    }

    /// Resolve `index` to a resident slot, refilling on a miss. The MRU slot is
    /// checked first so straight-line execution pays the same compares as the
    /// single-tile shape; a hit in the other slot flips MRU (slot 1 is
    /// permanently empty in the single-tile shape, so that check never hits).
    #[inline(always)]
    fn locate(&mut self, index: u32) -> Option<usize> {
        if self.slot_hit(self.mru_slot, index) {
            return Some(self.mru_slot);
        }
        let other = 1 - self.mru_slot;
        if self.slot_hit(other, index) {
            self.mru_slot = other;
            return Some(other);
        }
        if self.refill(index) {
            Some(self.mru_slot)
        } else {
            None
        }
    }
}

impl<H: PsramHal> CodeSource for PsramCodeWindow<'_, H> {
    /// KOTO-0169 Stage 2 (H2-b): interpreting PSRAM-windowed code is the
    /// device's hot path, so under `ram_interpreter` the session routes this
    /// source through the SRAM-placed twin of the frame loop. The resident
    /// `SliceCode` small-app fallback keeps the default (`false`) and stays
    /// in flash — only this instantiation pays the ~2.4 KiB RAM cost.
    const PLACE_HOT_LOOP_IN_SRAM: bool = true;

    fn word(&mut self, index: u32) -> Option<[u8; 4]> {
        if index >= self.code_words {
            return None;
        }
        let slot = self.locate(index)?;
        let offset =
            self.slot_byte_offset(slot) + ((index - self.slot_base_word[slot]) as usize) * 4;
        self.window[offset..offset + 4].try_into().ok()
    }

    /// H1 fetch fast path (KOTO-0169 Stage 1): resolve `index`'s tile exactly
    /// like [`Self::word`] — same cached check, same single refill on a miss —
    /// then serve as many consecutive words as the *current* tile holds, so
    /// the VM pays the window's lookup once per run instead of once per
    /// instruction. The run is clamped to the tile boundary and never triggers
    /// a second refill, so `refills`/`code_tiles`/`cw_refill_us` stay
    /// byte-identical to word-by-word execution for any execution path.
    ///
    /// ram_interpreter (KOTO-0169 Stage 2): this is the one `CodeSource` call
    /// left on the VM hot path (one per fetch-line refill / taken branch), so
    /// it rides in the same SRAM section as the interpreter loop. The refill
    /// slow path it calls stays in flash (2 calls/frame steady).
    #[cfg_attr(feature = "ram_interpreter", link_section = ".data.koto_vm_interp")]
    fn word_run(&mut self, index: u32, dst: &mut [[u8; 4]]) -> usize {
        if dst.is_empty() || index >= self.code_words {
            return 0;
        }
        let Some(slot) = self.locate(index) else {
            return 0;
        };
        // `refill` clamps the tile to `code_words`, so the tile end is also
        // the end-of-code bound; no separate `code_words` clamp is needed. The
        // run stays inside the hit tile, so it never triggers a second refill.
        let tile_end = self.slot_base_word[slot] + self.slot_len_words[slot];
        let run = dst.len().min((tile_end - index) as usize);
        let start =
            self.slot_byte_offset(slot) + ((index - self.slot_base_word[slot]) as usize) * 4;
        for (i, out) in dst[..run].iter_mut().enumerate() {
            let offset = start + i * 4;
            out.copy_from_slice(&self.window[offset..offset + 4]);
        }
        run
    }

    fn reset_fetch_metrics(&mut self) {
        self.refills = 0;
        self.tiles_touched = 0;
        self.tile_refills = [0; CODE_TILE_BUCKETS];
        self.transitions = [CodeTileTransition::default(); CODE_TRANS_SLOTS];
        self.last_tile = -1;
        // Per-frame refill-timing accumulators (KOTO-0132 phase 1); the clock itself
        // is not reset, only the measured totals.
        self.cw_refill_us_total = 0;
        self.cw_refill_us_max = 0;
        self.cw_refill_bytes = 0;
    }

    fn fetch_refills(&self) -> u32 {
        self.refills
    }

    fn fetch_distinct_tiles(&self) -> u32 {
        self.tiles_touched.count_ones()
    }

    fn tile_refills(&self) -> &[u16] {
        &self.tile_refills
    }

    fn tile_transitions(&self) -> &[CodeTileTransition] {
        &self.transitions
    }

    fn cw_refill_us_total(&self) -> u32 {
        self.cw_refill_us_total
    }

    fn cw_refill_us_max(&self) -> u32 {
        self.cw_refill_us_max
    }

    fn cw_refill_bytes(&self) -> u32 {
        self.cw_refill_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::CodeSource;

    /// In-memory PSRAM backend used to exercise the block API in tests.
    struct MockPsram {
        cells: Vec<u8>,
        available: bool,
    }

    impl MockPsram {
        fn new(size: usize) -> Self {
            Self {
                cells: vec![0; size],
                available: true,
            }
        }

        fn unavailable() -> Self {
            Self {
                cells: Vec::new(),
                available: false,
            }
        }

        fn range(&self, address: u32, len: usize) -> Result<core::ops::Range<usize>, HalError> {
            let start = address as usize;
            let end = start.checked_add(len).ok_or(HalError::InvalidArgument)?;
            if end > self.cells.len() {
                return Err(HalError::InvalidArgument);
            }
            Ok(start..end)
        }
    }

    impl PsramHal for MockPsram {
        fn available(&self) -> bool {
            self.available
        }

        fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
            let range = self.range(address, dst.len())?;
            dst.copy_from_slice(&self.cells[range]);
            Ok(())
        }

        fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
            let range = self.range(address, src.len())?;
            self.cells[range].copy_from_slice(src);
            Ok(())
        }
    }

    fn device(size: u32) -> PsramBlocks<MockPsram> {
        PsramBlocks::try_new(MockPsram::new(size as usize), size).unwrap()
    }

    #[test]
    fn reports_capacity_and_block_count() {
        let psram = device(PSRAM_BLOCK_SIZE as u32 * 4 + 7);
        assert_eq!(psram.capacity(), PSRAM_BLOCK_SIZE as u32 * 4 + 7);
        assert_eq!(psram.block_count(), 4);
    }

    #[test]
    fn round_trips_bytes_through_sram_buffer() {
        let mut psram = device(1024);
        let payload = [0xAB, 0xCD, 0xEF, 0x42];
        psram.write(16, &payload).unwrap();

        let mut dst = [0u8; 4];
        psram.read(16, &mut dst).unwrap();
        assert_eq!(dst, payload);
    }

    #[test]
    fn round_trips_a_full_block() {
        let mut psram = device(PSRAM_BLOCK_SIZE as u32 * 2);
        let src = [0x5Au8; PSRAM_BLOCK_SIZE];
        psram.write_block(1, &src).unwrap();

        let mut dst = [0u8; PSRAM_BLOCK_SIZE];
        psram.read_block(1, &mut dst).unwrap();
        assert_eq!(dst, src);

        // Block 0 stays untouched by the block-1 write.
        let mut other = [0xFFu8; PSRAM_BLOCK_SIZE];
        psram.read_block(0, &mut other).unwrap();
        assert_eq!(other, [0u8; PSRAM_BLOCK_SIZE]);
    }

    #[test]
    fn rejects_unavailable_backend() {
        assert_eq!(
            PsramBlocks::try_new(MockPsram::unavailable(), 1024).err(),
            Some(PsramError::Unavailable)
        );
    }

    #[test]
    fn rejects_reads_past_capacity() {
        let mut psram = device(64);
        let mut dst = [0u8; 8];
        assert_eq!(psram.read(60, &mut dst), Err(PsramError::OutOfRange));
    }

    #[test]
    fn rejects_writes_past_capacity() {
        let mut psram = device(64);
        let src = [0u8; 8];
        assert_eq!(psram.write(60, &src), Err(PsramError::OutOfRange));
    }

    #[test]
    fn rejects_address_length_overflow() {
        let mut psram = device(64);
        let mut dst = [0u8; 4];
        assert_eq!(
            psram.read(u32::MAX - 1, &mut dst),
            Err(PsramError::OutOfRange)
        );
    }

    #[test]
    fn rejects_out_of_range_block_index() {
        let mut psram = device(PSRAM_BLOCK_SIZE as u32 * 2);
        let mut dst = [0u8; PSRAM_BLOCK_SIZE];
        assert_eq!(psram.read_block(2, &mut dst), Err(PsramError::OutOfRange));
    }

    #[test]
    fn rejects_wrong_sized_block_buffer() {
        let mut psram = device(PSRAM_BLOCK_SIZE as u32 * 2);
        let mut dst = [0u8; PSRAM_BLOCK_SIZE - 1];
        assert_eq!(
            psram.read_block(0, &mut dst),
            Err(PsramError::BlockSizeMismatch)
        );
    }

    fn word_bytes(i: u32) -> [u8; 4] {
        i.to_le_bytes()
    }

    /// Stage `code_words` identifiable words at PSRAM base 0.
    fn staged_code(code_words: u32) -> PsramBlocks<MockPsram> {
        let mut psram = device(code_words * 4);
        for i in 0..code_words {
            psram.write(i * 4, &word_bytes(i)).unwrap();
        }
        psram
    }

    #[test]
    fn code_window_serves_every_word_across_tiles() {
        // 10 words, a 4-word window tiling them into [0,4) [4,8) [8,10): reads must
        // cross tile boundaries and re-fetch earlier words after a backward jump.
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);
        for i in 0..code_words {
            assert_eq!(code.word(i), Some(word_bytes(i)));
        }
        assert_eq!(code.word(2), Some(word_bytes(2)));
        assert_eq!(code.word(9), Some(word_bytes(9)));
        assert_eq!(code.word(code_words), None);
    }

    #[test]
    fn code_window_whole_program_fits_one_tile() {
        let code_words = 3u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 64];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);
        for i in 0..code_words {
            assert_eq!(code.word(i), Some(word_bytes(i)));
        }
        // One fill covers the whole program; no later refills change the tile.
        assert_eq!(code.fetch_refills(), 1);
        let state = code.debug_state();
        assert_eq!(state.window_base_word, 0);
        assert_eq!(state.window_len_words, code_words);
    }

    #[test]
    fn two_tile_ping_pong_loads_each_tile_once() {
        // 10 words, a 32-byte buffer split into two 4-word tiles: [0,4) [4,8)
        // [8,10). The KOTO-0134 thrash signature — alternating between tile 0
        // and tile 2 — must load each tile once and then run from cache, where
        // the single-tile window refilled on every crossing.
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 32];
        let mut code = PsramCodeWindow::new_two_tile(&mut psram, &mut window, 0, code_words);

        for _ in 0..8 {
            assert_eq!(code.word(0), Some(word_bytes(0))); // tile 0
            assert_eq!(code.word(8), Some(word_bytes(8))); // tile 2
        }
        assert_eq!(code.fetch_refills(), 2, "each tile loads exactly once");
        assert_eq!(code.fetch_distinct_tiles(), 2);

        // Every word is still served correctly across all three tiles.
        for i in 0..code_words {
            assert_eq!(code.word(i), Some(word_bytes(i)));
        }
        assert_eq!(code.word(code_words), None);
    }

    #[test]
    fn two_tile_evicts_the_lru_slot() {
        // Three 4-word tiles, two slots. Touch t0, t1 (both resident), then t2:
        // the victim is the LRU (t0). A t1 read is still a hit; returning to t0
        // refills again, evicting t2 (the then-LRU).
        let code_words = 12u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 32];
        let mut code = PsramCodeWindow::new_two_tile(&mut psram, &mut window, 0, code_words);

        assert_eq!(code.word(0), Some(word_bytes(0))); // load t0
        assert_eq!(code.word(4), Some(word_bytes(4))); // load t1
        assert_eq!(code.fetch_refills(), 2);
        assert_eq!(code.word(8), Some(word_bytes(8))); // load t2, evict t0
        assert_eq!(code.fetch_refills(), 3);
        assert_eq!(code.word(5), Some(word_bytes(5))); // t1 still resident
        assert_eq!(code.fetch_refills(), 3);
        assert_eq!(code.word(1), Some(word_bytes(1))); // t0 again, evict t2
        assert_eq!(code.fetch_refills(), 4);
        assert_eq!(code.word(6), Some(word_bytes(6))); // t1 survived throughout
        assert_eq!(code.fetch_refills(), 4);
        assert_eq!(code.fetch_distinct_tiles(), 3);
    }

    #[test]
    fn two_tile_word_run_serves_word_identical_bytes_and_clamps_to_the_tile() {
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 32];
        let mut code = PsramCodeWindow::new_two_tile(&mut psram, &mut window, 0, code_words);

        // A 16-word request from word 1 clamps to tile 0's end: words 1..4.
        let mut dst = [[0u8; 4]; 16];
        assert_eq!(code.word_run(1, &mut dst), 3);
        for (i, w) in dst[..3].iter().enumerate() {
            assert_eq!(*w, word_bytes(1 + i as u32));
        }
        assert_eq!(code.fetch_refills(), 1);
        // A run in the second resident tile serves without evicting the first.
        assert_eq!(code.word_run(8, &mut dst), 2);
        assert_eq!(dst[0], word_bytes(8));
        assert_eq!(dst[1], word_bytes(9));
        assert_eq!(code.fetch_refills(), 2);
        assert_eq!(code.word_run(2, &mut dst), 2); // tile 0 still cached
        assert_eq!(code.fetch_refills(), 2);
        assert_eq!(code.word_run(code_words, &mut dst), 0);
    }

    #[test]
    fn two_tile_failed_refill_keeps_the_other_tile_servable() {
        // PSRAM holds only tile 0's 4 words; the code segment *claims* 8, so a
        // tile-1 refill fails at the HAL. The failure must invalidate only the
        // victim slot: tile 0 stays resident and servable afterwards.
        let mut psram = staged_code(4);
        let mut window = [0u8; 32];
        let mut code = PsramCodeWindow::new_two_tile(&mut psram, &mut window, 0, 8);

        assert_eq!(code.word(0), Some(word_bytes(0))); // tile 0 resident
        assert_eq!(code.word(4), None); // tile 1 refill fails
        assert_eq!(code.word(1), Some(word_bytes(1))); // tile 0 unharmed
        assert_eq!(
            code.fetch_refills(),
            1,
            "the failed transfer is not counted"
        );
    }

    #[test]
    fn code_window_counts_refills_and_distinct_tiles() {
        // 10 words, a 4-word window tiling them into [0,4) [4,8) [8,10). A
        // ping-pong between tile 0 and tile 2 refills on every crossing but only
        // ever touches two distinct tiles — the KOTO-0134 thrash signature.
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);

        assert_eq!(code.word(0), Some(word_bytes(0))); // fill tile 0
        assert_eq!(code.fetch_refills(), 1);
        assert_eq!(code.fetch_distinct_tiles(), 1);
        assert_eq!(code.word(3), Some(word_bytes(3))); // cached: no refill
        assert_eq!(code.fetch_refills(), 1);

        assert_eq!(code.word(8), Some(word_bytes(8))); // -> tile 2
        assert_eq!(code.word(0), Some(word_bytes(0))); // -> tile 0
        assert_eq!(code.word(8), Some(word_bytes(8))); // -> tile 2
        assert_eq!(code.fetch_refills(), 4);
        assert_eq!(code.fetch_distinct_tiles(), 2);

        // Reset zeroes the per-frame counters; a still-cached read adds nothing.
        code.reset_fetch_metrics();
        assert_eq!(code.fetch_refills(), 0);
        assert_eq!(code.fetch_distinct_tiles(), 0);
        assert_eq!(code.word(8), Some(word_bytes(8))); // tile 2 still cached
        assert_eq!(code.fetch_refills(), 0);
    }

    #[test]
    fn code_window_word_run_matches_word_and_clamps_to_the_tile() {
        // KOTO-0169 Stage 1: same 10-word / 4-word-window tiling as the word()
        // tests. A run must serve word()-identical bytes, stop at the tile
        // boundary (never a second refill), and report out-of-range as 0.
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);

        // A 16-word request from word 1 clamps to tile 0's end: words 1..4.
        let mut dst = [[0u8; 4]; 16];
        assert_eq!(code.word_run(1, &mut dst), 3);
        for (i, w) in dst[..3].iter().enumerate() {
            assert_eq!(*w, word_bytes(1 + i as u32));
        }
        assert_eq!(
            code.fetch_refills(),
            1,
            "one refill resolves the run's tile"
        );

        // The next straight-line run starts the next tile: exactly one more
        // refill, words 4..8.
        assert_eq!(code.word_run(4, &mut dst), 4);
        for (i, w) in dst[..4].iter().enumerate() {
            assert_eq!(*w, word_bytes(4 + i as u32));
        }
        assert_eq!(code.fetch_refills(), 2);

        // The last, short tile [8,10) clamps to end-of-code.
        assert_eq!(code.word_run(8, &mut dst), 2);
        assert_eq!(dst[0], word_bytes(8));
        assert_eq!(dst[1], word_bytes(9));
        assert_eq!(code.fetch_refills(), 3);

        // A cached in-tile run adds no refill; a one-slot buffer serves one
        // word; out-of-range and empty-buffer requests serve zero.
        assert_eq!(code.word_run(9, &mut dst[..1]), 1);
        assert_eq!(dst[0], word_bytes(9));
        assert_eq!(code.fetch_refills(), 3);
        assert_eq!(code.word_run(code_words, &mut dst), 0);
        assert_eq!(code.word_run(0, &mut dst[..0]), 0);
        assert_eq!(code.fetch_refills(), 3, "rejected requests never refill");
    }

    #[test]
    fn code_window_word_run_walk_keeps_refill_metrics_of_a_word_walk() {
        // KOTO-0169 Stage 1 observe-only proof at the window level: walking the
        // whole program by runs produces exactly the refill count, distinct
        // tiles, and bytes that the word-by-word walk produces.
        let code_words = 10u32;

        let mut psram_words = staged_code(code_words);
        let mut window_words = [0u8; 16];
        let mut by_word = PsramCodeWindow::new(&mut psram_words, &mut window_words, 0, code_words);
        for i in 0..code_words {
            assert_eq!(by_word.word(i), Some(word_bytes(i)));
        }

        let mut psram_runs = staged_code(code_words);
        let mut window_runs = [0u8; 16];
        let mut by_run = PsramCodeWindow::new(&mut psram_runs, &mut window_runs, 0, code_words);
        let mut dst = [[0u8; 4]; 16];
        let mut next = 0u32;
        while next < code_words {
            let served = by_run.word_run(next, &mut dst);
            assert!(served > 0, "run stalled at word {next}");
            for (i, w) in dst[..served].iter().enumerate() {
                assert_eq!(*w, word_bytes(next + i as u32));
            }
            next += served as u32;
        }

        assert_eq!(by_run.fetch_refills(), by_word.fetch_refills());
        assert_eq!(
            by_run.fetch_distinct_tiles(),
            by_word.fetch_distinct_tiles()
        );
        assert_eq!(by_run.cw_refill_bytes(), by_word.cw_refill_bytes());
    }

    #[test]
    fn code_window_histogram_and_transitions_localize_a_ping_pong() {
        // Same 4-word window over 10 words (tiles 0/1/2). Drive a tile-0<->tile-2
        // ping-pong: the histogram must concentrate on those two buckets and the
        // transition table must surface the `0>2` / `2>0` pair (KOTO-0136 triage).
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);

        assert_eq!(code.word(0), Some(word_bytes(0))); // fill tile 0
        for _ in 0..3 {
            assert_eq!(code.word(8), Some(word_bytes(8))); // -> tile 2
            assert_eq!(code.word(0), Some(word_bytes(0))); // -> tile 0
        }
        // 1 (initial tile 0) + 3*(tile 2 + tile 0) = 7 refills across 2 tiles.
        assert_eq!(code.fetch_refills(), 7);
        assert_eq!(code.fetch_distinct_tiles(), 2);

        let hist = code.tile_refills();
        assert_eq!(hist[0], 4); // tile 0: initial + 3 returns
        assert_eq!(hist[2], 3); // tile 2: 3 entries
        assert_eq!(hist[1], 0); // tile 1 never touched
        assert_eq!(hist.iter().map(|&c| c as u32).sum::<u32>(), 7);

        // Transitions: 0>2 three times, 2>0 three times (the initial fill has no
        // predecessor, so it records no transition).
        let trans = code.tile_transitions();
        let find = |from: u8, to: u8| {
            trans
                .iter()
                .find(|t| t.count != 0 && t.from == from && t.to == to)
                .map(|t| t.count)
        };
        assert_eq!(find(0, 2), Some(3));
        assert_eq!(find(2, 0), Some(3));
        assert_eq!(trans.iter().filter(|t| t.count != 0).count(), 2);

        // Reset clears the histogram and transition table too.
        code.reset_fetch_metrics();
        assert!(code.tile_refills().iter().all(|&c| c == 0));
        assert!(code.tile_transitions().iter().all(|t| t.count == 0));
    }

    #[test]
    fn code_window_honors_nonzero_base_address() {
        let code_words = 5u32;
        let base = (PSRAM_BLOCK_SIZE as u32) * 2;
        let mut psram = device(base + code_words * 4);
        for i in 0..code_words {
            psram.write(base + i * 4, &word_bytes(i)).unwrap();
        }
        let mut window = [0u8; 8];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, base, code_words);
        for i in 0..code_words {
            assert_eq!(code.word(i), Some(word_bytes(i)));
        }
    }

    use core::sync::atomic::{AtomicU64, Ordering};

    /// Deterministic monotonic clock for the refill-timing test (KOTO-0132 phase 1):
    /// each call advances 7 µs and returns the pre-advance value, so a refill (which
    /// reads the clock twice, before and after its PSRAM transfer) measures exactly
    /// 7 µs. Only one test installs it, so the shared atomic carries no contention.
    static TEST_CLOCK_US: AtomicU64 = AtomicU64::new(0);
    fn tick_clock() -> u64 {
        TEST_CLOCK_US.fetch_add(7, Ordering::Relaxed)
    }

    #[test]
    fn code_window_times_refills_with_installed_clock() {
        TEST_CLOCK_US.store(0, Ordering::Relaxed);
        // 10 words, 4-word tiles: [0,4) [4,8) [8,10). Tile 0/1 refill 16 bytes, the
        // short tail tile 2 refills 8 bytes (words 8,9).
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);
        code.set_refill_clock(tick_clock);

        // Before any refill the accumulators are zero.
        assert_eq!(code.cw_refill_us_total(), 0);
        assert_eq!(code.cw_refill_us_max(), 0);
        assert_eq!(code.cw_refill_bytes(), 0);

        assert_eq!(code.word(0), Some(word_bytes(0))); // refill tile 0: 16 B, 7 us
        assert_eq!(code.fetch_refills(), 1);
        assert_eq!(code.cw_refill_us_total(), 7);
        assert_eq!(code.cw_refill_us_max(), 7);
        assert_eq!(code.cw_refill_bytes(), 16);

        assert_eq!(code.word(3), Some(word_bytes(3))); // cached: no refill, no time
        assert_eq!(code.cw_refill_us_total(), 7);
        assert_eq!(code.cw_refill_bytes(), 16);

        assert_eq!(code.word(8), Some(word_bytes(8))); // refill tile 2: 8 B, 7 us
        assert_eq!(code.fetch_refills(), 2);
        assert_eq!(code.cw_refill_us_total(), 14);
        assert_eq!(code.cw_refill_us_max(), 7);
        assert_eq!(code.cw_refill_bytes(), 24); // 16 + 8

        // Reset zeroes the timing accumulators (the clock itself persists).
        code.reset_fetch_metrics();
        assert_eq!(code.cw_refill_us_total(), 0);
        assert_eq!(code.cw_refill_us_max(), 0);
        assert_eq!(code.cw_refill_bytes(), 0);

        assert_eq!(code.word(8), Some(word_bytes(8))); // tile 2 still cached: no time
        assert_eq!(code.cw_refill_us_total(), 0);
        assert_eq!(code.word(0), Some(word_bytes(0))); // refill tile 0 again: 16 B, 7 us
        assert_eq!(code.cw_refill_us_total(), 7);
        assert_eq!(code.cw_refill_bytes(), 16);
    }

    #[test]
    fn code_window_default_clock_reports_zero_refill_time() {
        // Without an installed clock the no-op default measures 0 µs, but the bytes
        // moved are still counted (they do not depend on the clock).
        let code_words = 10u32;
        let mut psram = staged_code(code_words);
        let mut window = [0u8; 16];
        let mut code = PsramCodeWindow::new(&mut psram, &mut window, 0, code_words);
        assert_eq!(code.word(0), Some(word_bytes(0)));
        assert_eq!(code.fetch_refills(), 1);
        assert_eq!(code.cw_refill_us_total(), 0);
        assert_eq!(code.cw_refill_us_max(), 0);
        assert_eq!(code.cw_refill_bytes(), 16);
    }
}
