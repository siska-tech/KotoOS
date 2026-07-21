# KotoOS Architecture

This document turns the requirements into an implementation-facing architecture. It is intentionally small and should evolve with code.

## Layer Model

| Layer | Responsibility | Portability |
| :---- | :------------- | :---------- |
| KotoShell | Launcher, status UI, app selection | Core portable logic plus HAL calls |
| KotoRuntime | Bytecode VM, app sandbox, host API dispatch | Core portable |
| KotoSDK | Public Rust API exposed to apps and engines | Core portable |
| KotoIME / KotoFont | Japanese input, glyph lookup, rasterization | Core portable with storage hooks |
| KotoFS | Virtual paths, package access, save data | Core portable with HAL FS backend |
| HAL | Video, input, audio, storage, time, power | Backend-specific |
| Platform Backend | PC host backend, RP2040/RP2350 embedded backend | Backend-specific |

## Core Rules

1. Core modules are written in Rust and should remain `no_std` compatible where practical.
2. Core modules must not depend on backend-specific crates directly.
3. Platform state enters the core through HAL traits and owned data structures.
4. RP2040 PSRAM is accessed only through explicit block-transfer APIs.
5. Full-screen RGB565 framebuffers are forbidden on RP2040.
6. Rendering paths must support dirty rectangles and scanline buffers from the start.
7. App code runs through KotoRuntime and KotoSDK; dynamic native loading is out of scope.
8. C/C++ libraries, when needed, are isolated behind Rust FFI adapters.

## Initial Directory Shape

The following layout is the intended implementation target:

```text
src/
  koto-core/
    shell/
    runtime/
    sdk/
    ime/
    font/
    fs/
  koto-hal/
    host/
    pico/
  koto-sim/
  apps/
    builtin/
tools/
  pack_kpa/
harness/
  fixtures/
docs/
```

The repository does not need all directories immediately, but new code should fit this shape unless a better local pattern emerges.

## Runtime Model

Apps are distributed as `.kpa` packages. A package contains bytecode plus sequentially arranged assets using the [KPA Package Format](../spec/KPA_FORMAT.md). KotoRuntime loads package metadata, maps app-visible paths into a sandbox, and interprets bytecode while calling KotoSDK host functions for drawing, input, audio, and storage.

On RP2040, bytecode and large assets may live in PSRAM or SD-backed streams, but execution uses SRAM working buffers. Code must never assume PSRAM pointers can be dereferenced.

The device launcher and app VM are foreground-exclusive. The Pico backend uses
one tagged SRAM slot for either `ShellState` or the resident VM CodeWindow,
swapping the inactive shell through a reserved PSRAM region at app boundaries.
The ownership and failure rules are defined in
[RP2040 Shell / Code-Window Resident Overlay](RP2040_SHELL_CODE_RESIDENT_OVERLAY.md).

The first `kotoruntime-bytecode` executable contract is the `KBC1` format and
host-call ABI in [RUNTIME_BYTECODE_ABI.md](../spec/RUNTIME_BYTECODE_ABI.md). Runtime
execution is cooperative: each active app receives a bounded instruction budget
per frame, with input sampled once before execution and drawing flushed after
the frame.

## Rendering Model

The video API is line-oriented with dirty rectangle support:

- UI and text apps mark changed rectangles.
- Game modes may render into small scanline buffers.
- 160x160 and RGB111 modes are treated as performance modes.
- KotoDOS may choose a 128KB-class 320x200 VRAM only when memory budget permits.

The HAL owns the physical LCD transfer strategy. Core code describes what changed; the backend decides how to push pixels.

## Input Model

Input is polled once per frame into a normalized state:

- Direction keys
- Confirm/cancel/menu controls
- Optional A/B/X/Y style actions
- Raw key events for text input

The default game mapping must be validated on real PicoCalc hardware because the keyboard matrix can ghost or block some simultaneous key combinations.

## Power Model

Power and battery data are optional HAL capabilities. KotoShell should display them when available, and file-writing paths should be designed so low-battery warnings can reduce data-loss risk.
