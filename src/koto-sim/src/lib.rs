//! Host-side KotoOS simulator backend.
//!
//! Public exports stay at the crate root while implementation is split into
//! focused modules. This preserves the CLI and test API during KOTO-0061.

pub mod audio;
mod framebuffer;
mod host_fs;
pub mod koto_blocks_audio;
mod manifest;
mod runtime;

#[cfg(feature = "window")]
pub mod window;

pub use framebuffer::{
    describe_render_command, framebuffer_to_argb, load_font_bytes, render_splash_frame, write_bmp,
    Framebuffer, RenderRecorder,
};
pub use host_fs::{HostDirEntry, HostFile, HostFs};
pub use manifest::{parse_launch_manifest, parse_manifest, PackageLaunch};
pub use runtime::*;
