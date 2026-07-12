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
- `sdcard_mock/apps` contains at least one simulator package manifest.
- The initial Cargo workspace and Rust crate entry points exist.
- The asset pipeline fixture converts a PBM icon, validates a `.kfont` Japanese
  preview, and verifies generated asset placement.
- Golden frame validation compares a stable KotoSim trace for the shell launcher
  and the SDK hello-text app against `fixtures/golden_frames/sim.trace`.

The harness is deliberately small. As KotoSim and the package toolchain appear, add checks here before adding heavier test infrastructure.

## App build loop

`build_apps.py` compiles the source-authored apps listed in `apps/apps.json` into
their committed `sdcard_mock` bytecode. Each app declares a `kind`: `koto`
(high-level source via `koto-compiler`) or `asm` (low-level IR via `kbc-asm`).

```powershell
python harness\build_apps.py            # rebuild committed bytecode
python harness\build_apps.py --check    # fail if committed bytecode is stale
```

`--check` is part of `check_all.py`, so source/bytecode drift or a manifest entry
that does not point at the built output fails the local checks.

## Local checks

`check_all.py` runs the full local gate: `cargo fmt --check`, Clippy, `cargo test`,
the app build sync (`build_apps.py --check`), the scripted memo validation, golden
frame validation, the runtime budget gate, and `check_project.py`.

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
