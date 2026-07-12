//! KotoVM: the bytecode interpreter for KotoOS apps.
//!
//! This crate owns the VM-side contract — opcode set, bytecode decoder, value
//! and stack/call-frame handling, runtime limits, error/trap types, execution
//! stats, and the two integration traits ([`CodeSource`] for the bytecode reader
//! and [`VmHost`] for hostcalls). It is `no_std` and carries no platform
//! dependencies; KotoOS supplies the concrete [`CodeSource`]/[`VmHost`]
//! implementations (PSRAM code window, graphics/audio/input hostcalls).
#![cfg_attr(not(test), no_std)]

pub const KBC_MAGIC: [u8; 4] = *b"KBC1";
pub const KBC_HEADER_SIZE: usize = 64;
pub const KBC_VERSION_MAJOR: u16 = 1;
pub const KBC_VERSION_MINOR: u16 = 0;
pub const HOST_ABI_MAJOR: u16 = 1;
pub const HOST_ABI_MINOR: u16 = 15;
/// Size of the VM local register file. Locals are shared across all of an app's
/// functions (the compiler gives each function a non-overlapping `slot_base`),
/// so this is the ceiling on *total* named locals a program may declare, less the
/// three codegen scratch slots. It is a deliberate, predictable bound for
/// RP2040-class SRAM (`[i32; 48]` = 192 bytes per VM), sized at roughly twice the
/// most complex current app's needs. An app that approaches it is the signal to
/// teach the compiler per-scope slot reuse, not to raise this further.
pub const VM_LOCAL_SLOTS: usize = 48;
pub const KBC_DEBUG_MAGIC: [u8; 4] = *b"KDBG";
pub const KBC_DEBUG_VERSION: u16 = 1;
pub const KBC_DEBUG_HEADER_SIZE: usize = 16;
pub const KBC_DEBUG_ENTRY_SIZE: usize = 14;

pub mod opcode {
    pub const NOP: u8 = 0x00;
    pub const HALT: u8 = 0x01;
    pub const BR: u8 = 0x02;
    pub const BR_IF_ZERO: u8 = 0x03;
    pub const CALL: u8 = 0x04;
    pub const RET: u8 = 0x05;

    pub const PUSH_I16: u8 = 0x10;
    pub const DUP: u8 = 0x11;
    pub const DROP: u8 = 0x12;
    pub const SWAP: u8 = 0x13;

    pub const LOAD_LOCAL: u8 = 0x20;
    pub const STORE_LOCAL: u8 = 0x21;

    pub const ADD_I32: u8 = 0x30;
    pub const SUB_I32: u8 = 0x31;
    pub const MUL_I32: u8 = 0x32;
    pub const DIV_I32: u8 = 0x33;
    pub const AND_I32: u8 = 0x34;
    pub const OR_I32: u8 = 0x35;
    pub const XOR_I32: u8 = 0x36;
    pub const SHL_I32: u8 = 0x37;
    pub const SHR_I32: u8 = 0x38;

    pub const LOAD8: u8 = 0x40;
    pub const STORE8: u8 = 0x41;
    pub const LOAD16: u8 = 0x42;
    pub const STORE16: u8 = 0x43;
    pub const LOAD32: u8 = 0x44;
    pub const STORE32: u8 = 0x45;

    pub const HOST_CALL: u8 = 0x50;
}

pub mod host_call {
    pub const EXIT: u8 = 0x00;
    pub const YIELD_FRAME: u8 = 0x01;
    pub const DRAW_RECT: u8 = 0x10;
    pub const DRAW_TEXT: u8 = 0x11;
    pub const DRAW_PIXELS_RGB565: u8 = 0x12;
    /// Draw UTF-8 text in a caller-chosen RGB565 colour. Args `(x, y, ptr, len, rgb565)`.
    pub const DRAW_TEXT_COLOR: u8 = 0x13;

    // Game2D retained tile renderer (KOTO-0135). The host holds a tilemap layer
    // of 16x16 cells; the app writes only the cells that change and presents
    // once per frame, instead of re-blitting the whole board every frame. Tile
    // art stays in the app heap, referenced by byte offset (`tile_ref`), reusing
    // the `draw_pixels_rgb565` heap re-read. IDs `0x19`-`0x1C` are the retained
    // sprite/stamp layer (KOTO-0140) and `0x1D`-`0x1F` the retained text layer
    // (KOTO-0141); see GAME2D_ABI.md.
    /// Write one tilemap cell. Args `(layer, x, y, tile_ref)`; `tile_ref` is the
    /// app-heap byte offset of a 16x16 RGB565 tile, or `< 0` to clear the cell.
    pub const GAME2D_SET_TILE: u8 = 0x14;
    /// Clear every cell of a tilemap layer. Arg `(layer)`.
    pub const GAME2D_CLEAR_LAYER: u8 = 0x15;
    /// Composite and present the retained tilemap for this frame. No args.
    pub const GAME2D_PRESENT: u8 = 0x16;

    // Game2D retained static/background command layer (KOTO-0136). Between
    // `begin` and `end` the host captures `draw_rect` / `draw_text` /
    // `draw_text_color` / `draw_pixels` into a retained layer instead of the
    // per-frame immediate list; the presenter composites it beneath the board
    // tilemap and the immediate commands every frame. An app builds it once (or
    // when its layout changes) so static page/well/grid/panel/label UI no longer
    // costs a host call and an immediate command every frame.
    /// Begin static-layer capture: clear the retained static layer and route
    /// subsequent draw calls into it until `end`. No args.
    pub const GAME2D_STATIC_BEGIN: u8 = 0x17;
    /// End static-layer capture: route draw calls back to the per-frame immediate
    /// list. No args.
    pub const GAME2D_STATIC_END: u8 = 0x18;

    // Game2D retained sprite/stamp layer (KOTO-0140). A *stamp* is a reusable
    // position-independent cell pattern (defined once); a *sprite* is a retained
    // placed instance of a stamp at a pixel position drawing a given tile. Stamp
    // cell data lives in the app heap by byte offset (and naturally in the
    // KOTO-0139 const heap image); the host stores only descriptors. v1 is
    // cell-stamp-only (no pixel stamps). The presenter composites the sprite
    // layer in fixed z-order above the tile layer, below the immediate list.
    /// Register a stamp descriptor. Args `(stamp_id, cells_off, count, format)`:
    /// `count` cells at heap byte offset `cells_off`; `format 0` = packed
    /// `(dcol,drow)` nibbles (the KOTO-0138 cell layout: nibble `p` = `drow*4+dcol`).
    pub const GAME2D_STAMP_DEFINE: u8 = 0x19;
    /// Create or update a retained sprite. Args `(inst_id, stamp_id, x, y, tile_ref)`:
    /// draw `stamp_id`'s cells at `(x + dcol*16, y + drow*16)`, each blitting the
    /// 16x16 tile at heap byte offset `tile_ref`.
    pub const GAME2D_SPRITE_SET: u8 = 0x1A;
    /// Hide sprite `inst_id` (its footprint becomes a dirty erase next present).
    /// Arg `(inst_id)`.
    pub const GAME2D_SPRITE_HIDE: u8 = 0x1B;
    /// Hide every sprite. No args.
    pub const GAME2D_SPRITE_CLEAR_ALL: u8 = 0x1C;

    // Game2D retained text layer (KOTO-0141). A retained text item is a string
    // pinned at a pixel position with a colour, keyed by a stable `id`. Updating an
    // id compares by id (not array position, unlike the immediate `draw_text`
    // list), so a value that changes repaints only its own row band and a value
    // that does not change costs nothing — removing the per-frame text churn that
    // shifted the immediate command count and forced positional-diff full repaints
    // (KOTO-0143 `CommandCountShift`). The presenter composites the text layer in
    // fixed z-order above the sprite layer, below the immediate list. v1 keeps the
    // existing pixel-font row-height band as the footprint (no tight CJK metrics).
    /// Create or update retained text item `id`. Args `(id, x, y, str_ptr, len,
    /// rgb565)`: draw the UTF-8 string at `str_ptr`/`len` (an app-heap byte range,
    /// decoded like `draw_text`) at `(x, y)` in colour `rgb565`.
    pub const GAME2D_TEXT_SET: u8 = 0x1D;
    /// Hide retained text item `id` (its footprint becomes a dirty erase next
    /// present). Arg `(id)`.
    pub const GAME2D_TEXT_HIDE: u8 = 0x1E;
    /// Hide every retained text item. No args.
    pub const GAME2D_TEXT_CLEAR_ALL: u8 = 0x1F;

    pub const INPUT_SNAPSHOT: u8 = 0x20;
    /// Frame-stable typed-character input. Returns `(codepoint, intent_bits)`.
    pub const TEXT_INPUT: u8 = 0x21;
    pub const AUDIO_SUBMIT_I16: u8 = 0x30;
    /// Trigger a one-shot host sound effect by id (host ABI minor 8). Arg `(id)`.
    pub const PLAY_SFX: u8 = 0x31;
    /// Start a looping host background-music track by id (host ABI minor 8). Arg `(id)`.
    pub const PLAY_BGM: u8 = 0x32;
    /// Stop the looping host background-music track (host ABI minor 8). No args.
    pub const STOP_BGM: u8 = 0x33;
    /// Start looping KotoMML from a package asset (host ABI minor 10).
    /// Args `(path_ptr, path_len)`.
    pub const PLAY_BGM_ASSET: u8 = 0x34;
    /// Play one-shot KotoMML from a package asset (host ABI minor 11).
    /// Args `(path_ptr, path_len)`.
    pub const PLAY_SFX_ASSET: u8 = 0x35;
    pub const FILE_OPEN: u8 = 0x40;
    pub const FILE_READ: u8 = 0x41;
    pub const FILE_WRITE: u8 = 0x42;
    pub const FILE_CLOSE: u8 = 0x43;
    /// Read a read-only package asset fully into a heap buffer in one shot.
    /// Args `(path_ptr, path_len, dst_ptr, dst_max)`; returns bytes copied / `-1`.
    pub const ASSET_LOAD: u8 = 0x44;

    // Text-composition / text-buffer service (host ABI minor 1). These keep the
    // VM neutral: the host owns the IME and editor models; bytecode drives them.
    /// Feed one key into the host IME+editor. Args `(kind, codepoint)`.
    pub const IME_FEED_KEY: u8 = 0x60;
    /// Run dictionary conversion on the current reading. No args.
    pub const IME_CONVERT: u8 = 0x61;
    /// Serialize the IME composition line into the app heap. Args `(ptr, max_len)`.
    pub const IME_QUERY_LINE: u8 = 0x62;
    /// Move the editor cursor. Arg `(dir)` from [`edit_dir`].
    pub const EDIT_MOVE: u8 = 0x63;
    /// Delete around the cursor. Arg `(kind)` from [`edit_delete`].
    pub const EDIT_DELETE: u8 = 0x64;
    /// Load document text from the app heap. Args `(ptr, len)`.
    pub const EDIT_LOAD: u8 = 0x65;
    /// Read document text into the app heap. Args `(ptr, max_len)`. Returns `(len, cursor)`.
    pub const EDIT_QUERY_TEXT: u8 = 0x66;
    /// Render the IME composition as a plain UTF-8 display string into the app
    /// heap (candidate, else reading plus pending romaji). Args `(ptr, max_len)`.
    pub const IME_DISPLAY: u8 = 0x67;
    /// Read one visible editor line. Args `(ptr, max_len, row)`. Returns `(len)`.
    pub const EDIT_VISIBLE_LINE: u8 = 0x68;
    /// Read the visible cursor position. No args. Returns `(col, row)`.
    pub const EDIT_CURSOR_VIEW: u8 = 0x69;
    /// Read the first visible document row. No args. Returns `(scroll_row)`.
    pub const EDIT_SCROLL_ROW: u8 = 0x6A;
    /// Read the editor viewport cell metrics. No args. Returns `(cell_w, cell_h)`.
    pub const EDIT_VIEW_METRICS: u8 = 0x6B;
    /// Write a compact cursor status string. Args `(ptr, max_len)`. Returns `(bytes_written)`.
    pub const EDIT_CURSOR_STATUS: u8 = 0x6C;
    /// Read the editor document line count. No args. Returns `(total_lines)`.
    pub const EDIT_TOTAL_LINES: u8 = 0x6D;
    /// Read the soft-wrap state. No args. Returns `(1)` when wrapping, else `(0)`.
    pub const EDIT_WRAP: u8 = 0x6E;
    /// Read horizontal scroll metrics for a no-wrap scrollbar. No args. Returns
    /// `(hscroll_columns, cursor_line_columns)`.
    pub const EDIT_HSCROLL_VIEW: u8 = 0x6F;

    /// Enumerate one entry of the app save-data sandbox directory (host ABI minor
    /// 7). Args `(ptr, max_len, index)`; writes the `index`-th filename (sorted)
    /// into the app heap and returns `(entry_count, name_len)`.
    pub const DIR_LIST: u8 = 0x70;

    /// Reserve the bottom `rows` rows of the editor viewport (host ABI minor 9) so
    /// the host keeps the cursor scrolled above them. Apps set this to the number
    /// of rows an overlay (such as the IME conversion panel) covers, and back to
    /// `0` once it is gone. Arg `(rows)`. No result.
    pub const EDIT_RESERVE_ROWS: u8 = 0x71;
    /// Configure the host text editor viewport in app-visible columns and rows.
    /// Args `(cols, rows)` (host ABI minor 12).
    pub const EDIT_CONFIGURE: u8 = 0x72;

    /// A short, stable name for a host-call id, for diagnostics and the runtime
    /// inspector. Unknown ids return `"unknown"`.
    pub fn name(id: u8) -> &'static str {
        match id {
            EXIT => "exit",
            YIELD_FRAME => "yield_frame",
            DRAW_RECT => "draw_rect",
            DRAW_TEXT => "draw_text",
            DRAW_TEXT_COLOR => "draw_text_color",
            DRAW_PIXELS_RGB565 => "draw_pixels_rgb565",
            GAME2D_SET_TILE => "game2d_set_tile",
            GAME2D_CLEAR_LAYER => "game2d_clear_layer",
            GAME2D_PRESENT => "game2d_present",
            GAME2D_STATIC_BEGIN => "game2d_static_begin",
            GAME2D_STATIC_END => "game2d_static_end",
            GAME2D_STAMP_DEFINE => "game2d_stamp_define",
            GAME2D_SPRITE_SET => "game2d_sprite_set",
            GAME2D_SPRITE_HIDE => "game2d_sprite_hide",
            GAME2D_SPRITE_CLEAR_ALL => "game2d_sprite_clear_all",
            GAME2D_TEXT_SET => "game2d_text_set",
            GAME2D_TEXT_HIDE => "game2d_text_hide",
            GAME2D_TEXT_CLEAR_ALL => "game2d_text_clear_all",
            INPUT_SNAPSHOT => "input_snapshot",
            TEXT_INPUT => "text_input",
            AUDIO_SUBMIT_I16 => "audio_submit_i16",
            PLAY_SFX => "play_sfx",
            PLAY_BGM => "play_bgm",
            STOP_BGM => "stop_bgm",
            PLAY_BGM_ASSET => "play_bgm_asset",
            PLAY_SFX_ASSET => "play_sfx_asset",
            FILE_OPEN => "file_open",
            FILE_READ => "file_read",
            FILE_WRITE => "file_write",
            FILE_CLOSE => "file_close",
            ASSET_LOAD => "asset_load",
            IME_FEED_KEY => "ime_feed_key",
            IME_CONVERT => "ime_convert",
            IME_QUERY_LINE => "ime_query_line",
            EDIT_MOVE => "edit_move",
            EDIT_DELETE => "edit_delete",
            EDIT_LOAD => "edit_load",
            EDIT_QUERY_TEXT => "edit_query_text",
            IME_DISPLAY => "ime_display",
            EDIT_VISIBLE_LINE => "edit_visible_line",
            EDIT_CURSOR_VIEW => "edit_cursor_view",
            EDIT_SCROLL_ROW => "edit_scroll_row",
            EDIT_VIEW_METRICS => "edit_view_metrics",
            EDIT_CURSOR_STATUS => "edit_cursor_status",
            EDIT_TOTAL_LINES => "edit_total_lines",
            EDIT_WRAP => "edit_wrap",
            EDIT_HSCROLL_VIEW => "edit_hscroll_view",
            DIR_LIST => "dir_list",
            EDIT_RESERVE_ROWS => "edit_reserve_rows",
            EDIT_CONFIGURE => "edit_configure",
            _ => "unknown",
        }
    }
}

/// `kind` values for [`host_call::IME_FEED_KEY`], mirroring the host IME key model.
pub mod ime_key {
    pub const CHARACTER: i32 = 0;
    pub const SHIFT: i32 = 1;
    pub const CONVERT: i32 = 2;
    pub const COMMIT: i32 = 3;
    pub const CANCEL: i32 = 4;
    pub const OTHER: i32 = 5;
    pub const TOGGLE: i32 = 6;
    /// Backspace within an active composition: delete the last reading/pending
    /// character, ending the composition when nothing remains.
    pub const BACKSPACE: i32 = 7;
}

/// `dir` values for [`host_call::EDIT_MOVE`].
pub mod edit_dir {
    pub const LEFT: i32 = 0;
    pub const RIGHT: i32 = 1;
    pub const UP: i32 = 2;
    pub const DOWN: i32 = 3;
    pub const HOME: i32 = 4;
    pub const END: i32 = 5;
}

/// `kind` values for [`host_call::EDIT_DELETE`].
pub mod edit_delete {
    pub const BACKSPACE: i32 = 0;
    pub const FORWARD: i32 = 1;
}

