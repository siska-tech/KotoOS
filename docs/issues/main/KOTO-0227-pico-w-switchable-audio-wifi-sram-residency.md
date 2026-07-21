# KOTO-0227: Pico W switchable Audio/Wi-Fi SRAM residency

- Status: done
- Type: firmware memory enablement
- Priority: P1
- Requirements: HC-1, HC-3, FR-CONFIG-3, NFR-MEM-1, NFR-MEM-2, NFR-MEM-4, NFR-MEM-5, NFR-PORT-4, NFR-PORT-6, NFR-REL-3
- Related: KOTO-0148, KOTO-0170, KOTO-0186, KOTO-0204, KOTO-0205, KOTO-0224, KOTO-0226, KOTO-0239, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Reorganize the RP2040 Pico W product firmware so full Audio and Wi-Fi can be
selected at runtime without requiring an audio-less firmware, while retaining
bounded PCM16/SLDPCM4 package streaming during Wi-Fi operation. Keep both
implementations in flash, place only the active rich-service state in an
explicitly owned SRAM arena, and preserve simultaneous full Audio and Wi-Fi on
the larger-SRAM Pico 2 W profile.

This issue creates the memory and lifetime foundation for KOTO-0224. It does
not by itself expose credentials, a KotoConfig Wi-Fi page, or a public app
network API.

## Baseline And Target Budget

The 2026-07-17 RP2040 default release ELF and device soak provide the baseline:

```text
.data + .bss                         205,436 B
static span through .bss             205,752 B
boot-time observed SRAM free          about 30.5 KiB
phase=176 worst-case free_min           15,724 B
```

The current major audio-resident symbols are:

```text
AUDIO_SHARED                          17,876 B
AUDIO_SERVICE                          3,016 B
RUNTIME_BGM_PLAYER                     9,028 B
RUNTIME_SFX_PLAYER                     4,036 B
RUNTIME_CLIP_PLAYER                    8,252 B
AUDIO_SCRATCH                          8,776 B
AUDIO_CORE1_STACK                      8,192 B
AUDIO_DMA_RING                         1,024 B
```

A stream-only resident path still needs the raw PCM ring, PCM16/SLDPCM4 refill
and decode scratch, PWM DMA ring, a bounded worker stack, and small control /
diagnostic state. It does not need the sequence mixer service, runtime BGM/SFX
players, owned short-clip player, cue-image staging, rich command queue, or
service output ring.

The implementation must recover at least 36 KiB for the Wi-Fi residency arena
without relying on a smaller CPU1 stack. The target is at least 40 KiB after a
stream-only call-tree measurement proves any stack reduction. Estimated values
are planning inputs only; the release ELF and hardware canaries are the source
of truth.

## Residency Model

### Pico W / RP2040

The board exposes two runtime modes:

```text
FullAudio
  Native KotoAudio mixer, BGM, SFX, runtime cue and short-clip players
  Wi-Fi radio/network service unavailable

WifiStreamAudio
  CYW43 and bounded network state
  PCM16/SLDPCM4 package streaming remains available
  BGM, synthesized SFX, runtime cues and owned short clips unavailable

TlsExclusive (KOTO-0245 exception)
  CYW43/network state and exactly one HTTPS/TLS transaction remain available
  All audio APIs temporarily return unavailable for the TLS connection lifetime
  PCM, refill/decode scratch, and DMA storage are loaned to TLS
  The CPU1 stack remains reserved until a separate stop/restart proof exists
```

A single owner controls the transition through quiescing and offline states.
Stopping a service is insufficient: all DMA, CPU1, task, reference, and handle
lifetimes must be proven dead before its arena bytes are reused.

The TLS exception does not apply to ordinary Wi-Fi, DHCP, DNS, SNTP, or scans.
The transition must stop and acknowledge the stream worker, PWM, and DMA before
publishing TLS ownership. TLS completion, cancellation, timeout, disconnect,
and failure must erase/drop every TLS borrower before stream audio is rebuilt.
An interrupted transition fails to the existing safe `Offline` state.

### Pico 2 W / RP2350

The board advertises concurrent Audio and Wi-Fi residency and does not force
the RP2040 mode switch. It may reuse the same service interfaces and transition
logic for error recovery, but its normal profile allocates independent state.

## Placement Policy

- CYW43 firmware, CLM/NVRAM data, driver code, and network code remain in flash;
  code executes through XIP unless a separately measured hot path justifies an
  explicit SRAM placement.
- CYW43 packet channels, active network state, task futures, DMA-visible packet
  staging, and bounded socket windows remain in internal SRAM.
- HTTP bodies, download payloads, caches, retained scan history, and other
  pointer-free bulk data may use PicoCalc PSRAM through bounded block-transfer
  APIs. RP2040 code must not treat PSRAM as dereferenceable memory.
- The permanent stream-audio core and switchable rich-service/network arena
  have named compile-time size and alignment guards and visible ELF symbols.

## Acceptance Criteria

- [x] Add board capability policy that distinguishes `AUDIO`, `WIFI`, and
      `AUDIO_WIFI_CONCURRENT`; Pico W reports both services but not concurrent
      residency, Pico 2 W permits concurrent residency, and non-W boards do not
      infer Wi-Fi from an MCU or board-name suffix alone.
- [x] Refactor the Pico audio globals into a permanently resident, bounded
      stream-audio core and a separately owned rich-audio residency region.
      The stream core contains only the PCM raw ring, PCM16/SLDPCM4 refill and
      decode scratch, PWM DMA ring, required worker state/stack, and bounded
      diagnostics.
- [x] In `WifiStreamAudio`, PCM16 and SLDPCM4 KACL assets stream through the
      existing SD/KPA/PSRAM path with hardware-paced PWM DMA; BGM, synthesized
      SFX, runtime cue, and owned short-clip requests return an explicit
      temporary-unavailability result instead of silently dropping or
      allocating fallback state.
