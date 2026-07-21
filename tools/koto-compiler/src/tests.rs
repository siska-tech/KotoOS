use super::*;
use koto_core::runtime::VmBudget;
use koto_core::{
    BytecodeVm, HostCallOutcome, RuntimeLimits, VerifiedProgram, VmHost, VmInputSnapshot,
    VmRunResult,
};

#[derive(Default)]
struct CaptureHost {
    text: Vec<(i32, i32, String)>,
    ui_mounts: Vec<Vec<u8>>,
    ui_updates: Vec<Vec<u8>>,
    ui_events: Vec<Vec<u8>>,
    ui_presents: usize,
    reject_ui_update: bool,
}

impl VmHost for CaptureHost {
    fn draw_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _rgb565: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
    fn draw_text(&mut self, x: i32, y: i32, text: &str) -> HostCallOutcome {
        self.text.push((x, y, text.to_string()));
        HostCallOutcome::Ok0
    }
    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }
    fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
    }
    fn file_read(&mut self, _handle: i32, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }
    fn file_write(&mut self, _handle: i32, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }
    fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
    fn ui_capabilities(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        if dst.len() < 64 {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY);
        }
        dst[..64].fill(0);
        dst[..4].copy_from_slice(b"KUC1");
        dst[4..6].copy_from_slice(&1u16.to_le_bytes());
        dst[30..32].copy_from_slice(&1u16.to_le_bytes());
        dst[32] = 5;
        dst[36..40].copy_from_slice(&1u32.to_le_bytes());
        dst[40..45].copy_from_slice(b"en-US");
        HostCallOutcome::Ok1(64)
    }
    fn ui_mount(&mut self, src: &[u8]) -> HostCallOutcome {
        self.ui_mounts.push(src.to_vec());
        HostCallOutcome::Ok0
    }
    fn ui_update(&mut self, src: &[u8]) -> HostCallOutcome {
        self.ui_updates.push(src.to_vec());
        if self.reject_ui_update {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY);
        }
        HostCallOutcome::Ok0
    }
    fn ui_present(&mut self) -> HostCallOutcome {
        self.ui_presents += 1;
        HostCallOutcome::Ok0
    }
    fn ui_poll_event(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        if self.ui_events.is_empty() {
            return HostCallOutcome::Ok1(0);
        }
        let event = self.ui_events.remove(0);
        if event.len() > dst.len() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY);
        }
        dst[..event.len()].copy_from_slice(&event);
        HostCallOutcome::Ok1(event.len() as i32)
    }
    fn ui_reset(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
}

#[derive(Default)]
struct UiSuccessHost;

impl VmHost for UiSuccessHost {
    fn draw_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _rgb565: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn draw_text(&mut self, _x: i32, _y: i32, _text: &str) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }

    fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
    }

    fn file_read(&mut self, _handle: i32, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }

    fn file_write(&mut self, _handle: i32, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }

    fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn ui_capabilities(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Ok1(64)
    }

    fn ui_mount(&mut self, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn ui_update(&mut self, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn ui_present(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn ui_poll_event(&mut self, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Ok1(0)
    }

    fn ui_reset(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
}

#[derive(Default)]
struct OlderHost;

impl VmHost for OlderHost {
    fn draw_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _rgb565: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn draw_text(&mut self, _x: i32, _y: i32, _text: &str) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }

    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }

    fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
    }

    fn file_read(&mut self, _handle: i32, _dst: &mut [u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }

    fn file_write(&mut self, _handle: i32, _src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
    }

    fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
        HostCallOutcome::Ok0
    }
}

/// Compile, verify, then run one frame through the VM with a capture host.
fn run(source: &str) -> (VmRunResult, CaptureHost) {
    let (result, host, _) = run_with_budget(source);
    (result, host)
}

/// Compile, verify, run one frame, and return the VM budget high-water marks.
fn run_with_budget(source: &str) -> (VmRunResult, CaptureHost, VmBudget) {
    let bytecode = compile("test.koto", source).expect("compiles");
    let program: VerifiedProgram =
        verify_kbc(&bytecode, RuntimeLimits::simulator_default()).expect("verifies");
    let mut vm = BytecodeVm::<16, 4>::new(&program).expect("vm");
    // Initialize the heap from the const heap image, exactly as the real loaders do
    // (KOTO-0139): rodata becomes heap[0..rodata_size], the rest stays zeroed.
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    let result = vm
        .execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .expect("runs without trapping");
    let budget = vm.budget();
    (result, host, budget)
}

#[test]
fn compiles_and_runs_exit_code() {
    let (result, _) = run("fn main() { exit(7); }");
    assert_eq!(result, VmRunResult::Exited(7));
}

#[test]
fn integer_enums_autoincrement_alias_and_lower_without_runtime_cost() {
    let source = "enum Domain { Zero, One, Alias = 1, Negative = -7, Next, }\nfn main() { exit(Domain::One + Domain::Alias + Domain::Negative + Domain::Next); }\n";
    assert_eq!(run(source).0, VmRunResult::Exited(-11));

    let enum_bytes = compile(
        "enum.koto",
        "enum Domain { Value = 42, }\nfn main() { exit(Domain::Value); }\n",
    )
    .unwrap();
    let const_bytes = compile(
        "const.koto",
        "const VALUE = 42;\nfn main() { exit(VALUE); }\n",
    )
    .unwrap();
    let enum_program = verify_kbc(&enum_bytes, RuntimeLimits::simulator_default()).unwrap();
    let const_program = verify_kbc(&const_bytes, RuntimeLimits::simulator_default()).unwrap();
    let (enum_start, enum_end) = enum_program.code_range();
    let (const_start, const_end) = const_program.code_range();
    assert_eq!(
        &enum_bytes[enum_start..enum_end],
        &const_bytes[const_start..const_end]
    );
    assert_eq!(
        enum_program.rodata_range().map(|(s, e)| &enum_bytes[s..e]),
        const_program
            .rodata_range()
            .map(|(s, e)| &const_bytes[s..e])
    );
}

#[test]
fn enum_members_work_in_compile_time_integer_intrinsics() {
    let source = "enum Actors { Count = 2, }\nfn main() { let actors = actor_array_new(Actors::Count); actor_set_pos(actors, 1, 42, 0); exit(actor_x(actors, 1)); }";
    assert_eq!(run(source).0, VmRunResult::Exited(42));
}

#[test]
fn enum_diagnostics_cover_duplicates_unknowns_and_bounds() {
    for (source, expected) in [
        ("enum E { A, A, } fn main() {}", "already defined"),
        (
            "enum E { A, } enum E { B, } fn main() {}",
            "already defined",
        ),
        ("fn main() { exit(Missing::A); }", "unknown enum"),
        (
            "enum E { A, } fn main() { exit(E::Missing); }",
            "unknown enum member",
        ),
        ("enum E { A = 2147483648, } fn main() {}", "32-bit range"),
        (
            "enum E { A = 2147483647, B, } fn main() {}",
            "implicit value overflows",
        ),
        (
            "enum E { A = nope, } fn main() {}",
            "signed integer literal",
        ),
    ] {
        let error = compile("enum_error.koto", source).expect_err(source);
        assert!(error.message.contains(expected), "{error}");
    }
}

#[test]
fn enum_include_collision_is_attributed_to_included_file() {
    let root = "enum Shared { Root, }\ninclude \"duplicate.koto\";\nfn main() {}\n";
    let error = compile_with_loader(
        "main.koto",
        root,
        CodegenOptions::default(),
        &mut loader(&[("duplicate.koto", "enum Shared { Included, }\n")]),
    )
    .expect_err("duplicate enum");
    assert_eq!((error.file.as_str(), error.line), ("duplicate.koto", 1));
}

