//! Target-layout SRAM probe for KOTO-0245's portable Fetch control plane.
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use koto_core::{
    AppFetchService, FetchAllowlist, FetchPinTable, FetchTransportMailbox, HttpResponseDecoder,
    UnavailableFetchBackend,
};
use panic_halt as _;

const SERVICE_BYTES: usize = core::mem::size_of::<AppFetchService<UnavailableFetchBackend>>();
const ALLOWLIST_BYTES: usize = core::mem::size_of::<FetchAllowlist>();
const DECODER_BYTES: usize = core::mem::size_of::<HttpResponseDecoder>();
const PIN_TABLE_BYTES: usize = core::mem::size_of::<FetchPinTable>();
const TRANSPORT_MAILBOX_BYTES: usize = core::mem::size_of::<FetchTransportMailbox>();

#[used]
#[unsafe(no_mangle)]
static APP_FETCH_SERVICE_CONTROL_SIZE: [u8; SERVICE_BYTES] = [0; SERVICE_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_ALLOWLIST_SIZE: [u8; ALLOWLIST_BYTES] = [0; ALLOWLIST_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_HTTP_DECODER_SIZE: [u8; DECODER_BYTES] = [0; DECODER_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_PIN_TABLE_SIZE: [u8; PIN_TABLE_BYTES] = [0; PIN_TABLE_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TRANSPORT_MAILBOX_SIZE: [u8; TRANSPORT_MAILBOX_BYTES] =
    [0; TRANSPORT_MAILBOX_BYTES];

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let _peripherals = embassy_rp::init(Default::default());
    loop {
        cortex_m::asm::wfi();
    }
}