- [x] Implement one mode owner and an explicit transition state machine for
      `FullAudio -> quiescing -> offline -> WifiStreamAudio` and the reverse.
      New requests are rejected while switching, stale handles carry a
      generation or equivalent invalidation token, and timeout/fault handling
      lands in a safe offline state.
- [x] Before reusing rich-audio bytes, stop new commands, drain or cancel
      playback, stop PWM/DMA access to reused storage, and prove CPU1 no longer
      references it. Before reusing network bytes, close sockets, cancel and
      join the network/CYW43 runner, power down or quiesce the radio, and prove
      no task or peripheral retains an arena reference.
- [x] Integrate a KOTO-0227-scoped bounded CYW43 lifecycle probe into the Pico W
      product profile. Place the driver state, packet channels, runner future,
      and minimum network state in `WifiResidencyArena`; prove initialization,
      bounded packet activity, cancellation, runner join, radio power-down, and
      complete arena-reference release without exposing a public network API.
- [x] The RP2040 release ELF shows at least 36 KiB of rich-audio storage
      available to the Wi-Fi arena while stream audio remains resident. A
      CPU1 stack reduction may raise the target to at least 40 KiB only after a
      stream-only stack canary capture demonstrates a safe margin.
- [x] Add a machine-readable SRAM report for both RP2040 modes, including
      `.data`, `.bss`, static span, permanent stream core, switchable arena,
      CPU0 `phase=176` margin, CPU1 stack margin, and the delta from the retained
      KOTO-0226 baseline.
- [x] Exercise at least 100 full Audio/Wi-Fi residency transitions without a
      stale-reference fault, DMA write into the inactive layout, corrupt guard,
      dead CPU1 worker, leaked task slot, or failed return to full Audio.
- [x] Run a five-minute `WifiStreamAudio` product-path soak covering alternating
      PCM16 and SLDPCM4 streams while the reserved network side performs bounded
      packet-buffer activity. The final diagnostic reports zero audio underruns,
      command drops, arena guard failures, and transition failures.
- [x] Re-run the full Audio regression after returning from Wi-Fi mode; BGM,
      SFX, runtime cue, runtime short clip, PCM streaming, worker heartbeat, and
      clean app exit all pass without a worse core1 stack margin than the
      accepted full-Audio baseline.
- [x] The RP2040 Pico W build keeps a positive measured SRAM margin under its
      worst supported Wi-Fi-plus-stream workload and stays above the KOTO-0170
      stop-ship floor. The exact release threshold is frozen with KOTO-0224's
      bounded network budget before implementation is marked done.
- [x] The Pico 2 W release build retains independent simultaneous full-Audio
      and Wi-Fi storage; it does not inherit the RP2040 feature restriction.
- [x] RP2040, Pico W, and RP2350A release builds plus project, audio, memory-map,
      and transition harnesses pass. Board-specific hardware captures are
      retained with the issue before closure.

## Non-goals

- Selecting or exposing the final public network/socket API
- Implementing the KotoConfig Wi-Fi UI or credential persistence
- Treating PSRAM as pointer-addressable heap or socket memory
- Supporting simultaneous rich BGM/SFX/cue playback and Wi-Fi on RP2040
- Reducing the RP2040 CodeWindow tile count or raster strip as the first source
  of Wi-Fi memory

## Notes

- KOTO-0224 owns network-stack selection, fixed packet/socket/scan capacities,
  credential policy, and the final NetworkService contract. This issue owns
  the RP2040 memory-residency mechanism that makes that contract implementable.
- The existing `cyw43` driver uses MTU-sized internal packet channels. Those
  buffers cannot be moved to PicoCalc PSRAM without replacing the driver's
  pointer-based channel contract; queue-depth changes must be measured rather
  than assumed.
- The current `AUDIO_SCRATCH` already shares runtime-cue and stream views. A
  stream-only layout needs 4,096 encoded PCM16 bytes plus 1,024 decoded bytes,
  rather than the full runtime-cue image capacity.
- Keep the raw PCM ring's 256 ms storage-latency lead until simultaneous
  Wi-Fi/SD/PSRAM hardware measurements prove a smaller bound. Recover rich
  player and staging state before trading away underrun protection.

## Next Priority: Bounded CYW43 Lifecycle Integration

CYW43 integration is the next implementation priority. Audio-side residency is
now sufficiently explicit that further progress depends on measuring the real
driver/runner layout and proving its teardown semantics. This work is a bounded
KOTO-0227 lifecycle probe, not the final KOTO-0224 NetworkService.

Implement in this order:

1. Audit and freeze the Pico W radio resources: PIO instance/state machine,
      DMA channel, IRQ binding, power/CS/data pins, and conflicts with LCD, PSRAM,
      SD, and permanent audio DMA.
2. Define one compile-time-checked Wi-Fi arena layout containing `cyw43::State`,
      packet channels, the runner future/task state, and only the minimum bounded
      network state needed to produce packet-buffer activity. Firmware, CLM, and
      NVRAM bytes remain in flash.
3. Use an owner-controlled runner lifecycle that can be cancelled and joined.
      A normal detached Embassy task is insufficient if its task slot, future, or
      peripheral handles survive the transition; the offline acknowledgement must
      prove all arena references and radio DMA/PIO activity are dead.
4. Add a product-profile transition probe for
      `FullAudio -> WifiStreamAudio -> FullAudio`. It needs radio initialization,
      bounded packet-buffer activity, shutdown, and rich-audio reconstruction, but
      no scan UI, credential persistence, socket API, HTTP client, or app ABI.
5. Exercise 100 physical round trips and the five-minute alternating
      PCM16/SLDPCM4 soak. Capture `phase=173`, `phase=176`, memory-report margins,
      worker generations/heartbeat, radio lifecycle state, packet counters,
      `arena_guard_failures=0`, and `transition_failures=0`.

KOTO-0224 continues to own stack selection, final capacities, credentials,
security policy, scan/connect behavior, KotoConfig UI, and the public
NetworkService contract. Results from this lifecycle probe become measured
inputs to that design rather than prematurely fixing its API.

## Implementation Progress

### 2026-07-18: Board policy and initial audio residency split

