//! Host-side integration test that drives the VM end-to-end through its *public*
//! API only — no `koto-core`, no platform crates. This both proves the extracted
//! `koto-vm` crate runs standalone and gives later optimization work a reusable
//! harness (build a synthetic program, run a frame, read execution stats).
//!
//! Every program here is hand-assembled with [`insn`]/[`fixture`] and run through
//! the verifier plus interpreter, so the tests *document current behavior* — opcode
//! semantics, the host-call stack ABI, trap conditions, and how [`RuntimeLimits`]
//! gates a program at launch. They deliberately do not redefine any of it; if a
//! result surprises you, the VM is the source of truth, not these assertions.

use koto_vm::{
    host_call, opcode, verify_kbc, BytecodeSession, BytecodeVm, CountingCode, HostCallOutcome,
    HostErrorCode, RuntimeLimits, SessionError, SliceCode, VerifyError, VmError, VmHost,
    VmInputSnapshot, VmRunResult, HOST_ABI_MAJOR, HOST_ABI_MINOR, KBC_HEADER_SIZE, KBC_MAGIC,
    KBC_VERSION_MAJOR, KBC_VERSION_MINOR,
};

/// Encode one 4-byte instruction word in the on-disk little-endian layout
/// (`[imm_lo, imm_hi, operand, opcode]`), matching the in-crate unit-test helper
/// and the assembler's emit format.
fn insn(op: u8, operand: u8, immediate: u16) -> [u8; 4] {
    let imm = immediate.to_le_bytes();
    [imm[0], imm[1], operand, op]
}

