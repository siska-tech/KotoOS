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

The command creates `apps/todo_list/src/main.koto`, a smoke scenario under
`apps/todo_list/scenarios/`, a `.kpa.json` manifest, an icon placeholder, and an
`apps/apps.json` entry. It validates the generated app ID, display name, runtime,
entry path, and icon path through the same package manifest rules used by the
runtime and packer.

## 2. Register the app

Add an entry to [`apps/apps.json`](../../apps/apps.json):

```json
{ "app_id": "dev.koto.memo", "kind": "koto",
  "source": "apps/memo/src/main.koto",
  "output": "sdcard_mock/bytecode/memo.kbc",
  "manifest": "sdcard_mock/apps/memo.kpa.json" }
```

`kind` is `koto` (compiled by `koto-compiler`) or `asm` (assembled by `kbc-asm`).

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

## 6. Diagnostics

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

## 7. Save Data

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

## 8. Validate

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