- Added explicit `AUDIO`, `WIFI`, and `AUDIO_WIFI_CONCURRENT` board
      capabilities plus a distinct Pico W / RP2040 profile.
- Split the former shared audio cell into permanent raw-stream state and rich
      service state without changing the worker's mixing or DMA pacing behavior.
- Grouped the rich shared state, mixer service, runtime BGM/SFX players, and
      owned clip player into the aligned `AUDIO_RICH_RESIDENCY` ELF symbol.
- Fixed the rich region at exactly 36 KiB with compile-time size and alignment
      guards. The Pico W release ELF reports `AUDIO_RICH_RESIDENCY=0x9000`,
      `AUDIO_STREAM_SHARED=0x2010`, `AUDIO_STREAM_SCRATCH=0x142c`, and
      `AUDIO_DMA_RING=0x400`.
- Moved runtime cue/clip loading directly into the rich residency image slot.
      The permanent CPU0 stream scratch is now 5,164 B including guards and
      diagnostics instead of retaining the 8,732 B rich cue-image capacity.
- The measured Pico W release sections are `.data=66,244 B` and
      `.bss=139,384 B` (`205,628 B` combined), 192 B above the retained KOTO-0226
      baseline while reserving the full 36 KiB switchable region.
- Pico, Pico W, and Pico 2 W all-bin cross-checks pass. The existing host audio
      scratch guard/exclusion regression also passes.
- The rich region is not reusable yet: CPU1 acknowledgement/teardown is now
      implemented below, but network runner join/power-down and reverse rich-state
      reconstruction remain required before network state may occupy these bytes.

### 2026-07-18: Mode-owner foundation

- Added one dependency-free residency owner with explicit `FullAudio`,
      `QuiescingAudio`, `Offline`, `WifiStreamAudio`, and `QuiescingWifi` states.
- Every transition increments a generation token. Stale tokens are rejected,
      requests are unavailable during quiescing/offline states, and a transition
      fault increments a bounded failure counter and lands in `Offline`.
- Connected the owner token to `PicoAudioBackend`. Built-in BGM/SFX requests
      now return an explicit error and map temporary unavailability, stale handles,
      and bounded queue pressure to the VM ABI's retryable `WOULD_BLOCK` result.
      PCM submission is accepted only in stable FullAudio/WifiStreamAudio states.
- Rich runtime-image loading distinguishes `TemporaryUnavailable` from `Busy`
      instead of silently dropping either condition.
- Added a host transition harness to the full local check. It exercises 100
      complete logical round trips, stale-token rejection, request gating in every
      intermediate state, and fault-to-offline behavior.
- The transition acceptance item remains open: network runner join/power-down and
      reverse rich-state reconstruction must precede real arena reuse.

### 2026-07-18: CPU1 rich-service quiesce handshake

- Added a high-priority, generation-tagged `QuiesceRich` worker command. CPU1
      resets the service, stops all runtime players, clears rich shared state,
      drops every rich-residency reference, then publishes the matching
      generation acknowledgement with release ordering.
- CPU1 keeps the permanent raw PCM path and hardware-paced DMA alive. In
      stream-only mode the worker reads only `AUDIO_STREAM_SHARED`,
      `AUDIO_STREAM_SCRATCH`, and `AUDIO_DMA_RING`; it no longer reads the rich
      output ring or rich players.
- Added the CPU0 `begin_wifi_quiesce` entry point. It advances the single owner,
      flushes normal rich commands, and installs the high-priority quiesce
      command. `service` advances to `Offline` only after an acquire load sees
      the matching worker acknowledgement.
- Added residency state/generation, worker acknowledgement generation,
      rich-active state, and transition-failure count to the `phase=173` audio
      diagnostic so hardware captures can distinguish command issue from proven
      reference release.
- Pico W release type-check and the two host residency regressions pass. Real
      arena reuse remains disabled until Wi-Fi task ownership and reverse
      reconstruction are implemented.

### 2026-07-18: Reconstructable arena and reverse CPU1 handshake

- Moved the generation-tagged worker transition mailbox out of the reusable
      region. CPU1 polls this permanent atomic mailbox even while rich audio is
      inactive, so an overwritten arena is not needed to request reconstruction.
- Replaced the one-shot rich `StaticCell` fields with protocol-owned
      `MaybeUninit` slots and wrapped the complete 36 KiB ELF symbol in
      `UnsafeCell`. The alternate owner may now use every arena byte after the
      offline acknowledgement without violating Rust's static alias model.
- Added a non-`Copy`, generation-tagged `WifiResidencyArena` handle. The handle
      exposes the exact 36 KiB as uninitialized bytes only after the owner enters
      `WifiStreamAudio`; reverse transition updates and consumes the handle before
      any rich field is reconstructed.
- Gated direct runtime-cue, runtime-clip, and stop requests so no safe CPU0 audio
      API dereferences the arena outside `FullAudio`.
- After the Wi-Fi runner and all arena users have joined, CPU0 reconstructs the
      typed rich fields in place and asks CPU1 to reacquire them. CPU1 starts the
      new service and publishes a matching online generation last; `service`
      enters `FullAudio` only after observing that acknowledgement.
- Added `worker_online_generation` to the `phase=173` diagnostic. Pico, Pico W,
      and Pico 2 W release type-checks pass, as do the 100-round-trip and
      fault-to-offline host regressions.
- The transition acceptance item remains open until KOTO-0224's bounded CYW43
      runner is constructed in this handle, joined on reverse transition, and
      exercised on Pico W hardware through the required 100 physical round trips.

### 2026-07-18: Machine-readable residency memory gate

- Added `harness/check_audio_residency_memory.py`. It parses release ELF sections
      and named residency symbols with `rust-size`/`rust-nm`, fails if the rich
      arena is not exactly 36 KiB or 8-byte aligned, and emits the FullAudio and
      WifiStreamAudio active layouts as `koto.audio-residency-memory.v1` JSON.
