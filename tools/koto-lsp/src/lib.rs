//! Protocol-independent language intelligence for KOTO-0194.

use koto_compiler::{
    compile_source, Compilation, CompileRequest, Diagnostic, IncludeResolver, SlotMap, SourceSpan,
    Symbol, SymbolDetail, SymbolKind, UI_SDK_FUNCTIONS,
};

#[derive(Clone, Debug)]
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    pub slot_map: Option<SlotMap>,
    pub symbols: Vec<Symbol>,
}

pub fn analyze(root_file: &str, root_source: &str, resolver: &mut dyn IncludeResolver) -> Analysis {
    let Compilation {
        diagnostics,
        slot_map,
        symbols,
        ..
    } = compile_source(CompileRequest::new(root_file, root_source), resolver);
    Analysis {
        diagnostics,
        slot_map,
        symbols,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetInlay {
    pub used: usize,
    pub capacity: usize,
    pub warning: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    /// LSP CompletionItemKind: 3 function, 6 variable, 21 constant.
    pub kind: u8,
}

pub fn completion_items(
    analysis: &Analysis,
    source: &str,
    line: usize,
    character: usize,
) -> Vec<CompletionItem> {
    let prefix = word_prefix_at(source, line, character).unwrap_or("");
    let enum_container = enum_prefix_at(source, line, character);
    let receiver_container = receiver_type_at(&analysis.symbols, source, line, character);
    let mut items = Vec::new();
    for symbol in analysis
        .symbols
        .iter()
        .filter(
            |symbol| match (enum_container, receiver_container.as_deref()) {
                (Some(container), _) => {
                    symbol.container.as_deref() == Some(container)
                        && symbol.kind == SymbolKind::EnumMember
                }
                (None, Some(container)) => {
                    symbol.container.as_deref() == Some(container)
                        && matches!(symbol.kind, SymbolKind::Field | SymbolKind::Method)
                }
                (None, None) => symbol.container.is_none(),
            },
        )
        .filter(|symbol| symbol.name.starts_with(prefix))
    {
        let (detail, kind) = match &symbol.detail {
            SymbolDetail::Function {
                parameters,
                return_type,
            } => {
                let params = parameters
                    .iter()
                    .map(|param| format!("{}: {}", param.name, param.ty))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = return_type
                    .as_ref()
                    .map(|ty| format!(" -> {ty}"))
                    .unwrap_or_default();
                (format!("fn {}({params}){ret}", symbol.name), 3)
            }
            SymbolDetail::Constant { value } => (format!("const {} = {value}", symbol.name), 21),
            SymbolDetail::Enum { members } => {
                (format!("enum {} ({members} members)", symbol.name), 13)
            }
            SymbolDetail::EnumMember { value } => (format!("{} = {value}", symbol.name), 20),
            SymbolDetail::Data { elements, .. } => {
                (format!("data {} ({elements} elements)", symbol.name), 6)
            }
            SymbolDetail::Struct { fields, bytes } => (
                format!("struct {} ({fields} fields, {bytes} bytes)", symbol.name),
                22,
            ),
            SymbolDetail::Static { ty, bytes } => {
                (format!("static {}: {ty} ({bytes} bytes)", symbol.name), 6)
            }
            SymbolDetail::Field { ty, offset } => (format!("{}: {ty} (+{offset})", symbol.name), 5),
            SymbolDetail::BufferField { capacity, offset } => {
                (format!("{}: buf[{capacity}] (+{offset})", symbol.name), 5)
            }
            SymbolDetail::Parameter { .. } => continue,
        };
        items.push(CompletionItem {
            label: symbol.name.clone(),
            detail,
            kind,
        });
    }
    for function in UI_SDK_FUNCTIONS
        .iter()
        .filter(|function| function.name.starts_with(prefix))
    {
        if items.iter().any(|item| item.label == function.name) {
            continue;
        }
        let params = function.parameters.join(", ");
        items.push(CompletionItem {
            label: function.name.to_string(),
            detail: format!(
                "fn {}({params}) -> {} [KotoSDK]",
                function.name, function.returns
            ),
            kind: 3,
        });
    }
    items.sort_by(|a, b| a.label.cmp(&b.label));
    items
}

pub fn definition_at(
    analysis: &Analysis,
    source: &str,
    line: usize,
    character: usize,
) -> Option<Definition> {
    let word = word_at(source, line, character)?;
    let symbol = symbol_for_position(&analysis.symbols, source, line, character, word)?;
    Some(Definition {
        span: symbol.definition.clone(),
    })
}

pub fn hover_at(analysis: &Analysis, source: &str, line: usize, character: usize) -> Option<Hover> {
    let word = word_at(source, line, character)?;
    let symbol = symbol_for_position(&analysis.symbols, source, line, character, word)?;
    let mut markdown = match &symbol.detail {
        SymbolDetail::Constant { value } => {
            format!("```koto\nconst {} = {value}\n```", symbol.name)
        }
        SymbolDetail::Enum { members } => format!(
            "```koto\nenum {} {{ ... }}\n```\n\n{members} members; integer-backed",
            symbol.name
        ),
        SymbolDetail::EnumMember { value } => format!(
            "```koto\n{}::{} = {value}\n```\n\nCompile-time `int` constant",
            symbol.container.as_deref().unwrap_or("enum"),
            symbol.name
        ),
        SymbolDetail::Data {
            element_bits,
            elements,
        } => format!(
            "```koto\ndata {} = u{element_bits}[...];\n```\n\n{elements} elements",
            symbol.name
        ),
        SymbolDetail::Struct { fields, bytes } => format!(
            "```koto\nstruct {} {{ ... }}\n```\n\n{fields} fields; {bytes} static bytes",
            symbol.name
        ),
        SymbolDetail::Static { ty, bytes } => format!(
            "```koto\nstatic {}: {ty}\n```\n\nOne App-lifetime record; {bytes} heap bytes",
            symbol.name
        ),
        SymbolDetail::Field { ty, offset } => format!(
            "```koto\n{}: {ty}\n```\n\n32-bit field at byte offset {offset}",
            symbol.name
        ),
        SymbolDetail::BufferField { capacity, offset } => format!(
            "```koto\n{}: buf[{capacity}]\n```\n\n{capacity}-byte buffer field at byte offset {offset}; reads as its region address, `len` folds to {capacity}",
            symbol.name
        ),
        SymbolDetail::Function {
            parameters,
            return_type,
        } => {
            let params = parameters
                .iter()
                .map(|param| format!("{}: {}", param.name, param.ty))
                .collect::<Vec<_>>()
                .join(", ");
            let ret = return_type
                .as_ref()
                .map(|ty| format!(" -> {ty}"))
                .unwrap_or_default();
            format!("```koto\nfn {}({params}){ret}\n```", symbol.name)
        }
        SymbolDetail::Parameter { ty } => format!("```koto\n{}: {ty}\n```", symbol.name),
    };
    if symbol.kind == SymbolKind::Function {
        if let Some(slots) = analysis.slot_map.as_ref().and_then(|map| {
            map.functions
                .iter()
                .find(|function| function.name == symbol.name)
        }) {
            markdown.push_str(&format!(
                "\n\nSlot footprint: **{}** ({} params + {} locals)",
                slots.slots(),
                slots.params,
                slots.locals
            ));
        }
    }
    Some(Hover {
        markdown,
        span: symbol.definition.clone(),
    })
}

pub fn budget_inlay(analysis: &Analysis) -> Option<BudgetInlay> {
    let map = analysis.slot_map.as_ref()?;
    Some(BudgetInlay {
        used: map.user_slots_used,
        capacity: map.user_slots_cap,
        warning: map.user_slots_used * 100 >= map.user_slots_cap * 90,
    })
}

fn symbol_for_word<'a>(symbols: &'a [Symbol], word: &str) -> Option<&'a Symbol> {
    symbols
        .iter()
        .find(|symbol| symbol.name == word && symbol.kind != SymbolKind::Parameter)
        .or_else(|| symbols.iter().find(|symbol| symbol.name == word))
}

