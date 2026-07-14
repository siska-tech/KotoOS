#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_main)]

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use core::sync::atomic::{compiler_fence, Ordering};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_rp::{
    pac::{
        self,
        dma::vals::{DataSize, TreqSel},
    },
    uart::{Blocking, Config as UartConfig, UartTx},
    Peri,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const DMA_WORD_COUNT: usize = 4;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const DMA_TIMEOUT_POLLS: u32 = 1_000_000;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const STATUS_REPEAT_DELAY_LOOPS: u32 = 1_000_000;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PRE_DMA_BOOT_REPEATS: usize = 300;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut SRC_WORDS: [u32; DMA_WORD_COUNT] = [0; DMA_WORD_COUNT];
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut DST_WORDS: [u32; DMA_WORD_COUNT] = [0; DMA_WORD_COUNT];

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);
    register_panic_uart(&mut uart);

    log_line(&mut uart, "boot via PicoCalc UART-USB");
    repeat_boot_log(&mut uart);
    log_line(&mut uart, "boot ok");
    log_line(&mut uart, "embassy_rp::init start/ok");
    log_line(&mut uart, "dma_pac_probe step=entry");
    log_line(&mut uart, "dma_pac_probe step=uart_ready");

    let src = unsafe { &mut *core::ptr::addr_of_mut!(SRC_WORDS) };
    let dst = unsafe { &mut *core::ptr::addr_of_mut!(DST_WORDS) };
    src.copy_from_slice(&[0x0123_4567, 0x89ab_cdef, 0xfeed_cafe, 0x1357_9bdf]);
    dst.fill(0);

    log_dma_config(&mut uart, src, dst);
    log_line(&mut uart, "dma_pac_probe step=before_register_config");
    configure_dma_ch0_memcpy(src, dst);
    log_line(&mut uart, "dma_pac_probe step=after_register_config");

    log_line(&mut uart, "dma_pac_probe step=before_wait");
    let wait_ok = poll_dma_ch0_until_done(DMA_TIMEOUT_POLLS);
    log_line(&mut uart, "dma_pac_probe step=after_wait");

    let status = dma_ch0_status(wait_ok);
    let ok = status.wait_ok && !status.ahb_error && src == dst;
    loop {
        log_dma_result(&mut uart, src, dst, status, ok);
        if ok {
            log_line(&mut uart, "dma_pac_probe status=ok");
        } else {
            log_line(&mut uart, "dma_pac_probe status=error");
        }
        log_line(&mut uart, "dma_pac_probe alive");
        delay();
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn configure_dma_ch0_memcpy(src: &[u32; DMA_WORD_COUNT], dst: &mut [u32; DMA_WORD_COUNT]) {
    let ch = pac::DMA.ch(0);

    ch.ctrl_trig().write(|w| {
        w.set_chain_to(0);
        w.set_en(false);
    });
    ch.read_addr().write_value(src.as_ptr() as u32);
    ch.write_addr().write_value(dst.as_mut_ptr() as u32);
    ch.trans_count().write(|w| {
        *w = DMA_WORD_COUNT as u32;
    });

    compiler_fence(Ordering::SeqCst);
    ch.ctrl_trig().write(|w| {
        w.set_treq_sel(TreqSel::PERMANENT);
        w.set_data_size(DataSize::SIZE_WORD);
        w.set_incr_read(true);
        w.set_incr_write(true);
        w.set_chain_to(0);
        w.set_irq_quiet(true);
        w.set_bswap(false);
        w.set_sniff_en(false);
        w.set_en(true);
    });
    compiler_fence(Ordering::SeqCst);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn poll_dma_ch0_until_done(timeout_polls: u32) -> bool {
    let ch = pac::DMA.ch(0);
    for _ in 0..timeout_polls {
        let ctrl = ch.ctrl_trig().read();
        if ctrl.ahb_error() {
            return false;
        }
        if !ctrl.busy() {
            return true;
        }
    }

    false
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Clone, Copy)]
struct DmaPacStatus {
    wait_ok: bool,
    busy: bool,
    read_error: bool,
    write_error: bool,
    ahb_error: bool,
    ctrl_trig: u32,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn dma_ch0_status(wait_ok: bool) -> DmaPacStatus {
    let ctrl = pac::DMA.ch(0).ctrl_trig().read();
    DmaPacStatus {
        wait_ok,
        busy: ctrl.busy(),
        read_error: ctrl.read_error(),
        write_error: ctrl.write_error(),
        ahb_error: ctrl.ahb_error(),
        ctrl_trig: ctrl.0,
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn picocalc_uart_usb_tx(
    uart0: Peri<'static, embassy_rp::peripherals::UART0>,
    tx: Peri<'static, embassy_rp::peripherals::PIN_0>,
) -> UartTx<'static, Blocking> {
    let mut config = UartConfig::default();
    config.baudrate = PICOCALC_UART_USB_BAUD;

    // PicoCalc UART-USB bridge: RP2040 UART0 TX on GP0. RX/GP1 is unused for logs.
    UartTx::new_blocking(uart0, tx, config)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut PANIC_UART: *mut UartTx<'static, Blocking> = core::ptr::null_mut();

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn register_panic_uart(uart: &mut UartTx<'static, Blocking>) {
    unsafe {
        PANIC_UART = uart as *mut _;
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn repeat_boot_log(uart: &mut UartTx<'static, Blocking>) {
    for _ in 0..PRE_DMA_BOOT_REPEATS {
        delay();
        log_line(uart, "booting...");
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn delay() {
    for _ in 0..STATUS_REPEAT_DELAY_LOOPS {
        core::hint::spin_loop();
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        if let Some(uart) = PANIC_UART.as_mut() {
            log_str(uart, "panic");
            if let Some(location) = info.location() {
                log_str(uart, " at ");
                log_str(uart, location.file());
                log_byte(uart, b':');
                log_dec_u32(uart, location.line());
            }
            log_newline(uart);
        }
    }
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dma_config(
    uart: &mut UartTx<'static, Blocking>,
    src: &[u32; DMA_WORD_COUNT],
    dst: &[u32; DMA_WORD_COUNT],
) {
    log_str(uart, "dma_pac_probe config src_addr=0x");
    log_hex_u32(uart, src.as_ptr() as u32);
    log_str(uart, " dst_addr=0x");
    log_hex_u32(uart, dst.as_ptr() as u32);
    log_str(uart, " count=");
    log_dec_usize(uart, DMA_WORD_COUNT);
    log_str(uart, " transfer_size=");
    log_dec_usize(uart, core::mem::size_of::<u32>());
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dma_result(
    uart: &mut UartTx<'static, Blocking>,
    src: &[u32; DMA_WORD_COUNT],
    dst: &[u32; DMA_WORD_COUNT],
    status: DmaPacStatus,
    ok: bool,
) {
    log_str(uart, "dma_pac_probe final_state src_addr=0x");
    log_hex_u32(uart, src.as_ptr() as u32);
    log_str(uart, " dst_addr=0x");
    log_hex_u32(uart, dst.as_ptr() as u32);
    log_str(uart, " count=");
    log_dec_usize(uart, DMA_WORD_COUNT);
    log_str(uart, " transfer_size=");
    log_dec_usize(uart, core::mem::size_of::<u32>());
    log_str(uart, " dst0=0x");
    log_hex_u32(uart, dst[0]);
    log_str(uart, " dst1=0x");
    log_hex_u32(uart, dst[1]);
    log_str(uart, " dst2=0x");
    log_hex_u32(uart, dst[2]);
    log_str(uart, " dst3=0x");
    log_hex_u32(uart, dst[3]);
    log_str(uart, " wait_ok=");
    log_bool(uart, status.wait_ok);
    log_str(uart, " busy=");
    log_bool(uart, status.busy);
    log_str(uart, " read_error=");
    log_bool(uart, status.read_error);
    log_str(uart, " write_error=");
    log_bool(uart, status.write_error);
    log_str(uart, " ahb_error=");
    log_bool(uart, status.ahb_error);
    log_str(uart, " ctrl_trig=0x");
    log_hex_u32(uart, status.ctrl_trig);
    log_str(uart, " status=");
    if ok {
        log_str(uart, "ok");
    } else {
        log_str(uart, "error");
    }
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_line(uart: &mut UartTx<'static, Blocking>, line: &str) {
    log_str(uart, line);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_str(uart: &mut UartTx<'static, Blocking>, text: &str) {
    let _ = uart.blocking_write(text.as_bytes());
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_newline(uart: &mut UartTx<'static, Blocking>) {
    let _ = uart.blocking_write(b"\r\n");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_byte(uart: &mut UartTx<'static, Blocking>, byte: u8) {
    let _ = uart.blocking_write(&[byte]);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_bool(uart: &mut UartTx<'static, Blocking>, value: bool) {
    if value {
        log_str(uart, "true");
    } else {
        log_str(uart, "false");
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_hex_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
        log_nibble(uart, ((value >> shift) & 0x0f) as u8);
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_nibble(uart: &mut UartTx<'static, Blocking>, nibble: u8) {
    let byte = if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    };
    log_byte(uart, byte);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_usize(uart: &mut UartTx<'static, Blocking>, value: usize) {
    log_dec_u32(uart, value as u32);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_u32(uart: &mut UartTx<'static, Blocking>, mut value: u32) {
    let mut buf = [0u8; 10];
    let mut len = 0;

    if value == 0 {
        log_byte(uart, b'0');
        return;
    }

    while value > 0 {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    while len > 0 {
        len -= 1;
        log_byte(uart, buf[len]);
    }
}
