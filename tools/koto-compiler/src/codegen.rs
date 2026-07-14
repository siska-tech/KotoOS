//! Semantic analysis and code generation for the Koto app language.
//!
//! The KOTO-0033 verifier performs a single linear pass over the code words with
//! one running operand-stack depth; it is not control-flow aware. To stay within
//! that contract the compiler:
//!
//! - **Inlines every function** at its call site instead of emitting `call`/`ret`
//!   (the VM also shares a single 16-slot local file, so a real call stack of
//!   locals is impossible). Recursion is rejected up front.
//! - Emits **branchless comparisons and logical operators** so branch joins never
//!   make the linear depth disagree with the real depth.
//! - Routes `return` through a reserved return-value slot, so the (unreachable but
//!   linearly-scanned) code after a `return` keeps a consistent depth.
//! - Shapes host-call result handling to the verifier's success-shape model.
//!
//! Output is `kbc-asm` assembly text; `kbc_asm::assemble` produces the `KBC1`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::parser::{BinOp, DataWidth, Expr, Program, Stmt, Type, UnOp};
use crate::Diag;

/// The compiler reserves three codegen scratch local slots: a return-value slot
/// (`return` routing and the value-returning inline fallthrough) and two operand
/// slots for the `%` lowering. They are never user-visible.
///
/// Rather than pinning them at fixed top-of-file indices, the scratch slots *float*
/// just above the program's user-slot high-water mark (`max_slot`): the emitter
/// writes the placeholder operands below and [`Codegen::finish`] resolves them to
/// real indices once codegen has measured the peak (KOTO-0146). The VM's
/// `local_slots_peak` tracks the highest slot *index* touched (not a live count), so
/// floating the scratch region keeps the reported peak proportional to actual local
/// pressure instead of pinning every app that uses `%` or a return value to the top
/// of the file. User locals still occupy `0..USER_LOCAL_SLOTS`; the scratch slots
/// sit at `max_slot..max_slot + SCRATCH_SLOTS`, which never exceeds the file because
/// `alloc_local`/`emit_inline` cap `max_slot` at `USER_LOCAL_SLOTS`.
const SCRATCH_SLOTS: usize = 3;
const USER_LOCAL_SLOTS: usize = koto_core::runtime::VM_LOCAL_SLOTS - SCRATCH_SLOTS;
const SCRATCH_RET: &str = "@scratch_ret";
const SCRATCH_S_A: &str = "@scratch_a";
const SCRATCH_S_B: &str = "@scratch_b";

/// Minimum header requests so compiled apps load on the simulator VM profile.
/// `MIN_STACK` / `CALL_PROFILE` derive from the canonical profile
/// (`RuntimeLimits::simulator_default`, KOTO-0060): the VM provides exactly these
/// many fixed slots and rejects an app whose header requests fewer, so the compiler
/// floors those requests at them. Heap is different: it is sized per app (KOTO-0096),
/// so the header carries the app's *actual* need (buffers + string data) and is only
/// floored at a tiny minimum; `verify_kbc` enforces the device heap ceiling
/// (`RuntimeLimits::max_heap_bytes`).
const MIN_STACK: u16 = koto_core::runtime::RuntimeLimits::simulator_default().max_stack_slots;
const CALL_PROFILE: u16 = koto_core::runtime::RuntimeLimits::simulator_default().max_call_depth;
const HEAP_FLOOR: u32 = 64;
const MAX_STACK: usize = 256;
const ACTOR_SIZE: u32 = 12;

mod actor_field {
    pub const X: u32 = 0;
    pub const Y: u32 = 2;
    pub const VX: u32 = 4;
    pub const VY: u32 = 6;
    pub const STATE: u32 = 8;
    pub const FRAME: u32 = 9;
    pub const TIMER: u32 = 10;
}

#[derive(Clone, Copy)]
enum ResultKind {
    /// Terminates the app; produces no value.
    Exit,
    /// Status-only host call: result is the status word (0 on success).
    Status,
    /// `Ok1` host call: result is the value, or `-1` on failure.
    Value,
    /// `Ok2` host call: result is the first/second value, or `-1` on failure.
    Value2First,
    Value2Second,
}

struct Intrinsic {
    /// The `kbc-asm` `host_call` mnemonic to emit (the assembler resolves it to a
    /// numeric host-call ID). May differ from the Koto name for aliases that
    /// project a single host call to one of its result values.
    asm: &'static str,
    args: usize,
    result: ResultKind,
}

/// Built-in host-call wrappers. KOTO-0047 formalizes these as the documented SDK
/// prelude with named constants.
fn intrinsic(name: &str) -> Option<Intrinsic> {
    let (asm, args, result) = match name {
        "exit" => ("exit", 1, ResultKind::Exit),
        "yield_frame" => ("yield_frame", 0, ResultKind::Status),
        "draw_rect" => ("draw_rect", 5, ResultKind::Status),
        "draw_text" => ("draw_text", 4, ResultKind::Status),
        "draw_text_color" => ("draw_text_color", 5, ResultKind::Status),
        "draw_pixels" => ("draw_pixels_rgb565", 6, ResultKind::Status),
        "draw_pixels_persistent" => ("draw_pixels_persistent_rgb565", 6, ResultKind::Status),
        "game2d_set_tile" => ("game2d_set_tile", 4, ResultKind::Status),
        "game2d_clear_layer" => ("game2d_clear_layer", 1, ResultKind::Status),
        "game2d_configure_tilemap" => ("game2d_configure_tilemap", 5, ResultKind::Status),
        "game2d_present" => ("game2d_present", 0, ResultKind::Status),
        "game2d_static_begin" => ("game2d_static_begin", 0, ResultKind::Status),
        "game2d_static_end" => ("game2d_static_end", 0, ResultKind::Status),
        "game2d_stamp_define" => ("game2d_stamp_define", 4, ResultKind::Status),
        "game2d_sprite_set" => ("game2d_sprite_set", 5, ResultKind::Status),
        "game2d_sprite_hide" => ("game2d_sprite_hide", 1, ResultKind::Status),
        "game2d_sprite_clear_all" => ("game2d_sprite_clear_all", 0, ResultKind::Status),
        "game2d_text_set" => ("game2d_text_set", 6, ResultKind::Status),
        "game2d_text_hide" => ("game2d_text_hide", 1, ResultKind::Status),
        "game2d_text_clear_all" => ("game2d_text_clear_all", 0, ResultKind::Status),
        "audio_submit" => ("audio_submit_i16", 3, ResultKind::Value),
        "play_sfx" => ("play_sfx", 1, ResultKind::Status),
        "play_bgm_asset" => ("play_bgm_asset", 2, ResultKind::Status),
        "play_sfx_asset" => ("play_sfx_asset", 2, ResultKind::Status),
        "stop_bgm" => ("stop_bgm", 0, ResultKind::Status),
        "asset_load_range" => ("asset_load_range", 5, ResultKind::Value),
        "input_held" => ("input_snapshot", 0, ResultKind::Value2First),
        "input_pressed" => ("input_snapshot", 0, ResultKind::Value2Second),
        "text_input" => ("text_input", 0, ResultKind::Value2First),
        "text_intent" => ("text_input", 0, ResultKind::Value2Second),
        "file_open" => ("file_open", 3, ResultKind::Value),
        "file_read" => ("file_read", 3, ResultKind::Value),
        "file_write" => ("file_write", 3, ResultKind::Value),
        "file_close" => ("file_close", 1, ResultKind::Status),
        "asset_load" => ("asset_load", 4, ResultKind::Value),
        "ime_feed_key" => ("ime_feed_key", 2, ResultKind::Status),
        "ime_convert" => ("ime_convert", 0, ResultKind::Status),
        "ime_query_line" => ("ime_query_line", 2, ResultKind::Value),
        "edit_move" => ("edit_move", 1, ResultKind::Status),
        "edit_delete" => ("edit_delete", 1, ResultKind::Value),
        "edit_load" => ("edit_load", 2, ResultKind::Status),
        "edit_query_text" => ("edit_query_text", 2, ResultKind::Value2First),
        "ime_display" => ("ime_display", 2, ResultKind::Value),
        "edit_visible_line" => ("edit_visible_line", 3, ResultKind::Value),
        "edit_cursor_col" => ("edit_cursor_view", 0, ResultKind::Value2First),
        "edit_cursor_row" => ("edit_cursor_view", 0, ResultKind::Value2Second),
        "edit_scroll_row" => ("edit_scroll_row", 0, ResultKind::Value),
        "edit_cell_width" => ("edit_view_metrics", 0, ResultKind::Value2First),
        "edit_cell_height" => ("edit_view_metrics", 0, ResultKind::Value2Second),
        "edit_cursor_status" => ("edit_cursor_status", 2, ResultKind::Value),
        "edit_total_lines" => ("edit_total_lines", 0, ResultKind::Value),
        "edit_wrap" => ("edit_wrap", 0, ResultKind::Value),
        "edit_hscroll" => ("edit_hscroll_view", 0, ResultKind::Value2First),
        "edit_line_cols" => ("edit_hscroll_view", 0, ResultKind::Value2Second),
        "dir_count" => ("dir_list", 3, ResultKind::Value2First),
        "dir_name" => ("dir_list", 3, ResultKind::Value2Second),
        "edit_reserve_rows" => ("edit_reserve_rows", 1, ResultKind::Status),
        "edit_configure" => ("edit_configure", 2, ResultKind::Status),
        _ => return None,
    };
    Some(Intrinsic { asm, args, result })
}

/// Predefined SDK constants exposed to every program. Sourced from the host ABI
/// modules so they cannot drift from the runtime; see `docs/spec/KOTO_SDK.md`.
fn sdk_consts() -> Vec<(&'static str, i64)> {
    use koto_core::runtime::{edit_delete, edit_dir, ime_key, text_intent};
    vec![
        // file_open modes (see koto-sim SimRuntimeHost::file_open)
        ("MODE_READ", 0),
        ("MODE_WRITE", 1),
        ("MODE_READWRITE", 2),
        // ime_feed_key kinds
        ("IME_CHARACTER", ime_key::CHARACTER as i64),
        ("IME_SHIFT", ime_key::SHIFT as i64),
        ("IME_CONVERT", ime_key::CONVERT as i64),
        ("IME_COMMIT", ime_key::COMMIT as i64),
        ("IME_CANCEL", ime_key::CANCEL as i64),
        ("IME_BACKSPACE", ime_key::BACKSPACE as i64),
        ("IME_OTHER", ime_key::OTHER as i64),
        ("IME_TOGGLE", ime_key::TOGGLE as i64),
        // edit_move directions
        ("DIR_LEFT", edit_dir::LEFT as i64),
        ("DIR_RIGHT", edit_dir::RIGHT as i64),
        ("DIR_UP", edit_dir::UP as i64),
        ("DIR_DOWN", edit_dir::DOWN as i64),
        ("DIR_HOME", edit_dir::HOME as i64),
        ("DIR_END", edit_dir::END as i64),
        // edit_delete kinds
        ("DELETE_BACKSPACE", edit_delete::BACKSPACE as i64),
        ("DELETE_FORWARD", edit_delete::FORWARD as i64),
        // text-input intent bits
        ("INTENT_SHIFT", text_intent::SHIFT as i64),
        ("INTENT_CONVERT", text_intent::CONVERT as i64),
        ("INTENT_COMMIT", text_intent::COMMIT as i64),
        ("INTENT_CANCEL", text_intent::CANCEL as i64),
        ("INTENT_BACKSPACE", text_intent::BACKSPACE as i64),
        ("INTENT_DELETE", text_intent::DELETE as i64),
        ("INTENT_LEFT", text_intent::LEFT as i64),
        ("INTENT_RIGHT", text_intent::RIGHT as i64),
        ("INTENT_UP", text_intent::UP as i64),
        ("INTENT_DOWN", text_intent::DOWN as i64),
        ("INTENT_HOME", text_intent::HOME as i64),
        ("INTENT_END", text_intent::END as i64),
        ("INTENT_NEWLINE", text_intent::NEWLINE as i64),
        ("INTENT_SAVE", text_intent::SAVE as i64),
        ("INTENT_EXIT", text_intent::EXIT as i64),
        ("INTENT_IME_TOGGLE", text_intent::IME_TOGGLE as i64),
        ("INTENT_OPEN", text_intent::OPEN as i64),
        ("INTENT_NEW", text_intent::NEW as i64),
        // Audio sound-effect and music ids (host-owned audio service)
    ]
}

