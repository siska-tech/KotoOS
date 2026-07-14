# KotoIDE Roadmap

"KotoIDE" is not a single application: it is the plan for turning the current
CLI-first development loop into an integrated experience covering **code,
graphics, and sound** — authoring, conversion, audition/preview, and execution.
This document is also the design note requested by
[KOTO-0184](../issues/main/KOTO-0184-audio-gfx-dev-tooling.md) (candidate
tooling ranked by iteration-time saved; see the ranking table below).

## Current State

The parts of an IDE already exist as CLI tools and library code:

- **Code loop.** Scaffold → `apps.json` → `harness/build_apps.py` →
  `koto-sim --app / --window / --image / --app-script / --inspect / --budget`,
  plus `koto-compiler --slot-map` ([App Dev Loop](../guides/APP_DEV_LOOP.md)).
- **Diagnostics.** Every compiler diagnostic reports real `file:line:col`
  across include boundaries via the SourceMap (KOTO-0183), and KDBG carries
  line/col for runtime PC attribution.
- **Graphics assets.** `.kspr` ASCII sprite sheets compile to `KIM1` strips in
  `build_apps.py`; icons come from hand-written PBM via
  `harness/asset_pipeline.py`; fonts from BDF; tilemaps from `.map`
  ([Asset Pipeline](../guides/ASSET_PIPELINE.md)).
- **Audio.** The device runtime (koto-audio, KOTO-0165/KOTO-0180) plays PCM16
  mono KACL clips, and the host converters already exist in Rust:
  `koto-audio-convert` (WAV → KACL with downmix/resample/SNR report),
  `koto-audio-decode` (KACL → WAV listening), `koto-audio-drum-table`.
  `.kmml` sequences feed `koto-audio-gen` cue tables; PCM16 and SLD4 KACL
  clips share one bounded package playback path on SIM and Pico.
- **Execution.** KotoSim window mode (minifb + cpal), deterministic
  `--app-script` replay, `--image` screenshots, golden-frame validation.

## The Gap

What is missing is not compilers or runtimes — it is the **authoring paths and
feedback speed** around them:

1. **No path from standard art tools.** Sprite art is typed as ASCII, one
   character per pixel; there is no PNG import, and previewing means launching
   an app with `--image`.
2. **No audition.** Hearing a `.kmml` edit requires wiring it into an app and
   launching the sim; there is no "play this file now".
3. **Clip codec parity must be visible.** PCM16 and SLD4 assets need an
   explicit package demo and the same runtime behavior on SIM and Pico.
4. **Converters are scattered.** Icon/font tools are Python in `harness/`,
   audio tools are Rust in `koto-audio-tools`, sprite compilation is inside
   `build_apps.py` — there is no one documented toolset.
5. **Editor integration is zero.** No syntax highlighting, no in-editor
   diagnostics, no build/run tasks, no slot/budget visibility while typing.

## Design Principles

1. **CLI/library first.** Every capability lands as a reproducible,
   harness-testable converter or renderer. GUI layers are thin skins over the
   same Rust code, so `check_all.py` can gate everything and a future
   standalone IDE could reuse it all.
2. **Text formats stay the source of truth.** `.kspr`, `.kmml`, and
   map `.map` remain the committed, diffable sources; binaries (`.kim`,
   `.kacl`, `.kbc`) stay generated. Import tools *produce* text sources, they
   do not replace them.
3. **Editor integration goes through VS Code.** Grammars, tasks, webview
   custom editors, and (later) an LSP client — with all intelligence in Rust,
   not TypeScript.
4. **Device parity is visible.** Audio audition should render through the
   koto-audio pipeline where possible so what you hear is what the device
   plays; Native KotoAudio phrases can be baked to PCM16 KACL, and WAV sources
   can be converted explicitly to PCM16 or SLD4.

## Architecture: Three Layers

| Layer | Contents | Value on its own |
| :---- | :------- | :--------------- |
| 1. Conversion toolset (CLI) | `koto-img` (PNG ↔ `.kspr`, `.kim` → PNG), `.kmml` audition/WAV render, PCM16/SLD4 KACL conversion | Author in Aseprite/a DAW, hear edits instantly, ship clips to device |
| 2. Editor integration | VS Code extension: grammars, build/run/screenshot tasks, problem matcher, sim live-reload, `.kspr`/`.kmml` custom editors | One-window workflow; save → see/hear |
| 3. Language intelligence | Compiler library split + structured diagnostics, `koto-lsp` (live diagnostics, include-aware definitions, slot/budget inlays) | Errors and budget pressure while typing, not at build time |

