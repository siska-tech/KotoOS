//! KOTO-0245 product HTTPS Fetch session.
//!
//! One SPKI-pinned TLS 1.3 connection plus the streaming HTTP/1.1 pump for a
//! single bounded GET. The dispatcher installs this future into the RP2040
//! TLS/audio-exclusive workspace arena; it borrows the TCP socket, record
//! buffers, decoder, and staging from the caller and retains none of them
//! after completion. Trust is rooted exclusively in the manifest SPKI pins —
//! the advisory SNTP clock is never consulted (KOTO-0244 independence).

use embassy_net::tcp::TcpSocket;
use embassy_rp::clocks::RoscRng;
use embassy_time::Timer;
use embedded_tls::{
    Aes128GcmSha256, CryptoProvider, TlsConfig, TlsConnection, TlsContext, TlsError, TlsVerifier,
};
use koto_core::{
    FetchError, FetchPinSet, FetchRequestId, HttpDecodeState, HttpResponseDecoder,
    FETCH_TRANSPORT_CHUNK_BYTES,
};

use crate::firmware::fetch_tls_adapter::TlsIoAdapter;
use crate::firmware::fetch_tls_verifier::PinnedP256Verifier;
use crate::firmware::wifi_residency::FetchTransportShared;

/// Producer-side wait granularity against the VM drain/cancel signals.
const MAILBOX_WAIT_MS: u64 = 5;

/// Caller-owned plaintext staging. It lives in the dispatcher frame, not the
/// workspace future, so the by-value `ArenaFuture` install stays small; the
/// dispatcher zeroizes it on every request exit.
pub struct FetchTlsScratch {
    plaintext: [u8; FETCH_TRANSPORT_CHUNK_BYTES],
    chunk: [u8; FETCH_TRANSPORT_CHUNK_BYTES],
}

impl FetchTlsScratch {
    pub const fn new() -> Self {
        Self {
            plaintext: [0; FETCH_TRANSPORT_CHUNK_BYTES],
            chunk: [0; FETCH_TRANSPORT_CHUNK_BYTES],
        }
    }

    /// Volatile overwrite of the decrypted response staging.
    pub fn zeroize(&mut self) {
        for byte in self.plaintext.iter_mut().chain(self.chunk.iter_mut()) {
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
    }
}

impl Default for FetchTlsScratch {
    fn default() -> Self {
        Self::new()
    }
}

struct PinnedFetchProvider {
    rng: RoscRng,
    verifier: PinnedP256Verifier,
}

impl CryptoProvider for PinnedFetchProvider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 64];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    /// Structurally infallible: embedded-tls skips certificate and
    /// CertificateVerify checks when verifier acquisition errors, so failure
    /// must live inside the verifier methods, never here (KOTO-0245 source
    /// inspection).
    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut self.verifier)
    }
}

fn cancelled(mailbox: &FetchTransportShared, request: FetchRequestId) -> bool {
    mailbox.with(|mailbox| mailbox.cancel_requested(request))
}

/// Publishes response headers, then holds until the VM has observed
/// `Headers { status }` once. This preserves the KotoSim guarantee that
/// success metadata is always observable before the first body chunk
/// supersedes it.
async fn publish_headers_observed(
    mailbox: &FetchTransportShared,
    request: FetchRequestId,
    status: u16,
) -> Result<(), FetchError> {
    loop {
        if cancelled(mailbox, request) {
            return Err(FetchError::Cancelled);
        }
        match mailbox.with_mut(|mailbox| mailbox.publish_headers(request, status)) {
            Ok(()) => break,
            Err(FetchError::Busy) => Timer::after_millis(MAILBOX_WAIT_MS).await,
            Err(error) => return Err(error),
        }
    }
    loop {
        if cancelled(mailbox, request) {
            return Err(FetchError::Cancelled);
        }
        if mailbox.with(koto_core::FetchTransportMailbox::headers_polled) {
            return Ok(());
        }
        Timer::after_millis(MAILBOX_WAIT_MS).await;
    }
}

/// Publishes one bounded chunk after the VM drained the previous one. The
/// mailbox contract forbids producer reuse of an undrained slot, so `Busy`
/// waits are expected steady-state behaviour.
async fn publish_body_drained(
    mailbox: &FetchTransportShared,
    request: FetchRequestId,
    bytes: &[u8],
) -> Result<(), FetchError> {
    loop {
        if cancelled(mailbox, request) {
            return Err(FetchError::Cancelled);
        }
        match mailbox.with_mut(|mailbox| mailbox.publish_body(request, bytes)) {
            Ok(()) => return Ok(()),
            Err(FetchError::Busy) => Timer::after_millis(MAILBOX_WAIT_MS).await,
            Err(error) => return Err(error),
        }
    }
}