/// Build a minimal valid `.kbc` image around a code slice, mirroring the header
/// fields the verifier checks. Kept intentionally close to the in-crate fixture
/// so the synthetic ABI here stays in lock-step with the real one. The header
/// requests an 8-slot operand stack, 4 call frames, and a 256-byte heap.
fn fixture(code: &[[u8; 4]]) -> Vec<u8> {
    let bytecode_size = KBC_HEADER_SIZE + code.len() * 4;
    let mut bytes = vec![0u8; bytecode_size];
    bytes[0..4].copy_from_slice(&KBC_MAGIC);
    bytes[4..6].copy_from_slice(&KBC_VERSION_MAJOR.to_le_bytes());
    bytes[6..8].copy_from_slice(&KBC_VERSION_MINOR.to_le_bytes());
    bytes[8..12].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&(bytecode_size as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
    bytes[24..28].copy_from_slice(&((code.len() * 4) as u32).to_le_bytes());
    bytes[40..42].copy_from_slice(&8u16.to_le_bytes());
    bytes[42..44].copy_from_slice(&4u16.to_le_bytes());
    bytes[44..48].copy_from_slice(&256u32.to_le_bytes());
    bytes[48..50].copy_from_slice(&HOST_ABI_MAJOR.to_le_bytes());
    bytes[50..52].copy_from_slice(&HOST_ABI_MINOR.to_le_bytes());
    for (index, word) in code.iter().enumerate() {
        let offset = KBC_HEADER_SIZE + index * 4;
        bytes[offset..offset + 4].copy_from_slice(word);
    }
    bytes
}

/// The size of the scratch heap the direct-`BytecodeVm` helpers run against; the
/// `fixture` header requests exactly this many heap bytes.
const HEAP_BYTES: usize = 256;

/// Minimal host: records the side-effecting host calls a program makes so a test
/// can assert what it drew and how it was sampled. Every other hostcall keeps the
/// trait's default behavior (notably `draw_pixels_rgb565`, which the default
/// reports `UNSUPPORTED`, used below to exercise the host-call failure ABI).
#[derive(Default)]
struct RecordingHost {
    rects: Vec<(i32, i32, i32, i32, i32)>,
    texts: Vec<(i32, i32, String)>,
    snapshots: Vec<VmInputSnapshot>,
}

impl VmHost for RecordingHost {
    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome {
        self.rects.push((x, y, w, h, rgb565));
        HostCallOutcome::Ok0
    }

    fn draw_text(&mut self, x: i32, y: i32, text: &str) -> HostCallOutcome {
        self.texts.push((x, y, text.to_string()));
        HostCallOutcome::Ok0
    }

    // The remaining no-default hostcalls are unused by these programs; stub them so
    // the trait is satisfied without pulling in a real platform.
    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        self.snapshots.push(input);
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }

    fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn file_read(&mut self, _handle: i32, _buf: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn file_write(&mut self, _handle: i32, _buf: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
}

/// Verify `bytes` under the canonical simulator profile, panicking with context if
/// the synthetic program is malformed (a test-authoring error, not a VM behavior).
fn verified(bytes: &[u8]) -> koto_vm::VerifiedProgram {
    verify_kbc(bytes, RuntimeLimits::simulator_default())
        .expect("synthetic program should verify under the simulator profile")
}

/// Run one frame of `code` through a freshly built `BytecodeVm` over its own
/// 256-byte heap, returning the run result, the VM (for stack/budget inspection),
/// and the heap (for memory effects). This is the public-API analogue of the
/// in-crate `execute_test_frame` helper.
fn run_frame(
    code: &[[u8; 4]],
    host: &mut RecordingHost,
    input: VmInputSnapshot,
    fuel: u32,
) -> (VmRunResult, BytecodeVm<8, 4>, [u8; HEAP_BYTES]) {
    let bytes = fixture(code);
    let program = verified(&bytes);
    let mut vm = BytecodeVm::<8, 4>::new(&program).expect("VM capacity covers the fixture header");
    let mut heap = [0u8; HEAP_BYTES];
    let result = vm
        .execute_frame(&bytes, &program, host, input, fuel, &mut heap)
        .expect("frame should run without a trap");
    (result, vm, heap)
}

#[test]
fn runs_synthetic_program_to_clean_exit_with_stats() {
    // Draw one rect, then exit with code 0 — the smallest program that exercises
    // a host call plus the exit path.
    let bytes = fixture(&[
        insn(opcode::PUSH_I16, 0, 10),
        insn(opcode::PUSH_I16, 0, 20),
        insn(opcode::PUSH_I16, 0, 30),
        insn(opcode::PUSH_I16, 0, 40),
        insn(opcode::PUSH_I16, 0, 0x07e0),
        insn(opcode::HOST_CALL, host_call::DRAW_RECT, 0),
        insn(opcode::DROP, 0, 0),
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ]);

    let frame_fuel = 1_000;
    let mut session =
        BytecodeSession::<8, 4>::new(&bytes, RuntimeLimits::simulator_default(), frame_fuel)
            .expect("synthetic program should verify and initialize");
    let mut host = RecordingHost::default();
    let mut heap = [0u8; 256];

    let result = session
        .step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap)
        .expect("frame should run without a trap");

    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(host.rects, [(10, 20, 30, 40, 0x07e0)]);

    // Execution stats remain available on the public API (the optimization hooks).
    assert_eq!(
        session.last_frame_host_calls(),
        2,
        "DRAW_RECT + EXIT are the two host calls this frame"
    );
    assert!(
        session.last_frame_fuel() < frame_fuel,
        "running instructions must consume fuel"
    );
}

/// Emit a `push addr; push lhs; push rhs; <op>; store32` block: compute `lhs <op>
/// rhs` and write the i32 result to `heap[addr..addr+4]`. The block is operand-
/// stack neutral (peak depth 3), so a sequence of them verifies and runs as one
/// straight-line program.
fn store_binop(addr: u16, lhs: i16, rhs: i16, op: u8) -> [[u8; 4]; 5] {
    [
        insn(opcode::PUSH_I16, 0, addr),
        insn(opcode::PUSH_I16, 0, lhs as u16),
        insn(opcode::PUSH_I16, 0, rhs as u16),
        insn(op, 0, 0),
        insn(opcode::STORE32, 0, 0),
    ]
}

#[test]
fn arithmetic_and_bitwise_opcodes_compute_expected_values() {
    // Every operator writes 42 to a distinct heap word; picking a single result
    // makes the table easy to scan while still exercising each opcode's math.
    let mut code = Vec::new();
    code.extend(store_binop(0, 40, 2, opcode::ADD_I32)); //  40 + 2
    code.extend(store_binop(4, 100, 58, opcode::SUB_I32)); // 100 - 58
    code.extend(store_binop(8, 6, 7, opcode::MUL_I32)); //   6 * 7
    code.extend(store_binop(12, 84, 2, opcode::DIV_I32)); //  84 / 2
    code.extend(store_binop(16, 0x7E, 0x2A, opcode::AND_I32)); // 126 & 42
    code.extend(store_binop(20, 0x28, 0x02, opcode::OR_I32)); //  40 | 2
    code.extend(store_binop(24, 0x3F, 0x15, opcode::XOR_I32)); // 63 ^ 21
    code.extend(store_binop(28, 21, 1, opcode::SHL_I32)); //  21 << 1
    code.extend(store_binop(32, 84, 1, opcode::SHR_I32)); //  84 >> 1
    code.push(insn(opcode::PUSH_I16, 0, 0));
    code.push(insn(opcode::HOST_CALL, host_call::EXIT, 0));

    let mut host = RecordingHost::default();
    let (result, _vm, heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 1_000);

    assert_eq!(result, VmRunResult::Exited(0));
    for word in 0..9 {
        let base = word * 4;
        assert_eq!(
            &heap[base..base + 4],
            &42i32.to_le_bytes(),
            "binary opcode #{word} should compute 42"
        );
    }
}

#[test]
fn arithmetic_edge_cases_match_documented_wrapping_and_shift_semantics() {
    let code = [
        // Signed integer division truncates toward zero: -7 / 2 == -3.
        store_binop(0, -7, 2, opcode::DIV_I32),
        // SHR is a *logical* (unsigned) shift: (-1 as u32) >> 1 == 0x7FFF_FFFF.
        store_binop(4, -1, 1, opcode::SHR_I32),
        // The shift amount is masked to 5 bits, so `1 << 33` == `1 << (33 & 31)` == 2.
        store_binop(8, 1, 33, opcode::SHL_I32),
    ]
    .concat();
    let mut code = code;
    code.push(insn(opcode::PUSH_I16, 0, 0));
    code.push(insn(opcode::HOST_CALL, host_call::EXIT, 0));

    let mut host = RecordingHost::default();
    let (result, _vm, heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 1_000);

    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(
        &heap[0..4],
        &(-3i32).to_le_bytes(),
        "div truncates toward zero"
    );
    assert_eq!(
        &heap[4..8],
        &0x7FFF_FFFFi32.to_le_bytes(),
        "shr is a logical (unsigned) shift"
    );
    assert_eq!(
        &heap[8..12],
        &2i32.to_le_bytes(),
        "shift amount masks to 5 bits"
    );
}

#[test]
fn exit_code_carries_an_arbitrary_computed_value() {
    // EXIT pops the operand on top of the stack as the process exit code, so the
    // result of in-VM arithmetic flows straight out as the run result.
    let code = [
        insn(opcode::PUSH_I16, 0, 3),
        insn(opcode::PUSH_I16, 0, 4),
        insn(opcode::ADD_I32, 0, 0),
        insn(opcode::PUSH_I16, 0, 6),
        insn(opcode::MUL_I32, 0, 0), // (3 + 4) * 6 == 42
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, _vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 100);
    assert_eq!(result, VmRunResult::Exited(42));
}

#[test]
fn branches_and_jumps_drive_a_counting_loop() {
    // Sum 5 + 4 + 3 + 2 + 1 with a back-edge loop: slot 0 is the counter, slot 1
    // the accumulator. BR_IF_ZERO exits the loop, BR forms the back edge. Branch
    // targets are absolute code-word indices, so the layout below is load-bearing.
    let code = [
        insn(opcode::PUSH_I16, 0, 5),
        insn(opcode::STORE_LOCAL, 0, 0), // slot0 = 5 (counter)
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::STORE_LOCAL, 1, 0), // slot1 = 0 (accumulator)
        // word 4: loop head
        insn(opcode::LOAD_LOCAL, 0, 0),
        insn(opcode::BR_IF_ZERO, 0, 15), // counter == 0 -> exit at word 15
        insn(opcode::LOAD_LOCAL, 1, 0),
        insn(opcode::LOAD_LOCAL, 0, 0),
        insn(opcode::ADD_I32, 0, 0),
        insn(opcode::STORE_LOCAL, 1, 0), // acc += counter
        insn(opcode::LOAD_LOCAL, 0, 0),
        insn(opcode::PUSH_I16, 0, 1),
        insn(opcode::SUB_I32, 0, 0),
        insn(opcode::STORE_LOCAL, 0, 0), // counter -= 1
        insn(opcode::BR, 0, 4),          // back edge
        // word 15: loop exit
        insn(opcode::LOAD_LOCAL, 1, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, _vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 1_000);
    assert_eq!(result, VmRunResult::Exited(15));
}

#[test]
fn nested_call_frames_run_and_return_through_ret() {
    // main calls f, f calls g; g and f each write a heap word as a visible side
    // effect, then RET unwinds back to main. Each function is operand-stack neutral
    // so the linear verifier accepts the call sites. The deepest the call stack
    // reaches is 2 frames (main -> f -> g), which budget diagnostics should record.
    let code = [
        insn(opcode::CALL, 0, 3), // main: call f at word 3
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
        // word 3: f
        insn(opcode::CALL, 0, 8), // f: call g at word 8
        insn(opcode::PUSH_I16, 0, 4),
        insn(opcode::PUSH_I16, 0, 7),
        insn(opcode::STORE32, 0, 0), // heap[4] = 7
        insn(opcode::RET, 0, 0),
        // word 8: g
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::PUSH_I16, 0, 42),
        insn(opcode::STORE32, 0, 0), // heap[0] = 42
        insn(opcode::RET, 0, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, vm, heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 1_000);

    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(&heap[0..4], &42i32.to_le_bytes());
    assert_eq!(&heap[4..8], &7i32.to_le_bytes());
    assert_eq!(
        vm.budget().call_depth_peak,
        2,
        "main -> f -> g is two nested return frames"
    );
}

#[test]
fn top_level_ret_and_halt_both_exit_zero() {
    // Under the simulator profile (`treat_ret_as_exit == true`), a RET with an
    // empty call stack exits the program with code 0, as does HALT.
    let mut host = RecordingHost::default();
    let (ret_result, _vm, _heap) = run_frame(
        &[insn(opcode::RET, 0, 0)],
        &mut host,
        VmInputSnapshot::empty(),
        10,
    );
    assert_eq!(ret_result, VmRunResult::Exited(0));

    let (halt_result, _vm, _heap) = run_frame(
        &[insn(opcode::HALT, 0, 0)],
        &mut host,
        VmInputSnapshot::empty(),
        10,
    );
    assert_eq!(halt_result, VmRunResult::Exited(0));
}

#[test]
fn locals_round_trip_through_load_and_store() {
    // STORE_LOCAL pops the stack into slot N; LOAD_LOCAL pushes slot N back. Store
    // a value into a high slot, read it back, and exit with it to prove the slot
    // file survived the intervening pushes.
    let code = [
        insn(opcode::PUSH_I16, 0, 123),
        insn(opcode::STORE_LOCAL, 7, 0), // slot 7 = 123
        insn(opcode::PUSH_I16, 0, 999),  // clobber the operand stack
        insn(opcode::DROP, 0, 0),
        insn(opcode::LOAD_LOCAL, 7, 0), // read slot 7 back
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 100);
    assert_eq!(result, VmRunResult::Exited(123));
    assert_eq!(
        vm.budget().local_slots_peak,
        8,
        "touching slot index 7 reports 8 slots in use"
    );
}

#[test]
fn draw_text_reads_a_utf8_string_from_the_heap() {
    // Stage "Hi" at heap offset 0, then DRAW_TEXT(x=1, y=2, ptr=0, len=2). The VM
    // decodes the heap bytes as UTF-8 before handing them to the host.
    let code = [
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::PUSH_I16, 0, 0x6948), // 'H'=0x48 low, 'i'=0x69 high
        insn(opcode::STORE16, 0, 0),
        insn(opcode::PUSH_I16, 0, 1), // x
        insn(opcode::PUSH_I16, 0, 2), // y
        insn(opcode::PUSH_I16, 0, 0), // ptr
        insn(opcode::PUSH_I16, 0, 2), // len
        insn(opcode::HOST_CALL, host_call::DRAW_TEXT, 0),
        insn(opcode::DROP, 0, 0),
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, _vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 100);
    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(host.texts, [(1, 2, "Hi".to_string())]);
}