fn symbol_for_position<'a>(
    symbols: &'a [Symbol],
    source: &str,
    line: usize,
    character: usize,
    word: &str,
) -> Option<&'a Symbol> {
    if let Some(container) = enum_prefix_at(source, line, character) {
        if let Some(symbol) = symbols
            .iter()
            .find(|symbol| symbol.name == word && symbol.container.as_deref() == Some(container))
        {
            return Some(symbol);
        }
    }
    if let Some(container) = receiver_type_at(symbols, source, line, character) {
        if let Some(symbol) = symbols
            .iter()
            .find(|symbol| symbol.name == word && symbol.container.as_deref() == Some(&container))
        {
            return Some(symbol);
        }
    }
    symbol_for_word(symbols, word)
}

fn receiver_type_at(
    symbols: &[Symbol],
    source: &str,
    line: usize,
    character: usize,
) -> Option<String> {
    let text = source.lines().nth(line)?;
    let byte = utf16_column_to_byte(text, character).min(text.len());
    let before = &text[..byte];
    let dot = before.rfind('.')?;
    let bytes = before.as_bytes();
    let mut start = dot;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let receiver = &before[start..dot];
    let symbol = symbols.iter().find(|symbol| {
        symbol.name == receiver && matches!(symbol.kind, SymbolKind::Static | SymbolKind::Parameter)
    })?;
    match &symbol.detail {
        SymbolDetail::Static {
            ty: koto_compiler::SymbolType::Struct(name),
            ..
        }
        | SymbolDetail::Parameter {
            ty: koto_compiler::SymbolType::Struct(name),
        } => Some(name.clone()),
        _ => None,
    }
}

