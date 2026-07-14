//! PIO backend boundary.

pub mod blocking;
#[cfg(feature = "dma")]
pub mod dma;
#[cfg(any(
    test,
    all(
        any(feature = "rp2040-embassy", feature = "rp235xa-embassy"),
        target_os = "none"
    )
))]
pub(crate) mod word_stream;