/// Edit-intent bit flags carried in [`VmInputSnapshot::intent_bits`] and returned
/// by [`host_call::TEXT_INPUT`]. The host maps physical keys to intents; bytecode
/// reads them to decide which host calls to make.
pub mod text_intent {
    pub const SHIFT: u32 = 1 << 0;
    pub const CONVERT: u32 = 1 << 1;
    pub const COMMIT: u32 = 1 << 2;
    pub const CANCEL: u32 = 1 << 3;
    pub const BACKSPACE: u32 = 1 << 4;
    pub const DELETE: u32 = 1 << 5;
    pub const LEFT: u32 = 1 << 6;
    pub const RIGHT: u32 = 1 << 7;
    pub const UP: u32 = 1 << 8;
    pub const DOWN: u32 = 1 << 9;
    pub const HOME: u32 = 1 << 10;
    pub const END: u32 = 1 << 11;
    pub const NEWLINE: u32 = 1 << 12;
    pub const SAVE: u32 = 1 << 13;
    pub const EXIT: u32 = 1 << 14;
    pub const IME_TOGGLE: u32 = 1 << 15;
    /// Open the app's file open/save surface (host ABI minor 7).
    pub const OPEN: u32 = 1 << 16;
    /// Create a new document in apps that provide a document workflow.
    pub const NEW: u32 = 1 << 17;
}

/// Sound-effect ids for the host-owned audio service ([`host_call::PLAY_SFX`],
/// host ABI minor 8). App-specific music is package-local KotoMML played through
/// [`host_call::PLAY_BGM_ASSET`].
pub mod audio_id {
    pub const SFX_SHELL_NAV: i32 = 6;
    pub const SFX_SHELL_CONFIRM: i32 = 7;
    pub const SFX_SHELL_CANCEL: i32 = 8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyError {
    TruncatedHeader,
    BadMagic,
    UnsupportedVersion,
    BadHeaderSize,
    NonzeroReserved,
    NonzeroFlags,
    BadBytecodeSize,
    BadCodeRange,
    BadDataRange,
    /// `rodata_size` exceeds the program's heap request, so it cannot be the
    /// initial image of `heap[0..rodata_size]` (KOTO-0139).
    RodataExceedsHeap,
    OverlappingRanges,
    BadEntry,
    ResourceLimitExceeded,
    UnsupportedHostAbi,
    UnknownOpcode,
    BadBranch,
    StackUnderflow,
    StackOverflow,
    BadHostCall,
    BadInstruction,
}

/// The capacities a runtime offers to a bytecode program. This is the single
/// source of the simulator profile (KOTO-0060): `verify_kbc` rejects any program
/// whose header requests more than these. The simulator builds its `BytecodeVm`
/// stack/call depth and per-frame fuel from these, and `max_heap_bytes` is the
/// device heap *ceiling* — each app is given a heap sized to its own KBC header
/// request (per-app profile, KOTO-0096), which must not exceed this ceiling.
/// `frame_fuel` is the host-side per-frame instruction budget (not a program
/// request, so the verifier ignores it).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub max_stack_slots: u16,
    pub max_call_depth: u16,
    pub max_heap_bytes: u32,
    pub frame_fuel: u32,
    pub treat_ret_as_exit: bool,
}

impl RuntimeLimits {
    /// The canonical KotoSim runtime profile. The simulator's `SIM_VM_*` /
    /// `SIM_FRAME_FUEL` constants and the compiler's header-request floors all
    /// derive from these, so verification can never be more permissive than launch.
    ///
    /// `frame_fuel` is sized for an interactive, full-screen app: a tile/sprite game
    /// such as KotoBlocks repaints a 200-cell board plus a side panel every frame and
    /// may shift rows on a line clear, which is several times the cost of the memo /
    /// shell UIs. 60,000 instructions/frame leaves comfortable headroom over the
    /// heaviest observed KotoBlocks frame while remaining a tiny slice of the RP2040's
    /// per-frame instruction budget at 60 fps. `max_heap_bytes` is the device heap
    /// ceiling (16 KB); individual apps request only what they need in their header.
    pub const fn simulator_default() -> Self {
        Self {
            max_stack_slots: 16,
            max_call_depth: 4,
            max_heap_bytes: 16 * 1024,
            frame_fuel: 60_000,
            treat_ret_as_exit: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KbcHeader {
    pub bytecode_size: u32,
    pub code_offset: u32,
    pub code_size: u32,
    pub rodata_offset: u32,
    pub rodata_size: u32,
    pub entry_word: u32,
    pub max_stack_slots: u16,
    pub max_call_depth: u16,
    pub max_heap_bytes: u32,
    pub host_abi_major: u16,
    pub host_abi_minor: u16,
    pub debug_offset: u32,
    pub debug_size: u32,
}

impl KbcHeader {
    /// Parse and structurally validate a KBC header from at least the first
    /// [`KBC_HEADER_SIZE`] bytes of a program. Platforms that stream code from
    /// external storage (KOTO-0127) read the resident header first to learn the
    /// code segment's offset/size and heap request before staging the body.
    pub fn parse(bytes: &[u8]) -> Result<Self, VerifyError> {
        parse_header(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedProgram {
    header: KbcHeader,
    code_words: u32,
}

impl VerifiedProgram {
    pub const fn header(&self) -> KbcHeader {
        self.header
    }

    pub const fn code_words(&self) -> u32 {
        self.code_words
    }

    pub const fn code_range(&self) -> (usize, usize) {
        (
            self.header.code_offset as usize,
            (self.header.code_offset + self.header.code_size) as usize,
        )
    }

    /// The `[start, end)` byte range of the const heap image within the `.kbc`, or
    /// `None` when the program has no `rodata` (KOTO-0139). A loader copies these
    /// bytes into `heap[0..rodata_size]` before the first frame; the verifier has
    /// already bounded `rodata_size <= max_heap_bytes`.
    pub const fn rodata_range(&self) -> Option<(usize, usize)> {
        if self.header.rodata_size == 0 {
            None
        } else {
            Some((
                self.header.rodata_offset as usize,
                (self.header.rodata_offset + self.header.rodata_size) as usize,
            ))
        }
    }
}

/// Random-access source of a program's 4-byte code words.
///
/// The VM and verifier read code only one word at a time, indexed by code-word
/// position (the program counter), and never need rodata, the debug segment, or
/// the file header at runtime (KOTO-0127). Abstracting that single access lets the
/// same session contract run from whatever storage holds the code segment: the
/// simulator and host tools back it with a direct byte slice ([`SliceCode`]);
/// PicoCalc backs it with a small SRAM window cached over PSRAM-resident code, so
/// a program never has to fit entirely in SRAM.
pub trait CodeSource {
    /// The 4-byte little-endian instruction word at code-word `index`, or `None`
    /// if `index` is out of range or the backing store cannot serve it.
    fn word(&mut self, index: u32) -> Option<[u8; 4]>;

    /// Straight-line batch fetch (KOTO-0169 Stage 1, the H1 fetch fast path):
    /// copy up to `dst.len()` *consecutive* code words starting at `index` into
    /// `dst` and return how many were served (`0` exactly when `word(index)`
    /// would return `None`). The served bytes must be identical to calling
    /// [`Self::word`] for each index, and the implementation must not perform
    /// any backing-store work beyond what `word(index)` itself would — i.e. it
    /// resolves the *first* word exactly like `word` and then only extends the
    /// run over storage that resolution already made resident (the device
    /// window clamps the run at its tile boundary, so refill counts and timing
    /// are unchanged for any execution path). Serving fewer words than
    /// requested is always valid.
    ///
    /// The default serves exactly one word via [`Self::word`], so existing
    /// sources (and test doubles) keep today's one-fetch-per-instruction
    /// behavior and observable fetch metrics.
    fn word_run(&mut self, index: u32, dst: &mut [[u8; 4]]) -> usize {
        match dst.first_mut() {
            Some(slot) => match self.word(index) {
                Some(word) => {
                    *slot = word;
                    1
                }
                None => 0,
            },
            None => 0,
        }
    }

    /// Placement hint for the device frame loop (KOTO-0169 Stage 2, H2-b):
    /// `true` on a source whose interpretation is the device's hot path (the
    /// PSRAM code window). Under the `ram_interpreter` feature the session
    /// runs such a source through the SRAM-placed twin of the frame loop, so
    /// only that instantiation pays RAM — the resident-slice small-app
    /// fallback (and every host/sim source) stays in flash. Without the
    /// feature the constant is never read. This is a *placement* hint only;
    /// both twins execute identical code.
    const PLACE_HOT_LOOP_IN_SRAM: bool = false;

    /// Bytecode-fetch diagnostics (KOTO-0134). A code source that serves the VM
    /// from a small SRAM window over slower backing store (the device PSRAM
    /// window) refills that window on a fetch outside the cached tile; a hot loop
    /// that ping-pongs across tiles can refill many times per frame, the dominant
    /// cost once the draw path is cheap. These let the frame loop attribute
    /// `vm_us` to code-fetch thrashing without coupling to the device type.
    ///
    /// `reset_fetch_metrics` zeroes the per-frame counters (call before a frame
    /// step); `fetch_refills` is the number of window refills since the reset; and
    /// `fetch_distinct_tiles` is how many distinct tiles those refills touched (a
    /// ping-pong between two tiles shows few distinct tiles but many refills). The
    /// default — a resident SRAM slice that never refills — reports zero.
    fn reset_fetch_metrics(&mut self) {}
    fn fetch_refills(&self) -> u32 {
        0
    }
    fn fetch_distinct_tiles(&self) -> u32 {
        0
    }

    /// Per-frame refill count by tile index (KOTO-0136 triage): `tile_refills()[t]`
    /// is how many times tile `t` was refilled since the last reset. A concentrated
    /// histogram (a few tiles with high counts) is a hot-path tile ping-pong; an
    /// even spread is a many-tile walk. The default reports an empty histogram.
    fn tile_refills(&self) -> &[u16] {
        &[]
    }

    /// Per-frame top tile→tile refill transitions (KOTO-0136 triage), bounded to a
    /// small table: the dominant `from>to` pairs of a ping-pong (e.g. `1>3` and
    /// `3>1`). The default reports no transitions.
    fn tile_transitions(&self) -> &[CodeTileTransition] {
        &[]
    }

    /// PSRAM code-window refill timing (KOTO-0132 phase 1). A window over slower
    /// backing store times each refill's synchronous transfer so the frame loop can
    /// measure whether `vm_us` is dominated by PSRAM refill latency rather than VM
    /// work. `cw_refill_us_total` is the microseconds spent in refills since the last
    /// [`Self::reset_fetch_metrics`] (saturating), `cw_refill_us_max` the slowest
    /// single refill, and `cw_refill_bytes` the bytes those refills transferred. The
    /// default — a resident SRAM slice that never refills — reports zero.
    fn cw_refill_us_total(&self) -> u32 {
        0
    }
    fn cw_refill_us_max(&self) -> u32 {
        0
    }
    fn cw_refill_bytes(&self) -> u32 {
        0
    }
}

/// One tile→tile code-window refill transition and its per-frame count, for the
/// [`CodeSource::tile_transitions`] thrash diagnostic (KOTO-0136). Identifies the
/// hot ping-pong pairs behind a high refill count.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CodeTileTransition {
    pub from: u8,
    pub to: u8,
    pub count: u16,
}

/// [`CodeSource`] over a full in-memory `.kbc` byte slice. `code_start` is the
/// byte offset of code word 0 (the program's `code_offset`). This is the resident
/// path used by KotoSim, host tools, and the device's small-app fallback; its
/// behavior is identical to indexing the slice directly.
pub struct SliceCode<'a> {
    bytes: &'a [u8],
    code_start: usize,
}

impl<'a> SliceCode<'a> {
    pub const fn new(bytes: &'a [u8], code_start: usize) -> Self {
        Self { bytes, code_start }
    }
}

impl CodeSource for SliceCode<'_> {
    fn word(&mut self, index: u32) -> Option<[u8; 4]> {
        let offset = self
            .code_start
            .checked_add((index as usize).checked_mul(4)?)?;
        let end = offset.checked_add(4)?;
        self.bytes.get(offset..end)?.try_into().ok()
    }
}

/// A [`CodeSource`] adapter that counts code-fetch traffic against an inner source,
/// for benches and tests (KOTO-0153). It is transparent — every `word` and every
/// optional fetch-metric method forwards to the wrapped source, so wrapping does not
/// change what the VM observes. Counting code reads through this wrapper, rather than
/// adding counters to the [`CodeSource`] trait, keeps profiling hooks out of the
/// runtime path: production `CodeSource`s (the resident [`SliceCode`], the device
/// PSRAM window) are untouched. `reads` is the number of `word` calls served (a code
/// word is 4 bytes, so [`Self::bytes_read`] is `reads * 4`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CountingCode<C> {
    inner: C,
    reads: u64,
}

impl<C> CountingCode<C> {
    pub const fn new(inner: C) -> Self {
        Self { inner, reads: 0 }
    }

    /// Number of `word` fetches served since construction or the last
    /// [`Self::reset_counts`].
    pub const fn reads(&self) -> u64 {
        self.reads
    }

    /// Bytes those fetches transferred (`reads * 4`, the on-disk code-word width).
    pub const fn bytes_read(&self) -> u64 {
        self.reads * 4
    }

    /// Zero the fetch counter (e.g. between frames) without disturbing the inner
    /// source.
    pub fn reset_counts(&mut self) {
        self.reads = 0;
    }

    pub fn inner(&self) -> &C {
        &self.inner
    }

    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C: CodeSource> CodeSource for CountingCode<C> {
    fn word(&mut self, index: u32) -> Option<[u8; 4]> {
        self.reads = self.reads.saturating_add(1);
        self.inner.word(index)
    }

    fn word_run(&mut self, index: u32, dst: &mut [[u8; 4]]) -> usize {
        // Forward to the inner source's own batching and count every word it
        // served, so `reads` keeps meaning "code words fetched" regardless of
        // whether the VM fetched them singly or in a run.
        let served = self.inner.word_run(index, dst);
        self.reads = self.reads.saturating_add(served as u64);
        served
    }

    // Transparent wrapper: inherit the wrapped source's placement hint.
    const PLACE_HOT_LOOP_IN_SRAM: bool = C::PLACE_HOT_LOOP_IN_SRAM;

    // The fetch-metric surface forwards verbatim so the wrapper is observationally
    // identical to the source it wraps.
    fn reset_fetch_metrics(&mut self) {
        self.inner.reset_fetch_metrics();
    }
    fn fetch_refills(&self) -> u32 {
        self.inner.fetch_refills()
    }
    fn fetch_distinct_tiles(&self) -> u32 {
        self.inner.fetch_distinct_tiles()
    }
    fn tile_refills(&self) -> &[u16] {
        self.inner.tile_refills()
    }
    fn tile_transitions(&self) -> &[CodeTileTransition] {
        self.inner.tile_transitions()
    }
    fn cw_refill_us_total(&self) -> u32 {
        self.inner.cw_refill_us_total()
    }
    fn cw_refill_us_max(&self) -> u32 {
        self.inner.cw_refill_us_max()
    }
    fn cw_refill_bytes(&self) -> u32 {
        self.inner.cw_refill_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugMapError {
    BadProgram,
    BadRange,
    Truncated,
    BadMagic,
    UnsupportedVersion,
    BadHeaderSize,
    BadFile,
    BadEntry,
    NonMonotonicEntries,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation<'a> {
    pub pc: u32,
    pub file: &'a str,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugMap<'a> {
    bytes: &'a [u8],
    file_count: u16,
    entry_count: u32,
    files_start: usize,
    entries_start: usize,
}

impl<'a> DebugMap<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DebugMapError> {
        if bytes.len() < KBC_DEBUG_HEADER_SIZE {
            return Err(DebugMapError::Truncated);
        }
        if bytes[0..4] != KBC_DEBUG_MAGIC {
            return Err(DebugMapError::BadMagic);
        }
        if u16_at(bytes, 4) != KBC_DEBUG_VERSION {
            return Err(DebugMapError::UnsupportedVersion);
        }
        if usize::from(u16_at(bytes, 6)) != KBC_DEBUG_HEADER_SIZE {
            return Err(DebugMapError::BadHeaderSize);
        }
        if u32_at(bytes, 12) != 0 {
            return Err(DebugMapError::BadHeaderSize);
        }
        let file_count = u16_at(bytes, 8);
        let entry_count = u16_at(bytes, 10) as u32;
        let mut offset = KBC_DEBUG_HEADER_SIZE;
        for _ in 0..file_count {
            let len = read_u16_checked(bytes, offset).ok_or(DebugMapError::Truncated)? as usize;
            offset = offset.checked_add(2).ok_or(DebugMapError::BadFile)?;
            let end = offset.checked_add(len).ok_or(DebugMapError::BadFile)?;
            let file = bytes.get(offset..end).ok_or(DebugMapError::Truncated)?;
            core::str::from_utf8(file).map_err(|_| DebugMapError::BadFile)?;
            offset = end;
        }
        let entries_bytes = usize::try_from(entry_count)
            .ok()
            .and_then(|count| count.checked_mul(KBC_DEBUG_ENTRY_SIZE))
            .ok_or(DebugMapError::BadEntry)?;
        let end = offset
            .checked_add(entries_bytes)
            .ok_or(DebugMapError::BadEntry)?;
        if end != bytes.len() {
            return Err(DebugMapError::BadEntry);
        }
        let map = Self {
            bytes,
            file_count,
            entry_count,
            files_start: KBC_DEBUG_HEADER_SIZE,
            entries_start: offset,
        };
        map.validate_entries()?;
        Ok(map)
    }

    pub fn file_count(&self) -> u16 {
        self.file_count
    }

    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    pub fn lookup_pc(&self, pc: u32) -> Option<SourceLocation<'a>> {
        let mut best = None;
        for index in 0..self.entry_count {
            let entry = self.entry_at(index).ok()?;
            if entry.pc > pc {
                break;
            }
            best = Some(entry);
            if entry.pc == pc {
                break;
            }
        }
        best
    }

    fn validate_entries(&self) -> Result<(), DebugMapError> {
        let mut previous = None;
        for index in 0..self.entry_count {
            let entry = self.entry_at(index)?;
            if let Some(previous) = previous {
                if entry.pc <= previous {
                    return Err(DebugMapError::NonMonotonicEntries);
                }
            }
            previous = Some(entry.pc);
        }
        Ok(())
    }

    fn entry_at(&self, index: u32) -> Result<SourceLocation<'a>, DebugMapError> {
        if index >= self.entry_count {
            return Err(DebugMapError::BadEntry);
        }
        let offset = self.entries_start + index as usize * KBC_DEBUG_ENTRY_SIZE;
        let pc = u32_at(self.bytes, offset);
        let file_index = u16_at(self.bytes, offset + 4);
        if file_index >= self.file_count {
            return Err(DebugMapError::BadEntry);
        }
        if u16_at(self.bytes, offset + 6) != 0 {
            return Err(DebugMapError::BadEntry);
        }
        let line = u32_at(self.bytes, offset + 8);
        let col = u16_at(self.bytes, offset + 12) as u32;
        Ok(SourceLocation {
            pc,
            file: self.file(file_index)?,
            line,
            col,
        })
    }

    fn file(&self, index: u16) -> Result<&'a str, DebugMapError> {
        if index >= self.file_count {
            return Err(DebugMapError::BadFile);
        }
        let mut offset = self.files_start;
        for current in 0..self.file_count {
            let len =
                read_u16_checked(self.bytes, offset).ok_or(DebugMapError::Truncated)? as usize;
            offset += 2;
            let end = offset.checked_add(len).ok_or(DebugMapError::BadFile)?;
            let file = self
                .bytes
                .get(offset..end)
                .ok_or(DebugMapError::Truncated)?;
            if current == index {
                return core::str::from_utf8(file).map_err(|_| DebugMapError::BadFile);
            }
            offset = end;
        }
        Err(DebugMapError::BadFile)
    }
}

pub fn debug_map(bytes: &[u8]) -> Result<Option<DebugMap<'_>>, DebugMapError> {
    let header = parse_header(bytes).map_err(|_| DebugMapError::BadProgram)?;
    let bytecode_size =
        usize::try_from(header.bytecode_size).map_err(|_| DebugMapError::BadRange)?;
    if bytecode_size > bytes.len() || bytecode_size < KBC_HEADER_SIZE {
        return Err(DebugMapError::BadRange);
    }
    let range = optional_range(header.debug_offset, header.debug_size, bytecode_size)
        .map_err(|_| DebugMapError::BadRange)?;
    match range {
        Some((start, end)) => DebugMap::parse(&bytes[start..end]).map(Some),
        None => Ok(None),
    }
}

