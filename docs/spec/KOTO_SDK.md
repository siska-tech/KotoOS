# KotoSDK Prelude

The KotoSDK prelude is the app-facing contract for [Koto language](KOTO_APP_LANGUAGE.md)
apps: named functions for drawing, input, IME, the text editor, sandboxed files,
and app lifecycle, plus named constants — so app source never hard-codes numeric
[host-call IDs](RUNTIME_BYTECODE_ABI.md). The prelude is built into the
[koto-compiler](../../tools/koto-compiler): every program may call these functions
and use these constants with no import.

Checked-in sample apps are documented in [KotoSDK Samples](SDK_SAMPLES.md).

## Conventions

- **Values are `int`.** Every SDK function returns an `int`; there are no
  exceptions. Buffers are passed as a `buf` (which evaluates to its heap offset)
  together with an explicit length argument.
- **Error results.** Functions that produce a value return it on success
  (a handle, byte count, codepoint, etc.) and `-1` on failure (`result < 0` is the
  error test). Status-only functions return `0` on success and a negative status
  (`-error_code`) on failure. Every host call pushes a fixed number of values on
  both paths, so failures never desync the operand stack.
- **Buffer ownership.** Buffers are app-owned regions of the app heap declared with
  `buf name[N];`. The host only reads or writes the exact `(buffer, length)` slice
  you pass, and that length must stay within the buffer. Each app is given a heap
  sized to exactly what its buffers and string literals need (the compiler records it
  in the KBC header; per-app profile, KOTO-0096), up to a **16 KB** device ceiling;
  the host editor document holds up to **1024 bytes**.
- **Sandboxing.** File paths are app-virtual and resolved through the app's
  save-data sandbox (`data/<app_id>/...`); a path cannot escape it.

## Lifecycle

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `exit(code)` | `exit` | (never returns) | Terminates the app with `code`. |
| `yield_frame()` | `yield_frame` | status | Ends the current frame; the app resumes next frame with state intact. |

## Drawing

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `draw_rect(x, y, w, h, rgb565)` | `draw_rect` | status | Fills a clipped rectangle in RGB565. |
| `draw_text(x, y, buf, len)` | `draw_text` | status | Draws `len` UTF-8 bytes from `buf`. The host chooses the font/colour. |
| `draw_text_color(x, y, buf, len, rgb565)` | `draw_text_color` | status | Like `draw_text`, but draws in the caller-chosen RGB565 colour. |
| `draw_pixels(x, y, w, h, buf, len)` | `draw_pixels_rgb565` | status | Blits a `w`x`h` block of little-endian RGB565 pixels from `buf` (row-major, `len == w*h*2` bytes). The opaque tile/sprite primitive (PicoMings model); for sprite transparency, omit empty cells. |
| `game2d_set_tile(layer, x, y, tile_ref)` | `game2d_set_tile` | status | Writes one cell of a host-retained 16x16-tile layer. `tile_ref` is the byte offset of a 16x16 RGB565 tile in the app heap (as passed to `draw_pixels`), or `< 0` to clear the cell. Write only the cells that change. |
| `game2d_clear_layer(layer)` | `game2d_clear_layer` | status | Clears every cell of a tilemap `layer`. |
| `game2d_present()` | `game2d_present` | status | Composites the retained layers for this frame in fixed z-order (static → tile → sprite → immediate). Call once per frame after updating retained state; on the simulator it must follow every `sprite_set` so the final sprite state is re-emitted. |
| `game2d_static_begin()` | `game2d_static_begin` | status | Clears the retained static/background layer and routes subsequent `draw_rect`/`draw_text`/`draw_text_color`/`draw_pixels` into it until `game2d_static_end`. |
| `game2d_static_end()` | `game2d_static_end` | status | Routes draw calls back to the per-frame immediate list. |
| `game2d_stamp_define(stamp_id, cells_off, count, format)` | `game2d_stamp_define` | status | Registers a reusable cell *stamp*: `count` cells at app-heap byte offset `cells_off`. `format 0` packs each cell as a `(dcol,drow)` nibble (`drow*4 + dcol`). Define once; the cell bytes stay in the heap. |
| `game2d_sprite_set(inst_id, stamp_id, x, y, tile_ref)` | `game2d_sprite_set` | status | Creates/updates retained sprite `inst_id`: draws `stamp_id`'s cells at `(x + dcol*16, y + drow*16)`, each blitting the 16x16 tile at heap offset `tile_ref`. Diffed by stable `inst_id`. |
| `game2d_sprite_hide(inst_id)` | `game2d_sprite_hide` | status | Hides a retained sprite (its footprint is erased next present). |
| `game2d_sprite_clear_all()` | `game2d_sprite_clear_all` | status | Hides every retained sprite. |