struct FnInfo {
    params: Vec<(String, Type)>,
    ret: Option<Type>,
    body: Vec<Stmt>,
}

struct Scope {
    func: String,
    locals: HashMap<String, usize>,
    buffers: HashMap<String, (u32, u32)>,
    next_slot: usize,
    end_label: String,
}

/// Compile a parsed program to assembly text, with the KOTO-0156 code-window layout
/// options. `CodegenOptions::default()` (both off) reproduces the byte-for-byte baseline
/// layout; equivalence tests compile both ways and assert identical runtime behavior.
pub fn compile_to_asm_with(
    file: &str,
    program: &Program,
    options: CodegenOptions,
) -> Result<String, Diag> {
    let mut codegen = Codegen::new(file, program)?;
    codegen.options = options;
    codegen.run()?;
    Ok(codegen.finish())
}

struct Codegen<'a> {
    file: &'a str,
    program: &'a Program,
    consts: HashMap<String, i64>,
    funcs: HashMap<String, FnInfo>,
    buffer_offsets: HashMap<(String, String), (u32, u32)>,
    /// Top-level `data` const buffers, by name → (heap byte offset, byte length).
    /// These live at the bottom of the heap and are initialized from `rodata`
    /// (KOTO-0139), so they resolve as a value (their offset) from any function.
    data_offsets: HashMap<String, (u32, u32)>,
    /// The const heap image: `rodata[i]` becomes `heap[i]` at load. Built from the
    /// `data` declarations in source order; emitted as the `.rodata` directive.
    rodata: Vec<u8>,
    actor_arrays: HashMap<(usize, usize), (u32, u32)>,
    string_offsets: HashMap<Vec<u8>, u32>,
    strings: Vec<(u32, Vec<u8>)>,
    total_heap: u32,
    lines: Vec<String>,
    scopes: Vec<Scope>,
    loops: Vec<(String, String)>,
    label_id: usize,
    depth: i64,
    max_depth: i64,
    /// Highest user local slot reached (`next_slot` high-water) across the whole
    /// program. With call-site scoped inline-slot reuse (KOTO-0104) this is the
    /// post-reuse physical user-slot usage, the primary local-budget metric.
    max_slot: usize,
    /// KOTO-0156 codegen layout options. Both transforms are **off by default**
    /// ([`CodegenOptions::default`]) and opt-in per app (app.json `codegen` block →
    /// build_apps → CLI flags); the shipped default keeps every other app's bytecode
    /// byte-identical. See [`CodegenOptions`] and
    /// docs/devlog/KOTO_KOTOBLOCKS_COLD_BLOCK_OUTLINING.md.
    options: CodegenOptions,
    /// KOTO-0156 #2: cold blocks captured during emission to relocate to the code tail
    /// (`(outline_label, branch_back_label, captured_lines)`). Emitted after `exit` in
    /// [`run`]; the original site becomes a `br outline_label`.
    pending_outlines: Vec<(String, String, Vec<String>)>,
}

/// KOTO-0156 code-window layout options. Both are **off by default**; they are opt-in
/// per app because they only help apps whose per-frame loop crosses a 16 KiB PSRAM
/// code-window tile boundary (KotoBlocks), and enabling them globally would relayout —
/// and could regress — other large apps (kotorun/kotorogue/kotoshogi).
///
/// - `relocate_preamble` (#1): move the one-time string-init preamble to the code tail
///   so the per-frame loop starts near word 0. **Alone it regresses KotoBlocks**: it
///   slides the tile boundary out of cold branched-over code into the hot board-diff
///   loop (steady refills 2 → 442). It only pays off paired with `outline_cold_blocks`.
/// - `outline_cold_blocks` (#2): relocate large, non-fall-through `if` then-blocks (the
///   title-state block) to the code tail via branch-to-tail + branch-back (no CALL/RET),
///   shrinking the per-frame loop span so it fits one tile. Together with #1 the
///   KotoBlocks steady loop fits tile 0 → **0 steady refills**.
///
/// Both are layout-only and behavior-preserving (proven by equivalence tests). See
/// docs/devlog/KOTO_KOTOBLOCKS_COLD_BLOCK_OUTLINING.md.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodegenOptions {
    pub relocate_preamble: bool,
    pub outline_cold_blocks: bool,
    /// KOTO-0169 Stage 4 opt-OUT: emit the pre-Stage-4 boolean/comparison
    /// templates (normalize-both `&&`/`||`, `1 - b` logical not, value-form
    /// conditions with no branch-sense inversion). The compact templates are
    /// the default because they shrink every app's per-frame instruction
    /// count; this escape hatch exists for apps whose PSRAM code-window tile
    /// layout regresses under the smaller code (the KOTO-0156 lesson —
    /// kotosnake/kotoshogi ping-ponged when their hot loop slid across a tile
    /// boundary), pinning them to the exact pre-Stage-4 bytecode instead.
    pub legacy_compare_templates: bool,
}

/// KOTO-0156 #2: minimum emitted instruction count for a cold `if` then-block to be
/// worth relocating out of line. Below this the two relocation branches cost more than
/// the tile-occupancy they free, so small early-out blocks stay inline.
const OUTLINE_MIN_WORDS: usize = 64;

