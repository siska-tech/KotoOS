#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts,
    i2c::{Config as I2cConfig, I2c, InterruptHandler as I2cInterruptHandler},
    peripherals,
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Instant, Timer};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use koto_pico::{
    keyboard::{
        key_name, Candidate, HeldKeys, KeyEvent, CANDIDATES, FIFO_CAPACITY, FIFO_REGISTER,
        FRAME_PERIOD_MS, STABLE_SAMPLE_COUNT,
    },
    pins::KeyboardPins,
    power::{decode_version, VERSION_REGISTER},
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
    I2C1_IRQ => I2cInterruptHandler<peripherals::I2C1>;
});

const BUS_HZ: u32 = 100_000;
const REGISTER_SETTLE_US: u64 = 250;
const VERSION_SETTLE_MS: u64 = 16;
const MAX_EVENTS_PER_FRAME: usize = 4;
const ATTEMPT_TIMEOUT_MS: u64 = 15_000;
const MAX_ATTEMPTS: u8 = 3;
const TEST_COUNT: usize = 44;

const UP: u8 = 1 << 0;
const DOWN: u8 = 1 << 1;
const LEFT: u8 = 1 << 2;
const RIGHT: u8 = 1 << 3;
const ACTION_A: u8 = 1 << 4;
const ACTION_B: u8 = 1 << 5;
const ACTION_X: u8 = 1 << 6;
const ACTION_Y: u8 = 1 << 7;

const DIRECTIONS: [u8; 4] = [UP, DOWN, LEFT, RIGHT];
const DIAGONALS: [u8; 4] = [UP | LEFT, UP | RIGHT, DOWN | LEFT, DOWN | RIGHT];
const ACTIONS: [u8; 4] = [ACTION_A, ACTION_B, ACTION_X, ACTION_Y];
const ACTION_PAIRS: [u8; 6] = [
    ACTION_A | ACTION_B,
    ACTION_A | ACTION_X,
    ACTION_A | ACTION_Y,
    ACTION_B | ACTION_X,
    ACTION_B | ACTION_Y,
    ACTION_X | ACTION_Y,
];
const OPPOSITES: [u8; 2] = [UP | DOWN, LEFT | RIGHT];

const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0067 keyboard-matrix v",
    env!("CARGO_PKG_VERSION"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = BUS_HZ;
    let mut keyboard = I2c::new_async(p.I2C1, p.PIN_7, p.PIN_6, Irqs, i2c_config);

    let driver = Driver::new(p.USB, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x0167);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS keyboard matrix probe");
    usb_config.serial_number = Some("KOTO-0067-MATRIX");
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

        loop {
            cdc.wait_connection().await;
            if write_packets(&mut cdc, BANNER).await.is_err() {
                continue;
            }

            let firmware = read_version(&mut keyboard).await;
            line.clear();
            match firmware {
                Ok(Some((major, minor))) => {
                    let _ = write!(
                        line,
                        "{{\"kind\":\"probe\",\"board\":\"picocalc-rp2040\",\"stm32_firmware\":\"{}.{}\",\"bus_hz\":{},\"tests_per_candidate\":{},\"stable_samples\":{},\"timeout_ms\":{}}}\r\n",
                        major,
                        minor,
                        BUS_HZ,
                        TEST_COUNT,
                        STABLE_SAMPLE_COUNT,
                        ATTEMPT_TIMEOUT_MS
                    );
                }
                _ => {
                    let _ = write!(
                        line,
                        "{{\"kind\":\"probe\",\"board\":\"picocalc-rp2040\",\"stm32_firmware\":\"unknown\",\"bus_hz\":{},\"tests_per_candidate\":{},\"stable_samples\":{},\"timeout_ms\":{}}}\r\n",
                        BUS_HZ, TEST_COUNT, STABLE_SAMPLE_COUNT, ATTEMPT_TIMEOUT_MS
                    );
                }
            }
            if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                continue;
            }
            if write_packets(
                &mut cdc,
                b"{\"kind\":\"instruction\",\"message\":\"Release all keys, then hold each prompted chord until result appears.\"}\r\n",
            )
            .await
            .is_err()
            {
                continue;
            }

            let mut selected = None;
            let mut disconnected = false;

            for candidate in CANDIDATES {
                let mut passed = 0usize;
                let mut failed = 0usize;
                let mut held = HeldKeys::new();

                if wait_for_clear(&mut keyboard, &mut held).await.is_err() {
                    disconnected = true;
                    break;
                }

                for test_index in 0..TEST_COUNT {
                    let expected = test_mask(test_index);
                    let mut test_passed = false;

                    for attempt in 1..=MAX_ATTEMPTS {
                        format_prompt(&mut line, candidate, expected, attempt);
                        if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                            disconnected = true;
                            break;
                        }

                        let observation =
                            observe_chord(&mut keyboard, &mut held, candidate, expected).await;
                        format_result(&mut line, candidate, expected, attempt, &held, observation);
                        if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                            disconnected = true;
                            break;
                        }

                        if observation.passed {
                            test_passed = true;
                            break;
                        }

                        if wait_for_clear(&mut keyboard, &mut held).await.is_err() {
                            disconnected = true;
                            break;
                        }
                    }

                    if disconnected {
                        break;
                    }
                    if test_passed {
                        passed += 1;
                    } else {
                        failed += 1;
                    }

                    line.clear();
                    let _ = write!(
                        line,
                        "{{\"kind\":\"instruction\",\"message\":\"Release all keys\",\"candidate\":\"{}\"}}\r\n",
                        candidate.name
                    );
                    if write_packets(&mut cdc, line.as_bytes()).await.is_err()
                        || wait_for_clear(&mut keyboard, &mut held).await.is_err()
                    {
                        disconnected = true;
                        break;
                    }
                }

                if disconnected {
                    break;
                }

                let candidate_passed = failed == 0 && passed == TEST_COUNT;
                line.clear();
                let _ = write!(
                    line,
                    "{{\"kind\":\"candidate_result\",\"candidate\":\"{}\",\"passed\":{},\"failed\":{},\"status\":\"{}\"}}\r\n",
                    candidate.name,
                    passed,
                    failed,
                    if candidate_passed { "pass" } else { "fail" }
                );
                if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                    disconnected = true;
                    break;
                }

                if candidate_passed {
                    selected = Some(candidate.name);
                    break;
                }
            }

            if disconnected {
                continue;
            }

            line.clear();
            if let Some(candidate) = selected {
                let _ = write!(
                    line,
                    "{{\"kind\":\"selection\",\"status\":\"pass\",\"selected_candidate\":\"{}\",\"reason\":\"first_passing_candidate\"}}\r\n",
                    candidate
                );
            } else {
                let _ = write!(
                    line,
                    "{{\"kind\":\"selection\",\"status\":\"fail\",\"selected_candidate\":null,\"reason\":\"no_candidate_passed\"}}\r\n"
                );
            }
            if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                continue;
            }
            let _ = write_packets(
                &mut cdc,
                b"KOTO-0067 matrix capture complete; reset board to restart\r\n",
            )
            .await;
            loop {
                Timer::after_secs(60).await;
            }
        }
    };

    join(usb_task, probe_task).await;
}

