# KOTO-0169: steady-frame `vm_us` attribution and reduction

- Status: done — all four landed levers device-validated 2026-07-06;
  cumulative KotoRun steady `vm_us` ≈ **−35%** on the opt-in
  `ram_interpreter` build (ns/op 1,975 → 1,418 via Stages 2+3; ops/frame
  −8–12% via Stage 4; H1 falsified along the way; kotosnake/kotoshogi
  pinned to legacy templates). `ram_interpreter` default-on is spun off as
  [KOTO-0170](KOTO-0170-ram-interpreter-default-on.md) (stack-peak
  measurement first; resolved 2026-07-07 — measured margin cleared the bar
  and the feature is now default-on, opt-out via `--no-default-features`). H2-c (Result/StepOutcome plumbing) is deliberately NOT
  taken: with `vm_us` at ~13.5 ms the frame is co-dominated by the present
  path again (raster ≈ 13 ms) and the fixed 3.6 ms/frame refill tax
  (KOTO-0132 / psram_fast_code_window) is the larger remaining VM-adjacent
  slice, so H2-c's ~0.5–1.5 ms would be spent where Amdahl gives the least
  back. Reopen it from this ledger if VM time becomes the floor again.
- Type: performance investigation (VM / firmware frame loop / compiler)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs)
(`vm_us` measured around the whole `step_frame_with`
[app_runtime.rs:811-813](../../../src/koto-pico/src/firmware/app_runtime.rs#L811)),
[koto-vm `CodeSource::word`](../../../src/koto-vm/src/lib.rs#L483) (the per-instruction
fetch the device serves through the PSRAM-backed CodeWindow),
[koto-vm `BytecodeVm::step`] (dispatch + fuel accounting),
`DeviceRuntimeHost` hostcall dispatch (`app_host.rs`),
[KOTO_VM_PROFILE_KOTOBLOCKS.md](../../devlog/KOTO_VM_PROFILE_KOTOBLOCKS.md) (the host-side
`opcode_stats` profiling method and the two compiler optimization targets).

Relates to: [KOTO-0168](KOTO-0168-kotorun-render-performance.md) (made KotoRun's
render cheap, exposing this ceiling), [KOTO-0154](KOTO-0154-compiler-peephole.md)
(landed constant-fold peephole; deliberately skipped template target #1),
[GFX-0011](../kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md) (recorded the
KotoSnake `vm_us ≈ 165 ms` spike as an out-of-scope VM-side cost),
[KOTO-0132](KOTO-0132-profile-and-optimize-pio-psram-read-bandwidth.md) (the
`cw_refill_us` refill timing `vm_us` can already be partially attributed against;
the fetch counters themselves live on `CodeSource`),
[KOTO_KOTOBLOCKS_COLD_BLOCK_OUTLINING.md](../../devlog/KOTO_KOTOBLOCKS_COLD_BLOCK_OUTLINING.md)
(KOTO-0156 per-app codegen opts — kotorun measured no refill win, so its ceiling is
not tile thrash),
[DIAG-0001](../diagnostics/DIAG-0001-runtime-diagnostic-profiles.md) (diag gating).

## Observed problem

After KOTO-0168, KotoRun's quiet steady frames are render-cheap on device — and the
frame time is now floored by the VM, not the present path:

```
phase=160 ... frame=1350 vm_us=17565 raster_us=13409 transfer_us=7968 dirty_px=9425  fps=24 lat_ms=41
phase=160 ... frame=1380 vm_us=23973 raster_us=11283 transfer_us=7473 dirty_px=8614  fps=22 lat_ms=45
```

`vm_us` 17.5–24 ms per frame caps quiet-frame fps at ~24 even with < 10k dirty px.
This is not KotoRun-specific: GFX-0011's hardware smoke recorded KotoSnake `full=0`
frames with `vm_us ≈ 165 ms` (a long-body frame), i.e. the same ceiling at a larger
scale. Two data points suggest the per-instruction cost is high: the sim budget
run has KotoRun peaking at ~9.3k executed instructions/frame (`fuel_peak=9264`),
which against ~20 ms of device `vm_us` reads as **~2 µs per executed instruction**
— hundreds of CPU cycles each. That is only an estimate (sim fuel vs device wall
time); producing the real number is Stage 0's job.

## What `vm_us` actually contains

`vm_us` wraps the whole `step_frame_with` call
([app_runtime.rs:811-813](../../../src/koto-pico/src/firmware/app_runtime.rs#L811)), so it
conflates four costs that need separating before any optimization is chosen:

1. **Interpreter execution** — opcode dispatch, fuel accounting, stack ops.
2. **Instruction fetch** — every instruction word is served via
   [`CodeSource::word`](../../../src/koto-vm/src/lib.rs#L483); on device that is the
   PSRAM-backed CodeWindow's tile lookup per fetch (the *refill* cost is already
   metered separately — `cw_refill_us` — but the per-word *hit path* is not).
3. **Hostcall handling** — KotoRun steady frames make 60–95 hostcalls
   (`hostcalls=61..76` above), each marshalled through `DeviceRuntimeHost`.
4. **Code-window refills** — already attributed (`refills=2`, `cw_refill_us`);
   known small for KotoRun (2 refills/frame steady).

## Hypotheses (to be ranked by Stage 0, not assumed)

- **H1 — fetch hit path:** the per-word CodeWindow translation (tile index math,
  bounds checks, `Option` plumbing) costs tens of cycles per instruction and
  dominates at ~9k instructions/frame.
- **H2 — dispatch overhead:** per-op match dispatch + fuel bookkeeping dominate.
- **H3 — hostcall marshalling:** ~70 calls/frame at some fixed per-call cost
  (argument popping, outcome push, draw capture) explain the floor; implies the
  lever is fewer/cheaper calls, not faster interpretation.
- **H4 — bytecode volume:** the compiler's branchless boolean/comparison templates
  (the `swap/dup/or/shr` population KOTO-0154 deliberately left — its target #1)
  and app-level recomputation inflate instructions/frame; implies a compiler
  template rewrite or app tuning.

## Staged plan

### Stage 0 — attribution diagnostics (observe-only; the only thing this issue lands unconditionally)

(a) **Per-frame executed-instruction count on device.** The session already tracks
it (`last_frame_fuel()` — the sim fixtures read it today); surface it on the
`phase=160` line (`ops=`) or, if the line approaches the 352 B UART `LineBuffer`
limit, on a new sparse line (the GFX-0011 Stage-0b split precedent). Gives the
real device **ns/op** when read against `vm_us`.

(b) **Per-frame hostcall wall time.** Accumulate `Instant` deltas around the
`DeviceRuntimeHost` hostcall dispatch into a per-frame `host_us` counter, emitted
alongside (a). `vm_us − host_us − cw_refill_us` is then pure interpret+fetch time.
Gate any non-trivial timer cost behind the DIAG profile (DIAG-0001).

(c) **Host-side opcode profile for KotoRun steady play.** Reuse the
`opcode_stats` fixture profiler (the KOTO-0154 capture method) on a steady-run
input script: instructions/frame, opcode histogram, and the hot-loop shape, for
KotoRun (and a KotoSnake long-body frame for the 165 ms contrast).

**Gate:** a table attributing KotoRun's ~20 ms and KotoSnake's ~165 ms across
interpret/fetch/hostcall/refill, plus device ns/op. No behaviour change.

### Stage 1 — one lever, chosen by the Stage-0 numbers (gated, separate sign-off)

Candidates, in rough expected-value order per hypothesis:

- **H1 →** fetch fast path: resolve the current tile once and serve straight-line
  words from the cached slice until the next tile boundary / branch invalidates it,
  falling back to the slow lookup exactly as today. No format or ABI change.
- **H2 →** interpreter micro-optimization (dispatch table shape, fuel batching per
  basic block). `koto-vm` semantics and the verifier contract stay byte-identical.
- **H3 →** hostcall cost: cheapen the hot captures (`draw_rect` is ~85% of calls)
  and/or reduce app-side call count (KotoRun currently re-emits ~55 background
  rects/frame that a retained scroll-offset primitive could eliminate — an ABI
  question, recorded, not proposed).
- **H4 →** the KOTO-0154 follow-up template rewrite (boolean-normalization /
  comparison lowering — its skipped target #1), which shrinks every app's
  instructions/frame at the compiler level.

### Non-goals

- No render/present-path change (KOTO-0168 owns that; its Stage-4 dials stay there).
- No `KBC1` format, opcode-set, hostcall-ID, or verifier change.
- No PSRAM timing/clkdiv change (psram_fast_code_window remains its own,
  hardware-unverified track).
- No `RuntimeLimits` / fuel-budget retune (this is about the cost of an
  instruction, not how many are allowed).

## Stage 0 implementation record (2026-07-06)

### 0a/0b — the `phase=175 app-vm-cost` line (implemented; hardware capture pending)

One new sparse line, emitted on the same cadence as `phase=160` (frame 1, then
every `sample_period()` — 120 under the default Perf profile), carrying every
attribution input for that frame's step:

```
phase=175 app-vm-cost app=<id> frame=<n> vm_us=<n> ops=<n> host_us=<n> cw_refill_us=<n> refills=<n>
```

- `ops` (Stage 0a) is `session.last_frame_fuel()` — the executed-instruction
  count the sim fixtures already read. **Device ns/op = `vm_us * 1000 / ops`.**
- `host_us` (Stage 0b) is accumulated by a new observe-only seam:
  `VmHost::hostcall_dispatch_begin/_end` (default no-ops, koto-vm) bracket the
  whole `HOST_CALL` dispatch — argument pops, host method, outcome push — on
  success and error paths alike (unit-tested both ways). `DeviceHost`
  implements the pair with `Instant` deltas into a per-frame counter reset by
  `clear_frame`. Two timer-counter reads per call (~70 calls/frame steady) is
  trivial next to a ~20 ms `vm_us`, so it is not DIAG-gated (per the Stage-0b
  "non-trivial cost" clause). Sim hosts keep the no-op defaults.
- `vm_us − host_us − cw_refill_us` = pure interpret + fetch time (H1 vs H2
  population); `host_us` sizes H3 directly.

A separate line rather than new `phase=160` fields (the GFX-0011 Stage-0b
split precedent): the headline line already runs long against the 352 B
`LineBuffer`, and this keeps its bytes identical — the observe-only proof is
`phase=160` unchanged, `phase=175` new. Worst-case `phase=175` length is
~110 B, no truncation risk. No koto-gfx/present-path change; `KBC1`, opcodes,
hostcall IDs, verifier all untouched (the trait hooks are host-API only).

### 0c — host-side execution profiles (captured)

Method: KOTO_VM_PROFILE_KOTOBLOCKS.md's fixture runner. New tests
`kotorun_steady_play_execution_profile` and
`kotosnake_long_body_execution_profile` in
[fixture_runner.rs](../../../src/koto-sim/tests/fixture_runner.rs); run with
`cargo test -p koto-sim --features opcode_stats --test fixture_runner
<name> -- --nocapture`.

**KotoRun steady play** (title->run at frame 2, then 60 flat-run frames):

| Metric | Value |
| --- | --- |
| Instructions/frame (steady) | 3,821–5,247, stepping up with the visible course (fuel peak 5,247 at 61 hostcalls/frame) |
| Hostcalls/frame | 48 → 61 as segments appear (recorded 47 → 60) |
| Hostcall mix (3,250 recorded) | `draw_rect` 2,928 (**90%**), `draw_text_color` 255, `text_input` 63, one-shot static/audio 4 |
| Cumulative | 267,929 instructions / 63 frames; stack peak 7, call depth 0 |

Opcode histogram (top of 267,929): `push_i16` 29.5%, `load_local` 12.6%,
`sub_i32` 11.6%, `shr_i32` 7.9%, `swap` 7.9%, `or_i32` 5.9%, `dup` 5.0%,
`store_local` 4.6%, `br_if_zero` 3.3%, `add_i32` 3.2%. The KOTO-0154 skipped
target #1 signature (`swap`/`dup`/`or`/`shr`) is ~26.7% — the H4 population is
real for KotoRun too, same shape as KotoBlocks.

Note: this flat-run script peaks at 5.2k instructions/frame; the ~9.3k
`fuel_peak=9264` cited above came from the denser budget-run script. Against
the device's 17.5–24 ms `vm_us` even the denser figure reads as µs-scale
per-op; the honest ns/op number is exactly what `ops=` on hardware yields.

**KotoSnake long-body** (greedy apple-seeker, 6,000 frames):

| len (cells) | frames | avg instr | max instr |
| --- | --- | --- | --- |
| 4–7 | 265 | 5,557 | 10,827 |
| 8–11 | 239 | 8,232 | 14,399 |
| 12–15 | 214 | 9,558 | 14,296 |
| 16–19 | 5,282 | 7,778 | 18,708 |

Peak frame: 18,708 instructions at len 18 (69 immediate draws). Histogram
(46.6 M instructions, long-body dominated): `push_i16` 26.1%, `load_local`
16.0%, `sub_i32` 12.5%, `swap` 7.7%, `shr_i32` 6.3%, `store_local` 5.5%,
`or_i32` 4.7% — same push/arith/shuffle mix. Caveat: the greedy seeker
plateaus at len ≈ 19, far short of the device session behind the 165 ms
frame; instructions/frame clearly scales with length (5.5k → 9.6k avg,
18.7k peak), but the 165 ms contrast itself must come from the device
`ops=` line during real long-body play.

**What Stage 0c already says about the hypotheses:** the sim capture bounds
H3's call count (~48–61 calls/frame KotoRun — the per-call device cost is what
`host_us` will divide out) and confirms H4's compressible population (~27% of
executed instructions are the branchless-template shuffle). H1 vs H2 cannot be
separated host-side; that split is `vm_us − host_us − cw_refill_us` on device.

### Verification (observe-only proof, host side)

- `cargo test -p koto-vm` — 37 + 19 pass (includes the two new seam tests).
- `cargo test -p koto-sim` — 103 + 21 pass (goldens untouched).
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` — clean;
  clippy on the touched crates adds no new warnings (pre-existing
  `fixture_runner` lints and rustfmt drift unchanged, same counts).
- `phase=160` emission code untouched; `phase=175` is additive. The device
  A/B (byte-identical render fields, only the new line appearing) rides the
  next hardware run with the attribution capture.

### Hardware attribution capture (2026-07-06)

`phase=175` captured on device for KotoRun steady play (frames 1–330) and a
KotoSnake session grown long-body (frames 1–1770). All lines complete and
untruncated. Derived columns: `i+f = vm_us − host_us − cw_refill_us`
(interpret + fetch) and `ns/op = i+f × 1000 / ops`.

| App / frame | vm_us | ops | host_us | cw_refill_us | i+f (µs) | i+f ns/op | i+f / refill / host share |
| --- | --- | --- | --- | --- | --- | --- | --- |
| KotoRun f60 (quiet) | 12,631 | 4,335 | 379 | 3,531 | 8,721 | 2,012 | 69% / 28% / 3% |
| KotoRun f300 (steady) | 20,731 | 8,390 | 444 | 3,635 | 16,652 | 1,985 | 80% / 18% / 2% |
| KotoRun f210 (peak) | 24,395 | 10,218 | 568 | 3,636 | 20,191 | 1,976 | 83% / 15% / 2% |
| KotoSnake f90 (short) | 12,738 | 4,178 | 342 | 3,699 | 8,697 | 2,082 | 68% / 29% / 3% |
| KotoSnake f1710 (long-body peak) | 41,523 | 18,748 | 807 | 3,801 | 36,915 | 1,969 | 89% / 9% / 2% |

**The headline: interpret+fetch is a flat ~2.0 µs per executed instruction,
app-independent and load-independent.** Across all 20 steady samples (11
KotoRun, 9 KotoSnake) the i+f ns/op sits in 1,933–2,088 (KotoRun mean 1,982,
KotoSnake mean 1,998) while `ops` varies 5× (3.8k → 18.7k). At the 125 MHz
sys clock that is **~250 CPU cycles per instruction** — the frame cost is
purely `ops × 2 µs` plus two fixed taxes:

- **Code-window refills: a fixed ~3.6 ms/frame** (steady `refills=2`,
  ~1.8 ms/refill — 28% of a quiet 12.6 ms frame, 9% of the long-body peak).
  Out of scope here (non-goal), but this is now the measured size of the
  KOTO-0132 / `psram_fast_code_window` prize.
- **Hostcalls: 0.34–0.81 ms/frame (~2–3%)** — ~6–9 µs/call at 50–90
  calls/frame. **H3 is ruled out**; no hostcall lever moves this frame.

The GFX-0011 `vm_us ≈ 165 ms` KotoSnake frame back-solves to ~82k executed
instructions at this rate — a very long body doing ~82k ops of per-cell work,
not a pathological VM state. The linear model predicts it exactly; the
capture's own 41.5 ms @ 18,748 ops point already matches the sim's 18,708-op
peak frame at len 18.

**Hypothesis ranking from the numbers:**

- **H3 (hostcall marshalling): eliminated** — 2–3% of `vm_us`.
- **H1/H2 (fetch hit path + dispatch): confirmed as the ceiling, 69–89% of
  `vm_us`.** Stage 0 cannot split them from each other on-device, but ~250
  cycles/op on a Cortex-M0+ is far beyond plausible pure match-dispatch cost
  (tens of cycles); the per-word `CodeSource::word` CodeWindow hit path (tile
  index math, bounds checks, `Option` plumbing on every fetch) is the prime
  suspect inside the block.
- **H4 (bytecode volume): real and multiplicative** — the Stage-0c histograms
  put the compressible `swap/dup/or/shr` template population at ~27% of
  executed instructions, and every op removed saves a flat 2 µs.

**Stage-1 recommendation (recorded, awaiting sign-off): H1 — the fetch fast
path** (resolve the current tile once, serve straight-line words from the
cached slice until a tile boundary / branch invalidates it, slow path
unchanged). It attacks the dominant 69–89% block with no format/ABI change,
and its effect is directly measurable as the `ns/op` on this same `phase=175`
line dropping. H4 (the KOTO-0154 template rewrite) stays the natural second
lever — its ~27% op-count cut multiplies with whatever ns/op Stage 1 reaches.

## Stage 1 implementation record (2026-07-06, H1 signed off)

The signed-off lever: **H1 — the fetch fast path**. Straight-line words are now
served from a VM-local line instead of one `CodeSource` round-trip per
instruction; the slow path is byte-for-byte today's behavior.

**Design (three small pieces, no ABI/format/opcode/verifier change):**

1. **`CodeSource::word_run(index, dst) -> usize`** (koto-vm, additive trait
   method): copy up to `dst.len()` consecutive words, serving the *first* word
   exactly like `word` (same refill-on-miss) and extending the run only over
   storage that resolution already made resident. The default serves exactly
   one word via `word()`, so every existing source — `SliceCode`, the sim, the
   `TileRecorder`/`WindowedCode` test doubles — keeps today's
   one-fetch-per-instruction behavior and metrics unless it opts in.
2. **A 16-word fetch line in `BytecodeVm`** (`line`/`line_base`/`line_len`,
   64 B): `step` serves the next word from the line on a single wrapping
   range check and only calls `word_run` on a miss (line exhausted, or a
   branch/call left it). Valid because the code segment is immutable for the
   life of the VM; the only invalidation point is frame entry, where the
   caller may hand in a different `CodeSource`.
3. **`PsramCodeWindow::word_run`** (koto-core): resolves the tile with the
   identical cached-check/refill as `word`, then serves the run clamped to
   the current tile — never a second refill, so `refills=`, `code_tiles=`,
   `cw_refill_us=` and their per-tile histograms are unchanged for any
   execution path.

Steady KotoRun/KotoSnake code is push/arith-dominated with branches ~5% of
executed ops, so typical runs span many instructions; the per-instruction
cross-crate `word()` call, window bounds/tile checks, and `try_into` collapse
to one array index + compare on the hot path.

**Deliberately not done:** `SliceCode` keeps the 1-word default (sim numbers
— `CountingCode` reads == instructions — stay byte-identical); no fuel-
accounting batching (that is H2, a separate lever); no window-size or PSRAM
change.

**Host-side verification:**

- New koto-vm test `straight_line_fetch_line_matches_word_by_word_execution`:
  a backward-loop + forward-branch + >16-word-tail program over 1/3/5/64-word
  windowed sources — results, draws, and executed-instruction counts identical
  to the resident-slice run, and multi-word windows make strictly fewer source
  calls than instructions (the amortization actually engaged).
- New koto-core tests `code_window_word_run_matches_word_and_clamps_to_the_tile`
  and `code_window_word_run_walk_keeps_refill_metrics_of_a_word_walk`
  (refills / distinct tiles / refill bytes equal to a word-by-word walk).
- Full suites green: koto-core 134, koto-vm 38+19, koto-sim 103+21 (fixture
  profiles unchanged). Firmware builds dev + release; clippy/fmt deltas zero
  (pre-existing drift only).

**Success metric (device, pending):** the `phase=175` interpret+fetch ns/op
(`(vm_us − host_us − cw_refill_us) / ops`) dropping from the ~1,980 baseline
on the same KotoRun steady play, with `ops=`, `refills=`, `cw_refill_us=` and
all `phase=160` render fields unchanged.

### Stage 1 device validation (2026-07-06) — null result

`phase=175` re-captured on the Stage-1 build: KotoRun steady play (frames
1–690, 22 steady samples) and a KotoSnake session grown long-body (frames
1–1260, 41 steady samples).

| Capture | i+f ns/op range | mean | baseline mean | Δ |
| --- | --- | --- | --- | --- |
| KotoRun steady (ops 4.3k–10.5k) | 1,913–2,051 | **1,969** | 1,982 | −0.7% |
| KotoSnake short→long (ops 3.8k–15.9k) | 1,917–2,143 | **~1,990** | 1,998 | −0.4% |

**The fetch fast path did not move the per-op cost.** Every observe-only
invariant held (steady `refills=2`, `cw_refill_us` ≈ 3.6–3.9 ms, `ops`
patterns and `host_us` unchanged — the fast path is semantically invisible,
as designed), but the ns/op is statistically identical to the baseline. Both
captures also reproduce the baseline's second-order shape: small-`ops` frames
read slightly higher ns/op (~2,050–2,140 at ~4k ops) than large ones
(~1,940–1,980 at >10k ops), i.e. a small fixed per-frame cost plus a flat
per-op cost — unchanged by Stage 1.

**What this falsifies:** H1. Even with the per-instruction `CodeSource` call,
window bounds/tile checks, and `try_into` amortized 16× into one array index
+ compare, per-op time is unchanged — so the ~250 cycles/op were never in the
bytecode-fetch hit path. By elimination within the Stage-0 attribution, the
cost sits in the **H2 population**: the cycles spent *around* every
instruction regardless of how its word arrives — opcode dispatch, per-op
fuel/stats/budget bookkeeping (three counters including a `u64` per op), the
`Result`/`StepOutcome` plumbing through two call layers per op, and plausibly
the interpreter's *own* instruction stream executing from RP2040 flash XIP
(cache misses no bytecode-side change can touch).

**Stage-1 code disposition: kept.** It is semantics-neutral, fully tested,
proved no regression on device, removes the fetch path from the suspect list
for good, and the `word_run` seam is the infrastructure any future
window-shape work needs.

**Next lever (H2) — needs its own sign-off, not started:** candidates, each
measurable on the same `phase=175` ns/op line, none touching semantics:
(a) hoist the per-op fuel/stats/budget bookkeeping out of the step loop
(count fuel by loop induction, fold `stats.instructions` /
`frame_fuel_peak` in once per frame); (b) place the interpreter hot loop in
SRAM (RAM-function) to take XIP flash misses out of every dispatch;
(c) flatten the per-op `Result`/`StepOutcome` plumbing. H4 (the KOTO-0154
template rewrite, ~27% fewer executed ops) remains the orthogonal multiplier.

## Stage 2 implementation record (2026-07-06, H2-b signed off)

The signed-off second lever: **H2-b — run the interpreter hot loop from SRAM**
instead of RP2040 flash XIP. Rationale: the Stage-1 null result plus the flat,
app-independent ~250 cycles/op point at a constant per-op overhead *around*
every instruction; the interpreter's own machine code fetching through the
16 KiB XIP cache — shared with the render/present code that runs every frame
— fits that shape exactly. All hardware evaluation is `--release` (confirmed),
so this is not an optimization-level artifact.

**Device attempt #1 (default-on, every instantiation tagged): boot failure.**
The first shape tagged `execute_frame_with` itself, so *both* device
monomorphizations (the PSRAM-window path and the `SliceCode` small-app
fallback) landed in RAM — `.data` = 4,444 B, static free RAM 79.3 → 73.9 KiB
(the stack grows down from `0x2004_2000` into `.bss` with no linker guard —
the pico-firmware-stack-headroom note). The device did not boot; feature off
boots. The flash image itself was verified good (UF2 parsed block-by-block:
the `.data` payload at LMA `0x100A_EB40` present and byte-identical to the
ELF section), so the failure is consistent with the main-stack peak sitting
inside the ~74–79 KiB window — i.e. the budget was already within ~5 KiB of
the edge, and this is a RAM-budget collision, not a mechanism fault.

**Reshape after the failure (current form) — half the RAM, opt-in only:**

- `ram_interpreter` is now an explicit **opt-in** koto-pico feature
  (`--features ram_interpreter`), not a default: the default firmware build
  is byte-identical to pre-Stage-2 (`.data` = 0) and always boots.
- The frame loop is split into a shared `#[inline(always)]` core with two
  thin twins: `execute_frame_with` (untagged, flash) and
  `execute_frame_with_ram` (`link_section = ".data.koto_vm_interp"` +
  `inline(never)`, cfg-gated). A new associated const
  `CodeSource::PLACE_HOT_LOOP_IN_SRAM` (default `false`;
  `PsramCodeWindow` = `true`; `CountingCode` forwards) lets the session pick
  the twin per instantiation at compile time — so **only the PSRAM-window
  hot path pays RAM**; the `SliceCode` small-app fallback and every host/sim
  instantiation stay in flash.
- `step` / `exec_binary` / `push`/`pop`/`peek`/`branch` are
  `#[cfg_attr(feature, inline(always))]` instead of section-tagged: forced
  into each twin's body, so the twin's placement deterministically governs
  where dispatch executes (an outlined shared copy would silently pin one
  path to flash). `PsramCodeWindow::word_run` keeps the section tag (its only
  hot caller is the RAM twin); `exec_host_call` stays **pinned in flash**
  (`inline(never)`) — single-call-site, would otherwise inline its 4.7 KiB
  body into the RAM copy, and at ~70 dispatches/frame its XIP residency is
  noise.

**Verified placement (release ELF with `--features ram_interpreter`,
llvm-nm):** one `execute_frame_with_ram` copy (2,272 B — `step`, the helpers,
and `word_run` all inlined) at `0x2000_0000` plus a 12 B thunk; `.data` =
**2,356 B** (was 4,444), static free RAM ≈ 76.4 KiB (−2.9 vs the booting
baseline's 79.3). `exec_host_call` in flash. Default build: `.data` = 0,
byte-identical layout to pre-Stage-2.

**Host-side verification:** all suites green (feature never enabled on host):
koto-core 134, koto-vm 38+19, koto-sim 103+21; firmware builds dev + release
in both configurations.

**Success metric (device, pending retry):** flash
`cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi
--release --features ram_interpreter`; first gate is boot + `phase=152
app-started`. If it boots: `phase=175` interpret+fetch ns/op vs the
~1,970–1,980 KotoRun-steady baseline. If it still fails to boot at −2.9 KiB,
the stack budget cannot host H2-b at all — record that, keep the feature
opt-in (off), and move to H2-a (the bookkeeping hoist, zero RAM cost).

**Retry odds, stated honestly:** today's boot threshold is only bracketed as
"somewhere in (73.9, 79.3) KiB static free" — the KOTO-0136 era hung boot at
~78.5 KiB free (different `.bss` layout, so not directly comparable), and
clean develop boots at 79.3. The reshaped build's 76.4 KiB sits inside that
uncertainty band: a genuine coin flip. It is still worth one flash cycle —
either outcome sharpens the real stack ceiling by ~2.5 KiB, and the failure
mode is benign (reflash the default build). H2-b beyond that needs a ~5 KiB
`.bss` sponsor (every candidate — code window, audio core1 stack, IME/SKK
buffers — is a deliberate budget with its own trade-off; none is touched
here, per the deliberate-budget-sizing discipline).

### Stage 2 device validation (2026-07-06) — CONFIRMED, −16% ns/op

The reshaped opt-in build **boots clean** (76.4 KiB static free — the boot
threshold is now bracketed to (73.9, 76.4) KiB) and `phase=175` was captured
over KotoRun steady play, frames 600–1230 (22 samples, ops 4.0k–10.7k):

| Capture | i+f ns/op range | mean | baseline | Δ |
| --- | --- | --- | --- | --- |
| KotoRun steady, Stage-2 build | 1,631–1,693 | **1,646** | 1,969–1,982 | **−16.4%** |

Every observe-only invariant held (`refills=2`, `cw_refill_us` ≈ 3.4–3.8 ms,
`host_us` ≈ 0.43–0.65 ms, unchanged). The improvement is flat across the ops
range — even the small-ops frames (~4k ops: 1,671–1,693) sit far below the
baseline's small-ops band (~2,050–2,140) — exactly the signature of a
constant per-op cost removed.

**Conclusion:** the interpreter's own flash-XIP residency was a real,
measurable component of the per-op cost: **~325 ns/op ≈ 40 cycles/op**, i.e.
~14.5% of steady `vm_us` at constant ops (e.g. ~10.2k-op frames: 24.4 →
20.9 ms). The remaining ~1,646 ns/op (~206 cycles) is the rest of the H2
population — per-op fuel/stats/budget bookkeeping, dispatch, and
`Result`/`StepOutcome` plumbing — plus M0+ fundamentals.

**Default-on is deliberately NOT flipped:** the booting margin above the
newly bracketed threshold is < 2.5 KiB, too thin to bet the default firmware
on (the KOTO-0136 hang precedent). Making `ram_interpreter` default needs
either a measured stack peak (paint-and-scan canary) proving real headroom,
or a ~3–5 KiB `.bss` sponsor — its own signed-off task. Until then the win
is available behind `--features ram_interpreter`.

**Remaining levers, updated:** H2-a (hoist the per-op fuel/stats/budget
bookkeeping; ~15–25 cycles/op candidate, zero RAM, stacks with this win) and
H4 (the KOTO-0154 template rewrite, ~27% fewer executed ops, multiplicative).
Both need their own sign-off.

## Stage 3 implementation record (2026-07-06, H2-a signed off)

**H2-a — hoist the per-op fuel/stats/budget bookkeeping out of the step
loop.** The fuel loop now counts executed instructions in one induction
variable and folds it into `last_frame_fuel`, the cumulative
`stats.instructions` (`u64`), and the `frame_fuel_peak` high-water exactly
once per frame, on every exit path (yield / exit / trap / fuel exhaustion).
Per op that removes a `u64` saturating add and a peak-max compare
(~15–25 M0+ cycles expected). End-of-frame counter values are identical by
construction: the count only grows, and the trapping instruction was already
counted before its `step` — so every accessor (`last_frame_fuel`,
`stats()`, `budget()`) reads the same numbers as before. All suites green
(koto-vm 38+19, koto-core 134, koto-sim 103+21 — the fixture per-frame
instruction tables are byte-identical).

**Device metric (pending, shared capture with Stage 4):** `phase=175` ns/op
vs the post-Stage-2 ~1,646 baseline (needs the `--features ram_interpreter`
build to compare like-for-like).

## Stage 4 implementation record (2026-07-06, H4 signed off)

**H4 — the KOTO-0154 follow-up template rewrite** (its deliberately skipped
target #1): shrink the branchless boolean/comparison population
(`swap`/`dup`/`or`/`shr`, ~27% of executed ops) at the compiler level. Two
mechanisms, all rewrites exact for every input (no opcode/ABI/verifier
change; `&&`/`||` still evaluate both operands — no short-circuit change):

1. **Branch-context lowering with sense inversion**
   (`Codegen::emit_cond_branch_if_false`): an `if`/`while` condition whose
   truth is exactly the *zero-ness* of a cheaper value drives `br_if_zero`
   directly instead of materializing a 0/1: `a == b` → `sub` + inverted
   branch (was 11 ops), `a != b` → `sub` (was 8), `a >= b` / `a <= b` → the
   `<` / `>` sign template + inverted branch (was 6/7, now 3/4), `!x` →
   recurse and flip. Inversion keeps the block layout unchanged — it only
   prepends `br_if_zero <true>; br <false>; <true>:` (one extra branch on
   the false path). Exactness: `a−b == 0 ⟺ a == b` in wrapping arithmetic,
   and `>=`/`<=` are by definition the negation of the existing `<`/`>`
   sign-bit templates, so overflow behavior is bit-identical.
2. **Value-context template shrinks:** `x || y` = `(x | y) != 0` (15 → 8
   ops); `x && y` via bit-31 mask conjunction `((x|-x) & (y|-y)) >>u 31`
   (15 → 13); `== 0` / `!` via complement `(~(d|-d)) >>u 31` (`~v = -v-1`
   flips the sign bit exactly; 10 → 9); `a <= b` = `sign(a-b-1)` =
   `sign(~(b-a))` (7 → 5).

**The KOTO-0156 tile-layout lesson struck exactly as predicted:** the
globally smaller code slid hot loops across PSRAM code-window tile
boundaries in two apps — KotoSnake's steady play regressed from the
KOTO-0155 2-refill monotone walk to a **10-refill ping-pong**, and
KotoShogi's KOTO-0156-tuned layout broke — both caught by the sim tile
profiles. Resolution follows the KOTO-0156 pattern: a per-app opt-OUT
(`codegen.legacy_compare_templates` in apps.json →
`--legacy-compare-templates`) pins those two apps to the byte-identical
pre-Stage-4 templates; every other app keeps the win. Their tile profiles
are back at the 2-refill floor.

**Host-side results (sim, opcode_stats):**

| App | Instructions | Δ |
| --- | --- | --- |
| KotoRun steady (63 frames) | 267,929 → **234,856** | **−12.3%** (fuel peak 5,247 → 4,597) |
| KotoBlocks title (8 frames) | 66,081 → **52,949** | **−19.9%** (fuel peak 9,452 → 7,817) |

KotoRun histogram shifts: `swap` −36%, `sub_i32` −25%, `shr_i32` −26%,
`dup` −21%; `br` +23% (the inverted-branch prologues — cheap ops replacing
expensive templates). Hostcall counts and draws identical (3,313 / 3,250 —
semantics untouched). 14 of 16 fixtures shrank (e.g. sokoban 19,798 →
18,302 B); kotosnake/kotoshogi byte-identical by design.

**Verification:** koto-compiler 65 tests (incl. two new Stage-4 suites: an
exhaustive value-vs-branch truth-table agreement at INT_MIN/INT_MAX edges,
and `&&`/`||`/`!` normalization + no-short-circuit proof); koto-sim 103+21
(tile profiles green); golden frames byte-identical (only the pre-existing
`packages=16→17` drift); budget gate OK; firmware builds dev + release.

**Device metric (pending):** one capture on the `--features ram_interpreter`
build measures both new stages independently — Stage 3 as the `phase=175`
ns/op drop below ~1,646, Stage 4 as the `ops=` drop (~−12% expected on the
same KotoRun course positions), multiplying into the `vm_us` reduction.

### Stage 3+4 device validation (2026-07-06) — both CONFIRMED

Captured in two cleanly separated runs on the `ram_interpreter` build:
Stage-3 firmware with the *old* `.kbc` (isolates the hoist), then the
Stage-4 `.kbc` swapped in (isolates the codegen), KotoRun steady play:

| Lever | Metric | Result |
| --- | --- | --- |
| Stage 3 (bookkeeping hoist) | i+f ns/op, old kbc | 1,403–1,448, mean **1,418** vs 1,646 → **−13.9%** (~28 cycles/op — in the 15–25-cycle estimate band) |
| Stage 4 (compact templates) | ns/op, new kbc | **1,429** — unchanged, as designed (the lever cuts op *count*, not op cost) |
| Stage 4 (compact templates) | `ops=` | deterministic frame 1: 3,334 → 3,077 (**−7.7%**); early-course frames ~**−12%** (sim predicted −12.3%) |

Cross-checks: at matched ops the two runs agree (9,034 ops → 17.10 ms vs
9,028 ops → 16.96 ms); `refills=2` steady in both (no device tile
regression for KotoRun); `host_us` unchanged; `cw_refill_us` eased slightly
(3.4–3.6 ms — the shrunken last tile transfers fewer bytes).

**Cumulative journey (KotoRun steady, interpret+fetch):**

| Milestone | ns/op | ops/frame | net `vm_us` at a fixed course position |
| --- | --- | --- | --- |
| Stage 0 baseline | ~1,975 | 1.00× | 1.00× |
| + Stage 2 (SRAM interp, opt-in) | 1,646 | 1.00× | 0.83× |
| + Stage 3 (hoist) | 1,418 | 1.00× | 0.72× |
| + Stage 4 (templates) | ~1,429 | ~0.88–0.92× | **~0.64–0.66×** |

A ~20.7 ms steady KotoRun VM frame is now ~13.5 ms — roughly **−35%
`vm_us`** with the `ram_interpreter` build, −22% on the default (flash)
build (Stages 3+4 only). Residual per-op cost ≈ 177 cycles: dispatch,
`Result`/`StepOutcome` plumbing (the unexplored H2-c), stack-op bounds
checks, and M0+ fundamentals.

## Acceptance criteria

- [x] Stage 0a/0b: `ops=` and `host_us=` (and existing `cw_refill_us`) readable on
      hardware for KotoRun steady play — complete, untruncated lines
      (`phase=175 app-vm-cost`, captured 2026-07-06).
- [x] Stage 0c: host-side opcode profile captured for KotoRun steady play and a
      KotoSnake long-body frame, recorded in this doc (long-body caveat: the sim
      seeker plateaus at len ≈ 19; the device contrast came from `ops=`).
- [x] Attribution table: KotoRun ~20 ms and KotoSnake long-body frames split across
      interpret+fetch / hostcall / refill with device ns/op derived (~2.0 µs/op,
      ~250 cycles); the 165 ms frame back-solves to ~82k ops on the same line.
- [x] A Stage-1 lever chosen (or the issue closed "intrinsic — cost is where it
      should be") with the evidence written down here. *(H1 signed off and
      implemented 2026-07-06 — see "Stage 1 implementation record"; H4 remains
      the recorded follow-up multiplier.)*
- [x] Observe-only proof for Stage 0: `phase=160` emission code untouched and
      render fields unchanged by construction (no present-path change); the
      device capture shows only the new `phase=175` diagnostics appearing.
- [x] Stage 1 device validation captured 2026-07-06: **null result** — ns/op
      statistically unchanged (KotoRun 1,982 → 1,969; KotoSnake 1,998 → ~1,990,
      both within noise) with every observe-only invariant intact. H1 falsified
      as the dominant per-op cost; the ceiling is the H2 population. The next
      lever needs its own sign-off (see "Stage 1 device validation").
- [x] Stage 2 (H2-b) device validation: attempt #1 (default-on @ 5.4 KiB)
      FAILED to boot — RAM-budget collision; the reshaped opt-in @ 2.4 KiB
      **boots clean and delivers −16.4% interpret+fetch ns/op**
      (1,969–1,982 → 1,646 mean over 22 KotoRun steady samples), all
      observe-only invariants intact. Default stays off (boot margin
      < 2.5 KiB above the bracketed threshold); flipping it needs a measured
      stack peak or a `.bss` sponsor — its own task.
- [x] Stage 3 (H2-a) implemented: per-op fuel/stats/budget bookkeeping folded
      to once-per-frame with identical end-of-frame counters; all suites
      green. Device ns/op delta rides the shared Stage-3/4 capture.
- [x] Stage 4 (H4) implemented: branch-sense condition lowering + compact
      boolean/comparison templates, exact for all inputs; sim −12.3% KotoRun /
      −19.9% KotoBlocks ops; kotosnake/kotoshogi pinned to legacy templates
      (`codegen.legacy_compare_templates`) after sim tile-profile regressions,
      back at the 2-refill floor; goldens byte-identical, budgets OK.
- [x] Stage 3+4 device validation (two separated captures on the
      `ram_interpreter` build): Stage 3 ns/op 1,646 → **1,418 (−13.9%)** with
      the old kbc; Stage 4 ops **−7.7% on the deterministic first frame /
      ~−12% early-course** with ns/op unchanged (1,429) on the new kbc;
      `refills=2` steady, invariants intact. Net KotoRun steady `vm_us`
      ≈ **−35%** vs the Stage-0 baseline on the opt-in build.
