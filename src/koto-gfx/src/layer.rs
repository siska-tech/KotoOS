//! The retained Game2D layer data model — the plain-old-data layout of the
//! immediate command list and the retained board/sprite/text/static layers.
//!
//! These types and their capacity constants were lifted verbatim from the Pico
//! firmware's `app_host.rs` / `config.rs` (KotoGFX migration Stage 2, GFX-0002).
//! They are the *layout* half of the old `DeviceRuntimeHost`: pure `no_std`
//! structs with no host state, no VM coupling, and no hardware. The firmware
//! still owns the *instances* (the diff double-buffer) and the VM hostcall
//! landing pad; it re-exports these types so no call site, hostcall ID, or field
//! byte changes. Field layout is preserved exactly, asserted by the `size_of`/
//! `align_of` tests below.
//!
//! These structs carry no `koto-core` types, so the crate stays dependency-free.
//! The one method that previously returned a `koto_core::HostCallOutcome`
//! (`AppStaticLayer::push`) is split: the pure capacity check lives here as
//! [`AppStaticLayer::try_push`], and the firmware maps its `Result` back to a
//! hostcall outcome at the dispatch site.

/// Max UTF-8 byte length of one immediate `draw_text` / `AppDrawCommand::Text`
/// string (KOTO-0129). Longer strings are rejected at the hostcall boundary.
pub const MAX_APP_TEXT_BYTES: usize = 64;

/// Max UTF-8 byte length of one retained [`Game2dText`] item (KOTO-0141). Kept
/// smaller than [`MAX_APP_TEXT_BYTES`] because retained status values are short.
pub const GAME2D_TEXT_BYTES: usize = 32;

/// Capacity of the retained static/background command layer ([`AppStaticLayer`],
/// KOTO-0136). KotoBlocks' chrome is 65 commands; 80 leaves layout headroom.
pub const GAME2D_STATIC_CMD_CAP: usize = 80;

/// Retained Game2D board tilemap geometry (KOTO-0135): a `GAME2D_BOARD_COLS` x
/// `GAME2D_BOARD_ROWS` grid of 16x16 cells at the KotoBlocks well origin.
pub const GAME2D_BOARD_COLS: usize = 10;
pub const GAME2D_BOARD_ROWS: usize = 20;
pub const GAME2D_BOARD_CELLS: usize = GAME2D_BOARD_COLS * GAME2D_BOARD_ROWS;

/// Side length in pixels of one Game2D tile/cell (16x16).
pub const GAME2D_TILE_PX: i32 = 16;
/// Bytes of one Game2D tile: a 16x16 little-endian RGB565 block. The board and
/// sprite compositors read exactly this many bytes per tile from the app heap.
pub const GAME2D_TILE_BYTES: usize = (GAME2D_TILE_PX * GAME2D_TILE_PX) as usize * 2;
/// Top-left pixel origin of the retained board layer on the app surface.
pub const GAME2D_ORIGIN_X: i32 = 8;
pub const GAME2D_ORIGIN_Y: i32 = 0;

/// The retained board tilemap shape: one `tile_ref` per cell — the app-heap byte
/// offset of a 16x16 RGB565 tile, or `-1` for empty. Row-major (`cy * COLS + cx`).
pub type Game2dBoard = [i32; GAME2D_BOARD_CELLS];

/// Returned by [`AppStaticLayer::try_push`] when the layer is already at capacity.
/// The firmware maps this to the `NO_MEMORY` hostcall outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerFull;

/// One immediate app draw command. The bounded per-frame command list and the
/// retained static layer are arrays of these; the present path composites them.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum AppDrawCommand {
    Empty,
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        rgb565: u16,
    },
    Text {
        x: i32,
        y: i32,
        rgb565: u16,
        bytes: [u8; MAX_APP_TEXT_BYTES],
        len: u8,
    },
    // A `draw_pixels_rgb565` blit. The little-endian RGB565 block lives in the
    // resident app heap; the command keeps only a lightweight `(off, len)`
    // reference into it (`len == w * h * 2`) so the present path re-reads the
    // bytes at compose time instead of copying every tile into the command list
    // (KOTO-0129). Two commands compare equal — and so skip a delta repaint
    // (KOTO-0128) — when they blit the same source bytes to the same place; this
    // holds for tile/sprite games that bake their tiles once and animate by
    // changing which tile is blitted and where, not by rewriting tile bytes in
    // place at a fixed offset.
    Pixels {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        off: u32,
        len: u32,
    },
}

