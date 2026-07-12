//! KotoShell launcher rendering: full, rect, and selection-delta repaints.

use core::fmt::Write;

use embassy_rp::uart::UartTx;
use embassy_time::Instant;
use koto_core::{BitmapFont, Canvas, Rect, RenderCommandList, RenderUpdate, ShellState};

use crate::dashboard::LineBuffer;
use crate::firmware::config::{RASTER_STRIP_BYTES, RGB666_STRIP_BYTES};
use crate::firmware::diag::{uart_write_line, PaintMetrics};
use crate::lcd::PicoCalcLcd;

pub async fn paint_shell(
    lcd: &mut PicoCalcLcd<'_>,
    shell: &ShellState,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    let mut metrics = PaintMetrics::default();
    paint_shell_rect(
        lcd,
        shell,
        font,
        strip,
        scratch,
        Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
        },
        &mut metrics,
        uart,
    )
    .await;
    metrics
}

pub async fn paint_shell_rect_metrics(
    lcd: &mut PicoCalcLcd<'_>,
    shell: &ShellState,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rect: Rect,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    let mut metrics = PaintMetrics::default();
    paint_shell_rect(lcd, shell, font, strip, scratch, rect, &mut metrics, uart).await;
    metrics
}

pub async fn paint_selection_change(
    lcd: &mut PicoCalcLcd<'_>,
    shell: &ShellState,
    font: &BitmapFont<'_>,
    previous: usize,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> PaintMetrics {
    let mut commands = RenderCommandList::<8>::new();
    if shell
        .render_selection_change(previous, &mut commands)
        .is_err()
    {
        return paint_shell(lcd, shell, font, strip, scratch, uart).await;
    }
    let mut metrics = PaintMetrics::default();
    for command in commands.iter() {
        match command.update {
            RenderUpdate::Full => {
                return paint_shell(lcd, shell, font, strip, scratch, uart).await;
            }
            RenderUpdate::Rect(rect) => {
                paint_shell_rect(lcd, shell, font, strip, scratch, rect, &mut metrics, uart).await;
            }
            RenderUpdate::Scanlines { y, line_count } => {
                paint_shell_rect(
                    lcd,
                    shell,
                    font,
                    strip,
                    scratch,
                    Rect {
                        x: 0,
                        y: i32::from(y),
                        w: 320,
                        h: i32::from(line_count),
                    },
                    &mut metrics,
                    uart,
                )
                .await;
            }
        }
    }
    metrics
}

#[allow(clippy::too_many_arguments)]
async fn paint_shell_rect(
    lcd: &mut PicoCalcLcd<'_>,
    shell: &ShellState,
    font: &BitmapFont<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rect: Rect,
    metrics: &mut PaintMetrics,
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
        // Raster only the pixels in this band that belong to the dirty rect;
        // `paint_rect` skips the cost of components outside `viewport` rather
        // than rerunning the full shell painter (KOTO-0120 / NFR-DRAW-2).
        let raster_started = Instant::now();
        strip[..used].fill(0);
        let mut canvas = Canvas::new_viewport(&mut strip[..used], 320, 320, viewport).unwrap();
        shell.paint_rect(&mut canvas, font, viewport);
        metrics.record_raster(raster_started);

        let transfer_started = Instant::now();
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
        metrics.record_transfer(transfer_started, (width * lines) as u32);
        local_y += lines;
    }
}
