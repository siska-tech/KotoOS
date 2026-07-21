# RP2040 Shell / Code-Window Resident Overlay

## Decision

KotoShell and an application VM code window are mutually exclusive on the
device: the shell does not run while an app owns the foreground, and app
bytecode does not execute while the launcher is active. The Pico firmware
therefore stores them in one tagged SRAM slot rather than reserving both at the
same time.

The shell is copied to a reserved region at the top of PSRAM immediately before
the slot becomes the VM code-window cache. When the app returns, the exact shell
object representation is copied back into the slot before the shell is made
active again.

This decision was introduced by KOTO-0220 after the App-authored KotoUI Gallery
exposed an RP2040 resident-SRAM regression. It preserves the RP2040 two-tile
CodeWindow and the 16-row raster/RGB666 pipeline instead of trading VM or display
performance for stack headroom.

## Memory Shape

The owning static is:

```text
SHELL_CODE_RESIDENT = tagged union(max(sizeof(ShellState), CODE_WINDOW_TOTAL_BYTES))
```

For the current RP2040 profile:

```text
ShellState                  about 28,308 B
CodeWindow                  2 * 16 KiB = 32,768 B
SHELL_CODE_RESIDENT symbol             32,776 B
```

Keeping both values separately would cost their sum. The overlay costs only the
larger value plus its tag/alignment, recovering about 28 KiB of SRAM. RP2350A
uses the same ownership model with its board-selected three-tile CodeWindow.

The shell snapshot uses a 256-byte-aligned reservation at the top of the 8 MiB
PicoCalc PSRAM:

```text
SHELL_SWAP_BYTES      = align_up(sizeof(ShellState), 256)
SHELL_SWAP_PSRAM_ADDR = PSRAM_CAPACITY - SHELL_SWAP_BYTES
APP_PSRAM_LIMIT       = SHELL_SWAP_PSRAM_ADDR
```

App bytecode and runtime audio assets must remain below `APP_PSRAM_LIMIT`.
Their allocators must not treat the shell reservation as available capacity.

## State Machine

```text
                    successful PSRAM snapshot
    +-------------+ ----------------------------> +-------------+
    | ShellActive |                               | CodeActive  |
    | SRAM=shell  | <---------------------------- | SRAM=tiles  |
    +-------------+    successful PSRAM restore  +-------------+
```

### ShellActive

- The union's active field is `ShellState`.
- Shell rendering, input, configuration, package selection, and status updates
  may borrow the shell.
- The code-window field must not be borrowed.
- A launch without usable PSRAM is rejected; the shell remains usable.

### Launch transition

1. Verify that storage and PSRAM are available.
2. Copy exactly `size_of::<ShellState>()` bytes to
   `SHELL_SWAP_PSRAM_ADDR`.
3. If that write fails, leave the tag as `ShellActive`, report
   `phase=264 shell-swap-write-error`, and return to the launcher.
4. Only after a successful snapshot, switch the tag to `CodeActive` and lend
   the slot as `[u8; CODE_WINDOW_TOTAL_BYTES]` to the app runtime.

No shell reference may survive step 4. The foreground app exclusively owns the
slot until `run_device_app()` returns.

### App return transition

1. While the tag is still `CodeActive`, read the saved shell bytes directly
   into the start of the byte-addressable code slot.
2. Change the tag to `ShellActive` only after the whole read succeeds.
3. Reborrow the restored `ShellState`, reset transient input state, and repaint
   the shell.

If the restore fails, the firmware reports
`phase=265 shell-swap-read-error fatal=1` and stops. Continuing would interpret
partially restored code-window bytes as a `ShellState`; a fatal stop is safer
than exposing corrupted state or writing preferences through it.

## Representation Safety

`ShellCodeResident` is a private firmware mechanism, not a serialization format
or persistent ABI. The snapshot is restored by the same firmware image during
the same boot.

The implementation enforces these invariants:

- `size_of::<ShellState>() <= CODE_WINDOW_TOTAL_BYTES` at compile time;
- `ShellState` must not require drop;
- the active tag is checked before either union field is borrowed;
- `[u8; N]` accepts every bit pattern while the code field is active;
- the tag is not changed back to `ShellActive` after a failed fill;
- only a read-only shell byte view is exposed for the PSRAM copy.

Any future `ShellState` field that owns heap storage, requires destruction, or
depends on addresses that cannot survive an exact same-boot byte copy invalidates
this design and must fail a compile-time guard or trigger an explicit redesign.
Do not turn the snapshot into a versioned disk file.

## Product Consequences

- PicoCalc PSRAM is now required to launch apps, even when an app's code would
  fit in one SRAM tile. A device without usable PSRAM can still boot and use the
  shell, but launch is rejected with
  `phase=255 launch-memory-budget-error reason=shell-swap-requires-psram`.
- Shell and app execution remain strictly foreground-exclusive. Background
  shell mutation during app execution is not supported.
- The swap adds one PSRAM write per launch and one PSRAM read per return, outside
  the per-frame VM/render hot path.
- The shell snapshot region is unavailable to app code, cue caches, clip data,
  or future general PSRAM allocators.

## Validation And Regression Gates

- Unit tests in `firmware/resident.rs` preserve a shell field across a
  code-window overwrite and prove that a failed restore does not activate the
  shell.
- RP2040/RP2350 release builds must keep the compile-time layout assertions.
- ELF SRAM reports must retain one `SHELL_CODE_RESIDENT` symbol rather than
  separate full-size shell and code-window statics.
- Device launch/return tests must preserve launcher selection, filtering,
  configuration/status state, and app relaunch behavior.
- The RP2040 `phase=176` canary remains the final SRAM gate. The first measured
  post-overlay worst case is `used=48,932`, `free_min=10,532` after launching
  KotoUI Gallery.
- Tests of absent/failing PSRAM must observe launch rejection or the documented
  fatal restore path, never a partially active shell.

## Implementation References

- `src/koto-pico/src/firmware/resident.rs` — tagged union and transition API
- `src/koto-pico/src/bin/koto_firmware.rs` — launch/return orchestration
- `src/koto-pico/src/firmware/config.rs` — CodeWindow sizing and PSRAM reservation
- KOTO-0220 — device failure, memory measurements, and the originating fix
