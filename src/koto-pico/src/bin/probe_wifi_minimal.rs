//! Minimal Pico 2 W CYW43 control probe for KOTO-0227.
#![no_std]
#![no_main]

use core::{fmt::Write, future::pending};

use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Level, Output},
    peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::Timer;
use koto_pico::{
    dashboard::LineBuffer,
    firmware::{
        diag::{uart_log, uart_write_line},
        wifi_residency::wifi_spi_telemetry,
    },
};
use panic_halt as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<peripherals::PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>, dma::InterruptHandler<peripherals::DMA_CH1>, dma::InterruptHandler<peripherals::DMA_CH2>, dma::InterruptHandler<peripherals::DMA_CH3>;
});

static WIFI_FIRMWARE: cyw43::Aligned<cyw43::A4, [u8; 231_077]> =
    cyw43::Aligned(*cyw43_firmware::CYW43_43439A0);
static RADIO_STATE: StaticCell<cyw43::State> = StaticCell::new();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = koto_pico::board::split_minimal_radio_probe(embassy_rp::init(Default::default()));
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);
    #[cfg(not(any(
        feature = "wifi_minimal_dma23_probe",
        feature = "wifi_minimal_cooperative_probe"
    )))]
    uart_log(
        &mut uart,
        "phase=227 wifi-minimal start transport=direct-pio dma=0,1 divider=rm2\r\n",
    );
    #[cfg(all(
        feature = "wifi_minimal_dma23_probe",
        not(feature = "wifi_minimal_cooperative_probe")
    ))]
    uart_log(
        &mut uart,
        "phase=227 wifi-minimal start transport=direct-pio dma=2,3 divider=rm2\r\n",
    );
    #[cfg(feature = "wifi_minimal_cooperative_probe")]
    uart_log(
        &mut uart,
        "phase=227 wifi-minimal start transport=cooperative-pio dma=2,3 divider=rm2\r\n",
    );

    let power = Output::new(p.power, Level::Low);
    let cs = Output::new(p.cs, Level::High);
    let mut pio = Pio::new(p.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        RM2_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.data,
        p.clock,
        dma::Channel::new(p.dma_tx, Irqs),
        dma::Channel::new(p.dma_rx, Irqs),
    );
    #[cfg(feature = "wifi_minimal_cooperative_probe")]
    let spi = koto_pico::firmware::wifi_residency::CooperativePioSpi::new(spi);
    let nvram = cyw43::aligned_bytes!("../../nvram_rp2040.bin");
    let driver = cyw43::new(
        RADIO_STATE.init(cyw43::State::new()),
        power,
        spi,
        &WIFI_FIRMWARE,
        nvram,
    );
    let (_net_driver, mut control, runner) = match select(driver, Timer::after_secs(10)).await {
        Either::First(driver) => driver,
        Either::Second(()) => {
            uart_log(&mut uart, "phase=227 wifi-minimal driver-timeout\r\n");
            #[cfg(feature = "wifi_minimal_cooperative_probe")]
            {
                let telemetry = wifi_spi_telemetry();
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=227 wifi-minimal spi reads={} writes={} status=0x{:08x} word=0x{:08x}\r\n",
                    telemetry.reads,
                    telemetry.writes,
                    telemetry.last_status,
                    telemetry.last_word
                );
                uart_write_line(&mut uart, &line);
            }
            pending::<()>().await;
            unreachable!()
        }
    };
    uart_log(&mut uart, "phase=227 wifi-minimal driver-ready\r\n");

    let radio_init = select(
        runner.run(),
        control.init(cyw43_firmware::CYW43_43439A0_CLM),
    );
    match select(radio_init, Timer::after_secs(10)).await {
        Either::First(Either::Second(())) => {
            uart_log(&mut uart, "phase=227 wifi-minimal radio-ready\r\n");
        }
        Either::Second(()) => {
            uart_log(&mut uart, "phase=227 wifi-minimal clm-timeout\r\n");
        }
        Either::First(Either::First(_)) => unreachable!(),
    }
    pending::<()>().await;
}