#[derive(Clone, Copy)]
struct Observation {
    passed: bool,
    stable_samples: u8,
    poll_us: u64,
    timed_out: bool,
    read_error: bool,
}

async fn observe_chord(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
    held: &mut HeldKeys,
    candidate: Candidate,
    expected: u8,
) -> Observation {
    let started = Instant::now();
    let mut previous = *held;
    let mut stable_samples = 0u8;
    let mut max_poll_us = 0u64;

    loop {
        let frame_start = Instant::now();
        let read_error = poll_events(keyboard, held).await.is_err();
        let poll_us = frame_start.elapsed().as_micros();
        max_poll_us = max_poll_us.max(poll_us);

        if *held == previous {
            stable_samples = stable_samples.saturating_add(1);
        } else {
            previous = *held;
            stable_samples = 1;
        }

        if read_error {
            return Observation {
                passed: false,
                stable_samples,
                poll_us: max_poll_us,
                timed_out: false,
                read_error: true,
            };
        }

        let exact = held_matches(candidate, held, expected);
        if exact && stable_samples >= STABLE_SAMPLE_COUNT {
            return Observation {
                passed: max_poll_us <= 16_667,
                stable_samples,
                poll_us: max_poll_us,
                timed_out: false,
                read_error: false,
            };
        }
        if started.elapsed().as_millis() >= ATTEMPT_TIMEOUT_MS {
            return Observation {
                passed: false,
                stable_samples,
                poll_us: max_poll_us,
                timed_out: true,
                read_error: false,
            };
        }
        wait_frame(frame_start).await;
    }
}

async fn wait_for_clear(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
    held: &mut HeldKeys,
) -> Result<(), ()> {
    let mut stable = 0u8;
    loop {
        let frame_start = Instant::now();
        poll_events(keyboard, held).await?;
        if held.as_slice().is_empty() {
            stable = stable.saturating_add(1);
            if stable >= STABLE_SAMPLE_COUNT {
                return Ok(());
            }
        } else {
            stable = 0;
        }
        wait_frame(frame_start).await;
    }
}

async fn poll_events(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
    held: &mut HeldKeys,
) -> Result<(), ()> {
    let mut events = 0usize;
    while events < FIFO_CAPACITY && events < MAX_EVENTS_PER_FRAME {
        let event = read_event(keyboard).await.map_err(|_| ())?;
        if event.is_empty() {
            break;
        }
        held.apply(event);
        events += 1;
    }
    Ok(())
}

async fn read_event(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
) -> Result<KeyEvent, ()> {
    keyboard
        .write_async(KeyboardPins::I2C_ADDRESS, [FIFO_REGISTER])
        .await
        .map_err(|_| ())?;
    Timer::after_micros(REGISTER_SETTLE_US).await;
    let mut raw = [0u8; 2];
    keyboard
        .read_async(KeyboardPins::I2C_ADDRESS, &mut raw)
        .await
        .map_err(|_| ())?;
    Ok(KeyEvent::from_wire(raw))
}

