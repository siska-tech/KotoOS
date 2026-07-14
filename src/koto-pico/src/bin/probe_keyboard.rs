//! `probe_keyboard` — STM32 keyboard FIFO probe over I2C (KOTO-0067).
//!
//! Polls the keyboard bridge at I2C address 0x1F (FIFO register 0x09), reports
//! raw press/release codes, per-poll latency against the 16.67 ms frame budget,
//! and the normalized buttons each candidate mapping would detect. Output is
//! JSONL over UART0 and, when connected, USB CDC. This is the canonical
//! keyboard bring-up probe; the
//! one-time chord-matrix campaign that selected `arrow-zxas` is archived under
//! `bringup/archive/keyboard_matrix.rs`.
//!
//! Not part of normal development: flash manually only to re-validate the
//! keyboard. See `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts,
    i2c::{Config as I2cConfig, I2c, InterruptHandler as I2cInterruptHandler},
    peripherals,
    uart::{Config as UartConfig, UartTx},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Instant, Timer};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use koto_pico::{
    board::BOARD_ID,
    keyboard::{
        key_name, HeldKeys, KeyEvent, CANDIDATES, FIFO_CAPACITY, FIFO_REGISTER, FRAME_PERIOD_MS,
        STABLE_SAMPLE_COUNT,
    },
    pins::KeyboardPins,
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
    I2C1_IRQ => I2cInterruptHandler<peripherals::I2C1>;
});

// Physical validation showed that the keyboard bridge requires the PicoCalc
// mainboard to be powered separately from the Pico USB connection.
const BUS_HZ: u32 = 100_000;
// Keep the register-to-read gap short enough for the frame budget. The powered
// hardware run determines whether this bridge firmware accepts the interval.
const REGISTER_SETTLE_US: u64 = 250;
// Bound FIFO draining so repeated HOLD events cannot consume an entire frame.
// Remaining events stay queued in the STM32 bridge for the following frame.
const MAX_EVENTS_PER_FRAME: usize = 4;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0067 keyboard-i2c v",
    env!("CARGO_PKG_VERSION"),
    " board=",
    env!("KOTO_BOARD_ID"),
    " mcu=",
    env!("KOTO_MCU_ID"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));

    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = BUS_HZ;
    let mut keyboard = I2c::new_async(
        p.keyboard_i2c,
        p.keyboard_sda,
        p.keyboard_scl,
        Irqs,
        i2c_config,
    );

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);

    let driver = Driver::new(p.usb, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x0067);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS keyboard I2C probe");
    usb_config.serial_number = Some("KOTO-0067");
    usb_config.max_power = 100;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut msos_descriptor = [0; 128];
    let mut control_buf = [0; 64];
    let mut cdc_state = State::new();
    let mut builder = Builder::new(
        driver,
        usb_config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );
    let mut cdc = CdcAcmClass::new(&mut builder, &mut cdc_state, 64);
    let mut usb = builder.build();

    let usb_task = usb.run();
    let probe_task = async {
        let mut line = LineBuffer::new();
        let mut held = HeldKeys::new();
        let mut previous = held;
        let mut stable_samples = 0u8;
        let mut sequence = 0u32;
        let mut usb_was_connected = false;

        let _ = uart.blocking_write(BANNER);
        let _ =
            uart.blocking_write(b"log=uart0 tx=GP0 baud=115200 format=jsonl usb_cdc=optional\r\n");
        write_headers_uart(&mut uart, &mut line);

        // Polling starts immediately. Native USB CDC is mirrored when present,
        // but PicoCalc's mainboard UART path never has to satisfy a USB wait.
        loop {
            let usb_connected = cdc.dtr();
            if usb_connected && !usb_was_connected {
                let _ = write_packets(&mut cdc, BANNER).await;
                for candidate in CANDIDATES {
                    format_header(&mut line, candidate.name);
                    let _ = write_packets(&mut cdc, line.as_bytes()).await;
                }
            }
            usb_was_connected = usb_connected;

            let frame_start = Instant::now();
            let mut changed = false;
            let mut read_error = None;
            let mut events = 0usize;

            while events < FIFO_CAPACITY && events < MAX_EVENTS_PER_FRAME {
                match read_event(&mut keyboard).await {
                    Ok(event) if event.is_empty() => break,
                    Ok(event) => {
                        changed |= held.apply(event);
                        events += 1;
                    }
                    Err(operation) => {
                        read_error = Some(operation);
                        break;
                    }
                }
            }

            if held == previous {
                stable_samples = stable_samples.saturating_add(1);
            } else {
                stable_samples = 1;
                previous = held;
                changed = true;
            }
            let stable = stable_samples >= STABLE_SAMPLE_COUNT;
            let poll_us = frame_start.elapsed().as_micros();

            if let Some(operation) = read_error {
                line.clear();
                let _ = write!(
                    line,
                    "{{\"kind\":\"error\",\"seq\":{},\"bus_hz\":{},\"operation\":\"{}\",\"poll_us\":{}}}\r\n",
                    sequence, BUS_HZ, operation, poll_us
                );
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
            } else if changed || stable_samples == STABLE_SAMPLE_COUNT {
                for candidate in CANDIDATES {
                    format_sample(
                        &mut line,
                        sequence,
                        candidate.name,
                        &held,
                        candidate.detected(&held),
                        stable,
                        poll_us,
                    );
                    emit(&mut uart, &mut cdc, line.as_bytes()).await;
                }
            }

            sequence = sequence.wrapping_add(1);
            let elapsed_ms = frame_start.elapsed().as_millis();
            if elapsed_ms < FRAME_PERIOD_MS {
                Timer::after_millis(FRAME_PERIOD_MS - elapsed_ms).await;
            }
        }
    };

    join(usb_task, probe_task).await;
}

