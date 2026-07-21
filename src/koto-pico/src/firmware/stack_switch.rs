//! Cortex-M0+ stack trampoline for KOTO-0245.
//!
//! The TLS handshake crypto (P-256 CertificateVerify + AES-GCM) is the deepest
//! synchronous call tree in the firmware, and it runs during the network-future
//! poll on core0's main stack — which already peaks just below `_stack_end`
//! during an app session (KOTO-0252). Running that one poll on a dedicated
//! stack keeps the spike off the main stack. Only the transient per-poll call
//! tree moves; the async future state stays in its arena.

use core::arch::asm;

/// Runs `f` with the stack pointer switched to `stack_top`, restoring it
/// afterward, and returns `f`'s boolean result. `f` executes entirely (to a
/// return) before this function returns — nothing on the switched stack
/// survives the call.
///
/// # Safety
/// - `stack_top` must be the 8-byte-aligned exclusive high end of a writable
///   region large enough for the whole synchronous call tree of `f` plus any
///   interrupt frames that may nest while `f` runs.
/// - The region must not alias anything else active for the duration.
/// - `f` must not unwind across the boundary (this build aborts on panic).
#[inline(never)]
pub unsafe fn call_on_stack(stack_top: *mut u8, f: &mut dyn FnMut() -> bool) -> bool {
    // Reconstructs the `&mut dyn FnMut` from a thin pointer and invokes it on
    // the switched stack. `extern "C"` gives a stable, single-argument ABI.
    extern "C" fn trampoline(closure: *mut ()) -> u32 {
        // `closure` points at the caller's `&mut &mut dyn FnMut() -> bool`.
        let f = unsafe { &mut *(closure as *mut &mut dyn FnMut() -> bool) };
        u32::from((*f)())
    }

    let mut f = f;
    let closure_ptr = core::ptr::from_mut(&mut f) as *mut ();
    let result: u32;
    // Save SP in the callee-saved r4, switch to the dedicated stack, call the
    // trampoline (arg in r0), then restore SP. The save/restore is balanced,
    // so the compiler's SP-relative frame is intact on return.
    #[cfg(feature = "mcu-rp2040")]
    unsafe {
        asm!(
            "mov r4, sp",
            "mov sp, {top}",
            "blx {tramp}",
            "mov sp, r4",
            top = in(reg) stack_top,
            tramp = in(reg) trampoline as extern "C" fn(*mut ()) -> u32,
            inout("r0") closure_ptr as u32 => result,
            out("r4") _,
            clobber_abi("C"),
        );
    }
    // LLVM's hard-float C clobber set includes the callee-saved D16-D31
    // registers on thumbv8m.main-none-eabihf and warns that naming them can be
    // undefined. Spell out the caller-saved integer and S0-S15 hard-float ABI
    // sets instead.
    #[cfg(feature = "mcu-rp235xa")]
    unsafe {
        asm!(
            "mov r4, sp",
            "mov sp, {top}",
            "blx {tramp}",
            "mov sp, r4",
            top = in(reg) stack_top,
            tramp = in(reg) trampoline as extern "C" fn(*mut ()) -> u32,
            inout("r0") closure_ptr as u32 => result,
            out("r1") _,
            out("r2") _,
            out("r3") _,
            out("r4") _,
            out("r12") _,
            out("lr") _,
            out("s0") _,
            out("s1") _,
            out("s2") _,
            out("s3") _,
            out("s4") _,
            out("s5") _,
            out("s6") _,
            out("s7") _,
            out("s8") _,
            out("s9") _,
            out("s10") _,
            out("s11") _,
            out("s12") _,
            out("s13") _,
            out("s14") _,
            out("s15") _,
        );
    }
    result != 0
}

/// Sentinel written across the dedicated stack before use so the high-water
/// mark can be recovered afterward.
pub const STACK_PAINT: u8 = 0xC5;

/// Paints `region` with [`STACK_PAINT`] and returns its 8-byte-aligned
/// exclusive high end for use as a downward-growing stack top.
pub fn paint_and_top(region: &mut [u8]) -> *mut u8 {
    region.fill(STACK_PAINT);
    let end = region.as_mut_ptr_range().end as usize;
    (end & !7) as *mut u8
}

/// Returns the peak bytes consumed from a painted, downward-growing stack:
/// the distance from the aligned top to the lowest byte the callee overwrote.
/// A paint-valued byte written by the callee undercounts, which is acceptable
/// for a headroom check.
pub fn high_water(region: &[u8]) -> usize {
    let touched_from_base = region
        .iter()
        .position(|&byte| byte != STACK_PAINT)
        .unwrap_or(region.len());
    let aligned_top = region.as_ptr_range().end as usize & !7;
    let base = region.as_ptr() as usize;
    aligned_top.saturating_sub(base + touched_from_base)
}

/// Paints a physically contiguous address range that may span adjacent linker
/// symbols. No Rust slice is formed across those distinct static allocations.
///
/// # Safety
/// Every address in `base..end` must be writable and exclusively owned until
/// the switched-stack call and high-water readback complete.
pub unsafe fn paint_raw_and_top(base: usize, end: usize) -> *mut u8 {
    let top = end & !7;
    let mut address = base;
    while address < top {
        unsafe { (address as *mut u8).write_volatile(STACK_PAINT) };
        address += 1;
    }
    top as *mut u8
}

/// Returns the high-water mark for a range painted by
/// [`paint_raw_and_top`], without constructing a cross-symbol slice.
///
/// # Safety
/// `base..top` must remain readable and exclusively owned.
pub unsafe fn high_water_raw(base: usize, top: usize) -> usize {
    let mut address = base;
    while address < top {
        if unsafe { (address as *const u8).read_volatile() } != STACK_PAINT {
            return top - address;
        }
        address += 1;
    }
    0
}
