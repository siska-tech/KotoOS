//! Isolated KOTO-0245 RP2040 embedded-tls target-layout probe.
#![no_std]
#![no_main]

use core::convert::Infallible;

use embassy_executor::Spawner;
use embedded_io::ErrorType;
use embedded_io_async::{Read, Write};
use embedded_tls::{Aes128GcmSha256, TlsConnection};
#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
use embedded_tls::{CertificateRef, CertificateVerifyRef};
#[cfg(feature = "app_fetch_tls_handshake_probe")]
use embedded_tls::{
    CryptoProvider, MaxFragmentLength, TlsConfig, TlsContext, TlsError, TlsVerifier,
};
#[cfg(feature = "app_fetch_tls_verifier_probe")]
use koto_core::{FetchPinSet, SpkiSha256};
#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
use koto_pico::firmware::fetch_tls_adapter::TlsIoAdapter;
#[cfg(feature = "app_fetch_tls_verifier_probe")]
use koto_pico::firmware::fetch_tls_verifier::PinnedP256Verifier;
use panic_halt as _;
#[cfg(feature = "app_fetch_tls_handshake_probe")]
use rand_core::{CryptoRng, Error as RngError, RngCore};

const RECORD_RX_BYTES: usize = 4 * 1024;
const RECORD_TX_BYTES: usize = 1024;
#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
type ProbeTlsSocket = TlsIoAdapter<ProbeSocket>;
#[cfg(not(feature = "app_fetch_tls_socket_adapter_probe"))]
type ProbeTlsSocket = ProbeSocket;
const CONNECTION_BYTES: usize =
    core::mem::size_of::<TlsConnection<'static, ProbeTlsSocket, Aes128GcmSha256>>();
#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
const SOCKET_ADAPTER_BYTES: usize =
    core::mem::size_of::<TlsIoAdapter<embassy_net::tcp::TcpSocket<'static>>>();

/// Zero-storage transport used only to expose the TLS connection layout. The
/// actual embassy-net 0.6-to-0.7 adapter and handshake are separate probe rows.
#[derive(Clone, Copy)]
struct ProbeSocket;

/// Layout stand-in for the eventual pinning verifier. It deliberately rejects
/// every certificate/signature, so this diagnostic can never create an
/// unauthenticated connection. The fields model the retained P-256 public key,
/// transcript hash, two pins, and verifier state without retaining full DER.
#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
struct FailClosedProbeVerifier {
    public_key: [u8; 65],
    transcript_hash: [u8; 32],
    pins: [[u8; 32]; 2],
    certificate_seen: bool,
}

#[cfg(feature = "app_fetch_tls_verifier_probe")]
type ProbeVerifier = PinnedP256Verifier;
#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
type ProbeVerifier = FailClosedProbeVerifier;

#[cfg(feature = "app_fetch_tls_verifier_probe")]
fn probe_verifier() -> ProbeVerifier {
    let mut pins = FetchPinSet::empty();
    let _ = pins.push(SpkiSha256::from_bytes([0x4b; 32]));
    PinnedP256Verifier::new(pins)
}

#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
fn probe_verifier() -> ProbeVerifier {
    FailClosedProbeVerifier::new()
}

#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
impl FailClosedProbeVerifier {
    const fn new() -> Self {
        Self {
            public_key: [0; 65],
            transcript_hash: [0; 32],
            pins: [[0; 32]; 2],
            certificate_seen: false,
        }
    }
}

#[cfg(all(
    feature = "app_fetch_tls_handshake_probe",
    not(feature = "app_fetch_tls_verifier_probe")
))]
impl TlsVerifier<Aes128GcmSha256> for FailClosedProbeVerifier {
    fn set_hostname_verification(&mut self, _hostname: &str) -> Result<(), TlsError> {
        Ok(())
    }

    fn verify_certificate(
        &mut self,
        _transcript: &embedded_tls::Sha256,
        _cert: CertificateRef,
    ) -> Result<(), TlsError> {
        self.certificate_seen = true;
        self.transcript_hash[0] = self.public_key[0] ^ self.pins[0][0] ^ self.pins[1][0];
        Err(TlsError::InvalidCertificate)
    }

    fn verify_signature(&mut self, _verify: CertificateVerifyRef) -> Result<(), TlsError> {
        let _certificate_state = self.certificate_seen && self.transcript_hash[0] != 0;
        Err(TlsError::InvalidSignature)
    }
}