#[test]
fn host_call_results_push_values_then_a_status_word() {
    // INPUT_SNAPSHOT returns two values; the VM pushes them followed by a 0 status
    // word, so a YIELD afterwards leaves the stack as [held, pressed, status, yield]
    // from bottom to top. The host returns (held, pressed) for the snapshot.
    let code = [
        insn(opcode::HOST_CALL, host_call::INPUT_SNAPSHOT, 0),
        insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
    ];
    let input = VmInputSnapshot {
        held_bits: 0x12,
        pressed_bits: 0x34,
        ..VmInputSnapshot::empty()
    };
    let mut host = RecordingHost::default();
    let (result, mut vm, _heap) = run_frame(&code, &mut host, input, 100);

    assert_eq!(result, VmRunResult::Yielded);
    assert_eq!(host.snapshots, [input]);
    assert_eq!(vm.pop_value().unwrap(), 0, "yield status");
    assert_eq!(vm.pop_value().unwrap(), 0, "input_snapshot status");
    assert_eq!(vm.pop_value().unwrap(), 0x34, "pressed_bits");
    assert_eq!(vm.pop_value().unwrap(), 0x12, "held_bits");
}

#[test]
fn failing_host_call_pushes_a_negative_status_with_fixed_arity() {
    // DRAW_PIXELS_RGB565 has no default implementation, so the trait reports it
    // UNSUPPORTED. A no-result failure pushes exactly one value — the negated error
    // code — keeping static and runtime stack accounting in lock-step. An empty
    // (ptr=0, len=0) heap slice is in bounds, so this reaches the host, not a trap.
    let code = [
        insn(opcode::PUSH_I16, 0, 0), // x
        insn(opcode::PUSH_I16, 0, 0), // y
        insn(opcode::PUSH_I16, 0, 0), // w
        insn(opcode::PUSH_I16, 0, 0), // h
        insn(opcode::PUSH_I16, 0, 0), // ptr
        insn(opcode::PUSH_I16, 0, 0), // len
        insn(opcode::HOST_CALL, host_call::DRAW_PIXELS_RGB565, 0),
        insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, mut vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 100);

    assert_eq!(result, VmRunResult::Yielded);
    assert_eq!(vm.pop_value().unwrap(), 0, "yield status");
    assert_eq!(
        vm.pop_value().unwrap(),
        -HostErrorCode::UNSUPPORTED.0,
        "unsupported host call reports its negated error code"
    );
}