fn enum_prefix_at(source: &str, line: usize, character: usize) -> Option<&str> {
    let text = source.lines().nth(line)?;
    let byte = utf16_column_to_byte(text, character).min(text.len());
    let before = &text[..byte];
    let separator = before.rfind("::")?;
    let bytes = before.as_bytes();
    let mut start = separator;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    (start < separator).then(|| &before[start..separator])
}

/// Extract an ASCII Koto identifier at an LSP (0-based, UTF-16) position.
pub fn word_at(source: &str, line: usize, character: usize) -> Option<&str> {
    let text = source.lines().nth(line)?;
    let byte = utf16_column_to_byte(text, character);
    let bytes = text.as_bytes();
    let mut start = byte.min(bytes.len());
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = byte.min(bytes.len());
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    (start < end).then(|| &text[start..end])
}

fn word_prefix_at(source: &str, line: usize, character: usize) -> Option<&str> {
    let text = source.lines().nth(line)?;
    let byte = utf16_column_to_byte(text, character).min(text.len());
    let bytes = text.as_bytes();
    let mut start = byte;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    Some(&text[start..byte])
}

fn utf16_column_to_byte(text: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= target {
            return byte;
        }
        units += ch.len_utf16();
        if units > target {
            return byte;
        }
    }
    text.len()
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_compiler::{IncludeLoader, OverlayLoader};
    use std::path::Path;

    struct Missing;

    impl IncludeLoader for Missing {
        fn load(&mut self, path: &Path) -> Result<String, String> {
            Err(format!("missing {}", path.display()))
        }
    }

    fn included_analysis() -> (Analysis, String) {
        let root =
            "include \"util.koto\";\nconst LIMIT = 42;\nfn main() { exit(helper(LIMIT)); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert(
            "util.koto",
            "fn helper(value: int) -> int { let copy = value; return copy; }\n",
        );
        (analyze("main.koto", root, &mut resolver), root.to_string())
    }

    fn insert_koto_ui_sdk(resolver: &mut OverlayLoader<Missing>) {
        resolver.insert(
            "sdk/koto_ui.koto",
            include_str!("../../../sdk/koto_ui.koto"),
        );
        resolver.insert(
            "sdk/koto_ui/abi.koto",
            include_str!("../../../sdk/koto_ui/abi.koto"),
        );
        resolver.insert(
            "sdk/koto_ui/resources.koto",
            include_str!("../../../sdk/koto_ui/resources.koto"),
        );
        resolver.insert(
            "sdk/koto_ui/builders.koto",
            include_str!("../../../sdk/koto_ui/builders.koto"),
        );
        resolver.insert(
            "sdk/koto_ui/events_locale.koto",
            include_str!("../../../sdk/koto_ui/events_locale.koto"),
        );
    }

    #[test]
    fn definition_crosses_include_boundary() {
        let (analysis, root) = included_analysis();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let definition = definition_at(&analysis, &root, 2, 19).expect("helper definition");
        assert_eq!(definition.span.file, "util.koto");
        assert_eq!(
            (definition.span.start.line, definition.span.start.column),
            (1, 4)
        );
    }

    #[test]
    fn hover_reports_signature_footprint_and_const_value() {
        let (analysis, root) = included_analysis();
        let helper = hover_at(&analysis, &root, 2, 19).expect("helper hover");
        assert!(helper.markdown.contains("fn helper(value: int) -> int"));
        assert!(helper.markdown.contains("Slot footprint: **2**"));
        let constant = hover_at(&analysis, &root, 2, 26).expect("const hover");
        assert!(constant.markdown.contains("const LIMIT = 42"));
    }

    #[test]
    fn budget_inlay_matches_slot_map_and_warns_at_ninety_percent() {
        let (analysis, _) = included_analysis();
        let inlay = budget_inlay(&analysis).expect("budget inlay");
        let slots = analysis.slot_map.as_ref().unwrap();
        assert_eq!((inlay.used, inlay.capacity), (slots.user_slots_used, 45));
        assert!(!inlay.warning);

        let mut pressure = analysis;
        pressure.slot_map.as_mut().unwrap().user_slots_used = 41;
        assert!(budget_inlay(&pressure).unwrap().warning);
    }

    #[test]
    fn utf16_position_finds_identifier_after_japanese_text() {
        let source = "draw_text(0, 0, \"日本\"); helper();\n";
        // Japanese BMP characters each occupy one UTF-16 unit.
        assert_eq!(word_at(source, 0, 24), Some("helper"));
    }

    #[test]
    fn unsaved_overlay_diagnostic_maps_to_included_file() {
        let root = "include \"util.koto\";\nfn main() { exit(0); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert("util.koto", "fn broken( { }\n");
        let analysis = analyze("main.koto", root, &mut resolver);
        let span = analysis.diagnostics[0].span.as_ref().unwrap();
        assert_eq!(span.file, "util.koto");
        assert_eq!(span.start.line, 1);
    }

    #[test]
    fn asset_consts_analyze_and_hover_with_their_folded_values() {
        // KOTO-0236: the editor compiles real on-disk paths, so `asset_len`
        // discovers the nearest app.json above the (possibly unsaved) root
        // source and hover/inlay simply see an ordinary folded const.
        let dir = std::env::temp_dir().join("koto_lsp_asset_len_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        std::fs::write(dir.join("locales").join("en-US.txt"), b"hello\nworld\n").unwrap();
        std::fs::write(dir.join("locales").join("ja-JP.txt"), "日本語\n".as_bytes()).unwrap();
        std::fs::write(
            dir.join("app.json"),
            br#"{"assets":[{"source":"locales/en-US.txt"},{"source":"locales/ja-JP.txt"}]}"#,
        )
        .unwrap();
        let root = concat!(
            "const RAW = asset_len(\"locales/en-US.txt\");\n",
            "const LINES = asset_text_line_count(\"locales/en-US.txt\");\n",
            "const TEXT = asset_text_max_range_bytes(0, LINES, \"locales/en-US.txt\");\n",
            "const WIDEST = asset_text_max_line_bytes(0, LINES, \"locales/en-US.txt\");\n",
            "const TOTAL = TEXT + WIDEST - 1;\n",
            "fn main() { buf raw[RAW]; exit(len(raw)); }\n",
        );
        let root_file = dir.join("src").join("main.koto");
        let root_file = root_file.to_str().unwrap();

        let analysis = analyze(root_file, root, &mut OverlayLoader::new(Missing));
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let constant = hover_at(&analysis, root, 0, 7).expect("const hover");
        assert!(constant.markdown.contains("const RAW = 12"), "{constant:?}");
        let lines = hover_at(&analysis, root, 1, 7).expect("line count hover");
        assert!(lines.markdown.contains("const LINES = 2"), "{lines:?}");
        let text = hover_at(&analysis, root, 2, 7).expect("range bytes hover");
        assert!(text.markdown.contains("const TEXT = 10"), "{text:?}");
        // KOTO-0238: the line-maximum helper and additive chains hover with
        // their folded values through the same const path.
        let widest = hover_at(&analysis, root, 3, 7).expect("line max hover");
        assert!(widest.markdown.contains("const WIDEST = 5"), "{widest:?}");
        let total = hover_at(&analysis, root, 4, 7).expect("additive hover");
        assert!(total.markdown.contains("const TOTAL = 14"), "{total:?}");
        assert!(budget_inlay(&analysis).is_some());

        // An undeclared path surfaces as an ordinary mapped diagnostic.
        let broken = "const RAW = asset_len(\"locales/nope.txt\");\nfn main() { exit(RAW); }\n";
        let analysis = analyze(root_file, broken, &mut OverlayLoader::new(Missing));
        let diagnostic = &analysis.diagnostics[0];
        assert!(
            diagnostic.message.contains("locales/nope.txt"),
            "{diagnostic:?}"
        );
        let mismatched = concat!(
            "const LINES = asset_text_line_count(\"locales/en-US.txt\", ",
            "\"locales/ja-JP.txt\");\nfn main() {}\n",
        );
        let analysis = analyze(root_file, mismatched, &mut OverlayLoader::new(Missing));
        assert!(
            analysis.diagnostics[0]
                .message
                .contains("has 1 lines; expected 2"),
            "{:?}",
            analysis.diagnostics
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completion_combines_ui_intrinsics_and_included_builders() {
        let root = r#"include <sdk/koto_ui.koto>;
static mount_builder: UiMountBuilder = {
    packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
    data_offset: 0, data_cursor: 0, status: 0, active: false,
};
static text_resource: TextResource = {
    storage: 0, capacity: 0, line_capacity: 0, line_count: 0,
    payload_offset: 0, payload_len: 0, status: 0, complete: false,
};
static list_rows: UiListRowsBuilder = {
    blob: 0, capacity: 0, row_capacity: 0, row_count: 0,
    label_cursor: 0, status: 0, active: false, complete: false,
};
static update_builder: UiUpdateBuilder = {
    packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
    data_offset: 0, data_cursor: 0, status: 0, active: false,
};
fn main() { let ui_ = 0; mount_builder.finish(); text_resource.count(); list_rows.finish(); update_builder.submit(); ui_present(); }
"#;
        let mut resolver = OverlayLoader::new(Missing);
        insert_koto_ui_sdk(&mut resolver);
        let analysis = analyze("main.koto", root, &mut resolver);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let character = root.lines().nth(17).unwrap().find("ui_").unwrap() + 3;
        let items = completion_items(&analysis, root, 17, character);
        assert!(items.iter().any(|item| item.label == "ui_mount"));
        assert!(items.iter().any(|item| item.label == "ui_mount_add_button"));
        assert!(items.iter().any(|item| item.label == "ui_update_set_text"));
        assert!(items
            .iter()
            .any(|item| item.label == "ui_capabilities_validate"));

        let receiver_source = "fn main() { mount_builder.fi }";
        let receiver_col =
            receiver_source.find("mount_builder.fi").unwrap() + "mount_builder.fi".len();
        let receiver_items = completion_items(&analysis, receiver_source, 0, receiver_col);
        assert!(receiver_items.iter().any(|item| item.label == "finish"));

        let text_source = "fn main() { text_resource.line_ }";
        let text_col =
            text_source.find("text_resource.line_").unwrap() + "text_resource.line_".len();
        let text_items = completion_items(&analysis, text_source, 0, text_col);
        assert!(text_items.iter().any(|item| item.label == "line_ptr"));
        assert!(text_items.iter().any(|item| item.label == "line_len"));

        let rows_source = "fn main() { list_rows.resource_ }";
        let rows_col =
            rows_source.find("list_rows.resource_").unwrap() + "list_rows.resource_".len();
        let rows_items = completion_items(&analysis, rows_source, 0, rows_col);
        assert!(rows_items.iter().any(|item| item.label == "resource_row"));

        let update_source = "fn main() { update_builder.text_ }";
        let update_col =
            update_source.find("update_builder.text_").unwrap() + "update_builder.text_".len();
        let update_items = completion_items(&analysis, update_source, 0, update_col);
        assert!(update_items
            .iter()
            .any(|item| item.label == "text_resource"));
    }

    #[test]
    fn split_koto_ui_definitions_and_hover_point_to_owning_sources() {
        let root = r#"include <sdk/koto_ui.koto>;
static text: TextResource = {
    storage: 0, capacity: 0, line_capacity: 0, line_count: 0,
    payload_offset: 0, payload_len: 0, status: 0, complete: false,
};
static update: UiUpdateBuilder = {
    packet: 0, capacity: 0, record_capacity: 0, record_count: 0,
    data_offset: 0, data_cursor: 0, status: 0, active: false,
};
fn probe(a: int, b: int) { ui_mount_capacity(1, 0); ui_mount_begin(a, 40, 1, 40, 1, -1); text.count(); update.submit(); ui_locale_match(a, 2, b, 2); }
fn main() { exit(UiLocaleMatch::None); }
"#;
        let mut resolver = OverlayLoader::new(Missing);
        insert_koto_ui_sdk(&mut resolver);
        let analysis = analyze("main.koto", root, &mut resolver);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let line = root.lines().nth(9).unwrap();
        for (symbol, owner) in [
            ("ui_mount_capacity", "sdk/koto_ui/abi.koto"),
            ("ui_mount_begin", "sdk/koto_ui/abi.koto"),
            ("count", "sdk/koto_ui/resources.koto"),
            ("submit", "sdk/koto_ui/builders.koto"),
            ("ui_locale_match", "sdk/koto_ui/events_locale.koto"),
        ] {
            let column = line.find(symbol).unwrap() + 1;
            let definition = definition_at(&analysis, root, 9, column).expect(symbol);
            assert_eq!(definition.span.file, owner);
            assert!(
                hover_at(&analysis, root, 9, column).is_some(),
                "hover for {symbol}"
            );
        }
        let enum_line = root.lines().nth(10).unwrap();
        let enum_column = enum_line.find("UiLocaleMatch").unwrap() + 1;
        let definition = definition_at(&analysis, root, 10, enum_column).unwrap();
        assert_eq!(definition.span.file, "sdk/koto_ui/events_locale.koto");
    }

    #[test]
    fn helper_sized_buffers_and_len_analyze_clean_with_call_site_definitions() {
        // KOTO-0233: sizing facts live on the declaration; `len(packet)` feeds
        // the builder call. The helper keeps its SDK definition target from the
        // buf-size position.
        let root = r#"include <sdk/koto_ui.koto>;
fn main() {
    buf packet[ui_mount_capacity(1, 0)];
    let status = ui_mount_begin(packet, len(packet), 1, 88, 1, -1);
    exit(status);
}
"#;
        let mut resolver = OverlayLoader::new(Missing);
        insert_koto_ui_sdk(&mut resolver);
        let analysis = analyze("main.koto", root, &mut resolver);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let buf_line = root.lines().nth(2).unwrap();
        let column = buf_line.find("ui_mount_capacity").unwrap() + 1;
        let definition = definition_at(&analysis, root, 2, column).expect("helper definition");
        assert_eq!(definition.span.file, "sdk/koto_ui/abi.koto");
    }

    #[test]
    fn len_and_buf_capacity_diagnostics_map_to_unsaved_overlay_files() {
        let root = "include \"helper.koto\";\nfn main() { broken(); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert("helper.koto", "fn broken() { exit(len(missing)); }\n");
        let analysis = analyze("main.koto", root, &mut resolver);
        let diagnostic = &analysis.diagnostics[0];
        assert!(diagnostic.message.contains("undefined name `missing`"));
        let span = diagnostic.span.as_ref().unwrap();
        assert_eq!(span.file, "helper.koto");
        assert_eq!(span.start.line, 1);

        let root = "include \"packet.koto\";\nfn main() { oversized(); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert(
            "packet.koto",
            "fn oversized() { buf packet[ui_update_capacity(1, 1985)]; exit(packet); }\n",
        );
        let analysis = analyze("main.koto", root, &mut resolver);
        let diagnostic = &analysis.diagnostics[0];
        assert!(diagnostic
            .message
            .contains("exceed the KotoUI v1 packet capacities"));
        assert_eq!(diagnostic.span.as_ref().unwrap().file, "packet.koto");
    }

    #[test]
    fn enum_definition_hover_and_qualified_completion_work_across_overlay_include() {
        let root = "include \"domain.koto\";\nfn main() { exit(Screen::Play); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert("domain.koto", "enum Screen { Title, Play, Over, }\n");
        let analysis = analyze("main.koto", root, &mut resolver);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );

        let play = root.lines().nth(1).unwrap().find("Play").unwrap();
        let items = completion_items(&analysis, root, 1, play + 2);
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["Play"]
        );
        let definition = definition_at(&analysis, root, 1, play + 1).expect("member definition");
        assert_eq!(definition.span.file, "domain.koto");
        assert_eq!(definition.span.start.column, 22);
        let hover = hover_at(&analysis, root, 1, play + 1).expect("member hover");
        assert!(hover.markdown.contains("Screen::Play = 1"));

        let sdk_items = completion_items(&analysis, "fn main() { exit(FileMode::R); }", 0, 28);
        assert!(sdk_items.iter().any(|item| item.label == "Read"));
        assert!(sdk_items.iter().any(|item| item.label == "ReadWrite"));
    }

    #[test]
    fn static_record_definition_hover_and_receiver_completion_work() {
        let source = r#"struct State { count: int, ready: bool, }
static state: State = { count: 1, ready: true, };
impl State { fn reset(self) { self.count = 0; } }
fn read(value: State) -> int { return value.count; }
fn main() { state.reset(); exit(read(state)); }
"#;
        let analysis = analyze("records.koto", source, &mut OverlayLoader::new(Missing));
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let completion_source = "fn main() { state.re }";
        let completion_col = completion_source.find("state.re").unwrap() + "state.re".len();
        let items = completion_items(&analysis, completion_source, 0, completion_col);
        assert!(items.iter().any(|item| item.label == "ready"));
        assert!(items.iter().any(|item| item.label == "reset"));

        let count_col = source.lines().nth(3).unwrap().find("count").unwrap() + 1;
        let definition = definition_at(&analysis, source, 3, count_col).expect("field definition");
        assert_eq!(definition.span.start.line, 1);
        let hover = hover_at(&analysis, source, 1, 8).expect("static hover");
        assert!(hover.markdown.contains("One App-lifetime record"));
    }

    #[test]
    fn buffer_field_hover_completion_and_definition_show_capacity_and_offset() {
        let source = r#"struct Storage { mode: int, raw: buf[384], }
static storage: Storage = { mode: 1, };
fn main() { exit(heap_get_u8(storage.raw) + len(storage.raw)); }
"#;
        let analysis = analyze("buffers.koto", source, &mut OverlayLoader::new(Missing));
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let raw_col = source.lines().next().unwrap().find("raw").unwrap() + 1;
        let hover = hover_at(&analysis, source, 0, raw_col).expect("buffer field hover");
        assert!(
            hover.markdown.contains("raw: buf[384]"),
            "{}",
            hover.markdown
        );
        assert!(
            hover
                .markdown
                .contains("384-byte buffer field at byte offset 4"),
            "{}",
            hover.markdown
        );
        assert!(
            !hover.markdown.contains("32-bit field"),
            "{}",
            hover.markdown
        );

        let completion_source = "fn main() { storage.ra }";
        let completion_col = completion_source.find("storage.ra").unwrap() + "storage.ra".len();
        let items = completion_items(&analysis, completion_source, 0, completion_col);
        let raw_item = items
            .iter()
            .find(|item| item.label == "raw")
            .expect("raw completion");
        assert_eq!(raw_item.detail, "raw: buf[384] (+4)");

        let use_line = 2;
        let use_col = source.lines().nth(use_line).unwrap().find("raw").unwrap() + 1;
        let definition =
            definition_at(&analysis, source, use_line, use_col).expect("field definition");
        assert_eq!(definition.span.start.line, 1);

        // The static's byte size counts buffer regions into the layout.
        let storage_col = source.lines().nth(1).unwrap().find("storage").unwrap() + 1;
        let static_hover = hover_at(&analysis, source, 1, storage_col).expect("static hover");
        assert!(
            static_hover.markdown.contains("388 heap bytes"),
            "{}",
            static_hover.markdown
        );
    }

    #[test]
    fn unsaved_include_exposes_record_members_and_definition_locations() {
        let root = "include \"state.koto\";\nstatic state: State = { value: 1, };\nfn main() { state.reset(); }\n";
        let mut resolver = OverlayLoader::new(Missing);
        resolver.insert(
            "state.koto",
            "struct State { value: int, }\nimpl State { fn reset(self) { self.value = 0; } }\n",
        );
        let analysis = analyze("main.koto", root, &mut resolver);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let reset = root.lines().nth(2).unwrap().find("reset").unwrap() + 1;
        let definition = definition_at(&analysis, root, 2, reset).expect("method definition");
        assert_eq!(definition.span.file, "state.koto");
        assert_eq!(definition.span.start.line, 2);
    }
}