impl<'a> Codegen<'a> {
    fn new(file: &'a str, program: &'a Program) -> Result<Self, Diag> {
        // SDK constants are predefined; a user `const` of the same name overrides.
        let mut consts: HashMap<String, i64> = sdk_consts()
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        for def in &program.consts {
            consts.insert(def.name.clone(), def.value);
        }

        // Function signatures. Functions are inlined and their user-local slots are
        // allocated at the *call site* (KOTO-0104): each inline expansion takes slots
        // above the caller's live locals and releases them when it ends, so disjoint
        // helpers reuse the same physical slots. No fixed per-function block is
        // reserved here; per-scope reuse (KOTO-0092) and the call-site reuse together
        // bound the peak, which `alloc_local` / `emit_inline` enforce against the cap.
        let mut funcs = HashMap::new();
        for function in &program.functions {
            if funcs.contains_key(&function.name) {
                return Err(Diag::new(
                    function.line,
                    function.col,
                    format!("function `{}` is already defined", function.name),
                ));
            }
            funcs.insert(
                function.name.clone(),
                FnInfo {
                    params: function
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty))
                        .collect(),
                    ret: function.ret,
                    body: function.body.clone(),
                },
            );
        }

        let main = funcs
            .get("main")
            .ok_or_else(|| Diag::new(1, 1, "program needs a `fn main()`".to_string()))?;
        if !main.params.is_empty() || main.ret.is_some() {
            return Err(Diag::new(
                1,
                1,
                "`main` must take no parameters and return nothing".to_string(),
            ));
        }

        let mut codegen = Codegen {
            file,
            program,
            consts,
            funcs,
            buffer_offsets: HashMap::new(),
            data_offsets: HashMap::new(),
            rodata: Vec::new(),
            actor_arrays: HashMap::new(),
            string_offsets: HashMap::new(),
            strings: Vec::new(),
            total_heap: 0,
            lines: Vec::new(),
            scopes: Vec::new(),
            loops: Vec::new(),
            label_id: 0,
            depth: 0,
            max_depth: 0,
            max_slot: 0,
            options: CodegenOptions::default(),
            pending_outlines: Vec::new(),
        };
        codegen.reject_recursion()?;
        codegen.assign_heap()?;
        Ok(codegen)
    }

    fn assign_heap(&mut self) -> Result<(), Diag> {
        let mut cursor = 0u32;
        // Mutable `buf`s come first so an app that addresses a buffer by an absolute
        // heap offset across functions (e.g. KotoBlocks' tile cache at offset 0, read
        // by `blit_piece` and the host as `t*512`) keeps that offset. Const `data`
        // sits above them; the heap image (`rodata`) is the heap *prefix* up to the
        // end of the const region (KOTO-0139) — the mutable prefix is a zero fill.
        for function in &self.program.functions {
            collect_buffers(&function.body, &mut |name, size| {
                self.buffer_offsets
                    .insert((function.name.clone(), name.to_string()), (cursor, size));
                cursor += size;
            });
        }
        // Const `data` buffers, in source order. Record each one's bytes at its heap
        // offset, then materialize the rodata image as `heap[0..data_end]` with the
        // mutable region below zero-filled.
        let mut data_blocks: Vec<(u32, Vec<u8>)> = Vec::new();
        for def in &self.program.data {
            if self.data_offsets.contains_key(&def.name) {
                return Err(Diag::new(
                    def.line,
                    def.col,
                    format!("`data {}` is already defined", def.name),
                ));
            }
            let offset = cursor;
            let mut block = Vec::new();
            for &value in &def.values {
                match def.width {
                    DataWidth::U8 => {
                        let byte = u8::try_from(value).map_err(|_| {
                            Diag::new(
                                def.line,
                                def.col,
                                format!("`data {}` value {value} does not fit in u8", def.name),
                            )
                        })?;
                        block.push(byte);
                    }
                    DataWidth::U16 => {
                        let half = u16::try_from(value).map_err(|_| {
                            Diag::new(
                                def.line,
                                def.col,
                                format!("`data {}` value {value} does not fit in u16", def.name),
                            )
                        })?;
                        block.extend_from_slice(&half.to_le_bytes());
                    }
                }
            }
            let bytes = block.len() as u32;
            self.data_offsets.insert(def.name.clone(), (offset, bytes));
            data_blocks.push((offset, block));
            cursor += bytes;
        }
        if !data_blocks.is_empty() {
            let data_end = cursor as usize;
            self.rodata = vec![0u8; data_end];
            for (offset, block) in &data_blocks {
                let start = *offset as usize;
                self.rodata[start..start + block.len()].copy_from_slice(block);
            }
        }
        for function in &self.program.functions {
            collect_actor_arrays(&function.body, &self.consts, &mut |line, col, count| {
                let bytes = count.saturating_mul(ACTOR_SIZE);
                self.actor_arrays.insert((line, col), (cursor, count));
                cursor = cursor.saturating_add(bytes);
            });
        }
        for function in &self.program.functions {
            collect_strings(&function.body, &mut |bytes| {
                if !self.string_offsets.contains_key(bytes) {
                    let offset = cursor;
                    cursor += bytes.len() as u32;
                    self.string_offsets.insert(bytes.to_vec(), offset);
                    self.strings.push((offset, bytes.to_vec()));
                }
            });
        }
        self.total_heap = cursor;
        Ok(())
    }

    fn reject_recursion(&self) -> Result<(), Diag> {
        // DFS over the user-function call graph; any cycle is unsupported.
        let mut visiting = HashSet::new();
        let mut done = HashSet::new();
        for function in &self.program.functions {
            self.visit_calls(&function.name, &mut visiting, &mut done)?;
        }
        Ok(())
    }

    fn visit_calls(
        &self,
        name: &str,
        visiting: &mut HashSet<String>,
        done: &mut HashSet<String>,
    ) -> Result<(), Diag> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return Err(Diag::new(
                1,
                1,
                format!("recursion is not supported (in `{name}`)"),
            ));
        }
        if let Some(info) = self.funcs.get(name) {
            let mut callees = Vec::new();
            collect_calls(&info.body, &mut |callee| callees.push(callee.to_string()));
            for callee in callees {
                if self.funcs.contains_key(&callee) {
                    self.visit_calls(&callee, visiting, done)?;
                }
            }
        }
        visiting.remove(name);
        done.insert(name.to_string());
        Ok(())
    }

    fn run(&mut self) -> Result<(), Diag> {
        self.emit("main:".to_string());
        let strings = self.strings.clone();
        // KOTO-0156: the string-init preamble (`store_str` per string literal) copies
        // every string into the heap and runs exactly *once* at launch. Emitted at the
        // front it pins the per-frame loop body to start after it, pushing the steady
        // hot loop across a 16 KiB PSRAM code-window tile boundary (see
        // docs/devlog/KOTO_KOTOBLOCKS_COLD_BLOCK_OUTLINING.md). Relocating it past the body
        // shifts the whole body toward word 0: at entry we branch once to the
        // tail-placed preamble, which initializes the strings and branches back to the
        // body start. Both branches are one-time launch code, so the steady hot path
        // gains no instruction, and the transform is layout-only — identical opcodes,
        // operands, and string bytes, so behavior and the ABI are unchanged. With no
        // strings there is nothing to relocate and the emission matches the old front
        // layout exactly (and no extra labels are allocated, keeping golden ASM
        // stable for stringless programs).
        let relocate = self.options.relocate_preamble && !strings.is_empty();
        let preamble_labels = if relocate {
            let init_label = self.new_label("init_strings");
            let body_label = self.new_label("body");
            self.emit(format!("br {init_label}"));
            self.emit(format!("{body_label}:"));
            Some((init_label, body_label))
        } else {
            for (offset, bytes) in &strings {
                self.emit(format!("store_str {offset}, \"{}\"", render_string(bytes)));
            }
            None
        };
        // Inline main's body.
        let main_body = self.funcs.get("main").unwrap().body.clone();
        let end_label = self.new_label("main_end");
        self.scopes.push(Scope {
            func: "main".to_string(),
            locals: HashMap::new(),
            buffers: HashMap::new(),
            // main is the root expansion: its locals start at user slot 0.
            next_slot: 0,
            end_label: end_label.clone(),
        });
        self.emit_block(&main_body)?;
        self.scopes.pop();
        self.emit(format!("{end_label}:"));
        self.emit("push_i16 0".to_string());
        self.emit("host_call exit".to_string());
        // Tail: the relocated one-time string-init preamble. Reached only via the
        // entry branch (never by fallthrough past `exit`); it initializes the strings
        // once and branches back to the body start.
        if let Some((init_label, body_label)) = preamble_labels {
            self.emit(format!("{init_label}:"));
            for (offset, bytes) in &strings {
                self.emit(format!("store_str {offset}, \"{}\"", render_string(bytes)));
            }
            self.emit(format!("br {body_label}"));
        }
        // KOTO-0156 #2: emit the relocated cold blocks at the code tail. Each was
        // captured at its `if` site (which now holds `br outline_label`); here we place
        // `outline_label: <then-block> br back_label`. The block is reached only by that
        // forward branch and rejoins the body via the back branch — call-free, no
        // CALL/RET. The operand stack is empty between statements, so the block (and the
        // tail) sit at depth 0 and the linear verifier accepts them; if the block
        // diverges (continue/break/return) the back branch is simply never reached.
        let outlines = std::mem::take(&mut self.pending_outlines);
        for (outline_label, back_label, lines) in outlines {
            self.emit(format!("{outline_label}:"));
            self.lines.extend(lines);
            self.emit(format!("br {back_label}"));
        }
        Ok(())
    }

    fn finish(mut self) -> String {
        // KOTO-0154: conservative, ABI-preserving peephole cleanup of the emitted
        // instruction stream before final assembly. Runs after all depth/slot
        // accounting, so it never affects the header's `.stack` ceiling (it only
        // ever lowers the real peak, which stays within the already-sized header).
        peephole(&mut self.lines);
        let stack = ((self.max_depth + 2).max(i64::from(MIN_STACK)) as usize).min(MAX_STACK) as u16;
        let heap = self.total_heap.max(HEAP_FLOOR);
        let mut header = format!(
            ".debug_file \"{}\"\n.stack {stack}\n.calls {CALL_PROFILE}\n.heap {heap}\n.abi 1 {}\n.entry main\n",
            render_string(self.file.as_bytes()),
            koto_core::HOST_ABI_MINOR
        );
        // The const heap image (KOTO-0139): emit the `data` bytes as a single hex
        // `.rodata` directive. The assembler places it after the code segment and the
        // loader copies it into the bottom of the heap before entry.
        if !self.rodata.is_empty() {
            let mut hex = String::with_capacity(self.rodata.len() * 2);
            for byte in &self.rodata {
                hex.push_str(&format!("{byte:02x}"));
            }
            header.push_str(&format!(".rodata {hex}\n"));
        }
        // Resolve the floating codegen scratch slots: they sit just above the
        // program's user-slot high-water mark so the VM's highest-index local peak
        // tracks real pressure rather than always reaching the top of the file
        // (KOTO-0146). `max_slot <= USER_LOCAL_SLOTS` is enforced at allocation, so
        // the top scratch index always fits the VM local file.
        let scratch_a = self.max_slot;
        let scratch_b = self.max_slot + 1;
        let scratch_ret = self.max_slot + 2;
        debug_assert!(
            scratch_ret < koto_core::runtime::VM_LOCAL_SLOTS,
            "floating scratch slots overflow the VM local file"
        );
        for line in &self.lines {
            if !line.ends_with(':') {
                header.push_str("    ");
            }
            header.push_str(&resolve_scratch(line, scratch_a, scratch_b, scratch_ret));
            header.push('\n');
        }
        header
    }

    // ---- statements ----

    fn emit_block(&mut self, stmts: &[Stmt]) -> Result<(), Diag> {
        // Open a lexical scope: remember which locals are visible and the slot
        // cursor on entry. On exit, drop the block's locals and rewind the cursor
        // so a later disjoint block reuses those slots (KOTO-0092). `buffers` are
        // heap-allocated globally and are intentionally not scoped here.
        let scope = self.scopes.last().unwrap();
        let saved_locals = scope.locals.clone();
        let saved_next_slot = scope.next_slot;
        for stmt in stmts {
            self.emit_stmt(stmt)?;
        }
        let scope = self.scopes.last_mut().unwrap();
        scope.locals = saved_locals;
        scope.next_slot = saved_next_slot;
        Ok(())
    }

    fn emit_stmt(&mut self, stmt: &Stmt) -> Result<(), Diag> {
        match stmt {
            Stmt::Let {
                name,
                value,
                line,
                col,
                ..
            } => {
                self.emit_loc(*line, *col);
                let produced = self.emit_expr(value)?;
                if !produced {
                    return Err(Diag::new(*line, *col, "let value has no value".to_string()));
                }
                let slot = self.alloc_local(name, *line, *col)?;
                self.emit(format!("store_local {slot}"));
                self.adjust(-1);
            }
            Stmt::BufDecl {
                name,
                size,
                line,
                col,
            } => {
                let func = self.scopes.last().unwrap().func.clone();
                let offset = self
                    .buffer_offsets
                    .get(&(func, name.clone()))
                    .copied()
                    .ok_or_else(|| {
                        Diag::new(*line, *col, "internal: buffer offset missing".to_string())
                    })?;
                self.scopes
                    .last_mut()
                    .unwrap()
                    .buffers
                    .insert(name.clone(), offset);
                let _ = size;
            }
            Stmt::Assign {
                name,
                value,
                line,
                col,
            } => {
                self.emit_loc(*line, *col);
                let slot = self.lookup_local(name).ok_or_else(|| {
                    Diag::new(*line, *col, format!("`{name}` is not an assignable local"))
                })?;
                let produced = self.emit_expr(value)?;
                if !produced {
                    return Err(Diag::new(
                        *line,
                        *col,
                        "assignment value has no value".to_string(),
                    ));
                }
                self.emit(format!("store_local {slot}"));
                self.adjust(-1);
            }
            Stmt::BufStore {
                name,
                index,
                value,
                line,
                col,
            } => {
                self.emit_loc(*line, *col);
                let (offset, size) = self
                    .lookup_buffer(name)
                    .ok_or_else(|| Diag::new(*line, *col, format!("`{name}` is not a buffer")))?;
                let _ = size;
                // address = offset + index
                self.emit(format!("push_i16 {offset}"));
                self.adjust(1);
                self.require_value(index)?;
                self.emit("add_i32".to_string());
                self.adjust(-1);
                // value
                self.require_value(value)?;
                self.emit("store8".to_string());
                self.adjust(-2);
            }
            Stmt::Expr(expr) => {
                let (line, col) = expr.position();
                self.emit_loc(line, col);
                let produced = self.emit_expr(expr)?;
                if produced {
                    self.emit("drop".to_string());
                    self.adjust(-1);
                }
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let (line, col) = cond.position();
                self.emit_loc(line, col);
                let else_label = self.new_label("else");
                let end_label = self.new_label("endif");
                self.emit_cond_branch_if_false(cond, &else_label)?;
                // KOTO-0156 #2: outline a large, non-fall-through then-block to the code
                // tail. `block_diverges` (ends in continue/break/return) is a coldness
                // proxy — such early-out branches are rarely the per-frame hot path (the
                // KotoBlocks title block is the motivating case). Correctness does not
                // depend on it: the tail copy always ends in `br end_label`, so even a
                // mis-classified fall-through block rejoins correctly. We capture the
                // block first to size it; only blocks at/over the threshold relocate.
                if self.options.outline_cold_blocks && block_diverges(then) {
                    let saved = std::mem::take(&mut self.lines);
                    self.emit_block(then)?;
                    let captured = std::mem::replace(&mut self.lines, saved);
                    if count_code_words(&captured) >= OUTLINE_MIN_WORDS {
                        let outline_label = self.new_label("cold");
                        self.emit(format!("br {outline_label}"));
                        self.pending_outlines
                            .push((outline_label, end_label.clone(), captured));
                    } else {
                        self.lines.extend(captured);
                        self.emit(format!("br {end_label}"));
                    }
                } else {
                    self.emit_block(then)?;
                    self.emit(format!("br {end_label}"));
                }
                self.emit(format!("{else_label}:"));
                self.emit_block(otherwise)?;
                self.emit(format!("{end_label}:"));
            }
            Stmt::While { cond, body } => {
                let (line, col) = cond.position();
                self.emit_loc(line, col);
                let top = self.new_label("while");
                let end = self.new_label("endwhile");
                self.emit(format!("{top}:"));
                self.emit_cond_branch_if_false(cond, &end)?;
                self.loops.push((top.clone(), end.clone()));
                self.emit_block(body)?;
                self.loops.pop();
                self.emit(format!("br {top}"));
                self.emit(format!("{end}:"));
            }
            Stmt::Loop { body } => {
                let top = self.new_label("loop");
                let end = self.new_label("endloop");
                self.emit(format!("{top}:"));
                self.loops.push((top.clone(), end.clone()));
                self.emit_block(body)?;
                self.loops.pop();
                self.emit(format!("br {top}"));
                self.emit(format!("{end}:"));
            }
            Stmt::Break { line, col } => {
                self.emit_loc(*line, *col);
                let (_, end) = self
                    .loops
                    .last()
                    .ok_or_else(|| Diag::new(*line, *col, "`break` outside a loop".to_string()))?;
                self.emit(format!("br {end}"));
            }
            Stmt::Continue { line, col } => {
                self.emit_loc(*line, *col);
                let (top, _) = self.loops.last().ok_or_else(|| {
                    Diag::new(*line, *col, "`continue` outside a loop".to_string())
                })?;
                self.emit(format!("br {top}"));
            }
            Stmt::Return { value, line, col } => {
                self.emit_loc(*line, *col);
                self.emit_return(value, *line, *col)?;
            }
        }
        Ok(())
    }

    fn emit_return(&mut self, value: &Option<Expr>, line: usize, col: usize) -> Result<(), Diag> {
        let scope = self.scopes.last().unwrap();
        let func = scope.func.clone();
        let end_label = scope.end_label.clone();
        let ret_ty = self.funcs.get(&func).unwrap().ret;
        match (value, ret_ty) {
            (Some(expr), Some(_)) => {
                self.require_value(expr)?;
                // Route through the return slot so the post-return code keeps a
                // consistent linear depth for the verifier.
                self.emit(format!("store_local {SCRATCH_RET}"));
                self.adjust(-1);
                self.emit(format!("br {end_label}"));
            }
            (None, None) => {
                self.emit(format!("br {end_label}"));
            }
            (Some(_), None) => {
                return Err(Diag::new(line, col, format!("`{func}` returns no value")))
            }
            (None, Some(_)) => {
                return Err(Diag::new(
                    line,
                    col,
                    format!("`{func}` must return a value"),
                ))
            }
        }
        Ok(())
    }

    // ---- expressions ----

    /// Emit `expr`, returning whether it left a value on the operand stack.
    fn emit_expr(&mut self, expr: &Expr) -> Result<bool, Diag> {
        match expr {
            Expr::Int { value, line, col } => {
                if i32::try_from(*value).is_err() {
                    return Err(Diag::new(
                        *line,
                        *col,
                        "integer literal out of 32-bit range".to_string(),
                    ));
                }
                self.emit_int_const(*value as i32);
                Ok(true)
            }
            Expr::Bool { value, .. } => {
                self.emit(format!("push_i16 {}", i32::from(*value)));
                self.adjust(1);
                Ok(true)
            }
            Expr::Str { bytes, line, col } => {
                let offset = self.string_offsets.get(bytes).copied().ok_or_else(|| {
                    Diag::new(*line, *col, "internal: string offset missing".to_string())
                })?;
                self.emit(format!("push_i16 {offset}"));
                self.adjust(1);
                Ok(true)
            }
            Expr::Ident { name, line, col } => {
                if let Some(slot) = self.lookup_local(name) {
                    self.emit(format!("load_local {slot}"));
                    self.adjust(1);
                } else if let Some((offset, _)) = self.lookup_buffer(name) {
                    self.emit(format!("push_i16 {offset}"));
                    self.adjust(1);
                } else if let Some((offset, _)) = self.lookup_data(name) {
                    self.emit_int_const(offset as i32);
                } else if let Some(value) = self.consts.get(name).copied() {
                    let value = i32::try_from(value).map_err(|_| {
                        Diag::new(*line, *col, format!("const `{name}` out of 32-bit range"))
                    })?;
                    self.emit_int_const(value);
                } else {
                    return Err(Diag::new(*line, *col, format!("undefined name `{name}`")));
                }
                Ok(true)
            }
            Expr::BufIndex {
                name,
                index,
                line,
                col,
            } => {
                let (offset, _) = self
                    .lookup_buffer(name)
                    .or_else(|| self.lookup_data(name))
                    .ok_or_else(|| Diag::new(*line, *col, format!("`{name}` is not a buffer")))?;
                self.emit_int_const(offset as i32);
                self.require_value(index)?;
                self.emit("add_i32".to_string());
                self.adjust(-1);
                self.emit("load8".to_string());
                Ok(true)
            }
            Expr::Unary { op, expr, .. } => {
                self.require_value(expr)?;
                match op {
                    UnOp::Neg => {
                        // 0 - x
                        self.emit("push_i16 0".to_string());
                        self.adjust(1);
                        self.emit("swap".to_string());
                        self.emit("sub_i32".to_string());
                        self.adjust(-1);
                    }
                    UnOp::Not => {
                        // (x == 0) as 0/1 — one op shorter than
                        // normalize-then-`1 - b` and bit-identical (Stage 4).
                        self.emit_is_zero();
                    }
                }
                Ok(true)
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                self.emit_binary(*op, lhs, rhs)?;
                Ok(true)
            }
            Expr::Call {
                name,
                args,
                line,
                col,
            } => self.emit_call(name, args, *line, *col),
        }
    }

    fn require_value(&mut self, expr: &Expr) -> Result<(), Diag> {
        if self.emit_expr(expr)? {
            Ok(())
        } else {
            let (line, col) = expr.position();
            Err(Diag::new(line, col, "expression has no value".to_string()))
        }
    }

    fn emit_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<(), Diag> {
        match op {
            BinOp::LAnd | BinOp::LOr if self.options.legacy_compare_templates => {
                self.require_value(lhs)?;
                self.emit_normalize_bool();
                self.require_value(rhs)?;
                self.emit_normalize_bool();
                self.emit(if matches!(op, BinOp::LAnd) {
                    "and_i32".to_string()
                } else {
                    "or_i32".to_string()
                });
                self.adjust(-1);
                return Ok(());
            }
            // KOTO-0169 Stage 4 template shrinks (both exact for all inputs;
            // both still evaluate BOTH operands — `&&`/`||` are not
            // short-circuiting in Koto and that contract is unchanged):
            BinOp::LOr => {
                // (x || y) == ((x | y) != 0): one normalize instead of two.
                self.require_value(lhs)?;
                self.require_value(rhs)?;
                self.binop("or_i32");
                self.emit_normalize_bool();
                return Ok(());
            }
            BinOp::LAnd => {
                // bit 31 of (x | -x) is (x != 0); AND the masks, then extract
                // the shared bit once — drops one shift pair vs normalizing
                // each side to 0/1 first.
                self.require_value(lhs)?;
                self.emit_nonzero_mask();
                self.require_value(rhs)?;
                self.emit_nonzero_mask();
                self.emit("and_i32".to_string());
                self.adjust(-1);
                self.emit("push_i16 31".to_string());
                self.adjust(1);
                self.emit("shr_i32".to_string());
                self.adjust(-1);
                return Ok(());
            }
            _ => {}
        }

        self.require_value(lhs)?;
        self.require_value(rhs)?;
        match op {
            BinOp::Add => self.binop("add_i32"),
            BinOp::Sub => self.binop("sub_i32"),
            BinOp::Mul => self.binop("mul_i32"),
            BinOp::Div => self.binop("div_i32"),
            BinOp::And => self.binop("and_i32"),
            BinOp::Or => self.binop("or_i32"),
            BinOp::Xor => self.binop("xor_i32"),
            BinOp::Shl => self.binop("shl_i32"),
            BinOp::Shr => self.binop("shr_i32"),
            BinOp::Mod => self.emit_mod(),
            BinOp::Eq => {
                self.binop("sub_i32");
                self.emit_is_zero();
            }
            BinOp::Ne => {
                self.binop("sub_i32");
                self.emit_is_nonzero();
            }
            BinOp::Lt => {
                self.binop("sub_i32");
                self.emit_sign_bit();
            }
            BinOp::Gt => {
                self.emit("swap".to_string());
                self.binop("sub_i32");
                self.emit_sign_bit();
            }
            BinOp::Le if self.options.legacy_compare_templates => {
                self.emit("swap".to_string());
                self.binop("sub_i32");
                self.emit_sign_bit();
                self.emit_logical_not_value();
            }
            BinOp::Le => {
                // (a <= b) == sign(a - b - 1): `~d = -d - 1`, so
                // `a - b - 1 = ~(b - a)` and its sign bit is exactly
                // `1 - sign(b - a)` — the old swap/sub/sign/not sequence,
                // two ops shorter and bit-identical for all inputs (Stage 4).
                self.binop("sub_i32");
                self.emit("push_i16 1".to_string());
                self.adjust(1);
                self.emit("sub_i32".to_string());
                self.adjust(-1);
                self.emit_sign_bit();
            }
            BinOp::Ge => {
                self.binop("sub_i32");
                self.emit_sign_bit();
                self.emit_logical_not_value();
            }
            BinOp::LAnd | BinOp::LOr => unreachable!("handled above"),
        }
        Ok(())
    }

    fn emit_call(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
        col: usize,
    ) -> Result<bool, Diag> {
        if let Some(result) = self.emit_heap_call(name, args, line, col)? {
            return Ok(result);
        }
        if let Some(result) = self.emit_actor_call(name, args, line, col)? {
            return Ok(result);
        }
        if let Some(intr) = intrinsic(name) {
            if args.len() != intr.args {
                return Err(Diag::new(
                    line,
                    col,
                    format!(
                        "`{name}` takes {} argument(s), got {}",
                        intr.args,
                        args.len()
                    ),
                ));
            }
            let base = self.depth;
            for arg in args {
                self.require_value(arg)?;
            }
            self.emit(format!("host_call {}", intr.asm));
            return Ok(self.fold_host_result(intr.result, base));
        }

        if self.funcs.contains_key(name) {
            return self.emit_inline(name, args, line, col);
        }

        Err(Diag::new(
            line,
            col,
            format!("call to undefined function `{name}`"),
        ))
    }

    fn emit_heap_call(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
        col: usize,
    ) -> Result<Option<bool>, Diag> {
        match name {
            "heap_get_u8" | "heap_get_u16" | "heap_get_i16" => {
                self.expect_arg_count(name, args, 1, line, col)?;
                self.require_value(&args[0])?;
                match name {
                    "heap_get_u8" => self.emit("load8".to_string()),
                    "heap_get_u16" => self.emit("load16".to_string()),
                    "heap_get_i16" => {
                        self.emit("load16".to_string());
                        self.emit_int_const(0x8000);
                        self.emit("xor_i32".to_string());
                        self.adjust(-1);
                        self.emit_int_const(0x8000);
                        self.emit("sub_i32".to_string());
                        self.adjust(-1);
                    }
                    _ => unreachable!(),
                }
                Ok(Some(true))
            }
            "heap_set_u8" | "heap_set_u16" | "heap_set_i16" => {
                self.expect_arg_count(name, args, 2, line, col)?;
                self.require_value(&args[0])?;
                self.require_value(&args[1])?;
                self.emit(match name {
                    "heap_set_u8" => "store8".to_string(),
                    "heap_set_u16" | "heap_set_i16" => "store16".to_string(),
                    _ => unreachable!(),
                });
                self.adjust(-2);
                Ok(Some(false))
            }
            _ => Ok(None),
        }
    }

    fn emit_actor_call(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
        col: usize,
    ) -> Result<Option<bool>, Diag> {
        match name {
            "actor_array_new" => {
                self.expect_arg_count(name, args, 1, line, col)?;
                let (base, _) = self
                    .actor_arrays
                    .get(&(line, col))
                    .copied()
                    .ok_or_else(|| {
                        Diag::new(
                            line,
                            col,
                            "`actor_array_new` requires a positive constant count".to_string(),
                        )
                    })?;
                self.emit_int_const(base as i32);
                Ok(Some(true))
            }
            "actor_set_pos" => {
                self.expect_arg_count(name, args, 4, line, col)?;
                self.emit_actor_store_i16(&args[0], &args[1], actor_field::X, &args[2])?;
                self.emit_actor_store_i16(&args[0], &args[1], actor_field::Y, &args[3])?;
                Ok(Some(false))
            }
            "actor_set_vel" => {
                self.expect_arg_count(name, args, 4, line, col)?;
                self.emit_actor_store_i16(&args[0], &args[1], actor_field::VX, &args[2])?;
                self.emit_actor_store_i16(&args[0], &args[1], actor_field::VY, &args[3])?;
                Ok(Some(false))
            }
            "actor_set_state" => {
                self.expect_arg_count(name, args, 3, line, col)?;
                self.emit_actor_store_u8(&args[0], &args[1], actor_field::STATE, &args[2])?;
                Ok(Some(false))
            }
            "actor_set_frame" => {
                self.expect_arg_count(name, args, 3, line, col)?;
                self.emit_actor_store_u8(&args[0], &args[1], actor_field::FRAME, &args[2])?;
                Ok(Some(false))
            }
            "actor_set_timer" => {
                self.expect_arg_count(name, args, 3, line, col)?;
                self.emit_actor_store_u16(&args[0], &args[1], actor_field::TIMER, &args[2])?;
                Ok(Some(false))
            }
            "actor_x" | "actor_y" | "actor_vx" | "actor_vy" => {
                self.expect_arg_count(name, args, 2, line, col)?;
                let field = match name {
                    "actor_x" => actor_field::X,
                    "actor_y" => actor_field::Y,
                    "actor_vx" => actor_field::VX,
                    "actor_vy" => actor_field::VY,
                    _ => unreachable!(),
                };
                self.emit_actor_address(&args[0], &args[1], field)?;
                self.emit("load16".to_string());
                self.emit_int_const(0x8000);
                self.emit("xor_i32".to_string());
                self.adjust(-1);
                self.emit_int_const(0x8000);
                self.emit("sub_i32".to_string());
                self.adjust(-1);
                Ok(Some(true))
            }
            "actor_state" | "actor_frame" => {
                self.expect_arg_count(name, args, 2, line, col)?;
                let field = if name == "actor_state" {
                    actor_field::STATE
                } else {
                    actor_field::FRAME
                };
                self.emit_actor_address(&args[0], &args[1], field)?;
                self.emit("load8".to_string());
                Ok(Some(true))
            }
            "actor_timer" => {
                self.expect_arg_count(name, args, 2, line, col)?;
                self.emit_actor_address(&args[0], &args[1], actor_field::TIMER)?;
                self.emit("load16".to_string());
                Ok(Some(true))
            }
            _ => Ok(None),
        }
    }

    fn expect_arg_count(
        &self,
        name: &str,
        args: &[Expr],
        expected: usize,
        line: usize,
        col: usize,
    ) -> Result<(), Diag> {
        if args.len() == expected {
            return Ok(());
        }
        Err(Diag::new(
            line,
            col,
            format!("`{name}` takes {expected} argument(s), got {}", args.len()),
        ))
    }

    fn emit_actor_store_i16(
        &mut self,
        base: &Expr,
        index: &Expr,
        field: u32,
        value: &Expr,
    ) -> Result<(), Diag> {
        self.emit_actor_address(base, index, field)?;
        self.require_value(value)?;
        self.emit("store16".to_string());
        self.adjust(-2);
        Ok(())
    }

    fn emit_actor_store_u16(
        &mut self,
        base: &Expr,
        index: &Expr,
        field: u32,
        value: &Expr,
    ) -> Result<(), Diag> {
        self.emit_actor_store_i16(base, index, field, value)
    }

    fn emit_actor_store_u8(
        &mut self,
        base: &Expr,
        index: &Expr,
        field: u32,
        value: &Expr,
    ) -> Result<(), Diag> {
        self.emit_actor_address(base, index, field)?;
        self.require_value(value)?;
        self.emit("store8".to_string());
        self.adjust(-2);
        Ok(())
    }

    fn emit_actor_address(&mut self, base: &Expr, index: &Expr, field: u32) -> Result<(), Diag> {
        self.require_value(base)?;
        self.require_value(index)?;
        self.emit_int_const(ACTOR_SIZE as i32);
        self.emit("mul_i32".to_string());
        self.adjust(-1);
        self.emit("add_i32".to_string());
        self.adjust(-1);
        if field != 0 {
            self.emit_int_const(field as i32);
            self.emit("add_i32".to_string());
            self.adjust(-1);
        }
        Ok(())
    }

    fn emit_inline(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
        col: usize,
    ) -> Result<bool, Diag> {
        let info_params = self.funcs.get(name).unwrap().params.clone();
        let info_ret = self.funcs.get(name).unwrap().ret;
        let info_body = self.funcs.get(name).unwrap().body.clone();
        if args.len() != info_params.len() {
            return Err(Diag::new(
                line,
                col,
                format!(
                    "`{name}` takes {} argument(s), got {}",
                    info_params.len(),
                    args.len()
                ),
            ));
        }
        // Call-site scoped slots (KOTO-0104): the expansion takes user slots above
        // the caller's live locals and frees them when it ends, so a later disjoint
        // inline call reuses the same physical slots. The parameter slots live at
        // `call_base..call_base + params`; the body's locals run above them.
        let call_base = self.scopes.last().unwrap().next_slot;
        if call_base + info_params.len() > USER_LOCAL_SLOTS {
            return Err(Diag::new(
                line,
                col,
                format!("too many simultaneously-live locals (max {USER_LOCAL_SLOTS} user slots)"),
            ));
        }
        // Bind arguments into the parameter slots. Each bound slot is reserved on the
        // caller scope before the next argument is evaluated, so a nested inline call
        // inside a later argument allocates above the params it must not clobber.
        let mut locals = HashMap::new();
        for (slot_index, ((pname, _), arg)) in info_params.iter().zip(args).enumerate() {
            self.require_value(arg)?;
            let slot = call_base + slot_index;
            self.emit(format!("store_local {slot}"));
            self.adjust(-1);
            locals.insert(pname.clone(), slot);
            self.scopes.last_mut().unwrap().next_slot = slot + 1;
        }
        self.max_slot = self.max_slot.max(call_base + info_params.len());
        let end_label = self.new_label(&format!("ret_{name}"));
        self.scopes.push(Scope {
            func: name.to_string(),
            locals,
            buffers: HashMap::new(),
            next_slot: call_base + info_params.len(),
            end_label: end_label.clone(),
        });
        self.emit_block(&info_body)?;
        self.scopes.pop();
        // Release the parameter slots: the caller resumes at its pre-call cursor.
        self.scopes.last_mut().unwrap().next_slot = call_base;
        // Fallthrough default for value functions: route a 0 through the return
        // slot. Explicit `return`s already stored their value and jumped to the
        // end label, skipping this.
        if info_ret.is_some() {
            self.emit("push_i16 0".to_string());
            self.adjust(1);
            self.emit(format!("store_local {SCRATCH_RET}"));
            self.adjust(-1);
        }
        self.emit(format!("{end_label}:"));
        if info_ret.is_some() {
            self.emit(format!("load_local {SCRATCH_RET}"));
            self.adjust(1);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ---- host-call result folding (verifier success-shape safe) ----

    fn fold_host_result(&mut self, kind: ResultKind, base: i64) -> bool {
        match kind {
            ResultKind::Exit => {
                // Host `exit` pops its argument(s) and pushes nothing, then
                // terminates; depth returns to the pre-argument baseline.
                self.depth = base;
                false
            }
            ResultKind::Status => {
                // Success pushes only the status (0). Use it as the result.
                self.depth = base + 1;
                self.bump_max(self.depth);
                true
            }
            ResultKind::Value => {
                self.depth = base + 2;
                self.bump_max(self.depth);
                let ok = self.new_label("hc_ok");
                let done = self.new_label("hc_done");
                self.emit(format!("br_if_zero {ok}"));
                self.emit("drop".to_string());
                self.emit("push_i16 -1".to_string());
                self.emit(format!("br {done}"));
                self.emit(format!("{ok}:"));
                self.emit(format!("{done}:"));
                self.depth = base + 1;
                true
            }
            ResultKind::Value2First | ResultKind::Value2Second => {
                self.depth = base + 3;
                self.bump_max(self.depth);
                let ok = self.new_label("hc_ok");
                let done = self.new_label("hc_done");
                self.emit(format!("br_if_zero {ok}"));
                self.emit("drop".to_string());
                self.emit("push_i16 -1".to_string());
                self.emit(format!("br {done}"));
                self.emit(format!("{ok}:"));
                if matches!(kind, ResultKind::Value2Second) {
                    self.emit("swap".to_string());
                }
                self.emit("drop".to_string());
                self.emit(format!("{done}:"));
                self.depth = base + 1;
                true
            }
        }
    }

    // ---- branchless helpers ----

    fn binop(&mut self, mnemonic: &str) {
        self.emit(mnemonic.to_string());
        self.adjust(-1);
    }

    /// top: x -> (x | -x). Bit 31 of the result is `(x != 0)`; the lower bits
    /// are garbage a following logical `shr 31` discards (KOTO-0169 Stage 4).
    fn emit_nonzero_mask(&mut self) {
        self.emit("dup".to_string());
        self.adjust(1);
        self.emit("push_i16 0".to_string());
        self.adjust(1);
        self.emit("swap".to_string());
        self.emit("sub_i32".to_string());
        self.adjust(-1);
        self.emit("or_i32".to_string());
        self.adjust(-1);
    }

    /// top: x -> (x != 0) as 0/1, via `(x | -x) >>u 31`.
    fn emit_normalize_bool(&mut self) {
        self.emit_nonzero_mask();
        self.emit("push_i16 31".to_string());
        self.adjust(1);
        self.emit("shr_i32".to_string());
        self.adjust(-1);
    }

    /// top: d -> (d == 0) as 0/1, via `(~(d | -d)) >>u 31` (KOTO-0169 Stage 4:
    /// complementing flips the sign bit exactly — `~v = -v - 1` — so this is
    /// bit-identical to `1 - ((d | -d) >>u 31)` for every input, one op
    /// shorter than normalize-then-`1 - b`).
    fn emit_is_zero(&mut self) {
        if self.options.legacy_compare_templates {
            self.emit_is_nonzero();
            self.emit_logical_not_value();
            return;
        }
        self.emit_nonzero_mask();
        self.emit("push_i16 -1".to_string());
        self.adjust(1);
        self.emit("xor_i32".to_string());
        self.adjust(-1);
        self.emit("push_i16 31".to_string());
        self.adjust(1);
        self.emit("shr_i32".to_string());
        self.adjust(-1);
    }

    /// top: d -> (d != 0) as 0/1.
    fn emit_is_nonzero(&mut self) {
        self.emit_normalize_bool();
    }

    /// Lower a branch condition and emit the jump to `false_label` (KOTO-0169
    /// Stage 4, the KOTO-0154 "template rewrite" follow-up). Comparisons whose
    /// truth is exactly the *zero-ness* of a cheaper value skip the branchless
    /// 0/1 materialization entirely and drive `br_if_zero` directly, flipping
    /// the branch sense where needed:
    ///
    /// | condition | emitted value | sense |
    /// | --- | --- | --- |
    /// | `a == b` | `a - b` (zero ⟺ true) | inverted |
    /// | `a != b` | `a - b` (zero ⟺ false) | normal |
    /// | `a >= b` | `sign(a - b)` = the `<` template (zero ⟺ true) | inverted |
    /// | `a <= b` | `sign(b - a)` = the `>` template (zero ⟺ true) | inverted |
    /// | `!x` | recurse on `x` | flipped |
    /// | anything else | the ordinary 0/1 value | normal |
    ///
    /// All rows are exact: `a - b == 0 ⟺ a == b` in wrapping arithmetic, and
    /// `>=`/`<=` are by definition the negations of the existing `<`/`>`
    /// sign-bit templates, so overflow behavior is bit-identical to the value
    /// forms. The inverted sense keeps the block layout unchanged — it only
    /// prepends `br_if_zero <true>; br <false>; <true>:`, one extra branch on
    /// the false path against 3–10 ops saved on every evaluation.
    fn emit_cond_branch_if_false(&mut self, cond: &Expr, false_label: &str) -> Result<(), Diag> {
        if self.options.legacy_compare_templates {
            self.require_value(cond)?;
            self.emit(format!("br_if_zero {false_label}"));
            self.adjust(-1);
            return Ok(());
        }
        let inverted = self.emit_branch_cond(cond)?;
        if inverted {
            let cond_true = self.new_label("brt");
            self.emit(format!("br_if_zero {cond_true}"));
            self.adjust(-1);
            self.emit(format!("br {false_label}"));
            self.emit(format!("{cond_true}:"));
        } else {
            self.emit(format!("br_if_zero {false_label}"));
            self.adjust(-1);
        }
        Ok(())
    }

    /// Emit `cond` leaving one value whose zero-ness decides the branch;
    /// returns `true` when the sense is inverted (value == 0 ⟺ cond true).
    /// See [`Self::emit_cond_branch_if_false`] for the table.
    fn emit_branch_cond(&mut self, cond: &Expr) -> Result<bool, Diag> {
        match cond {
            Expr::Binary { op, lhs, rhs, .. } => match op {
                BinOp::Eq => {
                    self.require_value(lhs)?;
                    self.require_value(rhs)?;
                    self.binop("sub_i32");
                    Ok(true)
                }
                BinOp::Ne => {
                    self.require_value(lhs)?;
                    self.require_value(rhs)?;
                    self.binop("sub_i32");
                    Ok(false)
                }
                BinOp::Ge => {
                    // `a >= b` ⟺ NOT `a < b`: emit the `<` value, invert.
                    self.require_value(lhs)?;
                    self.require_value(rhs)?;
                    self.binop("sub_i32");
                    self.emit_sign_bit();
                    Ok(true)
                }
                BinOp::Le => {
                    // `a <= b` ⟺ NOT `a > b`: emit the `>` value, invert.
                    self.require_value(lhs)?;
                    self.require_value(rhs)?;
                    self.emit("swap".to_string());
                    self.binop("sub_i32");
                    self.emit_sign_bit();
                    Ok(true)
                }
                _ => {
                    self.require_value(cond)?;
                    Ok(false)
                }
            },
            Expr::Unary {
                op: UnOp::Not,
                expr,
                ..
            } => Ok(!self.emit_branch_cond(expr)?),
            _ => {
                self.require_value(cond)?;
                Ok(false)
            }
        }
    }

    /// top: d -> sign bit (1 if d < 0 in two's complement), via `d >>u 31`.
    fn emit_sign_bit(&mut self) {
        self.emit("push_i16 31".to_string());
        self.adjust(1);
        self.emit("shr_i32".to_string());
        self.adjust(-1);
    }

    /// top: b(0/1) -> 1 - b.
    fn emit_logical_not_value(&mut self) {
        self.emit("push_i16 1".to_string());
        self.adjust(1);
        self.emit("swap".to_string());
        self.emit("sub_i32".to_string());
        self.adjust(-1);
    }

    /// top: a, b -> a % b, via scratch slots and `a - (a / b) * b`.
    fn emit_mod(&mut self) {
        // stack: [a, b]
        self.emit(format!("store_local {SCRATCH_S_B}"));
        self.adjust(-1);
        self.emit(format!("store_local {SCRATCH_S_A}"));
        self.adjust(-1);
        self.emit(format!("load_local {SCRATCH_S_A}"));
        self.adjust(1);
        self.emit(format!("load_local {SCRATCH_S_A}"));
        self.adjust(1);
        self.emit(format!("load_local {SCRATCH_S_B}"));
        self.adjust(1);
        self.emit("div_i32".to_string());
        self.adjust(-1);
        self.emit(format!("load_local {SCRATCH_S_B}"));
        self.adjust(1);
        self.emit("mul_i32".to_string());
        self.adjust(-1);
        self.emit("sub_i32".to_string());
        self.adjust(-1);
    }

    // ---- scope + emit utilities ----

    /// Push a 32-bit integer constant, leaving one value on the operand stack.
    /// `push_i16` only carries a sign-extended 16-bit immediate, so values outside
    /// `i16` range are assembled from their two halves: `high << 16 | (low
    /// zero-extended)`. The low half is zero-extended with a logical `shr_i32` so a
    /// set bit 15 does not leak into the upper word.
    fn emit_int_const(&mut self, value: i32) {
        if (i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&value) {
            self.emit(format!("push_i16 {value}"));
            self.adjust(1);
            return;
        }
        let bits = value as u32;
        let high = (bits >> 16) as u16 as i16;
        let low = (bits & 0xFFFF) as u16 as i16;
        // high << 16
        self.emit(format!("push_i16 {high}"));
        self.adjust(1);
        self.emit("push_i16 16".to_string());
        self.adjust(1);
        self.emit("shl_i32".to_string());
        self.adjust(-1);
        // (low << 16) >> 16  — zero-extended low 16 bits
        self.emit(format!("push_i16 {low}"));
        self.adjust(1);
        self.emit("push_i16 16".to_string());
        self.adjust(1);
        self.emit("shl_i32".to_string());
        self.adjust(-1);
        self.emit("push_i16 16".to_string());
        self.adjust(1);
        self.emit("shr_i32".to_string());
        self.adjust(-1);
        self.emit("or_i32".to_string());
        self.adjust(-1);
    }

    fn alloc_local(&mut self, name: &str, line: usize, col: usize) -> Result<usize, Diag> {
        let scope = self.scopes.last_mut().unwrap();
        if scope.next_slot >= USER_LOCAL_SLOTS {
            return Err(Diag::new(
                line,
                col,
                format!("too many simultaneously-live locals (max {USER_LOCAL_SLOTS} user slots)"),
            ));
        }
        let slot = scope.next_slot;
        scope.next_slot += 1;
        let next = scope.next_slot;
        scope.locals.insert(name.to_string(), slot);
        self.max_slot = self.max_slot.max(next);
        Ok(slot)
    }

    fn lookup_local(&self, name: &str) -> Option<usize> {
        self.scopes
            .last()
            .and_then(|scope| scope.locals.get(name).copied())
    }

    fn lookup_buffer(&self, name: &str) -> Option<(u32, u32)> {
        self.scopes
            .last()
            .and_then(|scope| scope.buffers.get(name).copied())
    }

    /// A top-level `data` const buffer's `(offset, byte length)`. Unlike `buf`,
    /// these are global, so they resolve from any function (KOTO-0139).
    fn lookup_data(&self, name: &str) -> Option<(u32, u32)> {
        self.data_offsets.get(name).copied()
    }

    fn emit(&mut self, line: String) {
        self.lines.push(line);
    }

    fn emit_loc(&mut self, line: usize, col: usize) {
        self.emit(format!(".loc {line} {col}"));
    }

    fn new_label(&mut self, prefix: &str) -> String {
        self.label_id += 1;
        format!("{prefix}_{}", self.label_id)
    }

    fn adjust(&mut self, delta: i64) {
        self.depth += delta;
        self.bump_max(self.depth);
    }

    fn bump_max(&mut self, value: i64) {
        if value > self.max_depth {
            self.max_depth = value;
        }
    }
}

// ---- slot map (local-slot attribution, KOTO-0102 / KOTO-0104) ----

/// One function's own user-local footprint when inlined: its parameter slots plus
/// the peak simultaneously-live `let` slots in its body (per-scope reuse, KOTO-0092).
/// This is the slots the function itself contributes at its deepest point, not
/// counting nested inline calls; with call-site reuse (KOTO-0104) functions no
/// longer own fixed slot ranges.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FnSlots {
    pub name: String,
    /// Parameter slots (one per parameter).
    pub params: usize,
    /// Peak simultaneously-live `let` slots in the body (after per-scope reuse).
    pub locals: usize,
    /// The `fn` definition line. Codegen fills the include-expanded line; the
    /// crate boundary remaps it to the defining file's own line (KOTO-0183).
    pub line: usize,
    /// `file:line` attribution of the definition, filled at the crate boundary
    /// so slot pressure points at the file that owns it (KOTO-0183).
    pub src: String,
}

