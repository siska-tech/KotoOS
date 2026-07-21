//! KotoConfig full and clipped rendering on the PicoCalc LCD.

use embassy_rp::uart::UartTx;
use embassy_time::Instant;
use koto_core::{BitmapFont, Canvas, CanvasUiPainter, KotoConfigUi, KotoConfigWifiUi, Rect};
use koto_ui::UiRect;

use crate::firmware::config::{RASTER_STRIP_BYTES, RGB666_STRIP_BYTES};
use crate::firmware::diag::PaintMetrics;
use crate::lcd::PicoCalcLcd;

pub async fn paint_config(
    lcd: &mut PicoCalcLcd<'_>,
    ui: &KotoConfigUi,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    paint_config_rect(
        lcd,
        ui,
        font,
        strip,
        scratch,
        UiRect::new(0, 0, 320, 320),
        uart,
    )
    .await
}

pub async fn paint_config_rect(
    lcd: &mut PicoCalcLcd<'_>,
    ui: &KotoConfigUi,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rect: UiRect,
    _uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    let mut metrics = PaintMetrics::default();
    if rect.is_empty() {
        return metrics;
    }
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
        let raster_started = Instant::now();
        strip[..used].fill(0);
        let mut canvas = Canvas::new_viewport(&mut strip[..used], 320, 320, viewport).unwrap();
        let mut painter = CanvasUiPainter::new(&mut canvas, font);
        let _ = ui.paint(
            &mut painter,
            UiRect::new(viewport.x, viewport.y, viewport.w, viewport.h),
        );
        metrics.record_raster(raster_started);

        let transfer_started = Instant::now();
        let _ = lcd
            .write_rgb565_rect(
                rect.x as u16,
                y as u16,
                rect.w as u16,
                lines as u16,
                &strip[..used],
                scratch,
            )
            .await;
        metrics.record_transfer(transfer_started, (width * lines) as u32);
        local_y += lines;
    }
    metrics
}

/// Paints the capability-gated native Wi-Fi page through the same bounded
/// scanline adapter as the language page.
pub async fn paint_config_wifi(
    lcd: &mut PicoCalcLcd<'_>,
    ui: &KotoConfigWifiUi,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    paint_config_wifi_rect(
        lcd,
        ui,
        font,
        strip,
        scratch,
        UiRect::new(0, 0, 320, 320),
        uart,
    )
    .await
}

pub async fn paint_config_wifi_rect(
    lcd: &mut PicoCalcLcd<'_>,
    ui: &KotoConfigWifiUi,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rect: UiRect,
    _uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    let mut metrics = PaintMetrics::default();
    if rect.is_empty() {
        return metrics;
    }
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
        let raster_started = Instant::now();
        strip[..used].fill(0);
        let mut canvas = Canvas::new_viewport(&mut strip[..used], 320, 320, viewport).unwrap();
        let mut painter = CanvasUiPainter::new(&mut canvas, font);
        let _ = ui.paint(
            &mut painter,
            UiRect::new(viewport.x, viewport.y, viewport.w, viewport.h),
        );
        metrics.record_raster(raster_started);

        let transfer_started = Instant::now();
        let _ = lcd
            .write_rgb565_rect(
                rect.x as u16,
                y as u16,
                rect.w as u16,
                lines as u16,
                &strip[..used],
                scratch,
            )
            .await;
        metrics.record_transfer(transfer_started, (width * lines) as u32);
        local_y += lines;
    }
    metrics
}
