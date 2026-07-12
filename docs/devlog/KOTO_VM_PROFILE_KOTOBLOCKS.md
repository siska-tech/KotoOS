# KOTO-VM-0005: Profile KotoBlocks VM execution

Host-side execution profile of the `koto_blocks.kbc` fixture, captured through the
existing koto-sim bytecode fixture runner. This is an **observation** document: it
records what the VM did and flags likely optimization targets. It proposes no code
changes and changes no VM semantics, opcode values, the bytecode ABI, hostcall IDs,
`RuntimeLimits`, the `CodeSource`/`VmHost` traits, firmware, PSRAM, the CodeWindow,
graphics, audio, or app bytecode.

## How this was captured

The numbers come from the `koto_blocks_runs_and_reports_metrics` test in
[fixture_runner.rs](../../src/koto-sim/tests/fixture_runner.rs), which runs the fixture
through the public `BytecodeSession` API under the canonical simulator profile
(`RuntimeLimits::simulator_default()`, `SIM_FRAME_FUEL`, `SIM_VM_STACK_SLOTS`,
`SIM_VM_CALL_DEPTH`) against a `RecordingHost` mock and a `CountingCode` wrapper.

```sh
# Core metrics (VmStats, VmBudget, CountingCode, per-frame, hostcall mix):
cargo test -p koto-sim --test fixture_runner koto_blocks -- --nocapture

# Same, plus the per-opcode histogram (opt-in, feature-gated all the way down to koto-vm):
cargo test -p koto-sim --features opcode_stats --test fixture_runner koto_blocks -- --nocapture
```

The profiler is read-only: `CountingCode` and `RecordingHost` only observe, and the
`opcode_stats` histogram is opt-in and gated out of default and device builds. The
mock host returns success for every call so the fixture follows its normal path; it
owns no real graphics/audio/file backend, so absolute *wall-clock* timing is not
meaningful here — instruction, hostcall, code-fetch, and budget counts are.

- fixture: `koto_blocks.kbc`
- frames stepped: 8
- per-frame fuel budget: 60,000 instructions
- input: empty snapshot every frame (no key presses)

---

## Observations

### Cumulative (8 frames)

| Metric | Value |
| --- | --- |
| Instructions | 68,217 |
| VM-level host_calls | 62 |
| Recorded (external) hostcalls | 54 |
| Internal hostcalls (yield_frame/exit) | 8 |
| Failed hostcalls | 0 |
| Hostcall args consumed | 236 |
| CountingCode reads | 68,217 (~272,868 bytes) |
| Draws / audio / inputs | 46 / 0 / 8 |
| Final result | `Yielded` (no trap, no early exit) |

### Budget high-water peaks vs. simulator capacity

| Peak | Observed | Capacity | Headroom |
| --- | --- | --- | --- |
| stack_slots | 7 | 16 | 56% free |
| call_depth | 0 | 4 | never recurses |
| local_slots | 46 | 48 (`VM_LOCAL_SLOTS`) | 2 free (was 48/48 before KOTO-0146) |
| heap_bytes addressed | 5,015 | per-app header request | — |
| frame_fuel (busiest frame) | 9,718 | 60,000 | 84% free |
| hostcalls / frame | 9 | — | — |

### Per-frame breakdown

| Frame | Instructions | Fuel left | VM hostcalls | Recorded | Code reads | Result |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 9,718 | 50,282 | 7 | 6 | 9,718 | Yielded |
| 1 | 8,315 | 51,685 | 7 | 6 | 8,315 | Yielded |
| 2 | 8,315 | 51,685 | 7 | 6 | 8,315 | Yielded |
| 3 | 8,361 | 51,639 | 8 | 7 | 8,361 | Yielded |
| 4 | 8,376 | 51,624 | 8 | 7 | 8,376 | Yielded |
| 5 | 8,362 | 51,638 | 8 | 7 | 8,362 | Yielded |
| 6 | 8,362 | 51,638 | 8 | 7 | 8,362 | Yielded |
| 7 | 8,408 | 51,592 | 9 | 8 | 8,408 | Yielded |

### Recorded hostcall mix (external dispatches)

| ID | Name | Count |
| --- | --- | --- |
| `0x10` | draw_rect | 24 |
| `0x12` | draw_pixels_rgb565 | 6 |
| `0x13` | draw_text_color | 16 |
| `0x21` | text_input | 8 |

### Opcode histogram (`--features opcode_stats`)