pub fn verify_kbc(bytes: &[u8], limits: RuntimeLimits) -> Result<VerifiedProgram, VerifyError> {
    let header = parse_header(bytes)?;
    let bytecode_size =
        usize::try_from(header.bytecode_size).map_err(|_| VerifyError::BadBytecodeSize)?;
    if bytecode_size > bytes.len() || bytecode_size < KBC_HEADER_SIZE {
        return Err(VerifyError::BadBytecodeSize);
    }
    let code = code_range(header, bytecode_size)?;
    let mut source = SliceCode::new(bytes, code.0);
    verify_program(header, bytecode_size, &mut source, limits)
}

/// Verify a program whose code segment is reachable only through a [`CodeSource`]
/// (KOTO-0127). `header_bytes` must hold at least the resident 64-byte KBC header,
/// and `stored_len` is the total `.kbc` length on the backing storage (used for
/// the same range checks `verify_kbc` makes against an in-memory slice). The whole
/// program never has to reside in SRAM: only the header plus per-word code reads.
pub fn verify_kbc_streaming<C: CodeSource>(
    header_bytes: &[u8],
    stored_len: usize,
    code: &mut C,
    limits: RuntimeLimits,
) -> Result<VerifiedProgram, VerifyError> {
    let header = parse_header(header_bytes)?;
    let bytecode_size =
        usize::try_from(header.bytecode_size).map_err(|_| VerifyError::BadBytecodeSize)?;
    if bytecode_size > stored_len || bytecode_size < KBC_HEADER_SIZE {
        return Err(VerifyError::BadBytecodeSize);
    }
    verify_program(header, bytecode_size, code, limits)
}

/// Shared verification body for both the slice and streaming entry points. Every
/// check needs only the parsed header, the declared `bytecode_size`, and per-word
/// code reads — never rodata, debug, or a contiguous program slice.
fn verify_program<C: CodeSource>(
    header: KbcHeader,
    bytecode_size: usize,
    code: &mut C,
    limits: RuntimeLimits,
) -> Result<VerifiedProgram, VerifyError> {
    if header.max_stack_slots > limits.max_stack_slots
        || header.max_call_depth > limits.max_call_depth
        || header.max_heap_bytes > limits.max_heap_bytes
    {
        return Err(VerifyError::ResourceLimitExceeded);
    }
    if header.host_abi_major != HOST_ABI_MAJOR || header.host_abi_minor > HOST_ABI_MINOR {
        return Err(VerifyError::UnsupportedHostAbi);
    }

    let code_r = code_range(header, bytecode_size)?;
    let rodata = optional_range(header.rodata_offset, header.rodata_size, bytecode_size)?;
    // rodata is copied into the bottom of the heap at load (KOTO-0139), so it must
    // fit the program's heap request.
    if header.rodata_size > header.max_heap_bytes {
        return Err(VerifyError::RodataExceedsHeap);
    }
    let debug = optional_range(header.debug_offset, header.debug_size, bytecode_size)?;
    reject_overlap(Some(code_r), rodata)?;
    reject_overlap(Some(code_r), debug)?;
    reject_overlap(rodata, debug)?;

    let code_words = header.code_size / 4;
    if header.entry_word >= code_words {
        return Err(VerifyError::BadEntry);
    }

    verify_instructions(code, code_words, header.max_stack_slots, limits)?;

    Ok(VerifiedProgram { header, code_words })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmError {
    BadProgram,
    BadInstruction,
    BadBranch,
    StackUnderflow,
    StackOverflow,
    CallDepthExceeded,
    MemoryOutOfBounds,
    DivisionByZero,
    HostCallDenied,
    HostCallFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmRunResult {
    Yielded,
    Exited(i32),
    FuelExhausted,
}

/// Construction failure for a portable bytecode session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionError {
    Verify(VerifyError),
    Vm(VmError),
}

/// Portable launch/step/exit/failure state shared by simulator and device
/// frontends. Platforms own bytecode storage, heap storage, and [`VmHost`]
/// adapters; this type owns verification output, VM state, frame accounting,
/// bounded fuel, and lifecycle transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BytecodeSession<const STACK: usize, const CALLS: usize> {
    program: VerifiedProgram,
    vm: BytecodeVm<STACK, CALLS>,
    frame_fuel: u32,
    result: VmRunResult,
    frame: u32,
    last_error: Option<VmError>,
    last_input: VmInputSnapshot,
}

impl<const STACK: usize, const CALLS: usize> BytecodeSession<STACK, CALLS> {
    pub fn new(bytes: &[u8], limits: RuntimeLimits, frame_fuel: u32) -> Result<Self, SessionError> {
        let program = verify_kbc(bytes, limits).map_err(SessionError::Verify)?;
        Self::from_program(program, frame_fuel)
    }

    /// Construct a session whose code is reachable only through a [`CodeSource`]
    /// (KOTO-0127): verification streams code words from `code` while the resident
    /// 64-byte header lives in `header_bytes` and `stored_len` is the `.kbc`
    /// length on storage. The same verified program then runs through
    /// [`Self::step_frame_with`] without ever residing wholly in SRAM.
    pub fn new_streaming<C: CodeSource>(
        header_bytes: &[u8],
        stored_len: usize,
        code: &mut C,
        limits: RuntimeLimits,
        frame_fuel: u32,
    ) -> Result<Self, SessionError> {
        let program = verify_kbc_streaming(header_bytes, stored_len, code, limits)
            .map_err(SessionError::Verify)?;
        Self::from_program(program, frame_fuel)
    }

    fn from_program(program: VerifiedProgram, frame_fuel: u32) -> Result<Self, SessionError> {
        let vm = BytecodeVm::new(&program).map_err(SessionError::Vm)?;
        Ok(Self {
            program,
            vm,
            frame_fuel,
            result: VmRunResult::Yielded,
            frame: 0,
            last_error: None,
            last_input: VmInputSnapshot::empty(),
        })
    }

    pub fn step_frame<H: VmHost>(
        &mut self,
        bytes: &[u8],
        host: &mut H,
        input: VmInputSnapshot,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        let mut code = SliceCode::new(bytes, self.program.code_range().0);
        self.step_frame_with(&mut code, host, input, heap)
    }

    /// Run one frame reading code through any [`CodeSource`] (KOTO-0127). The
    /// device passes a small SRAM window cached over PSRAM-resident code; the
    /// slice-backed [`Self::step_frame`] is a thin wrapper over this.
    pub fn step_frame_with<C: CodeSource, H: VmHost>(
        &mut self,
        code: &mut C,
        host: &mut H,
        input: VmInputSnapshot,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        self.frame = self.frame.saturating_add(1);
        self.last_input = input;
        // KOTO-0169 Stage 2: sources that flag themselves as the device hot
        // path run through the SRAM-placed twin of the frame loop. The branch
        // is on an associated const, so each monomorphization keeps exactly
        // one call and the other twin is never instantiated for it.
        #[cfg(feature = "ram_interpreter")]
        let result = if C::PLACE_HOT_LOOP_IN_SRAM {
            self.vm
                .execute_frame_with_ram(code, &self.program, host, input, self.frame_fuel, heap)
        } else {
            self.vm
                .execute_frame_with(code, &self.program, host, input, self.frame_fuel, heap)
        };
        #[cfg(not(feature = "ram_interpreter"))]
        let result =
            self.vm
                .execute_frame_with(code, &self.program, host, input, self.frame_fuel, heap);
        match result {
            Ok(result) => {
                self.result = result;
                Ok(result)
            }
            Err(error) => {
                host.close_all_files();
                self.last_error = Some(error);
                Err(error)
            }
        }
    }

    pub const fn program(&self) -> &VerifiedProgram {
        &self.program
    }

    pub const fn result(&self) -> VmRunResult {
        self.result
    }

    pub const fn frame(&self) -> u32 {
        self.frame
    }

    pub const fn last_error(&self) -> Option<VmError> {
        self.last_error
    }

    pub const fn last_input(&self) -> VmInputSnapshot {
        self.last_input
    }

    pub const fn has_exited(&self) -> bool {
        matches!(self.result, VmRunResult::Exited(_))
    }

    pub fn pc(&self) -> u32 {
        self.vm.pc()
    }

    pub fn last_frame_fuel(&self) -> u32 {
        self.vm.last_frame_fuel()
    }

    pub fn last_host_call(&self) -> Option<u8> {
        self.vm.last_host_call()
    }

    /// `HOST_CALL`s dispatched during the most recent frame. Held until the next
    /// [`Self::step_frame_with`] resets it, so the frame loop can read it after a
    /// step to attribute per-frame host-call cost (KOTO-0131 perf metrics).
    pub fn last_frame_host_calls(&self) -> u32 {
        self.vm.last_frame_host_calls()
    }

    pub fn budget(&self) -> VmBudget {
        self.vm.budget()
    }

    /// The per-frame instruction budget this session runs each frame against.
    pub const fn frame_fuel(&self) -> u32 {
        self.frame_fuel
    }

    /// Fuel left unspent by the most recent frame: `frame_fuel - last_frame_fuel`
    /// (KOTO-0153). Zero when a frame ran to fuel exhaustion. The consumed half is
    /// [`Self::last_frame_fuel`].
    pub fn last_frame_fuel_remaining(&self) -> u32 {
        self.frame_fuel.saturating_sub(self.vm.last_frame_fuel())
    }

    /// Cumulative session counters (instructions, host calls, frames) for later
    /// profiling (KOTO-0153).
    pub fn stats(&self) -> VmStats {
        self.vm.stats()
    }