The Game2D tilemap (KOTO-0135) is the retained alternative to re-blitting a whole
board with `draw_pixels` every frame: the host keeps the layer and repaints only
cells whose `tile_ref` changed, so a still board costs nothing. Bake tile art in
the app heap exactly as for `draw_pixels`, then name cells by their heap offset.
Currently one 10x20 board layer (origin (8, 0), 16x16 cells) is supported; layer
must be `0`. See [GAME2D_ABI.md](GAME2D_ABI.md).

The Game2D static/background layer (KOTO-0136) is the retained alternative to
re-emitting unchanging chrome — a full-screen background, panel frames, fixed
labels — every frame. Capture it once between `game2d_static_begin()` and
`game2d_static_end()` (e.g. when entering a screen or when the layout changes);
the host composites it beneath the board tilemap and the per-frame immediate
commands, so a draw call per static element is spent once, not every frame. Rebuild
it (another `begin`/`end`) only when the static layout actually changes.

The Game2D sprite/stamp layer (KOTO-0140) is the retained alternative to
re-blitting moving pieces — an active piece, a ghost, previews, a cursor, a small
actor — with `draw_pixels` every frame. A *stamp* is a reusable cell pattern
(`game2d_stamp_define`, defined once, reusing the same packed-nibble cell table you
already use for blits); a *sprite* is a retained placed instance of a stamp drawing
a chosen tile (`game2d_sprite_set`). The host diffs sprites by stable `inst_id`, so
a moving sprite repaints only the union of the cells it left and entered — no
per-frame blit loop, no command-list churn. Hide an instance with
`game2d_sprite_hide` (or all with `game2d_sprite_clear_all`). v1 is cell-stamp-only
(a stamp draws one tile per cell); a sprite is composited above the tile layer and
below the immediate `draw_*` list. See [GAME2D_ABI.md](GAME2D_ABI.md).

## Input

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `text_input()` | `text_input` | codepoint | Typed Unicode scalar this frame, or `0` if none. |
| `text_intent()` | `text_input` | intent bits | Edit-intent bitset this frame (see `INTENT_*`). |
| `input_held()` | `input_snapshot` | held button bits | Frame-stable held buttons. |
| `input_pressed()` | `input_snapshot` | pressed button bits | Buttons newly pressed this frame. |

## Audio

Audio synthesis is **host-owned**: apps trigger built-in effects by id and start
package-local KotoMML with `play_bgm_asset`. The host synthesizes PCM, so no
waveform data lives in the VM heap. `audio_submit` is a low-level escape hatch for
apps that generate their own PCM.

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `play_sfx(id)` | `play_sfx` | status | Triggers a one-shot sound effect (`SFX_*`). |
| `play_sfx_asset(path, len)` | `play_sfx_asset` | status | Plays one-shot KotoMML from a package `audio/*.kmml` asset. |
| `play_bgm_asset(path, len)` | `play_bgm_asset` | status | Starts looping KotoMML from a package `audio/*.kmml` asset. |
| `stop_bgm()` | `stop_bgm` | status | Stops the looping background-music track. |
| `audio_submit(buf, frames, channels)` | `audio_submit_i16` | frames accepted / `-1` | Submits `frames * channels` interleaved little-endian i16 PCM samples from `buf` (so `buf` holds `frames * channels * 2` bytes). Nonblocking; may accept fewer frames. Stereo is downmixed to mono. |

## Heap Data

The low-level typed heap helpers are bounds-checked by the VM. They are intended
for SDK helpers and focused tests; normal app code should prefer named helpers such
as `ActorArray` accessors instead of open-coded field offsets.

