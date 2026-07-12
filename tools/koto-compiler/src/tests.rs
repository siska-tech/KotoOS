use super::*;
use koto_core::runtime::VmBudget;
use koto_core::{
    BytecodeVm, HostCallOutcome, RuntimeLimits, VerifiedProgram, VmHost, VmInputSnapshot,
    VmRunResult,
};

#[derive(Default)]
struct CaptureHost {
    text: Vec<(i32, i32, String)>,
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
    // IME_COMMIT == 3, INTENT_EXIT == 1<<14 == 16384.
    let asm = compile_to_asm(
        "sdk.koto",
        "
        fn main() {
            ime_feed_key(IME_COMMIT, 0);
            let q = INTENT_EXIT;
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