fn format_header(line: &mut LineBuffer, candidate: &str) {
    line.clear();
    let _ = write!(
        line,
        "{{\"kind\":\"keyboard_matrix_v1\",\"board\":\"{}\",\"firmware\":\"unknown\",\"bus_hz\":{},\"candidate\":\"{}\",\"git\":\"unknown\",\"frame_ms\":{},\"stable_samples\":{}}}\r\n",
        BOARD_ID, BUS_HZ, candidate, FRAME_PERIOD_MS, STABLE_SAMPLE_COUNT
    );
}

fn write_headers_uart(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, line: &mut LineBuffer) {
    for candidate in CANDIDATES {
        format_header(line, candidate.name);
        let _ = uart.blocking_write(line.as_bytes());
    }
}

async fn emit<'a>(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    bytes: &[u8],
) {
    let _ = uart.blocking_write(bytes);
    if cdc.dtr() {
        let _ = write_packets(cdc, bytes).await;
    }
}

async fn read_event(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
) -> Result<KeyEvent, &'static str> {
    keyboard
        .write_async(KeyboardPins::I2C_ADDRESS, [FIFO_REGISTER])
        .await
        .map_err(|_| "fifo_register_write")?;
    Timer::after_micros(REGISTER_SETTLE_US).await;
    let mut raw = [0u8; 2];
    keyboard
        .read_async(KeyboardPins::I2C_ADDRESS, &mut raw)
        .await
        .map_err(|_| "fifo_data_read")?;
    Ok(KeyEvent::from_wire(raw))
}

fn format_sample(
    line: &mut LineBuffer,
    sequence: u32,
    candidate: &str,
    held: &HeldKeys,
    detected: impl Iterator<Item = &'static str>,
    stable: bool,
    poll_us: u64,
) {
    line.clear();
    let _ = write!(
        line,
        "{{\"kind\":\"sample\",\"t_ms\":{},\"seq\":{},\"candidate\":\"{}\",\"chord\":\"observed\",\"held_keys\":[",
        Instant::now().as_millis(),
        sequence,
        candidate
    );
    write_key_names(line, held);
    let _ = write!(line, "],\"raw_codes\":[");
    write_numbers(line, held.as_slice());
    let _ = write!(line, "],\"detected\":[");
    write_strings(line, detected);
    let _ = write!(
        line,
        "],\"unexpected\":[],\"missing\":[],\"stable\":{},\"poll_us\":{},\"frame_budget_us\":16667}}\r\n",
        stable, poll_us
    );
}

fn write_key_names(line: &mut LineBuffer, held: &HeldKeys) {
    for (index, key) in held.as_slice().iter().copied().enumerate() {
        if index != 0 {
            let _ = line.write_str(",");
        }
        if let Some(name) = key_name(key) {
            let _ = write!(line, "\"{}\"", name);
        } else if key.is_ascii_graphic() && key != b'"' && key != b'\\' {
            let _ = write!(line, "\"{}\"", key as char);
        } else {
            let _ = write!(line, "\"0x{:02x}\"", key);
        }
    }
}

fn write_numbers(line: &mut LineBuffer, values: &[u8]) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            let _ = line.write_str(",");
        }
        let _ = write!(line, "{}", value);
    }
}

fn write_strings(line: &mut LineBuffer, values: impl Iterator<Item = &'static str>) {
    for (index, value) in values.enumerate() {
        if index != 0 {
            let _ = line.write_str(",");
        }
        let _ = write!(line, "\"{}\"", value);
    }
}

async fn write_packets<'a>(
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    bytes: &[u8],
) -> Result<(), ()> {
    for chunk in bytes.chunks(64) {
        cdc.write_packet(chunk).await.map_err(|_| ())?;
    }
    Ok(())
}

struct LineBuffer {
    bytes: [u8; 768],
    len: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 768],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for LineBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len);
        if text.len() > remaining {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