| Function | Emits | Returns | Notes |
| :------- | :---- | :------ | :---- |
| `heap_get_u8(addr)` | `load8` | `0..255` | Reads one byte. |
| `heap_set_u8(addr, value)` | `store8` | no value | Writes the low 8 bits. |
| `heap_get_u16(addr)` | `load16` | `0..65535` | Reads little-endian unsigned 16-bit. |
| `heap_set_u16(addr, value)` | `store16` | no value | Writes the low 16 bits, little-endian. |
| `heap_get_i16(addr)` | `load16` + sign extension | `-32768..32767` | Reads little-endian signed 16-bit. |
| `heap_set_i16(addr, value)` | `store16` | no value | Writes the low 16 bits, little-endian. |

## ActorArray

`ActorArray` is a small heap-backed game-state model. `actor_array_new(count)`
reserves `count * 12` bytes at compile time and returns the base offset. Each actor
stores `x`, `y`, `vx`, and `vy` as signed i16; `state` and `frame` as u8; and
`timer` as u16. Actor count changes the app heap request and addressed heap
high-water, not `user_slots_used`.

| Function | Returns | Notes |
| :------- | :------ | :---- |
| `actor_array_new(count)` | base offset | `count` must be a positive compile-time constant. |
| `actor_set_pos(actors, i, x, y)` | no value | Writes `x`/`y` as signed i16. |
| `actor_set_vel(actors, i, vx, vy)` | no value | Writes `vx`/`vy` as signed i16. |
| `actor_set_state(actors, i, state)` | no value | Writes one byte. |
| `actor_set_frame(actors, i, frame)` | no value | Writes one byte. |
| `actor_set_timer(actors, i, timer)` | no value | Writes unsigned 16-bit. |
| `actor_x(actors, i)`, `actor_y(actors, i)` | signed i16 | Position accessors. |
| `actor_vx(actors, i)`, `actor_vy(actors, i)` | signed i16 | Velocity accessors. |
| `actor_state(actors, i)`, `actor_frame(actors, i)` | u8 | State/frame accessors. |
| `actor_timer(actors, i)` | u16 | Timer accessor. |

## IME

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `ime_feed_key(kind, codepoint)` | `ime_feed_key` | status | Feeds one key (`IME_*` kind; `codepoint` used when `kind == IME_CHARACTER`). `IME_TOGGLE` switches IME on/off. |
| `ime_convert()` | `ime_convert` | status | Runs dictionary conversion on the current reading. |
| `ime_query_line(buf, max)` | `ime_query_line` | bytes written / `-1` | Serializes the structured composition line into `buf` (layout below). |
| `ime_display(buf, max)` | `ime_display` | bytes written | Writes a plain UTF-8 display string for the active composition. Active states are prefixed as `comp:`, `read:`, `cand:`, or `miss:` so apps can draw deterministic feedback directly with `draw_text`. |

The `ime_query_line` record is: `[mode:u8][sticky:u8]` then three length-prefixed
UTF-8 fields (`pending`, `reading`, `candidate`), each `[len:u8][bytes]`, then
`[cand_index:u8][cand_count:u8]` — the shown candidate's zero-based position and
the total candidate count for the current reading (both saturate at `255`; `0`
when there is no candidate). `mode` is `0` empty, `1` composing, `2` converting,
`3` candidate, `4` missing-candidate. Re-running `ime_convert` while a candidate
is shown cycles to the next candidate (wrapping).

### Simulator IME behavior

| Input | IME off | IME on |
| :---- | :------ | :----- |
| ASCII character | Inserts the character directly. | Alphabetic romaji is composed into kana. Unsupported romaji recovers as literal text instead of trapping. |
| Romaji sequence | Inserts each ASCII character directly. | Complete sequences commit kana (`ka` -> `か`); partial input is displayed as `comp:<pending>`. `nn`/`n'` commit a single ん with nothing left pending. |
| Punctuation / symbol | Inserts the ASCII character directly. | Inserts Japanese punctuation (`,`/`.` -> `、`/`。`, `-` -> `ー`, `/` -> `・`) or the full-width form for other printable symbols (`?`/`!` -> `？`/`！`, otherwise U+FF01..U+FF5E). Digits and spaces stay half-width. |
| Sticky Shift | No text change by itself. | Arms the next character as a conversion reading starter; the reading display uses `read:<reading>`. |
| Convert | No text change without a reading. | Looks up the reading. A hit displays `cand:<candidate>`; a miss displays `miss:<reading>`. Converting again while a candidate is shown cycles to the next candidate. |
| `q` while converting | Inserts `q`. | Commits the current reading converted to katakana and clears the composition. |
| Commit | No text change without active composition. | Commits the candidate or reading to the editor and clears the IME line. |
| Cancel | No text change. | Clears pending romaji, reading, candidate, and Sticky Shift state. |
| Backspace | Deletes before the editor cursor. | Deletes before the editor cursor. Apps may instead feed `IME_BACKSPACE` while composing to delete the last reading/pending character (ending the conversion when none remain) rather than editing the document. |
| Newline | Inserts a newline directly. | Flushes pending kana when possible, then inserts a newline. |