impl FnSlots {
    /// This function's own user-slot footprint (`params + locals`).
    pub fn slots(&self) -> usize {
        self.params + self.locals
    }
}

/// User-local-slot usage for an app. `user_slots_used` is the post-reuse physical
/// peak: the highest user slot the compiler actually allocates across the whole
/// program once call-site inline-slot reuse (KOTO-0104) and per-scope reuse
/// (KOTO-0092) collapse non-overlapping lifetimes. The per-function `functions`
/// list reports each function's own footprint, a guide to which helpers are heavy
/// to inline (they no longer own fixed ranges). The top three VM slots are codegen
/// scratch and are not user slots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotMap {
    /// Functions in source-definition order, with their own footprints.
    pub functions: Vec<FnSlots>,
    /// Post-reuse physical user-slot peak across the program.
    pub user_slots_used: usize,
    /// User slots available before the codegen scratch region.
    pub user_slots_cap: usize,
    /// Codegen scratch slots reserved at the top of the file.
    pub scratch_slots: usize,
    /// The full VM local file size.
    pub vm_local_slots: usize,
}

/// Compute the user-local-slot map for `program`. `user_slots_used` is the real
/// post-reuse peak, obtained by running codegen (so it cannot drift from what the
/// compiler actually allocates); the per-function footprints are static.
pub fn slot_map(file: &str, program: &Program) -> Result<SlotMap, Diag> {
    let mut codegen = Codegen::new(file, program)?;
    codegen.run()?;
    let user_slots_used = codegen.max_slot;
    let functions = program
        .functions
        .iter()
        .map(|function| FnSlots {
            name: function.name.clone(),
            params: function.params.len(),
            locals: peak_lets(&function.body),
            line: function.line,
            src: String::new(),
        })
        .collect();
    Ok(SlotMap {
        functions,
        user_slots_used,
        user_slots_cap: USER_LOCAL_SLOTS,
        scratch_slots: koto_core::runtime::VM_LOCAL_SLOTS - USER_LOCAL_SLOTS,
        vm_local_slots: koto_core::runtime::VM_LOCAL_SLOTS,
    })
}

