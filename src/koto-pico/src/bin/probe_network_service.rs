//! Compile/link + SRAM-budget probe for the KOTO-0239 bounded embassy-net IP
//! stack. It proves the stack links against this tree's Embassy generation and
//! the concrete cyw43 `NetDriver`, and materializes the fixed-capacity storage
//! costs as named ELF symbols for the residency memory report.
#![no_std]
#![no_main]

use koto_core::net::NetworkService;
use koto_core::net_ui::WifiPageController;
use koto_pico::firmware::wifi_residency::net_stack::{
    NETWORK_STACK_RESOURCES_BYTES, NETWORK_STACK_RUNNER_BYTES, NETWORK_STACK_STORAGE_BYTES,
};
use panic_halt as _;

const NETWORK_SERVICE_BYTES: usize = core::mem::size_of::<NetworkService>();
const NETWORK_PAGE_CONTROLLER_BYTES: usize = core::mem::size_of::<WifiPageController>();

#[used]
#[unsafe(no_mangle)]
static NETWORK_STACK_RESOURCES_SIZE: [u8; NETWORK_STACK_RESOURCES_BYTES] =
    [0; NETWORK_STACK_RESOURCES_BYTES];
#[used]
#[unsafe(no_mangle)]
static NETWORK_STACK_STORAGE_SIZE: [u8; NETWORK_STACK_STORAGE_BYTES] =
    [0; NETWORK_STACK_STORAGE_BYTES];
#[used]
#[unsafe(no_mangle)]
static NETWORK_STACK_RUNNER_SIZE: [u8; NETWORK_STACK_RUNNER_BYTES] =
    [0; NETWORK_STACK_RUNNER_BYTES];
#[used]
#[unsafe(no_mangle)]
static NETWORK_SERVICE_SIZE: [u8; NETWORK_SERVICE_BYTES] = [0; NETWORK_SERVICE_BYTES];
#[used]
#[unsafe(no_mangle)]
static NETWORK_PAGE_CONTROLLER_SIZE: [u8; NETWORK_PAGE_CONTROLLER_BYTES] =
    [0; NETWORK_PAGE_CONTROLLER_BYTES];

#[cortex_m_rt::entry]
fn main() -> ! {
    loop {
        cortex_m::asm::wfi();
    }
}