KotoSim window mode maps F1 to IME on/off, Tab to convert, Right Shift to commit,
Left Ctrl to cancel, F2 to save, F4 to open, F5 to create an unnamed empty memo,
and F10 to exit the app. Named-file saves ask for `y/n`; `n` opens Save As.
Backspace and Delete repeat while held. App input scripts use
the same intent names (`ime-toggle`, `convert`, `commit`, `cancel`, `save`,
`exit`) plus single-quoted characters such as `'a'` or `'\n'`.

## Text Editor

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `edit_load(buf, len)` | `edit_load` | status | Replaces the document with `len` UTF-8 bytes from `buf`. |
| `edit_query_text(buf, max)` | `edit_query_text` | byte length / `-1` | Copies the document into `buf` (up to `max`); returns its length. |
| `edit_visible_line(buf, max, row)` | `edit_visible_line` | byte length / `-1` | Copies one visible editor row into `buf`. `row` is relative to the current scroll position. |
| `edit_cursor_col()` | `edit_cursor_view` | column / `-1` | Returns the cursor column in the visible editor viewport. |
| `edit_cursor_row()` | `edit_cursor_view` | row / `-1` | Returns the cursor row in the visible editor viewport. |
| `edit_scroll_row()` | `edit_scroll_row` | row / `-1` | Returns the first visible document row, useful for scroll indicators. |
| `edit_cell_width()` | `edit_view_metrics` | pixels / `-1` | Returns the half-width cell advance used by the host editor and renderer. |
| `edit_cell_height()` | `edit_view_metrics` | pixels / `-1` | Returns the editor row pitch used by the host editor and renderer. |
| `edit_cursor_status(buf, max)` | `edit_cursor_status` | bytes written / `-1` | Writes a compact `Ln N Col M` status string for the current cursor. |
| `edit_total_lines()` | `edit_total_lines` | rows / `-1` | Returns the total visual (soft-wrapped) row count for scroll indicators. |
| `edit_wrap()` | `edit_wrap` | `1` / `0` | Returns whether the editor soft-wraps long lines (`0` = horizontal scroll). |
| `edit_hscroll()` | `edit_hscroll_view` | columns / `-1` | Horizontal scroll offset (columns); `0` while wrapping. |
| `edit_line_cols()` | `edit_hscroll_view` | columns / `-1` | Display width of the cursor's logical line, for a horizontal scrollbar. |
| `edit_move(dir)` | `edit_move` | status | Moves the cursor (`DIR_*`). |
| `edit_delete(kind)` | `edit_delete` | removed (`0`/`1`) / `-1` | Deletes around the cursor (`DELETE_*`). |
| `edit_reserve_rows(rows)` | `edit_reserve_rows` | status | Reserves the bottom `rows` rows of the viewport for an overlay (e.g. the IME panel) so the host scrolls the cursor above them; pass `0` to clear. |
| `edit_configure(cols, rows)` | `edit_configure` | status | Configures the app-visible editor viewport before loading document text. |

## Files

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `file_open(path, len, mode)` | `file_open` | handle / `-1` | Opens a sandboxed file (`MODE_*`). |
| `file_read(handle, buf, max)` | `file_read` | bytes read / `-1` | Reads into `buf`. |
| `file_write(handle, buf, len)` | `file_write` | bytes written / `-1` | Writes `len` bytes from `buf`. |
| `file_close(handle)` | `file_close` | status | Releases the handle. |
| `dir_count(buf, max, index)` | `dir_list` | entry count / `-1` | Number of files in the app save-data sandbox (also writes entry `index` into `buf`). |
| `dir_name(buf, max, index)` | `dir_list` | name length / `-1` | Writes the `index`-th sandbox filename (sorted) into `buf`; `0` when out of range. |