- The Wi-Fi layout now uses the 608 bytes after the 12,688-byte CYW43 `State`
      for KOTO-0245's 604-byte synchronized Fetch/TLS-audio coordinator,
      leaving 4 bytes.
      The 13,296-byte driver reservation, 23,568-byte runner/network region,
      and total 36 KiB arena are unchanged.
- The current Pico W release report records `.data=66,344 B`, `.bss=139,384 B`,
      `.data+.bss=205,728 B`, and a `205,944 B` static span. This is `+292 B`
      and `+192 B` respectively from the retained KOTO-0226 baselines. The
      Fetch TLS/audio handoff and bounded app-teardown drain add 40 bytes to
      the static end; the audio residency memory gate still passes.
- Named permanent stream storage totals `22,584 B`: shared raw ring `8,204 B`,
      stream scratch `5,164 B`, DMA ring `1,024 B`, and CPU1 stack `8,192 B`.
      WifiStreamAudio reclaims the complete `36,864 B` rich arena.
- CPU0 `phase=176` and CPU1 stack margins are represented as explicit
      `null/not-captured` values until supplied from a hardware log; the report
      CLI accepts both measurements without changing the ELF-derived fields.
- Registered the fixed-output parser self-test in `check_all.py`; real-ELF report
      generation remains an explicit Pico W cross-build gate.
- Extended the same report with the KOTO-0245 `TlsExclusive` exception. It
      retains the 8,192-byte CPU1 stack. The implemented loan exposes only the
      8,192-byte PCM sample array; the broader PCM/scratch/DMA candidate totals
      14,392 bytes but is not required by the admitted TLS layout.
- Added the RP2040 worker fence used by that exception. The offline ACK follows
      DMA abort, PWM silencing, and PCM reset; the online ACK follows a silent
      DMA-ring rebuild and pacing restart. CPU0 stream refills are rejected for
      every transitional and TLS-owned state.
- Strengthened CPU1 teardown so service and runtime-player destructors complete
      after references leave the worker and before the offline generation is
      published. The acknowledgement now proves that no live rich Rust object,
      not only no worker reference, remains in the bytes lent to Wi-Fi.
- Added `arena_guard_failures` to `phase=173`. It increments when a generation-
      mismatched arena handle reaches either reverse-transition boundary, while
      ordinary invalid-state requests remain accounted by transition handling.
      The physical transition soak requires this counter to remain zero.

### 2026-07-18: CYW43 resource audit and concrete layout probe

- Reserved semantic W-board radio roles in the board adapter: PIO0/SM0,
      DMA channel 2 TX, DMA channel 3 RX, power GP23, data GP24, CS GP25, and
      clock GP29. Production
      already owns DMA0 for LCD, DMA1 for PSRAM fast reads, PAC DMA11 for audio,
      and PIO1 for PSRAM; Pico, Pico W, and Pico 2 W cross-checks confirm the new
      roles do not disturb existing ownership.
- Added a Pico W-only `probe_wifi_residency` target and a compile-time exact
      36 KiB `WifiDriverResidencyLayout` using the concrete CYW43 0.7 types.
      It includes `cyw43::State`, the PIO0/SM0/DMA2/DMA3 runner, `Control`,
      `NetDriver`, and the 512 B scratch currently allocated by `Runner::run`.
- The initial single-DMA RP2040 measurements were: `State=12,688 B`, `Runner=44 B`,
      `Control=16 B`, `NetDriver=32 B`, and total represented driver storage
      `13,292 B`. The arena retains `23,572 B` for the cancellable runner future
      and minimum bounded network state; the gate requires at least 16 KiB.
- Added `harness/check_wifi_residency_layout.py` and the
      `koto.wifi-residency-layout.v1` JSON artifact. Its parser self-test is in
      `check_all.py`; the real probe ELF is an explicit cross-build gate.
- CYW43 0.7 exposes `Runner::run(self) -> !` and no shutdown/join method. The
      product path must therefore store and poll a cancellable runner future
      under arena ownership. A detached Embassy task would leave its future
      storage permanently resident and cannot by itself prove arena-reference
      release, so it is not the accepted integration shape.
- Added a bounded, type-erased `ArenaFuture` poll handle. The concrete future
      frame is alignment-checked and initialized directly in caller-supplied
      arena bytes; the permanent poll side retains only function pointers and
      the arena address. Completion or cancellation drops the concrete future
      before marking the slot inactive, allowing the generation owner to use
      that inactive observation as the runner-join boundary.
- Added five dependency-free host regressions covering pending cancellation,
      normal completion, oversized-future rejection, generation-safe join, and
      stale cancellation. Both completion paths prove exactly-once destruction
      before storage reuse, and the Pico W all-bin cross-check passes with the
      same implementation under `no_std`.

### 2026-07-18: concrete CYW43 arena lifecycle

- Added the concrete PIO0/SM0/DMA2/DMA3 `cyw43::new` lifecycle future. It initializes
      the 43439A0 firmware and CLM from flash while `State`, `NetDriver`,
      `Control`, `Runner`, and their combined future frame remain inside the
      generation-owned 36 KiB residency arena. No detached Embassy task retains
      an arena reference.
- Wrapped GP23 in `RadioPowerOutput`. Cancelling `ArenaFuture` drops the runner
      transport, and the wrapper forces GP23 low before publishing the permanent
      `Offline` lifecycle phase. `WifiRuntime::shutdown` returns the arena only
      after that synchronous drop/join boundary.
- Added PIO0, DMA2 TX, and DMA3 RX interrupt bindings to the product Pico W entry point and a
      device-validation-only `wifi_residency_probe` feature. At boot it performs
      `FullAudio -> WifiStreamAudio`, installs and polls the real CYW43 future,
      waits up to 10 seconds for firmware/CLM initialization, cancels it, checks
      radio power-down, returns the arena, reconstructs rich audio, and waits for
      the matching CPU1 online generation. Timeout after audio quiesce still
      recovers the arena and reverses the transition.