#[test]
fn division_by_zero_traps_and_is_recorded_on_the_session() {
    let bytes = fixture(&[
        insn(opcode::PUSH_I16, 0, 1),
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::DIV_I32, 0, 0),
    ]);
    let mut session =
        BytecodeSession::<8, 4>::new(&bytes, RuntimeLimits::simulator_default(), 100).unwrap();
    let mut host = RecordingHost::default();
    let mut heap = [0u8; 256];

    let err = session.step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap);
    assert_eq!(err, Err(VmError::DivisionByZero));
    assert_eq!(session.last_error(), Some(VmError::DivisionByZero));
    assert_eq!(
        session.frame(),
        1,
        "a trapping frame still counts as stepped"
    );
}

#[test]
fn out_of_bounds_heap_access_traps() {
    // ptr 300 + a 4-byte load runs past the 256-byte heap the fixture requests.
    let bytes = fixture(&[insn(opcode::PUSH_I16, 0, 300), insn(opcode::LOAD32, 0, 0)]);
    let mut session =
        BytecodeSession::<8, 4>::new(&bytes, RuntimeLimits::simulator_default(), 100).unwrap();
    let mut host = RecordingHost::default();
    let mut heap = [0u8; 256];

    let err = session.step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap);
    assert_eq!(err, Err(VmError::MemoryOutOfBounds));
    assert_eq!(session.last_error(), Some(VmError::MemoryOutOfBounds));
}

