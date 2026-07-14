use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::audio::{AudioEvent, SimAudio, DEFAULT_SAMPLE_RATE};
use crate::framebuffer::{describe_render_command, Framebuffer, RenderRecorder};
use crate::host_fs::{HostFile, HostFs};
use crate::manifest::{parse_launch_manifest, parse_manifest, PackageLaunch};

use koto_core::package::validate_app_id;
use koto_core::shell::{SortMode, SHELL_SURFACE};
use koto_core::{
    verify_kbc, BitmapFont, Buttons, BytecodeSession, BytecodeVm, Canvas, CellMetrics, FileHandle,
    FileMode, FsHal, HostCallOutcome, InputState, KotoMemoIme, KpaReader, MemoEditor, MemoImeKey,
    MemoImeLine, MemoImeMode, MemoMove, PackageIcon, PackageInfo, PackageList, PixelFormat, Rect,
    RenderSurface, Rgb565, RuntimeLimits, Sandbox, SessionError, ShellAction, ShellState,
    SkkLeadingIndex, VmHost, VmInputSnapshot, VmRunResult, WindowedDict, SKK_LOOKUP_WINDOW_BYTES,
};

#[cfg(test)]
use crate::framebuffer::write_bmp;
#[cfg(test)]
use koto_core::HalError;

mod audio_capture;
mod budget;
mod error;
mod host;
mod inspector;
mod memo_validation;
mod orchestration;
mod package;
mod render;
mod save_data;
mod scenario;
mod session;
mod shell_prefs;

pub use audio_capture::*;
pub use budget::*;
pub use error::*;
use host::*;
pub use inspector::*;
pub use memo_validation::*;
pub use orchestration::*;
pub use package::*;
pub use render::*;
pub use save_data::*;
pub use scenario::*;
pub use session::*;
pub use shell_prefs::*;

#[cfg(test)]
include!("tests.rs");