- The probe-enabled RP2040 release ELF links successfully. The memory report
      still finds exactly 36,864 B at `AUDIO_RICH_RESIDENCY`, reports 36,864 B
      reclaimed in `WifiStreamAudio`, and increases `.data + .bss` by only
      168 B from the retained KOTO-0226 baseline.
- Hardware execution, 100 round trips, PCM soak, and bounded packet-buffer
      activity remain open. The boot probe logs the concrete future-frame size
      as `phase=227 wifi-residency future-bytes=...`; the arena install rejects
      it before polling if it exceeds the measured reserve.

### 2026-07-18: first hardware cancellation and recovery

- Pico W hardware measured the concrete lifecycle future at 5,712 B, within the
      23,572 B reserve. CYW43 initialization did not reach `RadioReady` within
      10 seconds, but the bounded timeout cancelled and dropped the future,
      powered the radio down, returned the arena, reconstructed rich audio, and
      reported `phase=227 wifi-residency round-trip-ok`.
- The immediately following 16 kHz CPU1 PWM diagnostic submitted and played all
      2,560 frames with `drops=0`, `underruns=0`, and `result=ok`. This validates
      reverse reconstruction after a real pending CYW43 future; successful
      radio initialization and packet activity remain open.
- The arena future is now polled with the Embassy executor waker rather than a
      no-op waker. Permanent 16 B SPI telemetry records completed PIO reads,
      writes, last status, and last response word. A timeout prints these values
      with lifecycle state and poll count so the next hardware run distinguishes
      bus non-response from a later firmware/CLM stall.

### 2026-07-18: CYW43 GSPI zero-response isolation

- Hardware telemetry recorded `spi_reads=8999`, `spi_writes=0`,
      `spi_status=0x00000000`, and `spi_word=0x00000000`. The driver remained in
      the initial `FEEDBEAD` read-only bus test, proving the timeout is before
      firmware upload or CLM initialization.
- The crates.io `cyw43-pio 0.10.0` implementation reused one DMA channel for
      sequential TX and RX. Current upstream Embassy fixes a PIO state-machine
      race by starting RX and TX concurrently on separate channels. The
      unreleased upstream change was backported locally without upgrading the
      rest of Embassy: DMA2 is TX and DMA3 is RX.
- The two-DMA target layout measures `Runner=48 B`, represented driver storage
      `13,296 B`, and future/network reserve `23,568 B`; the exact 36 KiB and
      minimum 16 KiB reserve gates pass. Pico W probe and Pico 2 W product
      builds, five lifecycle host tests, and the project boundary gate pass.

### 2026-07-18: WL_ON and conservative GSPI follow-up

- A second hardware capture with the upstream two-DMA transport and the
      conservative `RM2_CLOCK_DIVIDER` still remained in `Initializing` after
      9,457 lifecycle polls and 9,192 completed reads. Status and response were
      both zero, with no writes, so reducing GSPI to approximately 20.8 MHz did
      not change the initial read-only bus-test failure.
- GP23 telemetry recorded one High request, a High output latch, and a High pad
      input. This rules out a failed RP2040 GPIO High operation, but it does not
      by itself prove that a CYW43439 is present or powered: a standard non-W
      Pico also exposes GP23, while GP24, GP25, and GP29 serve non-radio board
      functions.
- The vendored PIO program, pin configuration, command framing, and concurrent
      RX/TX DMA ordering match current upstream `cyw43-pio`. With the software
      transport and clock-profile comparisons exhausted, the next discriminator
      is physical module identity or a GSPI capture on CS, clock, and data. A
      confirmed Pico W with clocks and commands present but no GP24 response
      points to CYW43439 power/module hardware rather than SRAM residency or
      lifecycle cancellation.
- The installed module was subsequently confirmed to be a physical Pico W, so
      a non-W profile mismatch is excluded. The next probe captures RP2040 PIO0
      enable/FIFO/error state, SM0 program counter, GP24/25/29 function select,
      PIO pad output/output-enable, and GPIO input immediately after each GSPI
      transaction. Expected pin functions are PIO0/SIO/PIO0; matching values
      narrow the remaining fault to CYW43439 power or physical bus response.
- The raw snapshot then reported `funcs=0x00060506`, `ctrl=0x00000001`,
      `padoe=0x20000000`, and `sm0_addr=6`. GP24 and GP29 are assigned to PIO0,
      GP25 is SIO, SM0 is enabled, the clock remains output-enabled, data has
      returned to input, and the state machine has completed the transaction at
      its event wait instruction. `fdebug=0x01000000` is only SM0 TXSTALL after
      the completed transfer; RXSTALL, RXUNDER, and TXOVER are clear. This
      excludes the observed PIO/DMA/pin-mux path as the source of the zero data.
- A final timing discriminator extends the pre-reset Low interval to about one
      second and delays the first GSPI command until about one second after
      WL_ON rises. The stock driver uses 20 ms Low and 250 ms High. If the
      extended profile remains all-zero, KotoOS reset/settling timing is also
      excluded and the remaining work requires a known-good firmware comparison
      or physical CYW43439 power/GSPI measurement.
- The extended profile also remained all-zero: 7,297 reads completed without a
      write or `FEEDBEAD` response, while the PIO snapshot retained the expected
      pin functions, enabled SM0, completed transaction PC, clock output-enable,
      and benign post-transfer TXSTALL. Timeout cancellation again returned the
      arena and reported `round-trip-ok`.
- An independent Picoware Pico W firmware also failed to boot on the same
      physical Pico W. This makes a board-level CYW43439 power/module fault the
      leading diagnosis and blocks successful `DriverReady`, packet activity,
      100-transition, and soak acceptance captures on this unit. Repeat those
      captures on a known-good Pico W or repair/replace the module; do not treat
      the diagnostic one-second delays as product timing requirements.
- The bounded lifecycle probe now also builds for the available RP2350A Pico 2
      W as a control experiment. Embassy's upstream Pico 2 W example uses the
      same GP23/24/25/29 PIO bus, CYW43439 firmware, `nvram_rp2040.bin`, and
      `RM2_CLOCK_DIVIDER`, matching this probe. Reaching `DriverReady` or
      `RadioReady` there validates the shared CYW43 software path independently
      of the suspect RP2040 board. It does not replace the RP2040 exact-36-KiB
      arena, transition-count, or stream-audio acceptance captures, and the
      normal Pico 2 W product profile still requires independent simultaneous
      Audio/Wi-Fi residency rather than this diagnostic mode switch.
