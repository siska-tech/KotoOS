#![no_std]
#![no_main]

use cyw43::aligned_bytes;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Level, Output},
    peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Duration, Timer};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State as CdcState},
    Builder, Config,
};
use panic_halt as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<peripherals::PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>;
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
});

static WIFI_FIRMWARE: cyw43::Aligned<cyw43::A4, [u8; 231_077]> =
    cyw43::Aligned(*cyw43_firmware::CYW43_43439A0);

const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0065 Pico W blink+cdc v",
    env!("CARGO_PKG_VERSION"),
    "\r\n"
)
.as_bytes();
const HEARTBEAT: &[u8] = b"KotoOS Pico W probe alive\r\n";

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<
        'static,
        cyw43::SpiBus<Output<'static>, PioSpi<'static, peripherals::PIO0, 0>>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let radio_power = Output::new(peripherals.PIN_23, Level::Low);
    let radio_cs = Output::new(peripherals.PIN_25, Level::High);
    let mut pio = Pio::new(peripherals.PIO0, Irqs);
    let radio_spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        radio_cs,
        peripherals.PIN_24,
        peripherals.PIN_29,
        dma::Channel::new(peripherals.DMA_CH0, Irqs),
    );

    let nvram = aligned_bytes!("../../nvram_rp2040.bin");
    static RADIO_STATE: StaticCell<cyw43::State> = StaticCell::new();
    let radio_state = RADIO_STATE.init(cyw43::State::new());
    let (_net_device, mut radio, runner) =
        cyw43::new(radio_state, radio_power, radio_spi, &WIFI_FIRMWARE, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());
    radio.init(cyw43_firmware::CYW43_43439A0_CLM).await;

    let usb_driver = Driver::new(peripherals.USB, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x1065);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS Pico W probe");
    usb_config.serial_number = Some("KOTO-0065-W");
    usb_config.max_power = 100;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut msos_descriptor = [0; 128];
    let mut control_buf = [0; 64];
    let mut cdc_state = CdcState::new();
    let mut builder = Builder::new(
        usb_driver,
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
        let mut led_on = false;
        let mut heartbeat_ticks = 0u8;

        loop {
            radio.gpio_set(0, led_on).await;
            led_on = !led_on;
            Timer::after(Duration::from_millis(if led_on { 750 } else { 250 })).await;

            heartbeat_ticks += 1;
            if heartbeat_ticks < 4 {
                continue;
            }
            heartbeat_ticks = 0;

            if !cdc.dtr() {
                continue;
            }
            if cdc.write_packet(BANNER).await.is_err() {
                continue;
            }
            let _ = cdc.write_packet(HEARTBEAT).await;
        }
    };

    join(usb_task, probe_task).await;
}