// ---- AST walkers ----

/// The peak number of simultaneously-live locals in a block. A `let` at this
/// block level stays live for the rest of the block; a nested block's locals are
/// live only within it, and an `if`'s two arms are disjoint. This is the slot
/// budget a function needs once disjoint blocks reuse slots (KOTO-0092), and it
/// mirrors the actual codegen, where [`Codegen::emit_block`] frees a block's
/// slots when the block ends.
fn peak_lets(stmts: &[Stmt]) -> usize {
    let mut live = 0;
    let mut peak = 0;
    for stmt in stmts {
        match stmt {
            Stmt::Let { .. } => {
                live += 1;
                peak = peak.max(live);
            }
            Stmt::If {
                then, otherwise, ..
            } => {
                peak = peak.max(live + peak_lets(then).max(peak_lets(otherwise)));
            }
            Stmt::While { body, .. } | Stmt::Loop { body } => {
                peak = peak.max(live + peak_lets(body));
            }
            _ => {}
        }
    }
    peak
}

fn collect_buffers(stmts: &[Stmt], sink: &mut impl FnMut(&str, u32)) {
    for stmt in stmts {
        match stmt {
            Stmt::BufDecl { name, size, .. } => sink(name, *size as u32),
            Stmt::If {
                then, otherwise, ..
            } => {
                collect_buffers(then, sink);
                collect_buffers(otherwise, sink);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } => collect_buffers(body, sink),
            _ => {}
        }
    }
}

