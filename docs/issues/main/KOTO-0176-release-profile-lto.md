# KOTO-0176: deterministic release code layout (workspace LTO + codegen-units=1)

- Status: IN PROGRESS 2026-07-11 — Stage 0 landed (profile block + host
  measurements below); Stage 1 device gates pending.
- History: OPEN 2026-07-11 — spun out of the KOTO-0174 H-D layout-fragility
  incident. Not urgent, but every future firmware refactor rolls the layout
  dice until this lands.
- Type: build infrastructure (performance robustness)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth: the workspace root `Cargo.toml` — it has **no
`[profile.release]` today**, so firmware release builds use the defaults:
`opt-level = 3`, `codegen-units = 16`, **no LTO**. (Cargo ignores profile
sections in member crates; the change must land at the workspace root.)

Relates to: [KOTO-0174](KOTO-0174-present-path-cost-reduction.md) (the H-D
incident this fixes for good, and the source of every device baseline number
below), KOTO-0170/0172 (`phase=176` stack canary — the guard for LTO-induced
poll-frame changes), KOTO-0173 (code-window `refills=` must stay 0 for
kotorun).

## Motivation — the incident this prevents

During KOTO-0174 H-D (2026-07-11), moving one hot per-band function
(`convert_rgb565_to_rgb666`) across a crate boundary *without* `#[inline]`
produced two consecutive firmware builds (Gfx and Perf diag profiles) with:

- a **uniform ~2× CPU slowdown** — raster 0.5 → 0.95–1.17 µs/px, with vm and
  transfer inflated alongside — and
- **loud BGM crackle**.

The identical tree with `#[inline]` added was healthy under two profiles. The
mechanism: at `codegen-units = 16` with no LTO, function layout reshuffles per
build — and per DIAG profile, since dead-code elimination changes `.text` —
and an unlucky layout defeats the 16 KiB XIP cache residency the raster hot
loops depend on (KOTO-0174 H-B established that residency is what makes them
fast). The XIP cache is **shared by both cores**, so core0's thrash also
starves core1's audio-worker instruction fetches: the crackle was a symptom of
a core0 code-layout problem.

Today's protections are per-site `#[inline]` annotations and luck. The durable
fix is a deterministic, dense layout.

## Proposal

Add to the workspace root `Cargo.toml`:

```toml
[profile.release]
lto = "fat"
codegen-units = 1
```

`opt-level` stays at the default 3 — one variable at a time (`opt-level =
"s"` is a separate, later experiment if XIP pressure ever warrants it).
Fallback if fat-LTO build times are unacceptable: `lto = "thin"` +
`codegen-units = 1`.

Expected effects:

- **Deterministic layout**: the same tree always produces the same layout
  (profile changes still relayout, but reproducibly — an A/B is an A/B).
- **Denser `.text`** and whole-program inlining: better XIP residency for the
  hot loops; H-D's `#[inline]` becomes belt-and-braces instead of
  load-bearing. Perf expected flat-to-better across the board.
- **Cost**: fat LTO + one codegen unit serializes the backend — release
  builds get slower (measure it). Every device perf number on record becomes
  "pre-LTO"; comparisons across the change must say so.

## Stage 0 — land + host-side measurements

- Add the profile block; record for `koto_firmware` (thumbv6m release):
  `.text`/`.bss`/total size before/after, and wall build time before/after
  (clean build).
- Full host suites stay green (they run the dev/test profiles and are
  unaffected; sim release users just build slower).

### Stage-0 results (2026-07-11)

`lto = "fat"` + `codegen-units = 1` landed at the workspace root.
`koto_firmware`, thumbv6m release, clean build (`cargo clean` first), default
features, same tree both sides:

| metric | before (no LTO, cu=16) | after (fat LTO, cu=1) | delta |
| :--- | ---: | ---: | ---: |
| clean build wall time | 28.3 s | 29.0 s | +0.7 s (noise) |
| `.text` | 268,448 B | 303,812 B | +35,364 B (+13.2%) |
| `.rodata` | 459,860 B | 436,484 B | −23,376 B (−5.1%) |
| `.data` | 48,968 B | 48,780 B | −188 B |
| `.bss` | 124,496 B | 124,008 B | −488 B |
| ELF total (`rust-size`) | 902,423 B | 913,733 B | +11,310 B (+1.25%) |
| flash image (boot2+vt+text+rodata+data) | 777,724 B | 789,524 B | +11,800 B (+1.5%) |

Notes:

- **Build-time cost did not materialize** — fat LTO is free on this
  workspace's wall clock; no need for the `lto = "thin"` fallback.
