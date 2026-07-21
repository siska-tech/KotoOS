# KotoOS Harness

This directory contains dependency-free checks that can run before implementation code exists.

Run from the repository root:

```powershell
python harness\check_project.py
```

Current checks:

- Requirement IDs are unique across Markdown documents.
- Local Markdown links point to existing files.
- Repository-local issues have unique IDs and valid status values.
- `harness/fixtures/sample_app.kpa.json` has the expected package metadata shape.
- `harness/fixtures/sample_app.layout.csv` records asset offsets in monotonic
  read order, and the harness rejects a non-monotonic layout fixture.
- `sdcard_mock/apps` contains reproducible binary `.kpa` packages (the adjacent
  `.kpa.json` files are packer inputs, not runtime files).
- The initial Cargo workspace and Rust crate entry points exist.
- The asset pipeline fixture converts a PBM icon, validates a `.kfont` Japanese
  preview, and verifies generated asset placement.
- Golden frame validation compares a stable KotoSim trace for the shell launcher
  and the SDK hello-text app against `fixtures/golden_frames/sim.trace`.

The harness is deliberately small. As KotoSim and the package toolchain appear, add checks here before adding heavier test infrastructure.

## App build loop

`build_apps.py` discovers each app from its own `apps/<dir>/app.json` descriptor
(KOTO-0195), compiles its source into committed bytecode, generates the
packaging manifest, and packs every declared asset — including Native KotoAudio
KMML — into a binary `APPS/*.kpa` archive in the SD-card tree. Each app declares
a `kind`: `koto` (high-level source via `koto-compiler`) or `asm` (low-level IR
via `kbc-asm`).

```powershell
python harness\build_apps.py            # rebuild committed bytecode
python harness\build_apps.py --check    # fail if committed bytecode is stale
```

`--check` is part of `check_all.py`, so source/bytecode/SD-asset drift or a
manifest entry that does not point at the built output fails the local checks.

## Local checks

`check_all.py` runs the full local gate: `cargo fmt --check`, Clippy, `cargo test`,
the app build sync (`build_apps.py --check`), the scripted memo validation, golden
frame validation, the runtime budget gate, and `check_project.py`.

Embedded targets are an explicit cross-build gate because they require Rust
targets that host-only contributors may not have installed:

```powershell
rustup target add thumbv6m-none-eabi thumbv8m.main-none-eabihf
python harness\check_embedded.py
```

This checks every retained `koto-pico` binary for the default RP2040 profile
and the RP2350A/Pico 2 W profile (KOTO-0204).

## Runtime budget gate

`check_budgets.py` (KOTO-0101) runs each app's worst-ish scripted scenario in
`fixtures/budget/` through `koto-sim --budget`, parses the per-app memory/fuel
high-water report, warns when a VM peak reaches >=90% of its fixed profile
capacity (operand stack, call depth, local slots, frame fuel) or the heap request
nears the declared SRAM budget, and fails when a tracked peak exceeds the
per-scenario threshold. Thresholds carry headroom over today's measurements so the
gate is green now but catches regressions.

```powershell
python harness\check_budgets.py
```

## Audio residency memory report

`check_audio_residency_memory.py` (KOTO-0227) reads a Pico W release ELF with
`rust-size` and `rust-nm`, checks the exact 36 KiB switchable arena and named
permanent-stream symbols, and writes a machine-readable per-mode SRAM report.
The report also describes KOTO-0245's RP2040 `TlsExclusive` mode: it retains the
8 KiB CPU1 stack and loans the 8,192-byte PCM sample region to one HTTPS future.
Ordinary Wi-Fi remains in `WifiStreamAudio` with audio available.
Build the explicit Pico W profile first:

```powershell
cargo build --release -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --no-default-features --features board-picocalc-picow,ram_interpreter,ram_audio_mixer
python harness\check_audio_residency_memory.py --elf target\thumbv6m-none-eabi\release\koto_firmware
```

Hardware captures can populate the otherwise explicit `null` margins:

```powershell
python harness\check_audio_residency_memory.py --cpu0-free-min 15724 --cpu1-stack-free-min 4096
```

`check_all.py` runs the dependency-free parser self-test; the real-ELF command
remains an embedded release gate because it requires the cross-built artifact.

