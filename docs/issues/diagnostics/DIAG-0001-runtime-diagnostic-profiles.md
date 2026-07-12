# DIAG-0001: runtime diagnostic verbosity profiles

- Status: **Proposal** (no code lands with this issue; the deliverable is the phase
  inventory, the classification, the profile design, and a staged implementation plan).
- Type: infrastructure / proposal.
- Priority: P2 (blocks clean measurement of the VM-side spikes GFX-0011 surfaced —
  [KotoSnake VM-side frame spike](../kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md#L56-L59)).
- Requirements: NFR-PERF-1 (diagnostics must not distort the measurement they inform).

Source of truth (emit sites, not changed by this proposal):
[diag.rs](../../../src/koto-pico/src/firmware/diag.rs)
(the `log_app_*` formatters — [`log_app_frame_metrics` / `phase=160`](../../../src/koto-pico/src/firmware/diag.rs#L260),
[`log_dirty_rect_geometry` / `phase=164`](../../../src/koto-pico/src/firmware/diag.rs#L336),
[`log_code_window_fetch` / `phase=163`](../../../src/koto-pico/src/firmware/diag.rs#L399),
[`log_app_budget_observation` / `phase=168`](../../../src/koto-pico/src/firmware/diag.rs#L490),
[`log_app_cmdshift_correlation` / `phase=169`](../../../src/koto-pico/src/firmware/diag.rs#L560),
[`log_app_cmdshift_probe` / `phase=174`](../../../src/koto-pico/src/firmware/diag.rs#L605),
[`log_app_coalesce_pressure` / `phase=171`](../../../src/koto-pico/src/firmware/diag.rs#L656)),
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs)
(the per-frame emit gates — [the every-30/120 render cadence](../../../src/koto-pico/src/firmware/app_runtime.rs#L1127),
[the CommandCountShift gate](../../../src/koto-pico/src/firmware/app_runtime.rs#L1036),
[the audio-summary gate](../../../src/koto-pico/src/firmware/app_runtime.rs#L1274)),
[app_host.rs](../../../src/koto-pico/src/firmware/app_host.rs#L315) (`phase=172` audio hostcall),
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs#L670) (`phase=161` command sample),
[config.rs](../../../src/koto-pico/src/firmware/config.rs) (where the profile constant would live),
[Cargo.toml](../../../src/koto-pico/Cargo.toml#L131) (the existing `psram_qpi_code_window_prod_profile`
feature that already gates diagnostics as a side effect).

Depends on: nothing new — it reorganizes gating over the existing emit sites.

Relates to: [GFX-0011](../kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md)
(whose Stage-0b split proved the 352 B `LineBuffer` truncation problem and whose closing
note names the VM-side spike this proposal must let us measure cleanly),
[GFX-0008](../kotogfx/GFX-0008-commandcountshift-policy-refinement.md) / GFX-0010 (the
`phase=168`/`phase=169`/`phase=171` diagnostics being classified),
[KOTO_BUDGET_OBSERVE_MODE](../../devlog/KOTO_BUDGET_OBSERVE_MODE.md) (`phase=168`/`phase=169` origin).

## Problem

The GFX-0008 → GFX-0011 investigations each added a targeted diagnostic. Individually
every one is low-volume and caller-gated; **together** they now emit up to nine distinct
lines on the same throttled render cadence (`phase=160`, `163`, `164`, `167`, `168`,
`169`, `171`, `173`, `174`), plus `phase=172` on every audio hostcall and a burst of
`phase=161` command-sample lines on the first frames. On the UART link this is enough that
the transmit cost itself perturbs `vm_us`/`work_us` on the very `phase=160` line used to
detect regressions — the measurement distorts the measured.

GFX-0011 closed on exactly the wall this creates: with the present path no longer the
KotoSnake bottleneck, "the remaining major spike is now VM-side — `full=0` frames with
`vm_us ≈ 165 ms`." Profiling that spike requires the render/audio/CodeWindow chatter to be
*off by default* so the numbers are trustworthy, **without deleting** the diagnostics that
earned their place (each is the only evidence for its issue's acceptance criteria) and
**without losing** the small always-on population that detects a regression or a boot fault.

There is already a *de facto* profile — the `psram_qpi_code_window_prod_profile` cargo
feature toggles roughly a dozen `#[cfg(not(feature = "psram_qpi_code_window_prod_profile"))]`
guards and swaps the render cadence 30 → 120. But it is the wrong lever: it is welded to
PSRAM **backend** selection (it pulls in `psram_qpi_safe_read_code_window` +
`psram_qpi_backend_v2`), so you cannot quiet the logs without also changing the PSRAM read
path — the one thing you must *not* change while measuring. This proposal factors the
verbosity axis out of the backend axis.

## 1. Phase log inventory (every current site + frequency)

Grouped by cadence. "Frequency" is per running app-frame unless noted. The render cadence
`C` is **every 30 frames** in the default dev build and **every 120** under
`psram_qpi_code_window_prod_profile`; heartbeat/draw-usage use a fixed **every 60**.

### A. Boot / storage / prefs / power / launch — one-shot or on-event

| Phase | Name | Site | Frequency |
|---|---|---|---|
| 131/132 | sd-card-init start/ok | [storage.rs:35](../../../src/koto-pico/src/firmware/storage.rs#L35) | once at boot |
| 133–136 | fat/root/apps-dir open | storage.rs | once at boot |
| 137 | apps-list-ok | [storage.rs:121](../../../src/koto-pico/src/firmware/storage.rs#L121) | once at boot |
| 138/139 | manifest-read start/done | [storage.rs:126](../../../src/koto-pico/src/firmware/storage.rs#L126) | once per manifest at boot |
| 140 | icons-loaded | [storage.rs:223](../../../src/koto-pico/src/firmware/storage.rs#L223) | once at boot |
| 141–143 | prefs missing/applied/saved | [shell_prefs.rs:34](../../../src/koto-pico/src/firmware/shell_prefs.rs#L34) | on prefs event |
| 145 | power-poll changed | [koto_firmware.rs:653](../../../src/koto-pico/src/bin/koto_firmware.rs#L653) | on battery-state change |
| 146/147 | battery / bridge-version | [power.rs:46](../../../src/koto-pico/src/firmware/power.rs#L46) | boot + periodic power poll |
| 148/149 | sd removed/inserted | [koto_firmware.rs:445](../../../src/koto-pico/src/bin/koto_firmware.rs#L445) | on SD event |
| 150 | launch-request | [koto_firmware.rs:525](../../../src/koto-pico/src/bin/koto_firmware.rs#L525) | once per app launch |
| 151 | app-budget | [app_runtime.rs:737](../../../src/koto-pico/src/firmware/app_runtime.rs#L737) | once per launch (dev only) |
| 152 | app-started | [app_runtime.rs:762](../../../src/koto-pico/src/firmware/app_runtime.rs#L762) | once per launch |
| 153 | app-exited | [app_runtime.rs:824](../../../src/koto-pico/src/firmware/app_runtime.rs#L824) | once per exit |
| 154 | app-heartbeat | [app_runtime.rs:952](../../../src/koto-pico/src/firmware/app_runtime.rs#L952) | every 60 (dev only) |
| 155 | app-draw usage | [diag.rs:188](../../../src/koto-pico/src/firmware/diag.rs#L188) | frame 1 + every 60 + on-overflow (dev only) |
| 156 | app-staged | [app_runtime.rs:2598](../../../src/koto-pico/src/firmware/app_runtime.rs#L2598) | once per launch |
| 157 | pixel-diagnostic | [app_render.rs:150](../../../src/koto-pico/src/firmware/app_render.rs#L150) | once (blit self-test) |
| 158/159 | asset-load-ok / skk-loaded | [app_host.rs:1289](../../../src/koto-pico/src/firmware/app_host.rs#L1289) | on asset/dict load |
| 162 | app-draw-overflow | [diag.rs:460](../../../src/koto-pico/src/firmware/diag.rs#L460) | one-shot on first overflow (dev only) |
| 166 | cw-counters | [app_runtime.rs:414](../../../src/koto-pico/src/firmware/app_runtime.rs#L414) | once on app exit / VM error (feature-gated) |
| 170 | app-static-rebuild | [diag.rs:311](../../../src/koto-pico/src/firmware/diag.rs#L311) | one-shot + cadence on mid-session rebuild |
| 181–199 | error/fault paths | storage / prefs / power / launch | rare (error only) |

### B. Per-frame render cadence (every `C`) — the heavy population

| Phase | Name | Gate | Notes |
|---|---|---|---|
| **160** | app-frame | frame 1 \|\| every `C` | **the regression detector.** ~22 fields incl. `vm_us`, `raster_us`, `transfer_us`, `full`, `full_reason`, `refills`, `fps`, `lat_ms`. |
| 161 | hc (command sample) | frames ≤ 3 only (dev only) | **burst:** one line *per command* for the first 3 frames + a summary. Heaviest transient. |
| 163 | cw (CodeWindow fetch) | every `C` | refill histogram + top transitions + timing. Headline `refills=`/`code_tiles=` already ride `phase=160`. |
| 164 | dirty-rects | every `C`, incremental frames only (dev only) | pre/post-coalesce rect geometry (KOTO-0159). |
| 167 | cw-fast-clkdiv2 | every `C` (feature `psram_fast_code_window`) | fast-read success/fallback counts; **superset overlap with 163's `read_mode`/`chunk`.** |
| 168 | app-budget-obs | frame 1 \|\| every `C` \|\| newly-pressured | observe-only budget/class per command population. |
| 169 | app-cmdshift | CommandCountShift frames: one-shot + every `C` | edit-region shape (GFX-0011). |
| 171 | app-coalesce-decide | coalesce-pressure frames: one-shot + every `C` | coalesce-before-decide contrast (GFX-0010). |
| 173 | audio-summary | every `C` (dev only) | 17-field aggregate audio counters. |
| 174 | app-cmdshift-probe | paired with 169 | raw/coalesced rect+area contrast (GFX-0011). |

### C. Event-driven, off-cadence

| Phase | Name | Gate | Notes |
|---|---|---|---|
| 172 | audio hostcall | **every audio hostcall** (buffered in `host.diag`, drained per frame) | [app_host.rs:327](../../../src/koto-pico/src/firmware/app_host.rs#L327). Can be the highest-rate line during music/SFX. |
| 164 | cw-verify | feature `psram_qpi_code_window_verbose` only | **phase-number reuse** — distinct from the `dirty-rects` 164. |
| 165 | cw-map-verify | feature `psram_qpi_code_window_verbose` only | verbose bring-up only. |

> **Namespace-hygiene note (not fixed here).** Three phase numbers are reused across
> subsystems: `164` (dirty-rects vs cw-verify), `171` (app-coalesce-decide vs the boot
> `audio_pcm_diag` in [koto_firmware.rs:289](../../../src/koto-pico/src/bin/koto_firmware.rs#L289)),
> and `184`/`185` (bridge-register errors vs prefs errors). The reuse is disambiguated by
> subsystem/feature today and is harmless, but a profile that filters *by phase* must key on
> the emit **site/class**, not the raw number. Recorded as a follow-up cleanup, out of scope.

## 2. Classification

Each site tagged with the smallest profile that should still emit it. `C` = render cadence.

| Class | Phases | Rationale |
|---|---|---|
| **always-on** | 131–143, 145–153, 156–159, 162, 166, 181–199, panic/fault | boot sequence, launch, app-exit, ready, fatal, one-shot overflow. The "did it boot / did it crash" spine. Never silenced. |
| **perf-default** | **160**, 168 (thinned to frame-1 + on-pressure) | the regression detector plus the observe-only budget verdict. Enough to spot a perf or budget regression; nothing per-`C` beyond the one headline line. |
| **gfx-debug** | 164, 169, 170, 171, 174 | dirty-rect / coalesce / count-shift / static-rebuild geometry. Only meaningful while investigating a repaint decision. |
| **audio-debug** | 172, 173 | audio hostcall trace + aggregate summary. Only while investigating drops/underruns. |
| **codewindow-debug** | 163, 167 | refill histogram + fast-read counters. Only while investigating PSRAM refill cost. |
| **verbose-only** | 161, 164 cw-verify, 165, 154, 155 | first-frame command dump, CodeWindow verify/map-verify, heartbeat, draw-usage. Bring-up firehose. |
| **event-only** | 158, 159, 170 (one-shot), 172, 162 | fire on a discrete event, not a cadence; ride whichever profile enables their subsystem. Listed to show they are *not* cadence spam. |

Notes:
- `phase=160` stays in **perf-default** because it is the one line whose absence would blind
  a regression run — but it is *also* the line most distorted by the others' transmit cost,
  which is the whole motivation.
- `phase=168` is kept in perf-default but **thinned** to `frame == 1 || newly_pressured`
  (drop the every-`C` sample); sustained pressure is rare and the one-shot already catches
  the onset. This removes a per-`C` line from the default without losing the signal.
- `phase=154`/`155` are demoted to **verbose-only**: `phase=160` already carries frame,
  fps, and the draw peak/overflow counts, so the heartbeat and draw-usage lines are
  redundant for a perf run.

## 3. Duplicate / overlap findings

1. **`phase=168` vs `phase=169` (budget/class).** *Already de-duplicated* by GFX-0011
   Stage 0b: the budget/class fields were **removed** from `phase=169`; `phase=168` is now
   the sole owner of budget/class, and `phase=169` carries only edit-region shape. No
   further overlap — but the classification honors it by putting `168` in perf-default and
   `169` in gfx-debug, so they are never both on unless GFX debugging is selected.

2. **`phase=172` (audio hostcall) vs `phase=173` (audio-summary).** Real overlap: `173`'s
   `audio_events` / `samples_submitted` / `drops` are the running **aggregate** of exactly
   what `172` reports per individual hostcall. `172` is the high-rate event trace; `173` is
   the low-rate rollup. **Recommendation:** both live in **audio-debug**, but `173`
   (the summary) is the default member of that class and `172` (per-call) is gated behind an
   additional `audio-debug + verbose` step, so enabling audio-debug gives the rollup without
   the per-call firehose.

3. **`phase=163` (cw) vs `phase=167` (cw-fast-clkdiv2).** `167` is a strict add-on for the
   `psram_fast_code_window` feature: it re-emits `read_mode` and `chunk` (already on `163`)
   plus fast/fallback counters. **Recommendation:** fold both into **codewindow-debug**;
   the headline `refills=`/`code_tiles=` already ride `phase=160`, so neither `163` nor
   `167` needs to be on for a perf run to see *whether* refills are happening — only to see
   *why*.

Net: after classification the **perf-default** per-`C` population is a single line
(`phase=160`), down from up to nine.

## 4. Recommended profile structure

Six named profiles, additive over the always-on spine. Each profile is a **set of classes**;
selecting a profile emits always-on + that profile's classes.

| Profile | Classes enabled (beyond always-on) | Use |
|---|---|---|
| **Quiet** | *(none)* | pure timing runs / demos; only boot + launch + exit + faults. |
| **Perf** *(default)* | perf-default | normal development + performance smoke — clean `phase=160`. |
| **Gfx** | perf-default + gfx-debug | dirty-rect / coalescing / count-shift investigations. |
| **Audio** | perf-default + audio-debug | drops / underruns / event investigations. |
| **CodeWindow** | perf-default + codewindow-debug | PSRAM refill-cost investigations. |
| **Verbose** | all classes (incl. verbose-only + per-call audio) | bring-up firehose (current behaviour). |

- Profiles are **additive on the always-on spine**, so no profile can hide a boot fault or an
  app crash — that safety is structural, not per-profile opt-in.
- **Cadence is a profile property too.** Quiet/Perf keep the slow `C = 120`; Gfx/Audio/
  CodeWindow use `C = 30` (you want denser samples while debugging); Verbose keeps 30. This
  folds the current `psram_qpi_code_window_prod_profile` cadence swap into the profile,
  decoupled from the PSRAM backend.
- **Default = Perf**, matching the task's suggestion and giving a regression-detecting run
  with no cadence chatter.

## 5. Mechanism recommendation — compile-time firmware constant (enum), not cargo features

**Recommend: a single compile-time constant** in
[config.rs](../../../src/koto-pico/src/firmware/config.rs) —
`pub const DIAG_PROFILE: DiagProfile = DiagProfile::Perf;` — with a `const fn`
`DiagProfile::enables(self, class: DiagClass) -> bool` and a `const fn sample_period(self)`,
and each emit site gated `if DIAG_PROFILE.enables(DiagClass::Gfx) && on_cadence(frame)`.

Why this over the alternatives:

| Option | Verdict | Reason |
|---|---|---|
| **Compile-time const enum** | ✅ **recommend** | `no_std`-clean, **zero RAM** (const-folded), and disabled branches are **dead-code-eliminated** — quieting logs also shrinks `.text`, aiding the stack-headroom discipline. One symbol to change, decoupled from PSRAM. Matches the existing "firmware constant" idiom in config.rs. |
| Cargo features | ⚠️ later, as thin aliases | A `diag_gfx` / `diag_audio` feature *set* is fine as a **build-convenience wrapper** that just sets `DIAG_PROFILE`, but features as the *primary* mechanism invite the combinatorial-explosion + entanglement the current `psram_*_prod_profile` already demonstrates. Keep features as optional aliases over the const, not the source of truth. |
| Runtime config (prefs / key chord) | ❌ defer to Stage 3 | Adds a config surface, RAM, and a UART-parse/store path for a benefit (switch profile without reflashing) that smoke runs don't need. Costs the very RAM the firmware guards. Only worth it if on-device bring-up proves reflashing is a real bottleneck. |
| Plain `#[cfg]` scatter (status quo) | ❌ | what we have; no central switch, welded to backend selection. |

Consequence to call out: because it is compile-time, **changing profile means a rebuild+reflash**.
That is acceptable for the target workflow (perf/smoke runs are already rebuild-gated) and is
the price of zero RAM + dead-code elimination. Stage 3 adds runtime selection only if needed.

## 6. Staged implementation plan

Each stage is independently shippable and behaviour-preserving at its boundary.

### Stage 0 — this proposal (no code)
Inventory + classification + design above. Lands the doc only.

### Stage 1 — scaffolding + route the heavy cadence lines (first patch)
Introduce `DiagProfile` / `DiagClass` (an enum + two `const fn`s) in config.rs, plus a tiny
`diag::on_cadence(frame)` helper reading `DIAG_PROFILE.sample_period()`. Route **only** the
per-`C` GFX/audio/CodeWindow lines through the class gate — `phase=163, 164, 167, 169, 171,
173, 174` — and thin `phase=168` to frame-1 + on-pressure. Leave `phase=160` in
perf-default and every always-on site untouched. Default `DIAG_PROFILE::Perf`.
- **Byte-identical escape hatch:** `DiagProfile::Verbose` reproduces today's exact emit set +
  the 30-frame cadence, so a Verbose build is a regression-safe A/B against `main`.
- Replace the `psram_qpi_code_window_prod_profile` **cadence** swap (30↔120) with
  `sample_period()`; leave that feature owning **only** PSRAM backend selection.
- Re-run the `.text`/`.bss`/stack `llvm-size` check per the firmware stack-headroom
  discipline — expect `.text` to *shrink* under Perf (dead branches eliminated), never grow.

### Stage 2 — route the remaining sites + collapse the audio duplicate
Gate `phase=161, 154, 155` (verbose-only) and `phase=172` (audio per-call, behind
audio-debug+verbose) through the same mechanism; make `phase=173` the default audio-debug
member. Fold the remaining `#[cfg(not(feature = "psram_qpi_code_window_prod_profile"))]`
diagnostic guards onto `DIAG_PROFILE` so the PSRAM feature no longer gates any *log*. No new
formats; pure re-gating.

### Stage 3 — (deferred, only if needed) runtime selection
A prefs field or a boot key-chord that overrides `DIAG_PROFILE` at runtime, if on-device
bring-up shows reflash-to-reprofile is a real friction. Costs RAM + a config path; not
proposed now.

### Out of scope (recorded)
- Renaming the reused phase numbers (164/171/184/185) — hygiene follow-up.
- Any change to a diagnostic's **fields or format** (this issue only gates *whether* a line
  emits, never *what* it says).
- Any rendering / VM / hostcall / ABI / APP_DRAW / PSRAM / LCD / CodeWindow / audio /
  CPU-ownership behaviour change.

## Non-goals

- **No behaviour change** to rendering, VM, app bytecode, hostcalls, ABI, APP_DRAW, PSRAM,
  LCD, CodeWindow, audio mixing/playback, or CPU ownership. This is a pure log-gating change.
- **No diagnostic is deleted** — every phase line survives; profiles only decide whether it
  transmits.
- No change to any diagnostic's field set or wire format (that is what keeps host-side
  format tests green unchanged).
- No coupling of verbosity to PSRAM backend selection (the opposite — Stage 1/2 *removes* the
  existing coupling).
- Always-on (boot / launch / app-exit / ready / fatal / panic) stays emitting under every
  profile including Quiet.

## Acceptance criteria

- [ ] **Inventory:** every current `phase=` emit site is listed with its gate/frequency
      (§1), including the event-driven `phase=172` and the first-frames `phase=161` burst.
- [ ] **Classification:** each site is tagged always-on / perf-default / gfx-debug /
      audio-debug / codewindow-debug / verbose-only / event-only (§2), with `phase=160`
      in perf-default and the always-on spine unsilenceable.
- [ ] **Duplicates:** the three overlaps are addressed — `168`/`169` (already split;
      honored by class placement), `172`/`173` (summary is the default audio member, per-call
      gated behind verbose), `163`/`167` (both codewindow-debug; headline rides `160`) — §3.
- [ ] **Profile structure:** six additive profiles (Quiet/Perf/Gfx/Audio/CodeWindow/Verbose)
      over the always-on spine, cadence folded in, default Perf (§4).
- [ ] **Mechanism:** compile-time const enum in config.rs recommended over cargo features /
      runtime config, with the rebuild-to-reprofile tradeoff stated and the RAM/`.text`
      argument given (§5).
- [ ] **Staged plan:** Stage 1 (scaffold + heavy cadence lines, Verbose = byte-identical
      A/B), Stage 2 (remaining sites + audio-duplicate collapse + un-weld the PSRAM feature),
      Stage 3 (deferred runtime selection) — §6.
- [ ] **First-patch recommendation** named (§6 Stage 1 + §7 below).
- [ ] Enough default telemetry to detect regressions: Perf still emits `phase=160` +
      thinned `phase=168` + the always-on spine.

## Acceptance tests / hardware smoke plan

**Host-side (`cargo test -p koto-pico`, once Stage 1 lands):**
1. `DiagProfile::enables` truth-table test: `Perf` enables perf-default, disables gfx/audio/
   codewindow/verbose; `Gfx` enables perf-default + gfx-debug and nothing else; `Verbose`
   enables all; `Quiet` enables none. Always-on is independent of the profile.
2. `sample_period()` returns the expected cadence per profile (Perf=120, Gfx/Audio/CodeWindow
   =30, Verbose=30).
3. **Format tests unchanged:** the existing `phase=160/164/168/169/171/174` formatter tests
   still pass byte-for-byte (the formatters are untouched; only the call sites are gated).
4. A `Verbose`-profile build enumerates the *same* set of enabled emit sites as today's dev
   build (regression guard that Verbose is a faithful A/B baseline).

**Firmware build gates:**
5. `thumbv6m-none-eabi` build green under each of the six profiles (compile-time coverage).
6. `llvm-size` on release `koto_firmware`: `.bss` unchanged (±0, no new state), `.text` under
   `Perf` **≤** current (dead-branch elimination), captured per profile. Clippy adds no new
   finding (`-p koto-pico --target thumbv6m-none-eabi --bins`, per the firmware clippy note).

**Hardware smoke (KotoSnake / KotoBlocks, where a device is available):**
7. **Perf quiet-line proof.** Boot + launch KotoSnake under `Perf`. Confirm the only per-frame
   UART traffic is `phase=160` (+ thinned `phase=168`), and that boot spine (`131`→`152`),
   `phase=153` on exit, and any fault line still appear. Confirm `phase=160`'s own
   `vm_us`/`work_us` for a steady frame are **lower / less noisy** than the same frame under
   the current firehose — the concrete NFR-PERF-1 win (measure the VM spike without the
   diagnostics distorting it).
8. **Profile completeness.** Under `Gfx`, confirm `phase=164/169/171/174` return and classify
   a KotoSnake CommandCountShift frame exactly as GFX-0011 documented — i.e. no diagnostic was
   *lost*, only gated. Under `Audio`, confirm `phase=173` returns (and `172` only with the
   verbose step). Under `CodeWindow`, confirm `phase=163` returns.
9. **Attribution parity.** `phase=160` `full`/`full_reason` counts for a fixed scripted
   session are identical across profiles — gating a log must change no decision (mirrors the
   GFX-0011 observe-only invariant).

## 7. First implementation patch (recommendation)

Land Stage 1 as the first patch, scoped to the **maximum measurement win for minimum churn**:

1. Add `DiagProfile` (enum: Quiet/Perf/Gfx/Audio/CodeWindow/Verbose) + `DiagClass` +
   `enables()` / `sample_period()` `const fn`s + `DIAG_PROFILE: DiagProfile = Perf` to
   config.rs; add `diag::on_cadence(frame)`.
2. Gate the seven per-`C` render/audio/CodeWindow lines (`163, 164, 167, 169, 171, 173, 174`)
   through their class + `on_cadence`, and thin `phase=168` to frame-1 + on-pressure. Leave
   `phase=160` and every always-on site alone.
3. Point the cadence at `sample_period()`; leave `psram_qpi_code_window_prod_profile` owning
   only the PSRAM backend.
4. Add host truth-table + cadence tests (tests 1–4 above); confirm the format tests stay
   green; capture the `llvm-size` deltas.

This immediately delivers the stated goal — a clean `phase=160` for profiling the VM-side
spike — while `DiagProfile::Verbose` remains a byte-identical fallback to today, so the patch
is provably behaviour-neutral where it matters. Stage 2 then mops up the verbose-only lines
and the audio duplicate, and un-welds the last diagnostic guards from the PSRAM feature.
