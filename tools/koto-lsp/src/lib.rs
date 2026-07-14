//! Protocol-independent language intelligence for KOTO-0194.

use koto_compiler::{
    compile_source, Compilation, CompileRequest, Diagnostic, IncludeResolver, SlotMap, SourceSpan,
    Symbol, SymbolDetail, SymbolKind,
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

pub fn definition_at(
    analysis: &Analysis,
    source: &str,
    line: usize,
    character: usize,
) -> Option<Definition> {
    let word = word_at(source, line, character)?;
    let symbol = symbol_for_word(&analysis.symbols, word)?;
    Some(Definition {
        span: symbol.definition.clone(),
    })
}

pub fn hover_at(analysis: &Analysis, source: &str, line: usize, character: usize) -> Option<Hover> {
    let word = word_at(source, line, character)?;
    let symbol = symbol_for_word(&analysis.symbols, word)?;
    let mut markdown = match &symbol.detail {
        SymbolDetail::Constant { value } => {
            format!("```koto\nconst {} = {value}\n```", symbol.name)
        }
        SymbolDetail::Data {
            element_bits,
            elements,
        } => format!(
            "```koto\ndata {} = u{element_bits}[...];\n```\n\n{elements} elements",
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
}
