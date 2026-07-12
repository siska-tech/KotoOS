//! Boot splash rendering (KOTO-0181): the identity moment between LCD init
//! and the shell's first paint.
//!
//! The pixels come from the shared `koto_core::BootSplash` painter (the same
//! one KotoSim renders, so screenshots match the device); this module only
//! drives it through the firmware's strip-at-a-time transfer path, exactly
//! like `shell_render`. The full paint runs once after `phase=12 lcd-init-ok`;
//! afterwards each resolved checklist step repaints only its own line band
//! plus the progress bar, so the splash costs no wall time beyond those small
//! transfers while SD scan / prefs / power init proceed underneath it.

use core::fmt::Write;

use embassy_rp::uart::UartTx;
use koto_core::{
    splash_progress_rect, splash_step_rect, BitmapFont, BootSplash, BootStep, Canvas, Rect,
};

use crate::dashboard::LineBuffer;
use crate::firmware::config::{RASTER_STRIP_BYTES, RGB666_STRIP_BYTES};
use crate::firmware::diag::uart_write_line;
use crate::lcd::PicoCalcLcd;

/// Paint the whole 320x320 splash.
pub async fn paint_splash(
    lcd: &mut PicoCalcLcd<'_>,
    splash: &BootSplash,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    paint_splash_rect(
        lcd,
        splash,
        font,
        strip,
        scratch,
        Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
        },
        uart,
    )
    .await;
}

/// Repaint one resolved checklist step: its line band plus the progress bar.
pub async fn paint_splash_step(
    lcd: &mut PicoCalcLcd<'_>,
    splash: &BootSplash,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    step: BootStep,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    paint_splash_rect(
        lcd,
        splash,
        font,
        strip,
        scratch,
        splash_step_rect(step),
        uart,
    )
    .await;
    paint_splash_rect(
        lcd,
        splash,
        font,
        strip,
        scratch,
        splash_progress_rect(),
        uart,
    )
    .await;
}

/// Strip-banded raster + transfer of one splash rect; same shape as
/// `shell_render::paint_shell_rect`, without the paint metrics (the splash is
/// a one-shot boot path, not a cadence to profile).
async fn paint_splash_rect(
    lcd: &mut PicoCalcLcd<'_>,
    splash: &BootSplash,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rect: Rect,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    let width = rect.w as usize;
    let max_lines = (RASTER_STRIP_BYTES / (width * 2)).max(1);
    let mut local_y = 0usize;
    while local_y < rect.h as usize {
        let y = rect.y as usize + local_y;
        let lines = (rect.h as usize - local_y).min(max_lines);
        let used = width * lines * 2;
        let viewport = Rect {
            x: rect.x,
            y: y as i32,
            w: rect.w,
            h: lines as i32,
        };
        strip[..used].fill(0);
        let mut canvas = Canvas::new_viewport(&mut strip[..used], 320, 320, viewport).unwrap();
        splash.paint_rect(&mut canvas, font, viewport);

        let result = lcd
            .write_rgb565_rect(
                rect.x as u16,
                y as u16,
                rect.w as u16,
                lines as u16,
                &strip[..used],
                scratch,
            )
            .await;
        if result.is_err() {
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=92 rect-error x={} y={} w={} h={}\r\n",
                rect.x, y, width, lines
            );
            uart_write_line(uart, &line);
            break;
        }
        local_y += lines;
    }
}
