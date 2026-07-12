#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[entry]
fn main() -> ! {
    let _peripherals = embassy_rp::init(Default::default());

    loop {
        cortex_m::asm::wfi();
    }
}