#[test]
fn fuel_exhaustion_preserves_state_and_resumes_next_frame() {
    // `BR 0` spins forever, so each frame burns its whole fuel budget and reports
    // FuelExhausted without losing the program counter — the session simply resumes
    // on the next step.
    let bytes = fixture(&[insn(opcode::BR, 0, 0)]);
    let frame_fuel = 4;
    let mut session =
        BytecodeSession::<8, 4>::new(&bytes, RuntimeLimits::simulator_default(), frame_fuel)
            .unwrap();
    let mut host = RecordingHost::default();
    let mut heap = [0u8; 256];

    let first = session
        .step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap)
        .unwrap();
    assert_eq!(first, VmRunResult::FuelExhausted);
    assert_eq!(
        session.pc(),
        0,
        "the spinning PC is preserved across frames"
    );
    assert_eq!(
        session.last_frame_fuel(),
        frame_fuel,
        "the whole budget was spent"
    );
    assert_eq!(session.frame(), 1);

    let second = session
        .step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap)
        .unwrap();
    assert_eq!(second, VmRunResult::FuelExhausted);
    assert_eq!(session.frame(), 2, "the session keeps stepping");
}

#[test]
fn runtime_limits_reject_a_program_requesting_more_than_offered() {
    // The fixture header requests an 8-slot stack and a 256-byte heap. Limits that
    // offer less reject the program at verification time (before it can launch),
    // surfaced as SessionError::Verify(ResourceLimitExceeded).
    let bytes = fixture(&[
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ]);

    let tight_stack = RuntimeLimits {
        max_stack_slots: 4,
        ..RuntimeLimits::simulator_default()
    };
    assert_eq!(
        BytecodeSession::<8, 4>::new(&bytes, tight_stack, 100).map(|_| ()),
        Err(SessionError::Verify(VerifyError::ResourceLimitExceeded))
    );

    let tight_heap = RuntimeLimits {
        max_heap_bytes: 128,
        ..RuntimeLimits::simulator_default()
    };
    assert_eq!(
        BytecodeSession::<8, 4>::new(&bytes, tight_heap, 100).map(|_| ()),
        Err(SessionError::Verify(VerifyError::ResourceLimitExceeded))
    );
}