/// A retained Game2D sprite stamp (KOTO-0140): a reusable, position-independent
/// cell pattern. `count` cells live at app-heap byte offset `cells_off`. v1
/// supports only `format 0` (packed `(dcol,drow)` nibbles, the KOTO-0138 layout),
/// validated at define time, so the format is not stored. The host stores only
/// this descriptor; the cell bytes stay in the app heap. `count == 0` marks an
/// undefined stamp slot.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Game2dStampDef {
    pub cells_off: u32,
    pub count: u8,
}

impl Game2dStampDef {
    pub const fn undefined() -> Self {
        Self {
            cells_off: 0,
            count: 0,
        }
    }
}

/// A retained Game2D sprite (KOTO-0140): an on-screen instance of stamp `stamp_id`
/// at pixel `(x, y)` drawing the 16x16 tile at app-heap byte offset `tile_ref`.
/// Sprites are diffed by stable array index (`inst_id`), so a moving instance
/// yields one small stable dirty band rather than a positional-diff balloon.
/// `visible == false` is a hidden slot (its footprint becomes a dirty erase).
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Game2dSprite {
    pub stamp_id: u8,
    pub x: i16,
    pub y: i16,
    pub tile_ref: i32,
    pub visible: bool,
}

impl Game2dSprite {
    pub const fn hidden() -> Self {
        Self {
            stamp_id: 0,
            x: 0,
            y: 0,
            tile_ref: 0,
            visible: false,
        }
    }
}

/// A retained Game2D text item (KOTO-0141): the UTF-8 `bytes[..len]` string pinned
/// at pixel `(x, y)` in colour `rgb565`. Text items are diffed by stable array
/// index (`id`) across the two-list delta, so a value that does not change costs
/// nothing and a change repaints only its own row band — never the immediate
/// `draw_text` positional churn that shifted the command count (KOTO-0143
/// `CommandCountShift`). `visible == false` is a hidden slot (its footprint becomes
/// a dirty erase).
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Game2dText {
    pub x: i16,
    pub y: i16,
    pub rgb565: u16,
    pub bytes: [u8; GAME2D_TEXT_BYTES],
    pub len: u8,
    pub visible: bool,
}

impl Game2dText {
    pub const fn hidden() -> Self {
        Self {
            x: 0,
            y: 0,
            rgb565: 0,
            bytes: [0u8; GAME2D_TEXT_BYTES],
            len: 0,
            visible: false,
        }
    }
}

/// Retained Game2D static/background command layer (KOTO-0136). Draw commands
/// captured between `game2d_static_begin`/`game2d_static_end` for an app's static
/// chrome (page/well/grid/panel/label UI), composited *beneath* the board tilemap
/// and the per-frame immediate list so the app no longer re-emits them every
/// frame. The app's full-screen page clear lives here, so this layer also supplies
/// the delta's retained base colour (see `app_render::full_screen_base_color`).
///
/// Unlike the immediate per-frame lists, this is **not** double-buffered: it is
/// retained app-session state, not a positional-diff target, so the firmware owns
/// a single instance (KOTO-0136 fix — storing it inside the two double-buffered
/// draw hosts doubled its ~6 KiB cost and cost enough stack headroom to hang
/// boot). A rebuild is signalled by the explicit `rebuilt` flag rather than a
/// previous-vs-current diff: `game2d_static_begin` sets it, the presenter takes
/// one full repaint on it, and `clear_frame` resets it each frame.
#[derive(Clone, Eq, PartialEq)]
pub struct AppStaticLayer {
    pub commands: [AppDrawCommand; GAME2D_STATIC_CMD_CAP],
    pub len: usize,
    pub rebuilt: bool,
}

impl AppStaticLayer {
    pub const fn new() -> Self {
        Self {
            commands: [AppDrawCommand::Empty; GAME2D_STATIC_CMD_CAP],
            len: 0,
            rebuilt: false,
        }
    }

    /// Begin a capture: clear the layer and mark it rebuilt for this frame.
    pub fn begin(&mut self) {
        self.len = 0;
        self.rebuilt = true;
    }