## Package Assets

The `file_*` calls above target the per-app **save sandbox** (`data/<app_id>/...`).
Package assets are different: they are the read-only files the manifest ships
inside the `.kpa` (audio, icons, sprite sheets). Audio assets are played by path
(`play_*_asset`); other asset bytes are pulled into the app heap with `asset_load`.

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `asset_load(path, len, buf, max)` | `asset_load` | bytes read / `-1` | Copies a manifest-declared package asset (e.g. an `*.kim` tile sheet) into `buf` in one shot, up to `max` bytes. The path must be declared by the manifest `assets` list; it is resolved read-only against the package, never the save sandbox. |

A typical use is loading a [`KIM1`](../guides/ASSET_PIPELINE.md) tile sheet once at startup,
then blitting individual 16×16 tiles each frame with `draw_pixels`:

```koto
buf sheet[9736];                                   // KIM1 header (8) + 19 * 512
asset_load("sprites/tiles.kim", 17, sheet, 9736);  // load once
draw_pixels(x, y, 16, 16, sheet + 8 + tile * 512, 512);  // blit tile `tile`
```

## Constants

| Group | Names | Values |
| :---- | :---- | :----- |
| File modes | `MODE_READ`, `MODE_WRITE`, `MODE_READWRITE` | `0`, `1`, `2` |
| IME key kinds | `IME_CHARACTER`, `IME_SHIFT`, `IME_CONVERT`, `IME_COMMIT`, `IME_CANCEL`, `IME_OTHER`, `IME_TOGGLE`, `IME_BACKSPACE` | `0`–`7` |
| Cursor directions | `DIR_LEFT`, `DIR_RIGHT`, `DIR_UP`, `DIR_DOWN`, `DIR_HOME`, `DIR_END` | `0`–`5` |
| Delete kinds | `DELETE_BACKSPACE`, `DELETE_FORWARD` | `0`, `1` |
| Intent bits | `INTENT_SHIFT`, `INTENT_CONVERT`, `INTENT_COMMIT`, `INTENT_CANCEL`, `INTENT_BACKSPACE`, `INTENT_DELETE`, `INTENT_LEFT`, `INTENT_RIGHT`, `INTENT_UP`, `INTENT_DOWN`, `INTENT_HOME`, `INTENT_END`, `INTENT_NEWLINE`, `INTENT_SAVE`, `INTENT_EXIT`, `INTENT_IME_TOGGLE`, `INTENT_OPEN`, `INTENT_NEW` | `1<<0` … `1<<17` |

A user `const` of the same name overrides the predefined value. The constants are
sourced from the host ABI modules (`koto_core::runtime`), so they cannot drift from
the runtime.

## Examples

### Frame loop skeleton

```koto
fn main() {
    loop {
        let intent = text_intent();
        if intent & INTENT_EXIT != 0 {
            exit(0);
        }
        draw_rect(0, 0, 320, 320, 0);
        yield_frame();
    }
}
```

### Load and save a sandboxed file

```koto
fn main() {
    buf path[8];
    buf doc[512];
    // (path bytes filled elsewhere, e.g. via a string literal copy)

    let h = file_open(path, 8, MODE_READ);
    if h >= 0 {
        let n = file_read(h, doc, 512);
        file_close(h);
        edit_load(doc, n);
    }

    let len = edit_query_text(doc, 512);
    let w = file_open(path, 8, MODE_WRITE);
    if w >= 0 {
        file_write(w, doc, len);
        file_close(w);
    }
    exit(0);
}
```

### IME composition and commit

```koto
fn main() {
    buf line[96];
    loop {
        let cp = text_input();
        let intent = text_intent();
        if cp != 0 {
            ime_feed_key(IME_CHARACTER, cp);
        }
        if intent & INTENT_CONVERT != 0 {
            ime_convert();
        }
        if intent & INTENT_COMMIT != 0 {
            ime_feed_key(IME_COMMIT, 0);
        }
        let clen = ime_display(line, 96);
        draw_text(0, 300, line, clen);
        yield_frame();
    }
}
```

## Stability

This prelude is intentionally small so Koto Memo, KotoVN prototypes, and later PDA
utilities share the same runtime concepts. New host calls should be added here as
named wrappers with documented results rather than exposed as raw IDs.
