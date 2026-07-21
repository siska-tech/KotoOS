//! embedded-io compatibility boundary for KOTO-0245 (probe and, since the
//! `app_fetch_https` feature, the product HTTPS session).
//!
//! embassy-net 0.7 exposes its TCP socket through embedded-io-async 0.6 while
//! embedded-tls 0.19 consumes 0.7. This owned adapter forwards only Read,
//! Write, and flush and preserves the old error kind. It deliberately exposes
//! no socket-management or network-stack handle to application code.

use embedded_io::{Error as Error07, ErrorKind as ErrorKind07, ErrorType as ErrorType07};
use embedded_io_06::{Error as Error06, ErrorKind as ErrorKind06, ErrorType as ErrorType06};
use embedded_io_async::{Read as Read07, Write as Write07};
use embedded_io_async_06::{Read as Read06, Write as Write06};

#[derive(Debug)]
pub struct TlsIoError<E>(E);

impl<E> TlsIoError<E> {
    pub fn into_inner(self) -> E {
        self.0
    }
}

impl<E: Error06> core::fmt::Display for TlsIoError<E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "TLS transport I/O error: {:?}", self.0.kind())
    }
}

impl<E: Error06> core::error::Error for TlsIoError<E> {}

impl<E: Error06> Error07 for TlsIoError<E> {
    fn kind(&self) -> ErrorKind07 {
        match self.0.kind() {
            ErrorKind06::Other => ErrorKind07::Other,
            ErrorKind06::NotFound => ErrorKind07::NotFound,
            ErrorKind06::PermissionDenied => ErrorKind07::PermissionDenied,
            ErrorKind06::ConnectionRefused => ErrorKind07::ConnectionRefused,
            ErrorKind06::ConnectionReset => ErrorKind07::ConnectionReset,
            ErrorKind06::ConnectionAborted => ErrorKind07::ConnectionAborted,
            ErrorKind06::NotConnected => ErrorKind07::NotConnected,
            ErrorKind06::AddrInUse => ErrorKind07::AddrInUse,
            ErrorKind06::AddrNotAvailable => ErrorKind07::AddrNotAvailable,
            ErrorKind06::BrokenPipe => ErrorKind07::BrokenPipe,
            ErrorKind06::AlreadyExists => ErrorKind07::AlreadyExists,
            ErrorKind06::InvalidInput => ErrorKind07::InvalidInput,
            ErrorKind06::InvalidData => ErrorKind07::InvalidData,
            ErrorKind06::TimedOut => ErrorKind07::TimedOut,
            ErrorKind06::Interrupted => ErrorKind07::Interrupted,
            ErrorKind06::Unsupported => ErrorKind07::Unsupported,
            ErrorKind06::OutOfMemory => ErrorKind07::OutOfMemory,
            ErrorKind06::WriteZero => ErrorKind07::WriteZero,
            _ => ErrorKind07::Other,
        }
    }
}

#[repr(transparent)]
pub struct TlsIoAdapter<T> {
    inner: T,
}

impl<T> TlsIoAdapter<T> {
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: ErrorType06> ErrorType07 for TlsIoAdapter<T>
where
    T::Error: Error06,
{
    type Error = TlsIoError<T::Error>;
}

/// Records ciphertext byte flow for the product HTTPS session's stall triage;
/// a no-op in the isolated probe build.
#[cfg(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
fn count_io(rx: usize, tx: usize) {
    crate::firmware::wifi_residency::record_fetch_tls_io(rx, tx);
}
#[cfg(not(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
)))]
fn count_io(_rx: usize, _tx: usize) {}

#[cfg(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
fn read_request(requested: usize) {
    crate::firmware::wifi_residency::record_fetch_tls_read_request(requested);
}
#[cfg(not(all(
    feature = "app_fetch_https",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
)))]
fn read_request(_requested: usize) {}

impl<T: Read06> Read07 for TlsIoAdapter<T>
where
    T::Error: Error06,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        // Captured before awaiting so a stall preserves the requested size.
        read_request(buf.len());
        let count = Read06::read(&mut self.inner, buf)
            .await
            .map_err(TlsIoError)?;
        count_io(count, 0);
        Ok(count)
    }
}

impl<T: Write06> Write07 for TlsIoAdapter<T>
where
    T::Error: Error06,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let count = Write06::write(&mut self.inner, buf)
            .await
            .map_err(TlsIoError)?;
        count_io(0, count);
        Ok(count)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Write06::flush(&mut self.inner).await.map_err(TlsIoError)
    }
}
