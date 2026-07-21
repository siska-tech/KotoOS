//! KOTO-0239 firmware NetworkService binding.
//!
//! This module bridges the portable, poll-driven `koto_core::net::NetworkService`
//! to the async cyw43 radio. The async work is quarantined in an arena-resident
//! command loop that owns cyw43 `Control`; CPU0 talks to it only through a
//! bounded static mailbox. Both the `NetworkService::service` call and the
//! command loop run cooperatively on CPU0 (CPU1 owns audio and never touches
//! this state), so a `critical_section`-guarded `RefCell` is sufficient — there
//! is no cross-core sharing and no borrow is ever held across an `await`.
//!
//! Only radio association is bound here (`set_radio`, `scan`, `connect`,
//! `disconnect`); the embassy-net IP stack (DHCP/sockets) is a later step. The
//! `NetworkService` `Connected` state means associated, matching this layer.

use core::cell::RefCell;
use core::future::poll_fn;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::Poll;

use critical_section::Mutex;
use cyw43::{Control, JoinAuth, JoinError, JoinOptions, PowerManagementMode, ScanOptions};
use koto_core::net::{
    CredentialProvider, ForgetOutcome, HalFault, HalPoll, NetworkService, RawScanResult,
    RegulatoryRegion, Security, Ssid, BSSID_BYTES, CREDENTIAL_MAX_BYTES, SCAN_RESULTS_MAX,
    SSID_MAX_BYTES,
};

pub const SNTP_SERVER_MAX_BYTES: usize = 24;

/// The signed release/product regulatory region policy (KOTO-0224). cyw43 0.7
/// applies its own `WORLD_WIDE_XX` domain inside `Control::init`; this is the
/// KotoOS-side gate that must resolve to a supported region before the radio is
/// enabled. An absent/unsupported policy keeps the radio (and `WIFI_CONFIG`)
/// off without blocking offline boot. It is never a user override.
pub const PRODUCT_REGION: Option<[u8; 2]> = Some(*b"XX");

/// 802.11 Privacy capability bit; distinguishes secured from open APs.
const CAPABILITY_PRIVACY: u16 = 0x0010;

/// Status of the single in-flight command, reported back to the sync HAL.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpStatus {
    Idle,
    Pending,
    Ready,
    Failed(HalFault),
}

/// A command handed from the sync HAL to the async command loop.
#[derive(Clone, Copy)]
enum HalCommand {
    SetRadio(bool),
    Scan,
    Connect,
    Disconnect,
}

/// Bounded shared state between CPU0's sync HAL and the async command loop.
struct Mailbox {
    command: Option<HalCommand>,
    status: OpStatus,
    radio_present: bool,
    radio_on: bool,
    cancel: bool,
    ssid: [u8; SSID_MAX_BYTES],
    ssid_len: u8,
    secret: [u8; CREDENTIAL_MAX_BYTES],
    secret_len: u8,
    security_open: bool,
    scan: [Option<RawScanResult>; SCAN_RESULTS_MAX],
    scan_len: u8,
    link_up: bool,
    config_up: bool,
    ipv4: [u8; 4],
    sntp_server: [u8; SNTP_SERVER_MAX_BYTES],
    sntp_server_len: u8,
    sntp_server_generation: u32,
}

impl Mailbox {
    const fn new() -> Self {
        Self {
            command: None,
            status: OpStatus::Idle,
            // The radio is assumed present once the command loop is installed;
            // a fatal driver fault clears this and the service enters
            // RadioUnavailable.
            radio_present: true,
            radio_on: false,
            cancel: false,
            ssid: [0; SSID_MAX_BYTES],
            ssid_len: 0,
            secret: [0; CREDENTIAL_MAX_BYTES],
            secret_len: 0,
            security_open: true,
            scan: [None; SCAN_RESULTS_MAX],
            scan_len: 0,
            link_up: false,
            config_up: false,
            ipv4: [0; 4],
            sntp_server: [
                b'p', b'o', b'o', b'l', b'.', b'n', b't', b'p', b'.', b'o', b'r', b'g', 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
            sntp_server_len: 12,
            sntp_server_generation: 1,
        }
    }
}

static MAILBOX: Mutex<RefCell<Mailbox>> = Mutex::new(RefCell::new(Mailbox::new()));
static SNTP_UTC_SECONDS: AtomicU32 = AtomicU32::new(0);
static SNTP_GENERATION: AtomicU32 = AtomicU32::new(0);

fn with_mailbox<R>(f: impl FnOnce(&mut Mailbox) -> R) -> R {
    critical_section::with(|cs| f(&mut MAILBOX.borrow(cs).borrow_mut()))
}

/// The sync, non-blocking cyw43 radio HAL backing `NetworkService`.
///
/// It is a zero-sized handle over the static mailbox; every method only reads or
/// writes bounded mailbox fields and never blocks.
pub struct Cyw43WifiHal;

impl Cyw43WifiHal {
    fn submit(command: HalCommand) {
        with_mailbox(|m| {
            m.command = Some(command);
            m.status = OpStatus::Pending;
            m.cancel = false;
        });
    }

