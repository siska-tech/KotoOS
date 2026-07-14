# KotoOS App Development Loop

This is the practical loop for building and evaluating a bytecode app entirely on
the PC, from a source edit to a running app in KotoSim — through the same runtime
path the device will use.

```text
edit source ──> build ──> run in KotoSim ──> observe / script ──> repeat
```

## 1. Write the source

App sources live under `apps/<app_id>/`:

- High-level: `apps/<app_id>/src/main.koto` — the [Koto language](../spec/KOTO_APP_LANGUAGE.md),
  using the [KotoSDK prelude](../spec/KOTO_SDK.md) for drawing, input, IME, editor, files,
  and lifecycle.
- Low-level (IR/fixtures): `apps/<app_id>/*.kbc.asm` — `kbc-asm` assembly.

To create a high-level app skeleton and register it in the build loop:

```powershell
cargo run -p koto-app-scaffold -- --app-id dev.koto.apps.todo-list --name "Todo List"
```

In VS Code, the same operation is available without CLI flags: click the
new-folder action in the Explorer title or run **Koto: Create New App Project**
(KOTO-0197). Enter the reverse-DNS app ID, display name, and the proposed
`apps/<slug>` directory, then confirm the summary. The extension streams the
scaffold output and opens the generated `src/main.koto`; cancellation and all
duplicate/overwrite checks remain enforced by `koto-app-scaffold`.

The command creates a self-contained `apps/todo_list/` folder:
`src/main.koto` + `src/helpers.koto`, a smoke scenario under `scenarios/`, an
`icon.kicon` placeholder, and the `app.json` descriptor (section 2) — no shared
registry is touched. It validates the generated app ID, display name, runtime,
entry path, and icon path through the same package manifest rules used by the
runtime and packer.

## 2. The app descriptor (`app.json`)

Each app is a self-contained folder under `apps/` described by its own
`apps/<dir>/app.json` (KOTO-0195). The scaffold writes it; `build_apps.py`
discovers every descriptor by scanning `apps/**/app.json`, so there is no
shared registry to edit. A descriptor carries the build recipe **and** the
package fields (name, icon, palette, memory, permissions):

```json
{
  "app_id": "dev.koto.memo",
  "kind": "koto",
  "package": "memo",
  "name": "Koto Memo",
  "description": "テキストメモを作成・編集できます。",
  "category": "アプリ",
  "source": "src/main.koto",
  "icon": "icon.kicon",
  "memory": { "sram_work_bytes": 24576, "psram_cache_bytes": 32768 },
  "permissions": { "fs": "sandbox", "network": false }
}
```

In-app paths (`source`, `icon`, and the optional `audio` / `images` / `maps`
sources) are **app-relative**, so the folder is copy-paste portable. `kind` is
`koto` (compiled by `koto-compiler`) or `asm` (assembled by `kbc-asm`).
`package` is the staging/archive stem (`APPS/<package>.kpa`). The build
generates the `.kpa.json` manifest and stages `icon.kicon` into
`package_inputs/` — those are intermediates, not authored by hand. Optional
blocks: `codegen` (per-app compiler flags), `maps` (`.map` validation and packaging),
`images` (`.kspr` → `.kim`), `audio` (staged/compiled Native KotoAudio),
`shell_icon` (launcher palette).

## 3. Build

```powershell
python harness\build_apps.py            # rebuild committed bytecode
python harness\build_apps.py --check    # fail on source/bytecode drift
```

`koto-compiler` runs `verify_kbc` on its output, so a successful build is also a
verified one. The `--check` mode is part of `harness/check_all.py`.

## 4. Run in KotoSim

Launch a specific app directly, without navigating the shell:

```powershell
cargo run -p koto-sim -- --app dev.koto.memo
cargo run -p koto-sim -- --app dev.koto.memo --app-script scenarios/memo.txt
```

The run prints a report: frames stepped, exit/yield result, document byte count,
and IME line state.

To capture what the app draws, add `--image PATH` to an `--app` run: KotoSim steps
the app (scripted or to exit) and writes the final frame — composited through the
same draw path window mode uses — to a BMP. Point `--app-script` at a script whose
last line is a yielded frame (not `exit`) to capture gameplay rather than the
cleared exit frame.

```powershell
cargo run -p koto-sim -- --app dev.koto.games.koto-blocks --app-script frame_capture.txt --image frame.bmp
```

### App input scripts

An app script drives one frame per line. Tokens:

- a single-quoted character: `'a'`, `'\n'`, `'\s'` (space);
- intent names: `shift convert commit cancel backspace delete left right up down
  home end newline save exit open`;
- `frame` for an empty frame; `#` starts a comment.

```text
'k'              # type k
'a'              # type a -> か
convert          # SKK convert
commit           # commit the candidate
save exit        # save and quit
```

This is the same [`VmInputSnapshot`](../spec/RUNTIME_BYTECODE_ABI.md) model that window mode
feeds the live VM, so a scripted scenario and hands-on play exercise identical
input.

## 5. Window mode

```powershell
cargo run -p koto-sim --features window -- --window
```