#[test]
fn treat_ret_as_exit_limit_changes_whether_a_bare_ret_verifies() {
    // The same single-RET program is accepted when the profile treats a top-level
    // RET as an exit, and rejected as a bad instruction when it does not — the
    // behavior toggles purely on the RuntimeLimits flag, with no bytecode change.
    let bytes = fixture(&[insn(opcode::RET, 0, 0)]);

    assert!(
        verify_kbc(&bytes, RuntimeLimits::simulator_default()).is_ok(),
        "simulator profile treats top-level RET as exit"
    );

    let no_ret_exit = RuntimeLimits {
        treat_ret_as_exit: false,
        ..RuntimeLimits::simulator_default()
    };
    assert_eq!(
        verify_kbc(&bytes, no_ret_exit),
        Err(VerifyError::BadInstruction),
        "without ret-as-exit a top-level RET is rejected"
    );
}

#[test]
fn verifier_enforces_the_header_stack_bound() {
    // A header may request fewer slots than the runtime ceiling; the verifier then
    // holds the program to that smaller bound. Three pushes against a 2-slot
    // request overflow statically, before the program ever runs.
    let mut bytes = fixture(&[
        insn(opcode::PUSH_I16, 0, 1),
        insn(opcode::PUSH_I16, 0, 2),
        insn(opcode::PUSH_I16, 0, 3),
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ]);
    bytes[40..42].copy_from_slice(&2u16.to_le_bytes()); // request only 2 stack slots

    assert_eq!(
        verify_kbc(&bytes, RuntimeLimits::simulator_default()),
        Err(VerifyError::StackOverflow)
    );
}

