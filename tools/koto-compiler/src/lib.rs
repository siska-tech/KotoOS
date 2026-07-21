//! `koto-compiler`: a host-side, ahead-of-time compiler from the Koto app
//! language (see `docs/spec/KOTO_APP_LANGUAGE.md`) to verifier-valid `KBC1` bytecode.
//!
//! The pipeline is `source -> lex -> parse -> codegen (assembly text) ->
//! kbc-asm -> KBC1`, with a final `verify_kbc` guarantee. It is a development
//! tool only; nothing here runs on the device.

mod assets;
mod codegen;
mod lexer;
mod parser;
mod preprocess;

use koto_core::{verify_kbc, RuntimeLimits};

pub use assets::{AssetResolver, ManifestAssets};
/// Preferred name for [`IncludeLoader`] in editor/tooling APIs.
pub use preprocess::IncludeLoader as IncludeResolver;
pub use preprocess::{FsLoader, IncludeLoader, OverlayLoader, SourceMap};

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

/// Diagnostic severity. The compiler currently emits errors only; the enum is
/// intentionally ready for future lint/warning passes without changing the
/// LSP-facing data shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
}

/// One 1-based source position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
}

/// A half-open source span with its author-facing file after include mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: String,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

impl SourceSpan {
    fn mapped(map: &SourceMap, line: usize, col: usize, len: usize) -> Self {
        let (file, line) = map.resolve(line);
        Self {
            file: file.to_string(),
            start: SourcePosition { line, column: col },
            end: SourcePosition {
                line,
                column: col.saturating_add(len.max(1)),
            },
        }
    }
}

/// Structured compiler diagnostic suitable for an editor or language server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    /// Internal pipeline failures have no author-source span.
    pub span: Option<SourceSpan>,
}

impl Diagnostic {
    fn from_compile_error(error: CompileError) -> Self {
        let span = (error.line != 0 && error.col != 0).then(|| SourceSpan {
            file: error.file,
            start: SourcePosition {
                line: error.line,
                column: error.col,
            },
            end: SourcePosition {
                line: error.line,
                column: error.col.saturating_add(1),
            },
        });
        Self {
            severity: DiagnosticSeverity::Error,
            message: error.message,
            span,
        }
    }

    fn from_mapped_diag(map: &SourceMap, diag: Diag) -> Self {
        Self::from_compile_error(CompileError::from_diag_mapped(map, diag))
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.span {
            Some(span) => write!(
                f,
                "{}:{}:{}: {}",
                span.file, span.start.line, span.start.column, self.message
            ),
            None => f.write_str(&self.message),
        }
    }
}

/// Koto definitions exposed to tooling without leaking the compiler AST.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Constant,
    Enum,
    EnumMember,
    Data,
    Struct,
    Static,
    Field,
    Function,
    Method,
    Parameter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolType {
    Int,
    Bool,
    Struct(String),
}

impl std::fmt::Display for SymbolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Struct(name) => name,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolParameter {
    pub name: String,
    pub ty: SymbolType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolDetail {
    Constant {
        value: i64,
    },
    Enum {
        members: usize,
    },
    EnumMember {
        value: i64,
    },
    Data {
        element_bits: u8,
        elements: usize,
    },
    Struct {
        fields: usize,
        bytes: usize,
    },
    Static {
        ty: SymbolType,
        bytes: usize,
    },
    Field {
        ty: SymbolType,
        offset: usize,
    },
    /// A fixed-capacity buffer field in a struct layout (KOTO-0235): hover
    /// shows the region capacity and offset instead of a 32-bit scalar type.
    BufferField {
        capacity: usize,
        offset: usize,
    },
    Function {
        parameters: Vec<SymbolParameter>,
        return_type: Option<SymbolType>,
    },
    Parameter {
        ty: SymbolType,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Function name for a parameter; `None` for top-level definitions.
    pub container: Option<String>,
    pub definition: SourceSpan,
    pub detail: SymbolDetail,
}

/// Inputs for the editor-oriented compilation API. The root source is passed
/// explicitly, so it may be newer than the copy on disk.
#[derive(Clone, Copy, Debug)]
pub struct CompileRequest<'a> {
    pub file: &'a str,
    pub source: &'a str,
    pub options: CodegenOptions,
}

impl<'a> CompileRequest<'a> {
    pub fn new(file: &'a str, source: &'a str) -> Self {
        Self {
            file,
            source,
            options: CodegenOptions::default(),
        }
    }
}

/// Complete value result for one compiler pass. On failure, generated values
/// that depend on the failing stage are `None`, while any symbols already
/// parsed remain available alongside structured diagnostics.
#[derive(Clone, Debug)]
pub struct Compilation {
    pub bytecode: Option<Vec<u8>>,
    pub assembly: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub slot_map: Option<SlotMap>,
    pub symbols: Vec<Symbol>,
}

impl Compilation {
    pub fn succeeded(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error)
    }

