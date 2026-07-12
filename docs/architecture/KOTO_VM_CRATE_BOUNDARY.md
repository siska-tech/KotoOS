# KotoVM crate boundary

## koto-vm owns

- opcode definitions
- bytecode decoder
- interpreter
- value representation
- VM stack / locals / call frames
- RuntimeLimits
- VM errors and traps
- execution stats
- CodeSource trait
- VmHost trait

## KotoOS owns

- PSRAM backend
- PsramCodeWindow
- CodeWindow refill policy
- app launcher and storage
- graphics hostcall implementation
- audio hostcall implementation
- input hostcall implementation
- PicoCalc firmware logging and diagnostics

## Compatibility rules

- Do not change opcode values without ABI versioning.
- Do not change hostcall IDs without ABI versioning.
- Do not change `.kbc` format in koto-vm extraction/refactor work.
- Do not make koto-vm depend on koto-core.
- Do not make koto-vm depend on platform/HAL crates.
- Keep koto-vm no_std-compatible.