Layer 1 items are independent of each other and of Layers 2–3. Layer 2 items
consume Layer 1 CLIs. Layer 3 is deliberately last: SourceMap diagnostics
already work at build time, so the marginal pain there is smaller than on the
asset side.

## Implementation Path

| Phase | Issue | Outcome |
| :---- | :---- | :------ |
| 1a | [KOTO-0187](../issues/main/KOTO-0187-koto-img-sprite-converter.md) | `koto-img`: PNG → `.kspr` and `.kspr`/`.kim` → PNG, round-trip stable. Pixel art becomes authorable in any paint tool. |
| 1b | [KOTO-0188](../issues/main/KOTO-0188-kmml-audition-cli.md) | `.kmml` audition: render to WAV and play immediately on the host, device-parity voices. |
| 1c | [KOTO-0189](../issues/main/KOTO-0189-kwt-pcm16-bake.md) | Bake Native KotoAudio phrases to PCM16 KACL and package PCM16/SLD4 clips through one runtime path. |
| 2a | [KOTO-0190](../issues/main/KOTO-0190-vscode-extension-foundation.md) | VS Code extension: grammars for `.koto`/`.kmml`/`.kspr`, build/run/screenshot tasks, compiler problem matcher. |
| 2b | [KOTO-0191](../issues/main/KOTO-0191-sim-watch-live-reload.md) | `koto-sim --watch`: save a source/asset, the sim window rebuilds and relaunches (optionally replaying a script back to the scene). |
| 2c | [KOTO-0192](../issues/main/KOTO-0192-asset-custom-editors.md) | Webview custom editors: pixel-grid `.kspr` editor and `.kmml` play-button integration, saving the same text formats. |
| 3a | [KOTO-0193](../issues/main/KOTO-0193-compiler-library-diagnostics.md) | `koto-compiler` splits into a library with structured (typed, positioned) diagnostics; CLI unchanged. |
| 3b | [KOTO-0194](../issues/main/KOTO-0194-koto-lsp.md) | `koto-lsp`: live diagnostics, include-aware go-to-definition, hover, slot-map/budget inlays near the 45-slot cap. |

Phases 1a–1c are independent and each is a small, self-contained issue.
Recommended order within Phase 1: **1a → 1b → 1c** (art import removes the
most friction; audio already has a head start via `koto-audio-tools`).

## KOTO-0184 Candidate Ranking

Ranked by iteration-time saved per unit of effort:

| Rank | KOTO-0184 candidate | Disposition |
| :--- | :------------------ | :---------- |
| 1 | (implied) PNG import for sprite art | KOTO-0187 — biggest single accelerator; hand-typed ASCII art is the slowest loop today. |
| 2 | `.kmml` preview player | KOTO-0188. |
| 3 | Live-reload loop | KOTO-0191 — multiplies the value of every other tool. |
| 4 | KACL codec parity made visible | KOTO-0189 provides PCM16/SLD4 package playback; the cue-table dry-run report stays in KOTO-0184. |
| 5 | Sprite/tile sheet previewer | Subsumed by KOTO-0187's `.kspr`/`.kim` → PNG direction. |
| 6 | Cue-table dry-run | Remains in KOTO-0184 (small, but saves less time than the above). |
| 7 | Retained-layer inspector | Remains in KOTO-0184 — valuable for render debugging, but KOTO-0185's fixture-runner peak asserts already cover the budget-overflow failure mode. |

## Out of Scope (for now)

- **A standalone desktop IDE** (egui shell embedding koto-sim). Revisit only
  if the VS Code integration proves insufficient; principle 1 keeps that door
  open by keeping all logic in Rust libraries.
- **On-device editors** (sprite/MML editors as Koto apps). A separate
  dogfooding idea — the asset formats are text, so it is feasible, but the
  compiler stays host-side by design and this roadmap targets the PC loop.
- **MIDI → `.kmml`**, a tilemap editor, and font tooling upgrades: natural
  follow-ups once Phase 2 exists.
- **Consolidating the Python converters** (icon/font) into `koto-img`: decide
  when KOTO-0187 lands; not a prerequisite.
