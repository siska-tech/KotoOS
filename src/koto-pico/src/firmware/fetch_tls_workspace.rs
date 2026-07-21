//! RP2350A product HTTPS workspace.
//!
//! Unlike RP2040, RP2350A has enough internal SRAM to keep TLS independent of
//! the permanent audio path. One generation-owned network dispatcher may claim
//! this static workspace at a time. Every release overwrites the complete
//! region before another request can acquire it.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};

/// TLS 1.3 permits records up to 2^14 bytes plus bounded ciphertext overhead.
pub const RECORD_RX_BYTES: usize = 16 * 1024 + 256;
/// Covers the bounded 768-byte GET head and all client handshake records.
pub const RECORD_TX_BYTES: usize = 2 * 1024;
/// Dedicated synchronous crypto stack, including nested interrupt frames.
pub const CRYPTO_STACK_BYTES: usize = 16 * 1024;

#[repr(C, align(8))]
struct Rp2350TlsStorage {
    record_rx: [u8; RECORD_RX_BYTES],
    record_tx: [u8; RECORD_TX_BYTES],
    decoder: MaybeUninit<koto_core::HttpResponseDecoder>,
    plaintext: MaybeUninit<crate::firmware::fetch_https::FetchTlsScratch>,
    crypto_stack: [u8; CRYPTO_STACK_BYTES],
}

impl Rp2350TlsStorage {
    const fn new() -> Self {
        Self {
            record_rx: [0; RECORD_RX_BYTES],
            record_tx: [0; RECORD_TX_BYTES],
            decoder: MaybeUninit::uninit(),
            plaintext: MaybeUninit::uninit(),
            crypto_stack: [0; CRYPTO_STACK_BYTES],
        }
    }
}

#[repr(transparent)]
struct SharedWorkspace(UnsafeCell<Rp2350TlsStorage>);

// Safety: `WORKSPACE_CLAIMED` grants the only mutable access, and the product
// network dispatcher processes at most one Fetch transaction at a time.
unsafe impl Sync for SharedWorkspace {}

static WORKSPACE: SharedWorkspace = SharedWorkspace(UnsafeCell::new(Rp2350TlsStorage::new()));
static WORKSPACE_CLAIMED: AtomicBool = AtomicBool::new(false);

pub struct Rp2350TlsWorkspace {
    _private: (),
}

pub struct Rp2350TlsParts<'a> {
    pub record_rx: &'a mut [u8],
    pub record_tx: &'a mut [u8],
    pub decoder: &'a mut koto_core::HttpResponseDecoder,
    pub plaintext: &'a mut crate::firmware::fetch_https::FetchTlsScratch,
    pub crypto_stack: &'a mut [u8],
}

impl Rp2350TlsWorkspace {
    pub fn claim() -> Option<Self> {
        WORKSPACE_CLAIMED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { _private: () })
    }

    pub fn prepare(&mut self) -> Rp2350TlsParts<'_> {
        // Safety: this linear guard owns the successful global claim.
        let storage = unsafe { &mut *WORKSPACE.0.get() };
        storage.record_rx.fill(0);
        storage.record_tx.fill(0);
        storage.crypto_stack.fill(0);
        let decoder = storage.decoder.write(koto_core::HttpResponseDecoder::new());
        let plaintext = storage
            .plaintext
            .write(crate::firmware::fetch_https::FetchTlsScratch::new());
        Rp2350TlsParts {
            record_rx: &mut storage.record_rx,
            record_tx: &mut storage.record_tx,
            decoder,
            plaintext,
            crypto_stack: &mut storage.crypto_stack,
        }
    }
}

impl Drop for Rp2350TlsWorkspace {
    fn drop(&mut self) {
        // Volatile overwrite prevents TLS/plaintext state from surviving the
        // ownership boundary even when the optimizer can see no later read.
        let base = WORKSPACE.0.get().cast::<u8>();
        for offset in 0..core::mem::size_of::<Rp2350TlsStorage>() {
            unsafe { base.add(offset).write_volatile(0) };
        }
        WORKSPACE_CLAIMED.store(false, Ordering::Release);
    }
}

pub const WORKSPACE_BYTES: usize = core::mem::size_of::<Rp2350TlsStorage>();
