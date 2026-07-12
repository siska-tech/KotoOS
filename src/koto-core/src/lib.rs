#![cfg_attr(not(test), no_std)]

pub const KOTO_COPYRIGHT_NOTICE: &str = "Copyright 2026 Siska-Tech Lab.";

pub mod audio;
pub mod boot_splash;
pub mod dirty_tiles;
pub mod font;
pub mod fs;
pub mod hal;
pub mod ime;
pub mod keymap;
pub mod kotodos;
pub mod kpa;
pub mod layout;
pub mod memo;
pub mod memo_ime;
pub mod package;
pub mod psram;
pub mod raster;
pub mod render;
/// The KotoVM bytecode interpreter, extracted into the standalone `koto-vm`
/// crate. Re-exported here as `koto_core::runtime` so existing consumers
/// (`koto-pico`, `koto-sim`) keep their `koto_core::runtime::*` paths unchanged.
pub use koto_vm as runtime;
pub mod shell;
pub mod skk;

pub use audio::{AudioError, PcmMixer, PcmSliceStream};
pub use boot_splash::{
    splash_progress_rect, splash_step_rect, BootSplash, BootStep, BootStepStatus, SPLASH_STEP_COUNT,
};
pub use dirty_tiles::{coalesce_dirty_tiles, coalesce_rects, TileBand};
pub use font::{BitmapFont, FontError, Glyph};
pub use fs::{FsError, Sandbox, SandboxPath, MAX_VIRTUAL_PATH_LEN};
pub use hal::{
    AudioBuffer, AudioHal, AudioSource, Buttons, FileHandle, FileMode, FsHal, HalError, InputHal,
    InputState, PixelFormat, PowerHal, PowerState, PsramHal, Rect, Surface, VideoHal,
};
pub use ime::{
    ImeError, ImeOutput, RomajiKanaInput, StickyShift, StickyShiftKey, StickyShiftOutput,
    MAX_ROMAJI_BUFFER,
};
pub use kotodos::{
    KotoDosMode, KOTODOS_GAME_HEIGHT, KOTODOS_GAME_REGION, KOTODOS_SCREEN_HEIGHT,
    KOTODOS_SCREEN_WIDTH, KOTODOS_SURFACE, KOTODOS_UI_HEIGHT, KOTODOS_UI_REGION,
};
pub use kpa::{
    KpaEntry, KpaError, KpaHeader, KpaReader, PreloadWindow, KPA_ENTRY_SIZE,
    KPA_FIRST_ASSET_ALIGNMENT, KPA_FLAG_ENTRY, KPA_FLAG_PRELOAD, KPA_FLAG_SEQUENTIAL,
    KPA_HEADER_SIZE, KPA_MAGIC, KPA_PAYLOAD_ALIGNMENT, KPA_VERSION_MAJOR, KPA_VERSION_MINOR,
};
pub use layout::{CellMetrics, LayoutError, TextLayout, MAX_IME_LINES};
pub use memo::{
    MemoDirty, MemoDirtyLines, MemoEditor, MemoError, MemoMove, MEMO_DEFAULT_CAPACITY,
    MEMO_MAX_DIRTY_LINES,
};
pub use memo_ime::{
    KotoMemoIme, MemoIme, MemoImeError, MemoImeKey, MemoImeLine, MemoImeMode,
    MEMO_IME_CANDIDATE_CAPACITY, MEMO_IME_READING_CAPACITY,
};
pub use package::{
    IconError, ManifestError, ManifestFields, PackageIcon, PackageIconStyle, PackageIconTheme,
    PackageInfo, PackageList, PackageManifest, KPA_MANIFEST_FORMAT, KPA_MANIFEST_VERSION,
    MAX_APP_ID_LEN, MAX_ENTRY_PATH_LEN, MAX_ICON_PATH_LEN, MAX_NAME_LEN, MAX_PACKAGES,
    MAX_RUNTIME_NAME_LEN, PACKAGE_ICON_1BPP_BYTES, PACKAGE_ICON_HEIGHT, PACKAGE_ICON_WIDTH,
};
pub use psram::{PsramBlocks, PsramError, PSRAM_BLOCK_SIZE};
pub use raster::{Canvas, Rgb565};
pub use render::{RenderCommand, RenderCommandList, RenderError, RenderSurface, RenderUpdate};
pub use runtime::{
    debug_map, verify_kbc, verify_kbc_streaming, BytecodeSession, BytecodeVm, CodeSource,
    CodeTileTransition, DebugMap, DebugMapError, HostCallOutcome, HostErrorCode, KbcHeader,
    RuntimeLimits, SessionError, SliceCode, SourceLocation, VerifiedProgram, VerifyError, VmError,
    VmHost, VmInputSnapshot, VmRunResult, HOST_ABI_MAJOR, HOST_ABI_MINOR, KBC_DEBUG_ENTRY_SIZE,
    KBC_DEBUG_HEADER_SIZE, KBC_DEBUG_MAGIC, KBC_DEBUG_VERSION, KBC_HEADER_SIZE, KBC_MAGIC,
    KBC_VERSION_MAJOR, KBC_VERSION_MINOR,
};
pub use shell::{ShellAction, ShellItem, ShellSound, ShellState, ShellStatusText};
pub use skk::{
    Candidates, DictEntry, SkkDictAccess, SkkError, SkkIndex, SkkLeadingIndex, SkkRead, SliceDict,
    WindowedDict, DEFAULT_INDEX_CAPACITY, MAX_LEADING_KEY_BYTES, SKK_LOOKUP_WINDOW_BYTES,
};
