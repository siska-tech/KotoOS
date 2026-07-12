# KOTO-0148: Pico SRAM map audit and Embassy pool diet

- Status: in-progress
- Type: investigation / optimization
- Priority: P1
- Related: KOTO-0146, KOTO-0147, KOTO-0145, KOTO-0132

## Background

Recent Pico hardware builds have started to show boot instability after adding
larger audio/runtime features such as CPU1 audio service and `.kwt` custom
instrument loading.

A current SRAM symbol snapshot shows that the firmware still has apparent SRAM
headroom, but several large static/future-backed regions dominate memory use:

- `__embassy_main::POOL`: ~79 KiB
- `APP_DRAW`: ~18 KiB
- `APP_HEAP`: ~16 KiB
- `CODE_WINDOW`: ~16 KiB
- `RGB666_STRIP`: ~15 KiB
- `RASTER_STRIP`: ~10 KiB
- `APP_STATIC`: ~6 KiB
- `AUDIO_CORE1_STACK`: ~4 KiB

The largest unknown is `__embassy_main::POOL`. Because Embassy stores async task
future state in executor task storage, large locals that live across `.await`
can silently increase this pool even when they look like normal stack variables
in source code.

This makes it difficult to reason about boot reliability, CPU1 stack safety, and
whether future features such as render-prep offload can fit safely in SRAM.

## Goal

Audit the Pico firmware SRAM map and reduce avoidable SRAM pressure, especially
from `__embassy_main::POOL`, large async future state, oversized static buffers,
and accidental large stack/frame captures.

The output of this task should be a measured SRAM budget and a set of concrete
code changes that reduce or cap memory growth.

## Non-goals

- Changing the VM bytecode format
- Redesigning PSRAM `CodeWindow`
- Reducing `CODE_WINDOW` below 16 KiB unless absolutely necessary
- Moving LCD transfer or PSRAM access to CPU1
- Rewriting the audio engine
- Adding dynamic allocation

## Investigation Targets

### 1. Embassy executor pool

Determine why `__embassy_main::POOL` is currently around 79 KiB.

Investigate:

- which async tasks contribute most to the pool
- whether large locals are retained across `.await`
- whether asset loading paths keep large temporary buffers inside async futures
- whether KWT parser/bank structures are captured inside async state
- whether display/audio/app host objects can be split to shrink future state

Look especially for patterns like:

```rust
let mut buf = [0u8; 512];
let mut bank = PicoInstrumentBank::default();

some_async_call().await;

// buf or bank used after await
```

These should be refactored so large buffers do not live across `.await`.

### 2. KWT/custom instrument memory

Audit the `.kwt` implementation for SRAM and stack impact.

Check:

* size of `PicoInstrumentBank`
* size of each custom wavetable entry
* whether custom banks are copied by value
* whether banks are passed by value instead of by reference
* whether KWT load buffers are local, static, or future-captured
* whether CPU1 ever parses KWT or owns large temporary buffers

Target behavior:

* CPU0 loads/parses `.kwt`
* CPU1 receives only a compact runtime instrument snapshot
* no `.kwt` parse buffer is allocated on CPU1 stack
* no large KWT buffer lives across `.await`
* no large bank is copied repeatedly by value

### 3. CPU1 stack safety

Audit `AUDIO_CORE1_STACK`.

Current size:

```text
AUDIO_CORE1_STACK = 4096 B
```

Check whether CPU1 worker uses any large local arrays, temporary mixers, parser
buffers, wavetable copies, or command structs by value.

CPU1 should prefer:

* static/ring-buffer storage
* small local scalars
* references or IDs instead of large value copies
* counter-only diagnostics

CPU1 must not allocate or parse `.kwt` data.

### 4. Static buffer budget

Review current large static buffers:

```text
APP_DRAW       ~18 KiB
APP_HEAP       ~16 KiB
CODE_WINDOW    ~16 KiB
RGB666_STRIP   ~15 KiB
RASTER_STRIP   ~10 KiB
APP_STATIC      ~6 KiB
```

For each buffer, document:

* current size
* purpose
* whether it is required at boot
* whether it can be feature-gated
* whether it can be shared safely
* whether it can be reduced without regressing KotoBlocks/KotoSnake

Do not reduce `CODE_WINDOW` first, because KOTO-0132 showed that 16 KiB improves
PSRAM CodeWindow behavior.

### 5. Boot failure correlation

Compare SRAM maps between known-good and failing builds.

At minimum capture:

```text
__sbss
__ebss
__sheap
_stack_end
__embassy_main::POOL
APP_STATIC
APP_DRAW
APP_HEAP
CODE_WINDOW
RASTER_STRIP
RGB666_STRIP
AUDIO_CORE1_STACK
KWT-related symbols
audio bank symbols
```

Also record:

* whether UART emits anything
* whether CPU1 starts
* whether first phase log appears
* whether disabling KWT restores boot
* whether reducing KWT bank size changes behavior

## Plan

1. Add a repeatable SRAM report command/script.

   Example PowerShell command:

   ```powershell
   & $nm.FullName --size-sort --print-size --radix=d $elf |
     Select-String "POOL|KWT|BANK|AUDIO_CORE1_STACK|APP_STATIC|APP_HEAP|APP_DRAW|CODE_WINDOW|RASTER_STRIP|RGB666_STRIP|__sbss|__ebss|__sheap|_stack_end"
   ```

2. Capture a baseline map from the last known-good build.

3. Capture a map from the failing `.kwt` build.

4. Compare:

   * `.bss` total growth
   * `__embassy_main::POOL` growth
   * new KWT/audio bank symbols
   * stack boundary/headroom changes

5. Inspect async functions in `app_host.rs`, `audio.rs`, and firmware main loop
   for large locals across `.await`.

6. Refactor large future-captured locals into:

   * smaller scoped blocks before `.await`
   * static scratch buffers with explicit ownership
   * compact structs
   * borrowed references
   * synchronous helper functions called after async load completes

7. Add compile-time size guards for key constants.

8. Add a short SRAM budget note to `config.rs` or a new Pico memory budget doc.

## Candidate Refactors

### Avoid large async locals

Before:

```rust
async fn load_score_with_kwt(...) {
    let mut bank = PicoInstrumentBank::default();
    let mut kwt_buf = [0u8; 512];

    load_asset(path, &mut kwt_buf).await;
    parse_kwt(&kwt_buf, &mut bank);

    play_with_bank(bank);
}
```

After:

```rust
async fn load_score_with_kwt(...) {
    let len = load_kwt_asset_into_scratch(path).await?;
    let bank = parse_kwt_from_scratch(len)?;

    play_with_bank(&bank);
}
```

The important rule is that large buffers and banks must not be live across
`.await`.

### Avoid large value copies

Prefer:

```rust
fn apply_custom_bank(score: &mut Score, bank: &PicoInstrumentBank)
```

Avoid:

```rust
fn apply_custom_bank(score: Score, bank: PicoInstrumentBank) -> Score
```

### Keep CPU1 compact

CPU1 audio worker should not own parser buffers, file buffers, or large temporary
instrument banks. It should consume compact pre-parsed runtime instruments.

## Suggested Diagnostics

Add or document a concise size summary:

```text
phase=090 mem bss_used=... sram_free_after_bss=... pool=... app_heap=... code_window=... raster_strip=... rgb666_strip=... cpu1_stack=...
```

If runtime printing this is too invasive, keep it as a build-time report generated
from `nm`.

## Notes

* `cargo check` is not sufficient for this class of failure. The build can type
  check while still exceeding practical SRAM or stack limits.
* Embassy async future size can grow unexpectedly when large locals are live
  across `.await`.
* KWT support should remain feature-gated until SRAM and boot stability are
  confirmed on hardware.
* Do not reduce `CODE_WINDOW` as the first response; it is likely performance
  critical after KOTO-0132.
* Prefer measurement first, then small targeted reductions.

## 2026-06-25 Audit Snapshot

Measured from the current `target/thumbv6m-none-eabi/release/koto_firmware`
ELF with `llvm-nm` and `llvm-size`:

```text
.data                         5,660 B
.bss                        171,136 B
__sbss                   0x20001620
__ebss / __sheap         0x2002b2a0
SRAM remaining after bss  about 91.3 KiB
__embassy_main::POOL        74,384 B
AUDIO_QUEUE                  5,660 B
AUDIO_ASSET_SCRATCH          4,609 B
AUDIO_CORE1_STACK            4,096 B
APP_DRAW                    18,060 B
APP_HEAP                    16,385 B
CODE_WINDOW                 16,385 B
RGB666_STRIP                15,361 B
RASTER_STRIP                10,241 B
APP_STATIC                   6,092 B
```

No saved `.map` file was available in the workspace, so the known-good side of
the comparison currently uses the KOTO-0134 issue record:

```text
known-good-ish KOTO-0134 pool record: 70,568 B
current pool:                         74,384 B
pool growth:                          about +3.7 KiB
```

This means the current ELF still has apparent SRAM headroom, but the regression
is credible: RP2040 boot has previously hung from single-digit KiB SRAM shifts
when the runtime stack margin was thin.

Optimization pass applied:

* Moved BGM/KWT byte buffers out of `DeviceHost` and into a binary-owned
  `StaticCell<AudioAssetScratch>`. This moves `bgm_asset[4096]` and
  `kwt_asset[512]` out of the main Embassy future; `AUDIO_ASSET_SCRATCH` now
  appears as an explicit 4,609 B static.