    /// Append a command. `Ok(())` on success, `Err(LayerFull)` when the layer is
    /// at capacity (the caller maps this to the hostcall's `NO_MEMORY` outcome —
    /// the firmware keeps that koto-core mapping at the dispatch site).
    pub fn try_push(&mut self, command: AppDrawCommand) -> Result<(), LayerFull> {
        if self.len >= self.commands.len() {
            return Err(LayerFull);
        }
        self.commands[self.len] = command;
        self.len += 1;
        Ok(())
    }
}

impl Default for AppStaticLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    // Layout guards: these pin the byte layout the firmware double-buffer and the
    // hostcall field writes depend on. The element-type sizes are
    // platform-independent (no `usize`/pointer fields); `AppStaticLayer` carries a
    // `usize` so its size is asserted relative to its parts rather than as a fixed
    // magic number, keeping the assertion valid on both the 64-bit test host and
    // the 32-bit `thumbv6m` target.

    #[test]
    fn board_shape_is_cols_times_rows_i32s() {
        assert_eq!(GAME2D_BOARD_CELLS, 200);
        assert_eq!(size_of::<Game2dBoard>(), GAME2D_BOARD_CELLS * 4);
        assert_eq!(align_of::<Game2dBoard>(), 4);
    }

    #[test]
    fn stamp_def_layout() {
        assert_eq!(size_of::<Game2dStampDef>(), 8);
        assert_eq!(align_of::<Game2dStampDef>(), 4);
    }

    #[test]
    fn sprite_layout() {
        // i32 tile_ref forces align 4; 4 + 2 + 2 + 1 + 1(bool) padded to 12.
        assert_eq!(size_of::<Game2dSprite>(), 12);
        assert_eq!(align_of::<Game2dSprite>(), 4);
    }

    #[test]
    fn text_layout() {
        // 2 + 2 + 2 + 32 + 1 + 1 = 40, align 2 (no field wider than u16).
        assert_eq!(size_of::<Game2dText>(), 40);
        assert_eq!(align_of::<Game2dText>(), 2);
    }

    #[test]
    fn draw_command_layout() {
        // Largest variant is Text (75 payload bytes) plus the enum discriminant,
        // padded to the i32 alignment.
        assert_eq!(align_of::<AppDrawCommand>(), 4);
        // Text payload is 75 bytes (x,y i32 + rgb565 u16 + 64 bytes + len u8); the
        // enum discriminant packs into the trailing padding, so the whole enum is
        // 76 bytes — the per-command cost the draw-list budget is sized against.
        assert_eq!(size_of::<AppDrawCommand>(), 76);
    }

    #[test]
    fn static_layer_is_its_command_array_plus_len_and_flag() {
        // The static layer is exactly its command array + a usize len + a bool
        // rebuilt flag, with no hidden fields. Asserted relative to its parts so
        // the guard holds on both the 64-bit host and the 32-bit target (the
        // `usize` len makes a fixed magic number platform-specific).
        let array = size_of::<[AppDrawCommand; GAME2D_STATIC_CMD_CAP]>();
        assert!(size_of::<AppStaticLayer>() >= array + size_of::<usize>() + 1);
        assert!(size_of::<AppStaticLayer>() <= array + size_of::<usize>() + align_of::<usize>());
        assert_eq!(
            size_of::<AppStaticLayer>() % align_of::<AppStaticLayer>(),
            0
        );
        assert_eq!(align_of::<AppStaticLayer>(), align_of::<usize>());
    }

    #[test]
    fn constructors_make_empty_slots() {
        assert_eq!(Game2dStampDef::undefined().count, 0);
        assert!(!Game2dSprite::hidden().visible);
        assert!(!Game2dText::hidden().visible);
        assert_eq!(AppStaticLayer::new().len, 0);
    }

    #[test]
    fn try_push_fills_then_rejects() {
        let mut layer = AppStaticLayer::new();
        for _ in 0..GAME2D_STATIC_CMD_CAP {
            assert_eq!(layer.try_push(AppDrawCommand::Empty), Ok(()));
        }
        assert_eq!(layer.len, GAME2D_STATIC_CMD_CAP);
        assert_eq!(layer.try_push(AppDrawCommand::Empty), Err(LayerFull));
    }
}
