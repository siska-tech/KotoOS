//! STM32-bridge battery/version polling over the shared I2C bus (KOTO-0115).

use core::fmt::Write;

use embassy_rp::i2c::I2c;
use embassy_rp::peripherals;
use embassy_rp::uart::UartTx;
use embassy_time::{block_for, Duration};
use koto_core::PowerState;

use crate::dashboard::LineBuffer;
use crate::firmware::config::BATTERY_REGISTER_SETTLE_US;
use crate::firmware::diag::uart_write_line;
use crate::pins::KeyboardPins;
use crate::power::{decode_battery, decode_version, BATTERY_REGISTER, VERSION_REGISTER};

pub fn poll_power_state(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<PowerState> {
    if let Some(version_raw) = read_bridge_register(keyboard, VERSION_REGISTER, uart) {
        let mut line = LineBuffer::new();
        match decode_version(version_raw) {
            Some(version) => {
                let _ = write!(
                    line,
                    "phase=147 bridge-version raw=[0x{:02x},0x{:02x}] version={}.{}\r\n",
                    version_raw[0], version_raw[1], version.major, version.minor
                );
            }
            None => {
                let _ = write!(
                    line,
                    "phase=147 bridge-version raw=[0x{:02x},0x{:02x}] unavailable\r\n",
                    version_raw[0], version_raw[1]
                );
            }
        }
        uart_write_line(uart, &line);
    }
    let raw = read_bridge_register(keyboard, BATTERY_REGISTER, uart)?;
    let state = decode_battery(raw).unwrap_or_else(PowerState::unknown);
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=146 battery raw=[0x{:02x},0x{:02x}] state={:?}\r\n",
        raw[0], raw[1], state
    );
    uart_write_line(uart, &line);
    Some(state)
}

fn read_bridge_register(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    register: u8,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<[u8; 2]> {
    for attempt in 1..=3 {
        if let Err(error) = keyboard.blocking_write(KeyboardPins::I2C_ADDRESS, &[register]) {
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=184 bridge-register-write-error register=0x{:02x} attempt={} error={:?}\r\n",
                register, attempt, error
            );
            uart_write_line(uart, &line);
            block_for(Duration::from_millis(100));
            continue;
        }
        block_for(Duration::from_micros(BATTERY_REGISTER_SETTLE_US));
        let mut raw = [0u8; 2];
        match keyboard.blocking_read(KeyboardPins::I2C_ADDRESS, &mut raw) {
            Ok(()) => return Some(raw),
            Err(error) => {
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=185 bridge-register-read-error register=0x{:02x} attempt={} error={:?}\r\n",
                    register, attempt, error
                );
                uart_write_line(uart, &line);
                block_for(Duration::from_millis(100));
            }
        }
    }
    None
}