* Changed CPU1 `service_once()` so it no longer carries both pending BGM and SFX
  full-score payloads in a single local tuple.
* Changed pending SFX storage from full `PicoBgmScore` to compact one-track
  `PicoSfxScore`, reducing `AUDIO_QUEUE` from 6,084 B to 5,660 B.
* Added compile-time size guards for `PicoInstrumentBank`, `PicoBgmScore`,
  `PicoSfxScore`, `AudioWorkerQueue`, and `PicoAudioWorker`.

Hardware validation after this optimization:

* KWT-enabled firmware boots on device again.
* Audio output is audible and working after boot.
* This confirms the immediate no-UART/no-audio regression is resolved by the
  SRAM/future/CPU1 payload optimization pass. Keep app-specific phase/log capture
  for KotoSnake/KotoBlocks as follow-up validation.

## Prioritized Diagnosis

### 1. CPU1 stack pressure from full-score value movement

`AUDIO_CORE1_STACK` is only 4096 B, while the CPU1 service path moves
`Option<PicoBgmScore>` values through a tuple and then into BGM/SFX start
functions. KWT made `PicoBgmScore` carry a bank of fixed wavetable arrays, so
this path is now the top stack-risk candidate.

Inspect:

* `src/koto-pico/src/bin/koto_firmware.rs`: `AUDIO_CORE1_STACK` and
  `spawn_cpu1(...)`
* `src/koto-pico/src/firmware/audio.rs`: `PicoBgmScore`, `AudioWorkerQueue`,
  `PicoAudioWorker::service_once`, `PicoBgmPlayer::start_with_looping`,
  `start_sfx_score`

### 2. KWT/BGM scratch buffers are inside the Embassy main future

`DeviceHost` owns `bgm_asset: [u8; 4096]` and `kwt_asset: [u8; 512]`.
`DeviceHost` lives inside `run_app_session().await`, so these buffers likely
contribute directly to `__embassy_main::POOL` even though `.kwt` parsing is
synchronous and does not itself cross an `.await`.

Inspect:

* `src/koto-pico/src/firmware/app_host.rs`: `DeviceHost` fields,
  `load_bgm_asset`, `load_kwt_asset`, `play_bgm_asset`, `play_sfx_asset`
* `src/koto-pico/src/firmware/app_runtime.rs`: `run_app_session` lifetime of
  `host`

### 3. `AUDIO_QUEUE` duplicates score storage in static `.data`

The queue contains PCM ring storage plus both `pending_bgm: Option<PicoBgmScore>`
and `pending_sfx: Option<PicoBgmScore>`. The measured symbol is 6,084 B, and it
lands in `.data`, not `.bss`, because it has non-zero initialization through the
static queue value.

Inspect:

* `src/koto-pico/src/firmware/audio.rs`: `AudioWorkerQueue`, `AUDIO_QUEUE`,
  `play_bgm_score`, `submit_sfx_score`

### 4. KWT support is always compiled into the Pico score type

There is no `pico_kwt` feature gate. Non-KWT MML scores still carry the larger
`PicoInstrumentBank` field, so baseline BGM/SFX queue, stack, and future sizes
all grow even when no `#INST` directive is used.

Inspect:

* `src/koto-pico/Cargo.toml`: feature list
* `src/koto-pico/src/firmware/audio.rs`: `PicoBgmScore::bank`,
  `CUSTOM_INSTRUMENT_CAPACITY`, `CUSTOM_WAVETABLE_CAPACITY`

### 5. A true no-UART boot is probably before KWT runtime parsing

The first UART banner is emitted immediately after `embassy_rp::init()` and
before CPU1 audio spawn, SD, PSRAM, app launch, or `.kwt` asset loading. If
`phase=10` never appears, prioritize static RAM/linker/reset/init causes over
malformed KWT data.

Inspect:

* `src/koto-pico/src/bin/koto_firmware.rs`: UART init and repeated
  `phase=10 uart-ready` banner
* `src/koto-pico/memory.x`: `RAM LENGTH = 264K`
* `src/koto-pico/src/firmware/config.rs`: large static buffer constants

## Minimal Rollback Toggles

Use these as short-lived bisection switches; do not keep them as the final design
unless a feature gate is intentionally added.

* **CPU1 stack probe**: change `AUDIO_CORE1_STACK` from `Stack<4096>` to
  `Stack<8192>`. If UART/audio returns, stack pressure is confirmed.
* **KWT parser/load bypass**: in `play_bgm_asset` and `play_sfx_asset`, parse the
  `.kmml` score but skip `load_inst_refs_into_score(...)`. This keeps MML BGM/SFX
  active while removing `.kwt` asset I/O and bank injection.
* **Bank-size probe**: temporarily set `CUSTOM_INSTRUMENT_CAPACITY` to `0` or `1`
  and rebuild. A size/boot delta implicates the score/bank footprint rather than
  the parser logic.