    fn failed(diagnostic: Diagnostic) -> Self {
        Self {
            bytecode: None,
            assembly: None,
            diagnostics: vec![diagnostic],
            slot_map: None,
            symbols: Vec::new(),
        }
    }
}

/// Compile `source` to assembly text (the KOTO-0044 IR). Useful for debugging
/// and for golden tests of code generation.
pub fn compile_to_asm(file: &str, source: &str) -> Result<String, CompileError> {
    compile_to_asm_with_options(file, source, CodegenOptions::default())
}

pub use codegen::CodegenOptions;

/// Editor-facing metadata for host-call-backed KotoUI prelude functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdkFunction {
    pub name: &'static str,
    pub parameters: &'static [&'static str],
    pub returns: &'static str,
}

pub const UI_SDK_FUNCTIONS: &[SdkFunction] = &[
    SdkFunction {
        name: "ui_capabilities",
        parameters: &["dst", "max_len"],
        returns: "int",
    },
    SdkFunction {
        name: "ui_mount",
        parameters: &["src", "len"],
        returns: "int",
    },
    SdkFunction {
        name: "ui_update",
        parameters: &["src", "len"],
        returns: "int",
    },
    SdkFunction {
        name: "ui_present",
        parameters: &[],
        returns: "int",
    },
    SdkFunction {
        name: "ui_poll_event",
        parameters: &["dst", "max_len"],
        returns: "int",
    },
    SdkFunction {
        name: "ui_reset",
        parameters: &[],
        returns: "int",
    },
];

/// Compile an in-memory root source with injected include resolution and
/// return every value needed by editor tooling in one pass. Compile-time asset
/// helpers resolve against the nearest `app.json` above `request.file`
/// (KOTO-0236/0237); see [`compile_source_with_assets`] to inject the asset
/// namespace instead.
pub fn compile_source(
    request: CompileRequest<'_>,
    resolver: &mut dyn IncludeResolver,
) -> Compilation {
    let mut assets = ManifestAssets::for_root(request.file);
    compile_source_with_assets(request, resolver, &mut assets)
}

/// Like [`compile_source`], with compile-time asset resolution injected
/// alongside include loading so tests and editors can fold asset sizes and
/// text shape without an on-disk `app.json`.
pub fn compile_source_with_assets(
    request: CompileRequest<'_>,
    resolver: &mut dyn IncludeResolver,
    assets: &mut dyn AssetResolver,
) -> Compilation {
    let (expanded, map) = match preprocess::expand(request.file, request.source, resolver) {
        Ok(value) => value,
        Err(error) => return Compilation::failed(Diagnostic::from_compile_error(error)),
    };
    let tokens = match lexer::lex(&expanded) {
        Ok(tokens) => tokens,
        Err(diag) => return Compilation::failed(Diagnostic::from_mapped_diag(&map, diag)),
    };
    let program = match parser::parse(&tokens, assets) {
        Ok(program) => program,
        Err(diag) => return Compilation::failed(Diagnostic::from_mapped_diag(&map, diag)),
    };
    let symbols = collect_symbols(&program, &map);
    let assembly = match codegen::compile_to_asm_with(request.file, &program, request.options) {
        Ok(assembly) => assembly,
        Err(diag) => {
            let mut failed = Compilation::failed(Diagnostic::from_mapped_diag(&map, diag));
            failed.symbols = symbols;
            return failed;
        }
    };
    let mut slots = match codegen::slot_map(request.file, &program) {
        Ok(slots) => slots,
        Err(diag) => {
            let mut failed = Compilation::failed(Diagnostic::from_mapped_diag(&map, diag));
            failed.assembly = Some(assembly);
            failed.symbols = symbols;
            return failed;
        }
    };
    map_slot_sources(&mut slots, &map);
    let bytecode = match kbc_asm::assemble(&assembly) {
        Ok(bytecode) => bytecode,
        Err(error) => {
            let mut failed =
                Compilation::failed(Diagnostic::from_compile_error(CompileError::internal(
                    request.file,
                    format!("internal error: generated assembly did not assemble ({error})"),
                )));
            failed.assembly = Some(assembly);
            failed.slot_map = Some(slots);
            failed.symbols = symbols;
            return failed;
        }
    };
    if let Err(error) = verify_kbc(&bytecode, RuntimeLimits::simulator_default()) {
        let mut failed =
            Compilation::failed(Diagnostic::from_compile_error(CompileError::internal(
                request.file,
                format!("internal error: generated bytecode failed verification ({error:?})"),
            )));
        failed.assembly = Some(assembly);
        failed.slot_map = Some(slots);
        failed.symbols = symbols;
        return failed;
    }
    Compilation {
        bytecode: Some(bytecode),
        assembly: Some(assembly),
        diagnostics: Vec::new(),
        slot_map: Some(slots),
        symbols,
    }
}

