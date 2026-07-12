#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_rp::{
    bind_interrupts,
    gpio::{Level, Output},
    peripherals,
    usb::{Driver, InterruptHandler},
};
use embassy_time::{Duration, Timer};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<peripherals::USB>;
});

const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0065 blink+cdc v",
    env!("CARGO_PKG_VERSION"),
    "\r\n"
)
.as_bytes();
const HEARTBEAT: &[u8] = b"KotoOS probe alive\r\n";

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    let mut led = Output::new(peripherals.PIN_25, Level::Low);

    let driver = Driver::new(peripherals.USB, Irqs);
    let mut config = Config::new(0xc0de, 0x0065);
    config.manufacturer = Some("KotoOS");
    config.product = Some("KotoOS RP2040 probe");
    config.serial_number = Some("KOTO-0065");
    config.max_power = 100;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut msos_descriptor = [0; 128];
    let mut control_buf = [0; 64];

    let mut cdc_state = State::new();
    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );
    let mut cdc = CdcAcmClass::new(&mut builder, &mut cdc_state, 64);
    let mut usb = builder.build();

    let usb_task = usb.run();
    let log_task = async {
        loop {
            cdc.wait_connection().await;
            if cdc.write_packet(BANNER).await.is_err() {
                continue;
            }

            loop {
                Timer::after_secs(2).await;
                if cdc.write_packet(HEARTBEAT).await.is_err() {
                    break;
                }
            }
        }
    };
    let blink_task = async {
        loop {
            led.set_high();
            Timer::after(Duration::from_millis(250)).await;
            led.set_low();
            Timer::after(Duration::from_millis(750)).await;
        }
    };

    join3(usb_task, log_task, blink_task).await;
}