#[test]
fn session_stats_accumulate_across_frames() {
    // A frame loop that yields every frame: YIELD pushes a 0 status, the next frame
    // DROPs it, then BR closes the back edge into the yield. The first frame runs
    // just the YIELD (1 instruction); every later frame resumes at the DROP and runs
    // DROP + BR + YIELD (3 instructions). Each frame dispatches exactly one host call
    // (YIELD_FRAME), so the cumulative session counters are fully determined.
    let bytes = fixture(&[
        insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0), // word 0
        insn(opcode::DROP, 0, 0),                           // word 1: drop yield status
        insn(opcode::BR, 0, 0),                             // word 2: back to the yield
    ]);

    let frame_fuel = 100;
    let mut session =
        BytecodeSession::<8, 4>::new(&bytes, RuntimeLimits::simulator_default(), frame_fuel)
            .expect("synthetic program should verify and initialize");
    let mut host = RecordingHost::default();
    let mut heap = [0u8; 256];

    for _ in 0..3 {
        let result = session
            .step_frame(&bytes, &mut host, VmInputSnapshot::empty(), &mut heap)
            .expect("each frame yields without trapping");
        assert_eq!(result, VmRunResult::Yielded);
    }

    let stats = session.stats();
    assert_eq!(stats.frames, 3, "three frames were stepped");
    assert_eq!(stats.host_calls, 3, "one YIELD_FRAME host call per frame");
    assert_eq!(
        stats.instructions, 7,
        "1 instruction the first frame, then 3 each for the next two"
    );

    // The per-frame fuel split mirrors the steady-state frame: 3 consumed, the rest
    // left over.
    assert_eq!(session.last_frame_fuel(), 3);
    assert_eq!(session.last_frame_fuel_remaining(), frame_fuel - 3);
    assert_eq!(session.frame_fuel(), frame_fuel);
}

#[test]
fn counting_code_source_tallies_every_word_fetch() {
    // Each executed instruction fetches exactly one code word, so a transparent
    // CountingCode wrapper over the resident SliceCode reports one read per stepped
    // instruction without changing what the VM runs. The program steps PUSH + EXIT.
    let bytes = fixture(&[
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ]);
    let program = verified(&bytes);
    let mut vm = BytecodeVm::<8, 4>::new(&program).expect("VM capacity covers the fixture header");
    let mut host = RecordingHost::default();
    let mut heap = [0u8; HEAP_BYTES];
    let mut code = CountingCode::new(SliceCode::new(&bytes, program.code_range().0));

    let result = vm
        .execute_frame_with(
            &mut code,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100,
            &mut heap,
        )
        .expect("frame should run without a trap");

    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(vm.last_frame_fuel(), 2, "PUSH + EXIT are two instructions");
    assert_eq!(
        code.reads(),
        u64::from(vm.last_frame_fuel()),
        "one word fetch per executed instruction"
    );
    assert_eq!(
        code.bytes_read(),
        code.reads() * 4,
        "a code word is four bytes"
    );
    assert_eq!(vm.stats().instructions, 2);
}

#[cfg(feature = "opcode_stats")]
#[test]
fn opcode_counts_tally_each_executed_opcode() {
    // (2 + 3) then exit: two PUSH operands, one PUSH for the exit code, one ADD, one
    // HOST_CALL. With the `opcode_stats` feature the VM tallies each decoded opcode.
    let code = [
        insn(opcode::PUSH_I16, 0, 2),
        insn(opcode::PUSH_I16, 0, 3),
        insn(opcode::ADD_I32, 0, 0),
        insn(opcode::PUSH_I16, 0, 0),
        insn(opcode::HOST_CALL, host_call::EXIT, 0),
    ];
    let mut host = RecordingHost::default();
    let (result, vm, _heap) = run_frame(&code, &mut host, VmInputSnapshot::empty(), 100);

    assert_eq!(result, VmRunResult::Exited(0));
    let counts = vm.opcode_counts();
    assert_eq!(counts[usize::from(opcode::PUSH_I16)], 3);
    assert_eq!(counts[usize::from(opcode::ADD_I32)], 1);
    assert_eq!(counts[usize::from(opcode::HOST_CALL)], 1);
    assert_eq!(
        counts.iter().copied().sum::<u64>(),
        u64::from(vm.last_frame_fuel()),
        "every stepped instruction is tallied exactly once"
    );
}
