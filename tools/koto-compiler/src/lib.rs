//! `koto-compiler`: a host-side, ahead-of-time compiler from the Koto app
//! language (see `docs/spec/KOTO_APP_LANGUAGE.md`) to verifier-valid `KBC1` bytecode.
//!
//! The pipeline is `source -> lex -> parse -> codegen (assembly text) ->
//! kbc-asm -> KBC1`, with a final `verify_kbc` guarantee. It is a development
//! tool only; nothing here runs on the device.

mod codegen;
mod lexer;
mod parser;
mod preprocess;

use koto_core::{verify_kbc, RuntimeLimits};

pub use preprocess::{FsLoader, IncludeLoader, SourceMap};

/// An internal diagnostic with source line/column (1-based). Promoted to a
/// [`CompileError`] with the filename at the crate boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl Diag {
    pub fn new(line: usize, col: usize, message: String) -> Self {
        Self { line, col, message }
    }
}

/// A user-facing compile error carrying filename, line, column, and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.line, self.col, self.message
        )
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// Promote a [`Diag`] whose line refers to the include-expanded source
    /// (KOTO-0183): the [`SourceMap`] attributes it back to the file and line
    /// the author actually wrote.
    fn from_diag_mapped(map: &SourceMap, diag: Diag) -> Self {
        let (file, line) = map.resolve(diag.line);
        Self {
            file: file.to_string(),
            line,
            col: diag.col,
            message: diag.message,
        }
    }

    fn internal(file: &str, message: String) -> Self {
        Self {
            file: file.to_string(),
            line: 0,
            col: 0,
            message,
        }
    }
}

/// Compile `source` to assembly text (the KOTO-0044 IR). Useful for debugging
/// and for golden tests of code generation.
pub fn compile_to_asm(file: &str, source: &str) -> Result<String, CompileError> {
    compile_to_asm_with_options(file, source, CodegenOptions::default())
}

pub use codegen::CodegenOptions;

/// Like [`compile_to_asm`], but with the KOTO-0156 code-window layout options
/// ([`CodegenOptions`]). `CodegenOptions::default()` (both off) is the baseline layout.
/// Used by the CLI's `--emit-asm` and by equivalence/layout tests.
pub fn compile_to_asm_with_options(
    file: &str,
    source: &str,
    options: CodegenOptions,
) -> Result<String, CompileError> {
    compile_to_asm_with_loader(file, source, options, &mut FsLoader)
}

/// Like [`compile_to_asm_with_options`], with include loading injected
/// (KOTO-0183) so tests and future tools can resolve includes without a
/// filesystem.
pub fn compile_to_asm_with_loader(
    file: &str,
    source: &str,
    options: CodegenOptions,
    loader: &mut dyn IncludeLoader,
) -> Result<String, CompileError> {
    let (expanded, map) = preprocess::expand(file, source, loader)?;
    let tokens = lexer::lex(&expanded).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    let program = parser::parse(&tokens).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    codegen::compile_to_asm_with(file, &program, options)
        .map_err(|d| CompileError::from_diag_mapped(&map, d))
}

pub use codegen::{FnSlots, SlotMap};

/// Compute the user-local-slot attribution for `source` (KOTO-0102): which inlined
/// functions own which slots, and the user-slot total against the user/scratch/VM
/// capacities. Useful for budget diagnostics and slot-pressure reduction.
pub fn slot_map(file: &str, source: &str) -> Result<SlotMap, CompileError> {
    slot_map_with_loader(file, source, &mut FsLoader)
}

/// Like [`slot_map`], with include loading injected (KOTO-0183). Each
/// function's `src` is attributed through the [`SourceMap`], so slot pressure
/// points at the file that defines the function, not just the root source.
pub fn slot_map_with_loader(
    file: &str,
    source: &str,
    loader: &mut dyn IncludeLoader,
) -> Result<SlotMap, CompileError> {
    let (expanded, map) = preprocess::expand(file, source, loader)?;
    let tokens = lexer::lex(&expanded).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    let program = parser::parse(&tokens).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    let mut slots =
        codegen::slot_map(file, &program).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    for function in &mut slots.functions {
        let (src_file, src_line) = map.resolve(function.line);
        function.src = format!("{src_file}:{src_line}");
        function.line = src_line;
    }
    Ok(slots)
}

/// Render a [`SlotMap`] as deterministic `key=value` lines: a `slot-map` summary
/// line (with the post-reuse `user_slots_used` peak) followed by one `fn` line per
/// function reporting its own footprint. Stable for the budget gate
/// (`harness/check_budgets.py`) and tests.
pub fn describe_slot_map(map: &SlotMap) -> String {
    let mut out = format!(
        "slot-map user_slots_used={} user_slots_cap={} scratch_slots={} vm_local_slots={}",
        map.user_slots_used, map.user_slots_cap, map.scratch_slots, map.vm_local_slots
    );
    for func in &map.functions {
        out.push_str(&format!(
            "\nfn {} params={} locals={} footprint={} src={}",
            func.name,
            func.params,
            func.locals,
            func.slots(),
            func.src,
        ));
    }
    out
}

/// Compile `source` to verified `KBC1` bytecode (baseline layout, both KOTO-0156
/// transforms off — see [`compile_with_options`] to opt in per app).
pub fn compile(file: &str, source: &str) -> Result<Vec<u8>, CompileError> {
    compile_with_options(file, source, CodegenOptions::default())
}

/// Compile `source` to verified `KBC1` bytecode with the KOTO-0156 code-window layout
/// options ([`CodegenOptions`]). `CodegenOptions::default()` (both off) reproduces the
/// baseline layout; equivalence tests compile a program both ways and assert identical
/// runtime behavior. The shipped per-app opt-in (apps.json) flows through here.
pub fn compile_with_options(
    file: &str,
    source: &str,
    options: CodegenOptions,
) -> Result<Vec<u8>, CompileError> {
    compile_with_loader(file, source, options, &mut FsLoader)
}

/// Like [`compile_with_options`], with include loading injected (KOTO-0183).
pub fn compile_with_loader(
    file: &str,
    source: &str,
    options: CodegenOptions,
    loader: &mut dyn IncludeLoader,
) -> Result<Vec<u8>, CompileError> {
    let asm = compile_to_asm_with_loader(file, source, options, loader)?;
    let bytecode = kbc_asm::assemble(&asm).map_err(|error| {
        CompileError::internal(
            file,
            format!("internal error: generated assembly did not assemble ({error})"),
        )
    })?;
    verify_kbc(&bytecode, RuntimeLimits::simulator_default()).map_err(|error| {
        CompileError::internal(
            file,
            format!("internal error: generated bytecode failed verification ({error:?})"),
        )
    })?;
    Ok(bytecode)
}

#[cfg(test)]
mod tests;