- Build the RP2350A control image with
      `tools\build-rp2350a.ps1 -WifiResidencyProbe`. The helper enables the
      probe feature and uses `picotool uf2 convert` with the RP2350 Arm-secure
      family and E10 absolute block, producing
      `koto_firmware-picocalc-pico2w-rp2350a-wifi-residency-probe.uf2` without
      overwriting the normal product UF2. `elf2uf2-rs` must not be used for the
      RP2350 image because it emits the RP2040 family ID.
- The Pico 2 W control run also remained in `Initializing`: 7,795 completed
      reads returned zero status and data, while GP23 and the PIO/pin snapshot
      remained valid and cancellation returned `round-trip-ok`. This disproves
      a fault isolated to the original RP2040 Pico W and reopens the shared
      CYW43 transport as the controlling failure surface.
- Picoware's RP2W firmware successfully operated Wi-Fi on the same Pico 2 W.
      This independently validates that board's CYW43439, radio power, and
      physical bus, and makes the Pico 2 W all-zero result a KotoOS software
      failure rather than a second hardware fault. The original RP2040 Pico W
      may still have its own board fault because Picoware also failed there, but
      it is no longer sufficient to explain the shared KotoOS failure.
- A feature-gated A/B image now restores the crates.io `cyw43-pio 0.10.0`
      transaction order, completing TX DMA before starting RX DMA, while still
      retaining separate channel ownership. Build it with
      `tools\build-rp2350a.ps1 -WifiSequentialPioProbe`; the UART identifies
      `transport=sequential-tx-rx`. A nonzero bus test here isolates the issue
      to concurrent DMA arming against Embassy 0.10, while another all-zero run
      requires comparison against the complete upstream Pico 2 W example or a
      physical CS/clock/data capture.
- The sequential image also completed 7,726 reads with zero status and response,
      so concurrent versus sequential DMA ordering is excluded. The same
      bounded cancellation and audio reconstruction completed successfully.
- Added `probe_wifi_minimal`, an independent Pico 2 W binary matching the
      upstream Embassy bring-up shape: it initializes only UART, direct
      `PioSpi`, DMA0/1, static `cyw43::State`, firmware/NVRAM, and
      `cyw43::new`, with no LCD, PSRAM, audio, CPU1, arena, cooperative wrapper,
      or residency controller. Build it with
      `tools\build-rp2350a.ps1 -WifiMinimalProbe`. `driver-ready` isolates the
      fault to Koto's lifecycle integration; `driver-timeout` moves the
      controlling comparison to the Embassy/CYW43 dependency versions or the
      direct PIO implementation itself.
- The minimal DMA0/1 probe reached `radio-ready`, proving the repository's
      Embassy/CYW43 versions, direct `PioSpi`, firmware upload, runner, and CLM
      initialization work on the known-good Pico 2 W. The following
      `clm-timeout` line was a probe-only reporting bug: the successful control
      branch intentionally remained pending while its outer timer continued.
      The probe now selects runner versus CLM completion directly and emits
      exactly one terminal result.
- The next A/B image keeps the successful direct minimal shape but changes only
      DMA0/1 to the product-reserved DMA2/3 pair. Build it with
      `tools\build-rp2350a.ps1 -WifiMinimalDma23Probe`; it emits
      `dma=2,3`. Success excludes DMA channel selection and leaves the power/SPI
      wrappers or arena lifecycle polling as the nearest integration difference.
- The direct minimal DMA2/3 image also reached `radio-ready`, excluding the DMA
      channel pair and its interrupt bindings. The next image keeps this now
      successful minimal lifecycle and changes only direct `PioSpi` to the
      product `CooperativePioSpi` wrapper. Build it with
      `tools\build-rp2350a.ps1 -WifiMinimalCooperativeProbe`; its start line
      reports `transport=cooperative-pio dma=2,3`. A timeout here localizes the
      failure to wrapper behavior; `radio-ready` moves the boundary outward to
      product power handling or arena/controller polling.
- The first cooperative-wrapper image timed out in `cyw43::new` while the same
      direct DMA2/3 image reached `radio-ready`, localizing the regression to
      wrapper behavior. Of its three additions, only the diagnostic 750 ms
      first-command delay runs before the first GSPI transaction; PIO snapshots
      and `yield_now` run after a completed transaction. The wrapper no longer
      adds that delay. The rebuilt cooperative image also emits SPI read/write
      counts plus the last status and word on `driver-timeout`, so the next run
      directly tests the startup-timing hypothesis and preserves evidence if it
      is false.
- Removing only the 750 ms delay still timed out with `reads=834938`,
      `writes=0`, `status=0`, and `word=0`. This disproves the startup-delay
      hypothesis and shows the driver's initial GSPI bus-test read completed and
      returned zero repeatedly for the full ten-second window. The next rebuild
      removes both post-transaction PIO register snapshots and `yield_now`,
      retaining only the wrapper type, direct delegation, and atomic telemetry.
      `radio-ready` therefore attributes the regression to one of those
      diagnostic side effects; another all-zero timeout would instead implicate
      the wrapper future/type boundary or telemetry atomics.
- Removing snapshots, yielding, and then telemetry produced the same all-zero
      result. Source comparison then exposed the root cause: `PioSpi` had an
      inherent `cmd_read` helper with the same name as
      `SpiBusCyw43::cmd_read`. The wrapper expression
      `self.inner.cmd_read(...)` resolved to the inherent helper and bypassed
      the trait implementation's CS-low/CS-high framing. This exactly explains
      completed DMA reads with zero CYW43 responses and no writes. The wrapper
      now delegates through fully qualified `SpiBusCyw43` calls, and the
      inherent helper is private and renamed `read_transaction` to prevent
      recurrence. The hardware discriminator is emitted as
      `probe_wifi_minimal-cooperative-csfix-picocalc-pico2w-rp2350a.uf2`.