/// Marks the exchange complete once the final chunk has drained.
async fn finish_complete(
    mailbox: &FetchTransportShared,
    request: FetchRequestId,
) -> Result<(), FetchError> {
    loop {
        if cancelled(mailbox, request) {
            return Err(FetchError::Cancelled);
        }
        match mailbox.with_mut(|mailbox| mailbox.complete(request)) {
            Ok(()) => return Ok(()),
            Err(FetchError::Busy) => Timer::after_millis(MAILBOX_WAIT_MS).await,
            Err(error) => return Err(error),
        }
    }
}

/// Runs the complete pinned HTTPS exchange for one Fetch request. The future
/// resolves to `()` so it can be type-erased into the workspace arena; the
/// dispatcher reads `outcome` after the future completes. Dropping the future
/// mid-flight (cancellation, deadline) is safe: every borrow is released and
/// the caller aborts the socket and zeroizes the workspace and staging.
#[allow(clippy::too_many_arguments)]
pub async fn run_pinned_https_session(
    socket: &mut TcpSocket<'_>,
    record_rx: &mut [u8],
    record_tx: &mut [u8],
    hostname: &str,
    request_head: &[u8],
    pins: FetchPinSet,
    mailbox: &'static FetchTransportShared,
    request: FetchRequestId,
    decoder: &mut HttpResponseDecoder,
    scratch: &mut FetchTlsScratch,
    outcome: &mut Result<(), FetchError>,
) {
    // No max_fragment_length extension: most servers ignore RFC 6066 MFL, and
    // requesting it made this server withhold its Finished flight where a
    // stock client (no MFL) completed. Records are instead bounded by the
    // fixed 2 KiB receive buffer; an oversized record fails closed as `Tls`.
    let config = TlsConfig::new().with_server_name(hostname);
    let provider = PinnedFetchProvider {
        rng: RoscRng,
        verifier: PinnedP256Verifier::new(pins),
    };
    use crate::firmware::wifi_residency::{record_fetch_tls_phase, tls_phase};
    record_fetch_tls_phase(tls_phase::SESSION_ENTERED);
    let mut connection = TlsConnection::new(TlsIoAdapter::new(socket), record_rx, record_tx);
    *outcome = async {
        record_fetch_tls_phase(tls_phase::OPEN_STARTED);
        connection
            .open(TlsContext::new(&config, provider))
            .await
            .map_err(|_| FetchError::Tls)?;
        record_fetch_tls_phase(tls_phase::OPEN_DONE);

        let mut written = 0;
        while written < request_head.len() {
            if cancelled(mailbox, request) {
                return Err(FetchError::Cancelled);
            }
            let count = connection
                .write(&request_head[written..])
                .await
                .map_err(|_| FetchError::Tls)?;
            if count == 0 {
                return Err(FetchError::Disconnected);
            }
            written += count;
        }
        connection.flush().await.map_err(|_| FetchError::Tls)?;
        record_fetch_tls_phase(tls_phase::REQUEST_SENT);

        let mut headers_published = false;
        loop {
            if cancelled(mailbox, request) {
                return Err(FetchError::Cancelled);
            }
            let read = connection
                .read(&mut scratch.plaintext)
                .await
                .map_err(|_| FetchError::Tls)?;
            record_fetch_tls_phase(tls_phase::RESPONSE_STARTED);
            if read == 0 {
                // Close before decoder completion: response truncation must
                // never surface as success (truncation-never-complete).
                return Err(FetchError::Disconnected);
            }
            let mut fed = 0;
            while fed < read {
                let progress = decoder.push(&scratch.plaintext[fed..read], &mut scratch.chunk)?;
                fed += progress.consumed;
                if !headers_published {
                    if let Some(status) = progress.status {
                        publish_headers_observed(mailbox, request, status).await?;
                        headers_published = true;
                    }
                }
                if progress.written > 0 {
                    publish_body_drained(mailbox, request, &scratch.chunk[..progress.written])
                        .await?;
                }
                if progress.state == HttpDecodeState::Complete {
                    record_fetch_tls_phase(tls_phase::COMPLETE);
                    return finish_complete(mailbox, request).await;
                }
                if progress.consumed == 0 && progress.written == 0 {
                    return Err(FetchError::Protocol);
                }
            }
        }
    }
    .await;
}