`check_wifi_residency_layout.py` measures the concrete CYW43 0.7 Pico W target
types from the dedicated probe ELF and gates the residual runner/network budget:

```powershell
cargo build --release -p koto-pico --bin probe_wifi_residency --target thumbv6m-none-eabi --no-default-features --features board-picocalc-picow,ram_interpreter,ram_audio_mixer
python harness\check_wifi_residency_layout.py --elf target\thumbv6m-none-eabi\release\probe_wifi_residency
```

The report is written to `target/koto-dev/wifi_residency_layout.json`. The
parser self-test runs in `check_all.py`; the real layout gate requires the
cross-built probe ELF and `rust-nm`.

`check_app_fetch_budget.py` measures KOTO-0245's target-layout portable control
plane independently of the future DNS/TCP/TLS backend. The RP2040 probe freezes
the service slots, four-origin allowlist, two-pin-per-origin table, and
streaming HTTP decoder below a 3 KiB ceiling:

```powershell
cargo build --release -p koto-pico --bin probe_app_fetch_service --target thumbv6m-none-eabi
python harness\check_app_fetch_budget.py
```

The machine-readable report is written to
`target/koto-dev/app_fetch_budget.json`; `check_all.py` runs its dependency-free
parser self-test.

The report also measures the caller-owned single-request transport mailbox at
596 bytes under a separate 640-byte ceiling. `WifiRuntime` adds a four-byte
synchronization view plus the one-word TLS/audio exclusion coordinator and
places the 604-byte slot in the existing CYW43 driver reservation (4 bytes
remain), not product-static SRAM;
the Wi-Fi layout gate freezes that placement independently.

`check_app_fetch_tls_feasibility.py` combines that control-plane report with
the measured Wi-Fi arena and NetworkService reports. It records why ordinary
16 KiB-duplex and smaller 8 KiB/2 KiB TLS buffer profiles do not fit RP2040,
and admits only a narrowly bounded 4 KiB/1 KiB probe after both reclaiming one
socket window and proving that the HTTP decoder can overlay handshake storage:

```powershell
python harness\check_app_fetch_tls_feasibility.py
```

The resulting `target/koto-dev/app_fetch_tls_feasibility.json` is a preflight
envelope, not proof that a TLS implementation fits. A selected candidate must
still measure its exact connection/future state, peak SRAM, and linked flash.

The isolated `embedded-tls` probe has a type-only baseline and a handshake
variant so SRAM and incremental flash remain reproducible without enabling TLS
in any product feature:

```powershell
cargo build --release -p koto-pico --bin probe_app_fetch_tls --target thumbv6m-none-eabi --target-dir target/app-fetch-tls-base --no-default-features --features board-picocalc-picow,app_fetch_tls_probe
cargo build --release -p koto-pico --bin probe_app_fetch_tls --target thumbv6m-none-eabi --target-dir target/app-fetch-tls-handshake --no-default-features --features board-picocalc-picow,app_fetch_tls_handshake_probe
cargo build --release -p koto-pico --bin probe_app_fetch_tls --target thumbv6m-none-eabi --target-dir target/app-fetch-tls-adapter --no-default-features --features board-picocalc-picow,app_fetch_tls_socket_adapter_probe
cargo build --release -p koto-pico --bin probe_app_fetch_tls --target thumbv6m-none-eabi --target-dir target/app-fetch-tls-verifier --no-default-features --features board-picocalc-picow,app_fetch_tls_verifier_probe
python harness\check_app_fetch_tls_probe.py
```

`app_fetch_tls_probe` and `app_fetch_tls_handshake_probe` are dependency islands;
no board or product feature enables either. Concurrent stream audio is rejected:
the 7,528-byte handshake task leaves only 2,690 bytes in that arena plan. The
selected RP2040 policy instead excludes audio only for the TLS connection and
uses the implemented 8,192-byte PCM workspace while retaining the CPU1 stack,
scratch metadata, and DMA ring. The production-verifier task is 7,744 bytes,
leaving 448 bytes inside the workspace and 6,518 bytes across the admitted
RP2040 envelope. The socket
compatibility layer adds 120 bytes of `.text`, 8 bytes of `.rodata`, and 16
bytes of `.bss`; SPKI hashing and P-256 CertificateVerify add another 24,716
bytes of `.text`, 312 bytes of `.rodata`, and 200 bytes of `.bss`. A host
known-answer test covers valid, altered-transcript, and malformed-signature
cases. Live socket/endpoint validation remains excluded, so this is still a
continuation gate rather than product approval.