fn collect_actor_arrays(
    stmts: &[Stmt],
    consts: &HashMap<String, i64>,
    sink: &mut impl FnMut(usize, usize, u32),
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr(value) => {
                collect_actor_arrays_expr(value, consts, sink)
            }
            Stmt::BufStore { index, value, .. } => {
                collect_actor_arrays_expr(index, consts, sink);
                collect_actor_arrays_expr(value, consts, sink);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                collect_actor_arrays_expr(cond, consts, sink);
                collect_actor_arrays(then, consts, sink);
                collect_actor_arrays(otherwise, consts, sink);
            }
            Stmt::While { cond, body } => {
                collect_actor_arrays_expr(cond, consts, sink);
                collect_actor_arrays(body, consts, sink);
            }
            Stmt::Loop { body } => collect_actor_arrays(body, consts, sink),
            Stmt::Return {
                value: Some(expr), ..
            } => collect_actor_arrays_expr(expr, consts, sink),
            _ => {}
        }
    }
}

fn collect_actor_arrays_expr(
    expr: &Expr,
    consts: &HashMap<String, i64>,
    sink: &mut impl FnMut(usize, usize, u32),
) {
    match expr {
        Expr::Call {
            name,
            args,
            line,
            col,
        } => {
            if name == "actor_array_new" && args.len() == 1 {
                if let Some(count) = const_actor_count(&args[0], consts) {
                    sink(*line, *col, count);
                }
            }
            for arg in args {
                collect_actor_arrays_expr(arg, consts, sink);
            }
        }
        Expr::BufIndex { index, .. } => collect_actor_arrays_expr(index, consts, sink),
        Expr::Unary { expr, .. } => collect_actor_arrays_expr(expr, consts, sink),
        Expr::Binary { lhs, rhs, .. } => {
            collect_actor_arrays_expr(lhs, consts, sink);
            collect_actor_arrays_expr(rhs, consts, sink);
        }
        _ => {}
    }
}