    fn poll_status() -> HalPoll {
        with_mailbox(|m| match m.status {
            OpStatus::Ready => HalPoll::Ready,
            OpStatus::Idle | OpStatus::Pending => HalPoll::Pending,
            OpStatus::Failed(fault) => HalPoll::Failed(fault),
        })
    }
}

impl koto_core::net::WifiHal for Cyw43WifiHal {
    fn radio_present(&self) -> bool {
        with_mailbox(|m| m.radio_present)
    }

    fn begin_set_radio(&mut self, enabled: bool) {
        Self::submit(HalCommand::SetRadio(enabled));
    }

    fn poll_set_radio(&mut self) -> HalPoll {
        Self::poll_status()
    }

    fn begin_scan(&mut self) {
        Self::submit(HalCommand::Scan);
    }

    fn poll_scan(&mut self) -> HalPoll {
        with_mailbox(|m| match m.status {
            OpStatus::Ready => HalPoll::ReadyCount(m.scan_len),
            OpStatus::Idle | OpStatus::Pending => HalPoll::Pending,
            OpStatus::Failed(fault) => HalPoll::Failed(fault),
        })
    }

    fn scan_result(&self, index: u8) -> RawScanResult {
        with_mailbox(|m| {
            m.scan[usize::from(index)].unwrap_or(RawScanResult {
                ssid: Ssid::from_bytes(&[]),
                bssid: [0; BSSID_BYTES],
                rssi_dbm: i8::MIN,
                security: Security::Open,
            })
        })
    }

    fn begin_connect(
        &mut self,
        ssid: &Ssid,
        _bssid: &[u8; BSSID_BYTES],
        security: Security,
        secret: &[u8],
    ) {
        with_mailbox(|m| {
            let ssid_bytes = ssid.as_bytes();
            let ssid_len = ssid_bytes.len().min(SSID_MAX_BYTES);
            m.ssid[..ssid_len].copy_from_slice(&ssid_bytes[..ssid_len]);
            m.ssid_len = ssid_len as u8;
            let secret_len = secret.len().min(CREDENTIAL_MAX_BYTES);
            m.secret[..secret_len].copy_from_slice(&secret[..secret_len]);
            m.secret_len = secret_len as u8;
            m.security_open = matches!(security, Security::Open);
            m.command = Some(HalCommand::Connect);
            m.status = OpStatus::Pending;
            m.cancel = false;
        });
    }

    fn poll_connect(&mut self) -> HalPoll {
        Self::poll_status()
    }

    fn begin_disconnect(&mut self) {
        Self::submit(HalCommand::Disconnect);
    }

    fn poll_disconnect(&mut self) -> HalPoll {
        Self::poll_status()
    }

    fn cancel(&mut self) {
        with_mailbox(|m| m.cancel = true);
    }
}

/// Placeholder credential provider used before KOTO-0240 lands the two-slot
/// secret store. It reports availability and acknowledges forget commits so the
/// service and page can be exercised end to end; it retains no secrets.
pub struct StubCredentialProvider;

impl CredentialProvider for StubCredentialProvider {
    fn available(&self) -> bool {
        true
    }