Arrow keys move/position the cursor, letters type, F1 toggles IME on/off, Tab
converts, Shift arms Sticky Shift, Right-Shift commits, Left Ctrl cancels, F2
opens the memo save prompt (empty Enter overwrites, a typed name saves as), F4
opens the memo file picker, and F5 immediately creates an unnamed empty memo.
F2 asks `上書き保存しますか? (y/n)` for named files; `n` and unnamed files open
Save As. F10 quits the app, and Esc quits the simulator. Backspace and Delete use
normal key repeat when held. The
window paints the VM's own draw output and routes keys into the running VM.

### Direct app launch and live reload (`--watch`, KOTO-0191)

`--window --app` skips the shell and starts inside the app; adding `--watch`
turns the window into a live-reload loop:

```powershell
cargo run -p koto-sim --features window -- --window --app dev.koto.games.kotorogue `
  --watch apps/kotorogue --watch-replay apps/kotorogue/scenarios/play.txt
```

Saving any file under the watched tree (sources, includes, `.kspr`, `.kmml`,
maps) rebuilds **just that app** through the registry build
(`harness/build_apps.py --app`, so images/maps/assets stay in sync) and
relaunches it in the same window — typically well under a second. The rebuild
runs on the window thread, so the frame pauses for that moment.

- A compile error does **not** end the loop: the running app keeps its last
  good build, the `file:line:col` diagnostic prints to the console (the same
  format the `$koto` problem matcher parses), and the next good save
  recovers.
- `--watch-replay PATH` replays an [app input script](#app-input-scripts)
  after every (re)launch, landing back at the scene being iterated on — no VM
  snapshotting, just the deterministic replay the scripted runs already use.
- Like all window-mode sessions, watch runs against `sdcard_mock` directly
  (rebuilding rewrites the committed bytecode, as a manual rebuild would);
  headless `--app` runs keep their throwaway-copy behavior.

## 6. Editor setup (VS Code)

The repo ships a declarative VS Code extension (KOTO-0190) with syntax
highlighting for `.koto`/`.kmml`/`.kspr` and a `$koto` problem matcher
that turns `koto-compiler` `file:line:col` diagnostics — including inside
`include`d files — into in-editor squiggles. Install it once by linking it
into your extensions directory and reloading the window:

```powershell
New-Item -ItemType Junction `
  -Path "$env:USERPROFILE\.vscode\extensions\koto.vscode-koto" `
  -Target "tools\vscode-koto"