`check_arena_future.py` compiles and runs the dependency-free host regression
for the type-erased future slot used by the switchable arena. It proves pending
cancellation and normal completion both drop the concrete future exactly once
before storage reuse, and rejects a future larger than its supplied byte slice.

## Golden frame validation

`check_golden_frames.py` runs:

```powershell
cargo run -q -p koto-sim -- --golden-frames
```

and compares stdout with `harness/fixtures/golden_frames/sim.trace`. The trace is
intentionally text-based: it records the shell list render rectangles and one
app frame's draw calls, so reviews stay small while still catching UI and
render-command regressions.

Update `sim.trace` only when an intentional shell layout, package list, or app
first-frame rendering change is made. On failure, the checker prints a unified
diff; inspect the changed rectangles/text first, then regenerate the fixture from
the command above if the new output is the desired behavior.

## KotoUI component gallery

The gallery is a simulator-only developer surface covering Label, Button,
Checkbox, List, TextField, Panel, and modal Dialog. It is compiled into
`koto-sim`; it has no app descriptor or KPA archive and therefore never appears
in the shipped shell catalog.

Render the default 320x320 gallery to a BMP:

```powershell
cargo run -p koto-sim -- --ui-gallery --image C:\tmp\koto-ui-gallery.bmp
```

Run it interactively with the opt-in window backend:

```powershell
cargo run -p koto-sim --features window -- --ui-gallery --window
```

Arrow keys and Tab navigate, Enter activates, typing edits the focused field,
Backspace/Delete/Home/End edit it, F1 toggles the borrowed IME composition
example, Left Ctrl cancels a modal, and F10 exits. These keys are converted to
the same semantic `UiEvent` actions used by KotoOS controls.

`cargo test -p koto-sim --test ui_gallery` runs the deterministic interaction
trace, exact damage assertions, repeated-idle proof, bounded-capacity failures,
and RGB565 golden hashes for the default theme and modal backdrop. When a
component/theme change intentionally alters a golden, inspect an emitted BMP
first, run the test with `-- --nocapture`, and update both hash constants in
`src/koto-sim/tests/ui_gallery.rs` in the same reviewed change.

## Font conversion

`mplus_to_kfont.py` converts M+ BITMAP FONTS (BDF) into the compact `.kfont`
blobs read by `koto_core::font`. The vendored assets in `assets/fonts/` are
regenerated from the upstream M+ source archive:

```powershell
# extract xfonts-mplus_2.2.4.orig.tar.xz somewhere, then:
python harness\mplus_to_kfont.py --src <extracted-dir> --out assets\fonts
```

This produces `mplus10.kfont` / `mplus12.kfont` (full JIS X 0208) and copies the
M+ license files. M+ BITMAP FONTS are freely redistributable; see
`assets/fonts/LICENSE_E` and `assets/fonts/LICENSE_J`.

## Asset pipeline

`asset_pipeline.py` is the first KOTO-0054 host-side media pipeline prototype.
It can convert a 40x40 ASCII PBM launcher icon into `KICON1`, write a PBM
preview, validate `.kfont` metrics by rendering Japanese sample text, and verify
that generated assets match a KPA layout report.

```powershell
python harness\asset_pipeline.py convert-icon --src harness\fixtures\asset_pipeline\icon_40.pbm --out harness\fixtures\asset_pipeline\package_assets\icons\pipeline.kicon --preview harness\fixtures\asset_pipeline\package_assets\previews\pipeline_icon.pbm
python harness\asset_pipeline.py font-preview --font assets\fonts\mplus10.kfont --sample Koto日本語 --out harness\fixtures\asset_pipeline\font_preview.txt
python harness\asset_pipeline.py verify-layout --manifest harness\fixtures\asset_pipeline\asset_pipeline.kpa.json --layout harness\fixtures\asset_pipeline\asset_pipeline.layout.csv
```

The design contract is documented in `docs/ASSET_PIPELINE.md`.
