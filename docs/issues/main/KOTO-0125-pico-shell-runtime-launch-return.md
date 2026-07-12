# KOTO-0125: Pico Shell Runtime Launch And Return

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-2, FR-RT-1, FR-RT-3, FR-RT-4, FR-SDK-5, NFR-REL-1

## Goal

Launch a selected SD package through the portable bytecode runtime on PicoCalc
and return cleanly to KotoShell.

## Acceptance Criteria

- [x] Confirm opens the selected validated bytecode package rather than only
  showing visual feedback.
- [x] Runtime input, drawing, bounded execution fuel, and exit use the same
  portable contracts as KotoSim.
- [x] Launch, per-frame stepping, exit, and failure transitions use a shared
  runtime-session/orchestration implementation in `koto-core`; KotoSim and
  PicoCalc provide only platform adapters for package bytes, host services,
  input, rendering, and diagnostics.
- [x] App failure or malformed bytecode returns to a usable shell with a
  diagnostic instead of resetting the device.
- [x] Returning from an app restores or repaints KotoShell correctly.
- [x] At least one minimal sample app launches, draws, reads input, and exits
  on physical hardware.
- [x] File Note writes and reads its app-scoped file on physical hardware.
- [x] IME Playground shows composition/conversion state from the shared
  `KotoMemoIme` adapter on physical hardware.

## Notes

Depends on KOTO-0120 through KOTO-0123. Memo/IME, audio, and larger game
validation can follow after this minimal hardware runtime slice.

Increment 1 targets the small SDK samples with a bounded 8 KiB bytecode buffer,
16 KiB app heap, the shared verifier/VM and 60,000 instruction frame fuel.
Device host support initially covers rectangle/text drawing, normalized input,
yield, and exit. Unsupported host calls and runtime failures return to Shell
with UART diagnostics.

`BytecodeSession` now owns verification, VM construction, per-frame stepping,
fuel results, exit state, traps, frame count, and open-file cleanup in
`koto-core`. KotoSim and PicoCalc both delegate that lifecycle to the common
session and retain only their platform-specific byte loading, host services,
rendering, input mapping, and diagnostics.

Initial hardware coverage: Actor Array, Counter, Dirty Rects, Hello Text, and
Input Echo entered the shared VM successfully. The LCD adapter now clears the
surface on launch, suppresses identical frames, and diffs repeated full-screen
background command lists so moving actors retransmit only changed regions.
Sample App also supports its short 8.3 `MAIN.KBC` entry. The Pico host now
implements bounded app file handles over deterministic root-level 8.3 `.DAT`
names derived from the sandboxed app ID and logical path. Each operation opens
and closes the FAT handle promptly, so the long-lived `VolumeManager` remains
available to Shell.

The Pico host also embeds the shared fixed-capacity `MemoEditor` and
`KotoMemoIme`. Its `ime_display` output matches KotoSim's stable
`comp:`/`read:`/`cand:`/`miss:` protocol without heap allocation. Physical
input maps F1 to IME toggle, Tab to convert, left Shift to sticky shift, right
Shift to commit, and Control to cancel. IME Playground now consumes the toggle
and backspace intents and its committed bytecode has been regenerated.

Physical validation confirmed the retained LCD path dramatically improves Actor
Array. Counter, Dirty Rects, Hello Text, Input Echo, and the degraded IME
Playground also run. Dirty Rects matches KotoSim in showing a rectangle sliding
left-to-right over a black background, but the visible flicker occurs only on
the current device LCD adapter and remains a KOTO-0120 rendering optimization
defect rather than a runtime lifecycle blocker. Sample App
exposed and fixed a shared `BytecodeVm::new` capacity check: a platform VM may
have more stack/call capacity than the program requests; only an undersized VM
is invalid.

Remaining hardware validation is intentionally narrow: copy the regenerated SD
image, confirm File Note displays its saved text after a second launch, and
confirm IME Playground changes from empty to `comp:`/`read:`/`miss:` using the
key mappings above. Once those two checks pass, this issue can move to done.

Physical validation passed: File Note shows `saved from SDK sample` on a second
launch, and IME Playground reaches `read:`/`miss:` for `F1 -> Shift -> k -> a ->
Tab`. The initial "no reaction" report was a stale `sample_ime_playground.kbc` on
the physical SD card; the firmware, runtime, key mappings, and committed bytecode
were already correct (a scripted KotoSim run of the committed bytecode reaches
`MissingCandidate`). Refresh the SD package copy before hardware checks.

Out of scope for this increment: every game package fails launch with
`phase=253 launch-bytecode-oversize`. Their bytecode is 20-96 KiB
(kotorogue 96 KiB, koto_blocks 91 KiB, kotoshogi 73 KiB ...) versus this slice's
deliberate 8 KiB bytecode buffer (`MAX_DEVICE_BYTECODE_BYTES`) and 16 KiB heap
ceiling. Running them needs a deliberate RP2040 SRAM budget re-sizing (larger
static bytecode/heap buffers), tracked separately rather than bumped here.