```

See [tools/vscode-koto/README.md](../../tools/vscode-koto/README.md) for
details and uninstall.

With the extension active, the workspace tasks in `.vscode/tasks.json`
(Terminal → Run Task…) drive the loop above without leaving the editor:

- **Koto: build apps** (`Ctrl+Shift+B`) — `build_apps.py` with compile
  errors as diagnostics.
- **Koto: run current app (headless report)** / **Koto: screenshot current
  app** — resolve the app from the file you are editing (any file under
  `apps/<dir>/`, includes and assets too) via `harness/dev_app.py`;
  screenshots land in `target/koto-dev/<dir>.bmp`, driven by
  `apps/<dir>/scenarios/frame_capture.txt` when present.
- **Koto: watch current app (live reload)** — the KOTO-0191 loop above for
  the app you are editing; `apps/<dir>/scenarios/watch_replay.txt` is picked
  up automatically as the replay script when present.
- **Koto: sim window** — plain window mode (section 5).
- **Koto: check all** — the full local gate (section 9).

For `.koto` files, the extension also starts `koto-lsp` (KOTO-0194). Unsaved
changes receive compiler diagnostics after a 150 ms debounce; F12 follows
function/constant definitions across `include` files; hover shows function
signatures and slot footprints or constant values; and the first line shows a
`slots used/45` inlay with a warning at 90%. Set
`koto.languageServer.path` for a prebuilt server, or leave it empty to run the
workspace `koto-lsp` through Cargo.

The extension also ships asset editors and descriptor validation: open a
`.kspr` with *Open With… → Koto Sprite Editor* for a pixel-grid editor
(KOTO-0192); open a `.kicon` with *Koto Icon Editor* for a 40×40 mask editor
whose palette panel edits the sibling `app.json` `shell_icon`, or use the
**Koto: Open App Icon** button on an `app.json` (KOTO-0196). The adjacent
**Koto: Add App Resource** button selects an app-local `.kspr`, `.kmml`, or
`.kacl`, suggests its package output, and adds it to `images` or `audio`.
Maps remain manual because they require dimensions, glyphs, and source-marker
configuration. Use the play/stop editor-title buttons on
`.kmml` files to audition the native KotoAudio score (the KOTO-0188 CLI); and
`app.json` descriptors get schema completion and validation as you type. Both
asset editors edit the text format in place (byte-identical untouched saves,
one-line diffs per pixel). See
[tools/vscode-koto/README.md](../../tools/vscode-koto/README.md).

## 7. Diagnostics

When an app traps, the runner reports the app ID, the frame, the VM program counter,
and the VM error, for example:

```text
app dev.koto.memo trapped at frame 12 pc 87: DivisionByZero
```

Source-location mapping (bytecode PC → `main.koto` line) is added with bytecode
debug data in KOTO-0051.

### Runtime inspector

For state beyond a trap, add `--inspect` to a `--app` run to print a runtime
inspector snapshot from the final frame:

```powershell
cargo run -p koto-sim -- --app dev.koto.memo --app-script scenarios/memo.txt --inspect
```

```text
inspect dev.koto.memo frame=3 state=yielded pc=397 fuel=183 last_host_call=yield_frame error=<none> input(held=0x0 pressed=0x0 char=0 intent=0x0) open_files=0 draw_rects=1 draw_pixels=0 text_draws=2
```

It reports the VM run state, program counter, fuel consumed that frame, the last
host call, the last VM error, and the last input snapshot, plus host-side counts:
open sandboxed file handles and captured `draw_rect`/`draw_pixels`/`draw_text`
output. Handles
are reported as occupancy only — no host paths leave the sandbox model.

### Budget report

Where `--inspect` is the final frame, `--budget` is the whole run: it prints
per-app memory and frame-fuel high-water marks accumulated across every frame
(KOTO-0101), so a scripted run can validate the VM profile before device bring-up:

```powershell
cargo run -p koto-sim -- --app dev.koto.games.koto-blocks --app-script harness/fixtures/budget/koto_blocks.script --budget
```

```text
budget app=dev.koto.games.koto-blocks frames=81 stack_peak=7 stack_cap=16 call_peak=0 call_cap=4 local_peak=48 local_cap=48 heap_peak=3981 heap_request=3981 heap_budget=24576 fuel_peak=44277 fuel_cap=60000 host_calls_peak=153 open_files_peak=0 open_files_cap=8 draw_rects_peak=118 draw_pixels_peak=80 text_draws_peak=2 audio_events_peak=2
```

Each `*_peak` is paired with the capacity it must stay under: the SRAM-resident VM
state (`stack`/`local`/`heap`), the per-frame `fuel` budget, and host-owned working
sets (draw lists, file handles) whose pixel/PCM bytes never live in the VM heap.
`heap_peak` is the highest heap byte the VM addressed; `heap_request` is the KBC
heap it was given; `heap_budget` is the manifest's declared SRAM ceiling (`none`
if unset). `harness/check_budgets.py` runs the Memo and KotoBlocks scenarios under
this report, warns at >=90% of a fixed capacity, and fails when a peak exceeds its
configured threshold; it runs as part of `harness/check_all.py`.

### Local slot map

`local_peak` is the highest VM slot touched, which is 48 whenever a value-returning
function runs — the top three slots are codegen scratch (one is the return slot).
The actionable local number is the **user-slot** usage from the compiler
(KOTO-0102 / KOTO-0104):

```powershell
cargo run -p koto-compiler -- apps/koto_blocks/src/main.koto --slot-map
```

```text
slot-map user_slots_used=42 user_slots_cap=45 scratch_slots=3 vm_local_slots=48
fn pmid params=1 locals=0 footprint=1
fn shape params=1 locals=0 footprint=1
fn blit_piece params=4 locals=2 footprint=6
fn main params=0 locals=36 footprint=36
```

The compiler inlines every function but allocates each inline expansion's slots at
the **call site**, above the caller's live locals, and frees them when it ends
(KOTO-0104). So disjoint helpers reuse the same physical slots, and
`user_slots_used` is the real post-reuse peak (here 42, not the 44 the per-function
footprints sum to). Each `fn` line is that function's own footprint (`params +
peak_lets`, per-scope reuse from KOTO-0092) — a guide to which helpers are heavy to
inline; they no longer own fixed slot ranges. The budget gate reports
`user_slots_used` and warns as it nears the cap.

## 8. Save Data

Bytecode apps save through the sandboxed file host calls. A path such as
`memo.txt` for `dev.koto.memo` is stored under:

```text
sdcard_mock/data/dev.koto.memo/memo.txt
```

The app only sees its own virtual filename. The simulator management commands
report app IDs, file counts, and byte totals, but do not print host filesystem
paths:

```powershell
cargo run -p koto-sim -- --save-list
```

To reset one app before a scenario, clear its namespace:

```powershell
cargo run -p koto-sim -- --save-clear dev.koto.memo
```

The command validates the app ID with the same rules as package manifests and
refuses traversal-like values such as `../dev.koto.memo` or `dev/koto/memo`.
Direct `--app` runs still use a throwaway copy of `sdcard_mock`, so they do not
modify committed test state unless a separate save-data management command is
used.

## 9. Validate

`python harness\check_all.py` runs the whole local gate, including the app build
sync, the scripted memo validation, and golden frame validation, so a green run
means every committed app still builds, verifies, and behaves.

Golden frame validation compares `cargo run -q -p koto-sim -- --golden-frames`
with `harness/fixtures/golden_frames/sim.trace`. Update that fixture only for an
intentional shell layout, package list, or app first-frame rendering change; a
failure prints a unified diff for review.

Small SDK regression apps are listed in [KotoSDK Samples](../spec/SDK_SAMPLES.md). They
use the same registry, build command, bytecode verifier, manifests, and direct
KotoSim launch path as larger apps.
