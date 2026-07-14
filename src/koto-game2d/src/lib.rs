//! KotoGame2D — the app-facing Game2D semantic API over the KotoGFX retained
//! model.
//!
//! KotoGFX (GFX-0001..0005) owns the *layout* of the retained Game2D layers
//! (the board tilemap, the sprite/stamp layer, the text layer) and the
//! rasterizer/compositor that paints them. This crate is the next thin slice up
//! the stack (GFX-0006A): the **semantic operations** an app performs on that
//! retained model — set a board tile, place/hide a sprite, set a text item,
//! acknowledge a present — lifted out of the Pico firmware's VM hostcall landing
//! pad (`app_host.rs`) into one app-agnostic place.
//!
//! These operations were previously inlined in each `VmHost::game2d_*` method:
//! a fixed sequence of argument coercions, bounds checks, heap-extent validation,
//! and a single field write into the retained POD arrays. They are pure functions
//! of `(retained model, args, heap length)` with no host, VM, or hardware
//! coupling, so they live here verbatim. The firmware keeps the hostcall IDs and
//! the VM dispatch; each `game2d_*` method becomes a shim that borrows its
//! retained layers as a [`Game2dScene`], calls the matching method, and maps the
//! [`Game2dError`] back to a `koto_core::HostCallOutcome`. No hostcall ID, field
//! byte, validation order, or rendering output changes — this is the
//! non-behaviour-changing API-layer half of GFX-0006; budget enforcement (the
//! behavioural half) is *not* wired here.
//!
//! The retained layer *instances* (the diff double-buffer) stay in the firmware;
//! this layer never owns storage. It borrows the layers through [`Game2dScene`],
//! which makes the over-the-model API the single home for Game2D semantics while
//! the firmware keeps deciding where those layers are allocated. The immediate
//! draw-command path (`draw_rect`/`draw_pixels`/`draw_text`) and the
//! static-capture routing (`capturing_static`) are deliberately *not* moved: they
//! are immediate/host-routing state, not retained Game2D semantics.
//!
//! The crate is `no_std` (std is enabled only under `cfg(test)`) and never
//! allocates, so it builds for the `thumbv6m-none-eabi` firmware target. Its only
//! dependency is `koto-gfx`.

#![cfg_attr(not(test), no_std)]

use koto_gfx::{
    Game2dSprite, Game2dStampDef, Game2dText, Game2dTilemap, GAME2D_TEXT_BYTES,
    GAME2D_TILEMAP_MAX_COLS, GAME2D_TILEMAP_MAX_ROWS, GAME2D_TILE_BYTES,
};

/// Failure of a Game2D semantic operation. The firmware dispatch shim maps each
/// variant to the matching `koto_core::HostErrorCode` (`BadArgument` →
/// `BAD_ARGUMENT`, `NoMemory` → `NO_MEMORY`), so the hostcall outcomes an app
/// observes are byte-identical to the old inlined methods. Keeping a koto-core-
/// free error here is what lets this crate stay dependency-light (koto-gfx only).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Game2dError {
    /// An argument was out of range (bad layer/index/coords, an out-of-heap tile,
    /// or a malformed stamp descriptor). Maps to `BAD_ARGUMENT`.
    BadArgument,
    /// A bounded slot could not hold the value (an over-long retained text
    /// string). Maps to `NO_MEMORY`.
    NoMemory,
}

/// Result of a Game2D semantic operation. `Ok(())` corresponds to the hostcall's
/// `Ok0` (no return value); the firmware maps the error variants per
/// [`Game2dError`].
pub type Game2dResult = Result<(), Game2dError>;