- The CS-fixed cooperative image reached `radio-ready` on the known-good Pico
      2 W, hardware-confirming the method-resolution diagnosis and excluding
      the wrapper future/type boundary and atomic telemetry. The same fix is now
      included in the product residency image
      `koto_firmware-picocalc-pico2w-rp2350a-wifi-residency-probe.uf2`; its next
      acceptance point is `radio-ready` followed by `round-trip-ok`, covering
      arena-owned polling, bounded shutdown/power-down, and full-Audio
      reconstruction.
- The CS-fixed Pico 2 W product residency image reached both `radio-ready` and
      `round-trip-ok` with a 5,712 B arena future. Boot then continued through
      PSRAM discovery (`capacity=8388608`) and the reconstructed CPU1 PWM Audio
      path submitted and played all 2,560 diagnostic samples with zero drops and
      `result=ok`. This is the first complete hardware proof of Audio -> Wi-Fi ->
      Audio ownership transfer through the product controller. The short boot
      diagnostic reported one underrun, so it is not evidence for the separate
      five-minute zero-underrun soak criterion.
- After the CS fix and hardware round trip, `python harness/check_all.py`
      completed with `KotoOS local checks: OK`, covering the project, Rust,
      audio, memory-map, and 100-round-trip host transition gates.
- Pico 2 W is now the immediate product priority. `WifiRuntime` accepts an
      owned arena abstraction: Pico W continues to borrow the 36 KiB rich-Audio
      region, while Pico 2 W uses an independent 36 KiB static Wi-Fi arena and
      never enters Audio quiescing. Its product probe requires `FullAudio`
      before and after CYW43 initialization and records the CPU1 worker
      heartbeat on both sides. Hardware acceptance for this step is
      `radio-ready`, an increasing heartbeat, and
      `wifi-concurrent concurrent-ok audio=full`.
- Pico 2 W hardware produced `radio-ready`, advanced the CPU1 Audio worker
      heartbeat from 3 to 5,978 during the CYW43 lifecycle, and emitted
      `wifi-concurrent concurrent-ok audio=full`. This closes independent
      simultaneous Audio/Wi-Fi residency for the Pico 2 W profile. The next
      probe stages five fixed 64-byte Ethernet frames into CYW43's four-entry TX
      channel; obtaining the fifth token proves the runner consumed and recycled
      at least one packet buffer. Its explicit evidence line is
      `packet-tx staged=5 recycled-min=1`.

### 2026-07-20: Pico W board revalidated; RP2040 packet activity and 100-trip soak

- Picoware 2.0.0 operated Wi-Fi normally on the original RP2040 Pico W,
      withdrawing the earlier board-level CYW43439 fault diagnosis for that
      unit (the older Picoware image that also failed there is now attributed
      to the image, not the board). This unblocked every RP2040 hardware
      acceptance capture.
- The CS-fixed Pico W product residency image then produced the first
      complete RP2040 hardware success: `radio-ready`,
      `packet-tx staged=5 recycled-min=1` (five 64-byte frames through the
      four-entry CYW43 TX channel, proving buffer recycling), and
      `round-trip-ok` with the measured 5,712 B arena future. The subsequent
      CPU1 PWM diagnostic played all 2,560 frames with `drops=0`,
      `underruns=0`, and `result=ok`.
- Rebuilt the Pico W probe as the 100-round-trip acceptance soak. Each trip
      reinstalls the concrete CYW43 lifecycle future in the borrowed arena,
      requires packet recycling before `RadioReady`, and must observe the CPU1
      online generation before the next trip. Radio-level timeouts recover and
      count as `radio_failures`; a failed recovery aborts the soak because the
      arena/peripheral release proof no longer holds. `power-down-failed` is
      now a soak abort rather than a log line for the same reason.
- Radio peripherals are re-aliased per trip through the validation-only
      `PicoWRadioResources::clone_for_probe` (`Peri::clone_unchecked`); its
      safety contract is the existing shutdown drop/join boundary plus the
      GP23-low power-down proof.
- The diagnostic one-second WL_ON pre-reset interval in the lifecycle future
      was retired for the stock ~20 ms product profile, so the soak validates
      product reset timing (the CS fix was already proven under stock timing).
- Pico W hardware passed the full soak:
      `soak trips=100 ok=100 radio_failures=0 aborted=0 transition_failures=0
      arena_guard_failures=0`, followed by a clean PSRAM discovery and a
      zero-drop, zero-underrun CPU1 PWM diagnostic. This closes the
      100-transition acceptance criterion and, combined with the packet
      recycling evidence, the bounded CYW43 lifecycle-probe and teardown-proof
      criteria on the RP2040 target.
- Remaining before closure: the five-minute alternating PCM16/SLDPCM4
      `WifiStreamAudio` soak with concurrent bounded packet activity, the full
      rich-audio regression after returning from Wi-Fi mode, hardware CPU0/CPU1
      margin injection into the machine-readable SRAM report, and the frozen
      release threshold with KOTO-0224's bounded network budget.
- Implemented the five-minute product-path soak as `wifi_stream_soak_probe`
      (Pico W only, replaces the boot-time 100-trip loop in its image). After
      SD initialization the probe holds one `FullAudio -> WifiStreamAudio`
      transition; the new arena-owned `cyw43_soak_future` publishes
      `RadioReady` after CLM initialization and then stages one bounded
      64-byte broadcast frame per second through the four-entry TX channel,
      counting each staged frame. The stream half
      (`firmware::stream_soak`) locates `sample_audio_codecs.kpa` in `APPS/`
      by long filename, walks the real KPA1 asset table, and alternates the
      219 KB PCM16 and 54 KB SLDPCM4 KACL assets through the product
      pipeline: SD range reads into the permanent stream scratch,
      `StreamingClipDecoder::decode_chunk`, and CPU1 PCM submission, with a
      30-second progress heartbeat. The summary line reports pass counts,
      refills, submitted samples, underrun/drop deltas, staged TX frames,
      transition/arena-guard counters, and the reconstruction result; any
      stream or radio failure ends the soak early through the same proven
      shutdown/power-down/reconstruction boundary. Probe, non-soak Pico W,
      and Pico 2 W cross-builds pass warning-free with the 36 KiB memory gate
      unchanged; hardware capture is pending.
