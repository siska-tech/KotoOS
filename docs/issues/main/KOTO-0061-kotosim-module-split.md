# KOTO-0061: KotoSim Module Split

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-5, NFR-PORT-1, NFR-DEV-4

## Goal

Split `src/koto-sim/src/lib.rs` into focused modules without changing behavior,
so simulator work does not keep accumulating in one large file.

## Acceptance Criteria

- [x] Host filesystem code lives in a dedicated simulator module.
- [x] Runtime host/session code lives in dedicated simulator module(s).
- [x] Manifest/package loading, scenarios, save-data helpers, framebuffer/BMP,
  and golden-frame helpers are separated or intentionally grouped.
- [x] The remaining oversized runtime implementation is split behind a small
  `runtime/mod.rs` facade.
- [x] Public exports used by `koto-sim` CLI and tests remain stable or are
  updated mechanically.
- [x] `python harness\check_all.py` passes.

## Notes

Prefer mechanical moves first. Do not combine this with functional runtime or
window-mode changes.

## Resolution

The first KOTO-0061 pass made the crate root a small public facade:
`host_fs.rs` owns host filesystem adaptation, `manifest.rs` owns host-side
manifest parsing, and `framebuffer.rs` owns the PC framebuffer, BMP, and render
recorder helpers. The remaining runtime implementation was initially kept
together to avoid mixing the mechanical crate-root split with a functional
rewrite.

The follow-up pass replaced that oversized runtime module with the small
`runtime/mod.rs` facade and these focused submodules:

- `audio_capture.rs`: deterministic scripted audio capture.
- `budget.rs`: app runtime budget reporting.
- `error.rs`: simulator errors, profile constants, and shared runtime constants.
- `host.rs`: VM host implementation and host-call dispatch.
- `inspector.rs`: runtime inspector reports and formatting.
- `memo_validation.rs`: memo end-to-end validation helpers.
- `orchestration.rs`: golden-frame and shell input orchestration.
- `package.rs`: package discovery, launch loading, and launch reports.
- `render.rs`: app-session painting and frame rendering.
- `save_data.rs`: save-data listing, clearing, and reporting.
- `scenario.rs`: scripted app scenarios and diagnostics.
- `session.rs`: bytecode app session lifecycle and stepping.
- `shell_prefs.rs`: shell preference persistence.
- `tests.rs`: mechanically moved runtime test suite.

Existing crate-root exports remain available to the CLI and window backend.
The split changes module paths and internal visibility only; simulator behavior,
host ABI behavior, rendering, audio, golden-frame output, memo validation, and
budget output formats are unchanged.

Verified on June 18, 2026:

- `cargo fmt --check`
- `cargo test -p koto-sim` (75 passed)
- `cargo check -p koto-sim --features window --all-targets`
- `python harness/check_golden_frames.py`
- `python harness/check_budgets.py`
- `python harness/check_project.py`
- `python harness/check_all.py`

All checks pass.