/// The declaration-ordered byte size of a struct layout: 4 bytes per scalar
/// field plus each buffer field's declared capacity (KOTO-0235).
fn struct_layout_bytes(def: &parser::StructDef) -> usize {
    def.fields
        .iter()
        .map(|field| match &field.kind {
            parser::StructFieldKind::Scalar(_) => 4,
            parser::StructFieldKind::Buffer(capacity) => *capacity,
        })
        .sum()
}

fn collect_symbols(program: &parser::Program, map: &SourceMap) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (enum_name, members) in codegen::sdk_enums() {
        let definition = SourceSpan {
            file: "<KotoSDK>".to_string(),
            start: SourcePosition { line: 1, column: 1 },
            end: SourcePosition {
                line: 1,
                column: enum_name.len() + 1,
            },
        };
        symbols.push(Symbol {
            name: enum_name.to_string(),
            kind: SymbolKind::Enum,
            container: None,
            definition: definition.clone(),
            detail: SymbolDetail::Enum {
                members: members.len(),
            },
        });
        for (member, value) in members {
            symbols.push(Symbol {
                name: member.to_string(),
                kind: SymbolKind::EnumMember,
                container: Some(enum_name.to_string()),
                definition: definition.clone(),
                detail: SymbolDetail::EnumMember { value },
            });
        }
    }
    for def in &program.consts {
        symbols.push(Symbol {
            name: def.name.clone(),
            kind: SymbolKind::Constant,
            container: None,
            definition: SourceSpan::mapped(map, def.name_line, def.name_col, def.name.len()),
            detail: SymbolDetail::Constant { value: def.value },
        });
    }
    for def in &program.enums {
        symbols.push(Symbol {
            name: def.name.clone(),
            kind: SymbolKind::Enum,
            container: None,
            definition: SourceSpan::mapped(map, def.name_line, def.name_col, def.name.len()),
            detail: SymbolDetail::Enum {
                members: def.members.len(),
            },
        });
        for member in &def.members {
            symbols.push(Symbol {
                name: member.name.clone(),
                kind: SymbolKind::EnumMember,
                container: Some(def.name.clone()),
                definition: SourceSpan::mapped(
                    map,
                    member.name_line,
                    member.name_col,
                    member.name.len(),
                ),
                detail: SymbolDetail::EnumMember {
                    value: member.value,
                },
            });
        }
    }
    for def in &program.data {
        symbols.push(Symbol {
            name: def.name.clone(),
            kind: SymbolKind::Data,
            container: None,
            definition: SourceSpan::mapped(map, def.name_line, def.name_col, def.name.len()),
            detail: SymbolDetail::Data {
                element_bits: match def.width {
                    parser::DataWidth::U8 => 8,
                    parser::DataWidth::U16 => 16,
                },
                elements: def.values.len(),
            },
        });
    }
    for def in &program.structs {
        symbols.push(Symbol {
            name: def.name.clone(),
            kind: SymbolKind::Struct,
            container: None,
            definition: SourceSpan::mapped(map, def.name_line, def.name_col, def.name.len()),
            detail: SymbolDetail::Struct {
                fields: def.fields.len(),
                bytes: struct_layout_bytes(def),
            },
        });
        let mut offset = 0usize;
        for field in &def.fields {
            let (detail, bytes) = match &field.kind {
                parser::StructFieldKind::Scalar(ty) => (
                    SymbolDetail::Field {
                        ty: symbol_type(ty.clone()),
                        offset,
                    },
                    4,
                ),
                parser::StructFieldKind::Buffer(capacity) => (
                    SymbolDetail::BufferField {
                        capacity: *capacity,
                        offset,
                    },
                    *capacity,
                ),
            };
            symbols.push(Symbol {
                name: field.name.clone(),
                kind: SymbolKind::Field,
                container: Some(def.name.clone()),
                definition: SourceSpan::mapped(
                    map,
                    field.name_line,
                    field.name_col,
                    field.name.len(),
                ),
                detail,
            });
            offset += bytes;
        }
    }
    for def in &program.statics {
        let bytes = program
            .structs
            .iter()
            .find(|item| item.name == def.ty)
            .map(struct_layout_bytes)
            .unwrap_or(0);
        symbols.push(Symbol {
            name: def.name.clone(),
            kind: SymbolKind::Static,
            container: None,
            definition: SourceSpan::mapped(map, def.name_line, def.name_col, def.name.len()),
            detail: SymbolDetail::Static {
                ty: SymbolType::Struct(def.ty.clone()),
                bytes,
            },
        });
    }
    for function in &program.functions {
        symbols.push(Symbol {
            name: function.name.clone(),
            kind: SymbolKind::Function,
            container: None,
            definition: SourceSpan::mapped(
                map,
                function.name_line,
                function.name_col,
                function.name.len(),
            ),
            detail: SymbolDetail::Function {
                parameters: function
                    .params
                    .iter()
                    .map(|param| SymbolParameter {
                        name: param.name.clone(),
                        ty: symbol_type(param.ty.clone()),
                    })
                    .collect(),
                return_type: function.ret.clone().map(symbol_type),
            },
        });
        for param in &function.params {
            symbols.push(Symbol {
                name: param.name.clone(),
                kind: SymbolKind::Parameter,
                container: Some(function.name.clone()),
                definition: SourceSpan::mapped(map, param.line, param.col, param.name.len()),
                detail: SymbolDetail::Parameter {
                    ty: symbol_type(param.ty.clone()),
                },
            });
        }
    }
    for method in &program.methods {
        let function = &method.function;
        symbols.push(Symbol {
            name: function.name.clone(),
            kind: SymbolKind::Method,
            container: Some(method.target.clone()),
            definition: SourceSpan::mapped(
                map,
                function.name_line,
                function.name_col,
                function.name.len(),
            ),
            detail: SymbolDetail::Function {
                parameters: function
                    .params
                    .iter()
                    .skip(1)
                    .map(|param| SymbolParameter {
                        name: param.name.clone(),
                        ty: symbol_type(param.ty.clone()),
                    })
                    .collect(),
                return_type: function.ret.clone().map(symbol_type),
            },
        });
        let container = format!("{}::{}", method.target, function.name);
        for param in &function.params {
            symbols.push(Symbol {
                name: param.name.clone(),
                kind: SymbolKind::Parameter,
                container: Some(container.clone()),
                definition: SourceSpan::mapped(map, param.line, param.col, param.name.len()),
                detail: SymbolDetail::Parameter {
                    ty: symbol_type(param.ty.clone()),
                },
            });
        }
    }
    symbols
}