| Opcode | Name | Count | % of instructions |
| --- | --- | --- | --- |
| `0x10` | push_i16 | 19,665 | 28.8% |
| `0x31` | sub_i32 | 9,960 | 14.6% |
| `0x13` | swap | 7,256 | 10.6% |
| `0x35` | or_i32 | 6,715 | 9.8% |
| `0x20` | load_local | 6,069 | 8.9% |
| `0x38` | shr_i32 | 5,833 | 8.6% |
| `0x11` | dup | 5,156 | 7.6% |
| `0x30` | add_i32 | 2,649 | 3.9% |
| `0x41` | store8 | 1,425 | 2.1% |
| `0x21` | store_local | 1,292 | 1.9% |
| `0x03` | br_if_zero | 1,186 | 1.7% |
| `0x02` | br | 703 | 1.0% |
| `0x32` | mul_i32 | 85 | 0.1% |
| `0x50` | host_call | 62 | 0.1% |
| `0x12` | drop | 61 | 0.1% |
| `0x34` | and_i32 | 48 | 0.1% |
| `0x37` | shl_i32 | 46 | 0.1% |
| `0x33` | div_i32 | 6 | 0.0% |

### What the numbers say

1. **Steady state ~8,330 instr/frame; frame 0 is a ~1,400-instruction warm-up.**
   The first frame (9,718) runs one-time setup the later frames skip; frames
   1–7 sit in a tight ~8,315–8,408 band. The slow upward drift across 3→7 tracks
   one extra recorded hostcall per later frame (6 → 7 → 8), i.e. incremental UI as
   the run progresses, not a leak.

2. **Code reads equal instructions exactly (68,217 = 68,217).** This harness reads
   resident code through `SliceCode`, which serves one 4-byte word per stepped
   instruction and never refills, so the fetch count is exactly the instruction
   count. The device path streams the same code through the PSRAM `CodeWindow`,
   where the meaningful cost is window *refills*, which `SliceCode` reports as zero.
   This harness therefore measures **instruction count, the upstream driver of
   device code fetches**, not refill thrash directly.

3. **The VM is not the bottleneck under the simulator profile.** The busiest frame
   uses 9,718 of 60,000 fuel (16%); stack peaks at 7 of 16; the call stack never
   grows (KotoBlocks is single-function — no `CALL`/`RET` in the histogram). Local
   slots peak at **46 of 48 (`VM_LOCAL_SLOTS`)**: 43 user slots plus the 3 codegen
   scratch slots, which now float just above the user high-water mark (KOTO-0146)
   instead of pinning at the top of the file. The original capture read 48/48 purely
   because the scratch slots sat at indices 45/46/47 and `local_slots_peak` tracks
   the highest slot *index* touched, not a live count — so any app using `%` or a
   return value reported 48 regardless of real pressure. The actionable figure is the
   compiler's static `user_slots_used` (43/45).

4. **External hostcalls are negligible cost (62 of 68,217 instructions, 0.1%).**
   The VM's host_calls counter (62) exceeds the host's recorded calls (54) by
   exactly 8 — one internally-handled `yield_frame` per stepped frame, which the VM
   counts but never routes to a `VmHost` method.

5. **This capture only exercises the title screen, which is immediate-draw by
   design; gameplay renders through the retained Game2D layers.** The input here is
   an empty snapshot every frame, so the app never crosses its title->play
   transition (that needs an F1 / newline intent) and stays on the title screen for
   all 8 frames. The title screen — tile-cache bake, palette showcase, pulsing start
   button — is intentionally immediate (`draw_rect` / `draw_pixels_rgb565` /
   `draw_text_color`, zero `game2d_*`). It is *not* representative of play: once
   started, the board, locked cells, active piece, ghost, NEXT/HOLD previews, and
   the score/level/lines values all render through the retained Game2D tilemap
   (KOTO-0135), static UI (KOTO-0136), sprite (KOTO-0140), and text (KOTO-0141)
   layers, and steady-state play emits **no** immediate draws — only transient
   effects (line-clear flash, hard-drop trail, action-flash halo) and the
   pause/game-over/banner overlays do. The
   `koto_blocks_play_uses_retained_game2d_layers` test in
   [fixture_runner.rs](../../src/koto-sim/tests/fixture_runner.rs) scripts the input
   past the transition and asserts the `game2d_set_tile` / `game2d_sprite_set` /
   `game2d_text_set` / `game2d_static_begin` / `game2d_stamp_define` /
   `game2d_present` calls that prove this. To re-profile *play* rather than the
   title screen, drive that same scripted input (or
   `harness/fixtures/budget/koto_blocks.script`) instead of the empty snapshot.

6. **The instruction mix is push/arith/stack-shuffle dominated.** `push_i16` alone
   is 28.8%; integer arithmetic (`sub`/`or`/`shr`/`add`/`mul`/`and`/`shl`) totals
   ~37%; pure stack juggling (`swap` + `dup`) is ~18%. Memory and control flow are
   small: `store8`/`store_local` ~4% combined, branches ~2.7%. `or_i32` + `shr_i32`
   at ~18% combined is the signature of RGB565 pixel/color packing and 16-cell tile
   address math.