/// A borrowed view of an app's retained Game2D layers — the board tilemap, the
/// sprite stamp table and placed sprites, and the text items. This is the surface
/// the app-facing Game2D operations act on. The firmware constructs one per
/// hostcall from the fields of its retained host (no storage is moved here), so
/// the view is zero-cost and the layer sizes are read from the borrowed slices
/// rather than fixed constants — a host with a differently-sized sprite/text/stamp
/// table is bounded correctly without this crate hard-coding its capacities.
pub struct Game2dScene<'a> {
    /// The board tilemap: one `tile_ref` per cell (app-heap byte offset of a 16x16
    /// RGB565 tile, or `-1` empty), using the fixed maximum-width stride.
    pub board: &'a mut Game2dTilemap,
    /// The stamp descriptor table, indexed by `stamp_id`.
    pub stamps: &'a mut [Game2dStampDef],
    /// The placed-sprite table, indexed by `inst_id`.
    pub sprites: &'a mut [Game2dSprite],
    /// The retained text-item table, indexed by `id`.
    pub text_items: &'a mut [Game2dText],
}

impl Game2dScene<'_> {
    /// Configure and clear retained tilemap layer 0. Capacity is fixed at 20x20;
    /// active dimensions and the pixel origin are app-defined (KOTO-0199).
    pub fn configure_tilemap(
        &mut self,
        layer: i32,
        columns: i32,
        rows: i32,
        origin_x: i32,
        origin_y: i32,
    ) -> Game2dResult {
        if layer != 0
            || !(1..=GAME2D_TILEMAP_MAX_COLS as i32).contains(&columns)
            || !(1..=GAME2D_TILEMAP_MAX_ROWS as i32).contains(&rows)
        {
            return Err(Game2dError::BadArgument);
        }
        let (Ok(columns), Ok(rows), Ok(origin_x), Ok(origin_y)) = (
            u8::try_from(columns),
            u8::try_from(rows),
            i16::try_from(origin_x),
            i16::try_from(origin_y),
        ) else {
            return Err(Game2dError::BadArgument);
        };
        self.board.cells.fill(-1);
        self.board.columns = columns;
        self.board.rows = rows;
        self.board.origin_x = origin_x;
        self.board.origin_y = origin_y;
        Ok(())
    }

    /// `game2d_set_tile`: place (or clear, when `tile_ref < 0`) a tile in the
    /// single board layer (`layer 0`). A non-empty `tile_ref` is the app-heap byte
    /// offset of a 16x16 RGB565 tile; the whole tile must lie within `heap_len`
    /// so the present path can re-read it without bounds checks.
    pub fn set_tile(
        &mut self,
        layer: i32,
        x: i32,
        y: i32,
        tile_ref: i32,
        heap_len: usize,
    ) -> Game2dResult {
        if layer != 0 {
            return Err(Game2dError::BadArgument);
        }
        let (Ok(cx), Ok(cy)) = (usize::try_from(x), usize::try_from(y)) else {
            return Err(Game2dError::BadArgument);
        };
        if cx >= usize::from(self.board.columns) || cy >= usize::from(self.board.rows) {
            return Err(Game2dError::BadArgument);
        }
        if tile_ref >= 0 {
            let off = tile_ref as usize;
            if off
                .checked_add(GAME2D_TILE_BYTES)
                .is_none_or(|end| end > heap_len)
            {
                return Err(Game2dError::BadArgument);
            }
        }
        // `tile_ref < 0` clears the cell; store it verbatim (the painter treats any
        // negative value as empty).
        self.board.cells[Game2dTilemap::cell_index(cx, cy)] = tile_ref;
        Ok(())
    }

    /// `game2d_clear_layer`: clear the whole board layer (`layer 0`) to empty.
    pub fn clear_layer(&mut self, layer: i32) -> Game2dResult {
        if layer != 0 {
            return Err(Game2dError::BadArgument);
        }
        self.board.cells.fill(-1);
        Ok(())
    }

    /// `game2d_stamp_define`: define stamp `stamp_id` as `count` cells living at
    /// app-heap byte offset `cells_off`. v1 supports only `format 0`; `count == 0`
    /// is rejected (an undefined slot is `count == 0`). The cell bytes are
    /// validated against the heap at present time, mirroring board tile offsets.
    pub fn stamp_define(
        &mut self,
        stamp_id: i32,
        cells_off: i32,
        count: i32,
        format: i32,
    ) -> Game2dResult {
        let (Ok(id), Ok(off), Ok(count)) = (
            usize::try_from(stamp_id),
            u32::try_from(cells_off),
            u8::try_from(count),
        ) else {
            return Err(Game2dError::BadArgument);
        };
        if id >= self.stamps.len() || format != 0 || count == 0 {
            return Err(Game2dError::BadArgument);
        }
        self.stamps[id] = Game2dStampDef {
            cells_off: off,
            count,
        };
        Ok(())
    }

    /// `game2d_sprite_set`: place sprite `inst_id` (of stamp `stamp_id`) at pixel
    /// `(x, y)` drawing the 16x16 tile at app-heap byte offset `tile_ref`. The
    /// whole tile must lie within `heap_len` (mirrors [`set_tile`](Self::set_tile)).
    pub fn sprite_set(
        &mut self,
        inst_id: i32,
        stamp_id: i32,
        x: i32,
        y: i32,
        tile_ref: i32,
        heap_len: usize,
    ) -> Game2dResult {
        let (Ok(id), Ok(stamp_id), Ok(x), Ok(y)) = (
            usize::try_from(inst_id),
            u8::try_from(stamp_id),
            i16::try_from(x),
            i16::try_from(y),
        ) else {
            return Err(Game2dError::BadArgument);
        };
        if id >= self.sprites.len() {
            return Err(Game2dError::BadArgument);
        }
        if tile_ref < 0
            || (tile_ref as usize)
                .checked_add(GAME2D_TILE_BYTES)
                .is_none_or(|end| end > heap_len)
        {
            return Err(Game2dError::BadArgument);
        }
        self.sprites[id] = Game2dSprite {
            stamp_id,
            x,
            y,
            tile_ref,
            visible: true,
        };
        Ok(())
    }

    /// `game2d_sprite_hide`: hide sprite `inst_id` (its footprint becomes a dirty
    /// erase next present).
    pub fn sprite_hide(&mut self, inst_id: i32) -> Game2dResult {
        let Ok(id) = usize::try_from(inst_id) else {
            return Err(Game2dError::BadArgument);
        };
        if id >= self.sprites.len() {
            return Err(Game2dError::BadArgument);
        }
        self.sprites[id].visible = false;
        Ok(())
    }

    /// `game2d_sprite_clear_all`: hide every sprite slot.
    pub fn sprite_clear_all(&mut self) -> Game2dResult {
        for sprite in self.sprites.iter_mut() {
            sprite.visible = false;
        }
        Ok(())
    }

    /// `game2d_text_set`: set retained text item `id` to `text` at pixel `(x, y)`
    /// in colour `rgb565`. A string longer than [`GAME2D_TEXT_BYTES`] is rejected
    /// with `NoMemory` (the app can fall back to immediate `draw_text`), mirroring
    /// the immediate list's bound.
    pub fn text_set(&mut self, id: i32, x: i32, y: i32, text: &str, rgb565: i32) -> Game2dResult {
        let (Ok(id), Ok(x), Ok(y)) = (usize::try_from(id), i16::try_from(x), i16::try_from(y))
        else {
            return Err(Game2dError::BadArgument);
        };
        if id >= self.text_items.len() {
            return Err(Game2dError::BadArgument);
        }
        if text.len() > GAME2D_TEXT_BYTES {
            return Err(Game2dError::NoMemory);
        }
        let mut bytes = [0u8; GAME2D_TEXT_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.text_items[id] = Game2dText {
            x,
            y,
            rgb565: rgb565 as u16,
            bytes,
            len: text.len() as u8,
            visible: true,
        };
        Ok(())
    }

    /// `game2d_text_hide`: hide retained text item `id` (its footprint becomes a
    /// dirty erase next present).
    pub fn text_hide(&mut self, id: i32) -> Game2dResult {
        let Ok(id) = usize::try_from(id) else {
            return Err(Game2dError::BadArgument);
        };
        if id >= self.text_items.len() {
            return Err(Game2dError::BadArgument);
        }
        self.text_items[id].visible = false;
        Ok(())
    }

    /// `game2d_text_clear_all`: hide every text-item slot.
    pub fn text_clear_all(&mut self) -> Game2dResult {
        for item in self.text_items.iter_mut() {
            item.visible = false;
        }
        Ok(())
    }
}

