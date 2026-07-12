# KotoRuntime VM Selection

This spike chooses the first VM path to prototype. It is not a permanent rejection
of the other candidates; it is the smallest decision that lets KOTO-0019 define a
host API boundary and lets KotoRuntime execute a real package.

## Decision

Prototype a small custom stack VM first.

The first runtime package name remains `kotoruntime-bytecode`. The prototype
should start with a tiny fixed instruction set, bounded stack, bounded call
depth, host-call opcodes, and no dynamic allocation in the interpreter loop.
Bytecode should be readable from a contiguous package asset or a small streaming
window so it can later live on SD or PSRAM-backed storage.

The first executable contract is defined in
[RUNTIME_BYTECODE_ABI.md](../spec/RUNTIME_BYTECODE_ABI.md).

## Constraints

- RP2040-class SRAM is the limiting resource, so the VM state must fit in the
  "tens of KB" budget from FR-RT-3.
- PSRAM is block storage, not executable or pointer-addressable program memory.
- Core code should stay Rust-first and `no_std` compatible where practical.
- App isolation must come from the VM plus explicit KotoSDK host calls.
- The first prototype needs a predictable host API surface more than it needs a
  rich general-purpose language.

## Candidate Comparison

| Candidate | SRAM footprint estimate | Integration risk | Strengths | Risks |
| :-------- | :---------------------- | :--------------- | :-------- | :---- |
| Custom stack VM | Interpreter state can be kept around 2-16 KB before app stack/heap; code size likely 8-30 KB for a minimal Rust VM. | Low for first prototype; higher long-term because tools and validation are ours. | Best fit for `no_std` Rust, deterministic memory, simple host-call ABI, and current `kotoruntime-bytecode` manifest value. | Requires custom assembler/compiler, bytecode verifier, debugger story, and compatibility discipline. |
| Wasm interpreter | Runtime text size ranges from roughly 56-64 KB for small embedded interpreters before module memory; app linear memory must be explicitly capped. | Medium-high on RP2040 because mature runtimes are C/C++ and need FFI, allocator, and feature trimming. | Strong sandbox model, existing toolchains, language portability, clear module format. | Wasm is designed around linear memory and validation; useful runtimes still bring nontrivial porting and memory tuning work. |
| Lua | VM is lightweight and embeddable, but practical SRAM use depends on GC heap, tables, strings, and loaded libraries; budget likely starts around 32-64 KB for useful apps. | Medium-high because it adds a C VM, allocator policy, and dynamic-language sandboxing rules. | Excellent scripting ergonomics, mature embedding API, fast iteration. | Dynamic tables/strings and GC make strict memory accounting harder; sandbox must remove or wrap much of the standard environment. |
| mruby | Heavier than Lua for this target; useful configurations likely need 64-128 KB+ heap plus VM/code footprint. | High for first prototype because Ruby semantics, gems, GC, and bytecode toolchain add surface area. | Embeddable bytecode VM, friendly language, active upstream. | Too much language/runtime surface for the first constrained VM; C integration and heap pressure are not aligned with the immediate goal. |

## Wasm Note

If a later milestone revisits WebAssembly, prefer evaluating WAMR before Wasm3.
Wasm3 is very small and lists about 64 KB code plus about 10 KB RAM as a minimum
useful system, but it is currently in minimal-maintenance mode. WAMR is larger in
scope but actively maintained, has documented embedded build sizes, and supports
interpreter/AOT modes that can be feature-trimmed.

For KOTO-0018, that still makes Wasm the second path, not the first path:
KotoOS needs to prove host calls, package entry loading, error handling, and
sandbox identity before it needs cross-language portability.

## Prototype Boundary

The first custom VM prototype should include:

- A bytecode header with version, instruction width, entry offset, and declared
  stack/heap limits.
- Integer-first opcodes for constants, arithmetic, local load/store, branches,
  calls/returns, and host calls.
- A checked operand stack and checked call stack.
- A host-call dispatch table shared with KOTO-0019.
- Deterministic errors for invalid opcode, stack underflow/overflow, bad branch,
  bad host call, and exceeded fuel/instruction budget.

The first prototype should explicitly exclude:

- Floating point.
- Garbage collection.
- Threads/coroutines.
- Self-modifying code or writable code pages.
- General dynamic library loading.

## Follow-Up Issues

- KOTO-0019 defines the host API around a VM-neutral dispatch table, with custom
  bytecode using it first.
- A future issue should add the first bytecode verifier and a tiny assembler
  fixture from [RUNTIME_BYTECODE_ABI.md](../spec/RUNTIME_BYTECODE_ABI.md).
- A later Wasm comparison can test WAMR with the same host-call suite once the
  KotoRuntime contract is stable.

## Sources Checked

- Wasm3 README: https://github.com/wasm3/wasm3
- WAMR README: https://github.com/bytecodealliance/wasm-micro-runtime
- Lua 5.4 Reference Manual: https://www.lua.org/manual/5.4/manual.html
- mruby homepage and README: https://mruby.org/ and https://github.com/mruby/mruby