async fn read_version(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Async>,
) -> Result<Option<(u8, u8)>, ()> {
    keyboard
        .write_async(KeyboardPins::I2C_ADDRESS, [VERSION_REGISTER])
        .await
        .map_err(|_| ())?;
    Timer::after_millis(VERSION_SETTLE_MS).await;
    let mut raw = [0u8; 2];
    keyboard
        .read_async(KeyboardPins::I2C_ADDRESS, &mut raw)
        .await
        .map_err(|_| ())?;
    Ok(decode_version(raw).map(|version| (version.major, version.minor)))
}

async fn wait_frame(start: Instant) {
    let elapsed = start.elapsed().as_millis();
    if elapsed < FRAME_PERIOD_MS {
        Timer::after_millis(FRAME_PERIOD_MS - elapsed).await;
    }
}

fn held_matches(candidate: Candidate, held: &HeldKeys, expected: u8) -> bool {
    let expected_len = expected.count_ones() as usize;
    held.as_slice().len() == expected_len
        && candidate
            .bindings
            .iter()
            .enumerate()
            .all(|(index, (key, _))| {
                held.as_slice().contains(key) == (expected & (1 << index) != 0)
            })
}

fn test_mask(index: usize) -> u8 {
    match index {
        0..=3 => DIRECTIONS[index],
        4..=7 => DIAGONALS[index - 4],
        8..=11 => ACTIONS[index - 8],
        12..=17 => ACTION_PAIRS[index - 12],
        18..=33 => {
            let offset = index - 18;
            DIRECTIONS[offset / ACTIONS.len()] | ACTIONS[offset % ACTIONS.len()]
        }
        34..=41 => {
            let offset = index - 34;
            DIAGONALS[offset / 2] | ACTIONS[offset % 2]
        }
        42..=43 => OPPOSITES[index - 42],
        _ => 0,
    }
}

fn format_prompt(line: &mut LineBuffer, candidate: Candidate, expected: u8, attempt: u8) {
    line.clear();
    let _ = write!(
        line,
        "{{\"kind\":\"prompt\",\"candidate\":\"{}\",\"chord\":\"",
        candidate.name
    );
    write_mask_names(line, candidate, expected);
    let _ = write!(
        line,
        "\",\"attempt\":{},\"max_attempts\":{},\"hold_keys\":[",
        attempt, MAX_ATTEMPTS
    );
    write_expected_keys(line, candidate, expected);
    let _ = write!(line, "]}}\r\n");
}

fn format_result(
    line: &mut LineBuffer,
    candidate: Candidate,
    expected: u8,
    attempt: u8,
    held: &HeldKeys,
    observation: Observation,
) {
    line.clear();
    let _ = write!(
        line,
        "{{\"kind\":\"result\",\"candidate\":\"{}\",\"chord\":\"",
        candidate.name
    );
    write_mask_names(line, candidate, expected);
    let _ = write!(
        line,
        "\",\"attempt\":{},\"status\":\"{}\",\"stable_samples\":{},\"poll_us\":{},\"frame_budget_us\":16667,\"timed_out\":{},\"read_error\":{},\"raw_codes\":[",
        attempt,
        if observation.passed { "pass" } else { "fail" },
        observation.stable_samples,
        observation.poll_us,
        observation.timed_out,
        observation.read_error
    );
    write_numbers(line, held.as_slice());
    let _ = write!(line, "],\"held_keys\":[");
    write_held_keys(line, held);
    let _ = write!(line, "]}}\r\n");
}

fn write_mask_names(line: &mut LineBuffer, candidate: Candidate, mask: u8) {
    for (index, (_, name)) in candidate.bindings.iter().enumerate() {
        if mask & (1 << index) == 0 {
            continue;
        }
        if line.last_byte() != Some(b'"') {
            let _ = line.write_str("+");
        }
        let _ = line.write_str(name);
    }
}

fn write_expected_keys(line: &mut LineBuffer, candidate: Candidate, mask: u8) {
    let mut first = true;
    for (index, (key, _)) in candidate.bindings.iter().enumerate() {
        if mask & (1 << index) == 0 {
            continue;
        }
        if !first {
            let _ = line.write_str(",");
        }
        first = false;
        write_key(line, *key);
    }
}

fn write_held_keys(line: &mut LineBuffer, held: &HeldKeys) {
    for (index, key) in held.as_slice().iter().copied().enumerate() {
        if index != 0 {
            let _ = line.write_str(",");
        }
        write_key(line, key);
    }
}

fn write_key(line: &mut LineBuffer, key: u8) {
    if let Some(name) = key_name(key) {
        let _ = write!(line, "\"{}\"", name);
    } else if key.is_ascii_graphic() && key != b'"' && key != b'\\' {
        let _ = write!(line, "\"{}\"", key as char);
    } else {
        let _ = write!(line, "\"0x{:02x}\"", key);
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

    fn last_byte(&self) -> Option<u8> {
        self.as_bytes().last().copied()
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