/// `game2d_present`: acknowledge a present request. With the KOTO-0140 fixed
/// z-order the retained board/sprite/text layers composite at fixed positions in
/// the present path and ride the current-vs-previous host delta, so `present` no
/// longer injects a stream marker — it is a pure acknowledgement. It lives here
/// (rather than as a `Game2dScene` method) because it touches no retained state;
/// the present *trigger* and the compose/flush stay in the firmware display
/// service (GFX-0005). This is the API seam for a future present-as-request
/// abstraction.
pub fn present() -> Game2dResult {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_gfx::{Game2dTilemap, GAME2D_TILEMAP_MAX_COLS, GAME2D_TILE_PX};

    const MAX_STAMPS: usize = 32;
    const MAX_SPRITES: usize = 16;
    const MAX_TEXT_ITEMS: usize = 12;
    // A heap large enough to admit a tile at offset 0 but tight enough to test the
    // out-of-heap rejection at its tail.
    const HEAP_LEN: usize = GAME2D_TILE_BYTES * 4;

    struct Model {
        board: Game2dTilemap,
        stamps: [Game2dStampDef; MAX_STAMPS],
        sprites: [Game2dSprite; MAX_SPRITES],
        text_items: [Game2dText; MAX_TEXT_ITEMS],
    }

    impl Model {
        fn new() -> Self {
            Self {
                board: Game2dTilemap::legacy(),
                stamps: [Game2dStampDef::undefined(); MAX_STAMPS],
                sprites: [Game2dSprite::hidden(); MAX_SPRITES],
                text_items: [Game2dText::hidden(); MAX_TEXT_ITEMS],
            }
        }

        fn scene(&mut self) -> Game2dScene<'_> {
            Game2dScene {
                board: &mut self.board,
                stamps: &mut self.stamps,
                sprites: &mut self.sprites,
                text_items: &mut self.text_items,
            }
        }
    }

    #[test]
    fn set_tile_places_then_clears() {
        let mut m = Model::new();
        assert_eq!(m.scene().set_tile(0, 1, 2, 0, HEAP_LEN), Ok(()));
        assert_eq!(m.board.cells[Game2dTilemap::cell_index(1, 2)], 0);
        // Negative clears the cell verbatim.
        assert_eq!(m.scene().set_tile(0, 1, 2, -1, HEAP_LEN), Ok(()));
        assert_eq!(m.board.cells[Game2dTilemap::cell_index(1, 2)], -1);
    }

    #[test]
    fn configure_tilemap_sets_active_shape_origin_and_clears() {
        let mut m = Model::new();
        m.board.cells[5] = 42;
        assert_eq!(m.scene().configure_tilemap(0, 20, 7, -16, 24), Ok(()));
        assert_eq!((m.board.columns, m.board.rows), (20, 7));
        assert_eq!((m.board.origin_x, m.board.origin_y), (-16, 24));
        assert!(m.board.cells.iter().all(|&cell| cell == -1));
        assert_eq!(m.scene().set_tile(0, 19, 6, 0, HEAP_LEN), Ok(()));
    }

    #[test]
    fn configure_tilemap_rejects_invalid_capacity_layer_and_origin() {
        let mut m = Model::new();
        for args in [
            (1, 10, 10, 0, 0),
            (0, 0, 10, 0, 0),
            (0, GAME2D_TILEMAP_MAX_COLS as i32 + 1, 10, 0, 0),
            (0, 10, 21, 0, 0),
            (0, 10, 10, i16::MAX as i32 + 1, 0),
        ] {
            assert_eq!(
                m.scene()
                    .configure_tilemap(args.0, args.1, args.2, args.3, args.4),
                Err(Game2dError::BadArgument)
            );
        }
    }

    #[test]
    fn set_tile_rejects_bad_layer_coords_and_out_of_heap() {
        let mut m = Model::new();
        assert_eq!(
            m.scene().set_tile(1, 0, 0, 0, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(
            m.scene().set_tile(0, -1, 0, 0, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(
            m.scene().set_tile(0, 10, 0, 0, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
        // A tile whose 16x16 RGB565 extent runs past the heap end is rejected.
        let last_ok = (HEAP_LEN - GAME2D_TILE_BYTES) as i32;
        assert_eq!(m.scene().set_tile(0, 0, 0, last_ok, HEAP_LEN), Ok(()));
        assert_eq!(
            m.scene().set_tile(0, 0, 0, last_ok + 1, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
    }

    #[test]
    fn clear_layer_only_layer_zero() {
        let mut m = Model::new();
        m.board.cells[5] = 42;
        assert_eq!(m.scene().clear_layer(0), Ok(()));
        assert!(m.board.cells.iter().all(|&c| c == -1));
        assert_eq!(m.scene().clear_layer(1), Err(Game2dError::BadArgument));
    }

    #[test]
    fn stamp_define_validates_id_format_and_count() {
        let mut m = Model::new();
        assert_eq!(m.scene().stamp_define(0, 8, 4, 0), Ok(()));
        assert_eq!(m.stamps[0].cells_off, 8);
        assert_eq!(m.stamps[0].count, 4);
        assert_eq!(
            m.scene().stamp_define(MAX_STAMPS as i32, 0, 1, 0),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(
            m.scene().stamp_define(0, 0, 1, 1),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(
            m.scene().stamp_define(0, 0, 0, 0),
            Err(Game2dError::BadArgument)
        );
    }

    #[test]
    fn sprite_set_hide_and_clear_all() {
        let mut m = Model::new();
        assert_eq!(m.scene().sprite_set(0, 1, 10, 20, 0, HEAP_LEN), Ok(()));
        assert!(m.sprites[0].visible);
        assert_eq!(m.sprites[0].x, 10);
        assert_eq!(m.sprites[0].y, 20);
        assert_eq!(m.sprites[0].stamp_id, 1);
        // Out-of-heap tile and negative tile both rejected.
        assert_eq!(
            m.scene().sprite_set(1, 0, 0, 0, -1, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(
            m.scene()
                .sprite_set(MAX_SPRITES as i32, 0, 0, 0, 0, HEAP_LEN),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(m.scene().sprite_hide(0), Ok(()));
        assert!(!m.sprites[0].visible);
        m.scene().sprite_set(2, 0, 0, 0, 0, HEAP_LEN).unwrap();
        assert_eq!(m.scene().sprite_clear_all(), Ok(()));
        assert!(m.sprites.iter().all(|s| !s.visible));
    }

    #[test]
    fn text_set_hide_and_clear_all() {
        let mut m = Model::new();
        assert_eq!(m.scene().text_set(0, 3, 4, "HI", 0x1234), Ok(()));
        assert!(m.text_items[0].visible);
        assert_eq!(m.text_items[0].len, 2);
        assert_eq!(&m.text_items[0].bytes[..2], b"HI");
        // Over-long retained text is NoMemory, not BadArgument.
        let long = "x".repeat(GAME2D_TEXT_BYTES + 1);
        assert_eq!(
            m.scene().text_set(0, 0, 0, &long, 0),
            Err(Game2dError::NoMemory)
        );
        assert_eq!(
            m.scene().text_set(MAX_TEXT_ITEMS as i32, 0, 0, "a", 0),
            Err(Game2dError::BadArgument)
        );
        assert_eq!(m.scene().text_hide(0), Ok(()));
        assert!(!m.text_items[0].visible);
        m.scene().text_set(1, 0, 0, "b", 0).unwrap();
        assert_eq!(m.scene().text_clear_all(), Ok(()));
        assert!(m.text_items.iter().all(|t| !t.visible));
    }

    #[test]
    fn present_is_a_noop_ack() {
        assert_eq!(present(), Ok(()));
        // Sanity: the tile geometry the heap checks assume is the documented 16x16.
        assert_eq!(
            GAME2D_TILE_BYTES,
            (GAME2D_TILE_PX * GAME2D_TILE_PX) as usize * 2
        );
    }
}
