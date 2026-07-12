# KOTO-0122: Pico Shell Package Metadata And Icons

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-1, FR-FS-3, FR-SDK-1, NFR-MEM-2, NFR-DRAW-1

## Goal

Show the same package names, descriptions, categories, icon themes, and icon
assets on PicoCalc that KotoSim uses for KotoShell.

## Acceptance Criteria

- [x] Device manifest parsing validates all shell-visible metadata through the
  portable manifest model.
- [x] `.kicon` assets and manifest-driven icon themes render in the launcher.
- [x] Asset reads are bounded and sequential, with explicit SRAM/PSRAM cache
  budgets.
- [x] Missing or invalid icons use the same deterministic fallback icon policy
  as KotoSim.
- [x] Japanese package names and descriptions render with the shared font and
  clipping/wrapping behavior.

## Notes

Depends on KOTO-0121 and the dirty-region transport from KOTO-0120.

## Resolution

The portable shell already renders names, descriptions, categories, Japanese
text (shared M+ font with the same clip/wrap helpers), and the deterministic
`icon_kind_for` fallback. This issue adds the two missing pieces on device:

- **Icon themes.** `parse_shell_icon_theme` parses the manifest `shell_icon`
  object in `no_std` firmware, reusing the flat `json_string` scanner (the theme
  keys are unique within a manifest) and converting `#RRGGBB` to RGB565 with the
  same arithmetic as KotoSim's `required_rgb565`. The theme now rides in on the
  `ManifestFields` summary so manifest parsing validates all shell-visible
  metadata through the portable model. Host-style unit tests cover the parser
  and hex conversion (the `koto-pico` crate cannot run them on the host because
  `embassy-rp` is unconditional and ARM-only; the parser mirrors the validated
  sim path and is exercised by the target build).
- **`.kicon` assets.** After the manifest pass, `load_packages` opens `ICONS/`,
  and for each package with an `icon` path matches the asset's long name to its
  8.3 short name, reads it sequentially into a shared 2 KiB static `KICON_SCRATCH`
  buffer (explicit budget; no full-asset SRAM/PSRAM cache), parses it with
  `PackageIcon::from_kicon_text`, and attaches it via `set_icon`. A new
  `phase=140 icons-loaded count=N` UART line reports the result. Missing `ICONS/`
  dir, unmatched name, oversized, or malformed assets are skipped, leaving the
  deterministic fallback icon.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline -- -D warnings`
- `python harness/check_project.py`

### Hardware capture (2026-06-22) — all criteria met

`phase=139 manifest-read-done accepted=15` was followed by
`phase=140 icons-loaded count=15`: every package's `.kicon` matched and loaded.
The launcher rendered the same themed asset icons as KotoSim (visually
confirmed). The first full redraw's raster rose to ~92 ms with 15 themed asset
icons to blit (vs ~60 ms for the fallback set); this is the full-screen path,
not the same-page selection path KOTO-0120 holds to 33 ms.
