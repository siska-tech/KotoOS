//! Boot splash golden frame (KOTO-0181).
//!
//! The simulator must show the same splash the device paints so screenshots
//! and docs match the hardware. Both targets render through the one
//! `koto_core::BootSplash` painter; this test pins (a) byte parity between the
//! simulator's `render_splash_frame` path and a direct `koto-core` paint, and
//! (b) a golden checksum of the completed splash with the shipped device font,
//! so any unintended change to the art, layout, or palette fails CI.
//!
//! When the splash is changed *deliberately*, re-run with `--nocapture` and
//! update `GOLDEN_FNV1A` to the printed value.

use koto_core::{BitmapFont, BootSplash, BootStep, BootStepStatus, Canvas};
use koto_sim::render_splash_frame;

const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");

/// FNV-1a over the completed splash's RGB565 framebuffer bytes.
const GOLDEN_FNV1A: u64 = 0xbe50aa05c2a56ce5;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[test]
fn splash_golden_frame_matches_device_painter() {
    let font = BitmapFont::from_bytes(FONT_BYTES).expect("device font parses");
    let splash = BootSplash::complete();

    let mut framebuffer = render_splash_frame(&splash, &font);
    let sim_pixels = framebuffer.as_canvas().pixels().to_vec();

    // Byte parity with the painter the device firmware drives directly.
    let mut reference = vec![0u8; 320 * 320 * 2];
    splash.paint(&mut Canvas::new(&mut reference, 320, 320).unwrap(), &font);
    assert_eq!(
        sim_pixels, reference,
        "simulator splash diverged from the koto-core painter"
    );

    let hash = fnv1a(&sim_pixels);
    println!("splash fnv1a = {hash:#018x}");
    assert_eq!(
        hash, GOLDEN_FNV1A,
        "completed splash changed; update GOLDEN_FNV1A if intended"
    );
}

#[test]
fn splash_failure_frame_differs_from_success() {
    let font = BitmapFont::from_bytes(FONT_BYTES).expect("device font parses");
    let mut failed = BootSplash::complete();
    failed.resolve(BootStep::Storage, BootStepStatus::Failed("no sd"));

    let ok_pixels = render_splash_frame(&BootSplash::complete(), &font)
        .as_canvas()
        .pixels()
        .to_vec();
    let ng_pixels = render_splash_frame(&failed, &font)
        .as_canvas()
        .pixels()
        .to_vec();
    assert_ne!(
        ok_pixels, ng_pixels,
        "a failed step must be visible on the splash"
    );
}