fn const_actor_count(expr: &Expr, consts: &HashMap<String, i64>) -> Option<u32> {
    let value = match expr {
        Expr::Int { value, .. } => *value,
        Expr::Ident { name, .. } => *consts.get(name)?,
        _ => return None,
    };
    u32::try_from(value).ok().filter(|count| *count > 0)
}

/// KOTO-0156 #2: does this statement block always leave via an unconditional control
/// transfer (so control never falls through to the statement after the enclosing `if`)?
/// Used as a *coldness proxy* for cold-block outlining: a then-block whose last statement
/// is `continue`/`break`/`return` is an early-out branch (e.g. KotoBlocks' title-state
/// block ends in `continue`), rarely the per-frame hot path. This is a selection
/// heuristic only — outlining stays correct for fall-through blocks too because the
/// relocated copy always ends in a branch back to the merge point.
fn block_diverges(stmts: &[Stmt]) -> bool {
    matches!(
        stmts.last(),
        Some(Stmt::Continue { .. } | Stmt::Break { .. } | Stmt::Return { .. })
    )
}

/// KOTO-0156 #2: count the emitted *instructions* in a captured line buffer (excludes
/// labels, `.`-directives like `.loc`, and blanks) to size a candidate cold block
/// against [`OUTLINE_MIN_WORDS`]. Approximate — `store_str` is one line but several
/// words — which is fine for a size threshold.
fn count_code_words(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.ends_with(':') && !t.starts_with('.')
        })
        .count()
}

fn collect_strings(stmts: &[Stmt], sink: &mut impl FnMut(&[u8])) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr(value) => {
                collect_strings_expr(value, sink)
            }
            Stmt::BufStore { index, value, .. } => {
                collect_strings_expr(index, sink);
                collect_strings_expr(value, sink);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                collect_strings_expr(cond, sink);
                collect_strings(then, sink);
                collect_strings(otherwise, sink);
            }
            Stmt::While { cond, body } => {
                collect_strings_expr(cond, sink);
                collect_strings(body, sink);
            }
            Stmt::Loop { body } => collect_strings(body, sink),
            Stmt::Return {
                value: Some(expr), ..
            } => collect_strings_expr(expr, sink),
            _ => {}
        }
    }
}

fn collect_strings_expr(expr: &Expr, sink: &mut impl FnMut(&[u8])) {
    match expr {
        Expr::Str { bytes, .. } => sink(bytes),
        Expr::BufIndex { index, .. } => collect_strings_expr(index, sink),
        Expr::Unary { expr, .. } => collect_strings_expr(expr, sink),
        Expr::Binary { lhs, rhs, .. } => {
            collect_strings_expr(lhs, sink);
            collect_strings_expr(rhs, sink);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_strings_expr(arg, sink);
            }
        }
        _ => {}
    }
}

fn collect_calls(stmts: &[Stmt], sink: &mut impl FnMut(&str)) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr(value) => {
                collect_calls_expr(value, sink)
            }
            Stmt::BufStore { index, value, .. } => {
                collect_calls_expr(index, sink);
                collect_calls_expr(value, sink);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                collect_calls_expr(cond, sink);
                collect_calls(then, sink);
                collect_calls(otherwise, sink);
            }
            Stmt::While { cond, body } => {
                collect_calls_expr(cond, sink);
                collect_calls(body, sink);
            }
            Stmt::Loop { body } => collect_calls(body, sink),
            Stmt::Return {
                value: Some(expr), ..
            } => collect_calls_expr(expr, sink),
            _ => {}
        }
    }
}

fn collect_calls_expr(expr: &Expr, sink: &mut impl FnMut(&str)) {
    match expr {
        Expr::Call { name, args, .. } => {
            sink(name);
            for arg in args {
                collect_calls_expr(arg, sink);
            }
        }
        Expr::BufIndex { index, .. } => collect_calls_expr(index, sink),
        Expr::Unary { expr, .. } => collect_calls_expr(expr, sink),
        Expr::Binary { lhs, rhs, .. } => {
            collect_calls_expr(lhs, sink);
            collect_calls_expr(rhs, sink);
        }
        _ => {}
    }
}

/// Substitute the floating-scratch placeholder operands ([`SCRATCH_RET`],
/// [`SCRATCH_S_A`], [`SCRATCH_S_B`]) with their resolved slot indices. The tokens
/// only ever appear as the sole operand of a `load_local`/`store_local` line, so
/// matching the line prefix and an exact-token suffix is collision-proof: other
/// lines (including `store_str` string data that could contain arbitrary text)
/// pass through untouched. A token that slipped past here would be a non-numeric
/// operand the assembler rejects loudly, never a silent miscompile.
fn resolve_scratch(line: &str, a: usize, b: usize, ret: usize) -> Cow<'_, str> {
    if line.starts_with("load_local ") || line.starts_with("store_local ") {
        for (token, slot) in [(SCRATCH_RET, ret), (SCRATCH_S_A, a), (SCRATCH_S_B, b)] {
            if let Some(prefix) = line.strip_suffix(token) {
                return Cow::Owned(format!("{prefix}{slot}"));
            }
        }
    }
    Cow::Borrowed(line)
}

// ---- peephole optimization (KOTO-0154) ----
//
// A small, deliberately conservative cleanup pass over the emitted `kbc-asm`
// instruction lines, run after code generation and before final assembly. It
// targets the profiled hot mix (push/arith/stack-shuffle; see
// `docs/devlog/KOTO_VM_PROFILE_KOTOBLOCKS.md` optimization targets 1 and 2) without
// touching the VM. Every rewrite is justified mechanically against the exact
// interpreter semantics in `koto-vm` (`BytecodeVm::step` / `exec_binary`):
//
//   1. `push_i16 X; push_i16 Y; <binop>`  -> `push_i16 (X op Y)`   (constant fold)
//   2. `push_i16 <identity>; <binop>`      -> (deleted)            (algebraic id.)
//   3. `push_i16 X; drop`                  -> (deleted)            (dead literal)
//   4. `dup; drop`                         -> (deleted)            (round-trip)
//   5. `swap; swap`                        -> (deleted)            (cancel)
//
// Safety contract:
// - It optimizes only *within straight-line runs* of plain stack/arithmetic
//   instructions. Any label, directive (`.loc`/`.rodata`/header), branch
//   (`br`/`br_if_zero`), `call`/`ret`/`halt`, `host_call`, `store_str`, or any
//   other mnemonic ends the current run and is never crossed or reordered. So the
//   pass cannot disturb control flow, branch targets, the debug line table, or
//   host-call side effects, and it never reorders a memory `load*`/`store*`.
// - Every rewrite is net stack-neutral (rules 2–5) or `+1 -> +1` (rule 1), so the
//   verifier's single linear operand-depth scan stays consistent at every later
//   point; the only effect on depth is to *lower* intermediate peaks.
// - Constant folding replicates `exec_binary` exactly (same wrapping and the same
//   logical `shr`/masked-shift behavior) and only re-emits a single `push_i16`
//   when the folded value round-trips through the 16-bit sign-extended immediate,
//   so the materialized constant is bit-identical to running the three opcodes.
//   Division by zero is never folded, preserving the runtime `DivisionByZero` trap.
// - Opcode values, the bytecode ABI, hostcall IDs, `RuntimeLimits`, verifier
//   rules, and interpreter behavior are all untouched.
fn peephole(lines: &mut Vec<String>) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut run: Vec<String> = Vec::new();
    for line in lines.drain(..) {
        if is_run_instruction(&line) {
            run.push(line);
        } else {
            optimize_run(&mut run);
            out.append(&mut run);
            out.push(line);
        }
    }
    optimize_run(&mut run);
    out.append(&mut run);
    *lines = out;
}

/// The mnemonics that may appear inside an optimizable straight-line run. These are
/// exactly the value-/stack-pure and deterministic memory ops; anything else (a
/// label, a directive, a branch, `host_call`, `store_str`, `nop`, …) is a barrier
/// that ends the run. Memory `load*`/`store*` are admitted only so a run is not
/// fragmented around them — no rewrite rule ever moves or removes them.
fn is_run_instruction(line: &str) -> bool {
    matches!(
        mnemonic(line),
        "push_i16"
            | "dup"
            | "drop"
            | "swap"
            | "load_local"
            | "store_local"
            | "add_i32"
            | "sub_i32"
            | "mul_i32"
            | "div_i32"
            | "and_i32"
            | "or_i32"
            | "xor_i32"
            | "shl_i32"
            | "shr_i32"
            | "load8"
            | "load16"
            | "load32"
            | "store8"
            | "store16"
            | "store32"
    )
}

