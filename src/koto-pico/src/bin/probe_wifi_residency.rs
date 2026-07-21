//! Compile/link probe for the concrete KOTO-0227 CYW43 arena layout.
#![no_std]
#![no_main]

use koto_pico::firmware::wifi_residency::{
    WifiDriverResidencyLayout, CYW43_CONTROL_BYTES, CYW43_DRIVER_RESERVE_BYTES,
    CYW43_DRIVER_STORAGE_BYTES, CYW43_NET_DRIVER_BYTES, CYW43_RUNNER_BYTES, CYW43_STATE_BYTES,
    DRIVER_STORAGE_SPARE_BYTES, FETCH_MAILBOX_BYTES,
};
use panic_halt as _;

#[used]
#[unsafe(no_mangle)]
static CYW43_STATE_SIZE: [u8; CYW43_STATE_BYTES] = [0; CYW43_STATE_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_RUNNER_SIZE: [u8; CYW43_RUNNER_BYTES] = [0; CYW43_RUNNER_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_CONTROL_SIZE: [u8; CYW43_CONTROL_BYTES] = [0; CYW43_CONTROL_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_NET_DRIVER_SIZE: [u8; CYW43_NET_DRIVER_BYTES] = [0; CYW43_NET_DRIVER_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_DRIVER_STORAGE_SIZE: [u8; CYW43_DRIVER_STORAGE_BYTES] =
    [0; CYW43_DRIVER_STORAGE_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_DRIVER_RESERVE_SIZE: [u8; CYW43_DRIVER_RESERVE_BYTES] =
    [0; CYW43_DRIVER_RESERVE_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_FETCH_MAILBOX_SIZE: [u8; FETCH_MAILBOX_BYTES] = [0; FETCH_MAILBOX_BYTES];
#[used]
#[unsafe(no_mangle)]
static CYW43_DRIVER_STORAGE_SPARE_SIZE: [u8; DRIVER_STORAGE_SPARE_BYTES] =
    [0; DRIVER_STORAGE_SPARE_BYTES];
#[used]
#[unsafe(no_mangle)]
static WIFI_RESIDENCY_LAYOUT_SIZE: [u8; core::mem::size_of::<WifiDriverResidencyLayout>()] =
    [0; core::mem::size_of::<WifiDriverResidencyLayout>()];

#[cortex_m_rt::entry]
fn main() -> ! {
    loop {
        cortex_m::asm::wfi();
    }
}
