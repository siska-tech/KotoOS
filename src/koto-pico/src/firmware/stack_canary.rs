//! Core-0 main-stack paint-and-scan canary (KOTO-0170 Stage 0).
//!
//! RAM layout (memory.x + cortex-m-rt `link.x`): `.data`/`.bss`/`.uninit` grow
//! up from `0x2000_0000` and the main stack grows down from `_stack_start`
//! (`0x2004_2000`) with **no linker guard** — a stack peak that reaches
//! `__euninit` silently corrupts statics instead of faulting at link time
//! (the KOTO-0136 boot hang, and what kept `ram_interpreter` opt-in until the
//! KOTO-0170 Stage-1 flip). This module measures the real peak so that budget
//! stops being guessed at in 2.5 KiB-wide brackets:
//!
//! - [`paint`] runs once, as the first statement of `main`, and fills the
//!   untouched gap between `__euninit` and the live stack pointer with
//!   [`CANARY_WORD`].
//! - [`emit_peak`] scans upward from `__euninit` for the first word the stack
//!   has overwritten (the low-water mark) and emits the sparse
//!   `phase=176 stack-peak` UART line. The scan is a linear read of the
//!   *untouched* region only (~76 KiB worst case ≈ 19k word loads, well under
//!   a millisecond), so the sparse call sites stay always-on under every
//!   DIAG-0001 profile: this line is the permanent regression tripwire for
//!   future `.bss` growth, not an investigation-only diagnostic.
//!
//! The low-water mark is monotonic (pattern words are never repainted), so a
//! later scan always reports the session-wide peak, whichever call site runs
//! it. Core 1 (the audio worker's `AUDIO_CORE1_STACK`) is *not* covered here;
//! measuring it with the same technique is a KOTO-0170 Stage-1 candidate.

use core::fmt::Write;
use core::ptr::addr_of;
use core::sync::atomic::{AtomicUsize, Ordering};

use embassy_rp::uart::UartTx;

use crate::dashboard::LineBuffer;
use crate::firmware::diag::uart_write_line;

extern "C" {
    /// End of `.uninit` — the highest statically allocated RAM address
    /// (cortex-m-rt `link.x`; `__sheap` is provided as an alias of it).
    static __euninit: u8;
    /// Initial stack pointer: top of RAM (`ORIGIN(RAM) + LENGTH(RAM)`).
    static _stack_start: u8;
}

/// Fill pattern. Arbitrary but recognizable in a memory dump ("koto" in
/// little-endian ASCII). If a live stack word at the low-water boundary ever
/// equals it by chance the scan runs one word further and under-reports
/// `used` by 4 bytes — noise against the KiB-scale margins this measures.
const CANARY_WORD: u32 = 0x6F74_6F6B;

/// Words left unpainted immediately below the boot-time stack pointer.
/// Painting strictly below SP is already safe on ARM (no red zone; exception
/// frames complete before thread code resumes), so this only trims the
/// measurement range by 64 bytes as belt-and-braces around the paint loop's
/// own epilogue.
const PAINT_GUARD_BYTES: usize = 64;

/// Top of the painted region (the boot-time SP minus the guard), recorded by
/// [`paint`] so scans stay inside memory known to hold the pattern. Zero until
/// `paint` runs; thumbv6m has native atomic load/store, which is all this
/// single-core flag needs.
static PAINTED_TOP: AtomicUsize = AtomicUsize::new(0);

/// Base of RAM (`ORIGIN(RAM)` in memory.x).
pub const SRAM_ORIGIN: usize = 0x2000_0000;
/// Total on-chip SRAM (`LENGTH(RAM)` = 264 KiB).
pub const SRAM_TOTAL: usize = 264 * 1024;

#[inline]
fn region_bottom() -> usize {
    // `.uninit` ends 4-aligned in link.x, but align up defensively so the
    // word loop can never touch the last static byte.
    (addr_of!(__euninit) as usize + 3) & !3
}

/// Bytes of RAM occupied by the statics (`.data`/`.bss`/`.uninit`), i.e. the
/// span from [`SRAM_ORIGIN`] up to the end of `.uninit`. Complements the
/// [`scan`] low-water reading for the KOTO-0182 memory view.
pub fn static_used() -> usize {
    region_bottom() - SRAM_ORIGIN
}

#[inline]
fn stack_top() -> usize {
    addr_of!(_stack_start) as usize
}

/// Paint the free gap between the statics and the current stack pointer.
///
/// Must run before anything with a deep call tree: usage that returned before
/// the paint is overwritten and never measured. Everything below the SP read
/// here is dead memory (thread mode, single core, no DMA armed yet), so the
/// volatile word fill cannot clobber live state.
#[inline(never)]
pub fn paint() {
    let bottom = region_bottom();
    let sp = cortex_m::register::msp::read() as usize;
    let top = sp.saturating_sub(PAINT_GUARD_BYTES) & !3;
    if top <= bottom {
        return;
    }
    let mut word = bottom as *mut u32;
    while (word as usize) < top {
        // Volatile: the compiler must not elide or reorder the fill against
        // the later scans of the same untyped memory.
        unsafe {
            word.write_volatile(CANARY_WORD);
            word = word.add(1);
        }
    }
    PAINTED_TOP.store(top, Ordering::Relaxed);
}

/// A scanned low-water snapshot: the session-peak stack depth (`used`, from
/// the top of RAM) and the minimum margin left above the statics (`free_min`).
#[derive(Clone, Copy)]
pub struct StackPeak {
    pub used: usize,
    pub free_min: usize,
    pub low_water: usize,
}

/// Scan for the low-water mark. `None` until [`paint`] has run.
pub fn scan() -> Option<StackPeak> {
    let painted_top = PAINTED_TOP.load(Ordering::Relaxed);
    if painted_top == 0 {
        return None;
    }
    let bottom = region_bottom();
    let mut word = bottom as *const u32;
    while (word as usize) < painted_top {
        if unsafe { word.read_volatile() } != CANARY_WORD {
            break;
        }
        word = unsafe { word.add(1) };
    }
    let low_water = word as usize;
    Some(StackPeak {
        used: stack_top() - low_water,
        free_min: low_water - bottom,
        low_water,
    })
}

/// One-time region line so a UART capture is self-describing without the map
/// file: where the measured gap sits and how much of it was painted.
pub fn emit_region(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>) {
    let painted_top = PAINTED_TOP.load(Ordering::Relaxed);
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=176 stack-canary bottom=0x{:08x} painted_top=0x{:08x} stack_top=0x{:08x} painted={}\r\n",
        region_bottom(),
        painted_top,
        stack_top(),
        painted_top.saturating_sub(region_bottom()),
    );
    uart_write_line(uart, &line);
}

/// Scan and emit the sparse `phase=176 stack-peak` line. `at` names the call
/// site (`boot` / `shell` / `app` / `app-exit`) so a capture shows *when* the
/// peak was first observed, even though the mark itself is session-monotonic.
pub fn emit_peak(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, at: &str) {
    let Some(peak) = scan() else {
        return;
    };
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=176 stack-peak at={} used={} free_min={} lw=0x{:08x}\r\n",
        at, peak.used, peak.free_min, peak.low_water,
    );
    uart_write_line(uart, &line);
}