    /// Per-opcode execution counts for this session (KOTO-0153), opt-in behind the
    /// `opcode_stats` feature. See [`BytecodeVm::opcode_counts`].
    #[cfg(feature = "opcode_stats")]
    pub fn opcode_counts(&self) -> &[u64; OPCODE_COUNT_SLOTS] {
        self.vm.opcode_counts()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmInputSnapshot {
    pub held_bits: u32,
    pub pressed_bits: u32,
    /// Typed Unicode scalar value for this frame, or `0` if no character was typed.
    pub text_codepoint: u32,
    /// Edit-intent flags for this frame; see [`text_intent`].
    pub intent_bits: u32,
}

impl VmInputSnapshot {
    pub const fn empty() -> Self {
        Self {
            held_bits: 0,
            pressed_bits: 0,
            text_codepoint: 0,
            intent_bits: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostErrorCode(pub i32);

impl HostErrorCode {
    pub const INVALID_CALL: Self = Self(1);
    pub const BAD_ARGUMENT: Self = Self(2);
    pub const PERMISSION_DENIED: Self = Self(3);
    pub const NOT_FOUND: Self = Self(4);
    pub const UNSUPPORTED: Self = Self(5);
    pub const WOULD_BLOCK: Self = Self(6);
    pub const IO_ERROR: Self = Self(7);
    pub const NO_MEMORY: Self = Self(8);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostCallOutcome {
    Ok0,
    Ok1(i32),
    Ok2(i32, i32),
    Err(HostErrorCode),
}

pub trait VmHost {
    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome;
    fn draw_text(&mut self, x: i32, y: i32, text: &str) -> HostCallOutcome;
    /// Draw text in a caller-chosen RGB565 colour. The default ignores the colour
    /// and falls back to [`Self::draw_text`], so hosts that do not implement
    /// per-call colours still render the text.
    fn draw_text_color(&mut self, x: i32, y: i32, text: &str, _rgb565: i32) -> HostCallOutcome {
        self.draw_text(x, y, text)
    }
    /// Blit a `w`x`h` block of little-endian RGB565 pixels (`pixels` is exactly
    /// `w * h * 2` bytes, row-major) at (`x`, `y`). The default host has no pixel
    /// surface and reports the call unsupported.
    fn draw_pixels_rgb565(
        &mut self,
        _x: i32,
        _y: i32,
        _w: i32,
        _h: i32,
        _pixels: &[u8],
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Write one cell of a host-retained tilemap `layer` (KOTO-0135). `tile_ref`
    /// is the app-heap byte offset of a 16x16 little-endian RGB565 tile (512
    /// bytes), or `< 0` to clear the cell. The host marks the cell so the next
    /// [`Self::game2d_present`] repaints only what changed. The default host has
    /// no tile renderer and reports unsupported.
    fn game2d_set_tile(
        &mut self,
        _layer: i32,
        _x: i32,
        _y: i32,
        _tile_ref: i32,
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Clear every cell of a host-retained tilemap `layer` (KOTO-0135).
    fn game2d_clear_layer(&mut self, _layer: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Composite and present the retained tilemap for this frame (KOTO-0135).
    /// `heap` is the app heap so a host that composites into a draw model can
    /// re-read tile art by offset (mirroring `draw_pixels_rgb565`); hosts that
    /// retain pixels directly may ignore it. The default host reports unsupported.
    fn game2d_present(&mut self, _heap: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Begin capturing draw calls into the retained Game2D static/background layer
    /// (KOTO-0136): clear the layer, then route subsequent `draw_rect` /
    /// `draw_text` / `draw_text_color` / `draw_pixels_rgb565` into it until
    /// [`Self::game2d_static_end`]. The presenter composites the layer beneath the
    /// board tilemap and the per-frame immediate commands. The default host has no
    /// static layer and reports unsupported.
    fn game2d_static_begin(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// End static-layer capture (KOTO-0136): route draw calls back to the
    /// per-frame immediate list. The default host reports unsupported.
    fn game2d_static_end(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Register a retained sprite *stamp* (KOTO-0140): a reusable, position-
    /// independent cell pattern. `count` cells live at app-heap byte offset
    /// `cells_off`; `format 0` packs each cell as a `(dcol,drow)` nibble (the
    /// KOTO-0138 layout, nibble `p` = `drow*4 + dcol`). The host stores only the
    /// descriptor — the cell bytes stay in the app heap, re-read at present time.
    /// The default host has no sprite layer and reports unsupported.
    fn game2d_stamp_define(
        &mut self,
        _stamp_id: i32,
        _cells_off: i32,
        _count: i32,
        _format: i32,
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Create or update retained sprite `inst_id` (KOTO-0140): an on-screen
    /// instance of stamp `stamp_id` whose cells are drawn at `(x + dcol*16,
    /// y + drow*16)`, each blitting the 16x16 RGB565 tile at app-heap byte offset
    /// `tile_ref`. The host retains the sprite across frames and diffs it by stable
    /// `inst_id`. The default host reports unsupported.
    fn game2d_sprite_set(
        &mut self,
        _inst_id: i32,
        _stamp_id: i32,
        _x: i32,
        _y: i32,
        _tile_ref: i32,
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Hide retained sprite `inst_id` (KOTO-0140): its footprint becomes a dirty
    /// erase on the next present. The default host reports unsupported.
    fn game2d_sprite_hide(&mut self, _inst_id: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Hide every retained sprite (KOTO-0140). The default host reports unsupported.
    fn game2d_sprite_clear_all(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Create or update retained text item `id` (KOTO-0141): the UTF-8 `text`
    /// pinned at pixel `(x, y)` in colour `rgb565`. The host retains it across
    /// frames and diffs it by stable `id`, so a value that does not change costs
    /// nothing and a change repaints only its own footprint. The default host has
    /// no text layer and reports unsupported.
    fn game2d_text_set(
        &mut self,
        _id: i32,
        _x: i32,
        _y: i32,
        _text: &str,
        _rgb565: i32,
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Hide retained text item `id` (KOTO-0141): its footprint becomes a dirty
    /// erase on the next present. The default host reports unsupported.
    fn game2d_text_hide(&mut self, _id: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Hide every retained text item (KOTO-0141). The default host reports
    /// unsupported.
    fn game2d_text_clear_all(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Submit `channels`-interleaved i16 PCM frames to the host audio output.
    /// `samples` is exactly `frames * channels * 2` little-endian bytes (the host
    /// decodes them, like `draw_pixels_rgb565` decodes RGB565), so the no-alloc VM
    /// passes the app-heap slice through unchanged. Nonblocking: returns
    /// `Ok1(frames_accepted)`, which may be fewer than offered. The default host has
    /// no audio output and reports unsupported.
    fn audio_submit_i16(
        &mut self,
        _frames: i32,
        _channels: i32,
        _samples: &[u8],
    ) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Trigger a one-shot host-owned sound effect by id (see [`audio_id`]). The
    /// default host has no audio output and reports unsupported.
    fn play_sfx(&mut self, _id: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Start a looping host-owned background-music track by id (see [`audio_id`]).
    fn play_bgm(&mut self, _id: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Start looping KotoMML loaded from a read-only package asset path.
    fn play_bgm_asset(&mut self, _path: &str) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Play one-shot KotoMML loaded from a read-only package asset path.
    fn play_sfx_asset(&mut self, _path: &str) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Stop the looping host-owned background-music track.
    fn stop_bgm(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome;
    fn file_open(&mut self, path: &str, mode: i32) -> HostCallOutcome;
    fn file_read(&mut self, handle: i32, dst: &mut [u8]) -> HostCallOutcome;
    fn file_write(&mut self, handle: i32, src: &[u8]) -> HostCallOutcome;
    fn file_close(&mut self, handle: i32) -> HostCallOutcome;
    fn close_all_files(&mut self) {}

    /// Copy a read-only package asset (declared by the launching manifest) into the
    /// `dst` heap slice, returning the number of bytes written. Unlike `file_*`,
    /// which target the per-app save sandbox, this reads the immutable package.
    /// The default host has no package, so it reports the asset as unavailable.
    fn asset_load(&mut self, _path: &str, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Frame-stable typed-character input. Returns `(codepoint, intent_bits)`.
    /// The default reflects the current frame snapshot unchanged.
    fn text_input(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.text_codepoint as i32, input.intent_bits as i32)
    }

    /// Feed one key into the host IME+editor. `kind` is an [`ime_key`] value and
    /// `codepoint` is the typed character when `kind == ime_key::CHARACTER`.
    fn ime_feed_key(&mut self, _kind: i32, _codepoint: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Run dictionary conversion on the current IME reading.
    fn ime_convert(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Serialize the IME composition line into `dst`. Returns `(bytes_written)`.
    fn ime_query_line(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Move the editor cursor in the [`edit_dir`] direction.
    fn edit_move(&mut self, _dir: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Delete around the cursor; `kind` is an [`edit_delete`] value. Returns
    /// `(removed)` where `removed` is `1` when text was deleted.
    fn edit_delete(&mut self, _kind: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Replace the document with UTF-8 text from `src`.
    fn edit_load(&mut self, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Read document text into `dst`. Returns `(bytes_written, cursor_byte_offset)`.
    fn edit_query_text(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Write the IME composition as a plain UTF-8 display string into `dst`.
    /// Returns `(bytes_written)`.
    fn ime_display(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Read one visible editor line into `dst`. Returns `(bytes_written)`.
    fn edit_visible_line(&mut self, _row: i32, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return `(cursor_col, cursor_row)` in the visible editor viewport.
    fn edit_cursor_view(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return the first visible document row.
    fn edit_scroll_row(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return `(cell_width, cell_height)` for the editor viewport.
    fn edit_view_metrics(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Write a compact line/column status string into `dst`.
    fn edit_cursor_status(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return the total logical line count in the editor document.
    fn edit_total_lines(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return `(1)` when soft wrapping is on, else `(0)`.
    fn edit_wrap(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Return `(hscroll_columns, cursor_line_columns)` for a no-wrap scrollbar.
    fn edit_hscroll_view(&mut self) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Write the `index`-th save-data sandbox filename (sorted, deterministic)
    /// into `dst`. Returns `(entry_count, name_len)`; `name_len` is `0` when
    /// `index` is out of range. An absent save directory is an empty listing,
    /// not an error.
    fn dir_list(&mut self, _index: i32, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Reserve the bottom `rows` rows of the editor viewport so the cursor stays
    /// scrolled above them (negative values clamp to `0`).
    fn edit_reserve_rows(&mut self, _rows: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    fn edit_configure(&mut self, _cols: i32, _rows: i32) -> HostCallOutcome {
        HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
    }

    /// Observe-only timing seam around one `HOST_CALL` dispatch (KOTO-0169
    /// Stage 0b): the interpreter calls `hostcall_dispatch_begin` immediately
    /// before decoding a `HOST_CALL` (so the argument pops, the host method,
    /// and the outcome push are all inside the window) and
    /// `hostcall_dispatch_end` right after it returns, on success and error
    /// alike. The internally handled `yield_frame`/`exit` calls are bracketed
    /// too; a host that accumulates wall time sees their (trivial) marshalling
    /// cost once per frame. Defaults are no-ops so existing hosts (and the
    /// simulator) are unaffected; the device host uses the pair to accumulate
    /// a per-frame `host_us` counter.
    fn hostcall_dispatch_begin(&mut self) {}
    fn hostcall_dispatch_end(&mut self) {}
}

/// Session high-water marks for a running VM, accumulated across every frame and
/// never reset (budget diagnostics, KOTO-0101). Each peak is the largest value the
/// VM has reached so far; the host pairs them with the fixed VM/program capacities
/// to validate SRAM, stack, local-slot, and frame-fuel budgets before bring-up.
///
/// `heap_bytes_peak` is the high-water of the highest heap byte the VM's own
/// `LOAD*`/`STORE*` instructions address (see [`BytecodeVm::pop_address`]). Host
/// calls move data through caller-supplied heap buffers, but the app writes those
/// buffers (then the host reads) or reads them back (after the host writes), so a
/// directly-addressed high-water captures the heap the program actually uses.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VmBudget {
    /// Deepest the operand stack has been (slots).
    pub stack_slots_peak: u16,
    /// Deepest the call stack has been (return frames).
    pub call_depth_peak: u16,
    /// Highest local slot index touched, plus one (slots in use).
    pub local_slots_peak: u16,
    /// Highest heap byte addressed by a `LOAD*`/`STORE*`, i.e. bytes in use.
    pub heap_bytes_peak: u32,
    /// Most instructions stepped in a single frame.
    pub frame_fuel_peak: u32,
    /// Most `HOST_CALL`s dispatched in a single frame.
    pub host_calls_per_frame_peak: u32,
}

/// Cumulative, session-wide execution counters for later profiling (KOTO-0153).
///
/// Distinct from [`VmBudget`], which records per-session *high-water peaks*: this
/// accumulates running *totals* across every executed frame so a profiler can
/// derive averages and rates (e.g. instructions per frame, host-call density).
/// All counters are saturating, observation-only, and never gate execution, so
/// they add no VM semantics — they only describe what the interpreter already did.
/// Per-opcode execution counts are additionally available behind the `opcode_stats`
/// feature (see [`BytecodeVm::opcode_counts`]); they are kept out of this `Copy`
/// struct so the common path stays a few cheap words rather than a 2 KiB table.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VmStats {
    /// Total instructions stepped across the whole session (the cumulative analogue
    /// of the per-frame [`BytecodeVm::last_frame_fuel`]).
    pub instructions: u64,
    /// Total `HOST_CALL`s dispatched across the whole session.
    pub host_calls: u64,
    /// Frames actually executed: each [`BytecodeVm::execute_frame_with`] that entered
    /// the step loop (one with non-zero fuel on a VM that has not yet exited),
    /// including a frame that trapped or exhausted its fuel.
    pub frames: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BytecodeVm<const STACK: usize, const CALLS: usize> {
    pc: u32,
    stack: [i32; STACK],
    stack_len: usize,
    calls: [u32; CALLS],
    call_len: usize,
    locals: [i32; VM_LOCAL_SLOTS],
    exited: Option<i32>,
    /// Inspector state: the most recent `HOST_CALL` id dispatched, kept across
    /// frames so a runtime inspector can report the last host interaction.
    last_host_call: Option<u8>,
    /// Inspector state: fuel consumed (instructions stepped) during the most
    /// recent [`Self::execute_frame`].
    last_frame_fuel: u32,
    /// Budget diagnostics: session high-water marks (KOTO-0101).
    budget: VmBudget,
    /// `HOST_CALL`s dispatched in the current frame, feeding the per-frame peak.
    host_calls_this_frame: u32,
    /// Cumulative session counters for later profiling (KOTO-0153).
    stats: VmStats,
    /// Straight-line fetch line (KOTO-0169 Stage 1, the H1 fast path): a small
    /// decode-side copy of consecutive code words filled by
    /// [`CodeSource::word_run`]. `line[i]` is code word `line_base + i` for
    /// `i < line_len`. Valid because the code segment is immutable for the
    /// life of the VM (no VM op writes code); invalidated only at frame entry,
    /// where the caller may hand in a different [`CodeSource`]. A branch or
    /// call that leaves the line simply misses the containment check and
    /// refills it — no explicit invalidation is needed.
    line: [[u8; 4]; VM_CODE_LINE_WORDS],
    line_base: u32,
    line_len: u32,
    /// Per-opcode execution counts (KOTO-0153), opt-in behind `opcode_stats`. A
    /// 2 KiB table indexed by raw opcode byte; off by default so device builds do
    /// not carry it. Counting is the only effect — it never changes execution.
    #[cfg(feature = "opcode_stats")]
    opcode_counts: [u64; OPCODE_COUNT_SLOTS],
}

/// Capacity of the [`BytecodeVm`] straight-line fetch line, in code words
/// (KOTO-0169 Stage 1). 16 words (64 B) amortizes the per-instruction
/// fetch-path cost — the cross-crate `CodeSource` call plus the window's
/// bounds/tile checks — over up to 16 straight-line instructions, while the
/// copy itself stays a trivial SRAM-to-SRAM move. Sized as a decode buffer,
/// not a cache: one line, refilled on any miss.
const VM_CODE_LINE_WORDS: usize = 16;

/// Number of slots in the [`BytecodeVm::opcode_counts`] table — one per possible
/// opcode byte, so any decoded opcode (including a rejected one) indexes in range.
#[cfg(feature = "opcode_stats")]
pub const OPCODE_COUNT_SLOTS: usize = 256;

impl<const STACK: usize, const CALLS: usize> BytecodeVm<STACK, CALLS> {
    /// Construct a VM for `program`. The app heap is **not** owned by the VM: it is
    /// supplied per frame to [`Self::execute_frame`] (sized to the program's
    /// `max_heap_bytes`), so a program can use only the heap its KBC header
    /// requested — see the per-app heap profile (KOTO-0096).
    pub fn new(program: &VerifiedProgram) -> Result<Self, VmError> {
        if STACK == 0
            || STACK < usize::from(program.header.max_stack_slots)
            || CALLS < usize::from(program.header.max_call_depth)
        {
            return Err(VmError::BadProgram);
        }
        Ok(Self {
            pc: program.header.entry_word,
            stack: [0; STACK],
            stack_len: 0,
            calls: [0; CALLS],
            call_len: 0,
            locals: [0; VM_LOCAL_SLOTS],
            exited: None,
            last_host_call: None,
            last_frame_fuel: 0,
            budget: VmBudget::default(),
            host_calls_this_frame: 0,
            stats: VmStats::default(),
            line: [[0; 4]; VM_CODE_LINE_WORDS],
            line_base: 0,
            line_len: 0,
            #[cfg(feature = "opcode_stats")]
            opcode_counts: [0; OPCODE_COUNT_SLOTS],
        })
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }

    /// Session high-water marks accumulated across every executed frame (budget
    /// diagnostics, KOTO-0101). All zero before the first frame.
    pub fn budget(&self) -> VmBudget {
        self.budget
    }

    /// The most recent `HOST_CALL` id this VM dispatched, or `None` before any
    /// host call. Retained across frames for the runtime inspector.
    pub fn last_host_call(&self) -> Option<u8> {
        self.last_host_call
    }

    /// Fuel consumed (instructions stepped) during the most recent
    /// [`Self::execute_frame`]. Zero before the first frame.
    pub fn last_frame_fuel(&self) -> u32 {
        self.last_frame_fuel
    }

    /// `HOST_CALL`s dispatched during the most recent [`Self::execute_frame_with`].
    /// `host_calls_this_frame` is reset at each frame's start and accumulates as
    /// the frame runs, so after a frame returns it holds that frame's total until
    /// the next frame begins (KOTO-0131 perf metrics).
    pub fn last_frame_host_calls(&self) -> u32 {
        self.host_calls_this_frame
    }

    /// Cumulative session counters (instructions, host calls, frames) for later
    /// profiling (KOTO-0153). All zero before the first frame; accumulate across
    /// every executed frame, unlike the per-frame [`Self::last_frame_fuel`] and the
    /// high-water [`Self::budget`].
    pub fn stats(&self) -> VmStats {
        self.stats
    }

    /// Per-opcode execution counts indexed by raw opcode byte (KOTO-0153), opt-in
    /// behind the `opcode_stats` feature. `opcode_counts()[op]` is how many times
    /// opcode `op` was decoded this session (saturating). All zero before the first
    /// frame.
    #[cfg(feature = "opcode_stats")]
    pub fn opcode_counts(&self) -> &[u64; OPCODE_COUNT_SLOTS] {
        &self.opcode_counts
    }

    pub fn stack_len(&self) -> usize {
        self.stack_len
    }

    pub fn push_value(&mut self, value: i32) -> Result<(), VmError> {
        self.push(value)
    }

    pub fn pop_value(&mut self) -> Result<i32, VmError> {
        self.pop()
    }

    /// Run one frame against a full in-memory `.kbc` slice. Thin wrapper over
    /// [`Self::execute_frame_with`] for the resident slice path (simulator, tools,
    /// tests).
    pub fn execute_frame<H: VmHost>(
        &mut self,
        bytes: &[u8],
        program: &VerifiedProgram,
        host: &mut H,
        input: VmInputSnapshot,
        fuel: u32,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        let mut code = SliceCode::new(bytes, program.code_range().0);
        self.execute_frame_with(&mut code, program, host, input, fuel, heap)
    }

    /// Run one frame reading code through any [`CodeSource`] (KOTO-0127). The
    /// device feeds a small SRAM window cached over PSRAM-resident code so the
    /// program never has to fit in SRAM.
    pub fn execute_frame_with<C: CodeSource, H: VmHost>(
        &mut self,
        code: &mut C,
        program: &VerifiedProgram,
        host: &mut H,
        input: VmInputSnapshot,
        fuel: u32,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        self.execute_frame_core(code, program, host, input, fuel, heap)
    }

    /// SRAM-placed twin of [`Self::execute_frame_with`] (KOTO-0169 Stage 2,
    /// H2-b): byte-for-byte the same frame loop, but tagged into
    /// `.data.koto_vm_interp` so a cortex-m-rt startup copies this copy to RAM
    /// and per-op dispatch stops paying flash-XIP fetch misses. Kept as a twin
    /// rather than an attribute on `execute_frame_with` itself so only the
    /// instantiation the device routes here (the PSRAM-window hot path, via
    /// [`CodeSource::PLACE_HOT_LOOP_IN_SRAM`]) pays RAM — the first, default-on
    /// shape tagged every instantiation and its ~5.4 KiB `.data` cost collided
    /// with the firmware's main-stack budget (boot failure; see the Stage-2
    /// record in KOTO-0169). `inline(never)` pins the section: inlining into a
    /// flash-resident caller would silently discard it.
    #[cfg(feature = "ram_interpreter")]
    #[link_section = ".data.koto_vm_interp"]
    #[inline(never)]
    pub fn execute_frame_with_ram<C: CodeSource, H: VmHost>(
        &mut self,
        code: &mut C,
        program: &VerifiedProgram,
        host: &mut H,
        input: VmInputSnapshot,
        fuel: u32,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        self.execute_frame_core(code, program, host, input, fuel, heap)
    }

    /// Shared body of [`Self::execute_frame_with`] and its SRAM twin.
    /// `inline(always)`: each wrapper must carry a full copy so the twin's
    /// link section governs where the loop actually executes from.
    #[inline(always)]
    fn execute_frame_core<C: CodeSource, H: VmHost>(
        &mut self,
        code: &mut C,
        program: &VerifiedProgram,
        host: &mut H,
        input: VmInputSnapshot,
        fuel: u32,
        heap: &mut [u8],
    ) -> Result<VmRunResult, VmError> {
        self.last_frame_fuel = 0;
        self.host_calls_this_frame = 0;
        // Drop the straight-line fetch line at the frame boundary (KOTO-0169
        // Stage 1): the caller may hand this frame a different `CodeSource`
        // (tests do), and a stale line from another source must never serve a
        // word. Within a frame the source and the code segment are fixed, so
        // this is the only invalidation point.
        self.line_len = 0;
        if (heap.len() as u64) < u64::from(program.header.max_heap_bytes) {
            return Err(VmError::BadProgram);
        }
        if let Some(code) = self.exited {
            return Ok(VmRunResult::Exited(code));
        }
        if fuel == 0 {
            return Ok(VmRunResult::FuelExhausted);
        }
        self.stats.frames = self.stats.frames.saturating_add(1);

        // H2-a bookkeeping hoist (KOTO-0169 Stage 3): the loop counts executed
        // instructions in one induction variable and folds it into the three
        // counters — `last_frame_fuel`, the cumulative `stats.instructions`,
        // and the `frame_fuel_peak` high-water — exactly once per frame, on
        // every exit path (yield / exit / trap / fuel exhaustion). Per-op that
        // removes a `u64` saturating add and a peak max from the hot loop; the
        // end-of-frame values are identical because the count only grows and
        // the trapping instruction was already counted before its `step`.
        let mut executed: u32 = 0;
        let result = loop {
            if executed == fuel {
                break Ok(VmRunResult::FuelExhausted);
            }
            executed += 1;
            match self.step(code, program, host, input, heap) {
                Ok(StepOutcome::Continue) => {}
                Ok(StepOutcome::Yielded) => break Ok(VmRunResult::Yielded),
                Ok(StepOutcome::Exited(code)) => {
                    host.close_all_files();
                    self.exited = Some(code);
                    break Ok(VmRunResult::Exited(code));
                }
                Err(error) => break Err(error),
            }
        };
        self.last_frame_fuel = executed;
        self.stats.instructions = self.stats.instructions.saturating_add(u64::from(executed));
        self.budget.frame_fuel_peak = self.budget.frame_fuel_peak.max(executed);
        result
    }

    // ram_interpreter: force-inlined into the frame-loop twins so the placement
    // of each twin (SRAM vs flash) deterministically governs where dispatch
    // executes — an outlined shared copy would silently pin one path to flash.
    // Same for the small per-op helpers below. Host builds never enable the
    // feature, so their codegen is untouched.
    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn step<C: CodeSource, H: VmHost>(
        &mut self,
        code: &mut C,
        program: &VerifiedProgram,
        host: &mut H,
        input: VmInputSnapshot,
        heap: &mut [u8],
    ) -> Result<StepOutcome, VmError> {
        if self.pc >= program.code_words {
            return Err(VmError::BadBranch);
        }

        let current_pc = self.pc;
        // H1 fetch fast path (KOTO-0169 Stage 1): serve straight-line words
        // from the local line and only call into the `CodeSource` on a miss
        // (line exhausted, or a branch/call left it). The wrapping subtraction
        // makes `current_pc < line_base` miss too (it wraps to a huge value).
        let line_offset = current_pc.wrapping_sub(self.line_base);
        let word = if line_offset < self.line_len {
            self.line[line_offset as usize]
        } else {
            // Clamp defends the local index against a source that (wrongly)
            // reports serving more words than the buffer holds.
            let served = code
                .word_run(current_pc, &mut self.line)
                .min(VM_CODE_LINE_WORDS);
            if served == 0 {
                return Err(VmError::BadProgram);
            }
            self.line_base = current_pc;
            self.line_len = served as u32;
            self.line[0]
        };
        self.pc = self.pc.checked_add(1).ok_or(VmError::BadBranch)?;

        let opcode = word[3];
        let operand = word[2];
        let immediate = u16::from_le_bytes([word[0], word[1]]);

        #[cfg(feature = "opcode_stats")]
        {
            let slot = &mut self.opcode_counts[usize::from(opcode)];
            *slot = slot.saturating_add(1);
        }

        match opcode {
            opcode::NOP => Ok(StepOutcome::Continue),
            opcode::HALT => Ok(StepOutcome::Exited(0)),
            opcode::BR => {
                self.branch(immediate, program)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BR_IF_ZERO => {
                let value = self.pop()?;
                if value == 0 {
                    self.branch(immediate, program)?;
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CALL => {
                if self.call_len >= CALLS {
                    return Err(VmError::CallDepthExceeded);
                }
                check_target(immediate, program.code_words).map_err(|_| VmError::BadBranch)?;
                self.calls[self.call_len] = self.pc;
                self.call_len += 1;
                self.budget.call_depth_peak = self.budget.call_depth_peak.max(self.call_len as u16);
                self.pc = u32::from(immediate);
                Ok(StepOutcome::Continue)
            }
            opcode::RET => {
                if self.call_len == 0 {
                    Ok(StepOutcome::Exited(0))
                } else {
                    self.call_len -= 1;
                    self.pc = self.calls[self.call_len];
                    Ok(StepOutcome::Continue)
                }
            }
            opcode::PUSH_I16 => {
                self.push(i32::from(i16::from_le_bytes(immediate.to_le_bytes())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::DUP => {
                let value = self.peek()?;
                self.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::DROP => {
                self.pop()?;
                Ok(StepOutcome::Continue)
            }
            opcode::SWAP => {
                if self.stack_len < 2 {
                    return Err(VmError::StackUnderflow);
                }
                self.stack.swap(self.stack_len - 1, self.stack_len - 2);
                Ok(StepOutcome::Continue)
            }
            opcode::LOAD_LOCAL => {
                let index = usize::from(operand);
                if index >= VM_LOCAL_SLOTS || immediate != 0 {
                    return Err(VmError::BadInstruction);
                }
                self.budget.local_slots_peak = self.budget.local_slots_peak.max(index as u16 + 1);
                self.push(self.locals[index])?;
                Ok(StepOutcome::Continue)
            }
            opcode::STORE_LOCAL => {
                let index = usize::from(operand);
                if index >= VM_LOCAL_SLOTS || immediate != 0 {
                    return Err(VmError::BadInstruction);
                }
                self.budget.local_slots_peak = self.budget.local_slots_peak.max(index as u16 + 1);
                self.locals[index] = self.pop()?;
                Ok(StepOutcome::Continue)
            }
            opcode::ADD_I32
            | opcode::SUB_I32
            | opcode::MUL_I32
            | opcode::DIV_I32
            | opcode::AND_I32
            | opcode::OR_I32
            | opcode::XOR_I32
            | opcode::SHL_I32
            | opcode::SHR_I32 => {
                self.exec_binary(opcode)?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOAD8 => {
                let address = self.pop_address(1, heap.len())?;
                self.push(i32::from(heap[address]))?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOAD16 => {
                let address = self.pop_address(2, heap.len())?;
                self.push(i32::from(u16::from_le_bytes([
                    heap[address],
                    heap[address + 1],
                ])))?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOAD32 => {
                let address = self.pop_address(4, heap.len())?;
                self.push(i32::from_le_bytes([
                    heap[address],
                    heap[address + 1],
                    heap[address + 2],
                    heap[address + 3],
                ]))?;
                Ok(StepOutcome::Continue)
            }
            opcode::STORE8 => {
                let value = self.pop()?;
                let address = self.pop_address(1, heap.len())?;
                heap[address] = value as u8;
                Ok(StepOutcome::Continue)
            }
            opcode::STORE16 => {
                let value = self.pop()?;
                let address = self.pop_address(2, heap.len())?;
                heap[address..address + 2].copy_from_slice(&(value as u16).to_le_bytes());
                Ok(StepOutcome::Continue)
            }
            opcode::STORE32 => {
                let value = self.pop()?;
                let address = self.pop_address(4, heap.len())?;
                heap[address..address + 4].copy_from_slice(&value.to_le_bytes());
                Ok(StepOutcome::Continue)
            }
            opcode::HOST_CALL => {
                // KOTO-0169 Stage 0b: bracket the whole dispatch — pops, host
                // method, outcome push — so a timing host attributes the full
                // marshalling cost. `end` runs on the error path too.
                host.hostcall_dispatch_begin();
                let outcome = self.exec_host_call(operand, host, input, heap);
                host.hostcall_dispatch_end();
                outcome
            }
            _ => {
                self.pc = current_pc;
                Err(VmError::BadInstruction)
            }
        }
    }

    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn branch(&mut self, target: u16, program: &VerifiedProgram) -> Result<(), VmError> {
        check_target(target, program.code_words).map_err(|_| VmError::BadBranch)?;
        self.pc = u32::from(target);
        Ok(())
    }

    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn exec_binary(&mut self, opcode: u8) -> Result<(), VmError> {
        let rhs = self.pop()?;
        let lhs = self.pop()?;
        let value = match opcode {
            opcode::ADD_I32 => lhs.wrapping_add(rhs),
            opcode::SUB_I32 => lhs.wrapping_sub(rhs),
            opcode::MUL_I32 => lhs.wrapping_mul(rhs),
            opcode::DIV_I32 => {
                if rhs == 0 {
                    return Err(VmError::DivisionByZero);
                }
                lhs.wrapping_div(rhs)
            }
            opcode::AND_I32 => lhs & rhs,
            opcode::OR_I32 => lhs | rhs,
            opcode::XOR_I32 => lhs ^ rhs,
            opcode::SHL_I32 => lhs.wrapping_shl((rhs & 31) as u32),
            opcode::SHR_I32 => ((lhs as u32).wrapping_shr((rhs & 31) as u32)) as i32,
            _ => return Err(VmError::BadInstruction),
        };
        self.push(value)
    }

    // ram_interpreter: deliberately kept in flash — it is a single-call-site
    // function LLVM would otherwise inline wholesale into the RAM copy of
    // `step`, and at ~70 dispatches/frame (vs ~8k ops) its XIP residency is
    // noise while its size is the largest single body in the interpreter.
    #[cfg_attr(feature = "ram_interpreter", inline(never))]
    fn exec_host_call<H: VmHost>(
        &mut self,
        id: u8,
        host: &mut H,
        input: VmInputSnapshot,
        heap: &mut [u8],
    ) -> Result<StepOutcome, VmError> {
        self.last_host_call = Some(id);
        self.host_calls_this_frame += 1;
        self.stats.host_calls = self.stats.host_calls.saturating_add(1);
        self.budget.host_calls_per_frame_peak = self
            .budget
            .host_calls_per_frame_peak
            .max(self.host_calls_this_frame);
        match id {
            host_call::EXIT => {
                let code = self.pop()?;
                Ok(StepOutcome::Exited(code))
            }
            host_call::YIELD_FRAME => {
                self.push(0)?;
                Ok(StepOutcome::Yielded)
            }
            host_call::DRAW_RECT => {
                let rgb565 = self.pop()?;
                let h = self.pop()?;
                let w = self.pop()?;
                let y = self.pop()?;
                let x = self.pop()?;
                self.push_host_outcome(host.draw_rect(x, y, w, h, rgb565), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::DRAW_PIXELS_RGB565 => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let h = self.pop()?;
                let w = self.pop()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let Some(pixels) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                self.push_host_outcome(host.draw_pixels_rgb565(x, y, w, h, pixels), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_SET_TILE => {
                let tile_ref = self.pop()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let layer = self.pop()?;
                self.push_host_outcome(host.game2d_set_tile(layer, x, y, tile_ref), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_CLEAR_LAYER => {
                let layer = self.pop()?;
                self.push_host_outcome(host.game2d_clear_layer(layer), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_PRESENT => {
                self.push_host_outcome(host.game2d_present(heap), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_STATIC_BEGIN => {
                self.push_host_outcome(host.game2d_static_begin(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_STATIC_END => {
                self.push_host_outcome(host.game2d_static_end(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_STAMP_DEFINE => {
                let format = self.pop()?;
                let count = self.pop()?;
                let cells_off = self.pop()?;
                let stamp_id = self.pop()?;
                self.push_host_outcome(
                    host.game2d_stamp_define(stamp_id, cells_off, count, format),
                    0,
                )?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_SPRITE_SET => {
                let tile_ref = self.pop()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let stamp_id = self.pop()?;
                let inst_id = self.pop()?;
                self.push_host_outcome(
                    host.game2d_sprite_set(inst_id, stamp_id, x, y, tile_ref),
                    0,
                )?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_SPRITE_HIDE => {
                let inst_id = self.pop()?;
                self.push_host_outcome(host.game2d_sprite_hide(inst_id), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_SPRITE_CLEAR_ALL => {
                self.push_host_outcome(host.game2d_sprite_clear_all(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_TEXT_SET => {
                let rgb565 = self.pop()?;
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let id = self.pop()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let text = match core::str::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            0,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.game2d_text_set(id, x, y, text, rgb565), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_TEXT_HIDE => {
                let id = self.pop()?;
                self.push_host_outcome(host.game2d_text_hide(id), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::GAME2D_TEXT_CLEAR_ALL => {
                self.push_host_outcome(host.game2d_text_clear_all(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::AUDIO_SUBMIT_I16 => {
                let channels = self.pop()?;
                let frames = self.pop()?;
                let ptr = self.pop_usize()?;
                // Frame/channel counts must be sane before sizing the heap slice; a
                // bad shape is a rejected argument, not a trap.
                let Some(len) = audio_pcm_len(frames, channels) else {
                    self.push_host_outcome(HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT), 1)?;
                    return Ok(StepOutcome::Continue);
                };
                let Some(samples) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                self.push_host_outcome(host.audio_submit_i16(frames, channels, samples), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::PLAY_SFX => {
                let id = self.pop()?;
                self.push_host_outcome(host.play_sfx(id), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::PLAY_BGM => {
                let id = self.pop()?;
                self.push_host_outcome(host.play_bgm(id), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::PLAY_BGM_ASSET => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let path = match core::str::from_utf8(bytes) {
                    Ok(path) => path,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            0,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.play_bgm_asset(path), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::PLAY_SFX_ASSET => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let path = match core::str::from_utf8(bytes) {
                    Ok(path) => path,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            0,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.play_sfx_asset(path), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::STOP_BGM => {
                self.push_host_outcome(host.stop_bgm(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::DRAW_TEXT => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let text = match core::str::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            0,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.draw_text(x, y, text), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::DRAW_TEXT_COLOR => {
                let rgb565 = self.pop()?;
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let y = self.pop()?;
                let x = self.pop()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let text = match core::str::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            0,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.draw_text_color(x, y, text, rgb565), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::INPUT_SNAPSHOT => {
                self.push_host_outcome(host.input_snapshot(input), 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::FILE_OPEN => {
                let mode = self.pop()?;
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(bytes) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let path = match core::str::from_utf8(bytes) {
                    Ok(path) => path,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            1,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                self.push_host_outcome(host.file_open(path, mode), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::FILE_READ => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let handle = self.pop()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.file_read(handle, dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::FILE_WRITE => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let handle = self.pop()?;
                let Some(src) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.file_write(handle, src);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::FILE_CLOSE => {
                let handle = self.pop()?;
                self.push_host_outcome(host.file_close(handle), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::ASSET_LOAD => {
                let max = self.pop_usize()?;
                let dst_ptr = self.pop_usize()?;
                let path_len = self.pop_usize()?;
                let path_ptr = self.pop_usize()?;
                // Copy the path out of the heap into a fixed buffer first, so the
                // destination borrow below does not overlap the path's read borrow
                // (both live in the same heap, possibly in different regions).
                let mut path_buf = [0u8; MAX_ASSET_PATH_LEN];
                if path_len > MAX_ASSET_PATH_LEN {
                    self.push_host_outcome(HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT), 1)?;
                    return Ok(StepOutcome::Continue);
                }
                let Some(path_bytes) = heap_slice(heap, path_ptr, path_len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                path_buf[..path_len].copy_from_slice(path_bytes);
                let path = match core::str::from_utf8(&path_buf[..path_len]) {
                    Ok(path) => path,
                    Err(_) => {
                        self.push_host_outcome(
                            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
                            1,
                        )?;
                        return Ok(StepOutcome::Continue);
                    }
                };
                let Some(dst) = heap_slice_mut(heap, dst_ptr, max) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.asset_load(path, dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::TEXT_INPUT => {
                self.push_host_outcome(host.text_input(input), 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::IME_FEED_KEY => {
                let codepoint = self.pop()?;
                let kind = self.pop()?;
                self.push_host_outcome(host.ime_feed_key(kind, codepoint), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::IME_CONVERT => {
                self.push_host_outcome(host.ime_convert(), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::IME_QUERY_LINE => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.ime_query_line(dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_MOVE => {
                let dir = self.pop()?;
                self.push_host_outcome(host.edit_move(dir), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_DELETE => {
                let kind = self.pop()?;
                self.push_host_outcome(host.edit_delete(kind), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_LOAD => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(src) = heap_slice(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.edit_load(src);
                self.push_host_outcome(outcome, 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_QUERY_TEXT => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.edit_query_text(dst);
                self.push_host_outcome(outcome, 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::IME_DISPLAY => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.ime_display(dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_VISIBLE_LINE => {
                let row = self.pop()?;
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.edit_visible_line(row, dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_CURSOR_VIEW => {
                self.push_host_outcome(host.edit_cursor_view(), 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_SCROLL_ROW => {
                self.push_host_outcome(host.edit_scroll_row(), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_VIEW_METRICS => {
                self.push_host_outcome(host.edit_view_metrics(), 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_CURSOR_STATUS => {
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.edit_cursor_status(dst);
                self.push_host_outcome(outcome, 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_TOTAL_LINES => {
                self.push_host_outcome(host.edit_total_lines(), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_WRAP => {
                self.push_host_outcome(host.edit_wrap(), 1)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_HSCROLL_VIEW => {
                self.push_host_outcome(host.edit_hscroll_view(), 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::DIR_LIST => {
                let index = self.pop()?;
                let len = self.pop_usize()?;
                let ptr = self.pop_usize()?;
                let Some(dst) = heap_slice_mut(heap, ptr, len) else {
                    return Err(VmError::MemoryOutOfBounds);
                };
                let outcome = host.dir_list(index, dst);
                self.push_host_outcome(outcome, 2)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_RESERVE_ROWS => {
                let rows = self.pop()?;
                self.push_host_outcome(host.edit_reserve_rows(rows), 0)?;
                Ok(StepOutcome::Continue)
            }
            host_call::EDIT_CONFIGURE => {
                let rows = self.pop()?;
                let cols = self.pop()?;
                self.push_host_outcome(host.edit_configure(cols, rows), 0)?;
                Ok(StepOutcome::Continue)
            }
            _ => Err(VmError::HostCallDenied),
        }
    }

    /// Push a host-call result. Every call pushes a fixed number of values —
    /// `result_arity` result slots then a status word — regardless of success or
    /// failure, so static stack accounting matches runtime on both paths. Success
    /// pushes the results then `0`; failure pushes zeroed result slots then a
    /// negative status (`-error_code`). `result_arity` must match the success
    /// variant's result count for this call.
    fn push_host_outcome(
        &mut self,
        outcome: HostCallOutcome,
        result_arity: usize,
    ) -> Result<(), VmError> {
        match outcome {
            HostCallOutcome::Ok0 => self.push(0),
            HostCallOutcome::Ok1(value) => {
                self.push(value)?;
                self.push(0)
            }
            HostCallOutcome::Ok2(a, b) => {
                self.push(a)?;
                self.push(b)?;
                self.push(0)
            }
            HostCallOutcome::Err(code) => {
                for _ in 0..result_arity {
                    self.push(0)?;
                }
                self.push(-code.0)
            }
        }
    }

    fn pop_address(&mut self, width: usize, heap_len: usize) -> Result<usize, VmError> {
        let address = self.pop()?;
        let address = usize::try_from(address).map_err(|_| VmError::MemoryOutOfBounds)?;
        let end = address
            .checked_add(width)
            .ok_or(VmError::MemoryOutOfBounds)?;
        if end > heap_len {
            return Err(VmError::MemoryOutOfBounds);
        }
        self.budget.heap_bytes_peak = self.budget.heap_bytes_peak.max(end as u32);
        Ok(address)
    }

    fn pop_usize(&mut self) -> Result<usize, VmError> {
        usize::try_from(self.pop()?).map_err(|_| VmError::MemoryOutOfBounds)
    }

    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn peek(&self) -> Result<i32, VmError> {
        if self.stack_len == 0 {
            return Err(VmError::StackUnderflow);
        }
        Ok(self.stack[self.stack_len - 1])
    }

    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn pop(&mut self) -> Result<i32, VmError> {
        if self.stack_len == 0 {
            return Err(VmError::StackUnderflow);
        }
        self.stack_len -= 1;
        Ok(self.stack[self.stack_len])
    }

    #[cfg_attr(feature = "ram_interpreter", inline(always))]
    fn push(&mut self, value: i32) -> Result<(), VmError> {
        if self.stack_len >= STACK {
            return Err(VmError::StackOverflow);
        }
        self.stack[self.stack_len] = value;
        self.stack_len += 1;
        self.budget.stack_slots_peak = self.budget.stack_slots_peak.max(self.stack_len as u16);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepOutcome {
    Continue,
    Yielded,
    Exited(i32),
}

/// Upper bound on a package asset path copied out of the heap by `ASSET_LOAD`.
/// Package paths are short manifest-relative strings; this keeps the copy on a
/// fixed stack buffer so the `no_std` runtime needs no allocation.
const MAX_ASSET_PATH_LEN: usize = 128;

/// Borrow `len` bytes of the app heap at `ptr`, or `None` if the range is out of
/// bounds. The heap is supplied per frame (per-app sizing, KOTO-0096), so bounds
/// are checked against the supplied slice length.
fn heap_slice(heap: &[u8], ptr: usize, len: usize) -> Option<&[u8]> {
    let end = ptr.checked_add(len)?;
    heap.get(ptr..end)
}

fn heap_slice_mut(heap: &mut [u8], ptr: usize, len: usize) -> Option<&mut [u8]> {
    let end = ptr.checked_add(len)?;
    heap.get_mut(ptr..end)
}

/// Byte length of a `frames`x`channels` i16 PCM buffer (`frames * channels * 2`),
/// or `None` when either count is non-positive or the product overflows `usize`.
fn audio_pcm_len(frames: i32, channels: i32) -> Option<usize> {
    let frames = usize::try_from(frames).ok().filter(|&n| n > 0)?;
    let channels = usize::try_from(channels).ok().filter(|&n| n > 0)?;
    frames.checked_mul(channels)?.checked_mul(2)
}

fn parse_header(bytes: &[u8]) -> Result<KbcHeader, VerifyError> {
    if bytes.len() < KBC_HEADER_SIZE {
        return Err(VerifyError::TruncatedHeader);
    }
    if bytes[0..4] != KBC_MAGIC {
        return Err(VerifyError::BadMagic);
    }

    let version_major = u16_at(bytes, 4);
    let version_minor = u16_at(bytes, 6);
    if version_major != KBC_VERSION_MAJOR || version_minor != KBC_VERSION_MINOR {
        return Err(VerifyError::UnsupportedVersion);
    }
    if u32_at(bytes, 8) != KBC_HEADER_SIZE as u32 {
        return Err(VerifyError::BadHeaderSize);
    }
    if u32_at(bytes, 12) != 0 {
        return Err(VerifyError::NonzeroFlags);
    }
    if u32_at(bytes, 60) != 0 {
        return Err(VerifyError::NonzeroReserved);
    }

    Ok(KbcHeader {
        bytecode_size: u32_at(bytes, 16),
        code_offset: u32_at(bytes, 20),
        code_size: u32_at(bytes, 24),
        rodata_offset: u32_at(bytes, 28),
        rodata_size: u32_at(bytes, 32),
        entry_word: u32_at(bytes, 36),
        max_stack_slots: u16_at(bytes, 40),
        max_call_depth: u16_at(bytes, 42),
        max_heap_bytes: u32_at(bytes, 44),
        host_abi_major: u16_at(bytes, 48),
        host_abi_minor: u16_at(bytes, 50),
        debug_offset: u32_at(bytes, 52),
        debug_size: u32_at(bytes, 56),
    })
}

fn code_range(header: KbcHeader, bytecode_size: usize) -> Result<(usize, usize), VerifyError> {
    if header.code_size == 0
        || !header.code_size.is_multiple_of(4)
        || !header.code_offset.is_multiple_of(4)
    {
        return Err(VerifyError::BadCodeRange);
    }
    let start = usize::try_from(header.code_offset).map_err(|_| VerifyError::BadCodeRange)?;
    if start < KBC_HEADER_SIZE {
        return Err(VerifyError::BadCodeRange);
    }
    let size = usize::try_from(header.code_size).map_err(|_| VerifyError::BadCodeRange)?;
    let end = start.checked_add(size).ok_or(VerifyError::BadCodeRange)?;
    if end > bytecode_size {
        return Err(VerifyError::BadCodeRange);
    }
    Ok((start, end))
}

fn optional_range(
    offset: u32,
    size: u32,
    bytecode_size: usize,
) -> Result<Option<(usize, usize)>, VerifyError> {
    if offset == 0 {
        return if size == 0 {
            Ok(None)
        } else {
            Err(VerifyError::BadDataRange)
        };
    }
    let start = usize::try_from(offset).map_err(|_| VerifyError::BadDataRange)?;
    let len = usize::try_from(size).map_err(|_| VerifyError::BadDataRange)?;
    let end = start.checked_add(len).ok_or(VerifyError::BadDataRange)?;
    if start < KBC_HEADER_SIZE || end > bytecode_size {
        return Err(VerifyError::BadDataRange);
    }
    Ok(Some((start, end)))
}

fn reject_overlap(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Result<(), VerifyError> {
    let (Some(a), Some(b)) = (a, b) else {
        return Ok(());
    };
    if a.0 < b.1 && b.0 < a.1 {
        return Err(VerifyError::OverlappingRanges);
    }
    Ok(())
}

fn verify_instructions<C: CodeSource>(
    code: &mut C,
    code_words: u32,
    max_stack_slots: u16,
    limits: RuntimeLimits,
) -> Result<(), VerifyError> {
    let mut stack_depth = 0u16;
    for word_index in 0..code_words {
        let word = code.word(word_index).ok_or(VerifyError::BadBytecodeSize)?;
        let opcode = word[3];
        let operand = word[2];
        let immediate = u16::from_le_bytes([word[0], word[1]]);

        match opcode {
            opcode::NOP | opcode::HALT => {}
            opcode::BR | opcode::CALL => {
                check_target(immediate, code_words)?;
            }
            opcode::BR_IF_ZERO => {
                pop(&mut stack_depth, 1)?;
                check_target(immediate, code_words)?;
            }
            opcode::RET => {
                if !limits.treat_ret_as_exit && stack_depth == 0 {
                    return Err(VerifyError::BadInstruction);
                }
            }
            opcode::PUSH_I16 => push(&mut stack_depth, 1, max_stack_slots)?,
            opcode::DUP => {
                pop(&mut stack_depth, 1)?;
                push(&mut stack_depth, 2, max_stack_slots)?;
            }
            opcode::DROP => pop(&mut stack_depth, 1)?,
            opcode::SWAP => {
                pop(&mut stack_depth, 2)?;
                push(&mut stack_depth, 2, max_stack_slots)?;
            }
            opcode::LOAD_LOCAL => {
                if immediate != 0 {
                    return Err(VerifyError::BadInstruction);
                }
                let _local_index = operand;
                push(&mut stack_depth, 1, max_stack_slots)?;
            }
            opcode::STORE_LOCAL => {
                if immediate != 0 {
                    return Err(VerifyError::BadInstruction);
                }
                let _local_index = operand;
                pop(&mut stack_depth, 1)?;
            }
            opcode::ADD_I32
            | opcode::SUB_I32
            | opcode::MUL_I32
            | opcode::DIV_I32
            | opcode::AND_I32
            | opcode::OR_I32
            | opcode::XOR_I32
            | opcode::SHL_I32
            | opcode::SHR_I32 => {
                pop(&mut stack_depth, 2)?;
                push(&mut stack_depth, 1, max_stack_slots)?;
            }
            opcode::LOAD8 | opcode::LOAD16 | opcode::LOAD32 => {
                if immediate != 0 {
                    return Err(VerifyError::BadInstruction);
                }
                pop(&mut stack_depth, 1)?;
                push(&mut stack_depth, 1, max_stack_slots)?;
            }
            opcode::STORE8 | opcode::STORE16 | opcode::STORE32 => {
                if immediate != 0 {
                    return Err(VerifyError::BadInstruction);
                }
                pop(&mut stack_depth, 2)?;
            }
            opcode::HOST_CALL => {
                if immediate != 0 || !known_host_call(operand) {
                    return Err(VerifyError::BadHostCall);
                }
                let (pops, pushes) = host_call_stack_effect(operand);
                pop(&mut stack_depth, pops)?;
                push(&mut stack_depth, pushes, max_stack_slots)?;
            }
            _ => return Err(VerifyError::UnknownOpcode),
        }
    }
    Ok(())
}

fn check_target(target: u16, code_words: u32) -> Result<(), VerifyError> {
    if u32::from(target) >= code_words {
        return Err(VerifyError::BadBranch);
    }
    Ok(())
}

fn pop(stack_depth: &mut u16, count: u16) -> Result<(), VerifyError> {
    if *stack_depth < count {
        return Err(VerifyError::StackUnderflow);
    }
    *stack_depth -= count;
    Ok(())
}

fn push(stack_depth: &mut u16, count: u16, max_stack_slots: u16) -> Result<(), VerifyError> {
    *stack_depth = stack_depth
        .checked_add(count)
        .ok_or(VerifyError::StackOverflow)?;
    if *stack_depth > max_stack_slots {
        return Err(VerifyError::StackOverflow);
    }
    Ok(())
}

fn known_host_call(id: u8) -> bool {
    matches!(
        id,
        host_call::EXIT
            | host_call::YIELD_FRAME
            | host_call::DRAW_RECT
            | host_call::DRAW_TEXT
            | host_call::DRAW_TEXT_COLOR
            | host_call::DRAW_PIXELS_RGB565
            | host_call::GAME2D_SET_TILE
            | host_call::GAME2D_CLEAR_LAYER
            | host_call::GAME2D_PRESENT
            | host_call::GAME2D_STATIC_BEGIN
            | host_call::GAME2D_STATIC_END
            | host_call::GAME2D_STAMP_DEFINE
            | host_call::GAME2D_SPRITE_SET
            | host_call::GAME2D_SPRITE_HIDE
            | host_call::GAME2D_SPRITE_CLEAR_ALL
            | host_call::GAME2D_TEXT_SET
            | host_call::GAME2D_TEXT_HIDE
            | host_call::GAME2D_TEXT_CLEAR_ALL
            | host_call::INPUT_SNAPSHOT
            | host_call::TEXT_INPUT
            | host_call::AUDIO_SUBMIT_I16
            | host_call::PLAY_SFX
            | host_call::PLAY_BGM
            | host_call::PLAY_BGM_ASSET
            | host_call::PLAY_SFX_ASSET
            | host_call::STOP_BGM
            | host_call::FILE_OPEN
            | host_call::FILE_READ
            | host_call::FILE_WRITE
            | host_call::FILE_CLOSE
            | host_call::ASSET_LOAD
            | host_call::IME_FEED_KEY
            | host_call::IME_CONVERT
            | host_call::IME_QUERY_LINE
            | host_call::EDIT_MOVE
            | host_call::EDIT_DELETE
            | host_call::EDIT_LOAD
            | host_call::EDIT_QUERY_TEXT
            | host_call::IME_DISPLAY
            | host_call::EDIT_VISIBLE_LINE
            | host_call::EDIT_CURSOR_VIEW
            | host_call::EDIT_SCROLL_ROW
            | host_call::EDIT_VIEW_METRICS
            | host_call::EDIT_CURSOR_STATUS
            | host_call::EDIT_TOTAL_LINES
            | host_call::EDIT_WRAP
            | host_call::EDIT_HSCROLL_VIEW
            | host_call::DIR_LIST
            | host_call::EDIT_RESERVE_ROWS
            | host_call::EDIT_CONFIGURE
    )
}

/// Stack effect `(args_popped, values_pushed)` of a host call, used so the static
/// verifier tracks operand depth accurately across looping programs. Every call
/// pushes a fixed `values_pushed` (documented results plus the trailing status
/// word) on both success and failure (see `push_host_outcome`), so this is exact
/// rather than an approximation. IDs here must be kept in sync with
/// [`known_host_call`] and the runtime dispatch in `exec_host_call`.
fn host_call_stack_effect(id: u8) -> (u16, u16) {
    match id {
        host_call::EXIT => (1, 0),
        host_call::YIELD_FRAME => (0, 1),
        host_call::DRAW_RECT => (5, 1),
        host_call::DRAW_TEXT => (4, 1),
        host_call::DRAW_TEXT_COLOR => (5, 1),
        host_call::DRAW_PIXELS_RGB565 => (6, 1),
        host_call::GAME2D_SET_TILE => (4, 1),
        host_call::GAME2D_CLEAR_LAYER => (1, 1),
        host_call::GAME2D_PRESENT => (0, 1),
        host_call::GAME2D_STATIC_BEGIN => (0, 1),
        host_call::GAME2D_STATIC_END => (0, 1),
        host_call::GAME2D_STAMP_DEFINE => (4, 1),
        host_call::GAME2D_SPRITE_SET => (5, 1),
        host_call::GAME2D_SPRITE_HIDE => (1, 1),
        host_call::GAME2D_SPRITE_CLEAR_ALL => (0, 1),
        host_call::GAME2D_TEXT_SET => (6, 1),
        host_call::GAME2D_TEXT_HIDE => (1, 1),
        host_call::GAME2D_TEXT_CLEAR_ALL => (0, 1),
        host_call::INPUT_SNAPSHOT => (0, 3),
        host_call::TEXT_INPUT => (0, 3),
        host_call::AUDIO_SUBMIT_I16 => (3, 2),
        host_call::PLAY_SFX => (1, 1),
        host_call::PLAY_BGM => (1, 1),
        host_call::PLAY_BGM_ASSET => (2, 1),
        host_call::PLAY_SFX_ASSET => (2, 1),
        host_call::STOP_BGM => (0, 1),
        host_call::FILE_OPEN => (3, 2),
        host_call::FILE_READ => (3, 2),
        host_call::FILE_WRITE => (3, 2),
        host_call::FILE_CLOSE => (1, 1),
        host_call::ASSET_LOAD => (4, 2),
        host_call::IME_FEED_KEY => (2, 1),
        host_call::IME_CONVERT => (0, 1),
        host_call::IME_QUERY_LINE => (2, 2),
        host_call::EDIT_MOVE => (1, 1),
        host_call::EDIT_DELETE => (1, 2),
        host_call::EDIT_LOAD => (2, 1),
        host_call::EDIT_QUERY_TEXT => (2, 3),
        host_call::IME_DISPLAY => (2, 2),
        host_call::EDIT_VISIBLE_LINE => (3, 2),
        host_call::EDIT_CURSOR_VIEW => (0, 3),
        host_call::EDIT_SCROLL_ROW => (0, 2),
        host_call::EDIT_VIEW_METRICS => (0, 3),
        host_call::EDIT_CURSOR_STATUS => (2, 2),
        host_call::EDIT_TOTAL_LINES => (0, 2),
        host_call::EDIT_WRAP => (0, 2),
        host_call::EDIT_HSCROLL_VIEW => (0, 3),
        host_call::DIR_LIST => (3, 3),
        host_call::EDIT_RESERVE_ROWS => (1, 1),
        host_call::EDIT_CONFIGURE => (2, 1),
        _ => (0, 0),
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u16_checked(bytes: &[u8], offset: usize) -> Option<u16> {
    let bytes = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insn(op: u8, operand: u8, immediate: u16) -> [u8; 4] {
        let imm = immediate.to_le_bytes();
        [imm[0], imm[1], operand, op]
    }

    fn fixture(code: &[[u8; 4]]) -> Vec<u8> {
        let bytecode_size = KBC_HEADER_SIZE + code.len() * 4;
        let mut bytes = vec![0u8; bytecode_size];
        bytes[0..4].copy_from_slice(&KBC_MAGIC);
        bytes[4..6].copy_from_slice(&KBC_VERSION_MAJOR.to_le_bytes());
        bytes[6..8].copy_from_slice(&KBC_VERSION_MINOR.to_le_bytes());
        bytes[8..12].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(bytecode_size as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[24..28].copy_from_slice(&((code.len() * 4) as u32).to_le_bytes());
        bytes[40..42].copy_from_slice(&8u16.to_le_bytes());
        bytes[42..44].copy_from_slice(&4u16.to_le_bytes());
        bytes[44..48].copy_from_slice(&256u32.to_le_bytes());
        bytes[48..50].copy_from_slice(&HOST_ABI_MAJOR.to_le_bytes());
        bytes[50..52].copy_from_slice(&HOST_ABI_MINOR.to_le_bytes());
        for (index, word) in code.iter().enumerate() {
            let offset = KBC_HEADER_SIZE + index * 4;
            bytes[offset..offset + 4].copy_from_slice(word);
        }
        bytes
    }

    fn append_debug(bytes: &mut Vec<u8>, debug: &[u8]) {
        let offset = bytes.len() as u32;
        bytes.extend_from_slice(debug);
        let size = bytes.len() as u32;
        bytes[16..20].copy_from_slice(&size.to_le_bytes());
        bytes[52..56].copy_from_slice(&offset.to_le_bytes());
        bytes[56..60].copy_from_slice(&(debug.len() as u32).to_le_bytes());
    }

    fn debug_fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&KBC_DEBUG_MAGIC);
        bytes.extend_from_slice(&KBC_DEBUG_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(KBC_DEBUG_HEADER_SIZE as u16).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&9u16.to_le_bytes());
        bytes.extend_from_slice(b"test.koto");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&9u16.to_le_bytes());
        bytes
    }

    #[test]
    fn verifies_minimal_valid_bytecode() {
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 7),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
            insn(opcode::HALT, 0, 0),
        ]);

        let program = verify_kbc(&bytes, RuntimeLimits::simulator_default()).unwrap();

        assert_eq!(program.code_words(), 3);
        assert_eq!(program.header().entry_word, 0);
    }

    #[test]
    fn parses_debug_map_and_looks_up_source_locations() {
        let debug = debug_fixture();
        let map = DebugMap::parse(&debug).unwrap();

        assert_eq!(map.file_count(), 1);
        assert_eq!(map.entry_count(), 2);
        assert_eq!(
            map.lookup_pc(0),
            Some(SourceLocation {
                pc: 0,
                file: "test.koto",
                line: 3,
                col: 5
            })
        );
        assert_eq!(
            map.lookup_pc(3),
            Some(SourceLocation {
                pc: 2,
                file: "test.koto",
                line: 7,
                col: 9
            })
        );
    }

    #[test]
    fn reads_debug_map_from_kbc_debug_section() {
        let mut bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 7),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        append_debug(&mut bytes, &debug_fixture());
        verify_kbc(&bytes, RuntimeLimits::simulator_default()).unwrap();

        let map = debug_map(&bytes).unwrap().unwrap();

        assert_eq!(map.lookup_pc(0).unwrap().file, "test.koto");
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[0] = b'X';
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::BadMagic)
        );

        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_bad_code_range_and_entry() {
        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::BadCodeRange)
        );

        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::BadEntry)
        );
    }

    #[test]
    fn rejects_excessive_resource_requests() {
        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[40..42].copy_from_slice(&257u16.to_le_bytes());

        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::ResourceLimitExceeded)
        );
    }

    #[test]
    fn rejects_unknown_opcode_bad_branch_and_unknown_host_call() {
        let bytes = fixture(&[insn(0xFE, 0, 0)]);
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::UnknownOpcode)
        );

        let bytes = fixture(&[insn(opcode::BR, 0, 1)]);
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::BadBranch)
        );

        let bytes = fixture(&[insn(opcode::HOST_CALL, 0xFF, 0)]);
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::BadHostCall)
        );
    }

    #[test]
    fn rejects_static_stack_underflow_and_overflow() {
        let bytes = fixture(&[insn(opcode::DROP, 0, 0)]);
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::StackUnderflow)
        );

        let bytes = fixture(&[insn(opcode::PUSH_I16, 0, 1), insn(opcode::PUSH_I16, 0, 2)]);
        let limits = RuntimeLimits {
            max_stack_slots: 1,
            ..RuntimeLimits::simulator_default()
        };
        assert_eq!(
            verify_kbc(&bytes, limits),
            Err(VerifyError::ResourceLimitExceeded)
        );
    }

    #[test]
    fn rejects_program_exceeding_simulator_heap_profile() {
        // KOTO-0060: verification uses the same profile the VM is built from, so a
        // header requesting more heap than the simulator provides is rejected up
        // front rather than trapping at runtime.
        let mut bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let over = RuntimeLimits::simulator_default().max_heap_bytes + 1;
        bytes[44..48].copy_from_slice(&over.to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::ResourceLimitExceeded)
        );
    }

    #[test]
    fn rejects_rodata_larger_than_heap_request() {
        // KOTO-0139: rodata is copied into heap[0..rodata_size] at load, so a header
        // whose rodata exceeds its own heap request can never be the initial image.
        let mut bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        // Heap request smaller than the rodata we are about to append.
        bytes[44..48].copy_from_slice(&8u32.to_le_bytes());
        let rodata_offset = bytes.len() as u32;
        let rodata = vec![0u8; 16];
        bytes.extend_from_slice(&rodata);
        let bytecode_size = bytes.len() as u32;
        bytes[16..20].copy_from_slice(&bytecode_size.to_le_bytes());
        bytes[28..32].copy_from_slice(&rodata_offset.to_le_bytes());
        bytes[32..36].copy_from_slice(&(rodata.len() as u32).to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::RodataExceedsHeap)
        );
    }

    #[test]
    fn accepts_rodata_within_heap_request() {
        // The mirror of the rejection: rodata that fits the heap request verifies,
        // and `rodata_range` reports its byte span for the loader (KOTO-0139).
        let mut bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        bytes[44..48].copy_from_slice(&64u32.to_le_bytes());
        let rodata_offset = bytes.len() as u32;
        let rodata: Vec<u8> = (0..16u8).collect();
        bytes.extend_from_slice(&rodata);
        let bytecode_size = bytes.len() as u32;
        bytes[16..20].copy_from_slice(&bytecode_size.to_le_bytes());
        bytes[28..32].copy_from_slice(&rodata_offset.to_le_bytes());
        bytes[32..36].copy_from_slice(&(rodata.len() as u32).to_le_bytes());
        let program = verify_kbc(&bytes, RuntimeLimits::simulator_default()).unwrap();
        let (start, end) = program.rodata_range().expect("rodata present");
        assert_eq!(start, rodata_offset as usize);
        assert_eq!(end - start, rodata.len());
        assert_eq!(&bytes[start..end], &rodata[..]);
    }

    #[test]
    fn rejects_overlapping_data_ranges() {
        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        let rodata_offset = KBC_HEADER_SIZE as u32;
        bytes[28..32].copy_from_slice(&rodata_offset.to_le_bytes());
        bytes[32..36].copy_from_slice(&4u32.to_le_bytes());

        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::OverlappingRanges)
        );
    }

    #[derive(Default)]
    struct TestHost {
        rects: Vec<(i32, i32, i32, i32, i32)>,
        pixels: Vec<(i32, i32, i32, i32, Vec<u8>)>,
        snapshots: Vec<VmInputSnapshot>,
        ime_keys: Vec<(i32, i32)>,
        converted: bool,
        doc: Vec<u8>,
        feed_fail: bool,
        /// Recorded `audio_submit_i16` calls: `(frames, channels, little-endian bytes)`.
        audio_pcm: Vec<(i32, i32, Vec<u8>)>,
        sfx: Vec<i32>,
        bgm: Vec<i32>,
        bgm_stops: usize,
        /// Asset paths passed to `asset_load`, in call order.
        assets_requested: Vec<String>,
        /// Recorded `game2d_set_tile` calls: `(layer, x, y, tile_ref)`.
        tiles: Vec<(i32, i32, i32, i32)>,
        /// Number of `game2d_present` calls, and the heap len seen on the last one.
        presents: usize,
        present_heap_len: usize,
        /// KOTO-0169 Stage 0b: `hostcall_dispatch_begin`/`_end` invocations, to
        /// assert the timing seam brackets every `HOST_CALL` exactly once.
        dispatch_begins: u32,
        dispatch_ends: u32,
    }

    impl VmHost for TestHost {
        fn hostcall_dispatch_begin(&mut self) {
            self.dispatch_begins += 1;
        }

        fn hostcall_dispatch_end(&mut self) {
            self.dispatch_ends += 1;
        }

        fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome {
            self.rects.push((x, y, w, h, rgb565));
            HostCallOutcome::Ok0
        }

        fn draw_text(&mut self, _x: i32, _y: i32, _text: &str) -> HostCallOutcome {
            HostCallOutcome::Ok0
        }

        fn draw_pixels_rgb565(
            &mut self,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            pixels: &[u8],
        ) -> HostCallOutcome {
            self.pixels.push((x, y, w, h, pixels.to_vec()));
            HostCallOutcome::Ok0
        }

        fn game2d_set_tile(
            &mut self,
            layer: i32,
            x: i32,
            y: i32,
            tile_ref: i32,
        ) -> HostCallOutcome {
            self.tiles.push((layer, x, y, tile_ref));
            HostCallOutcome::Ok0
        }

        fn game2d_clear_layer(&mut self, _layer: i32) -> HostCallOutcome {
            self.tiles.clear();
            HostCallOutcome::Ok0
        }

        fn game2d_present(&mut self, heap: &[u8]) -> HostCallOutcome {
            self.presents += 1;
            self.present_heap_len = heap.len();
            HostCallOutcome::Ok0
        }

        fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
            self.snapshots.push(input);
            HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
        }

        fn audio_submit_i16(
            &mut self,
            frames: i32,
            channels: i32,
            samples: &[u8],
        ) -> HostCallOutcome {
            self.audio_pcm.push((frames, channels, samples.to_vec()));
            // Accept every offered frame.
            HostCallOutcome::Ok1(frames)
        }

        fn play_sfx(&mut self, id: i32) -> HostCallOutcome {
            self.sfx.push(id);
            HostCallOutcome::Ok0
        }

        fn play_bgm(&mut self, id: i32) -> HostCallOutcome {
            self.bgm.push(id);
            HostCallOutcome::Ok0
        }

        fn stop_bgm(&mut self) -> HostCallOutcome {
            self.bgm_stops += 1;
            HostCallOutcome::Ok0
        }

        fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
            HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
        }

        fn file_read(&mut self, _handle: i32, _dst: &mut [u8]) -> HostCallOutcome {
            HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
        }

        fn file_write(&mut self, _handle: i32, _src: &[u8]) -> HostCallOutcome {
            HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
        }

        fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
            HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
        }

        fn asset_load(&mut self, path: &str, dst: &mut [u8]) -> HostCallOutcome {
            self.assets_requested.push(path.to_string());
            let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
            let len = payload.len().min(dst.len());
            dst[..len].copy_from_slice(&payload[..len]);
            HostCallOutcome::Ok1(len as i32)
        }

        fn ime_feed_key(&mut self, kind: i32, codepoint: i32) -> HostCallOutcome {
            self.ime_keys.push((kind, codepoint));
            if self.feed_fail {
                HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT)
            } else {
                HostCallOutcome::Ok0
            }
        }

        fn ime_convert(&mut self) -> HostCallOutcome {
            self.converted = true;
            HostCallOutcome::Ok0
        }

        fn ime_query_line(&mut self, dst: &mut [u8]) -> HostCallOutcome {
            let text = b"line";
            let len = text.len().min(dst.len());
            dst[..len].copy_from_slice(&text[..len]);
            HostCallOutcome::Ok1(len as i32)
        }

        fn edit_load(&mut self, src: &[u8]) -> HostCallOutcome {
            self.doc = src.to_vec();
            HostCallOutcome::Ok0
        }

        fn edit_query_text(&mut self, dst: &mut [u8]) -> HostCallOutcome {
            let len = self.doc.len().min(dst.len());
            dst[..len].copy_from_slice(&self.doc[..len]);
            HostCallOutcome::Ok2(len as i32, self.doc.len() as i32)
        }
    }

    fn verified(bytes: &[u8]) -> VerifiedProgram {
        verify_kbc(bytes, RuntimeLimits::simulator_default()).unwrap()
    }

    impl<const STACK: usize, const CALLS: usize> BytecodeVm<STACK, CALLS> {
        /// Test helper: run one frame against a fixed 256-byte scratch heap (the
        /// `fixture` header requests 256 bytes). Tests that need to inspect the heap
        /// or exercise out-of-bounds access call `execute_frame` directly.
        fn execute_test_frame<H: VmHost>(
            &mut self,
            bytes: &[u8],
            program: &VerifiedProgram,
            host: &mut H,
            input: VmInputSnapshot,
            fuel: u32,
        ) -> Result<VmRunResult, VmError> {
            let mut heap = [0u8; 256];
            self.execute_frame(bytes, program, host, input, fuel, &mut heap)
        }
    }

    /// A [`CodeSource`] that serves words only from a small cached window over a
    /// backing code buffer, refilling on every out-of-window access. This mirrors
    /// the device PSRAM window (KOTO-0127) so a unit test can prove windowed fetch
    /// — including window-boundary refills — executes and verifies identically to
    /// a whole-program slice.
    struct WindowedCode {
        words: Vec<[u8; 4]>,
        window_words: usize,
        base: usize,
        cached: bool,
        refills: usize,
        /// `word_run` invocations, so a test can prove the VM amortized its
        /// fetches into runs (calls < executed instructions) — KOTO-0169 Stage 1.
        run_calls: usize,
    }

    impl WindowedCode {
        fn new(bytes: &[u8], program: &VerifiedProgram, window_words: usize) -> Self {
            let (code_start, _) = program.code_range();
            let words = (0..program.code_words() as usize)
                .map(|i| {
                    let o = code_start + i * 4;
                    [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]
                })
                .collect();
            Self {
                words,
                window_words,
                base: 0,
                cached: false,
                refills: 0,
                run_calls: 0,
            }
        }
    }

    impl CodeSource for WindowedCode {
        fn word(&mut self, index: u32) -> Option<[u8; 4]> {
            let i = index as usize;
            if i >= self.words.len() {
                return None;
            }
            let in_window = self.cached && i >= self.base && i < self.base + self.window_words;
            if !in_window {
                self.base = (i / self.window_words) * self.window_words;
                self.cached = true;
                self.refills += 1;
            }
            Some(self.words[i])
        }

        /// Mirror the device window's run semantics (KOTO-0169 Stage 1):
        /// resolve the first word exactly like `word` (same refill), then
        /// serve consecutive words clamped to the current window tile.
        fn word_run(&mut self, index: u32, dst: &mut [[u8; 4]]) -> usize {
            self.run_calls += 1;
            if dst.is_empty() {
                return 0;
            }
            let Some(first) = self.word(index) else {
                return 0;
            };
            dst[0] = first;
            let tile_end = (self.base + self.window_words).min(self.words.len());
            let run = dst.len().min(tile_end - index as usize);
            for (i, slot) in dst[1..run].iter_mut().enumerate() {
                *slot = self.words[index as usize + 1 + i];
            }
            run
        }
    }

    #[test]
    fn windowed_code_source_matches_slice_execution() {
        // A multi-word program with a backward branch, so a one-word window forces
        // a refill on every fetch and re-fetches earlier words after the loop.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 7),
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::PUSH_I16, 0, 9),
            insn(opcode::PUSH_I16, 0, 10),
            insn(opcode::PUSH_I16, 0, 0x1234u16),
            insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);

        let mut slice_host = TestHost::default();
        let slice_result = BytecodeVm::<8, 4>::new(&program)
            .unwrap()
            .execute_test_frame(
                &bytes,
                &program,
                &mut slice_host,
                VmInputSnapshot::empty(),
                100,
            )
            .unwrap();

        // Streaming verification over the same windowed source must accept it.
        let mut verify_window = WindowedCode::new(&bytes, &program, 1);
        let streamed = verify_kbc_streaming(
            &bytes,
            bytes.len(),
            &mut verify_window,
            RuntimeLimits::simulator_default(),
        )
        .unwrap();
        assert_eq!(streamed, program);

        let mut window = WindowedCode::new(&bytes, &program, 1);
        let mut window_host = TestHost::default();
        let mut heap = [0u8; 256];
        let window_result = BytecodeVm::<8, 4>::new(&program)
            .unwrap()
            .execute_frame_with(
                &mut window,
                &program,
                &mut window_host,
                VmInputSnapshot::empty(),
                100,
                &mut heap,
            )
            .unwrap();

        assert_eq!(slice_result, window_result);
        assert_eq!(slice_host.rects, window_host.rects);
        assert_eq!(slice_host.rects, [(7, 8, 9, 10, 0x1234)]);
        // A one-word window cannot satisfy consecutive PCs from cache, so the run
        // genuinely exercised repeated refills rather than a single whole-program load.
        assert!(window.refills > 1);
    }

    #[test]
    fn straight_line_fetch_line_matches_word_by_word_execution() {
        // KOTO-0169 Stage 1: a program with a backward loop (repeated branch
        // invalidation of the fetch line), a taken forward branch, and a tail
        // longer than the VM's 16-word line (line-exhaustion refill). Running
        // it over windowed sources that serve real multi-word runs must be
        // byte-identical to the resident-slice run, and the run-call count
        // must be below the executed-instruction count (fetches amortized).
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 3),
            insn(opcode::STORE_LOCAL, 0, 0),
            insn(opcode::LOAD_LOCAL, 0, 0), // word 2: loop head
            insn(opcode::BR_IF_ZERO, 0, 9), // -> done
            insn(opcode::LOAD_LOCAL, 0, 0),
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::SUB_I32, 0, 0),
            insn(opcode::STORE_LOCAL, 0, 0),
            insn(opcode::BR, 0, 2),       // back to loop head
            insn(opcode::PUSH_I16, 0, 5), // word 9: done
            insn(opcode::PUSH_I16, 0, 6),
            insn(opcode::PUSH_I16, 0, 7),
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::PUSH_I16, 0, 0x42),
            insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 11),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);

        let mut slice_host = TestHost::default();
        let mut slice_vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let slice_result = slice_vm
            .execute_test_frame(
                &bytes,
                &program,
                &mut slice_host,
                VmInputSnapshot::empty(),
                100,
            )
            .unwrap();
        assert_eq!(slice_result, VmRunResult::Exited(11));

        for window_words in [1usize, 3, 5, 64] {
            let mut window = WindowedCode::new(&bytes, &program, window_words);
            let mut host = TestHost::default();
            let mut heap = [0u8; 256];
            let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
            let result = vm
                .execute_frame_with(
                    &mut window,
                    &program,
                    &mut host,
                    VmInputSnapshot::empty(),
                    100,
                    &mut heap,
                )
                .unwrap();
            assert_eq!(result, slice_result, "window_words={window_words}");
            assert_eq!(host.rects, slice_host.rects, "window_words={window_words}");
            assert_eq!(
                vm.last_frame_fuel(),
                slice_vm.last_frame_fuel(),
                "window_words={window_words}"
            );
            // Multi-word windows must actually amortize: fewer source calls
            // than instructions executed. (A 1-word window degenerates to one
            // call per instruction, exactly today's behavior.)
            if window_words > 1 {
                assert!(
                    window.run_calls < vm.last_frame_fuel() as usize,
                    "window_words={window_words}: {} run calls for {} instructions",
                    window.run_calls,
                    vm.last_frame_fuel()
                );
            } else {
                assert_eq!(window.run_calls, vm.last_frame_fuel() as usize);
            }
        }
    }

    #[test]
    fn vm_runs_draw_and_exit_fixture() {
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::PUSH_I16, 0, 2),
            insn(opcode::PUSH_I16, 0, 3),
            insn(opcode::PUSH_I16, 0, 4),
            insn(opcode::PUSH_I16, 0, 0x1234),
            insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 7),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(7));
        assert_eq!(host.rects, [(1, 2, 3, 4, 0x1234)]);
    }

    #[test]
    fn vm_capacity_may_exceed_program_request_but_not_undersize_it() {
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);

        assert!(BytecodeVm::<16, 8>::new(&program).is_ok());
        assert!(matches!(
            BytecodeVm::<4, 4>::new(&program),
            Err(VmError::BadProgram)
        ));
        assert!(matches!(
            BytecodeVm::<8, 2>::new(&program),
            Err(VmError::BadProgram)
        ));
    }

    #[test]
    fn session_owns_frame_exit_and_trap_lifecycle() {
        let exit_bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 3),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let mut session =
            BytecodeSession::<8, 4>::new(&exit_bytes, RuntimeLimits::simulator_default(), 100)
                .unwrap();
        let mut host = TestHost::default();
        let mut heap = [0u8; 256];
        assert_eq!(
            session
                .step_frame(&exit_bytes, &mut host, VmInputSnapshot::empty(), &mut heap)
                .unwrap(),
            VmRunResult::Exited(3)
        );
        assert_eq!(session.frame(), 1);
        assert!(session.has_exited());
        assert_eq!(session.last_error(), None);

        let trap_bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::DIV_I32, 0, 0),
        ]);
        let mut session =
            BytecodeSession::<8, 4>::new(&trap_bytes, RuntimeLimits::simulator_default(), 100)
                .unwrap();
        assert_eq!(
            session.step_frame(&trap_bytes, &mut host, VmInputSnapshot::empty(), &mut heap),
            Err(VmError::DivisionByZero)
        );
        assert_eq!(session.frame(), 1);
        assert_eq!(session.last_error(), Some(VmError::DivisionByZero));
    }

    #[test]
    fn vm_blits_rgb565_pixels_from_heap() {
        // Store a 2x1 RGB565 block (two pixels = 4 bytes) at heap offset 0, then
        // blit it at (5, 6) with w=2, h=1, len=4.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::PUSH_I16, 0, 0x1234),
            insn(opcode::STORE16, 0, 0),
            insn(opcode::PUSH_I16, 0, 2),
            insn(opcode::PUSH_I16, 0, 0x5678),
            insn(opcode::STORE16, 0, 0),
            insn(opcode::PUSH_I16, 0, 5), // x
            insn(opcode::PUSH_I16, 0, 6), // y
            insn(opcode::PUSH_I16, 0, 2), // w
            insn(opcode::PUSH_I16, 0, 1), // h
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 4), // len
            insn(opcode::HOST_CALL, host_call::DRAW_PIXELS_RGB565, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.pixels, [(5, 6, 2, 1, vec![0x34, 0x12, 0x78, 0x56])]);
    }

    #[test]
    fn vm_writes_tile_and_presents() {
        // game2d_set_tile(layer=0, x=3, y=4, tile_ref=512) then game2d_present(),
        // checking arg order off the stack and that present receives the heap.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),   // layer
            insn(opcode::PUSH_I16, 0, 3),   // x
            insn(opcode::PUSH_I16, 0, 4),   // y
            insn(opcode::PUSH_I16, 0, 512), // tile_ref
            insn(opcode::HOST_CALL, host_call::GAME2D_SET_TILE, 0),
            insn(opcode::DROP, 0, 0), // status
            insn(opcode::HOST_CALL, host_call::GAME2D_PRESENT, 0),
            insn(opcode::DROP, 0, 0), // status
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.tiles, [(0, 3, 4, 512)]);
        assert_eq!(host.presents, 1);
        // `game2d_present` is handed the app heap so a draw-model host can re-read
        // tile art; `execute_test_frame` runs against the program's heap window.
        assert!(host.present_heap_len > 0);
    }

    #[test]
    fn vm_loads_package_asset_into_heap() {
        // Store the path "AB" at heap offset 100, asset_load it into heap[0..4], then
        // blit those 4 bytes so the round-trip (path read + payload write) is visible.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 100),    // path addr
            insn(opcode::PUSH_I16, 0, 0x4241), // "AB" little-endian
            insn(opcode::STORE16, 0, 0),
            insn(opcode::PUSH_I16, 0, 100), // path_ptr
            insn(opcode::PUSH_I16, 0, 2),   // path_len
            insn(opcode::PUSH_I16, 0, 0),   // dst_ptr
            insn(opcode::PUSH_I16, 0, 4),   // dst_max
            insn(opcode::HOST_CALL, host_call::ASSET_LOAD, 0),
            insn(opcode::DROP, 0, 0),     // status
            insn(opcode::DROP, 0, 0),     // bytes_read
            insn(opcode::PUSH_I16, 0, 0), // x
            insn(opcode::PUSH_I16, 0, 0), // y
            insn(opcode::PUSH_I16, 0, 2), // w
            insn(opcode::PUSH_I16, 0, 1), // h
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 4), // len
            insn(opcode::HOST_CALL, host_call::DRAW_PIXELS_RGB565, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.assets_requested, ["AB"]);
        assert_eq!(host.pixels, [(0, 0, 2, 1, vec![0xDE, 0xAD, 0xBE, 0xEF])]);
    }

    #[test]
    fn vm_submits_i16_pcm_from_heap() {
        // Store two little-endian i16 samples (4 bytes, one stereo frame) at heap
        // offset 0, then submit them with frames=1, channels=2 (len = 1*2*2 = 4).
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::PUSH_I16, 0, 0x0123),
            insn(opcode::STORE16, 0, 0),
            insn(opcode::PUSH_I16, 0, 2),
            insn(opcode::PUSH_I16, 0, 0x4567),
            insn(opcode::STORE16, 0, 0),
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 1), // frames
            insn(opcode::PUSH_I16, 0, 2), // channels
            insn(opcode::HOST_CALL, host_call::AUDIO_SUBMIT_I16, 0),
            insn(opcode::DROP, 0, 0), // status
            insn(opcode::DROP, 0, 0), // frames_written
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.audio_pcm, [(1, 2, vec![0x23, 0x01, 0x67, 0x45])]);
    }

    #[test]
    fn vm_rejects_audio_submit_with_bad_shape() {
        // frames = 0 is not a valid PCM shape: the call fails with BAD_ARGUMENT
        // without touching the heap, pushing a zeroed result and the negative status.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 0), // frames (invalid)
            insn(opcode::PUSH_I16, 0, 1), // channels
            insn(opcode::HOST_CALL, host_call::AUDIO_SUBMIT_I16, 0),
            insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Yielded);
        assert!(host.audio_pcm.is_empty());
        assert_eq!(vm.pop_value().unwrap(), 0); // yield status
        assert_eq!(vm.pop_value().unwrap(), -HostErrorCode::BAD_ARGUMENT.0); // status
        assert_eq!(vm.pop_value().unwrap(), 0); // zeroed frames_written
    }

    #[test]
    fn vm_dispatches_host_owned_audio_triggers() {
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0), // legacy host BGM id
            insn(opcode::HOST_CALL, host_call::PLAY_BGM, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 3), // Legacy host SFX id
            insn(opcode::HOST_CALL, host_call::PLAY_SFX, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::HOST_CALL, host_call::STOP_BGM, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.bgm, [0]);
        assert_eq!(host.sfx, [3]);
        assert_eq!(host.bgm_stops, 1);
    }

    #[test]
    fn vm_samples_input_once_per_frame_and_yields() {
        let bytes = fixture(&[
            insn(opcode::HOST_CALL, host_call::INPUT_SNAPSHOT, 0),
            insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        let input = VmInputSnapshot {
            held_bits: 0x12,
            pressed_bits: 0x34,
            ..VmInputSnapshot::empty()
        };

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, input, 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Yielded);
        assert_eq!(host.snapshots, [input]);
        assert_eq!(vm.pop_value().unwrap(), 0); // yield status
        assert_eq!(vm.pop_value().unwrap(), 0); // input_snapshot status
        assert_eq!(vm.pop_value().unwrap(), 0x34);
        assert_eq!(vm.pop_value().unwrap(), 0x12);
    }

    #[test]
    fn vm_preserves_state_after_fuel_exhaustion() {
        let bytes = fixture(&[insn(opcode::BR, 0, 0)]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        assert_eq!(
            vm.execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 3)
                .unwrap(),
            VmRunResult::FuelExhausted
        );
        assert_eq!(vm.pc(), 0);
        assert_eq!(
            vm.execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 1)
                .unwrap(),
            VmRunResult::FuelExhausted
        );
    }

    #[test]
    fn vm_executes_call_ret_locals_arithmetic_and_heap() {
        let bytes = fixture(&[
            insn(opcode::CALL, 0, 3),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
            insn(opcode::PUSH_I16, 0, 40),
            insn(opcode::PUSH_I16, 0, 2),
            insn(opcode::ADD_I32, 0, 0),
            insn(opcode::STORE_LOCAL, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::LOAD_LOCAL, 0, 0),
            insn(opcode::STORE32, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::LOAD32, 0, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::RET, 0, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        let mut heap = [0u8; 256];

        let result = vm
            .execute_frame(
                &bytes,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                100,
                &mut heap,
            )
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(&heap[..4], &42i32.to_le_bytes());
    }

    #[test]
    fn vm_tracks_budget_high_water_marks() {
        // One frame that calls a leaf function (call depth 1), writes 4 bytes at
        // heap offset 8 (heap high-water 12), touches local slot 2 (3 slots in
        // use), pushes the deepest stack for the DRAW_RECT args (5 slots), and
        // dispatches two host calls (DRAW_RECT + EXIT).
        let bytes = fixture(&[
            insn(opcode::CALL, 0, 18), // -> leaf RET at word 18
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::PUSH_I16, 0, 5),
            insn(opcode::STORE32, 0, 0), // heap [8..12]
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::PUSH_I16, 0, 2),
            insn(opcode::PUSH_I16, 0, 3),
            insn(opcode::STORE_LOCAL, 2, 0), // local slot 2
            insn(opcode::DROP, 0, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 1), // x
            insn(opcode::PUSH_I16, 0, 2), // y
            insn(opcode::PUSH_I16, 0, 3), // w
            insn(opcode::PUSH_I16, 0, 4), // h
            insn(opcode::PUSH_I16, 0, 0), // rgb565
            insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
            insn(opcode::RET, 0, 0), // word 18: leaf body
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        assert_eq!(vm.budget(), VmBudget::default());
        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();
        assert_eq!(result, VmRunResult::Exited(0));

        let budget = vm.budget();
        assert_eq!(budget.stack_slots_peak, 5);
        assert_eq!(budget.call_depth_peak, 1);
        assert_eq!(budget.local_slots_peak, 3);
        assert_eq!(budget.heap_bytes_peak, 12);
        assert_eq!(budget.host_calls_per_frame_peak, 2);
        assert_eq!(budget.frame_fuel_peak, vm.last_frame_fuel());
        assert!(budget.frame_fuel_peak > 0);
    }

    #[test]
    fn hostcall_dispatch_hooks_bracket_every_host_call() {
        // KOTO-0169 Stage 0b: two dispatched calls (DRAW_RECT + the internally
        // handled EXIT) must produce exactly two begin/end pairs.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 1), // x
            insn(opcode::PUSH_I16, 0, 2), // y
            insn(opcode::PUSH_I16, 0, 3), // w
            insn(opcode::PUSH_I16, 0, 4), // h
            insn(opcode::PUSH_I16, 0, 0), // rgb565
            insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 100)
            .unwrap();
        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.dispatch_begins, 2);
        assert_eq!(host.dispatch_ends, 2);
    }

    #[test]
    fn hostcall_dispatch_hooks_close_on_the_error_path() {
        // A DRAW_PIXELS_RGB565 whose (ptr, len) run past the heap traps during
        // argument marshalling (the verifier cannot see heap bounds); the end
        // hook must still fire so a timing host never leaks a window.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 0),     // x
            insn(opcode::PUSH_I16, 0, 0),     // y
            insn(opcode::PUSH_I16, 0, 1),     // w
            insn(opcode::PUSH_I16, 0, 1),     // h
            insn(opcode::PUSH_I16, 0, 0),     // ptr
            insn(opcode::PUSH_I16, 0, 30000), // len: past the fixture heap
            insn(opcode::HOST_CALL, host_call::DRAW_PIXELS_RGB565, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        assert_eq!(
            vm.execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 10),
            Err(VmError::MemoryOutOfBounds)
        );
        assert_eq!(host.dispatch_begins, 1);
        assert_eq!(host.dispatch_ends, 1);
    }

    #[test]
    fn vm_reports_runtime_traps_without_panicking() {
        let div_zero = fixture(&[
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::DIV_I32, 0, 0),
        ]);
        let program = verified(&div_zero);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        assert_eq!(
            vm.execute_test_frame(&div_zero, &program, &mut host, VmInputSnapshot::empty(), 10),
            Err(VmError::DivisionByZero)
        );

        // Address 300 is past the 256-byte fixture heap, so the 4-byte load is OOB.
        let out_of_bounds = fixture(&[insn(opcode::PUSH_I16, 0, 300), insn(opcode::LOAD32, 0, 0)]);
        let program = verified(&out_of_bounds);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        assert_eq!(
            vm.execute_test_frame(
                &out_of_bounds,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                10
            ),
            Err(VmError::MemoryOutOfBounds)
        );
    }

    #[test]
    fn heap_byte_and_word_accesses_are_bounds_checked() {
        let store8_oob = fixture(&[
            insn(opcode::PUSH_I16, 0, 256),
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::STORE8, 0, 0),
        ]);
        let program = verified(&store8_oob);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        assert_eq!(
            vm.execute_test_frame(
                &store8_oob,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                10
            ),
            Err(VmError::MemoryOutOfBounds)
        );

        let load16_oob = fixture(&[insn(opcode::PUSH_I16, 0, 255), insn(opcode::LOAD16, 0, 0)]);
        let program = verified(&load16_oob);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        assert_eq!(
            vm.execute_test_frame(
                &load16_oob,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                10
            ),
            Err(VmError::MemoryOutOfBounds)
        );
    }

    #[test]
    fn text_input_returns_codepoint_and_intent() {
        let bytes = fixture(&[
            insn(opcode::HOST_CALL, host_call::TEXT_INPUT, 0),
            insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        let input = VmInputSnapshot {
            text_codepoint: 0x3042,
            intent_bits: text_intent::SHIFT | text_intent::COMMIT,
            ..VmInputSnapshot::empty()
        };

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, input, 100)
            .unwrap();

        assert_eq!(result, VmRunResult::Yielded);
        assert_eq!(vm.pop_value().unwrap(), 0); // yield status
        assert_eq!(vm.pop_value().unwrap(), 0); // text_input status
        assert_eq!(
            vm.pop_value().unwrap() as u32,
            text_intent::SHIFT | text_intent::COMMIT
        );
        assert_eq!(vm.pop_value().unwrap(), 0x3042);
    }

    #[test]
    fn failing_host_calls_have_fixed_arity_and_do_not_leak_stack() {
        // A no-result host call that fails repeatedly must push exactly one value
        // (a negative status) each time, so `push args; call; drop` stays balanced
        // and never overflows the operand stack. Regression for the memo-app crash.
        let mut code = Vec::new();
        for _ in 0..32 {
            code.push(insn(opcode::PUSH_I16, 0, ime_key::CHARACTER as u16));
            code.push(insn(opcode::PUSH_I16, 0, 0x61));
            code.push(insn(opcode::HOST_CALL, host_call::IME_FEED_KEY, 0));
            code.push(insn(opcode::DROP, 0, 0));
        }
        code.push(insn(opcode::PUSH_I16, 0, 0));
        code.push(insn(opcode::HOST_CALL, host_call::EXIT, 0));
        let bytes = fixture(&code);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost {
            feed_fail: true,
            ..TestHost::default()
        };

        let result = vm
            .execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 1000)
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.ime_keys.len(), 32);
    }

    #[test]
    fn ime_and_edit_host_calls_dispatch_and_round_trip_heap() {
        let bytes = fixture(&[
            // edit_load(ptr=0, len=3)
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::PUSH_I16, 0, 3),
            insn(opcode::HOST_CALL, host_call::EDIT_LOAD, 0),
            insn(opcode::DROP, 0, 0),
            // ime_feed_key(kind=CHARACTER, codepoint='a')
            insn(opcode::PUSH_I16, 0, ime_key::CHARACTER as u16),
            insn(opcode::PUSH_I16, 0, 0x61),
            insn(opcode::HOST_CALL, host_call::IME_FEED_KEY, 0),
            insn(opcode::DROP, 0, 0),
            // ime_convert()
            insn(opcode::HOST_CALL, host_call::IME_CONVERT, 0),
            insn(opcode::DROP, 0, 0),
            // edit_query_text(ptr=16, len=8)
            insn(opcode::PUSH_I16, 0, 16),
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::HOST_CALL, host_call::EDIT_QUERY_TEXT, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut heap = [0u8; 256];
        heap[0..3].copy_from_slice(b"abc");
        let mut host = TestHost::default();

        let result = vm
            .execute_frame(
                &bytes,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                100,
                &mut heap,
            )
            .unwrap();

        assert_eq!(result, VmRunResult::Exited(0));
        assert_eq!(host.ime_keys, [(ime_key::CHARACTER, 0x61)]);
        assert!(host.converted);
        assert_eq!(host.doc, b"abc");
        assert_eq!(&heap[16..19], b"abc");
    }

    #[test]
    fn ime_query_line_writes_into_app_heap() {
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::HOST_CALL, host_call::IME_QUERY_LINE, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();
        let mut heap = [0u8; 256];

        assert_eq!(
            vm.execute_frame(
                &bytes,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                100,
                &mut heap
            ),
            Ok(VmRunResult::Exited(0))
        );
        assert_eq!(&heap[8..12], b"line");
    }

    #[test]
    fn ime_query_line_rejects_out_of_bounds_pointer() {
        // ptr 300 + len 8 lies past the 256-byte fixture heap.
        let bytes = fixture(&[
            insn(opcode::PUSH_I16, 0, 300),
            insn(opcode::PUSH_I16, 0, 8),
            insn(opcode::HOST_CALL, host_call::IME_QUERY_LINE, 0),
        ]);
        let program = verified(&bytes);
        let mut vm = BytecodeVm::<8, 4>::new(&program).unwrap();
        let mut host = TestHost::default();

        assert_eq!(
            vm.execute_test_frame(&bytes, &program, &mut host, VmInputSnapshot::empty(), 4),
            Err(VmError::MemoryOutOfBounds)
        );
    }

    #[test]
    fn rejects_unsupported_host_abi_minor() {
        let mut bytes = fixture(&[insn(opcode::HALT, 0, 0)]);
        bytes[50..52].copy_from_slice(&(HOST_ABI_MINOR + 1).to_le_bytes());
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::UnsupportedHostAbi)
        );
    }

    #[test]
    fn verifier_models_host_call_argument_consumption() {
        // `exit` pops its status code; with an empty operand stack the static
        // verifier must report underflow now that host-call effects are modeled.
        let bytes = fixture(&[insn(opcode::HOST_CALL, host_call::EXIT, 0)]);
        assert_eq!(
            verify_kbc(&bytes, RuntimeLimits::simulator_default()),
            Err(VerifyError::StackUnderflow)
        );
    }
}