    fn forget(&mut self, _profile_id: u16) -> ForgetOutcome {
        ForgetOutcome::Committed
    }
}

/// Services the `NetworkService` once against the cyw43-backed HAL and the stub
/// credential provider. Called from CPU0 outside painting with a bounded budget.
pub fn service_network(service: &mut NetworkService, now_ms: u64, work_budget: u32) {
    let mut hal = Cyw43WifiHal;
    let mut creds = StubCredentialProvider;
    service.service(now_ms, work_budget, &mut hal, &mut creds);
}

/// Services the product NetworkService against its persistent credential
/// provider. The provider remains owned by the KotoShell firmware loop; this
/// function only borrows it for one bounded service advance.
pub fn service_network_with_credentials(
    service: &mut NetworkService,
    now_ms: u64,
    work_budget: u32,
    credentials: &mut impl CredentialProvider,
) {
    let mut hal = Cyw43WifiHal;
    service.service(now_ms, work_budget, &mut hal, credentials);
}

/// Resets the bounded radio mailbox to its power-off defaults after the
/// arena-owned command loop has been cancelled and joined (KOTO-0251 Pico W
/// teardown). Staged SSID/secret bytes are zeroized; the OS-owned SNTP
/// endpoint configuration and the `radio_present` fault latch are retained.
pub fn reset_radio_mailbox() {
    with_mailbox(|m| {
        m.command = None;
        m.status = OpStatus::Idle;
        m.radio_on = false;
        m.cancel = false;
        m.ssid = [0; SSID_MAX_BYTES];
        m.ssid_len = 0;
        m.secret = [0; CREDENTIAL_MAX_BYTES];
        m.secret_len = 0;
        m.security_open = true;
        m.scan = [None; SCAN_RESULTS_MAX];
        m.scan_len = 0;
        m.link_up = false;
        m.config_up = false;
        m.ipv4 = [0; 4];
    });
}

/// Latches the radio unavailable for the rest of this power cycle. Used when a
/// teardown boundary cannot prove the arena or radio peripherals were released
/// (KOTO-0251): the service reports `RadioUnavailable`, no later enable can
/// reuse the unproven arena, and boot/Shell/offline app launch stay untouched.
pub fn mark_radio_unavailable() {
    with_mailbox(|m| m.radio_present = false);
}

/// Publishes the latest embassy-net DHCP/link status. Called by the arena stack
/// monitor; `ipv4` is the assigned address octets (`0.0.0.0` when unconfigured).
pub fn publish_dhcp_status(link_up: bool, config_up: bool, ipv4: [u8; 4]) {
    with_mailbox(|m| {
        m.link_up = link_up;
        m.config_up = config_up;
        m.ipv4 = ipv4;
    });
}

/// Reads the published DHCP status: `(link_up, config_up, ipv4_octets)`.
pub fn dhcp_status() -> (bool, bool, [u8; 4]) {
    with_mailbox(|m| (m.link_up, m.config_up, m.ipv4))
}

/// Publishes the current advisory UTC second from the arena-owned SNTP task.
/// Generation is stored last so readers never observe a valid partial value.
pub fn publish_sntp_utc(utc_seconds: i64, generation: u32) {
    if generation == 0 || !(0..=i64::from(u32::MAX)).contains(&utc_seconds) {
        return;
    }
    SNTP_UTC_SECONDS.store(utc_seconds as u32, Ordering::Relaxed);
    SNTP_GENERATION.store(generation, Ordering::Release);
}

pub fn sntp_utc_seconds() -> Option<(i64, u32)> {
    let generation = SNTP_GENERATION.load(Ordering::Acquire);
    (generation != 0).then(|| {
        (
            i64::from(SNTP_UTC_SECONDS.load(Ordering::Relaxed)),
            generation,
        )
    })
}

pub fn clear_sntp_utc() {
    SNTP_GENERATION.store(0, Ordering::Release);
    SNTP_UTC_SECONDS.store(0, Ordering::Relaxed);
}

/// Updates the OS-owned SNTP endpoint. A changed hostname advances generation,
/// causing the arena client to resynchronize without waiting for refresh.
pub fn publish_sntp_server(hostname: &str) {
    let bytes = hostname.as_bytes();
    if bytes.is_empty() || bytes.len() > SNTP_SERVER_MAX_BYTES || !bytes.is_ascii() {
        return;
    }
    with_mailbox(|m| {
        if &m.sntp_server[..usize::from(m.sntp_server_len)] == bytes {
            return;
        }
        m.sntp_server.fill(0);
        m.sntp_server[..bytes.len()].copy_from_slice(bytes);
        m.sntp_server_len = bytes.len() as u8;
        m.sntp_server_generation = m.sntp_server_generation.wrapping_add(1).max(1);
    });
}

pub fn sntp_server(dst: &mut [u8; SNTP_SERVER_MAX_BYTES]) -> (usize, u32) {
    with_mailbox(|m| {
        let len = usize::from(m.sntp_server_len);
        dst.fill(0);
        dst[..len].copy_from_slice(&m.sntp_server[..len]);
        (len, m.sntp_server_generation)
    })
}

/// The async command loop that owns cyw43 `Control`. It waits for one mailbox
/// command at a time, drives the radio, and publishes the redacted result. It
/// never returns; join it with the cyw43 runner inside the residency arena.
pub async fn radio_command_loop(mut control: Control<'static>, clm: &'static [u8]) {
    loop {
        let command = poll_fn(|_cx| {
            with_mailbox(|m| match m.command.take() {
                Some(command) => Poll::Ready(command),
                None => Poll::Pending,
            })
        })
        .await;

        match command {
            HalCommand::SetRadio(true) => {
                // Regulatory region is a release policy validated before enable.
                // An absent/unsupported policy refuses radio initialization
                // (keeping WIFI_CONFIG false) without blocking offline boot.
                if RegulatoryRegion::permits_radio_enable(PRODUCT_REGION) {
                    control.init(clm).await;
                    control
                        .set_power_management(PowerManagementMode::PowerSave)
                        .await;
                    with_mailbox(|m| {
                        m.radio_on = true;
                        m.status = OpStatus::Ready;
                    });
                } else {
                    with_mailbox(|m| {
                        m.radio_on = false;
                        m.status = OpStatus::Failed(HalFault::Firmware);
                    });
                }
            }
            HalCommand::SetRadio(false) => {
                control.leave().await;
                with_mailbox(|m| {
                    m.radio_on = false;
                    m.status = OpStatus::Ready;
                });
            }
            HalCommand::Scan => {
                let mut scanner = control.scan(ScanOptions::default()).await;
                let mut count = 0usize;
                while count < SCAN_RESULTS_MAX {
                    if with_mailbox(|m| m.cancel) {
                        break;
                    }
                    match scanner.next().await {
                        Some(bss) => {
                            let result = raw_from_bss(&bss);
                            with_mailbox(|m| m.scan[count] = Some(result));
                            count += 1;
                        }
                        None => break,
                    }
                }
                with_mailbox(|m| {
                    m.scan_len = count as u8;
                    m.status = OpStatus::Ready;
                });
            }
            HalCommand::Connect => {
                let mut ssid = [0u8; SSID_MAX_BYTES];
                let mut secret = [0u8; CREDENTIAL_MAX_BYTES];
                let (ssid_len, secret_len, open) = with_mailbox(|m| {
                    ssid[..usize::from(m.ssid_len)]
                        .copy_from_slice(&m.ssid[..usize::from(m.ssid_len)]);
                    secret[..usize::from(m.secret_len)]
                        .copy_from_slice(&m.secret[..usize::from(m.secret_len)]);
                    (m.ssid_len, m.secret_len, m.security_open)
                });
                let status = match core::str::from_utf8(&ssid[..usize::from(ssid_len)]) {
                    Ok(ssid_str) => {
                        let options = if open {
                            JoinOptions::new_open()
                        } else {
                            let mut options = JoinOptions::new(&secret[..usize::from(secret_len)]);
                            options.auth = JoinAuth::Wpa2;
                            options
                        };
                        match control.join(ssid_str, options).await {
                            Ok(()) => OpStatus::Ready,
                            Err(JoinError::NetworkNotFound) => OpStatus::Failed(HalFault::NotFound),
                            Err(JoinError::AuthenticationFailure) => {
                                OpStatus::Failed(HalFault::Auth)
                            }
                            Err(JoinError::JoinFailure(_)) => OpStatus::Failed(HalFault::Transient),
                        }
                    }
                    // A non-UTF-8 SSID cannot be joined through this driver API.
                    Err(_) => OpStatus::Failed(HalFault::NotFound),
                };
                // Wipe the local secret copy before yielding.
                secret.fill(0);
                with_mailbox(|m| m.status = status);
            }
            HalCommand::Disconnect => {
                control.leave().await;
                with_mailbox(|m| {
                    m.radio_on = false;
                    m.status = OpStatus::Ready;
                });
            }
        }
    }
}

/// Builds a bounded `RawScanResult` from a cyw43 `BssInfo`. `BssInfo` is
/// `#[repr(packed)]`, so every field is read by value into a local first.
fn raw_from_bss(bss: &cyw43::BssInfo) -> RawScanResult {
    let ssid_len = usize::from(bss.ssid_len).min(SSID_MAX_BYTES);
    let ssid_bytes = bss.ssid;
    let bssid = bss.bssid;
    let rssi = bss.rssi;
    let capability = bss.capability;
    let security = if capability & CAPABILITY_PRIVACY != 0 {
        Security::Wpa2PersonalAes
    } else {
        Security::Open
    };
    RawScanResult {
        ssid: Ssid::from_bytes(&ssid_bytes[..ssid_len]),
        bssid,
        rssi_dbm: rssi.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8,
        security,
    }
}