- `.text` **grew** (+13%) rather than densified: whole-program inlining
  duplicates bodies. Total `.text` size is not the gate — 16 KiB XIP cache
  residency of the *hot loops* is, and that's a Stage-1 device question
  (`phase=160` raster/transfer µs/px vs the post-H-D baselines).
- `.bss` shrank slightly; the `phase=176 free_min` gate still must run on
  device (poll-frame/stack effects are invisible to `rust-size`).
- **Host suites**: `cargo test`, app build sync, memo validation, and the
  runtime budget gate all pass. The failing checks (`fmt`, `clippy` on
  koto-gfx test code, golden `packages=16→17`, project-harness doc links) are
  all pre-existing drift on develop — koto-gfx has no working-tree diff, and
  a profile block cannot introduce lints. Host checks run dev/test profiles
  and are unaffected by `[profile.release]`, as predicted.

## Stage 1 — device gates (all must pass to keep)

### Stage-1 progress (2026-07-11)

**Perf profile: boot OK, all measured gates pass.**

- `phase=176`: boot used=18,328 / app used=19,796; **free_min=77,380 B** —
  ~+10 KiB *better* than the pre-LTO ~67 KiB-class margin (KOTO-0173).
  Whole-program inlining shrank the poll frame rather than growing it.
- `phase=160` KotoRun vs post-H-D baselines:
  - frame=1 full repaint (102,400 px): lat 77 ms (baseline ~76 — flat);
    raster 0.431 µs/px (≤ ~0.5 ✓); transfer 0.270 µs/px (0.29–0.35 band ✓).
  - frame=960 (19,708 px, 11 rects): vm_us 10.6 ms (7–11 band ✓); raster
    0.521 µs/px (~0.5, noise ✓); transfer computes to 0.40 µs/px — above the
    band, but the band was measured on heavy frames and small-area frames
    inflate per-px transfer (per-rect setup + overlapped-convert accounting);
    confirm with one ≥50 k px heavy frame before calling it noise.
  - `refills=0` on both frames ✓.

Still open: boot smoke on Audio + Gfx profiles; audio check (KotoRun BGM /
KotoBlocks, SMASH micro-crackle delta); app smokes (KotoBlocks / KotoShogi /
KotoRogue); ideally one `free_min` reading on the full worst-case session mix
(boot → shell → all games + IME/memo + audio, the KOTO-0170 mix).

1. **Boot smoke on all three DIAG profiles** (Audio — the shipping value —
   plus Perf and Gfx): each profile is a different layout; each must boot.
2. **`phase=176 free_min`**: whole-program inlining can change the main-task
   poll frame (the KOTO-0172 class of failure). Must stay comfortably above
   the KOTO-0170 stop-ship floor; compare against the current ~67 KiB-class
   margin.
3. **`phase=160` KotoRun capture vs the post-H-D baselines** (KOTO-0174):
   frame-1 full repaint ~76 ms; heavy frames raster ~0.5 µs/px and transfer
   ~0.29–0.35 µs/px; quiet frames fps ~103 (play) / ~311 (title); `vm_us`
   band ~7–11 ms; `refills=0`. Gate: nothing regresses past noise.
4. **Audio**: KotoRun BGM and KotoBlocks clean. Note whether KotoRun's
   SMASH-combo micro-crackle changes — the shared-XIP-cache theory predicts
   it may improve.
5. **App smokes**: KotoBlocks / KotoShogi / KotoRogue launch and play.

## Stage 2 — re-baseline

Update the KOTO-0174 numbers (or annotate them as pre-LTO) and the memory
notes; from then on the LTO layout is the baseline epoch for every device
measurement.

## Non-goals

- `opt-level` changes (`"s"`/`"z"`), SRAM function placement, and sysclk
  overclocking — separate levers, separate issues.
- Dev-profile changes (host iteration speed stays as is).

## Risks

- Poll-frame growth from aggressive inlining (stack) — `phase=176` catches it.
- Fat-LTO build time on this workspace may be minutes per firmware build; if
  it hurts iteration, fall back to `lto = "thin"` and re-run the gates.
- A perf *shift* (either direction) invalidates cross-epoch comparisons —
  Stage 2 exists so nobody quotes a stale number.

## Acceptance criteria

- [x] `[profile.release] lto + codegen-units = 1` landed at the workspace
      root, with size/build-time deltas recorded (Stage-0 results above).
- [ ] Stage-1 device gates all pass (boot ×3 profiles, `free_min`, KotoRun
      `phase=160` vs baselines, audio clean, app smokes).
- [ ] Stage-2 re-baseline recorded in KOTO-0174 and memory; incident
      mitigation (`#[inline]`-and-luck) documented as superseded.