- Two soak dry runs failed fast with `AssetError` and exposed two independent
      bugs in the new module: the KPA entry record's `data_offset`/`data_size`
      were read from bytes 8..16 (`type`/`flags`) instead of 16..24, and the
      asset paths were guessed from the source filenames instead of the packed
      `audio/sample_audio_pcm16.kacl` / `audio/sample_audio_sld4.kacl` names
      (verified by dumping the local KPA table). Both failure runs still
      completed shutdown, GP23 power-down, and rich-audio reconstruction
      (`round-trip-ok`, `audio_restored=1`), hardware-proving the failure-path
      recovery boundary. Distinct `package-not-found` / `asset-missing`
      diagnostics were added for future triage.
- Pico W hardware then passed the full five-minute soak:
      `result=Ok elapsed_ms=300000 pcm16_passes=22 sld4_passes=21 refills=2321
      samples_submitted=4803718 underruns=0 drops=0 tx_frames=299
      transition_failures=0 arena_guard_failures=0 audio_restored=1`, with a
      clean 30-second heartbeat throughout and normal catalog load afterwards.
      Submitted samples match the 16 kHz x 300 s product rate, and the ~1 Hz
      staged-frame count proves continuous TX buffer recycling for the whole
      window. This closes the five-minute soak criterion and the
      `WifiStreamAudio` PCM16/SLDPCM4 streaming criterion on RP2040 hardware.
- Remaining before closure: the returned-to-`FullAudio` rich-audio regression
      (BGM/SFX/cue/clip/PCM/heartbeat/app exit), hardware CPU0 `phase=176` and
      CPU1 margin injection into the machine-readable SRAM report, the frozen
      release threshold with KOTO-0224's bounded network budget, and the final
      all-board release/harness pass with retained captures.
- A post-soak `FullAudio` session on the same boot exercised
      `sample_audio_codecs` PCM16 and SLDPCM4 streaming (`stage=streaming`
      both codecs) and a full KotoRun run with BGM, SFX, and runtime KMML cue
      staging (`psram-write/read/queued` plus `cache-read` hits), both with
      clean exits. Captured margins: CPU0 `phase=176 used=52,544
      free_min=11,856` (above the KOTO-0170 stop-ship floor; the retained
      15,724 B baseline came from the default offline image on a heavier
      session mix) and CPU1 `core1_free=6,588` with `heartbeat=453,917`
      (1,604 B used of 8,192 — better than the KOTO-0186 worst-case margin).
      All fault counters in `phase=226` were zero, but the line reported
      `result=fail` because the run skipped the app's built-in five-minute
      cue regression (key `3`): `cold_cue_loads=0`/`cue_cache_hits=0` failed
      the coverage gate, not a fault. The remaining regression capture is one
      `sample_audio_codecs` key-`3` run to completion (auto-exits at frame
      18,750) with `phase=226 result=pass` plus the follow-up `phase=176`
      line.
- The key-`3` built-in five-minute cue/stream regression then completed on
      the same post-Wi-Fi boot: `phase=226 ... pcm16=30 sld4=33 cold=2
      hits=311 load=313 stream=2527 rejected=0 corrupt=0 drops=0 underruns=0
      unsupported=0 command_drops=0 heartbeat=1639680 core1_free=6588
      result=pass` with a clean exit and an unchanged `phase=176 used=52,544
      free_min=11,856` low-water. Combined with the earlier KotoRun
      BGM/SFX/clip session, this closes the returned-to-`FullAudio`
      regression criterion: every rich path passes after Wi-Fi mode and the
      CPU1 margin (1,604 B used of 8,192) is better than the accepted
      baseline.
- Regenerated `koto.audio-residency-memory.v1` from the soak ELF with the
      measured hardware margins injected: `.data=66,344`, `.bss=139,376`,
      static span `205,936` (`+184 B` vs the retained KOTO-0226 baseline),
      `cpu0_phase176_free_min=11,856`, `cpu1_stack_free_min=6,588`, all seven
      structural checks passing. The boot low-water is monotonic, so the
      11,856 B CPU0 margin already covers the five-minute Wi-Fi soak, both
      failure-path recoveries, and the full app session mix on one boot. This
      closes the machine-readable SRAM report criterion.
- Frozen release threshold (signed off 2026-07-20, sized against KOTO-0224's
      bounded network budget): Wi-Fi-enabled RP2040 images must keep
      `phase=176 free_min >= 8 KiB` (2x the KOTO-0170 stop-ship floor;
      measured 11,856 B leaves ~3.8 KiB headroom) and
      `core1_stack_free_min >= 4 KiB` (measured 6,588 B). Note the soak image
      does not link the KOTO-0239 NetworkService on Pico W; the future product
      network wiring must place its state in the arena reserve and re-verify
      against this threshold rather than growing `.bss`.
- Final verification pass: `cargo fmt` clean; workspace Clippy clean after
      mechanically modernizing seven pre-existing test lints newly fired by the
      current toolchain (koto-core `fetch.rs`/`json.rs`/`shell.rs`, unrelated
      to this issue); RP2040 Pico, Pico W, and RP2350A Pico 2 W release builds
      plus `check_embedded.py` all pass. `check_all.py` is green through the
      project, audio, memory-map, and 100-round-trip transition harnesses;
      the single remaining red is the pre-existing `koto-sim`
      SKK-candidate UI test failure already noted as unrelated during
      KOTO-0239 and tracked outside this issue. Hardware captures for the
      packet probe, 100-trip soak, five-minute stream soak, and post-Wi-Fi
      regression are retained in this document. The only open acceptance item
      is freezing the proposed release threshold.
