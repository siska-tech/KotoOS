# GFX-0005: Display service extraction (present-as-request)

- Status: done
- Type: refactor
- Priority: P1
- Requirements: NFR-PORT-2, NFR-DRAW-2, NFR-PERF-1

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 5, [design/KOTOOS_RESOURCE_OWNERSHIP.md](../../design/KOTOOS_RESOURCE_OWNERSHIP.md)
§4 (Display Ownership) + §3 (CPU0/CPU1).

Depends on: [GFX-0004](GFX-0004-rasterizer-compositor-into-koto-gfx.md).

## Goal

Interpose a display-service seam so the frame loop issues a **present request**
instead of inlining compose+flush to the live panel — realising the resource-
ownership §4 flow (request → damage merge → overlay/status-bar composite →
decide → coalesce → compose → flush). Today there is no place to insert a status
bar or a system-menu takeover; this creates that seam.

## Acceptance Criteria

- [x] A `present(request, &mut lcd, font, strip, scratch, &mut metrics)` entry —
      [`DisplayService::present`](../../../src/koto-pico/src/firmware/display_service.rs)
      — runs the §4 flow: receive request → collect (GFX-0003) → no-op overlay hook
      → decide (`FullRepaintPolicy`) → coalesce (GFX-0003) → compose (GFX-0004) →
      flush (HAL). `frame_inputs` is the new `PresentRequest` POD; the app surface is
      the fixed 320x320 panel everywhere on this path, so it stays implicit rather
      than a new param (no surface registry built here). The collect→…→flush body is
      **unchanged** — it still lives in `present_app_delta` / `present_app_commands`,
      which the service calls, so the seam is byte-equivalent.
- [x] The overlay / status-bar composite is a **no-op placeholder**
      (`DisplayService::composite_system_overlay`, empty body; system-app-only
      capability per §9, not built here) — present routes through the service
      boundary, no longer a straight shot to the LCD.
- [x] The frame loop (`app_runtime`) builds a `PresentRequest` and calls the
      service; it keeps only the present *trigger* (R11:
      `!has_previous_draw || *host.draw != *previous_draw || static_rebuilt` and the
      `previous_draw` copy-back) and no longer owns the delta-vs-first-frame
      compose/flush selection — that moved into the service.
- [x] `PaintMetrics` and `phase=160/164/161` stay in firmware (`diag` /
      `app_runtime`); `&mut metrics` is threaded through the service unchanged, so
      the field set and values are identical (logging untouched).
- [x] The present trigger fires on the same frames (gate unchanged); `koto-sim`
      golden frames 13/13 (KotoBlocks / KotoSnake / Sokoban + units) pass unchanged;
      `koto-gfx` 80 + `koto-core` 132 pass; `thumbv6m` firmware build green;
      `build_apps.py --check` OK; firmware-lib clippy adds no new finding
      (`display_service`/`app_runtime`/`app_render` clean; the pre-existing
      `probe_keyboard` bin lint is untouched).
- [ ] **Hardware-only, unverified on the authoring host (no device):** the
      `phase=160/163/164` before/after compare on a device — same posture as
      GFX-0001..0004. Diff those lines on hardware before treating as fully closed;
      the change is byte-equivalent by construction (present body untouched, only the
      call site wrapped), so no field or value drift is expected.

## Notes

**No CPU0/CPU1 re-homing in this issue.** §3 names a "Display-owned CPU1"
candidate and a CPU1 render-prep worker already exists
([KOTO-0147](../main/KOTO-0147-pico-cpu1-render-prep-worker.md)); the current split
is kept and only the boundary is localised. The service runs inline on the caller's
core exactly as the old call site did, and is held by the frame loop across the
session, so a future pin to `system_service_core` is a small, localised change.
Building overlays / status bar / takeover / capture / an async present queue /
a surface registry is separate, later work — only the seam (and its no-op overlay
hook) is created here.

`cargo test -p koto-pico` cannot run on the host (embassy-rp is ARM-only — the
limitation noted since GFX-0001); the `app_render` invariant tests compile into the
firmware but execute only on a device. The `thumbv6m` build is the host-side gate
for the firmware path, and `koto-sim`'s `fixture_runner` golden frames are the
host-side visual-parity gate for the apps (the firmware-only seam does not touch
them).