#[test]
fn sdk_enum_and_flat_alias_emit_the_same_host_argument() {
    let qualified = compile_to_asm(
        "sdk_enum.koto",
        "fn main() { file_open(\"x\", 1, FileMode::Read); exit(0); }",
    )
    .unwrap();
    let flat = compile_to_asm(
        "sdk_flat.koto",
        "fn main() { file_open(\"x\", 1, MODE_READ); exit(0); }",
    )
    .unwrap();
    let executable = |asm: &str| {
        asm.lines()
            .filter(|line| {
                !line.trim_start().starts_with(".loc")
                    && !line.trim_start().starts_with(".debug_file")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(executable(&qualified), executable(&flat));
}

#[test]
fn arithmetic_and_locals() {
    let (result, _) = run("
        fn main() {
            let a = 40;
            let b = 2;
            exit(a + b);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(42));
}

#[test]
fn wide_integer_constants_materialize() {
    // 65536 = 1<<16 does not fit a sign-extended 16-bit immediate; it must be
    // assembled from its two halves and stay exact.
    assert_eq!(
        run("fn main() { let v = 65536; exit(v >> 16); }").0,
        VmRunResult::Exited(1)
    );
    // 0x18000: the low half is 0x8000 (bit 15 set) and must be zero-extended, not
    // sign-extended, so it does not corrupt the upper word.
    assert_eq!(
        run("fn main() { let v = 98304; exit(v >> 15); }").0,
        VmRunResult::Exited(3)
    );
    // A value whose low half is 0xFFFF round-trips exactly.
    assert_eq!(
        run("fn main() { let v = 65535; exit(v); }").0,
        VmRunResult::Exited(65535)
    );
}

#[test]
fn comparisons_and_if_else() {
    let (result, _) = run("
        fn main() {
            let x = 5;
            if x < 10 {
                exit(1);
            } else {
                exit(0);
            }
        }
        ");
    assert_eq!(result, VmRunResult::Exited(1));
}

/// KOTO-0169 Stage 4: exhaustive truth-table agreement between the *value*
/// templates and the *branch-context* lowering for every comparison operator,
/// on operand pairs that include the two's-complement edges (INT_MIN/INT_MAX
/// are computed at runtime with shifts so the peephole cannot pre-fold them
/// away). Each pair's six comparisons are packed into a bitmask twice — once
/// through value context (`let m = m | (a < b) << k`), once through `if`
/// branches — and both masks must agree with the host-side truth.
#[test]
fn comparison_value_and_branch_contexts_agree_at_edges() {
    // (a, b) as Koto expressions; INT_MIN = 1<<31, INT_MAX = ~INT_MIN.
    let pairs = [
        ("3", "5"),
        ("5", "3"),
        ("7", "7"),
        ("0 - 4", "2"),
        ("1 << 31", "0"),       // INT_MIN vs 0
        ("0", "1 << 31"),       // 0 vs INT_MIN
        ("1 << 31", "1 << 31"), // INT_MIN vs INT_MIN
        ("(1 << 31) + 1", "1"), // near-MIN vs small
    ];
    for (a_src, b_src) in pairs {
        let src = format!(
            "
        fn main() {{
            let a = {a_src};
            let b = {b_src};
            let value_mask = ((a == b) << 0) | ((a != b) << 1) | ((a < b) << 2)
                | ((a > b) << 3) | ((a <= b) << 4) | ((a >= b) << 5);
            let branch_mask = 0;
            if a == b {{ branch_mask = branch_mask | 1; }}
            if a != b {{ branch_mask = branch_mask | 2; }}
            if a < b {{ branch_mask = branch_mask | 4; }}
            if a > b {{ branch_mask = branch_mask | 8; }}
            if a <= b {{ branch_mask = branch_mask | 16; }}
            if a >= b {{ branch_mask = branch_mask | 32; }}
            if value_mask != branch_mask {{ exit(64 + value_mask); }}
            exit(value_mask);
        }}
        "
        );
        // Host-side truth with the templates' documented semantics: the
        // ordered comparisons are sign-of-wrapping-difference (the existing
        // contract), equality is exact.
        let eval = |s: &str| -> i32 {
            match s {
                "2" => 2,
                "3" => 3,
                "5" => 5,
                "7" => 7,
                "0 - 4" => -4,
                "1 << 31" => i32::MIN,
                "0" => 0,
                "(1 << 31) + 1" => i32::MIN + 1,
                "1" => 1,
                other => panic!("unmapped operand {other}"),
            }
        };
        let (a, b) = (eval(a_src), eval(b_src));
        let d = a.wrapping_sub(b);
        let lt = ((d as u32) >> 31) as i32;
        let gt = ((b.wrapping_sub(a) as u32) >> 31) as i32;
        let expected = i32::from(a == b)
            | (i32::from(a != b) << 1)
            | (lt << 2)
            | (gt << 3)
            | ((1 - gt) << 4)
            | ((1 - lt) << 5);
        let (result, _) = run(&src);
        assert_eq!(
            result,
            VmRunResult::Exited(expected),
            "a={a_src} b={b_src} (value/branch masks must agree and match host truth)"
        );
    }
}

/// KOTO-0169 Stage 4: `&&` / `||` / `!` on arbitrary (non-0/1) truthy values,
/// in value and branch contexts, including nested `!` (which flips the branch
/// sense twice). Both operands of `&&`/`||` are always evaluated (no
/// short-circuit) — asserted via a counter side effect.
#[test]
fn logical_templates_normalize_and_never_short_circuit() {
    let (result, _) = run("
        fn main() {
            let evals = 0;
            let t1 = 6;
            let t2 = 0 - 3;
            let z = 0;
            let v = (t1 && t2) | ((t1 && z) << 1) | ((z || t2) << 2)
                | ((z || z) << 3) | ((!z) << 4) | ((!t1) << 5) | ((!(!t2)) << 6);
            let m = 0;
            if t1 && t2 { m = m | 1; }
            if t1 && z { m = m | 2; }
            if z || t2 { m = m | 4; }
            if z || z { m = m | 8; }
            if !z { m = m | 16; }
            if !t1 { m = m | 32; }
            if !(!t2) { m = m | 64; }
            if !(t1 == 6) { m = m | 128; }
            if !(t1 != 6) { m = m | 256; }
            if v != (m & 127) { exit(1000 + v); }
            if m != (16 | 4 | 1 | 64 | 256) { exit(2000 + m); }
            exit(v);
        }
        ");
    // t1&&t2=1, t1&&z=0, z||t2=1, z||z=0, !z=1, !t1=0, !!t2=1
    assert_eq!(result, VmRunResult::Exited(1 | 4 | 16 | 64));
}

#[test]
fn while_loop_sum() {
    let (result, _) = run("
        fn main() {
            let i = 0;
            let sum = 0;
            while i < 5 {
                sum = sum + i;
                i = i + 1;
            }
            exit(sum);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(10)); // 0+1+2+3+4
}

#[test]
fn modulo_and_division() {
    let (result, _) = run("fn main() { exit(17 % 5); }");
    assert_eq!(result, VmRunResult::Exited(2));
}

#[test]
fn inlined_function_call() {
    let (result, _) = run("
        fn add(a: int, b: int) -> int {
            return a + b;
        }
        fn main() {
            exit(add(30, 12));
        }
        ");
    assert_eq!(result, VmRunResult::Exited(42));
}

#[test]
fn early_return_in_inlined_function() {
    let (result, _) = run("
        fn pick(flag: int) -> int {
            if flag != 0 {
                return 1;
            }
            return 2;
        }
        fn main() {
            exit(pick(0));
        }
        ");
    assert_eq!(result, VmRunResult::Exited(2));
}

#[test]
fn draws_string_literal() {
    let (result, host) = run("
        fn main() {
            draw_text(0, 0, \"hi\", 2);
            exit(0);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(0));
    assert_eq!(host.text, [(0, 0, String::from("hi"))]);
}

#[test]
fn emits_debug_map_for_source_locations() {
    let bytecode = compile(
        "debug.koto",
        "fn main() {\n    let x = 1 / 0;\n    exit(x);\n}\n",
    )
    .unwrap();
    let map = koto_core::debug_map(&bytecode).unwrap().unwrap();

    let first = map.lookup_pc(0).unwrap();
    assert_eq!(first.file, "debug.koto");
    assert_eq!(first.line, 2);
    assert_eq!(first.col, 5);
}

#[test]
fn const_data_loads_from_rodata_image() {
    // KOTO-0139: a `data` table is initialized from the const heap image (no runtime
    // bake) and resolves by name as its heap offset, readable from any function.
    let (result, _) = run("
        data tbl = u16[1000, 2000, 65535];
        fn at(i: int) -> int { return heap_get_u16(tbl + i * 2); }
        fn main() {
            exit(at(0) + at(1) + at(2));
        }
        ");
    assert_eq!(result, VmRunResult::Exited(68535)); // 1000 + 2000 + 65535
}

#[test]
fn const_data_u8_and_indexing() {
    // u8 `data` packs one byte per value and supports byte indexing by name.
    let (result, _) = run("
        data bytes = u8[10, 20, 33];
        fn main() {
            exit(bytes[0] + bytes[1] + bytes[2]);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(63));
}

#[test]
fn const_data_emits_no_store_bake() {
    // The whole point of KOTO-0139: the const table lands in rodata, not as runtime
    // STORE16 bake code. The compiled program must carry a rodata segment and no
    // store16 for the table.
    let bytecode = compile(
        "test.koto",
        "
        data tbl = u16[7, 8, 9];
        fn main() { exit(heap_get_u16(tbl)); }
        ",
    )
    .expect("compiles");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).expect("verifies");
    let (start, end) = program.rodata_range().expect("rodata present");
    // tbl sits at the bottom (no mutable buffers here): 7, 8, 9 little-endian u16.
    assert_eq!(&bytecode[start..end], &[7, 0, 8, 0, 9, 0]);
}

#[test]
fn rejects_data_value_out_of_range() {
    let err = compile("t.koto", "data t = u8[256]; fn main() { exit(0); }").unwrap_err();
    assert!(err.message.contains("u8"), "got: {}", err.message);
}

#[test]
fn buffer_store_and_load() {
    let (result, _) = run("
        fn main() {
            buf b[4];
            b[0] = 65;
            b[1] = 66;
            exit(b[0] + b[1]);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(131)); // 65 + 66
}

#[test]
fn typed_heap_accessors_round_trip_unsigned_values() {
    let (result, _, budget) = run_with_budget(
        "
        fn main() {
            heap_set_u8(0, 255);
            heap_set_u16(2, 65535);
            exit(heap_get_u8(0) + heap_get_u16(2));
        }
        ",
    );
    assert_eq!(result, VmRunResult::Exited(65790));
    assert_eq!(budget.heap_bytes_peak, 4);
}

#[test]
fn typed_heap_i16_round_trips_signed_values() {
    let (result, _, budget) = run_with_budget(
        "
        fn main() {
            heap_set_i16(0, -1234);
            heap_set_i16(2, 2345);
            exit(heap_get_i16(0) + heap_get_i16(2));
        }
        ",
    );
    assert_eq!(result, VmRunResult::Exited(1111));
    assert_eq!(budget.heap_bytes_peak, 4);
}

#[test]
fn actor_array_fields_get_and_set() {
    let (result, _, budget) = run_with_budget(
        "
        fn main() {
            let actors = actor_array_new(2);
            actor_set_pos(actors, 1, -10, 20);
            actor_set_vel(actors, 1, 3, -4);
            actor_set_state(actors, 1, 7);
            actor_set_frame(actors, 1, 2);
            actor_set_timer(actors, 1, 300);
            exit(actor_x(actors, 1) + actor_y(actors, 1) + actor_vx(actors, 1)
                + actor_vy(actors, 1) + actor_state(actors, 1)
                + actor_frame(actors, 1) + actor_timer(actors, 1));
        }
        ",
    );
    assert_eq!(result, VmRunResult::Exited(318));
    assert_eq!(budget.heap_bytes_peak, 24);
}

#[test]
fn actor_count_increases_heap_not_user_slots() {
    let one = "
        fn main() {
            let actors = actor_array_new(1);
            actor_set_pos(actors, 0, 1, 2);
            exit(actor_x(actors, 0));
        }
        ";
    let many = "
        fn main() {
            let actors = actor_array_new(16);
            actor_set_pos(actors, 15, 1, 2);
            exit(actor_x(actors, 15));
        }
        ";

    let one_map = slot_map("one.koto", one).expect("one slot map");
    let many_map = slot_map("many.koto", many).expect("many slot map");
    assert_eq!(one_map.user_slots_used, many_map.user_slots_used);

    let (_, _, one_budget) = run_with_budget(one);
    let (_, _, many_budget) = run_with_budget(many);
    assert!(many_budget.heap_bytes_peak > one_budget.heap_bytes_peak);
    assert_eq!(one_budget.heap_bytes_peak, 4);
    assert_eq!(many_budget.heap_bytes_peak, 184);
}

#[test]
fn yield_then_resume_across_frames() {
    let bytecode = compile(
        "test.koto",
        "
        fn main() {
            yield_frame();
            exit(3);
        }
        ",
    )
    .unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    let first = vm
        .execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap();
    assert_eq!(first, VmRunResult::Yielded);
    let second = vm
        .execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap();
    assert_eq!(second, VmRunResult::Exited(3));
}

#[test]
fn aliased_intrinsic_projects_snapshot_value() {
    // `input_held` and `input_pressed` both issue `input_snapshot` but project a
    // different result value; verify the alias assembles and runs.
    let bytecode = compile("test.koto", "fn main() { exit(input_pressed()); }").unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    let input = VmInputSnapshot {
        pressed_bits: 9,
        ..VmInputSnapshot::empty()
    };
    let result = vm
        .execute_frame(&bytecode, &program, &mut host, input, 100_000, &mut heap)
        .unwrap();
    assert_eq!(result, VmRunResult::Exited(9));
}

// ---- SDK prelude (KOTO-0047) ----

#[test]
fn sdk_wrappers_emit_expected_host_calls() {
    let asm = compile_to_asm(
        "sdk.koto",
        "
        fn main() {
            buf path[8];
            draw_rect(0, 0, 320, 320, 0);
            draw_text(0, 0, \"hi\", 2);
            let h = file_open(path, 8, MODE_READ);
            file_close(h);
            ime_feed_key(IME_COMMIT, 0);
            ime_convert();
            edit_move(DIR_LEFT);
            edit_delete(DELETE_BACKSPACE);
            let cw = edit_cell_width();
            let ch = edit_cell_height();
            let slen = edit_cursor_status(path, 8);
            let lines = edit_total_lines();
            yield_frame();
            exit(0);
        }
        ",
    )
    .unwrap();
    for needle in [
        "host_call draw_rect",
        "host_call draw_text",
        "host_call file_open",
        "host_call file_close",
        "host_call ime_feed_key",
        "host_call ime_convert",
        "host_call edit_move",
        "host_call edit_delete",
        "host_call edit_view_metrics",
        "host_call edit_cursor_status",
        "host_call edit_total_lines",
        "host_call yield_frame",
        "host_call exit",
    ] {
        assert!(asm.contains(needle), "missing `{needle}` in:\n{asm}");
    }
}

#[test]
fn vault_sdk_wrappers_emit_expected_host_calls() {
    // KOTO-0248: `vault_handle` resolves the opaque handle and
    // `fetch_start_authenticated` starts the credentialed GET. The
    // `VAULT_SERVICE_FETCH` selector constant must resolve at compile time.
    let asm = compile_to_asm(
        "vault.koto",
        "
        fn main() {
            buf url[16];
            let handle = vault_handle(VAULT_SERVICE_FETCH, url, 16);
            let req = fetch_start_authenticated(url, 16, handle);
            exit(req);
        }
        ",
    )
    .unwrap();
    for needle in [
        "host_call vault_handle",
        "host_call fetch_start_authenticated",
    ] {
        assert!(asm.contains(needle), "missing `{needle}` in:\n{asm}");
    }
}

#[test]
fn vault_handle_rejects_wrong_argument_count() {
    // The host call takes three arguments (service, url_ptr, url_len).
    let err = compile_to_asm(
        "bad_vault.koto",
        "
        fn main() {
            buf url[16];
            let handle = vault_handle(VAULT_SERVICE_FETCH, url);
            exit(handle);
        }
        ",
    )
    .unwrap_err();
    assert!(
        format!("{err:?}").contains("vault_handle"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn mqtt_sdk_wrappers_emit_expected_host_calls() {
    // KOTO-0249: the app names a manifest broker/topic by index, polls, peeks
    // the message lengths (idempotent), reads into its own buffers, and reports
    // the overflow count. The poll-state and read-kind selector constants must
    // resolve at compile time.
    let asm = compile_to_asm(
        "mqtt.koto",
        "
        fn main() {
            buf topic[128];
            buf payload[192];
            let s = mqtt_connect(0);
            mqtt_subscribe(s, 0);
            let state = mqtt_poll(s);
            if state == MQTT_MESSAGE {
                let tlen = mqtt_peek_topic_len(s);
                let plen = mqtt_peek_payload_len(s);
                let kind = mqtt_read(s, topic, 128, payload, 192);
                if kind == MQTT_READ_RETAINED {
                    let dropped = mqtt_dropped(s);
                    exit(dropped);
                }
            }
            mqtt_disconnect(s);
            exit(0);
        }
        ",
    )
    .unwrap();
    for needle in [
        "host_call mqtt_connect",
        "host_call mqtt_subscribe",
        "host_call mqtt_poll",
        "host_call mqtt_peek",
        "host_call mqtt_read",
        "host_call mqtt_disconnect",
        "host_call mqtt_dropped",
    ] {
        assert!(asm.contains(needle), "missing `{needle}` in:\n{asm}");
    }
}

#[test]
fn mqtt_read_rejects_wrong_argument_count() {
    // `mqtt_read` takes five arguments (session, topic_ptr, topic_max,
    // payload_ptr, payload_max).
    let err = compile_to_asm(
        "bad_mqtt.koto",
        "
        fn main() {
            buf topic[16];
            let kind = mqtt_read(0, topic, 16);
            exit(kind);
        }
        ",
    )
    .unwrap_err();
    assert!(
        format!("{err:?}").contains("mqtt_read"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn ui_sdk_lifecycle_wrappers_emit_canonical_host_calls() {
    let asm = compile_to_asm(
        "ui_sdk.koto",
        "
        fn main() {
            buf capabilities[64];
            buf mount[128];
            buf update[64];
            buf event[64];
            let c = ui_capabilities(capabilities, UI_CAPABILITIES_BYTES);
            ui_mount(mount, 128);
            ui_update(update, 64);
            ui_present();
            let e = ui_poll_event(event, 64);
            ui_reset();
            exit(c + e);
        }
        ",
    )
    .unwrap();
    for call in [
        "host_call ui_capabilities",
        "host_call ui_mount",
        "host_call ui_update",
        "host_call ui_present",
        "host_call ui_poll_event",
        "host_call ui_reset",
    ] {
        assert!(asm.contains(call), "missing {call} in:\n{asm}");
    }
    assert_eq!(
        UI_SDK_FUNCTIONS
            .iter()
            .map(|function| function.name)
            .collect::<Vec<_>>(),
        [
            "ui_capabilities",
            "ui_mount",
            "ui_update",
            "ui_present",
            "ui_poll_event",
            "ui_reset"
        ]
    );
}

#[test]
fn ui_sdk_lifecycle_stack_results_match_success_and_older_host_failure() {
    let source = "
        fn main() {
            buf capabilities[64];
            buf packet[64];
            buf event[32];
            let result = ui_capabilities(capabilities, 64);
            result = result + ui_mount(packet, 64);
            result = result + ui_update(packet, 64);
            result = result + ui_present();
            result = result + ui_poll_event(event, 32);
            result = result + ui_reset();
            exit(result);
        }
    ";
    let bytecode = compile("ui_lifecycle_results.koto", source).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];

    let mut success_vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut success_host = UiSuccessHost;
    assert_eq!(
        success_vm
            .execute_frame(
                &bytecode,
                &program,
                &mut success_host,
                VmInputSnapshot::empty(),
                100_000,
                &mut heap,
            )
            .unwrap(),
        VmRunResult::Exited(64)
    );

    heap.fill(0);
    let mut older_vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut older_host = OlderHost;
    assert_eq!(
        older_vm
            .execute_frame(
                &bytecode,
                &program,
                &mut older_host,
                VmInputSnapshot::empty(),
                100_000,
                &mut heap,
            )
            .unwrap(),
        // Value-producing wrappers use the SDK's -1 failure sentinel; the four
        // status wrappers preserve the host's -UNSUPPORTED (-5) status.
        VmRunResult::Exited(-22)
    );
}

#[test]
fn ui_sdk_constants_are_sourced_and_folded() {
    let asm = compile_to_asm(
        "ui_constants.koto",
        "
        fn main() {
            let node = UI_NODE_DIALOG;
            let flags = UI_FLAG_VISIBLE + UI_FLAG_ENABLED + UI_FLAG_LTR + UI_FLAG_ELLIPSIS;
            let response = UI_RESPONSE_LOCALE_CHANGED;
            let capacity = UI_MAX_NODES;
            let error = UI_ERROR_NO_MEMORY;
            draw_rect(node, flags, response, capacity, error);
            exit(UI_ABI_MAJOR + UI_ABI_MINOR);
        }
        ",
    )
    .unwrap();
    for value in [7, 23, 10, 32, 8, 1] {
        assert!(
            asm.contains(&format!("push_i16 {value}")),
            "constant {value} not folded in:\n{asm}"
        );
    }
}

#[test]
fn ui_sdk_capacity_helpers_fold_in_consts_and_size_buffers() {
    let source = r#"
        include "koto_ui.koto";
        const MOUNT_RECORDS = 3;
        const MOUNT_DATA_CAPACITY = 17;
        const MOUNT_BYTES = ui_mount_capacity(MOUNT_RECORDS, MOUNT_DATA_CAPACITY);
        const MOUNT_BYTES_ALIAS = MOUNT_BYTES;
        const UPDATE_RECORDS = 1;
        const UPDATE_DATA_CAPACITY = 8;
        const UPDATE_BYTES = ui_update_capacity(UPDATE_RECORDS, UPDATE_DATA_CAPACITY);
        fn main() {
            buf mount[MOUNT_BYTES_ALIAS];
            buf update[UPDATE_BYTES];
            if MOUNT_BYTES != 201 || UPDATE_BYTES != 72 { exit(10); }
            if ui_mount_capacity(3, 17) != 201 || ui_update_capacity(1, 8) != 72 {
                exit(11);
            }
            if ui_mount_capacity(0, 0) != -2 || ui_mount_capacity(33, 0) != -2 ||
               ui_update_capacity(17, 0) != -2 || ui_update_capacity(1, 1985) != -2 {
                exit(12);
            }
            heap_set_u8(mount + 200, 1);
            heap_set_u8(update + 71, 1);
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "ui_capacity_helpers.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("capacity helpers should fold and const-sized buffers should compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    assert_eq!(program.header().max_heap_bytes, 273);
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );

    for invalid in [
        "const BAD = ui_mount_capacity(0, 0); fn main() {}",
        "const BAD = ui_mount_capacity(33, 0); fn main() {}",
        "const BAD = ui_update_capacity(17, 0); fn main() {}",
        "const BAD = ui_update_capacity(1, 1985); fn main() {}",
    ] {
        let error = compile("invalid_ui_capacity.koto", invalid).unwrap_err();
        assert!(error
            .message
            .contains("exceed the KotoUI v1 packet capacities"));
    }
}

#[test]
fn ui_capacity_helpers_size_buffers_at_call_sites_and_len_folds_to_capacity() {
    // KOTO-0233: the transaction-specific sizing facts live on the declaration
    // and `len(buf)` recovers the compile-time capacity at the `begin` call.
    let source = "
        const EXTRA = 8;
        fn main() {
            buf mount[ui_mount_capacity(3, 17)];
            buf update[ui_update_capacity(1, EXTRA)];
            if len(mount) != 201 || len(update) != 72 { exit(10); }
            heap_set_u8(mount + len(mount) - 1, 1);
            heap_set_u8(update + len(update) - 1, 1);
            if heap_get_u8(mount + 200) != 1 || heap_get_u8(update + 71) != 1 { exit(11); }
            exit(0);
        }
    ";
    let bytecode = compile("ui_capacity_call_sites.koto", source).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    assert_eq!(program.header().max_heap_bytes, 273);
    assert_eq!(run(source).0, VmRunResult::Exited(0));

    // Helper-sized declarations keep the KOTO-0232 boundary diagnostics.
    for invalid in [
        "fn main() { buf packet[ui_mount_capacity(0, 0)]; }",
        "fn main() { buf packet[ui_mount_capacity(33, 0)]; }",
        "fn main() { buf packet[ui_update_capacity(17, 0)]; }",
        "fn main() { buf packet[ui_update_capacity(1, 1985)]; }",
    ] {
        let error = compile("invalid_buf_capacity.koto", invalid).expect_err(invalid);
        assert!(error
            .message
            .contains("exceed the KotoUI v1 packet capacities"));
    }
}

#[test]
fn len_lowers_to_an_integer_constant_without_runtime_reads_or_slots() {
    let asm = compile_to_asm(
        "len_zero_cost.koto",
        "fn main() { buf packet[ui_update_capacity(1, 8)]; exit(len(packet)); }",
    )
    .unwrap();
    assert!(asm.contains("push_i16 72"), "capacity not folded:\n{asm}");
    for op in ["load8", "load16", "load32", "load_local", "store_local"] {
        assert!(!asm.contains(op), "`len` emitted runtime {op}:\n{asm}");
    }
}

#[test]
fn len_diagnostics_cover_invalid_unknown_out_of_scope_and_shadowed_operands() {
    for (source, expected) in [
        (
            "fn main() { buf b[8]; exit(len(b, b)); }",
            "`len` takes 1 argument(s), got 2",
        ),
        (
            "fn main() { exit(len(3)); }",
            "`len` operand must name a `buf`",
        ),
        (
            "fn main() { buf b[8]; exit(len(b + 1)); }",
            "`len` operand must name a `buf`",
        ),
        (
            "fn main() { let n = 4; exit(len(n)); }",
            "`n` is a local value here, not a `buf`",
        ),
        // A later `let` shadows the buffer for the rest of its block.
        (
            "fn main() { buf b[8]; let b = 1; exit(len(b)); }",
            "`b` is a local value here, not a `buf`",
        ),
        // Use before the declaration is out of scope, not a forward reference.
        (
            "fn main() { let n = len(b); buf b[8]; exit(n); }",
            "buf `b` is not in scope yet",
        ),
        // Buffers are function-local; another function's buffer is unknown.
        (
            "fn peek() -> int { return len(b); } fn main() { buf b[8]; exit(peek()); }",
            "undefined name `b`",
        ),
        (
            "fn main() { exit(len(missing)); }",
            "undefined name `missing`",
        ),
        (
            "const CAP = 8; fn main() { exit(len(CAP)); }",
            "`CAP` is not a `buf`",
        ),
        (
            "data table = u8[1, 2]; fn main() { exit(len(table)); }",
            "`table` is not a `buf`",
        ),
    ] {
        let error = compile("len_error.koto", source).expect_err(source);
        assert!(error.message.contains(expected), "for `{source}`: {error}");
    }

    // Block scoping: a shadowing `let` ends with its block, after which the
    // buffer capacity is visible again; a sibling block never saw the shadow.
    let scoped = "
        fn main() {
            buf b[8];
            let total = 0;
            if true { let b = 1; total = total + b; }
            if true { total = total + len(b); }
            exit(total + len(b));
        }
    ";
    assert_eq!(run(scoped).0, VmRunResult::Exited(17));
}

#[test]
fn ui_sdk_mount_header_builder_encodes_exact_bytes_and_invalid_finish_is_atomic() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf packet[200];
            let status = ui_mount_begin(packet, 200, 2, 136, 1, 2);
            if status != 0 { exit(status); }
            status = ui_mount_add_panel(packet, 200, 0, 1, -1, 3, 0, 0, 320, 320, 0, 0, 0, 0, 0);
            if status != 0 { exit(88); }
            status = ui_mount_add_button(packet, 200, 1, 2, 1, 3, 8, 8, 80, 20, 0, 0, 0, 0);
            if status != 0 { exit(89); }
            status = ui_mount_finish(packet, 200, 201, 136);
            if status != -2 { exit(90); }
            if heap_get_u16(packet + 8) != 0 { exit(91); }
            status = ui_mount_finish(packet, 200, 142, 136);
            if status != 0 { exit(status); }
            ui_mount(packet, 142);
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_builder_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("SDK builder source compiles");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
    let packet = &host.ui_mounts[0];
    assert_eq!(&packet[..4], b"KUI1");
    assert_eq!(u16::from_le_bytes(packet[4..6].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(packet[8..12].try_into().unwrap()), 142);
    assert_eq!(u16::from_le_bytes(packet[12..14].try_into().unwrap()), 2);
    assert_eq!(u16::from_le_bytes(packet[14..16].try_into().unwrap()), 48);
    assert_eq!(u32::from_le_bytes(packet[16..20].try_into().unwrap()), 40);
    assert_eq!(u32::from_le_bytes(packet[20..24].try_into().unwrap()), 136);
    assert_eq!(u32::from_le_bytes(packet[24..28].try_into().unwrap()), 6);
    assert_eq!(u16::from_le_bytes(packet[28..30].try_into().unwrap()), 1);
    assert_eq!(u16::from_le_bytes(packet[30..32].try_into().unwrap()), 2);
    assert!(packet[32..40].iter().all(|byte| *byte == 0));
    assert_eq!(packet[40 + 4], 6);
    assert_eq!(packet[88 + 4], 2);
    assert!(packet[136..].iter().all(|byte| *byte == 0));
}

#[test]
fn ui_sdk_stateful_builders_are_sticky_reusable_independent_and_wire_identical() {
    let source = r#"
        include "koto_ui.koto";
        data japanese = u8[227, 129, 130];

        static mount_a: UiMountBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        static mount_b: UiMountBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        static update_a: UiUpdateBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };

        fn main() {
            buf expected[139];
            if ui_mount_begin(expected, 139, 2, 136, 1, 2) != 0 { exit(10); }
            if ui_mount_add_panel(expected, 139, 0, 1, -1, 3,
                0, 0, 320, 320, 0, 0, 0, 0, 0) != 0 { exit(11); }
            if ui_mount_add_label(expected, 139, 1, 2, 1, 3,
                8, 8, 80, 16, 0, 3, 3, UI_ALIGN_START) != 0 { exit(12); }
            if ui_packet_copy(expected, 139, 136, japanese, 3) != 0 { exit(13); }
            if ui_mount_finish(expected, 139, 139, 136) != 0 { exit(14); }

            buf actual[139];
            buf other[136];
            if mount_a.begin(actual, 139, 2, 1, 2) != 0 { exit(20); }
            if mount_b.begin(other, 136, 2, 1, -1) != 0 { exit(21); }
            mount_a.panel(1, -1, 3, 0, 0, 320, 320, japanese, 0, 0, 0, 0);
            mount_b.panel(1, -1, 3, 0, 0, 320, 320, japanese, 0, 0, 0, 0);
            mount_a.label(2, 1, 3, 8, 8, 80, 16, japanese, 3, 3, UI_ALIGN_START);
            mount_b.label(2, 1, 3, 8, 8, 80, 16, japanese, 0, 0, UI_ALIGN_START);
            if mount_b.finish() != 136 { exit(22); }
            if mount_a.finish() != 139 { exit(23); }
            let i = 0;
            while i < 139 {
                if heap_get_u8(expected + i) != heap_get_u8(actual + i) { exit(24); }
                i = i + 1;
            }

            // A failed operation is sticky, does not write its record, and a
            // completed failed transaction can be explicitly reused.
            if mount_a.begin(actual, 139, 2, 1, 2) != 0 { exit(30); }
            mount_a.panel(1, -1, 3, 0, 0, 320, 320, japanese, 0, 0, 0, 0);
            heap_set_u8(actual + 88, 165);
            if mount_a.label(2, 1, 3, 8, 8, 80, 16,
                japanese, 3, 3, 99) != -2 { exit(31); }
            if heap_get_u8(actual + 88) != 165 { exit(32); }
            if mount_a.label(2, 1, 3, 8, 8, 80, 16,
                japanese, 3, 3, UI_ALIGN_START) != -2 { exit(33); }
            if mount_a.finish() != -2 || heap_get_u16(actual + 8) != 0 { exit(34); }
            if mount_a.begin(actual, 139, 2, 1, 2) != 0 { exit(35); }

            buf update[358];
            if update_a.begin(update, 358, 10) != 0 { exit(40); }
            update_a.text(1, japanese, 3);
            update_a.enabled(2, true);
            update_a.visible(3, true);
            update_a.checked(4, true);
            update_a.selection(5, 0);
            update_a.text_value(6, japanese, 3, 3);
            update_a.bounds(7, 10, 20, 100, 30);
            update_a.list_rows(8, japanese, 0, 0, -1);
            update_a.dialog_open(9, true);
            update_a.request_focus(10);
            if update_a.finish() != 358 { exit(41); }
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "ui_stateful_builder_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("stateful builders compile across an included SDK source");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            5_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn ui_sdk_text_resources_and_list_rows_are_bounded_atomic_and_typed() {
    let source = r#"
        include "koto_ui.koto";
        data mixed = u8[97, 10, 10, 227, 129, 130, 13, 10, 98];
        data second = u8[120, 10];
        data malformed = u8[192];
        data bare_cr = u8[97, 13, 98];
        static text_a: TextResource = {
            storage: 0, capacity: 0, line_capacity: 0, line_count: 0,
            payload_offset: 0, payload_len: 0, status: 0, complete: false,
        };
        static text_b: TextResource = {
            storage: 0, capacity: 0, line_capacity: 0, line_count: 0,
            payload_offset: 0, payload_len: 0, status: 0, complete: false,
        };
        static rows: UiListRowsBuilder = {
            blob: 0, capacity: 0, row_capacity: 0, row_count: 0,
            label_cursor: 0, status: 0, active: false, complete: false,
        };
        static mount: UiMountBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        static update: UiUpdateBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        fn main() {
            buf parsed_a[32]; buf parsed_b[5];
            if text_a.count() != -2 || text_a.line_ptr(0) != -2 { exit(9); }
            if text_a.parse(mixed, 9, parsed_a, 32, 4) != 4 { exit(10); }
            if text_b.parse(second, 2, parsed_b, 5, 1) != 1 { exit(11); }
            if text_a.count() != 4 || text_b.count() != 1 { exit(12); }
            if text_a.line_len(0) != 1 || text_a.line_len(1) != 0 ||
               text_a.line_len(2) != 3 || text_a.line_len(3) != 1 { exit(13); }
            if heap_get_u8(text_a.line_ptr(0)) != 97 ||
               heap_get_u8(text_a.line_ptr(2)) != 227 ||
               heap_get_u8(text_b.line_ptr(0)) != 120 { exit(14); }

            // Invalid parse/access is sticky, does not mutate output, and the
            // next parse explicitly resets the record for reuse.
            heap_set_u8(parsed_a, 165);
            if text_a.parse(malformed, 1, parsed_a, 32, 4) != -2 { exit(20); }
            if heap_get_u8(parsed_a) != 165 || text_a.line_ptr(0) != -2 { exit(21); }
            if text_a.parse(mixed, 9, parsed_a, 32, 4) != 4 { exit(22); }
            if text_a.line_len(4) != -2 || text_a.line_ptr(0) != -2 { exit(23); }
            if text_a.parse(mixed, 9, parsed_a, 32, 4) != 4 { exit(24); }
            heap_set_u8(parsed_b, 91);
            if text_b.parse(bare_cr, 3, parsed_b, 5, 1) != -2 ||
               heap_get_u8(parsed_b) != 91 { exit(25); }
            if text_b.parse(mixed, 9, parsed_b, 20, 4) != -2 ||
               text_b.parse(mixed, 9, parsed_b, 32, 3) != -2 ||
               text_b.parse(mixed, 65536, parsed_b, 5, 1) != -2 { exit(26); }
            if text_b.parse(mixed, 0, parsed_b, 0, 0) != 0 || text_b.count() != 0 {
                exit(27);
            }
            buf blob[32];
            if rows.begin(blob, 28, 2) != 0 { exit(30); }
            if rows.resource_row(true, text_a, 0, -7) != 0 { exit(31); }
            if rows.resource_row(false, text_a, 2, 42) != 0 { exit(32); }
            if rows.finish() != 28 || rows.row_count != 2 { exit(33); }
            if heap_get_u16(blob) != 24 || heap_get_u16(blob + 2) != 1 ||
               heap_get_u16(blob + 4) != 1 || ui_sdk_get_u32(blob, 8) != -7 ||
               heap_get_u16(blob + 12) != 25 || heap_get_u16(blob + 14) != 3 ||
               heap_get_u16(blob + 16) != 0 || ui_sdk_get_u32(blob, 20) != 42 ||
               heap_get_u8(blob + 24) != 97 || heap_get_u8(blob + 25) != 227 { exit(34); }
            if rows.row(true, mixed, 1, 0) != -2 { exit(35); }

            heap_set_u8(blob, 89);
            if rows.begin(blob, 28, 2) != 0 || rows.begin(blob, 28, 2) != -2 ||
               rows.row(true, mixed, 1, 1) != -2 || heap_get_u8(blob) != 89 ||
               rows.finish() != -2 { exit(36); }
            if rows.begin(blob, 28, 2) != 0 || rows.row(true, mixed, 1, 1) != 0 ||
               rows.row(true, mixed, 1, 2) != 0 || rows.finish() != 26 { exit(37); }
            if rows.begin(blob, 24, 2) != 0 || rows.row(true, mixed, 1, 0) != -2 ||
               rows.finish() != -2 { exit(38); }
            if rows.begin(blob, 12, 1) != 0 || rows.row(true, mixed, 0, 0) != 0 ||
               rows.row(true, mixed, 0, 1) != -2 || rows.finish() != -2 { exit(39); }

            buf max_blob[384];
            if rows.begin(max_blob, 384, UI_MAX_LIST_ROWS) != 0 { exit(64); }
            let ri = 0;
            while ri < UI_MAX_LIST_ROWS {
                if rows.row(true, mixed, 0, ri) != 0 { exit(65); }
                ri = ri + 1;
            }
            if rows.finish() != 384 { exit(66); }
            // Restore the representative two-row blob for typed consumers.
            if rows.begin(blob, 28, 2) != 0 ||
               rows.resource_row(true, text_a, 0, -7) != 0 ||
               rows.resource_row(false, text_a, 2, 42) != 0 ||
               rows.finish() != 28 { exit(67); }

            // Typed mount consumes the sealed count/length and stays byte
            // identical to the low-level v1 representation.
            buf packet[212];
            if mount.begin(packet, 212, 2, 1, -1) != 0 { exit(40); }
            mount.panel(1, -1, 3, 0, 0, 320, 320, mixed, 0, 0, 0, 0);
            mount.list_builder(2, 1, 3, 0, 0, 100, 40,
                rows, 28, -1, 20, true);
            if mount.finish() != 164 { exit(41); }
            if heap_get_u16(packet + 12) != 2 ||
               ui_sdk_get_u32(packet, 40 + 48 + 32) != 2 { exit(42); }

            // The packet finalizer, not the row builder, rejects selection of
            // a disabled row. The typed update path must preserve that check.
            buf update_packet[92];
            if update.begin(update_packet, 92, 1) != 0 { exit(50); }
            if update.list_rows_builder(2, rows, 1) != 0 { exit(51); }
            if update.finish() != -2 { exit(52); }

            // Failed row append is atomic/sticky, then begin explicitly reuses.
            heap_set_u8(blob, 90);
            if rows.begin(blob, 28, 2) != 0 { exit(60); }
            if rows.row(true, malformed, 1, 0) != -2 { exit(61); }
            if heap_get_u8(blob) != 90 || rows.row(true, mixed, 1, 0) != -2 ||
               rows.finish() != -2 { exit(62); }
            if rows.begin(blob, 0, 0) != 0 || rows.finish() != 0 { exit(63); }
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "ui_resources_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("text resources and typed List rows compile across an include");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            5_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn ui_sdk_submit_and_resource_text_are_atomic_sticky_exact_and_present_free() {
    let source = r#"
        include "koto_ui.koto";
        data mixed = u8[97, 10, 10, 227, 129, 130];
        data malformed = u8[192];
        static text: TextResource = {
            storage: 0, capacity: 0, line_capacity: 0, line_count: 0,
            payload_offset: 0, payload_len: 0, status: 0, complete: false,
        };
        static update: UiUpdateBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        fn main() {
            buf parsed[20];
            if text.parse(mixed, 6, parsed, 20, 3) != 3 { exit(10); }
            buf expected[132]; buf actual[132];
            if update.begin(expected, 132, 3) != 0 { exit(11); }
            update.text(1, text.line_ptr(0), text.line_len(0));
            update.text(2, text.line_ptr(1), text.line_len(1));
            update.text(3, text.line_ptr(2), text.line_len(2));
            if update.finish() != 132 { exit(12); }
            if update.begin(actual, 132, 3) != 0 { exit(13); }
            update.text_resource(1, text, 0);
            update.text_resource(2, text, 1);
            update.text_resource(3, text, 2);
            if update.submit() != 0 { exit(14); }
            let i = 0;
            while i < 132 {
                if heap_get_u8(expected + i) != heap_get_u8(actual + i) { exit(15); }
                i = i + 1;
            }
            if update.submit() != -2 { exit(16); }

            // Active re-entry and property/resource failures are sticky and
            // leave the next record byte untouched before submission.
            if update.begin(actual, 132, 3) != 0 || update.begin(actual, 132, 3) != -2 ||
               update.text(4, mixed, 1) != -2 || update.submit() != -2 { exit(20); }
            if update.begin(actual, 64, 1) != 0 { exit(21); }
            heap_set_u8(actual + 32, 165);
            if update.text_resource(1, text, 3) != -2 ||
               heap_get_u8(actual + 32) != 165 || update.submit() != -2 { exit(22); }
            if update.begin(actual, 64, 1) != 0 { exit(23); }
            text.payload_offset = 20;
            if update.text_resource(1, text, 0) != -2 || update.submit() != -2 { exit(26); }
            if text.parse(mixed, 6, parsed, 20, 3) != 3 { exit(27); }
            if update.begin(actual, 64, 1) != 0 { exit(28); }
            update.data_offset = 64;
            if update.text_resource(1, text, 0) != -2 || update.submit() != -2 { exit(29); }
            if update.begin(actual, 64, 1) != 0 { exit(23); }
            update.text(1, malformed, 1);
            if update.submit() != -2 { exit(24); }
            if update.begin(actual, 63, 1) != -2 || update.submit() != -2 { exit(25); }

            // The maximum KUP1 data payload fits exactly and is reusable after
            // each failed transaction above.
            buf raw[1984]; buf max_parsed[1988]; buf max_packet[2048];
            i = 0; while i < 1984 { heap_set_u8(raw + i, 120); i = i + 1; }
            if text.parse(raw, 1984, max_parsed, 1988, 1) != 1 { exit(30); }
            if update.begin(max_packet, 2048, 1) != 0 ||
               update.text_resource(9, text, 0) != 0 || update.submit() != 0 { exit(31); }
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "ui_submit_resource_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("submit and TextResource bridge compile through the split SDK");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            10_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
    assert_eq!(host.ui_updates.len(), 2);
    assert_eq!(host.ui_updates[0].len(), 132);
    assert_eq!(host.ui_updates[1].len(), 2048);
    assert_eq!(host.ui_presents, 0);

    let rejected = r#"
        include "koto_ui.koto";
        static update: UiUpdateBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        fn main() {
            buf packet[64];
            if update.begin(packet, 64, 1) != 0 { exit(40); }
            update.enabled(1, true);
            if update.submit() != -UI_ERROR_NO_MEMORY { exit(41); }
            if update.submit() != -2 { exit(42); }
            exit(0);
        }
    "#;
    let rejected_bytecode = compile_with_loader(
        "ui_submit_rejected_test.koto",
        rejected,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .unwrap();
    let rejected_program =
        verify_kbc(&rejected_bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut rejected_vm = BytecodeVm::<16, 4>::new(&rejected_program).unwrap();
    let mut rejected_heap = vec![0u8; rejected_program.header().max_heap_bytes as usize];
    let mut rejected_host = CaptureHost {
        reject_ui_update: true,
        ..CaptureHost::default()
    };
    assert_eq!(
        rejected_vm
            .execute_frame(
                &rejected_bytecode,
                &rejected_program,
                &mut rejected_host,
                VmInputSnapshot::empty(),
                1_000_000,
                &mut rejected_heap,
            )
            .unwrap(),
        VmRunResult::Exited(0)
    );
    assert_eq!(rejected_host.ui_updates.len(), 1);
    assert_eq!(rejected_host.ui_presents, 0);
}

#[test]
fn ui_sdk_stateful_counter_example_mounts_and_yields_within_runtime_budgets() {
    const COUNTER: &str = include_str!("../../../sdk/examples/koto_ui_counter.koto");
    let bytecode = compile_with_loader(
        "sdk/examples/koto_ui_counter.koto",
        COUNTER,
        CodegenOptions::default(),
        &mut standard_sdk_loader(),
    )
    .expect("the checked-in stateful counter compiles");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            60_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(host.ui_mounts.len(), 1);
    assert_eq!(host.ui_mounts[0].len(), 201);
    assert!(vm.budget().frame_fuel_peak < 60_000);
    assert!(vm.stats().host_calls <= 6);
}

#[test]
fn ui_sdk_stateful_builders_cover_full_and_overflow_cursor_boundaries() {
    let source = r#"
        include "koto_ui.koto";
        data text = u8[97, 98, 99];
        static mount: UiMountBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        static update: UiUpdateBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        fn main() {
            buf full_mount[1576];
            if mount.begin(full_mount, 1576, 32, 1, -1) != 0 { exit(10); }
            mount.panel(1, -1, 3, 0, 0, 320, 320, text, 0, 0, 0, 0);
            mount.checkbox(2, 1, 3, 0, 0, 1, 1, text, 0, 0, false);
            let i = 2;
            while i < 32 {
                mount.label(i + 1, 1, 3, 0, 0, 1, 1, text, 0, 0, UI_ALIGN_START);
                i = i + 1;
            }
            if mount.finish() != 1576 { exit(11); }

            buf full_update[544];
            if update.begin(full_update, 544, 16) != 0 { exit(20); }
            i = 0;
            while i < 16 {
                update.visible(i + 1, true);
                i = i + 1;
            }
            if update.finish() != 544 { exit(21); }

            buf short_mount[139];
            if mount.begin(short_mount, 139, 2, 1, -1) != 0 { exit(30); }
            mount.panel(1, -1, 3, 0, 0, 320, 320, text, 0, 0, 0, 0);
            if mount.label(2, 1, 3, 0, 0, 1, 1,
                text, 3, 4, UI_ALIGN_START) != -2 { exit(31); }
            if heap_get_u16(short_mount + 88) != 0 { exit(32); }
            if mount.finish() != -2 || heap_get_u16(short_mount + 8) != 0 { exit(33); }

            buf short_update[67];
            heap_set_u8(short_update, 165);
            if update.begin(short_update, 67, 17) != -2 { exit(40); }
            if heap_get_u8(short_update) != 165 { exit(41); }
            if update.begin(short_update, 66, 1) != 0 { exit(42); }
            if update.text(1, text, 3) != -2 { exit(43); }
            if heap_get_u16(short_update + 32) != 0 { exit(44); }
            if update.finish() != -2 || heap_get_u16(short_update + 8) != 0 { exit(45); }
            if update.begin(short_update, 67, 1) != 0 { exit(46); }
            update.text(1, text, 3);
            if update.finish() != 67 { exit(47); }
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "ui_stateful_boundaries.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("stateful full-capacity and overflow boundaries compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            5_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn ui_sdk_mount_finalizer_rejects_cross_record_and_utf8_errors_atomically() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf packet[235];
            let status = ui_mount_begin(packet, 235, 4, 232, 1, 4);
            if status != 0 { exit(70); }
            status = ui_mount_add_panel(packet, 235, 0, 1, -1, 3, 0, 0, 320, 320, 0, 0, 0, 0, 0);
            if status != 0 { exit(71); }
            status = ui_mount_add_dialog(packet, 235, 1, 2, 1, 3, 20, 20, 200, 200, 0, 0, 0, 8, 16, 3, -1, 0);
            if status != 0 { exit(72); }
            status = ui_mount_add_button(packet, 235, 2, 3, 2, 3, 30, 170, 80, 20, 0, 0, 0, 1);
            if status != 0 { exit(73); }
            status = ui_mount_add_text_field(packet, 235, 3, 4, 1, 3, 10, 240, 120, 20, 0, 0, 0, 0, 3, 3, 3, 1);
            if status != 0 { exit(74); }
            heap_set_u8(packet + 232, 227);
            heap_set_u8(packet + 233, 129);
            heap_set_u8(packet + 234, 130);

            heap_set_u16(packet + 184, 3);
            if ui_mount_finish(packet, 235, 235, 232) != -2 { exit(75); }
            if heap_get_u16(packet + 8) != 0 { exit(76); }
            heap_set_u16(packet + 184, 4);

            heap_set_u16(packet + 186, 99);
            if ui_mount_finish(packet, 235, 235, 232) != -2 { exit(77); }
            heap_set_u16(packet + 186, 1);

            heap_set_u8(packet + 233, 255);
            if ui_mount_finish(packet, 235, 235, 232) != -2 { exit(78); }
            heap_set_u8(packet + 233, 129);

            heap_set_u16(packet + 184 + 30, 257);
            if ui_mount_finish(packet, 235, 235, 232) != -2 { exit(79); }
            heap_set_u16(packet + 184 + 30, 3);

            ui_sdk_put_u32(packet, 88 + 40, 99);
            if ui_mount_finish(packet, 235, 235, 232) != -2 { exit(80); }
            ui_sdk_put_u32(packet, 88 + 40, 3);

            if ui_mount_finish(packet, 235, 235, 232) != 0 { exit(81); }
            ui_mount(packet, 235);
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_mount_finalizer_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("cross-record finalizer compiles within the VM local bound");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            2_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
    assert_eq!(host.ui_mounts.len(), 1);
    assert_eq!(&host.ui_mounts[0][232..], &[0xe3, 0x81, 0x82]);
}

#[test]
fn ui_sdk_named_node_builders_encode_all_v1_kinds() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf packet[400];
            let status = ui_mount_begin(packet, 400, 7, 376, 1, 3);
            if status != 0 { exit(status); }
            status = ui_mount_add_label(packet, 400, 1, 2, 1, 3, 8, 8, 80, 16, 0, 0, 0, 9);
            if status != -2 { exit(81); }
            if heap_get_u16(packet + 88) != 0 { exit(82); }
            status = ui_mount_add_panel(packet, 400, 0, 1, -1, 3, 0, 0, 320, 320, 0, 0, 0, 4, 12);
            if status != 0 { exit(83); }
            status = ui_mount_add_label(packet, 400, 1, 2, 1, 3, 8, 8, 80, 16, 0, 0, 0, UI_ALIGN_END);
            if status != 0 { exit(84); }
            status = ui_mount_add_button(packet, 400, 2, 3, 1, 3, 8, 28, 80, 20, 0, 0, 0, 1);
            if status != 0 { exit(85); }
            status = ui_mount_add_checkbox_with_mark_offset(packet, 400, 3, 4, 1, 3, 8, 52, 80, 20, 0, 0, 0, 1, 69, 0);
            if status != -2 { exit(86); }
            status = ui_mount_add_checkbox_with_mark_offset(packet, 400, 3, 4, 1, 3, 8, 52, 80, 20, 0, 0, 0, 1, 4, -2);
            if status != 0 { exit(86); }
            status = ui_mount_add_list(packet, 400, 4, 5, 1, 3, 8, 76, 120, 40, 0, 0, 0, 0, -1, 16, 1);
            if status != 0 { exit(87); }
            status = ui_mount_add_text_field(packet, 400, 5, 6, 1, 3, 8, 120, 120, 20, 0, 0, 0, 0, 0, 16, -1, 1);
            if status != 0 { exit(88); }
            status = ui_mount_add_dialog(packet, 400, 6, 7, 1, 3, 20, 150, 200, 100, 16, 0, 0, 8, 16, -1, -1, 0);
            if status != 0 { exit(89); }
            status = ui_mount_finish(packet, 400, 392, 376);
            if status != 0 { exit(status); }
            ui_mount(packet, 392);
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_node_builder_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("all named node builders compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            1_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );

    let packet = &host.ui_mounts[0];
    assert_eq!(packet.len(), 392);
    let records = &packet[40..376];
    assert_eq!(records[4], 6); // Panel root.
    assert_eq!(records[48 + 4], 1); // Label.
    assert_eq!(records[96 + 4], 2); // Button.
    assert_eq!(records[144 + 4], 3); // Checkbox.
    assert_eq!(records[192 + 4], 4); // List.
    assert_eq!(records[240 + 4], 5); // TextField.
    assert_eq!(records[288 + 4], 7); // Dialog.
    assert_eq!(
        u32::from_le_bytes(records[48 + 32..48 + 36].try_into().unwrap()),
        2
    );
    assert_eq!(
        u16::from_le_bytes(records[96 + 6..96 + 8].try_into().unwrap()),
        1
    );
    assert_eq!(
        u16::from_le_bytes(records[144 + 6..144 + 8].try_into().unwrap()),
        1
    );
    assert_eq!(
        i32::from_le_bytes(records[144 + 32..144 + 36].try_into().unwrap()),
        4
    );
    assert_eq!(
        i32::from_le_bytes(records[144 + 36..144 + 40].try_into().unwrap()),
        -2
    );

    let stateful_source = r#"
        include "koto_ui.koto";
        static mount: UiMountBuilder = {
            packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
            data_offset: 0, data_cursor: 0, status: 0, active: false,
        };
        fn main() {
            buf packet[400];
            if mount.begin(packet, 400, 7, 1, 3) != 0 { exit(90); }
            mount.panel(1, -1, 3, 0, 0, 320, 320, packet, 0, 0, 4, 12);
            mount.label(2, 1, 3, 8, 8, 80, 16, packet, 0, 0, UI_ALIGN_END);
            mount.button(3, 1, 3, 8, 28, 80, 20, packet, 0, 0, true);
            mount.checkbox_with_mark_offset(4, 1, 3, 8, 52, 80, 20,
                packet, 0, 0, true, 4, -2);
            mount.list(5, 1, 3, 8, 76, 120, 40,
                packet, 0, 0, 0, -1, 16, true);
            mount.text_field(6, 1, 3, 8, 120, 120, 20,
                packet, 0, 0, packet, 0, 16, -1, true);
            mount.dialog(7, 1, 3, 20, 150, 200, 100,
                packet, 0, 0, 8, 16, -1, -1, false);
            let len = mount.finish();
            if len != 392 { exit(91); }
            ui_mount(packet, len);
            exit(0);
        }
    "#;
    let stateful_bytecode = compile_with_loader(
        "ui_stateful_node_builder_test.koto",
        stateful_source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("all stateful node methods compile");
    let stateful_program =
        verify_kbc(&stateful_bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut stateful_vm = BytecodeVm::<16, 4>::new(&stateful_program).unwrap();
    let mut stateful_heap = vec![0u8; stateful_program.header().max_heap_bytes as usize];
    let mut stateful_host = CaptureHost::default();
    assert_eq!(
        stateful_vm
            .execute_frame(
                &stateful_bytecode,
                &stateful_program,
                &mut stateful_host,
                VmInputSnapshot::empty(),
                2_000_000,
                &mut stateful_heap,
            )
            .unwrap(),
        VmRunResult::Exited(0)
    );
    assert_eq!(stateful_host.ui_mounts[0], host.ui_mounts[0]);
}

#[test]
fn ui_sdk_targeted_update_builders_encode_all_v1_properties_atomically() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf packet[355];
            let status = ui_update_begin(packet, 355, 10, 352);
            if status != 0 { exit(40); }
            status = ui_update_set_bounds(packet, 355, 6, 7, 0, 0, 0, 20);
            if status != -2 { exit(41); }
            if heap_get_u16(packet + 224) != 0 { exit(42); }
            status = ui_update_set_text(packet, 355, 0, 1, 0, 3);
            if status != 0 { exit(43); }
            status = ui_update_set_enabled(packet, 355, 1, 2, 1);
            if status != 0 { exit(44); }
            status = ui_update_set_visible(packet, 355, 2, 3, 1);
            if status != 0 { exit(45); }
            status = ui_update_set_checked(packet, 355, 3, 4, 1);
            if status != 0 { exit(46); }
            status = ui_update_set_selection(packet, 355, 4, 5, 0);
            if status != 0 { exit(47); }
            status = ui_update_set_text_value(packet, 355, 5, 6, 0, 3, 3);
            if status != 0 { exit(48); }
            status = ui_update_set_bounds(packet, 355, 6, 7, 10, 20, 100, 30);
            if status != 0 { exit(49); }
            status = ui_update_set_list_rows(packet, 355, 7, 8, 0, 0, 0, -1);
            if status != 0 { exit(50); }
            status = ui_update_set_dialog_open(packet, 355, 8, 9, 1);
            if status != 0 { exit(51); }
            status = ui_update_request_focus(packet, 355, 9, 10);
            if status != 0 { exit(52); }
            heap_set_u8(packet + 352, 227);
            heap_set_u8(packet + 353, 129);
            heap_set_u8(packet + 354, 130);
            if ui_update_finish(packet, 355, 355, 352) != 0 { exit(53); }
            ui_update(packet, 355);
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_update_builder_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("all targeted update builders compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            2_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
    let packet = &host.ui_updates[0];
    assert_eq!(&packet[..4], b"KUP1");
    assert_eq!(packet.len(), 355);
    assert_eq!(u16::from_le_bytes(packet[12..14].try_into().unwrap()), 10);
    for property in 1..=10 {
        assert_eq!(packet[32 + (property - 1) * 32 + 2], property as u8);
    }
    assert_eq!(&packet[352..], &[0xe3, 0x81, 0x82]);
}

#[test]
fn ui_sdk_builders_accept_full_node_and_update_record_capacities() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf mount_packet[1576];
            heap_set_u8(mount_packet, 165);
            if ui_mount_begin(mount_packet, 1576, 33, 1576, 1, -1) != -2 { exit(30); }
            if heap_get_u8(mount_packet) != 165 { exit(31); }
            if ui_mount_begin(mount_packet, 1576, 32, 1576, 1, -1) != 0 { exit(32); }
            if ui_mount_add_panel(mount_packet, 1576, 0, 1, -1, 3, 0, 0, 320, 320, 0, 0, 0, 0, 0) != 0 { exit(33); }
            let i = 1;
            while i < 32 {
                let capacity = 0;
                if i == 1 { capacity = UI_MAX_DATA_BYTES; }
                if ui_mount_add_label(mount_packet, 1576, i, i + 1, 1, 3,
                    i, i, 1, 1, 0, 0, capacity, UI_ALIGN_START) != 0 { exit(34); }
                i = i + 1;
            }
            if ui_mount_finish(mount_packet, 1576, 1576, 1576) != 0 { exit(35); }
            ui_mount(mount_packet, 1576);

            buf update_packet[544];
            heap_set_u8(update_packet, 165);
            if ui_update_begin(update_packet, 544, 17, 544) != -2 { exit(36); }
            if heap_get_u8(update_packet) != 165 { exit(37); }
            if ui_update_begin(update_packet, 544, 16, 544) != 0 { exit(38); }
            i = 0;
            while i < 16 {
                if ui_update_set_visible(update_packet, 544, i, i + 1, 1) != 0 { exit(39); }
                i = i + 1;
            }
            if ui_update_finish(update_packet, 544, 544, 544) != 0 { exit(40); }
            ui_update(update_packet, 544);
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_full_capacity_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("full-capacity builders compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            5_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
    assert_eq!(host.ui_mounts.len(), 1);
    assert_eq!(host.ui_updates.len(), 1);
    assert_eq!(
        u16::from_le_bytes(host.ui_mounts[0][12..14].try_into().unwrap()),
        32
    );
    assert_eq!(
        u16::from_le_bytes(host.ui_updates[0][12..14].try_into().unwrap()),
        16
    );
}

#[test]
fn ui_sdk_counter_example_mounts_once_and_reaches_idle_yield() {
    const SAMPLE: &str = include_str!("../../../sdk/examples/koto_ui_counter.koto");
    let bytecode = compile_with_loader(
        "examples/koto_ui_counter.koto",
        SAMPLE,
        CodegenOptions::default(),
        &mut standard_sdk_loader(),
    )
    .expect("documented KotoUI example compiles");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut activated = vec![0u8; 32];
    activated[..4].copy_from_slice(b"KUE1");
    activated[4..6].copy_from_slice(&1u16.to_le_bytes());
    activated[8..12].copy_from_slice(&32u32.to_le_bytes());
    activated[12] = 1;
    activated[14..16].copy_from_slice(&3u16.to_le_bytes());
    let mut host = CaptureHost {
        ui_events: vec![activated],
        ..CaptureHost::default()
    };
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            5_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(host.ui_mounts.len(), 1);
    assert_eq!(host.ui_updates.len(), 1);
    assert_eq!(&host.ui_mounts[0][..4], b"KUI1");
    assert_eq!(&host.ui_updates[0][..4], b"KUP1");
    assert_eq!(&host.ui_updates[0][64..], b"Count: 1");
}

#[test]
fn ui_sdk_event_accessors_validate_and_decode_kue1() {
    let source = "
        include \"koto_ui.koto\";
        fn main() {
            buf event[40];
            heap_set_u8(event + 0, 75);
            heap_set_u8(event + 1, 85);
            heap_set_u8(event + 2, 69);
            heap_set_u8(event + 3, 49);
            heap_set_u16(event + 4, UI_ABI_MAJOR);
            heap_set_u16(event + 6, UI_ABI_MINOR);
            ui_sdk_put_u32(event, 8, 37);
            heap_set_u8(event + 12, UI_RESPONSE_SELECTION_CHANGED);
            heap_set_u16(event + 14, 42);
            ui_sdk_put_u32(event, 16, 3);
            ui_sdk_put_u32(event, 20, 9001);
            ui_sdk_put_u32(event, 24, 32);
            heap_set_u16(event + 28, 5);
            heap_set_u8(event + 32, 104);
            heap_set_u8(event + 33, 101);
            heap_set_u8(event + 34, 108);
            heap_set_u8(event + 35, 108);
            heap_set_u8(event + 36, 111);
            if ui_event_validate(event, 37) != 0 { exit(91); }
            if ui_event_response(event) != UI_RESPONSE_SELECTION_CHANGED { exit(92); }
            if ui_event_widget_id(event) != 42 { exit(93); }
            if ui_event_index(event) != 3 { exit(94); }
            if ui_event_aux(event) != 9001 { exit(95); }
            if ui_event_text_ptr(event) != event + 32 { exit(96); }
            if ui_event_text_len(event) != 5 { exit(97); }
            heap_set_u16(event + 30, 1);
            if ui_event_validate(event, 37) != -2 { exit(98); }
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_event_accessor_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("event accessors compile");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            1_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn ui_sdk_locale_accessors_and_resource_fallback_cover_product_and_test_tags() {
    let source = "
        include \"koto_ui.koto\";
        data en_us = u8[101, 110, 45, 85, 83];
        data en = u8[101, 110];
        data ja_jp = u8[106, 97, 45, 74, 80];
        data ja = u8[106, 97];
        data fr_ca = u8[102, 114, 45, 67, 65];
        data qps_ploc = u8[113, 112, 115, 45, 112, 108, 111, 99];
        data prefix_trap = u8[101, 110, 103];
        data bad_hyphen = u8[101, 110, 45, 45, 85, 83];

        fn set_capability_locale(cap: int, tag: int, tag_len: int, generation: int) {
            ui_sdk_zero(cap, 64);
            heap_set_u8(cap + 0, 75);
            heap_set_u8(cap + 1, 85);
            heap_set_u8(cap + 2, 67);
            heap_set_u8(cap + 3, 49);
            heap_set_u16(cap + 4, UI_ABI_MAJOR);
            heap_set_u16(cap + 6, UI_ABI_MINOR);
            heap_set_u16(cap + 30, UI_CAP_IME);
            heap_set_u8(cap + 32, tag_len);
            ui_sdk_put_u32(cap, 36, generation);
            let i = 0;
            while i < tag_len {
                heap_set_u8(cap + 40 + i, heap_get_u8(tag + i));
                i = i + 1;
            }
        }

        fn main() {
            buf cap[64];
            set_capability_locale(cap, en_us, 5, 1);
            if ui_capabilities_validate(cap, 64) != 0 { exit(60); }
            if ui_capabilities_locale_ptr(cap) != cap + 40 || ui_capabilities_locale_len(cap) != 5 { exit(61); }
            if ui_capabilities_locale_generation(cap) != 1 || ui_capabilities_direction(cap) != 0 { exit(62); }
            if ui_capabilities_has(cap, UI_CAP_IME) != 1 { exit(63); }

            set_capability_locale(cap, ja_jp, 5, 2);
            if ui_locale_fallback_rank(cap + 40, 5, ja_jp, 5) != 0 { exit(64); }
            if ui_locale_fallback_rank(cap + 40, 5, ja, 2) != 1 { exit(65); }
            if ui_locale_fallback_rank(cap + 40, 5, en_us, 5) != 2 { exit(66); }

            set_capability_locale(cap, fr_ca, 5, 3);
            if ui_locale_fallback_rank(cap + 40, 5, ja_jp, 5) != -1 { exit(67); }
            if ui_locale_fallback_rank(cap + 40, 5, en_us, 5) != 2 { exit(68); }

            set_capability_locale(cap, qps_ploc, 8, 4);
            if ui_locale_fallback_rank(cap + 40, 8, qps_ploc, 8) != 0 { exit(69); }

            if UiLocaleMatch::None != 0 || UiLocaleMatch::Language != 1 ||
               UiLocaleMatch::Exact != 2 { exit(73); }
            if ui_locale_match(en, 2, en, 2) != UiLocaleMatch::Exact ||
               ui_locale_match(en_us, 5, en_us, 5) != UiLocaleMatch::Exact ||
               ui_locale_match(en_us, 5, en, 2) != UiLocaleMatch::Language ||
               ui_locale_match(ja, 2, ja, 2) != UiLocaleMatch::Exact ||
               ui_locale_match(ja_jp, 5, ja, 2) != UiLocaleMatch::Language ||
               ui_locale_match(qps_ploc, 8, qps_ploc, 8) != UiLocaleMatch::Exact { exit(74); }
            if ui_locale_match(fr_ca, 5, ja, 2) != UiLocaleMatch::None ||
               ui_locale_match(prefix_trap, 3, en, 2) != UiLocaleMatch::None ||
               ui_locale_match(en, 2, prefix_trap, 3) != UiLocaleMatch::None ||
               ui_locale_match(en_us, 5, en_us, 4) != UiLocaleMatch::None ||
               ui_locale_match(en_us, 5, en_us, 0) != UiLocaleMatch::None ||
               ui_locale_match(bad_hyphen, 6, en, 2) != UiLocaleMatch::None { exit(75); }
            buf max_tag[24]; let mi = 0;
            while mi < 23 { heap_set_u8(max_tag + mi, 97); mi = mi + 1; }
            if ui_locale_match(max_tag, 23, max_tag, 23) != UiLocaleMatch::Exact ||
               ui_locale_match(max_tag, 24, max_tag, 23) != UiLocaleMatch::None { exit(76); }

            buf event[37];
            heap_set_u8(event + 0, 75);
            heap_set_u8(event + 1, 85);
            heap_set_u8(event + 2, 69);
            heap_set_u8(event + 3, 49);
            heap_set_u16(event + 4, UI_ABI_MAJOR);
            heap_set_u16(event + 6, UI_ABI_MINOR);
            ui_sdk_put_u32(event, 8, 37);
            heap_set_u8(event + 12, UI_RESPONSE_LOCALE_CHANGED);
            ui_sdk_put_u32(event, 16, 5);
            ui_sdk_put_u32(event, 24, 32);
            heap_set_u16(event + 28, 5);
            let i = 0;
            while i < 5 {
                heap_set_u8(event + 32 + i, heap_get_u8(en_us + i));
                i = i + 1;
            }
            if ui_locale_changed_validate(event, 37) != 0 { exit(70); }
            if ui_locale_changed_generation(event) != 5 || ui_locale_changed_direction(event) != 0 { exit(71); }
            if ui_locale_changed_tag_ptr(event) != event + 32 || ui_locale_changed_tag_len(event) != 5 { exit(72); }
            exit(0);
        }
    ";
    let bytecode = compile_with_loader(
        "ui_locale_sdk_test.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("locale SDK compiles");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            1_000_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn draw_pixels_emits_blit_host_call() {
    // The `draw_pixels` wrapper passes a buffer offset and length to the
    // draw_pixels_rgb565 host call (the sprite/tile blit primitive).
    let asm = compile_to_asm(
        "sprite.koto",
        "
        fn main() {
            buf tile[8];
            draw_pixels(10, 20, 2, 1, tile, 4);
            exit(0);
        }
        ",
    )
    .unwrap();
    assert!(
        asm.contains("host_call draw_pixels_rgb565"),
        "missing draw_pixels_rgb565 host call in:\n{asm}"
    );
}

#[test]
fn game2d_tilemap_wrappers_emit_host_calls() {
    // The Game2D wrappers project to the retained tilemap host calls (KOTO-0135):
    // set_tile names a cell, clear_layer empties one, present composites the frame.
    let asm = compile_to_asm(
        "tiles.koto",
        "
        fn main() {
            game2d_configure_tilemap(0, 20, 12, -8, 24);
            game2d_clear_layer(0);
            game2d_set_tile(0, 3, 4, 512);
            game2d_present();
            exit(0);
        }
        ",
    )
    .unwrap();
    for call in [
        "host_call game2d_set_tile",
        "host_call game2d_clear_layer",
        "host_call game2d_configure_tilemap",
        "host_call game2d_present",
    ] {
        assert!(asm.contains(call), "missing {call} in:\n{asm}");
    }
}

#[test]
fn game2d_static_layer_wrappers_emit_host_calls() {
    // The static/background layer wrappers (KOTO-0136) bracket a one-time capture
    // of draw calls into a retained host layer; both take no args and return status.
    let asm = compile_to_asm(
        "static.koto",
        "
        fn main() {
            game2d_static_begin();
            draw_rect(0, 0, 320, 320, 0);
            game2d_static_end();
            exit(0);
        }
        ",
    )
    .unwrap();
    for call in [
        "host_call game2d_static_begin",
        "host_call game2d_static_end",
    ] {
        assert!(asm.contains(call), "missing {call} in:\n{asm}");
    }
}

#[test]
fn game2d_sprite_wrappers_emit_host_calls() {
    // The retained sprite/stamp wrappers (KOTO-0140): stamp_define registers a cell
    // pattern, sprite_set places an instance, sprite_hide/clear_all retire them.
    let asm = compile_to_asm(
        "sprites.koto",
        "
        fn main() {
            game2d_stamp_define(0, 16, 4, 0);
            game2d_sprite_set(0, 0, 24, 0, 512);
            game2d_sprite_hide(1);
            game2d_sprite_clear_all();
            exit(0);
        }
        ",
    )
    .unwrap();
    for call in [
        "host_call game2d_stamp_define",
        "host_call game2d_sprite_set",
        "host_call game2d_sprite_hide",
        "host_call game2d_sprite_clear_all",
    ] {
        assert!(asm.contains(call), "missing {call} in:\n{asm}");
    }
}

#[test]
fn asset_load_emits_host_call() {
    // The `asset_load` wrapper reads a read-only package asset into a heap buffer
    // in one shot (path, len, dst, max) — the source for `draw_pixels` tile blits.
    let asm = compile_to_asm(
        "asset.koto",
        r#"
        fn main() {
            buf sheet[64];
            let n = asset_load("sprites/tiles.kim", 17, sheet, 64);
            exit(n);
        }
        "#,
    )
    .unwrap();
    assert!(
        asm.contains("host_call asset_load"),
        "missing asset_load host call in:\n{asm}"
    );
}

#[test]
fn fetch_sdk_wrappers_emit_bounded_host_calls() {
    let asm = compile_to_asm(
        "fetch.koto",
        r#"
        fn main() {
            buf response[512];
            let id = fetch_start("https://api.example.com/v1", 26);
            let state = fetch_poll_state(id);
            let metadata = fetch_poll_metadata(id);
            let n = fetch_read(id, response, FETCH_MAX_READ);
            fetch_cancel(id);
            exit(state + metadata + n);
        }
        "#,
    )
    .unwrap();
    for call in [
        "host_call fetch_start",
        "host_call fetch_poll",
        "host_call fetch_read",
        "host_call fetch_cancel",
    ] {
        assert!(asm.contains(call), "missing {call} in:\n{asm}");
    }
}

#[test]
fn audio_wrappers_emit_host_calls() {
    // The low-level `audio_submit` primitive plus the host-owned `play_sfx` /
    // `play_bgm` / package-asset BGM / `stop_bgm` triggers and SDK constants.
    let asm = compile_to_asm(
        "audio.koto",
        r#"
        fn main() {
            buf pcm[8];
            audio_submit(pcm, 2, 1);
            play_bgm_asset("audio/bgm.kmml", 15);
            play_sfx_asset("audio/clear.kmml", 16);
            stop_bgm();
            exit(0);
        }
        "#,
    )
    .unwrap();
    assert!(
        asm.contains("host_call audio_submit_i16"),
        "missing audio_submit_i16 host call in:\n{asm}"
    );
    assert!(
        asm.contains("host_call play_bgm_asset"),
        "missing play_bgm_asset host call in:\n{asm}"
    );
    assert!(
        asm.contains("host_call play_sfx_asset"),
        "missing play_sfx_asset host call in:\n{asm}"
    );
    assert!(
        asm.contains("host_call stop_bgm"),
        "missing stop_bgm host call in:\n{asm}"
    );
}

#[test]
fn sdk_constants_are_predefined() {
    // IME_COMMIT == 3, INTENT_EXIT == 1<<14 == 16384, UI_PARENT_ROOT == -1.
    let asm = compile_to_asm(
        "sdk.koto",
        "
        fn main() {
            ime_feed_key(IME_COMMIT, 0);
            let q = INTENT_EXIT;
            let root = UI_PARENT_ROOT;
            exit(0);
        }
        ",
    )
    .unwrap();
    assert!(asm.contains("push_i16 3"), "IME_COMMIT not folded: {asm}");
    assert!(
        asm.contains("push_i16 16384"),
        "INTENT_EXIT not folded: {asm}"
    );
    assert!(
        asm.contains("push_i16 -1"),
        "UI_PARENT_ROOT not folded: {asm}"
    );
}

#[test]
fn ui_sdk_sentinels_are_predefined_with_distinct_names() {
    assert_eq!(
        run("fn main() { exit(UI_PARENT_ROOT + UI_FOCUS_FIRST + UI_SELECTION_NONE + UI_CURSOR_END + UI_ACTION_NONE); }").0,
        VmRunResult::Exited(-5)
    );
}

#[test]
fn text_intent_aliases_text_input_host_call() {
    let asm = compile_to_asm("sdk.koto", "fn main() { exit(text_intent()); }").unwrap();
    assert!(asm.contains("host_call text_input"), "alias missing: {asm}");
}

#[test]
fn user_const_overrides_sdk_constant() {
    let asm = compile_to_asm(
        "sdk.koto",
        "
        const MODE_READ = 7;
        fn main() { let m = MODE_READ; exit(m); }
        ",
    )
    .unwrap();
    assert!(
        asm.contains("push_i16 7"),
        "user const did not override: {asm}"
    );
}

// ---- error paths ----

fn error(source: &str) -> CompileError {
    compile("bad.koto", source).expect_err("should fail to compile")
}

#[test]
fn rejects_syntax_error_with_location() {
    let err = error("fn main() { let x = ; }");
    assert_eq!(err.file, "bad.koto");
    assert!(err.line >= 1 && err.col >= 1);
}

#[test]
fn rejects_undefined_symbol() {
    let err = error("fn main() { exit(missing); }");
    assert!(err.message.contains("missing"), "got: {}", err.message);
}

#[test]
fn rejects_undefined_function() {
    let err = error("fn main() { nope(1); }");
    assert!(err.message.contains("nope"), "got: {}", err.message);
}

#[test]
fn rejects_wrong_argument_count() {
    let err = error("fn main() { exit(1, 2); }");
    assert!(err.message.contains("argument"), "got: {}", err.message);
}

#[test]
fn rejects_index_of_non_buffer() {
    let err = error("fn main() { let x = 1; exit(x[0]); }");
    assert!(err.message.contains("buffer"), "got: {}", err.message);
}

#[test]
fn rejects_recursion_as_unsupported() {
    let err = error(
        "
        fn loopy(n: int) -> int { return loopy(n); }
        fn main() { exit(loopy(1)); }
        ",
    );
    assert!(err.message.contains("recursion"), "got: {}", err.message);
}

#[test]
fn rejects_missing_main() {
    let err = error("fn helper() {}");
    assert!(err.message.contains("main"), "got: {}", err.message);
}

#[test]
fn small_app_requests_minimal_heap() {
    // Per-app heap profile (KOTO-0096): a program with no buffers or string data
    // requests only the small heap floor, far below the old fixed 4 KB, and runs.
    let src = "fn main() { let x = 2; exit(x + 3); }";
    let bytecode = compile("tiny.koto", src).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    assert!(
        program.header().max_heap_bytes < 1024,
        "expected a small heap request, got {}",
        program.header().max_heap_bytes
    );
    assert_eq!(run(src).0, VmRunResult::Exited(5));
}

// ---- Per-scope local slot reuse (KOTO-0092) ----

#[test]
fn reuses_slots_across_disjoint_blocks() {
    // Four disjoint `if` blocks of 15 locals each = 60 `let`s total, but only 15
    // are ever live at once. Without slot reuse this exceeds the 45-slot file;
    // with reuse it fits.
    let mut src = String::from("fn main() {\n");
    for blk in 0..4 {
        src.push_str("    if 1 == 1 {\n");
        for i in 0..15 {
            src.push_str(&format!("        let v{blk}_{i} = {i};\n"));
        }
        src.push_str(&format!("        exit(v{blk}_0);\n"));
        src.push_str("    }\n");
    }
    src.push_str("}\n");
    compile("reuse.koto", &src).expect("60 total locals fit within a peak of 15");
}

#[test]
fn rejects_too_many_simultaneously_live_locals() {
    // A single block whose locals are all live at once still hits the ceiling.
    let mut src = String::from("fn main() {\n");
    for i in 0..60 {
        src.push_str(&format!("    let v{i} = {i};\n"));
    }
    src.push_str("    exit(v0);\n}\n");
    let err = compile("toomany.koto", &src).expect_err("60 live locals overflow the file");
    assert!(
        err.message.contains("simultaneously-live"),
        "got: {}",
        err.message
    );
}

#[test]
fn let_does_not_leak_out_of_block() {
    let err = error("fn main() { if 1 == 1 { let x = 5; } exit(x); }");
    assert!(err.message.contains("undefined"), "got: {}", err.message);
}

#[test]
fn reused_slots_keep_disjoint_block_values_correct() {
    // `a` and `b` land in the same reused slot; the outer `r` must be untouched.
    let (result, _) = run("
        fn main() {
            let r = 0;
            if 1 == 1 { let a = 10; r = r + a; }
            if 1 == 1 { let b = 20; r = r + b; }
            exit(r);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(30));
}

#[test]
fn slot_map_reports_footprints_and_post_reuse_peak() {
    let map = slot_map(
        "test.koto",
        "
        fn add(a: int, b: int) -> int { return a + b; }
        fn main() {
            let x = 1;
            if 1 == 1 { let t = 2; x = x + t; }
            if 1 == 1 { let u = 3; x = x + u; }
            exit(add(x, 0));
        }
        ",
    )
    .expect("slot map");

    assert_eq!(map.user_slots_cap, koto_core::runtime::VM_LOCAL_SLOTS - 3);
    assert_eq!(map.scratch_slots, 3);

    // Per-function footprints (own params + peak lets), no fixed ranges anymore.
    let add = &map.functions[0];
    assert_eq!(add.name, "add");
    assert_eq!(add.params, 2);
    assert_eq!(add.locals, 0); // no lets
    assert_eq!(add.slots(), 2);

    let main = &map.functions[1];
    assert_eq!(main.name, "main");
    assert_eq!(main.params, 0);
    assert_eq!(main.locals, 2); // `x` plus one reused block slot (t/u share)
    assert_eq!(main.slots(), 2);

    // Post-reuse peak: `x` (slot 0) is live across the disjoint t/u blocks (reused
    // at slot 1) and across the inlined `add(x, 0)`, whose two params take slots
    // 1 and 2. The disjoint-block sum would be 4; reuse brings the real peak to 3.
    assert_eq!(map.user_slots_used, 3);

    let text = describe_slot_map(&map);
    assert!(text.starts_with("slot-map user_slots_used=3 user_slots_cap="));
    assert!(text.contains("\nfn add params=2 locals=0 footprint=2 src=test.koto:2"));
    assert!(text.contains("\nfn main params=0 locals=2 footprint=2 src=test.koto:3"));
}

#[test]
fn inline_slots_are_reused_across_call_boundaries() {
    // Two distinct value-returning helpers, each a 4-slot footprint, called in
    // sequence. With disjoint per-function blocks the sum would be f(4) + g(4) +
    // main(1) = 9 user slots; with call-site reuse they share the same physical
    // slots above `x`, so the post-reuse peak is just 1 + 4 = 5.
    let map = slot_map(
        "test.koto",
        "
        fn f(a: int, b: int, c: int, d: int) -> int { return a + b + c + d; }
        fn g(a: int, b: int, c: int, d: int) -> int { return a - b - c - d; }
        fn main() {
            let x = 0;
            x = f(1, 2, 3, 4);
            x = g(5, 6, 7, 8);
            exit(x);
        }
        ",
    )
    .expect("slot map");
    assert_eq!(map.user_slots_used, 5);
}

#[test]
fn value_returning_inline_correct_after_reuse() {
    // Two calls to a value-returning helper reuse the same slots; both must still
    // return the right value through the floating return scratch slot.
    let (result, _) = run("
        fn sq(n: int) -> int { return n * n; }
        fn main() {
            let a = sq(3);
            let b = sq(4);
            exit(a + b);
        }
        ");
    assert_eq!(result, VmRunResult::Exited(25)); // 9 + 16
}

#[test]
fn floating_scratch_keeps_local_peak_proportional_to_user_slots() {
    // The codegen scratch slots float just above the user-slot high-water mark
    // (KOTO-0146) instead of pinning at the top of the 48-slot file. The VM's
    // `local_slots_peak` tracks the highest slot *index* touched, so with the old
    // fixed top-of-file scratch this program would report 48 (slot 47 = the return
    // slot) even though it owns a single user local. With floating scratch the three
    // scratch slots sit just above `r`, so the reported peak reflects real pressure
    // -- and the `%` and value-returning call still compute correctly.
    let (result, _, budget) = run_with_budget(
        "
        fn dbl(n: int) -> int { return n * 2; }
        fn main() {
            let r = dbl(7) + 17 % 5;   // 14 + 2 = 16
            exit(r);
        }
        ",
    );
    assert_eq!(result, VmRunResult::Exited(16));
    assert!(
        budget.local_slots_peak < koto_core::runtime::VM_LOCAL_SLOTS as u16,
        "floating scratch should keep the peak below the ceiling, got {}",
        budget.local_slots_peak
    );
    // `r` lives in slot 0; mod and the return route through the floating scratch at
    // slots 1/2/3, so the highest index touched is 3 -> peak 4.
    assert_eq!(budget.local_slots_peak, 4);
}

#[test]
fn floating_scratch_does_not_corrupt_adjacent_user_locals() {
    // Pack several user locals so the floating scratch region sits immediately above
    // them, then run a `%` (scratch operand slots) and a value-returning call (the
    // return scratch slot) *while those locals are live*. If a scratch slot aliased a
    // live user slot the final sum would be wrong.
    let (result, _) = run("
        fn add1(n: int) -> int { return n + 1; }
        fn main() {
            let a = 1;
            let b = 2;
            let c = 3;
            let d = 4;
            let e = 5;
            let m = 17 % 5;       // 2, via the scratch operand slots
            let f = add1(a + b);  // 4, via the floating return scratch slot
            exit(a + b + c + d + e + m + f);  // 1+2+3+4+5+2+4 = 21
        }
        ");
    assert_eq!(result, VmRunResult::Exited(21));
}

#[test]
fn nested_inline_args_do_not_clobber_bound_params() {
    // Each argument to `add3` is itself an inline call. The nested expansion for a
    // later argument must allocate above the parameter slots already bound for the
    // earlier arguments, or it would clobber them.
    let (result, _) = run("
        fn inc(n: int) -> int { return n + 1; }
        fn add3(a: int, b: int, c: int) -> int { return a + b + c; }
        fn main() {
            exit(add3(inc(10), inc(20), inc(30)));
        }
        ");
    assert_eq!(result, VmRunResult::Exited(63)); // 11 + 21 + 31
}

// ---- KOTO-0156: one-time preamble relocation ----

/// KOTO-0156 #1 only (preamble relocation, no outlining).
fn relocate_only() -> CodegenOptions {
    CodegenOptions {
        relocate_preamble: true,
        ..CodegenOptions::default()
    }
}

/// KOTO-0156 #2 only (cold-block outlining, no preamble relocation).
fn outline_only() -> CodegenOptions {
    CodegenOptions {
        outline_cold_blocks: true,
        ..CodegenOptions::default()
    }
}

/// Run pre-assembled bytecode for one frame with a capture host, exactly as
/// [`run_with_budget`] does for source, so equivalence tests can run two layouts.
fn run_bytecode(bytecode: &[u8]) -> (VmRunResult, CaptureHost) {
    let program: VerifiedProgram =
        verify_kbc(bytecode, RuntimeLimits::simulator_default()).expect("verifies");
    let mut vm = BytecodeVm::<16, 4>::new(&program).expect("vm");
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = CaptureHost::default();
    let result = vm
        .execute_frame(
            bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .expect("runs without trapping");
    (result, host)
}

#[test]
fn preamble_relocation_preserves_behavior() {
    // The body reads several string literals (one repeated, to exercise dedup). The
    // tail-relocated preamble must still copy them into the heap before the body
    // draws them, so both layouts must produce identical observable output.
    let src = "
        fn main() {
            draw_text(1, 2, \"alpha\", 5);
            draw_text(3, 4, \"beta\", 4);
            draw_text(5, 6, \"alpha\", 5);
            exit(9);
        }
        ";
    let relocated =
        crate::compile_with_options("eq.koto", src, relocate_only()).expect("relocated compiles");
    let inline = crate::compile_with_options("eq.koto", src, CodegenOptions::default())
        .expect("inline compiles");
    // The layouts are genuinely different bytecode (the transform did something)...
    assert_ne!(
        relocated, inline,
        "relocation did not change the bytecode layout"
    );
    // ...yet observably identical when run.
    let (r_res, r_host) = run_bytecode(&relocated);
    let (i_res, i_host) = run_bytecode(&inline);
    assert_eq!(r_res, i_res, "result differs between layouts");
    assert_eq!(r_res, VmRunResult::Exited(9));
    assert_eq!(
        r_host.text, i_host.text,
        "captured text differs between layouts"
    );
    assert_eq!(
        r_host.text,
        [
            (1, 2, String::from("alpha")),
            (3, 4, String::from("beta")),
            (5, 6, String::from("alpha")),
        ]
    );
}

#[test]
fn preamble_relocation_moves_store_str_past_the_body() {
    let src = "fn main() { draw_text(0, 0, \"hi\", 2); exit(0); }";

    let asm = crate::compile_to_asm_with_options("layout.koto", src, relocate_only()).unwrap();
    let lines: Vec<&str> = asm.lines().map(str::trim).collect();
    // Entry branches to the relocated preamble instead of running store_str inline.
    let main_at = lines.iter().position(|l| *l == "main:").unwrap();
    assert!(
        lines[main_at + 1].starts_with("br init_strings"),
        "entry should branch to the relocated preamble, got: {}",
        lines[main_at + 1]
    );
    // store_str now lives *after* the body's terminator (the `main_end` label), and
    // the preamble branches back to the body.
    let store_at = lines
        .iter()
        .position(|l| l.starts_with("store_str"))
        .unwrap();
    let end_at = lines
        .iter()
        .position(|l| l.starts_with("main_end"))
        .unwrap();
    assert!(
        store_at > end_at,
        "store_str should be relocated past main_end:\n{asm}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("br body")),
        "missing branch back to the body:\n{asm}"
    );

    // With relocation disabled, the preamble stays inline before the body — the
    // byte-for-byte pre-KOTO-0156 layout, with no relocation branch.
    let inline =
        crate::compile_to_asm_with_options("layout.koto", src, CodegenOptions::default()).unwrap();
    let il: Vec<&str> = inline.lines().map(str::trim).collect();
    let i_store = il.iter().position(|l| l.starts_with("store_str")).unwrap();
    let i_end = il.iter().position(|l| l.starts_with("main_end")).unwrap();
    assert!(
        i_store < i_end,
        "inline layout should keep store_str before the body"
    );
    assert!(
        !il.iter().any(|l| l.starts_with("br init_strings")),
        "inline layout must not add a relocation branch"
    );
}

#[test]
fn stringless_program_is_byte_identical_either_way() {
    // No string literals -> nothing to relocate -> the two layouts must be identical,
    // and no spurious labels or branches are introduced.
    let src = "fn main() { let a = 2; let b = 3; exit(a * b); }";
    let relocated = crate::compile_with_options("nostr.koto", src, relocate_only()).unwrap();
    let inline = crate::compile_with_options("nostr.koto", src, CodegenOptions::default()).unwrap();
    assert_eq!(
        relocated, inline,
        "a program with no strings must be byte-identical with and without relocation"
    );
    assert_eq!(run_bytecode(&relocated).0, VmRunResult::Exited(6));
}

/// Build `fn main` with a loop whose `if cond { … continue; }` then-block is padded with
/// `count` draw_rect calls so it exceeds the outline threshold. `cond_takes_first` picks
/// a condition true on the first iteration (so the cold path actually executes once).
fn loop_with_cold_block(count: usize, marker: &str, cond_takes_first: bool) -> String {
    let mut cold = String::new();
    for i in 0..count {
        cold.push_str(&format!(
            "                    draw_rect({i}, 0, 1, 1, 0);\n"
        ));
    }
    let cond = if cond_takes_first { "n == 1" } else { "n < 0" };
    format!(
        "
        fn main() {{
            let n = 0;
            loop {{
                n = n + 1;
                if {cond} {{
                    draw_text(0, 0, \"{marker}\", 1);
{cold}                    continue;
                }}
                if n >= 3 {{ break; }}
            }}
            exit(n);
        }}
        "
    )
}

#[test]
fn cold_block_outlining_preserves_behavior() {
    // The cold block is taken once (first iteration), draws a marker, then continues —
    // exercising the branch-to-tail and the rejoin. Outlined vs baseline must match.
    let src = loop_with_cold_block(16, "A", true);
    let outlined =
        crate::compile_with_options("cold.koto", &src, outline_only()).expect("outlined compiles");
    let baseline = crate::compile_with_options("cold.koto", &src, CodegenOptions::default())
        .expect("baseline compiles");
    assert_ne!(
        outlined, baseline,
        "outlining did not change the bytecode layout"
    );
    let (o_res, o_host) = run_bytecode(&outlined);
    let (b_res, b_host) = run_bytecode(&baseline);
    assert_eq!(o_res, b_res, "result differs between layouts");
    assert_eq!(o_res, VmRunResult::Exited(3));
    assert_eq!(
        o_host.text, b_host.text,
        "captured text differs between layouts"
    );
    assert_eq!(o_host.text, [(0, 0, String::from("A"))]);
}

#[test]
fn cold_block_outlining_moves_block_past_exit() {
    let src = loop_with_cold_block(16, "Z", false);
    let asm = crate::compile_to_asm_with_options("cold.koto", &src, outline_only()).unwrap();
    let lines: Vec<&str> = asm.lines().map(str::trim).collect();
    // The cold then-block is replaced inline by a branch to a tail label...
    let cold_br = lines
        .iter()
        .position(|l| l.starts_with("br cold"))
        .expect("expected a branch to the outlined block");
    let main_end = lines
        .iter()
        .position(|l| l.starts_with("main_end"))
        .unwrap();
    // ...whose label sits past `main_end` (after the program's exit).
    let cold_label = lines
        .iter()
        .position(|l| l.starts_with("cold") && l.ends_with(':'))
        .expect("expected the outlined block label at the tail");
    assert!(cold_br < main_end, "the branch-to-tail must be in the body");
    assert!(
        cold_label > main_end,
        "the outlined block must be relocated past the exit"
    );

    // Baseline keeps the block inline: no outline branch, no tail block.
    let base =
        crate::compile_to_asm_with_options("cold.koto", &src, CodegenOptions::default()).unwrap();
    assert!(
        !base.lines().any(|l| l.trim().starts_with("br cold")),
        "baseline must not outline:\n{base}"
    );
}

#[test]
fn small_cold_block_stays_inline() {
    // A tiny continue-terminated block is below the threshold and must stay inline.
    let src = loop_with_cold_block(0, "x", false);
    let asm = crate::compile_to_asm_with_options("small.koto", &src, outline_only()).unwrap();
    assert!(
        !asm.lines().any(|l| l.trim().starts_with("br cold")),
        "a small cold block must not be outlined:\n{asm}"
    );
}

#[test]
fn outline_plus_relocate_preserves_behavior() {
    // The shipping KotoBlocks configuration: both transforms on. The program has a
    // string (so #1 relocates the preamble) and a large cold block (so #2 outlines it);
    // observable behavior must match the baseline exactly.
    let src = loop_with_cold_block(16, "X", true);
    let both = CodegenOptions {
        relocate_preamble: true,
        outline_cold_blocks: true,
        ..CodegenOptions::default()
    };
    let combined = crate::compile_with_options("both.koto", &src, both).expect("compiles");
    let baseline = crate::compile_with_options("both.koto", &src, CodegenOptions::default())
        .expect("compiles");
    assert_ne!(
        combined, baseline,
        "combined transforms did not change the layout"
    );
    let (c_res, c_host) = run_bytecode(&combined);
    let (b_res, b_host) = run_bytecode(&baseline);
    assert_eq!(c_res, b_res, "result differs between layouts");
    assert_eq!(c_res, VmRunResult::Exited(3));
    assert_eq!(
        c_host.text, b_host.text,
        "captured text differs between layouts"
    );
    assert_eq!(c_host.text, [(0, 0, String::from("X"))]);
}

// ---- KOTO-0183: include expansion through the full pipeline ----

/// In-memory include loader for hermetic multi-file tests.
struct MapLoader(std::collections::HashMap<&'static str, &'static str>);

impl IncludeLoader for MapLoader {
    fn load(&mut self, path: &std::path::Path) -> Result<String, String> {
        let key = path.to_string_lossy().replace('\\', "/");
        self.0
            .get(key.as_str())
            .map(|source| source.to_string())
            .ok_or_else(|| "no such file".to_string())
    }
}

fn loader(files: &[(&'static str, &'static str)]) -> MapLoader {
    MapLoader(files.iter().copied().collect())
}

fn sdk_loader() -> MapLoader {
    loader(&[
        ("koto_ui.koto", include_str!("../../../sdk/koto_ui.koto")),
        (
            "koto_ui/abi.koto",
            include_str!("../../../sdk/koto_ui/abi.koto"),
        ),
        (
            "koto_ui/resources.koto",
            include_str!("../../../sdk/koto_ui/resources.koto"),
        ),
        (
            "koto_ui/builders.koto",
            include_str!("../../../sdk/koto_ui/builders.koto"),
        ),
        (
            "koto_ui/events_locale.koto",
            include_str!("../../../sdk/koto_ui/events_locale.koto"),
        ),
    ])
}

fn standard_sdk_loader() -> MapLoader {
    loader(&[
        (
            "sdk/koto_ui.koto",
            include_str!("../../../sdk/koto_ui.koto"),
        ),
        (
            "sdk/koto_ui/abi.koto",
            include_str!("../../../sdk/koto_ui/abi.koto"),
        ),
        (
            "sdk/koto_ui/resources.koto",
            include_str!("../../../sdk/koto_ui/resources.koto"),
        ),
        (
            "sdk/koto_ui/builders.koto",
            include_str!("../../../sdk/koto_ui/builders.koto"),
        ),
        (
            "sdk/koto_ui/events_locale.koto",
            include_str!("../../../sdk/koto_ui/events_locale.koto"),
        ),
    ])
}

#[test]
fn include_compiles_identically_to_the_unsplit_source() {
    // A faithful (line-preserving) split: the extracted lines land in the
    // include file verbatim, so the expanded source — and therefore the
    // bytecode, KDBG line/col entries included — is byte-identical.
    let unsplit =
        "fn add(a: int, b: int) -> int { return a + b; }\nfn main() { exit(add(40, 2)); }\n";
    let root = "include \"util.koto\";\nfn main() { exit(add(40, 2)); }\n";
    let baseline = compile("test.koto", unsplit).expect("unsplit compiles");
    let split = compile_with_loader(
        "test.koto",
        root,
        CodegenOptions::default(),
        &mut loader(&[(
            "util.koto",
            "fn add(a: int, b: int) -> int { return a + b; }\n",
        )]),
    )
    .expect("split compiles");
    assert_eq!(baseline, split, "textual include must be bytecode-free");
    let (result, _) = run_bytecode(&split);
    assert_eq!(result, VmRunResult::Exited(42));
}

#[test]
fn errors_in_included_files_report_their_own_file_and_line() {
    // The parse error lives on line 2 of util.koto, not in the root.
    let root = "include \"util.koto\";\nfn main() { exit(0); }\n";
    let error = compile_with_loader(
        "test.koto",
        root,
        CodegenOptions::default(),
        &mut loader(&[("util.koto", "fn ok() { }\nfn bad( { }\n")]),
    )
    .expect_err("must not compile");
    assert_eq!(
        (error.file.as_str(), error.line),
        ("util.koto", 2),
        "{error}"
    );
}

#[test]
fn errors_after_an_include_report_shifted_root_lines_correctly() {
    // util.koto occupies 3 expanded lines; the root's own error on its line 3
    // must still be reported as test.koto:3.
    let root = "include \"util.koto\";\nfn main() { exit(0); }\nfn main() { exit(1); }\n";
    let error = compile_with_loader(
        "test.koto",
        root,
        CodegenOptions::default(),
        &mut loader(&[("util.koto", "fn a() { }\nfn b() { }\nfn c() { }\n")]),
    )
    .expect_err("duplicate main");
    assert_eq!(
        (error.file.as_str(), error.line),
        ("test.koto", 3),
        "{error}"
    );
    assert!(error.message.contains("already defined"), "{error}");
}

#[test]
fn duplicate_definitions_across_files_attribute_the_second_site() {
    let root = "include \"a.koto\";\ninclude \"b.koto\";\nfn main() { exit(0); }\n";
    let error = compile_with_loader(
        "test.koto",
        root,
        CodegenOptions::default(),
        &mut loader(&[
            ("a.koto", "fn helper() { }\n"),
            ("b.koto", "fn helper() { }\n"),
        ]),
    )
    .expect_err("duplicate helper");
    assert_eq!((error.file.as_str(), error.line), ("b.koto", 1), "{error}");
    assert!(error.message.contains("already defined"), "{error}");
}

#[test]
fn slot_map_attributes_functions_to_their_defining_files() {
    let root = "include \"util.koto\";\nfn main() { let x = big(1, 2, 3); exit(x); }\n";
    let map = slot_map_with_loader(
        "test.koto",
        root,
        &mut loader(&[(
            "util.koto",
            "fn big(a: int, b: int, c: int) -> int {\n    let t = a + b;\n    return t + c;\n}\n",
        )]),
    )
    .expect("slot map");
    let text = describe_slot_map(&map);
    assert!(
        text.contains("\nfn big params=3 locals=1 footprint=4 src=util.koto:1"),
        "{text}"
    );
    assert!(
        text.contains("\nfn main params=0 locals=1 footprint=1 src=test.koto:2"),
        "{text}"
    );
}

// ---- KOTO-0193: editor-facing compilation values ------------------------

#[test]
fn compile_source_returns_bytecode_slots_and_definition_symbols() {
    let source = "const LIMIT = 42;\nfn helper(value: int) -> int { return value; }\nfn main() { exit(helper(LIMIT)); }\n";
    let result = compile_source(CompileRequest::new("api.koto", source), &mut loader(&[]));
    assert!(result.succeeded(), "{:?}", result.diagnostics);
    assert_eq!(
        result.bytecode.as_deref(),
        Some(compile("api.koto", source).expect("legacy API").as_slice()),
        "value API must preserve bytecode"
    );
    assert!(result.assembly.is_some());
    let slots = result.slot_map.expect("slot map");
    assert_eq!(slots.functions[0].name, "helper");

    let limit = result
        .symbols
        .iter()
        .find(|symbol| symbol.name == "LIMIT")
        .expect("const symbol");
    assert_eq!(limit.kind, SymbolKind::Constant);
    assert_eq!(limit.definition.file, "api.koto");
    assert_eq!(
        (limit.definition.start.line, limit.definition.start.column),
        (1, 7)
    );
    assert_eq!(limit.definition.end.column, 12);

    let parameter = result
        .symbols
        .iter()
        .find(|symbol| symbol.kind == SymbolKind::Parameter)
        .expect("parameter symbol");
    assert_eq!(parameter.name, "value");
    assert_eq!(parameter.container.as_deref(), Some("helper"));
}

#[test]
fn overlay_loader_compiles_unsaved_include_buffer() {
    let root = "include \"util.koto\";\nfn main() { exit(answer()); }\n";
    // The fallback models the saved file. The overlay models an unsaved edit.
    let mut resolver = OverlayLoader::new(loader(&[(
        "util.koto",
        "fn answer() -> int { return 1; }\n",
    )]));
    resolver.insert("./util.koto", "fn answer() -> int { return 42; }\n");
    let result = compile_source(CompileRequest::new("main.koto", root), &mut resolver);
    assert!(result.succeeded(), "{:?}", result.diagnostics);
    let (run_result, _) = run_bytecode(result.bytecode.as_deref().expect("bytecode"));
    assert_eq!(run_result, VmRunResult::Exited(42));
    let answer = result
        .symbols
        .iter()
        .find(|symbol| symbol.name == "answer")
        .expect("included function symbol");
    assert_eq!(answer.definition.file, "util.koto");
    assert_eq!(answer.definition.start.line, 1);
}

#[test]
fn compile_source_maps_structured_diagnostic_to_overlay_file() {
    let root = "include \"util.koto\";\nfn main() { exit(0); }\n";
    let mut resolver = OverlayLoader::new(loader(&[]));
    resolver.insert("util.koto", "fn broken( { }\n");
    let result = compile_source(CompileRequest::new("main.koto", root), &mut resolver);
    assert!(!result.succeeded());
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    let span = diagnostic.span.as_ref().expect("source span");
    assert_eq!(span.file, "util.koto");
    assert_eq!(span.start.line, 1);
    assert!(span.start.column > 0);
    assert!(diagnostic.message.contains("expected"), "{diagnostic}");
}

// ---- KOTO-0228: static records and inline methods -----------------------

#[test]
fn static_record_fields_and_methods_round_trip_through_heap() {
    let source = r#"
struct Player { x: int, y: int, alive: bool, }
static player: Player = { alive: true, y: 20, x: 10, };
impl Player {
    fn move_by(self, dx: int, dy: int) {
        self.x = self.x + dx;
        self.y = self.y + dy;
    }
    fn score(self) -> int { return self.x + self.y; }
}
fn score(value: Player) -> int { return value.score(); }
fn main() {
    player.move_by(2, -1);
    if player.alive { exit(score(player)); }
    exit(-1);
}
"#;
    assert_eq!(run(source).0, VmRunResult::Exited(31));
    let asm = compile_to_asm("records.koto", source).unwrap();
    assert!(asm.contains("load32"), "{asm}");
    assert!(asm.contains("store32"), "{asm}");
    assert!(asm.contains(".rodata 0a0000001400000001000000"), "{asm}");
}

#[test]
fn checked_in_static_record_sample_runs() {
    assert_eq!(
        run(include_str!("../../../sdk/examples/static_record.koto")).0,
        VmRunResult::Exited(2)
    );
}

#[test]
fn static_record_mutation_persists_across_yield() {
    let source = r#"
struct State { value: int, }
static state: State = { value: 5, };
fn main() {
    state.value = state.value + 1;
    yield_frame();
    exit(state.value);
}
"#;
    let bytecode = compile("persist.koto", source).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0; program.header().max_heap_bytes as usize];
    let (start, end) = program.rodata_range().unwrap();
    heap[..end - start].copy_from_slice(&bytecode[start..end]);
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap
        )
        .unwrap(),
        VmRunResult::Yielded
    );
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap
        )
        .unwrap(),
        VmRunResult::Exited(6)
    );
}

#[test]
fn duplicate_method_in_include_reports_the_second_file() {
    let root = "include \"methods.koto\";\nstruct S { x: int, }\nimpl S { fn value(self) -> int { return self.x; } }\nfn main() {}\n";
    let mut resolver = loader(&[(
        "methods.koto",
        "impl S { fn value(self) -> int { return self.x; } }\n",
    )]);
    let error = compile_with_loader("main.koto", root, CodegenOptions::default(), &mut resolver)
        .unwrap_err();
    assert_eq!(error.file, "main.koto");
    assert!(
        error
            .message
            .contains("method `S::value` is already defined"),
        "{error}"
    );
}

#[test]
fn static_record_rejections_are_explicit() {
    for (source, expected) in [
        (
            "struct S { x: int, } static s: S = {}; fn main() {}",
            "missing initializer",
        ),
        (
            "struct S { x: int, } static s: S = { x: 1, x: 2, }; fn main() {}",
            "more than once",
        ),
        (
            "struct S { x: int, } static s: S = { y: 1, }; fn main() {}",
            "unknown field",
        ),
        (
            "struct S { x: int, } static s: S = { x: 1, }; fn main() { let copy = s; }",
            "stored struct aliases",
        ),
        (
            "struct S { x: int, } fn main() { let s = S { x: 1 }; }",
            "expected Semi",
        ),
        (
            "fn main() { static s: Missing = {}; }",
            "only allowed at top level",
        ),
        (
            "struct FileMode { x: int, } fn main() {}",
            "struct `FileMode` is already defined",
        ),
        (
            "struct S { x: int, } static UI_MAX_NODES: S = { x: 1, }; fn main() {}",
            "name `UI_MAX_NODES` is already defined",
        ),
    ] {
        let error = compile("reject.koto", source).unwrap_err();
        assert!(error.message.contains(expected), "{}: {}", expected, error);
    }
}

#[test]
fn static_bool_const_and_heap_limit_are_checked() {
    let source = "const READY = true; struct S { ready: bool, } static s: S = { ready: READY, }; fn main() { if s.ready { exit(1); } }";
    assert_eq!(run(source).0, VmRunResult::Exited(1));

    let mut oversized = String::from("struct Huge {");
    for index in 0..4097 {
        oversized.push_str(&format!("f{index}: int,"));
    }
    oversized.push_str("} static huge: Huge = {");
    for index in 0..4097 {
        oversized.push_str(&format!("f{index}: 0,"));
    }
    oversized.push_str("}; fn main() {}");
    let error = compile("huge.koto", &oversized).unwrap_err();
    assert!(error.message.contains("App heap request"), "{error}");
}

#[test]
fn static_initial_image_stays_compact_before_large_mutable_buffers() {
    let source = "struct S { value: int, } static s: S = { value: 7, }; fn main() { buf pixels[4096]; pixels[0] = 9; exit(s.value + pixels[0]); }";
    let bytecode = compile("compact.koto", source).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let (start, end) = program.rodata_range().expect("static rodata");
    assert_eq!(end - start, 4);
    assert_eq!(run(source).0, VmRunResult::Exited(16));
}

// ---- KOTO-0235: fixed buffer fields in static records ---------------------

#[test]
fn buffer_fields_size_with_literals_consts_and_capacity_helpers() {
    let source = r#"
const DOC_BYTES = 64;
struct Storage {
    raw: buf[384],
    doc: buf[DOC_BYTES],
    mount: buf[ui_mount_capacity(5, 176)],
    mode: int,
}
static storage: Storage = { mode: 2, };
fn main() {
    exit(len(storage.raw) + len(storage.doc) + storage.mode);
}
"#;
    assert_eq!(run(source).0, VmRunResult::Exited(384 + 64 + 2));
}

#[test]
fn buffer_field_sizes_carry_the_buf_and_capacity_helper_diagnostics() {
    for (source, expected) in [
        (
            "struct S { r: buf[0], } static s: S = {}; fn main() {}",
            "buffer size must be a positive integer literal, prior integer const, capacity helper, or `asset_len`",
        ),
        (
            "const OFF = -4; struct S { r: buf[OFF], } static s: S = {}; fn main() {}",
            "buffer size must be a positive integer literal, prior integer const, capacity helper, or `asset_len`",
        ),
        (
            "struct S { m: buf[ui_mount_capacity(0, 0)], } static s: S = {}; fn main() {}",
            "ui_mount_capacity arguments exceed the KotoUI v1 packet capacities",
        ),
    ] {
        let error = compile("reject.koto", source).unwrap_err();
        assert!(error.message.contains(expected), "{}: {}", expected, error);
    }
}

#[test]
fn buffer_field_reads_are_addresses_and_fold_on_static_receivers() {
    // `s.payload` on a static receiver folds to one constant: the static sits
    // at heap offset 0, so the field address is exactly 8.
    let source =
        "struct S { pad: buf[8], payload: buf[16], } static s: S = {}; fn main() { exit(s.payload); }";
    assert_eq!(run(source).0, VmRunResult::Exited(8));
    let asm = compile_to_asm("fold.koto", source).unwrap();
    assert!(!asm.contains("add_i32"), "{asm}");
    assert!(!asm.contains("load32"), "{asm}");

    // On a struct parameter the address is the base plus one runtime add, and
    // reads/writes through it hit the same bytes as the static receiver.
    let source = r#"
struct S { pad: buf[8], payload: buf[16], flag: int, }
static s: S = { flag: 3, };
fn poke(v: S, i: int, value: int) { heap_set_u8(v.payload + i, value); }
fn peek(v: S, i: int) -> int { return heap_get_u8(v.payload + i); }
fn main() {
    poke(s, 15, 44);
    exit(peek(s, 15) + heap_get_u8(s.payload + 15) + s.flag);
}
"#;
    assert_eq!(run(source).0, VmRunResult::Exited(44 + 44 + 3));
}

#[test]
fn buffer_fields_match_hand_written_base_plus_offset_behavior_and_bounds() {
    let structured = r#"
struct S { raw: buf[8], mode: int, }
static s: S = { mode: 5, };
fn get(v: S, i: int) -> int { return heap_get_u8(v.raw + i); }
fn main() {
    heap_set_u8(s.raw + 2, 40);
    exit(get(s, 2) + s.mode + len(s.raw));
}
"#;
    let hand_written = r#"
struct M { mode: int, }
static m: M = { mode: 5, };
const RAW_BYTES = 8;
fn get(base: int, i: int) -> int { return heap_get_u8(base + i); }
fn main() {
    buf raw[RAW_BYTES];
    heap_set_u8(raw + 2, 40);
    exit(get(raw, 2) + m.mode + RAW_BYTES);
}
"#;
    assert_eq!(run(structured).0, VmRunResult::Exited(53));
    assert_eq!(run(hand_written).0, VmRunResult::Exited(53));
    let structured_asm = compile_to_asm("structured.koto", structured).unwrap();
    let hand_asm = compile_to_asm("hand.koto", hand_written).unwrap();
    let header = |asm: &str, directive: &str| -> String {
        asm.lines()
            .find(|line| line.starts_with(directive))
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(header(&structured_asm, ".heap"), header(&hand_asm, ".heap"));
    assert_eq!(
        header(&structured_asm, ".stack"),
        header(&hand_asm, ".stack")
    );
}

#[test]
fn len_of_buffer_fields_folds_with_focused_diagnostics_for_invalid_operands() {
    // Folds to the capacity with no heap read or extra local slots.
    let source = "struct S { raw: buf[384], } static s: S = {}; fn main() { exit(len(s.raw)); }";
    assert_eq!(run(source).0, VmRunResult::Exited(384));
    let asm = compile_to_asm("len.koto", source).unwrap();
    assert!(!asm.contains("load"), "{asm}");
    assert!(!asm.contains("store_local"), "{asm}");

    for (source, expected) in [
        (
            "struct S { mode: int, } static s: S = { mode: 1, }; fn main() { exit(len(s.mode)); }",
            "is a 32-bit scalar field, not a buffer field",
        ),
        (
            "struct S { raw: buf[8], } static s: S = {}; fn main() { exit(len(s.missing)); }",
            "unknown field `S.missing`",
        ),
        (
            "fn main() { let n = 4; exit(len(n.raw)); }",
            "field receiver must be a struct reference",
        ),
    ] {
        let error = compile("reject.koto", source).unwrap_err();
        assert!(error.message.contains(expected), "{}: {}", expected, error);
    }
}

#[test]
fn buffer_field_assignment_and_initializers_are_rejected() {
    for (source, expected) in [
        (
            "struct S { raw: buf[8], } static s: S = {}; fn main() { s.raw = 1; }",
            "buffer field `raw` names a fixed region and cannot be assigned",
        ),
        (
            "struct S { raw: buf[8], } static s: S = { raw: 1, }; fn main() {}",
            "buffer field `S.raw` cannot take an initializer; buffer regions are zero-initialized",
        ),
    ] {
        let error = compile("reject.koto", source).unwrap_err();
        assert!(error.message.contains(expected), "{}: {}", expected, error);
    }
}

#[test]
fn buffer_field_regions_are_zero_initialized_and_skip_the_rodata_image() {
    // A buffer field *between* initialized bytes leaves a zero run inside the
    // image span; a trailing buffer extends the heap request but not the image.
    let source = r#"
struct A { head: int, region: buf[64], tail: int, }
static a: A = { head: 1, tail: 2, };
struct B { value: int, tail_region: buf[128], }
static b: B = { value: 9, };
fn main() {
    heap_set_u8(a.region + 5, 7);
    exit(heap_get_u8(a.region) + heap_get_u8(a.region + 5) + heap_get_u8(a.region + 63)
        + heap_get_u8(b.tail_region + 127) + a.head + a.tail + b.value);
}
"#;
    // region and tail_region read 0 except the one runtime write (7), so the
    // sum is 7 + head(1) + tail(2) + value(9).
    assert_eq!(run(source).0, VmRunResult::Exited(19));
    let bytecode = compile("zero.koto", source).unwrap();
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let (start, end) = program.rodata_range().expect("static rodata");
    // a: head 0..4, region 4..68 (zero run), tail 68..72; b: value 72..76,
    // tail_region 76..204 contributes no image bytes.
    assert_eq!(end - start, 76);
    let image = &bytecode[start..end];
    assert_eq!(&image[0..4], &1i32.to_le_bytes());
    assert!(image[4..68].iter().all(|&byte| byte == 0));
    assert_eq!(&image[68..72], &2i32.to_le_bytes());
    assert_eq!(&image[72..76], &9i32.to_le_bytes());
    assert!(program.header().max_heap_bytes >= 204);
}

// ---- KOTO-0236: compile-time asset sizes and SDK storage capacities -------

/// Hermetic `asset_len` resolver keyed by package asset path (the manifest
/// `assets` output namespace), mirroring how tests inject include loading.
struct MapAssets {
    sizes: std::collections::HashMap<&'static str, Result<u64, &'static str>>,
    bytes: std::collections::HashMap<&'static str, Result<&'static [u8], &'static str>>,
}

impl AssetResolver for MapAssets {
    fn asset_len(&mut self, path: &str) -> Result<u64, String> {
        match self.sizes.get(path) {
            Some(Ok(size)) => Ok(*size),
            Some(Err(message)) => Err((*message).to_string()),
            None => Err(format!(
                "\"{path}\" is not declared as an `assets` output in app.json"
            )),
        }
    }

    fn asset_bytes(&mut self, path: &str) -> Result<Vec<u8>, String> {
        match self.bytes.get(path) {
            Some(Ok(bytes)) => Ok(bytes.to_vec()),
            Some(Err(message)) => Err((*message).to_string()),
            None => Err(format!(
                "\"{path}\" is not declared as an `assets` output in app.json"
            )),
        }
    }
}

fn assets(entries: &[(&'static str, Result<u64, &'static str>)]) -> MapAssets {
    MapAssets {
        sizes: entries.iter().cloned().collect(),
        bytes: std::collections::HashMap::new(),
    }
}

fn text_assets(entries: &[(&'static str, Result<&'static [u8], &'static str>)]) -> MapAssets {
    MapAssets {
        sizes: entries
            .iter()
            .map(|(path, result)| (*path, result.map(|bytes| bytes.len() as u64)))
            .collect(),
        bytes: entries.iter().cloned().collect(),
    }
}

fn compile_with_assets(
    file: &str,
    source: &str,
    assets: &mut MapAssets,
) -> Result<Vec<u8>, CompileError> {
    compile_with_resolvers(
        file,
        source,
        CodegenOptions::default(),
        &mut loader(&[]),
        assets,
    )
}

#[test]
fn asset_text_line_count_folds_matching_assets() {
    let mut table = text_assets(&[
        ("locales/en-US.txt", Ok(&b"one\ntwo\nthree\n"[..])),
        ("locales/ja-JP.txt", Ok("一\n二\n三\n".as_bytes())),
    ]);
    let source = r#"
const LINES = asset_text_line_count("locales/en-US.txt", "locales/ja-JP.txt");
fn main() {
    if LINES != 3 { exit(10); }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("text_lines.koto", source, &mut table)
        .expect("asset_text_line_count folds matching text assets");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn text_asset_helpers_fold_ranges_at_all_declaration_sites() {
    let mut table = text_assets(&[
        ("locales/a.txt", Ok(&b"a\n0123456789\nz\n"[..])),
        ("locales/b.txt", Ok("あいう\nb\nかき\n".as_bytes())),
        ("locales/c.txt", Ok(&b"x\r\n\r\ny"[..])),
    ]);
    let source = r#"
const LINES = asset_text_line_count("locales/a.txt", "locales/b.txt", "locales/c.txt");
const RANGE = asset_text_max_range_bytes(0, LINES, "locales/a.txt", "locales/b.txt", "locales/c.txt");
const ROWS = ui_list_rows_capacity(LINES, asset_text_max_range_bytes(0, 3, "locales/a.txt", "locales/b.txt"));
struct Storage {
    line_slots: buf[asset_text_line_count("locales/a.txt", "locales/b.txt")],
    labels: buf[asset_text_max_range_bytes(0, 3, "locales/a.txt", "locales/b.txt")],
}
static storage: Storage = {};
fn main() {
    buf labels[asset_text_max_range_bytes(0, 3, "locales/a.txt", "locales/b.txt")];
    if LINES != 3 || RANGE != 16 || ROWS != 52 { exit(10); }
    if len(storage.line_slots) != 3 || len(storage.labels) != 16 || len(labels) != 16 {
        exit(11);
    }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("text_ranges.koto", source, &mut table)
        .expect("text asset helpers fold at every declaration site");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn text_asset_helpers_match_text_resource_line_boundaries() {
    for (bytes, expected_lines, expected_bytes) in [
        (&b""[..], 0, 0),
        (&b"a"[..], 1, 1),
        (&b"a\n"[..], 1, 1),
        (&b"a\n\nb"[..], 3, 2),
        (&b"a\r\n\r\nb\r\n"[..], 3, 2),
        ("日\n本".as_bytes(), 2, 6),
    ] {
        let mut table = text_assets(&[("text.txt", Ok(bytes))]);
        let source = format!(
            "const L = asset_text_line_count(\"text.txt\"); \
             fn main() {{ if L != {expected_lines} {{ exit(10); }} exit(0); }}"
        );
        compile_with_assets("line_boundaries.koto", &source, &mut table)
            .expect("line count follows TextResource::parse boundaries");

        if expected_lines > 0 {
            let mut table = text_assets(&[("text.txt", Ok(bytes))]);
            let source = format!(
                "const B = asset_text_max_range_bytes(0, {expected_lines}, \"text.txt\"); \
                 fn main() {{ if B != {expected_bytes} {{ exit(10); }} exit(0); }}"
            );
            compile_with_assets("range_boundaries.koto", &source, &mut table)
                .expect("range bytes exclude line delimiters");
        }
    }
}

#[test]
fn text_asset_helper_diagnostics_cover_shape_content_and_usage_errors() {
    let entries: &[(&'static str, Result<&'static [u8], &'static str>)] = &[
        ("two.txt", Ok(&b"a\nb\n"[..])),
        ("three.txt", Ok(&b"a\nb\nc\n"[..])),
        ("bare-cr.txt", Ok(&b"a\rb"[..])),
        ("invalid.txt", Ok(&b"\xff"[..])),
        ("unreadable.txt", Err("cannot read asset source unreadable.txt")),
        (
            "tiles.kim",
            Err("\"tiles.kim\" is a pipeline-transformed package asset; text asset helpers inspect verbatim `assets` entries only"),
        ),
    ];
    for (source, expected) in [
        (
            r#"const N = asset_text_line_count("two.txt", "three.txt"); fn main() {}"#,
            "three.txt\" has 3 lines; expected 2",
        ),
        (
            "const N = asset_text_line_count(); fn main() {}",
            "takes at least one package asset path",
        ),
        (
            "const N = asset_text_line_count(1); fn main() {}",
            "asset arguments must be string-literal",
        ),
        (
            r#"const N = asset_text_max_range_bytes(0, 0, "two.txt"); fn main() {}"#,
            "positive line count",
        ),
        (
            r#"const N = asset_text_max_range_bytes(0, -1, "two.txt"); fn main() {}"#,
            "positive line count",
        ),
        (
            "const N = asset_text_max_range_bytes(0, 1); fn main() {}",
            "takes at least one package asset path",
        ),
        (
            r#"const N = asset_text_max_range_bytes(-1, 1, "two.txt"); fn main() {}"#,
            "non-negative first line",
        ),
        (
            r#"const N = asset_text_max_range_bytes(2147483647, 1, "two.txt"); fn main() {}"#,
            "32-bit integer domain",
        ),
        (
            r#"const N = asset_text_max_range_bytes(1, 2, "two.txt"); fn main() {}"#,
            "range 1..3 exceeds its 2 lines",
        ),
        (
            r#"const N = asset_text_line_count("bare-cr.txt"); fn main() {}"#,
            "bare CR line ending",
        ),
        (
            r#"const N = asset_text_line_count("invalid.txt"); fn main() {}"#,
            "not valid UTF-8",
        ),
        (
            r#"const N = asset_text_line_count("missing.txt"); fn main() {}"#,
            "not declared as an `assets` output",
        ),
        (
            r#"const N = asset_text_line_count("unreadable.txt"); fn main() {}"#,
            "cannot read asset source unreadable.txt",
        ),
        (
            r#"const N = asset_text_line_count("tiles.kim"); fn main() {}"#,
            "pipeline-transformed package asset",
        ),
        (
            r#"fn main() { exit(asset_text_line_count("two.txt")); }"#,
            "`asset_text_line_count` is compile-time only",
        ),
        (
            r#"fn main() { let n = asset_text_max_range_bytes(0, 1, "two.txt"); exit(n); }"#,
            "`asset_text_max_range_bytes` is compile-time only",
        ),
        (
            r#"const N = asset_text_max_line_bytes(0, 0, "two.txt"); fn main() {}"#,
            "positive line count",
        ),
        (
            r#"const N = asset_text_max_line_bytes(0, -1, "two.txt"); fn main() {}"#,
            "positive line count",
        ),
        (
            r#"const N = asset_text_max_line_bytes(-1, 1, "two.txt"); fn main() {}"#,
            "non-negative first line",
        ),
        (
            r#"const N = asset_text_max_line_bytes(2147483647, 1, "two.txt"); fn main() {}"#,
            "32-bit integer domain",
        ),
        (
            "const N = asset_text_max_line_bytes(0, 1); fn main() {}",
            "takes at least one package asset path",
        ),
        (
            r#"const N = asset_text_max_line_bytes(1, 2, "two.txt"); fn main() {}"#,
            "range 1..3 exceeds its 2 lines",
        ),
        (
            r#"const N = asset_text_max_line_bytes(0, 1, "bare-cr.txt"); fn main() {}"#,
            "bare CR line ending",
        ),
        (
            r#"const N = asset_text_max_line_bytes(0, 1, "invalid.txt"); fn main() {}"#,
            "not valid UTF-8",
        ),
        (
            r#"const N = asset_text_max_line_bytes(0, 1, "missing.txt"); fn main() {}"#,
            "not declared as an `assets` output",
        ),
        (
            r#"fn main() { exit(asset_text_max_line_bytes(0, 1, "two.txt")); }"#,
            "`asset_text_max_line_bytes` is compile-time only",
        ),
    ] {
        let mut table = text_assets(entries);
        let error =
            compile_with_assets("invalid_text_asset.koto", source, &mut table).expect_err(source);
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

// ---- KOTO-0238: line maxima and additive compile-time sizing --------------

#[test]
fn asset_text_max_line_bytes_folds_the_single_line_maximum_across_assets() {
    // The winning single line (5 bytes, b.txt) must differ from the range-sum
    // maximum (8 bytes, won by a.txt) so the two range helpers cannot be
    // conflated, and the per-asset longest lines live in different assets.
    // c.txt exercises CRLF, an internal empty line, and non-ASCII UTF-8.
    let mut table = text_assets(&[
        ("locales/a.txt", Ok(&b"aaaa\nbb\ncc\n"[..])),
        ("locales/b.txt", Ok(&b"a\nbbbbb\nc\n"[..])),
        ("locales/c.txt", Ok("dd\r\n\r\néé".as_bytes())),
    ]);
    let source = r#"
const LINE_MAX = asset_text_max_line_bytes(
    0, 3, "locales/a.txt", "locales/b.txt", "locales/c.txt");
const RANGE_MAX = asset_text_max_range_bytes(
    0, 3, "locales/a.txt", "locales/b.txt", "locales/c.txt");
const TAIL = asset_text_max_line_bytes(
    2, 1, "locales/a.txt", "locales/b.txt", "locales/c.txt");
const ROWS = ui_list_rows_capacity(3,
    asset_text_max_line_bytes(0, 3, "locales/a.txt", "locales/b.txt"));
struct Storage {
    slot: buf[asset_text_max_line_bytes(0, 3, "locales/a.txt", "locales/b.txt")],
}
static storage: Storage = {};
fn main() {
    buf slot[asset_text_max_line_bytes(1, 2, "locales/b.txt")];
    if LINE_MAX != 5 || RANGE_MAX != 8 || TAIL != 4 { exit(10); }
    if ROWS != 41 { exit(11); }
    if len(storage.slot) != 5 || len(slot) != 5 { exit(12); }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("line_maxima.koto", source, &mut table)
        .expect("asset_text_max_line_bytes folds at every declaration site");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn additive_chains_fold_in_consts_buffers_fields_and_helper_arguments() {
    let mut table = text_assets(&[("locales/en.txt", Ok(&b"title\nokay\n"[..]))]);
    let source = r#"
const TITLE_BYTES = asset_text_max_line_bytes(0, 1, "locales/en.txt");
const DOC_BYTES = 64;
const ARENA = TITLE_BYTES + DOC_BYTES - 4;
struct S { arena: buf[TITLE_BYTES + DOC_BYTES], }
static s: S = {};
fn main() {
    buf tmp[DOC_BYTES + 4];
    buf packet[ui_update_capacity(2, TITLE_BYTES + DOC_BYTES)];
    buf same[ui_update_capacity(2, 69)];
    if ARENA != 65 { exit(10); }
    if len(s.arena) != 69 || len(tmp) != 68 { exit(11); }
    if len(packet) != len(same) { exit(12); }
    let runtime = ARENA + DOC_BYTES;
    if runtime - 4 != 125 { exit(13); }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("additive.koto", source, &mut table)
        .expect("additive chains fold at every compile-time integer position");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn additive_chain_diagnostics_check_domain_and_positive_buffer_sizes() {
    for (source, expected) in [
        (
            "const A = 2147483647 + 1; fn main() {}",
            "32-bit integer domain",
        ),
        (
            "const A = 0 - 2147483647 - 2; fn main() {}",
            "32-bit integer domain",
        ),
        (
            "const A = 3000000000 + 1; fn main() {}",
            "32-bit integer domain",
        ),
        (
            "fn main() { buf b[4 - 4]; }",
            "buffer size folded to 0; buffer sizes must be positive",
        ),
        (
            "fn main() { buf b[2 - 4]; }",
            "buffer size folded to -2; buffer sizes must be positive",
        ),
        (
            "struct S { r: buf[8 - 8], } static s: S = {}; fn main() {}",
            "buffer size folded to 0; buffer sizes must be positive",
        ),
        // A runtime value never spills into a compile-time chain.
        (
            "fn main() { let x = 4; buf b[4 + x]; }",
            "capacity helper arguments must be",
        ),
        (
            "const A = 1 + missing; fn main() {}",
            "capacity helper arguments must be",
        ),
        (
            "fn main() { buf b[ui_update_capacity(1, 5 - 900)]; }",
            "exceed the KotoUI v1 packet capacities",
        ),
    ] {
        let error = compile("additive_reject.koto", source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn koto_0237_gallery_apply_packet_example_folds_as_written() {
    // The KOTO-0237 Gallery apply-packet sketch: component text plus the
    // encoded List rows compose through `+` in the helper argument.
    let mut table = text_assets(&[
        (
            "locales/en-US.txt",
            Ok(&b"aaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\naaaa\n"[..]),
        ),
        (
            "locales/ja-JP.txt",
            Ok(
                "もも\nもも\nもも\nもも\nもも\nもも\nもも\nもも\nもも\nもも\nもも\nもも\n"
                    .as_bytes(),
            ),
        ),
    ]);
    let source = r#"
const GALLERY_LIST_ROWS = 3;
const GALLERY_APPLY_TEXT_BYTES = asset_text_max_range_bytes(
    0, 9, "locales/en-US.txt", "locales/ja-JP.txt");
const GALLERY_LIST_BYTES = ui_list_rows_capacity(GALLERY_LIST_ROWS,
    asset_text_max_range_bytes(9, GALLERY_LIST_ROWS, "locales/en-US.txt", "locales/ja-JP.txt"));
fn main() {
    buf update[ui_update_capacity(10, GALLERY_APPLY_TEXT_BYTES + GALLERY_LIST_BYTES)];
    buf same[ui_update_capacity(10, 108)];
    if GALLERY_APPLY_TEXT_BYTES != 54 || GALLERY_LIST_BYTES != 54 { exit(10); }
    if len(update) != len(same) { exit(11); }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("gallery_apply.koto", source, &mut table)
        .expect("the KOTO-0237 Gallery apply example folds as written");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn asset_len_folds_in_consts_buffers_fields_and_helper_arguments() {
    let mut table = assets(&[("locales/en-US.txt", Ok(40)), ("locales/ja-JP.txt", Ok(50))]);
    let source = r#"
const RAW_BYTES = asset_len("locales/en-US.txt", "locales/ja-JP.txt");
const LINES = 2;
const TABLE_BYTES = ui_text_resource_capacity(LINES, asset_len("locales/en-US.txt", "locales/ja-JP.txt"));
struct Storage {
    raw: buf[RAW_BYTES],
    table: buf[TABLE_BYTES],
    rows: buf[ui_list_rows_capacity(3, 30)],
}
static storage: Storage = {};
fn main() {
    buf single[asset_len("locales/en-US.txt")];
    if RAW_BYTES != 50 || TABLE_BYTES != 58 { exit(10); }
    if len(single) != 40 { exit(11); }
    if len(storage.raw) != 50 || len(storage.table) != 58 || len(storage.rows) != 66 {
        exit(12);
    }
    exit(0);
}
"#;
    let bytecode = compile_with_assets("asset_len.koto", source, &mut table)
        .expect("asset_len folds at every declaration site");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );
}

#[test]
fn asset_size_changes_refold_capacities_on_the_next_build() {
    // The acceptance loop for a growing translation: the same source folds a
    // different storage size when the loader-provided asset bytes change.
    let source = r#"
const RAW_BYTES = asset_len("locales/en-US.txt");
struct S { raw: buf[RAW_BYTES], }
static s: S = {};
fn main() { exit(len(s.raw)); }
"#;
    let heap_bytes = |size: u64| {
        let mut table = assets(&[("locales/en-US.txt", Ok(size))]);
        let bytecode = compile_with_assets("refold.koto", source, &mut table).unwrap();
        let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
        program.header().max_heap_bytes
    };
    let small = heap_bytes(100);
    let grown = heap_bytes(228);
    assert_eq!(
        grown,
        small + 128,
        "exact sizing must track the asset bytes"
    );
}

#[test]
fn asset_len_diagnostics_name_the_path_and_reject_non_compile_time_positions() {
    let table: &[(&'static str, Result<u64, &'static str>)] = &[
        ("locales/en-US.txt", Ok(40)),
        ("empty.bin", Ok(0)),
        (
            "images/tiles.kim",
            Err("\"images/tiles.kim\" is a pipeline-transformed package asset; `asset_len` folds verbatim `assets` entries only"),
        ),
    ];
    for (source, expected) in [
        (
            r#"const B = asset_len("locales/nope.txt"); fn main() {}"#,
            "asset_len: \"locales/nope.txt\" is not declared as an `assets` output",
        ),
        (
            r#"const B = asset_len("images/tiles.kim"); fn main() {}"#,
            "pipeline-transformed package asset",
        ),
        (
            r#"const B = asset_len(); fn main() {}"#,
            "`asset_len` takes at least one package asset path",
        ),
        (
            r#"const B = asset_len(4); fn main() {}"#,
            "`asset_len` arguments must be string-literal package asset paths",
        ),
        (
            r#"const P = 1; const B = asset_len(P); fn main() {}"#,
            "`asset_len` arguments must be string-literal package asset paths",
        ),
        (
            r#"fn main() { buf b[asset_len("empty.bin")]; }"#,
            "`asset_len` folded to 0 bytes; buffer sizes must be positive",
        ),
        (
            r#"fn main() { exit(asset_len("locales/en-US.txt")); }"#,
            "`asset_len` is compile-time only",
        ),
        (
            r#"fn main() { let s = asset_len("locales/en-US.txt"); exit(s); }"#,
            "`asset_len` is compile-time only",
        ),
    ] {
        let mut table = assets(table);
        let error = compile_with_assets("reject.koto", source, &mut table).expect_err(source);
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn storage_capacity_helpers_fold_with_boundary_diagnostics_at_all_sites() {
    // Folded forms agree with the checked SDK runtime forms (KOTO-0232 model).
    let source = r#"
        include "koto_ui.koto";
        const TABLE = ui_text_resource_capacity(22, 343);
        const ROWS = ui_list_rows_capacity(3, 30);
        fn main() {
            buf table[TABLE];
            buf rows[ui_list_rows_capacity(3, 30)];
            if TABLE != 431 || ROWS != 66 { exit(10); }
            if len(table) != 431 || len(rows) != 66 { exit(11); }
            if ui_text_resource_capacity(22, 343) != 431 || ui_list_rows_capacity(3, 30) != 66 {
                exit(12);
            }
            if ui_text_resource_capacity(0, 0) != -2 ||
               ui_text_resource_capacity(16384, 0) != -2 ||
               ui_text_resource_capacity(1, 65532) != -2 ||
               ui_list_rows_capacity(0, 0) != -2 ||
               ui_list_rows_capacity(33, 0) != -2 ||
               ui_list_rows_capacity(1, 65524) != -2 {
                exit(13);
            }
            exit(0);
        }
    "#;
    let bytecode = compile_with_loader(
        "storage_helpers.koto",
        source,
        CodegenOptions::default(),
        &mut sdk_loader(),
    )
    .expect("storage helpers fold and their runtime forms check");
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default()).unwrap();
    let mut vm = BytecodeVm::<16, 4>::new(&program).unwrap();
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    let mut host = CaptureHost::default();
    assert_eq!(
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .unwrap(),
        VmRunResult::Exited(0)
    );

    for (source, expected) in [
        (
            "const BAD = ui_text_resource_capacity(0, 0); fn main() {}",
            "ui_text_resource_capacity arguments exceed the KotoUI v1 text resource capacities",
        ),
        (
            "const BAD = ui_text_resource_capacity(16384, 0); fn main() {}",
            "text resource capacities",
        ),
        (
            "const BAD = ui_text_resource_capacity(1, 65532); fn main() {}",
            "text resource capacities",
        ),
        (
            "fn main() { buf b[ui_list_rows_capacity(0, 0)]; }",
            "ui_list_rows_capacity arguments exceed the KotoUI v1 list rows capacities",
        ),
        (
            "struct S { r: buf[ui_list_rows_capacity(33, 0)], } static s: S = {}; fn main() {}",
            "list rows capacities",
        ),
        (
            "const BAD = ui_list_rows_capacity(1, 65524); fn main() {}",
            "list rows capacities",
        ),
    ] {
        let error = compile("invalid_storage_capacity.koto", source).expect_err(source);
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn manifest_assets_discover_the_nearest_app_json_with_focused_errors() {
    let dir = std::env::temp_dir().join("koto0236_manifest_assets");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(dir.join("locales").join("en-US.txt"), b"12345").unwrap();
    std::fs::write(
        dir.join("app.json"),
        concat!(
            r#"{"assets":[{"source":"locales/en-US.txt"},"#,
            r#"{"source":"locales/missing.txt","output":"locales/missing.txt"}],"#,
            r#""images":[{"source":"tiles.kspr","output":"sprites/tiles.kim"}]}"#,
        ),
    )
    .unwrap();
    let root = dir.join("src").join("main.koto");
    let mut assets = ManifestAssets::for_root(root.to_str().unwrap());

    // `output` defaults to `source` exactly as the app build loop treats it.
    assert_eq!(assets.asset_len("locales/en-US.txt").unwrap(), 5);
    assert_eq!(assets.asset_bytes("locales/en-US.txt").unwrap(), b"12345");
    std::fs::remove_file(dir.join("locales").join("en-US.txt")).unwrap();
    assert_eq!(
        assets.asset_bytes("locales/en-US.txt").unwrap(),
        b"12345",
        "text bytes are read once and cached for the compilation"
    );
    let unreadable = assets.asset_len("locales/missing.txt").unwrap_err();
    assert!(
        unreadable.contains("cannot read asset source"),
        "{unreadable}"
    );
    let transformed = assets.asset_len("sprites/tiles.kim").unwrap_err();
    assert!(
        transformed.contains("pipeline-transformed package asset"),
        "{transformed}"
    );
    let undeclared = assets.asset_len("locales/nope.txt").unwrap_err();
    assert!(
        undeclared.contains("not declared as an `assets` output"),
        "{undeclared}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