fn mnemonic(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

/// The value a `push_i16 N` line actually pushes, as the VM observes it: the
/// assembler narrows the operand to a `u16` (`parse_imm16` accepts `-32768..=65535`)
/// and `BytecodeVm::step` sign-extends that `u16` to `i32`. Returns `None` for any
/// line that is not a `push_i16` with an in-range decimal immediate, so non-numeric
/// or hand-written operands are simply never folded.
fn push_value(line: &str) -> Option<i32> {
    let rest = line.strip_prefix("push_i16 ")?.trim();
    let n: i64 = rest.parse().ok()?;
    if !(-32768..=65535).contains(&n) {
        return None;
    }
    Some(i32::from((n as u16) as i16))
}

/// Fold a binary op over two compile-time constants using the *exact* semantics of
/// `koto_vm::BytecodeVm::exec_binary` (operands are `lhs` then `rhs`, i.e. the lower
/// then the upper stack slot). Returns `None` for non-foldable mnemonics and for
/// `div_i32` by zero (which must keep trapping at runtime).
fn fold_binop(op: &str, lhs: i32, rhs: i32) -> Option<i32> {
    Some(match op {
        "add_i32" => lhs.wrapping_add(rhs),
        "sub_i32" => lhs.wrapping_sub(rhs),
        "mul_i32" => lhs.wrapping_mul(rhs),
        "div_i32" => {
            if rhs == 0 {
                return None;
            }
            lhs.wrapping_div(rhs)
        }
        "and_i32" => lhs & rhs,
        "or_i32" => lhs | rhs,
        "xor_i32" => lhs ^ rhs,
        "shl_i32" => lhs.wrapping_shl((rhs & 31) as u32),
        "shr_i32" => ((lhs as u32).wrapping_shr((rhs & 31) as u32)) as i32,
        _ => return None,
    })
}

/// True when `push_i16 v; <op>` computes `lhs op v == lhs` for every `lhs`, so the
/// pair is a no-op that can be deleted. Justified per opcode against `exec_binary`:
/// `+0`, `-0`, `|0`, `^0`, `<<0`, `>>0` are identities; `*1`; and `& -1` (all bits
/// set). Shifts only when the amount is exactly 0 (kept simple — not `v & 31 == 0`).
fn is_identity(op: &str, v: i32) -> bool {
    match op {
        "add_i32" | "sub_i32" | "or_i32" | "xor_i32" | "shl_i32" | "shr_i32" => v == 0,
        "mul_i32" => v == 1,
        "and_i32" => v == -1,
        _ => false,
    }
}

/// Apply the peephole rewrite rules to a single straight-line run, in place, to a
/// fixpoint (folds and deletions can cascade, e.g. `push 2; push 3; mul` -> `push 6`
/// then a following `push 6; push 4; add` -> `push 10`).
fn optimize_run(run: &mut Vec<String>) {
    if run.len() < 2 {
        return;
    }
    let mut i = 0;
    while i < run.len() {
        if try_rewrite(run, i) {
            // A rewrite changed run[i] (and possibly merged neighbors). Step back so a
            // freshly-adjacent preceding instruction can combine with the new line.
            i = i.saturating_sub(2);
        } else {
            i += 1;
        }
    }
}

/// Try the highest-priority rewrite whose window starts at `run[i]`, mutating `run`
/// in place. Returns whether the run was modified.
fn try_rewrite(run: &mut Vec<String>, i: usize) -> bool {
    let here = mnemonic(&run[i]).to_string();
    let next = run.get(i + 1).map(|l| mnemonic(l).to_string());
    let next2 = run.get(i + 2).map(|l| mnemonic(l).to_string());

    // Rule 1: constant fold `push_i16 X; push_i16 Y; <binop>` -> `push_i16 (X op Y)`.
    if here == "push_i16" {
        if let (Some(x), Some("push_i16")) = (push_value(&run[i]), next.as_deref()) {
            if let (Some(y), Some(op)) = (push_value(&run[i + 1]), next2.as_deref()) {
                if let Some(folded) = fold_binop(op, x, y) {
                    if (-32768..=32767).contains(&folded) {
                        run.splice(i..i + 3, [format!("push_i16 {folded}")]);
                        return true;
                    }
                }
            }
        }
    }

    // Rule 2/3: `push_i16 v` followed by an identity binop, or by `drop` (dead push).
    if here == "push_i16" {
        if let Some(next) = next.as_deref() {
            if next == "drop" {
                run.drain(i..i + 2);
                return true;
            }
            if let Some(v) = push_value(&run[i]) {
                if is_identity(next, v) {
                    run.drain(i..i + 2);
                    return true;
                }
            }
        }
    }

    // Rule 4/5: round-trip `dup; drop` and cancelling `swap; swap`.
    if (here == "dup" && next.as_deref() == Some("drop"))
        || (here == "swap" && next.as_deref() == Some("swap"))
    {
        run.drain(i..i + 2);
        return true;
    }

    false
}

fn render_string(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod peephole_tests {
    //! KOTO-0154 peephole equivalence tests. These prove the optimized and
    //! unoptimized instruction streams produce *identical* VM behavior by
    //! assembling each body both ways and running both through the real
    //! `koto-vm` interpreter, asserting the same `Exited` code while confirming
    //! the optimized form is no longer (and, for the cases that should fire,
    //! strictly shorter).

    use super::*;
    use koto_core::{
        verify_kbc, BytecodeVm, HostCallOutcome, RuntimeLimits, VmHost, VmInputSnapshot,
        VmRunResult,
    };

    fn lines(body: &[&str]) -> Vec<String> {
        body.iter().map(|s| (*s).to_string()).collect()
    }

    /// A do-nothing host: the equivalence bodies are pure arithmetic terminated by
    /// `exit`, which the VM handles internally, so no host method is ever called.
    struct NullHost;
    impl VmHost for NullHost {
        fn draw_rect(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32) -> HostCallOutcome {
            HostCallOutcome::Ok0
        }
        fn draw_text(&mut self, _: i32, _: i32, _: &str) -> HostCallOutcome {
            HostCallOutcome::Ok0
        }
        fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
            HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
        }
        fn file_open(&mut self, _: &str, _: i32) -> HostCallOutcome {
            HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
        }
        fn file_read(&mut self, _: i32, _: &mut [u8]) -> HostCallOutcome {
            HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
        }
        fn file_write(&mut self, _: i32, _: &[u8]) -> HostCallOutcome {
            HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)
        }
        fn file_close(&mut self, _: i32) -> HostCallOutcome {
            HostCallOutcome::Ok0
        }
    }

    /// Wrap an instruction body that leaves one value on the stack into a minimal,
    /// verifier-valid program that exits with that value, assemble it, and run it.
    fn run_body(body: &[String]) -> VmRunResult {
        let mut asm = String::from(".stack 16\n.calls 4\n.heap 64\n.entry 0\n");
        for line in body {
            asm.push_str(line);
            asm.push('\n');
        }
        asm.push_str("host_call exit\nhalt\n");
        let bytecode = kbc_asm::assemble(&asm).expect("body assembles");
        let program =
            verify_kbc(&bytecode, RuntimeLimits::simulator_default()).expect("body verifies");
        let mut vm = BytecodeVm::<16, 4>::new(&program).expect("vm");
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        let mut host = NullHost;
        vm.execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            100_000,
            &mut heap,
        )
        .expect("runs without trapping")
    }

    /// The core guarantee: peephole-optimizing a body never changes what it computes.
    /// `expect_shorter` asserts the pass actually fired (rules under test really hit).
    fn assert_equivalent(body: &[&str], expect_shorter: bool) {
        let original = lines(body);
        let mut optimized = original.clone();
        peephole(&mut optimized);
        assert_eq!(
            run_body(&original),
            run_body(&optimized),
            "optimized body diverged from original: {body:?} -> {optimized:?}"
        );
        if expect_shorter {
            assert!(
                optimized.len() < original.len(),
                "expected peephole to shrink {body:?}, got {optimized:?}"
            );
        } else {
            assert_eq!(optimized, original, "expected no change for {body:?}");
        }
    }

    #[test]
    fn fold_binop_matches_vm_semantics() {
        assert_eq!(fold_binop("add_i32", 5, 7), Some(12));
        assert_eq!(fold_binop("sub_i32", 5, 7), Some(-2));
        assert_eq!(fold_binop("mul_i32", 6, 7), Some(42));
        assert_eq!(fold_binop("div_i32", 20, 4), Some(5));
        assert_eq!(
            fold_binop("div_i32", 1, 0),
            None,
            "div by zero must not fold"
        );
        assert_eq!(fold_binop("and_i32", 0b1100, 0b1010), Some(0b1000));
        assert_eq!(fold_binop("or_i32", 0b1100, 0b1010), Some(0b1110));
        assert_eq!(fold_binop("xor_i32", 0b1100, 0b1010), Some(0b0110));
        // Shifts mask the amount to 5 bits and `shr` is logical, exactly like the VM.
        assert_eq!(fold_binop("shl_i32", 1, 4), Some(16));
        assert_eq!(
            fold_binop("shl_i32", 1, 33),
            Some(2),
            "shift amount masked to 31"
        );
        assert_eq!(
            fold_binop("shr_i32", -1, 31),
            Some(1),
            "logical, not arithmetic"
        );
        assert_eq!(fold_binop("nop", 1, 2), None);
    }

    #[test]
    fn push_value_round_trips_through_imm16() {
        assert_eq!(push_value("push_i16 7"), Some(7));
        assert_eq!(push_value("push_i16 -1"), Some(-1));
        // 0x8000..=0xFFFF immediates sign-extend negative, matching the interpreter.
        assert_eq!(push_value("push_i16 65535"), Some(-1));
        assert_eq!(push_value("push_i16 32768"), Some(-32768));
        assert_eq!(push_value("push_i16 70000"), None, "out of u16 range");
        assert_eq!(push_value("load_local 3"), None);
    }

    #[test]
    fn constant_chain_folds_to_single_push() {
        let mut run = lines(&[
            "push_i16 2",
            "push_i16 3",
            "mul_i32",
            "push_i16 4",
            "add_i32",
        ]);
        peephole(&mut run);
        assert_eq!(run, vec!["push_i16 10".to_string()]);
    }

    #[test]
    fn out_of_range_fold_is_left_alone() {
        // 30000 + 30000 = 60000 does not fit a single sign-extended i16 push, so the
        // three opcodes must stay (correctness over a non-representable constant).
        let body = &["push_i16 30000", "push_i16 30000", "add_i32"];
        assert_equivalent(body, false);
    }

    #[test]
    fn identity_and_dead_push_and_shuffle_cancel() {
        // x + 0, x * 1, x | 0, x & -1, x << 0, x >> 0  -> x
        assert_equivalent(&["push_i16 9", "push_i16 0", "add_i32"], true);
        assert_equivalent(&["push_i16 9", "push_i16 1", "mul_i32"], true);
        assert_equivalent(&["push_i16 9", "push_i16 0", "or_i32"], true);
        assert_equivalent(&["push_i16 9", "push_i16 -1", "and_i32"], true);
        assert_equivalent(&["push_i16 9", "push_i16 0", "shr_i32"], true);
        // dead literal, round-trip dup/drop, cancelling swap/swap (value preserved).
        assert_equivalent(&["push_i16 9", "push_i16 123", "drop"], true);
        assert_equivalent(&["push_i16 9", "dup", "drop"], true);
        assert_equivalent(
            &["push_i16 8", "push_i16 9", "swap", "swap", "add_i32"],
            true,
        );
    }

    #[test]
    fn essential_shuffles_are_preserved() {
        // Unary negate lowering `0 - x` (push 0; swap; sub) must NOT be cancelled by
        // the `push 0; sub` identity rule — the intervening swap makes it `0 - x`.
        assert_equivalent(&["push_i16 5", "push_i16 0", "swap", "sub_i32"], false);
        // A real (non-identity) shift by a constant over a runtime value stays put:
        // only the shift amount is constant, so there is nothing to fold or cancel.
        assert_equivalent(&["load_local 2", "push_i16 31", "shr_i32"], false);
    }

    #[test]
    fn barriers_are_never_crossed() {
        // A label between the two pushes ends the run, so no fold may occur.
        let mut run = lines(&["push_i16 2", "skip:", "push_i16 3", "add_i32"]);
        peephole(&mut run);
        assert_eq!(
            run,
            lines(&["push_i16 2", "skip:", "push_i16 3", "add_i32"]),
            "fold must not cross a label barrier"
        );
        // A host_call between also blocks folding across it.
        let mut run = lines(&[
            "push_i16 2",
            "host_call yield_frame",
            "push_i16 3",
            "add_i32",
        ]);
        let before = run.clone();
        peephole(&mut run);
        assert_eq!(run, before, "fold must not cross a host_call");
    }
}