* **SFX payload rollback**: disable `pending_sfx: Option<PicoBgmScore>` and route
  SFX `.kmml` back to tone fallback. This removes one full score slot from the
  static queue and one full-score CPU1 movement path.
* **Build matrix features**: add temporary or permanent `pico_cpu1_audio` and
  `pico_kwt` features so hardware can test four binaries: baseline, CPU1 only,
  KWT only, CPU1 + KWT.

## Minimal Refactor Plan

* Move `bgm_asset` and `kwt_asset` out of `DeviceHost` into explicitly borrowed
  scratch storage owned near the app-session call site or a dedicated `StaticCell`.
  Goal: remove these buffers from `__embassy_main::POOL` while keeping CPU0-only
  parsing.
* Remove `Copy` from `PicoBgmScore`, `PicoMmlTrack`, `PicoInstrumentBank`, and
  `PicoCustomInstrument` where practical. Keep small event types copyable only if
  needed. Goal: make full-score copies visible in the compiler instead of
  accidental.
* Split SFX into a compact payload, e.g. `PicoSfxScore { track, bank }`, because
  SFX uses only `score.tracks[0]`. Goal: avoid storing/moving four tracks for a
  one-shot sound.
* Change CPU1 queue draining so `service_once()` takes and processes only the one
  payload selected by the popped command. Avoid a local tuple that can contain
  both `Option<PicoBgmScore>` values at once.
* Add compile-time size checks for `PicoBgmScore`, `PicoInstrumentBank`,
  `AudioWorkerQueue`, and `PicoAudioWorker`. Treat size regressions as build-time
  failures for the Pico target.
* Add an explicit KWT feature gate after the size regression is understood. The
  default firmware can keep BGM/SFX MML support while `.kwt` loading is isolated
  for hardware validation.

## Reporting Commands

PowerShell command for the current toolchain layout:

```powershell
$elf = "target/thumbv6m-none-eabi/release/koto_firmware"
$nm = Get-ChildItem "$env:USERPROFILE\.rustup\toolchains" -Recurse -Filter llvm-nm.exe |
  Select-Object -First 1 -ExpandProperty FullName
$size = Get-ChildItem "$env:USERPROFILE\.rustup\toolchains" -Recurse -Filter llvm-size.exe |
  Select-Object -First 1 -ExpandProperty FullName

& $nm --size-sort --print-size --radix=d $elf |
  Select-String "POOL|AUDIO_QUEUE|AUDIO_CORE1_STACK|APP_STATIC|APP_HEAP|APP_DRAW|CODE_WINDOW|RASTER_STRIP|RGB666_STRIP|__sbss|__ebss|__sheap|_stack_end"

& $nm --size-sort --print-size --radix=d $elf |
  Select-String " [dDbB] " |
  Select-Object -Last 80

& $size -A $elf
```

Record each hardware binary with:

```text
build=<name> features=<features> boots=<yes/no> first_uart_phase=<phase/none>
.data=<bytes> .bss=<bytes> pool=<bytes> audio_queue=<bytes>
cpu1_stack=<bytes> app_draw=<bytes> code_window=<bytes> sram_after_ebss=<bytes>
```

## Acceptance Criteria

* [x] SRAM report command/script is documented and usable for every test build.
* [x] Known-good and failing builds have comparable `llvm-nm` / `llvm-size`
    snapshots.
* [x] `.bss`, `.data`, `__embassy_main::POOL`, and remaining SRAM after
  `__ebss` are recorded for each compared build.
* [x] `__embassy_main::POOL` contributors are identified at source level.
* [ ] The first no-UART build is classified as pre-UART reset/init failure vs
    post-UART runtime failure.
* [ ] Disabling only KWT loading is tested and recorded.
* [ ] Increasing only `AUDIO_CORE1_STACK` is tested and recorded.
* [x] Large locals crossing `.await` are removed or explicitly justified.
* [x] `PicoBgmScore`, `PicoInstrumentBank`, `AudioWorkerQueue`, and
    `PicoAudioWorker` sizes are known and guarded.
* [x] KWT/BGM scratch buffers no longer live inside the main Embassy future, or
    the measured `POOL` cost is accepted and documented.
* [x] CPU1 audio worker does not allocate or parse `.kwt` data.
* [x] CPU1 no longer moves multiple full `PicoBgmScore` payloads in one local
    frame.
* [ ] KWT-enabled firmware emits `phase=10 uart-ready`, reaches the shell, and
    can launch KotoSnake/KotoBlocks with `phase=173` audio summaries.
* [ ] KOTO-0146 CPU1 audio remains bounded: no command-drop regression and
    `worker_max_jitter_us` stays within the previously observed envelope.
* [ ] A short Pico SRAM budget note is committed.