/// Deterministic diagnostic RNG. The fail-closed verifier makes a successful
/// handshake impossible; this exists solely to instantiate the open future.
#[cfg(feature = "app_fetch_tls_handshake_probe")]
struct ProbeRng(u32);

#[cfg(feature = "app_fetch_tls_handshake_probe")]
impl RngCore for ProbeRng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    fn next_u64(&mut self) -> u64 {
        u64::from(self.next_u32()) << 32 | u64::from(self.next_u32())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rand_core::impls::fill_bytes_via_next(self, dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RngError> {
        self.fill_bytes(dest);
        Ok(())
    }
}

#[cfg(feature = "app_fetch_tls_handshake_probe")]
impl CryptoRng for ProbeRng {}

#[cfg(feature = "app_fetch_tls_handshake_probe")]
struct ProbeProvider {
    rng: ProbeRng,
    verifier: ProbeVerifier,
}

#[cfg(feature = "app_fetch_tls_handshake_probe")]
impl CryptoProvider for ProbeProvider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut self.verifier)
    }
}

impl ErrorType for ProbeSocket {
    type Error = Infallible;
}

impl Read for ProbeSocket {
    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(0)
    }
}

impl Write for ProbeSocket {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
impl embedded_io_06::ErrorType for ProbeSocket {
    type Error = Infallible;
}

#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
impl embedded_io_async_06::Read for ProbeSocket {
    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(0)
    }
}

#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
impl embedded_io_async_06::Write for ProbeSocket {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_CONNECTION_SIZE: [u8; CONNECTION_BYTES] = [0; CONNECTION_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_RECORD_RX_SIZE: [u8; RECORD_RX_BYTES] = [0; RECORD_RX_BYTES];
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_RECORD_TX_SIZE: [u8; RECORD_TX_BYTES] = [0; RECORD_TX_BYTES];

#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_SOCKET_ADAPTER_SIZE: [u8; SOCKET_ADAPTER_BYTES] = [0; SOCKET_ADAPTER_BYTES];

#[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
fn require_tls_transport<T: embedded_io_async::Read + embedded_io_async::Write>() {}

#[cfg(feature = "app_fetch_tls_handshake_probe")]
const VERIFIER_BYTES: usize = core::mem::size_of::<ProbeVerifier>();
#[cfg(feature = "app_fetch_tls_handshake_probe")]
const PROVIDER_BYTES: usize = core::mem::size_of::<ProbeProvider>();

#[cfg(feature = "app_fetch_tls_handshake_probe")]
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_VERIFIER_SIZE: [u8; VERIFIER_BYTES] = [0; VERIFIER_BYTES];
#[cfg(feature = "app_fetch_tls_handshake_probe")]
#[used]
#[unsafe(no_mangle)]
static APP_FETCH_TLS_PROVIDER_SIZE: [u8; PROVIDER_BYTES] = [0; PROVIDER_BYTES];

#[cfg(feature = "app_fetch_tls_handshake_probe")]
#[embassy_executor::task]
async fn tls_handshake_layout_task() {
    let mut record_rx = [0; RECORD_RX_BYTES];
    let mut record_tx = [0; RECORD_TX_BYTES];
    let config = TlsConfig::new()
        .with_server_name("probe.invalid")
        .with_max_fragment_length(MaxFragmentLength::Bits12);
    let provider = ProbeProvider {
        rng: ProbeRng(0x4b4f_544f),
        verifier: probe_verifier(),
    };
    #[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
    let socket = TlsIoAdapter::new(ProbeSocket);
    #[cfg(not(feature = "app_fetch_tls_socket_adapter_probe"))]
    let socket = ProbeSocket;
    let mut connection = TlsConnection::new(socket, &mut record_rx, &mut record_tx);
    let _result = connection.open(TlsContext::new(&config, provider)).await;
    loop {
        core::future::pending::<()>().await;
    }
}

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(spawner: Spawner) {
    let _peripherals = embassy_rp::init(Default::default());
    #[cfg(feature = "app_fetch_tls_socket_adapter_probe")]
    require_tls_transport::<TlsIoAdapter<embassy_net::tcp::TcpSocket<'static>>>();
    #[cfg(feature = "app_fetch_tls_handshake_probe")]
    if let Ok(task) = tls_handshake_layout_task() {
        spawner.spawn(task);
    }
    #[cfg(not(feature = "app_fetch_tls_handshake_probe"))]
    let _unused = spawner;
    loop {
        cortex_m::asm::wfi();
    }
}
