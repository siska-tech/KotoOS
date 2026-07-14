//! `probe_power` — STM32 battery/version bridge probe (KOTO-0115).
//!
//! Reads the safe BIOS-version (0x01) and battery-percentage (0x0B) registers
//! from the keyboard STM32 over I2C using the validated 16 ms register-settle
//! interval, and maps the raw bytes to a `PowerState`. Diagnostics are emitted
//! over UART0 (GP0, 115200 8N1).
//!
//! Not part of normal development: flash manually only to re-validate power
//! status. See `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_rp::{
    i2c::{Config as I2cConfig, I2c},
    peripherals,
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::{block_for, Duration, Timer};
use koto_core::hal::PowerState;
use koto_pico::{
    pins::KeyboardPins,
    power::{decode_battery, decode_version, BATTERY_REGISTER, VERSION_REGISTER},
};
use panic_halt as _;

const BUS_HZ: u32 = 100_000;
// ClockworkPi's reference RP2040 reader waits one STM32 service interval after
// selecting a register. Shorter waits returned an unprepared [0, 0] response.
const REGISTER_SETTLE_US: u64 = 16_000;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0115 battery-power-uart v",
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

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);

    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = BUS_HZ;
    let mut bridge = I2c::new_blocking(p.keyboard_i2c, p.keyboard_sda, p.keyboard_scl, i2c_config);

    Timer::after_secs(2).await;
    let _ = uart.blocking_write(BANNER);
    let _ = uart
        .blocking_write(b"log=uart0 tx=GP0 baud=115200 i2c=I2C1 address=0x1f bus_hz=100000\r\n");
    let _ = uart.blocking_write(
        b"capability voltage=false charging=false external_power=false reason=not_exposed_by_stm32_register_protocol\r\n",
    );

    loop {
        let mut line = LineBuffer::new();
        let version_raw = read_register(&mut bridge, VERSION_REGISTER);
        match version_raw {
            Ok((raw, attempts)) => match decode_version(raw) {
                Some(version) => {
                    let _ = write!(
                        line,
                        "firmware available=true register=0x01 raw=[0x{:02x},0x{:02x}] version={}.{} attempts={}\r\n",
                        raw[0], raw[1], version.major, version.minor, attempts
                    );
                }
                None => {
                    let _ = write!(
                        line,
                        "firmware available=false register=0x01 raw=[0x{:02x},0x{:02x}] attempts={} reason=unexpected_marker\r\n",
                        raw[0], raw[1], attempts
                    );
                }
            },
            Err(operation) => {
                let _ = write!(
                    line,
                    "firmware available=false register=0x01 error={}\r\n",
                    operation
                );
            }
        }
        write_line(&mut uart, &line);

        line.clear();
        let battery_raw = read_register(&mut bridge, BATTERY_REGISTER);
        match battery_raw {
            Ok((raw, attempts)) => match decode_battery(raw) {
                Some(state) => {
                    let percent = match state {
                        PowerState::Percent { percent, .. } => percent,
                        PowerState::Charging {
                            percent: Some(percent),
                            ..
                        } => percent,
                        _ => 0,
                    };
                    let _ = write!(
                        line,
                        "battery available=true register=0x0b raw=[0x{:02x},0x{:02x}] percent={} mapped={:?} attempts={}\r\n",
                        raw[0], raw[1], percent, state, attempts
                    );
                }
                None => {
                    let _ = write!(
                        line,
                        "battery available=false register=0x0b raw=[0x{:02x},0x{:02x}] mapped={:?} attempts={} reason=no_battery_or_firmware_unavailable\r\n",
                        raw[0],
                        raw[1],
                        PowerState::unknown(),
                        attempts
                    );
                }
            },
            Err(operation) => {
                let _ = write!(
                    line,
                    "battery available=false register=0x0b error={} mapped={:?}\r\n",
                    operation,
                    PowerState::unsupported()
                );
            }
        }
        write_line(&mut uart, &line);
        let _ = uart.blocking_write(
            b"power mapped charging=encoded_in_battery_bit7 voltage_mv=unsupported external_power=unsupported\r\n",
        );
        let _ = uart.blocking_write(b"KOTO-0115 sample complete; next sample in 5s\r\n");
        Timer::after_secs(5).await;
    }
}

fn read_register(
    bridge: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    register: u8,
) -> Result<([u8; 2], u8), &'static str> {
    let mut last_error = "register_write";
    for attempt in 1..=3 {
        if bridge
            .blocking_write(KeyboardPins::I2C_ADDRESS, &[register])
            .is_err()
        {
            last_error = "register_write";
            block_for(Duration::from_millis(100));
            continue;
        }
        block_for(Duration::from_micros(REGISTER_SETTLE_US));
        let mut raw = [0u8; 2];
        match bridge.blocking_read(KeyboardPins::I2C_ADDRESS, &mut raw) {
            Ok(()) => return Ok((raw, attempt)),
            Err(_) => {
                last_error = "register_read";
                block_for(Duration::from_millis(100));
            }
        }
    }
    Err(last_error)
}

fn write_line(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, line: &LineBuffer) {
    let _ = uart.blocking_write(line.as_bytes());
}

struct LineBuffer {
    bytes: [u8; 320],
    len: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 320],
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
        if text.len() > self.bytes.len().saturating_sub(self.len) {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
