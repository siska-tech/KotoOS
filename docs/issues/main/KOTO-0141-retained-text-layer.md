# KOTO-0141: Retained Text Layer

- Status: done
- Type: feature
- Priority: P2
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §4.

## Goal

Stop apps re-emitting score/level/status as `draw_text` commands every frame. The `Text`
command inlines 64 bytes (`MAX_APP_TEXT_BYTES`), which is why a command costs ~80 B and
the 160-cap list costs ~24 KiB across both buffers — text is the heaviest churn on both
the VM and the command list. A retained text layer repaints a string only when it
changes.

Needed for the **Good UX** performance stage; not playable-blocking.

## ABI (host-call IDs `0x1D`–`0x1F`, reserved by GAME2D_ABI.md)

| ID | Name | Stack args | Effect |
| :-- | :-- | :-- | :-- |
| `0x1D` | `game2d_text_set` | `id x y ptr len rgb565` | Retain/update text item `id` (UTF-8 from app heap). |
| `0x1E` | `game2d_text_hide` | `id` | Hide a retained text item. |
| `0x1F` | `game2d_text_clear` | — | Hide all retained text. |

Composited in the fixed z-order above sprites, below the immediate list (KOTO-0140).

## Host data structures and budget

```rust
struct TextItem { x: i16, y: i16, rgb565: u16, len: u8, bytes: [u8; 48], visible: bool } // ~56 B
const MAX_TEXT_ITEMS: usize = 16;   // 896 B, ×2 for diff = ~1.8 KiB
```

## Scope

- **v1 (minimal):** dirty region per changed `id` is the current row-height band
  (`h = cell_h`, full width) so font-metric/CJK width math stays out of scope; the win is
  removing the per-frame VM + command-list churn. Pixel text via the existing `draw_text`
  font path.
- **v2 (deferred):** tight horizontal box from the layout advance (mixed half/full-width
  for CJK) so adjacent items don't repaint each other's row.
- **Later (deferred):** a cell-grid text mode for dense Shell/Memo text.

Static labels stay in the static layer (KOTO-0136); the text layer is for values that
*change*. Games set a handful of items on change; Shell sets app labels once per page;
Memo's dynamic status (`Ln/Col`, IME mode) becomes a few items.

## Dependencies

Cleaner after KOTO-0139 (any static text bytes as const). Independent of KOTO-0140.

## Acceptance criteria

- score / level / lines / status render from the text layer.
- Per-frame `Text` commands drop to **0** in steady play.
- A changed value repaints only its box (v1: its row band).
- Retained text SRAM ≤ ~1.8 KiB.

## Implementation (done)

- **ABI** ([runtime.rs, since moved into koto-vm](../../../src/koto-vm/src/lib.rs)): host calls `0x1D`
  `game2d_text_set(id, x, y, ptr, len, rgb565)`, `0x1E` `game2d_text_hide(id)`,
  `0x1F` `game2d_text_clear_all`, wired through the VM dispatch (the VM decodes the
  heap UTF-8 like `draw_text` and hands the host a `&str`), the verifier
  stack-effects (`(6,1)`/`(1,1)`/`(0,1)`), `known_host_call`, and the `name()`
  table. `HOST_ABI_MINOR` → 15. Mnemonics in [kbc-asm](../../../tools/kbc-asm/src/lib.rs);
  intrinsic wrappers (`game2d_text_set`/`_hide`/`_clear_all`) in
  [codegen.rs](../../../tools/koto-compiler/src/codegen.rs). `VmHost` trait methods
  default to `UNSUPPORTED`.
- **Sim** ([host.rs](../../../src/koto-sim/src/runtime/host.rs)): a `text_items`
  table on `SimRuntimeHost`, retained across the per-frame draw clear (like the
  tilemap/sprite layers). [render.rs](../../../src/koto-sim/src/runtime/render.rs)
  paints each visible item in the fixed z-order — above the sprite layer
  (re-emitted into `draw_pixels`) and below the per-frame immediate text — so the
  simulator composites it exactly where the device does.