---

## Optimization targets (proposals — not implemented here)

Ordered by estimated payoff. Each notes the layer it would touch; none requires a
VM-semantics, opcode, ABI, hostcall-ID, or `RuntimeLimits` change unless called out,
and all are explicitly out of scope for this profiling task.

1. **Compiler peephole for stack shuffling (`swap` + `dup` ≈ 18%).** 12,412
   instructions/run do nothing but reorder the operand stack. A peephole pass in
   the Koto compiler that better orders sub-expression evaluation (or reuses a local
   instead of `dup`/`swap`) would cut these directly. *Layer: koto-compiler only —
   no VM or ABI change.* Highest payoff, lowest risk.

2. **Constant materialization (`push_i16` ≈ 29%).** Nearly a third of all
   instructions push literals. Much of this is likely recomputed tile/cell
   geometry constants inside the per-frame loop. Compiler-side constant hoisting out
   of loops, or constant folding of `push;push;<arith>` chains, would shrink the hot
   path. *Layer: koto-compiler — no VM or ABI change.* A *new* opcode (wide-immediate
   or push-pair) would also help but is an ABI change and therefore out of scope.

3. **Move per-cell rendering onto the retained Game2D tile/sprite layers. — Already
   done (KOTO-0135/0136/0140/0141); the empty-input capture masked it.** The premise
   here — that KotoBlocks "repaints via immediate `draw_rect`/`draw_pixels`/
   `draw_text_color` every frame" — was inferred from the title-screen-only hostcall
   mix (observation 5) and does not hold for play. Gameplay already writes only
   changed cells to the retained tilemap (`game2d_set_tile`), places the active
   piece / ghost / NEXT / HOLD as retained sprites (`game2d_sprite_set`), keeps the
   fixed UI in the retained static layer, and renders the score/level/lines through
   the retained text layer — so the host owns compositing and steady-state play emits
   no immediate board math. The remaining immediate draws in play are transient
   effects and overlays (line-clear flash, hard-drop trail, action-flash halo, the
   pause/game-over/banner panels, and the game-over board's flat dim rects, which use
   rects deliberately for the simulator's rect-before-pixel compositor order). These
   are one-frame / few-frame visuals with no clean mapping onto the persistent
   retained primitives (a tile/sprite/text item lives until explicitly hidden), so
   they stay immediate by design — a retained *transient-overlay* primitive is the
   missing API, intentionally not invented here. *Layer: app bytecode + host — uses
   existing ABI, no VM change.* Proven by the
   `koto_blocks_play_uses_retained_game2d_layers` fixture-runner test.

4. **Relieve the local-slot ceiling (48/48). — Done (KOTO-0146).** KotoBlocks read
   48/48 only because the 3 codegen scratch slots were pinned at the top of the file
   (indices 45/46/47), so `local_slots_peak` (the highest slot *index* touched) hit
   48 for any app using `%` or a return value. The scratch slots now float to
   `max_slot..max_slot + 3`, just above the program's actual user-slot high-water
   mark, so the reported peak tracks real pressure: KotoBlocks drops to 46/48 with no
   change to user-slot allocation, the bytecode ABI, opcode values, hostcall IDs,
   `RuntimeLimits`, or interpreter semantics (only the slot *indices* in emitted
   `load_local`/`store_local` operands move). Further reduction of the 43 user slots
   would require true per-statement liveness reuse; the existing per-scope (KOTO-0092)
   and call-site (KOTO-0104) reuse, plus this app's hand-packed scratch locals, leave
   little slack there. *Layer: koto-compiler.*

### Non-targets (observed but deliberately left alone)

- **Frame fuel headroom.** 84% of the per-frame budget is unused, but 60,000 is a
  deliberately sized interactive-app ceiling with headroom over the heaviest
  KotoBlocks frame; this profile is not a reason to retune it.
- **External hostcall cost.** At 0.1% of instructions, the hostcall path is not a
  profitable target on this fixture.
- **VM interpreter dispatch.** Real dispatch/decode cost is invisible on this
  resident-`SliceCode` host harness (reads == instructions); it must be measured on
  the device PSRAM `CodeWindow` path, where refills — not raw fetch count — dominate.

## Non-goals

- No ABI, opcode, hostcall-ID, or `RuntimeLimits` changes.
- No CodeWindow / PSRAM changes.
- No VM-semantics or trait-signature changes; `koto-vm` stays `no_std`.
- Any added profiling code stays test-only and feature-gated.
