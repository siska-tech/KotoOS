# Archived bring-up binaries

These are early RP2040 bring-up experiments that have been superseded by the
official `koto_firmware` and the retained single-peripheral probes under
`../../src/bin/`. They are kept here for historical reference only.

They live outside `src/bin/` on purpose: Cargo only auto-discovers binaries in
`src/bin/`, so nothing here is compiled, and none has a `[[bin]]` entry in
`Cargo.toml`. They may reference older `koto_pico` APIs and are not guaranteed to
build. To revive one, move it back into `src/bin/` and add a `[[bin]]` entry.

| File | Was | Retired because |
| :--- | :--- | :--- |
| `bootstrap.rs` | KOTO-0064 minimal HAL `init` + `wfi` smoke test | The whole HAL is now exercised by `koto_firmware` boot. |
| `blink_cdc.rs` | KOTO-0065 LED blink + USB-CDC banner (standard Pico) | USB-CDC bring-up is superseded by UART0 firmware diagnostics. |
| `blink_cdc_pico_w.rs` | KOTO-0065 Pico W (CYW43) variant of the blink probe | Targets different hardware (Pico W); never physically validated. |
| `keyboard_matrix.rs` | KOTO-0067 chord-matrix selection campaign | One-time job: it picked the `arrow-zxas` default mapping. `probe_keyboard` covers ongoing keyboard checks. |
| `device_probe.rs` | Combined multi-peripheral USB-CDC dashboard | Superseded by `koto_firmware` boot plus the individual `probe_*` binaries. |