- **Device** ([app_host.rs](../../../src/koto-pico/src/firmware/app_host.rs),
  [app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)): a
  `text_items: [Game2dText; GAME2D_MAX_TEXT_ITEMS]` array in `DeviceRuntimeHost`,
  retained across frames and diffed by stable index via the existing
  current-vs-previous two-list delta. A changed item contributes one dirty rect —
  the union of its old and new footprints; v1 uses the same from-`x`, 17px-tall row
  band the immediate `Text` command used (`text_footprint_rect`), so a migrated
  value repaints exactly the region the old `draw_text` did (pixel parity). The
  present path composites `static → tile → sprite → text → immediate`
  (`paint_text_layer` between the sprite layer and the immediate list). New SRAM ≈
  0.96 KiB across both lists (`GAME2D_MAX_TEXT_ITEMS 12`, `GAME2D_TEXT_BYTES 32`,
  in [config.rs](../../../src/koto-pico/src/firmware/config.rs)).
- **KotoBlocks** ([main.koto](../../../apps/koto_blocks/src/main.koto)): the run-state
  badge (実行中/一時停止), the SCORE/LEVEL/LINES values, and the "Hで交換" hold hint
  migrate from per-frame `draw_text_color` to id-keyed `game2d_text_set` (constants
  `T_BADGE`/`T_SCORE`/`T_LEVEL`/`T_LINES`/`T_HOLD`). The badge is hidden in game
  over and the hold hint when HOLD is in use (retained items persist until hidden,
  unlike immediate draws). Transient overlays (pause / game-over / "4 LINE!" /
  "LEVEL UP!" / score popup) stay immediate — they are debug/overlay/transition
  draws, not steady gameplay. The immediate list is empty on a normal falling frame.
- **Verification:** sim BMP capture of a gameplay frame shows the badge, score,
  level, lines, and hold state rendering through the retained text layer with the
  hold hint correctly hidden. A new sim unit test
  (`game2d_text_layer_retains_and_diffs_by_id`) covers set/update/hide/clear and
  cross-frame retention. Full `fmt`, gated `clippy`, `cargo test`, `build_apps
  --check`, golden frames, budgets, project harness, and the thumbv6m firmware
  build pass.

Device UART confirmed the goal: normal gameplay has an empty immediate list
(`rect=0 text=0 pixels=0`) with no unexpected `CommandCountShift`.

### Follow-up: stale immediate/overlay invalidation fix

Device visual testing surfaced stale pixels once gameplay stayed incremental: a
transient overlay/highlight (move/rotate halo, line-clear white blocks, "4 LINE!" /
score-popup banner) could linger after it should have disappeared, erased only later
when a moving sprite re-dirtied that region. Root cause was in the **generic immediate
diff**, not the text layer: `app_render::command_at` read `host.commands[index]` for
any in-bounds array slot, ignoring `host.len`. `clear_frame` resets only `len`, so
slots in `[len, commands.len())` still hold this double-buffer's commands from two
frames ago. When an overlay disappeared (`previous.len > current.len`), the positional
diff compared the real previous command against those stale bytes — which frequently
*equalled* the previous command — so `old == new` skipped the erase and the footprint
was never dirtied.

This was latent before KOTO-0141 but masked by the frequent `CommandCountShift` full
repaints that the text churn caused; emptying the immediate list exposed it. Fix:
`command_at` returns `AppDrawCommand::Empty` for `index >= host.len`, so a disappeared
command's old footprint is dirtied (diff vs `Empty`), a changed command dirties
`union(old, new)`, and a small list-length change stays incremental (no forced full
repaint). Covered by four host-logic unit tests in `app_render.rs`
(`disappeared_immediate_rect`/`_text_dirties_its_old_footprint`,
`moved_immediate_command_dirties_union_of_footprints`,
`stable_empty_immediate_list_is_clean`); the same scenarios were validated against a
standalone host replica of the diff path. Sprites/text/board layers were unaffected
(they diff a full fixed-size array by a `visible` flag, with no `len` boundary).
