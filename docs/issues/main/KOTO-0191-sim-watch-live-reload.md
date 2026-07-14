# KOTO-0191: koto-sim --watch — live reload on source/asset change

- Status: done (2026-07-13) — `--window --app` direct launch (the KOTO-0190
  deferral) plus `--watch DIR` / `--watch-replay PATH` landed; rebuilds go
  through the new `build_apps.py --app` single-app filter; VS Code task
  "Koto: watch current app" wires it to the edited file via `dev_app.py`.
- Type: feature
- Priority: P3
- Related: KOTO-0184 (audio/gfx dev tooling — the "live-reload loop"
  candidate), KOTO-0049 (sim app dev experience), KotoIDE roadmap Phase 2b
  (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

The edit loop today is: edit → run `build_apps.py` → relaunch
`koto-sim --window`. Add `--watch` to a windowed app run: when the app's
sources or assets (`.koto` includes, `.kspr`, `.kmml`, maps) change on disk,
rebuild that app and relaunch it in the same window.

## Design notes

- **Rebuild scope is one app.** Reuse the `apps.json` entry to know the
  source root, `images`/`maps`/`assets`/`audio` inputs, and outputs; don't
  rerun the whole `build_apps.py` sweep.
- **Returning to the scene.** The VM is deterministic and window input already
  shares the `VmInputSnapshot` model with `--app-script`. An optional
  `--watch-replay <script>` replays a script after each relaunch to land back
  at the scene being iterated on — no VM snapshotting, no new mechanism.
- **State hygiene.** Direct `--app` runs use a throwaway `sdcard_mock` copy;
  `--watch` must keep that property across relaunches (fresh copy per
  relaunch, or document the deviation).
- Compile errors must not kill the loop: show the diagnostic (window overlay
  or console) and keep watching.
- File watching: mtime polling at ~250 ms is fine and dependency-free; a
  notify crate is acceptable if kept behind the `window` feature.

## Acceptance Criteria

- [x] Editing and saving `main.koto` (or an included file, `.kspr`, `.kmml`)
      refreshes the running window with the rebuilt app in ~1 s.
      → the whole `apps/<dir>` tree is mtime-polled every ~250 ms, so
      includes/sprites/audio/maps all trigger; smoke run on KotoRogue:
      touch → "rebuilt dev.koto.games.kotorogue in 266 ms; relaunching".
      The rebuild is `build_apps.py --app <id>` (new single-app filter), so
      images/maps/assets rebuild with the bytecode.
- [x] A compile error is reported without exiting; the next good save
      recovers.
      → smoke run: a syntax error printed
      `apps/kotorogue/src/main.koto:556:12: expected an identifier…` (the
      `$koto` matcher format) + "build failed; keeping the running app";
      restoring the source rebuilt and relaunched in 248 ms.
- [x] `--watch-replay` returns to a scripted scene after relaunch.
      → replays through the same `parse_app_script` + `step_frame` path as
      scripted validation on every (re)launch; a replay trap surfaces like a
      live trap. Smoke-verified with `scenarios/play.txt` (no replay errors,
      relaunch clean).
- [x] `docs/guides/APP_DEV_LOOP.md` documents the watch loop.
      → new "Direct app launch and live reload" subsection under Window mode,
      plus the "Koto: watch current app" task in the editor-setup section
      (auto-picks `scenarios/watch_replay.txt`). Documented deviation: watch
      runs against `sdcard_mock` directly, like every window-mode session —
      the rebuild rewrites committed bytecode exactly as a manual rebuild
      does.
