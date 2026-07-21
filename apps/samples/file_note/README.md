# SDK File Note

`dev.koto.samples.file-note` is the first existing application migrated from
per-frame immediate drawing to the app-facing KotoUI ABI (KOTO-0221). It keeps
the original sample's identity and sandboxed file behavior — first-run creation
of `note.txt` with `saved from SDK sample`, read-back, and the F10 exit route —
while replacing the repeated full-screen `draw_rect`/`draw_text` loop with one
retained form: a bounded note TextField (64 bytes), Save and Reload buttons,
and a status Label. The app owns the note bytes; KotoUI owns pixels, focus,
input routing, and damage.

Build and launch:

```powershell
python harness/build_apps.py --app dev.koto.samples.file-note
cargo run -p koto-sim -- --app dev.koto.samples.file-note
```

The field holds focus initially. Activate it to edit; the first change enables
the previously disabled Save button. Save writes exactly the current UTF-8
bytes and disables itself again; Reload reads the sandboxed file back through
the ABI and reports missing, oversized (>64 bytes), invalid-UTF-8, and normal
content deterministically without ever discarding unsaved text on failure.
Cancel (outside editing) and the F10/Shift+F5 lifecycle intent both exit.

Run the deterministic interaction and print the runtime budget:

```powershell
cargo run -p koto-sim -- --app dev.koto.samples.file-note `
  --app-script apps/samples/file_note/scenarios/interaction.txt `
  --inspect --budget
```

The scenario edits then reloads, so the note bytes end unchanged and repeated
runs stay deterministic. It peaks at 1,884 VM heap bytes, 28,625/60,000 fuel
(the mount frame), nine host calls in one frame, and 31 retained render
commands; low-level draw counts stay zero. Translator-facing text lives in
`locales/en-US.txt`, `ja-JP.txt`, and the 35–50% expanded `qps-ploc.txt` — 13
non-empty lines each (four component strings and nine status strings), loaded
as package assets at startup and on live `LocaleChanged` events. Unknown tags
fall back to English, and locale changes never touch the saved note bytes.
Every locale-dependent capacity — the per-component retained slots, the
status maximum, the mount data arena, and both update packet arenas — folds
from those packaged locale files at build time
(`asset_text_max_line_bytes`/`asset_text_max_range_bytes` plus additive
compile-time sizing, KOTO-0238), so a translation that grows a line re-sizes
storage on the next build instead of failing at load.

Run the App-specific behavior, locale, damage-trace, and pixel goldens:

```powershell
cargo test -p koto-sim --test koto_ui_file_note
```

Against the pre-migration immediate-drawing sample (1,428-byte KBC, 106-byte
heap, a full-screen rect plus two text draws submitted every frame), the pilot
trades bytecode size (the inlined SDK packet validators dominate the 184,092
bytes, PSRAM-resident on device) for an idle loop that repaints nothing and
interaction damage bounded to the affected components.
