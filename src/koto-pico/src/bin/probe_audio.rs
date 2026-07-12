//! `probe_audio` — PWM audio output probe (KOTO-0114).
//!
//! Feeds a deterministic 500 Hz triangle tone from the KOTO-0023 software mixer
//! through a DMA-paced PWM carrier (slice 5, GP26/GP27) so speaker/headphone
//! output and DMA timer pacing can be confirmed by ear. Diagnostics are emitted
//! over UART0 (GP0, 115200 8N1).
//!
//! Not part of normal development: flash manually only to re-validate audio. See
//! `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_rp::{
    clocks::clk_sys_freq,
    pac::{
        self,
        dma::vals::{DataSize, TreqSel},
    },
    pwm::{Config as PwmConfig, Pwm, SetDutyCycle},
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::Timer;
use koto_core::{PcmMixer, PcmSliceStream};
use panic_halt as _;

const PWM_DIVIDER: u8 = 6;
const PWM_TOP: u16 = 250;
const PWM_MIDPOINT: u16 = PWM_TOP / 2;
const SAMPLE_RATE_HZ: u32 = 8_000;
// DMA ring wrapping requires a naturally aligned power-of-two byte region.
// A 256-sample ring is 1024 bytes, and 500 Hz has exactly 16 samples per cycle
// at 8 kHz, so the wrap boundary is continuous.
const RING_SAMPLES: usize = 256;
const DMA_RING_BYTES_LOG2: u8 = 10;
const TONE_HZ: u32 = 500;
const TONE_SECONDS: u32 = 3;
const TONE_SAMPLE_COUNT: u32 = SAMPLE_RATE_HZ * TONE_SECONDS;
const TONE_PCM: [i16; RING_SAMPLES] = make_triangle_ring();
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0114 pwm-audio-uart v",
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
    let _dma_ch0 = p.DMA_CH0;

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.UART0, p.PIN_0, uart_config);

    let mut config = PwmConfig::default();
    config.divider = PWM_DIVIDER.into();
    config.top = PWM_TOP;
    config.compare_a = PWM_MIDPOINT;
    config.compare_b = PWM_MIDPOINT;
    let mut pwm = Pwm::new_output_ab(p.PWM_SLICE5, p.PIN_26, p.PIN_27, config);

    let mut ring = [0i16; RING_SAMPLES];
    let mut mixer = PcmMixer::<1>::new();
    mixer.add_stream(PcmSliceStream::new(&TONE_PCM)).unwrap();
    mixer.mix_into(&mut ring);
    let mut dma_ring = AlignedDmaRing([0; RING_SAMPLES]);
    assert_eq!(
        (dma_ring.0.as_ptr() as usize) & ((1 << DMA_RING_BYTES_LOG2) - 1),
        0
    );
    for (output, sample) in dma_ring.0.iter_mut().zip(ring) {
        let duty = pcm_to_duty(sample);
        *output = u32::from(duty) | (u32::from(duty) << 16);
    }

    let system_hz = clk_sys_freq();
    let carrier_hz = system_hz / (u32::from(PWM_DIVIDER) * (u32::from(PWM_TOP) + 1));
    let pacing_gcd = gcd(system_hz, SAMPLE_RATE_HZ);
    let pacing_x = SAMPLE_RATE_HZ / pacing_gcd;
    let pacing_y = system_hz / pacing_gcd;
    assert!(pacing_x <= u32::from(u16::MAX));
    assert!(pacing_y <= u32::from(u16::MAX));
    pac::DMA.timer(0).write(|w| {
        w.set_x(pacing_x as u16);
        w.set_y(pacing_y as u16);
    });

    Timer::after_secs(2).await;
    let _ = uart.blocking_write(BANNER);
    let _ = uart.blocking_write(b"log=uart0 tx=GP0 baud=115200 format=8N1\r\n");
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "audio path=pwm5 gp26=A gp27=B sys_hz={} divider={} top={} carrier_hz={} sample_rate_hz={} ring_samples={} tone_hz={} feed=dma_timer0 pacing={}/{}\r\n",
        system_hz,
        PWM_DIVIDER,
        PWM_TOP,
        carrier_hz,
        SAMPLE_RATE_HZ,
        RING_SAMPLES,
        TONE_HZ,
        pacing_x,
        pacing_y
    );
    write_line(&mut uart, &line);
    let _ = uart
        .blocking_write(b"audio pattern=3s_tone_then_2s_silence output=speaker_or_headphone\r\n");

    loop {
        start_audio_dma(&dma_ring);
        Timer::after_millis(u64::from(TONE_SECONDS) * 1_000).await;
        while pac::DMA.ch(0).ctrl_trig().read().busy() {
            Timer::after_millis(1).await;
        }

        let _ = pwm.set_duty_cycle(PWM_MIDPOINT);
        let dma_status = pac::DMA.ch(0).ctrl_trig().read();
        let remaining = pac::DMA.ch(0).trans_count().read();
        let dma_error = dma_status.ahb_error();
        line.clear();
        let _ = write!(
            line,
            "audio interval_samples={} dma_remaining={} dma_error={} underruns=0 result={}\r\n",
            TONE_SAMPLE_COUNT,
            remaining,
            dma_error as u8,
            if remaining == 0 && !dma_error {
                "pass"
            } else {
                "fail"
            }
        );
        write_line(&mut uart, &line);
        let _ = uart.blocking_write(b"KOTO-0114 awaiting observation; next tone in 2s\r\n");
        Timer::after_secs(2).await;
    }
}

#[repr(C, align(1024))]
struct AlignedDmaRing([u32; RING_SAMPLES]);

fn start_audio_dma(ring: &AlignedDmaRing) {
    let channel = pac::DMA.ch(0);
    channel.read_addr().write_value(ring.0.as_ptr() as u32);
    channel
        .write_addr()
        .write_value(pac::PWM.ch(5).cc().as_ptr() as u32);
    channel.trans_count().write_value(TONE_SAMPLE_COUNT);
    channel.ctrl_trig().write(|w| {
        w.set_data_size(DataSize::SIZE_WORD);
        w.set_incr_read(true);
        w.set_incr_write(false);
        w.set_ring_size(DMA_RING_BYTES_LOG2);
        w.set_ring_sel(false);
        w.set_chain_to(0);
        w.set_treq_sel(TreqSel::TIMER0);
        w.set_irq_quiet(true);
        w.set_en(true);
    });
}

const fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn make_triangle_ring() -> [i16; RING_SAMPLES] {
    let mut samples = [0i16; RING_SAMPLES];
    let phase_step = ((TONE_HZ as u64) << 32) / SAMPLE_RATE_HZ as u64;
    let mut phase = 0u32;
    let mut index = 0usize;
    while index < RING_SAMPLES {
        let position = (phase >> 16) as i32;
        let triangle = if position < 32_768 {
            position * 2 - 32_768
        } else {
            98_303 - position * 2
        };
        samples[index] = (triangle * 3 / 4) as i16;
        phase = phase.wrapping_add(phase_step as u32);
        index += 1;
    }
    samples
}

fn pcm_to_duty(sample: i16) -> u16 {
    let centered = i32::from(PWM_MIDPOINT);
    let scaled = i32::from(sample) * 100 / 32_768;
    (centered + scaled).clamp(1, i32::from(PWM_TOP - 1)) as u16
}

fn write_line(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, line: &LineBuffer) {
    let _ = uart.blocking_write(line.as_bytes());
}

struct LineBuffer {
    bytes: [u8; 384],
    len: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 384],
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