fn symbol_type(ty: parser::Type) -> SymbolType {
    match ty {
        parser::Type::Int => SymbolType::Int,
        parser::Type::Bool => SymbolType::Bool,
        parser::Type::Struct(name) => SymbolType::Struct(name),
    }
}

fn map_slot_sources(slots: &mut SlotMap, map: &SourceMap) {
    for function in &mut slots.functions {
        let (src_file, src_line) = map.resolve(function.line);
        function.src = format!("{src_file}:{src_line}");
        function.line = src_line;
    }
}

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
    let mut assets = ManifestAssets::for_root(file);
    compile_to_asm_with_resolvers(file, source, options, loader, &mut assets)
}

/// Like [`compile_to_asm_with_loader`], with compile-time asset resolution
/// also injected (KOTO-0236/0237).
pub fn compile_to_asm_with_resolvers(
    file: &str,
    source: &str,
    options: CodegenOptions,
    loader: &mut dyn IncludeLoader,
    assets: &mut dyn AssetResolver,
) -> Result<String, CompileError> {
    let (expanded, map) = preprocess::expand(file, source, loader)?;
    let tokens = lexer::lex(&expanded).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
    let program =
        parser::parse(&tokens, assets).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
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
    let mut assets = ManifestAssets::for_root(file);
    let program =
        parser::parse(&tokens, &mut assets).map_err(|d| CompileError::from_diag_mapped(&map, d))?;
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
/// runtime behavior. The shipped per-app opt-in (app.json) flows through here.
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
    let mut assets = ManifestAssets::for_root(file);
    compile_with_resolvers(file, source, options, loader, &mut assets)
}

/// Like [`compile_with_loader`], with compile-time asset resolution also
/// injected (KOTO-0236/0237) so hermetic tests can name package assets without
/// a manifest.
pub fn compile_with_resolvers(
    file: &str,
    source: &str,
    options: CodegenOptions,
    loader: &mut dyn IncludeLoader,
    assets: &mut dyn AssetResolver,
) -> Result<Vec<u8>, CompileError> {
    let asm = compile_to_asm_with_resolvers(file, source, options, loader, assets)?;
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
