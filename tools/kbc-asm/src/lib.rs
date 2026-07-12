//! `kbc-asm`: a small text assembler for the `KBC1` bytecode format.
//!
//! This is the reproducible low-level (IR) target for KotoOS bytecode. It is not
//! the preferred authoring format for real apps — durable apps are written in the
//! high-level Koto app language and compiled down — but it gives generated and
//! fixture bytecode a readable, diffable source and a deterministic encoder.
//!
//! ## Syntax
//!
//! - One statement per line. `;` or `#` starts a comment (ignored inside strings).
//! - Labels are written on their own line as `name:`.
//! - Header directives configure the `KbcHeader`:
//!   `.stack N`, `.calls N`, `.heap N`, `.abi MAJOR MINOR`, `.entry LABEL`.
//! - Instruction mnemonics match [`koto_core::runtime::opcode`]:
//!   - `push_i16 VALUE` (decimal, `0x` hex, or `'c'` char literal).
//!   - `load_local N` / `store_local N` (local slot index).
//!   - `br LABEL` / `br_if_zero LABEL` / `call LABEL`.
//!   - `host_call NAME` (e.g. `draw_text`, `file_open`, `ime_feed_key`).
//!   - all other mnemonics take no operand (`nop`, `halt`, `ret`, `dup`, `drop`,
//!     `swap`, arithmetic, `load8`..`store32`).
//! - `store_str OFFSET, "text"` is a convenience pseudo-instruction that expands
//!   to the `push`/`push`/`store8` sequence writing each byte of `text` into the
//!   app heap starting at `OFFSET` (the VM heap is zero-initialized and not part
//!   of the asset, so constant data is materialized by code).

use koto_core::runtime::{host_call, opcode};
use koto_core::{
    HOST_ABI_MAJOR, HOST_ABI_MINOR, KBC_DEBUG_ENTRY_SIZE, KBC_DEBUG_HEADER_SIZE, KBC_DEBUG_MAGIC,
    KBC_DEBUG_VERSION, KBC_HEADER_SIZE, KBC_MAGIC, KBC_VERSION_MAJOR, KBC_VERSION_MINOR,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError {
    pub line: usize,
    pub message: String,
}

impl AsmError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for AsmError {}

/// Header fields configurable from source, with verifier-friendly defaults.
struct Header {
    stack: u16,
    calls: u16,
    heap: u32,
    abi_major: u16,
    abi_minor: u16,
    entry: Entry,
    /// The const heap image (KOTO-0139): bytes that become `heap[0..rodata.len()]`
    /// at load. Emitted after the code segment as the KBC `rodata` segment.
    rodata: Vec<u8>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            stack: 16,
            calls: 4,
            heap: 0,
            abi_major: HOST_ABI_MAJOR,
            abi_minor: HOST_ABI_MINOR,
            entry: Entry::Word(0),
            rodata: Vec::new(),
        }
    }
}

enum Entry {
    Word(u32),
    Label(String),
}

/// A code word: either fully encoded, or a control-flow instruction whose
/// immediate is a label resolved once all instruction indexes are known.
enum Word {
    Fixed {
        bytes: [u8; 4],
        loc: Option<DebugLoc>,
    },
    Branch {
        opcode: u8,
        label: String,
        line: usize,
        loc: Option<DebugLoc>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DebugLoc {
    file_index: u16,
    line: u32,
    col: u16,
}

/// Assemble `source` into a verifier-shaped `KBC1` asset.
pub fn assemble(source: &str) -> Result<Vec<u8>, AsmError> {
    let mut header = Header::default();
    let mut words: Vec<Word> = Vec::new();
    let mut labels: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut debug_files: Vec<String> = Vec::new();
    let mut current_file = None;
    let mut current_loc = None;

    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let stripped = strip_comment(raw).trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(name) = stripped.strip_suffix(':') {
            let name = name.trim();
            if !is_identifier(name) {
                return Err(AsmError::new(line, format!("invalid label `{name}`")));
            }
            let word_index =
                u32::try_from(words.len()).map_err(|_| AsmError::new(line, "program too large"))?;
            if labels.insert(name.to_string(), word_index).is_some() {
                return Err(AsmError::new(line, format!("duplicate label `{name}`")));
            }
            continue;
        }

        let (mnemonic, args) = split_mnemonic(stripped);
        if let Some(directive) = mnemonic.strip_prefix('.') {
            apply_directive(
                &mut header,
                directive,
                args,
                line,
                &mut debug_files,
                &mut current_file,
                &mut current_loc,
            )?;
            continue;
        }

        assemble_instruction(mnemonic, args, line, current_loc, &mut words)?;
    }

    if words.is_empty() {
        return Err(AsmError::new(0, "program has no instructions"));
    }

    let code_words =
        u32::try_from(words.len()).map_err(|_| AsmError::new(0, "program too large"))?;
    let entry_word = match &header.entry {
        Entry::Word(word) => *word,
        Entry::Label(label) => *labels
            .get(label)
            .ok_or_else(|| AsmError::new(0, format!("entry label `{label}` is undefined")))?,
    };
    if entry_word >= code_words {
        return Err(AsmError::new(0, "entry word is outside the code"));
    }

    let code_size = code_words * 4;
    let debug = build_debug_section(&debug_files, &words, line_count(source))?;
    // Segment layout: header, code, rodata (the const heap image), then debug
    // (KOTO-0139). rodata and debug are each present only when non-empty; their
    // offsets are 0 when absent, matching the verifier's `optional_range` contract.
    let code_end = KBC_HEADER_SIZE + code_words as usize * 4;
    let rodata_offset = if header.rodata.is_empty() {
        0
    } else {
        code_end
    };
    let debug_offset = if debug.is_empty() {
        0
    } else {
        code_end + header.rodata.len()
    };
    let bytecode_size = code_end + header.rodata.len() + debug.len();
    let mut bytes = vec![0u8; bytecode_size];
    bytes[0..4].copy_from_slice(&KBC_MAGIC);
    bytes[4..6].copy_from_slice(&KBC_VERSION_MAJOR.to_le_bytes());
    bytes[6..8].copy_from_slice(&KBC_VERSION_MINOR.to_le_bytes());
    bytes[8..12].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&(bytecode_size as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(KBC_HEADER_SIZE as u32).to_le_bytes());
    bytes[24..28].copy_from_slice(&code_size.to_le_bytes());
    if !header.rodata.is_empty() {
        bytes[28..32].copy_from_slice(&(rodata_offset as u32).to_le_bytes());
        bytes[32..36].copy_from_slice(&(header.rodata.len() as u32).to_le_bytes());
    }
    bytes[36..40].copy_from_slice(&entry_word.to_le_bytes());
    bytes[40..42].copy_from_slice(&header.stack.to_le_bytes());
    bytes[42..44].copy_from_slice(&header.calls.to_le_bytes());
    bytes[44..48].copy_from_slice(&header.heap.to_le_bytes());
    bytes[48..50].copy_from_slice(&header.abi_major.to_le_bytes());
    bytes[50..52].copy_from_slice(&header.abi_minor.to_le_bytes());
    if !debug.is_empty() {
        bytes[52..56].copy_from_slice(&(debug_offset as u32).to_le_bytes());
        bytes[56..60].copy_from_slice(&(debug.len() as u32).to_le_bytes());
    }

    for (index, word) in words.iter().enumerate() {
        let encoded = match word {
            Word::Fixed { bytes, .. } => *bytes,
            Word::Branch {
                opcode,
                label,
                line,
                ..
            } => {
                let target = *labels
                    .get(label)
                    .ok_or_else(|| AsmError::new(*line, format!("undefined label `{label}`")))?;
                let target = u16::try_from(target)
                    .map_err(|_| AsmError::new(*line, "branch target out of range"))?;
                let imm = target.to_le_bytes();
                [imm[0], imm[1], 0, *opcode]
            }
        };
        let offset = KBC_HEADER_SIZE + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&encoded);
    }
    if !header.rodata.is_empty() {
        bytes[rodata_offset..rodata_offset + header.rodata.len()].copy_from_slice(&header.rodata);
    }
    if !debug.is_empty() {
        bytes[debug_offset..debug_offset + debug.len()].copy_from_slice(&debug);
    }

    Ok(bytes)
}

fn apply_directive(
    header: &mut Header,
    directive: &str,
    args: &str,
    line: usize,
    debug_files: &mut Vec<String>,
    current_file: &mut Option<u16>,
    current_loc: &mut Option<DebugLoc>,
) -> Result<(), AsmError> {
    match directive {
        "stack" => header.stack = parse_u16(args, line)?,
        "calls" => header.calls = parse_u16(args, line)?,
        "heap" => header.heap = parse_u32(args, line)?,
        "rodata" => header
            .rodata
            .extend_from_slice(&parse_hex_bytes(args.trim(), line)?),
        "abi" => {
            let mut parts = args.split_whitespace();
            let major = parts
                .next()
                .ok_or_else(|| AsmError::new(line, ".abi needs MAJOR"))?;
            let minor = parts
                .next()
                .ok_or_else(|| AsmError::new(line, ".abi needs MINOR"))?;
            header.abi_major = parse_u16(major, line)?;
            header.abi_minor = parse_u16(minor, line)?;
        }
        "entry" => {
            let target = args.trim();
            header.entry = if let Ok(word) = parse_u32(target, line) {
                Entry::Word(word)
            } else if is_identifier(target) {
                Entry::Label(target.to_string())
            } else {
                return Err(AsmError::new(line, ".entry needs a word index or label"));
            };
        }
        "debug_file" => {
            let bytes = parse_string(args.trim(), line)?;
            let file = String::from_utf8(bytes)
                .map_err(|_| AsmError::new(line, ".debug_file must be valid UTF-8"))?;
            if file.is_empty() {
                return Err(AsmError::new(line, ".debug_file must not be empty"));
            }
            let index = u16::try_from(debug_files.len())
                .map_err(|_| AsmError::new(line, "too many debug files"))?;
            debug_files.push(file);
            *current_file = Some(index);
            *current_loc = None;
        }
        "loc" => {
            let file_index = current_file
                .ok_or_else(|| AsmError::new(line, ".loc needs a preceding .debug_file"))?;
            let mut parts = args.split_whitespace();
            let source_line = parts
                .next()
                .ok_or_else(|| AsmError::new(line, ".loc needs LINE COL"))?;
            let source_col = parts
                .next()
                .ok_or_else(|| AsmError::new(line, ".loc needs LINE COL"))?;
            if parts.next().is_some() {
                return Err(AsmError::new(line, ".loc takes only LINE COL"));
            }
            *current_loc = Some(DebugLoc {
                file_index,
                line: parse_u32(source_line, line)?,
                col: parse_u16(source_col, line)?,
            });
        }
        other => return Err(AsmError::new(line, format!("unknown directive `.{other}`"))),
    }
    Ok(())
}

fn assemble_instruction(
    mnemonic: &str,
    args: &str,
    line: usize,
    loc: Option<DebugLoc>,
    words: &mut Vec<Word>,
) -> Result<(), AsmError> {
    match mnemonic {
        "push_i16" => {
            let value = parse_imm16(args, line)?;
            words.push(fixed(opcode::PUSH_I16, 0, value, loc));
        }
        "load_local" => words.push(fixed(opcode::LOAD_LOCAL, parse_u8(args, line)?, 0, loc)),
        "store_local" => words.push(fixed(opcode::STORE_LOCAL, parse_u8(args, line)?, 0, loc)),
        "br" => words.push(branch(opcode::BR, args, line, loc)?),
        "br_if_zero" => words.push(branch(opcode::BR_IF_ZERO, args, line, loc)?),
        "call" => words.push(branch(opcode::CALL, args, line, loc)?),
        "host_call" => {
            let id = host_call_id(args.trim()).ok_or_else(|| {
                AsmError::new(line, format!("unknown host call `{}`", args.trim()))
            })?;
            words.push(fixed(opcode::HOST_CALL, id, 0, loc));
        }
        "store_str" => assemble_store_str(args, line, loc, words)?,
        other => {
            let opcode = bare_opcode(other)
                .ok_or_else(|| AsmError::new(line, format!("unknown mnemonic `{other}`")))?;
            if !args.trim().is_empty() {
                return Err(AsmError::new(line, format!("`{other}` takes no operand")));
            }
            words.push(fixed(opcode, 0, 0, loc));
        }
    }
    Ok(())
}

fn assemble_store_str(
    args: &str,
    line: usize,
    loc: Option<DebugLoc>,
    words: &mut Vec<Word>,
) -> Result<(), AsmError> {
    let (offset, rest) = args
        .split_once(',')
        .ok_or_else(|| AsmError::new(line, "store_str needs OFFSET, \"text\""))?;
    let offset = parse_u16(offset.trim(), line)?;
    let text = parse_string(rest.trim(), line)?;
    for (index, byte) in text.iter().enumerate() {
        let at = offset
            .checked_add(u16::try_from(index).map_err(|_| AsmError::new(line, "string too long"))?)
            .ok_or_else(|| AsmError::new(line, "string overflows heap offset"))?;
        words.push(fixed(opcode::PUSH_I16, 0, at, loc));
        words.push(fixed(opcode::PUSH_I16, 0, u16::from(*byte), loc));
        words.push(fixed(opcode::STORE8, 0, 0, loc));
    }
    Ok(())
}

fn branch(opcode: u8, args: &str, line: usize, loc: Option<DebugLoc>) -> Result<Word, AsmError> {
    let label = args.trim();
    if !is_identifier(label) {
        return Err(AsmError::new(line, "branch needs a label"));
    }
    Ok(Word::Branch {
        opcode,
        label: label.to_string(),
        line,
        loc,
    })
}

fn fixed(opcode: u8, operand: u8, immediate: u16, loc: Option<DebugLoc>) -> Word {
    Word::Fixed {
        bytes: encode(opcode, operand, immediate),
        loc,
    }
}

fn encode(opcode: u8, operand: u8, immediate: u16) -> [u8; 4] {
    let imm = immediate.to_le_bytes();
    [imm[0], imm[1], operand, opcode]
}

fn build_debug_section(files: &[String], words: &[Word], line: usize) -> Result<Vec<u8>, AsmError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let mut previous = None;
    for (pc, word) in words.iter().enumerate() {
        let loc = match word {
            Word::Fixed { loc, .. } | Word::Branch { loc, .. } => *loc,
        };
        if let Some(loc) = loc {
            if Some(loc) != previous {
                entries.push((pc as u32, loc));
            }
        }
        previous = loc;
    }
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if entries.len() > u16::MAX as usize {
        return Err(AsmError::new(line, "too many debug entries"));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&KBC_DEBUG_MAGIC);
    out.extend_from_slice(&KBC_DEBUG_VERSION.to_le_bytes());
    out.extend_from_slice(&(KBC_DEBUG_HEADER_SIZE as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for file in files {
        let len = u16::try_from(file.len())
            .map_err(|_| AsmError::new(line, "debug file path too long"))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(file.as_bytes());
    }
    for (pc, loc) in entries {
        out.extend_from_slice(&pc.to_le_bytes());
        out.extend_from_slice(&loc.file_index.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&loc.line.to_le_bytes());
        out.extend_from_slice(&loc.col.to_le_bytes());
        debug_assert_eq!(KBC_DEBUG_ENTRY_SIZE, 14);
    }
    Ok(out)
}

fn line_count(source: &str) -> usize {
    source.lines().count().max(1)
}

fn bare_opcode(mnemonic: &str) -> Option<u8> {
    Some(match mnemonic {
        "nop" => opcode::NOP,
        "halt" => opcode::HALT,
        "ret" => opcode::RET,
        "dup" => opcode::DUP,
        "drop" => opcode::DROP,
        "swap" => opcode::SWAP,
        "add_i32" => opcode::ADD_I32,
        "sub_i32" => opcode::SUB_I32,
        "mul_i32" => opcode::MUL_I32,
        "div_i32" => opcode::DIV_I32,
        "and_i32" => opcode::AND_I32,
        "or_i32" => opcode::OR_I32,
        "xor_i32" => opcode::XOR_I32,
        "shl_i32" => opcode::SHL_I32,
        "shr_i32" => opcode::SHR_I32,
        "load8" => opcode::LOAD8,
        "store8" => opcode::STORE8,
        "load16" => opcode::LOAD16,
        "store16" => opcode::STORE16,
        "load32" => opcode::LOAD32,
        "store32" => opcode::STORE32,
        _ => return None,
    })
}

fn host_call_id(name: &str) -> Option<u8> {
    Some(match name {
        "exit" => host_call::EXIT,
        "yield_frame" => host_call::YIELD_FRAME,
        "draw_rect" => host_call::DRAW_RECT,
        "draw_text" => host_call::DRAW_TEXT,
        "draw_text_color" => host_call::DRAW_TEXT_COLOR,
        "draw_pixels_rgb565" => host_call::DRAW_PIXELS_RGB565,
        "game2d_set_tile" => host_call::GAME2D_SET_TILE,
        "game2d_clear_layer" => host_call::GAME2D_CLEAR_LAYER,
        "game2d_present" => host_call::GAME2D_PRESENT,
        "game2d_static_begin" => host_call::GAME2D_STATIC_BEGIN,
        "game2d_static_end" => host_call::GAME2D_STATIC_END,
        "game2d_stamp_define" => host_call::GAME2D_STAMP_DEFINE,
        "game2d_sprite_set" => host_call::GAME2D_SPRITE_SET,
        "game2d_sprite_hide" => host_call::GAME2D_SPRITE_HIDE,
        "game2d_sprite_clear_all" => host_call::GAME2D_SPRITE_CLEAR_ALL,
        "game2d_text_set" => host_call::GAME2D_TEXT_SET,
        "game2d_text_hide" => host_call::GAME2D_TEXT_HIDE,
        "game2d_text_clear_all" => host_call::GAME2D_TEXT_CLEAR_ALL,
        "input_snapshot" => host_call::INPUT_SNAPSHOT,
        "text_input" => host_call::TEXT_INPUT,
        "audio_submit_i16" => host_call::AUDIO_SUBMIT_I16,
        "play_sfx" => host_call::PLAY_SFX,
        "play_bgm" => host_call::PLAY_BGM,
        "play_bgm_asset" => host_call::PLAY_BGM_ASSET,
        "play_sfx_asset" => host_call::PLAY_SFX_ASSET,
        "stop_bgm" => host_call::STOP_BGM,
        "file_open" => host_call::FILE_OPEN,
        "file_read" => host_call::FILE_READ,
        "file_write" => host_call::FILE_WRITE,
        "file_close" => host_call::FILE_CLOSE,
        "asset_load" => host_call::ASSET_LOAD,
        "ime_feed_key" => host_call::IME_FEED_KEY,
        "ime_convert" => host_call::IME_CONVERT,
        "ime_query_line" => host_call::IME_QUERY_LINE,
        "edit_move" => host_call::EDIT_MOVE,
        "edit_delete" => host_call::EDIT_DELETE,
        "edit_load" => host_call::EDIT_LOAD,
        "edit_query_text" => host_call::EDIT_QUERY_TEXT,
        "ime_display" => host_call::IME_DISPLAY,
        "edit_visible_line" => host_call::EDIT_VISIBLE_LINE,
        "edit_cursor_view" => host_call::EDIT_CURSOR_VIEW,
        "edit_scroll_row" => host_call::EDIT_SCROLL_ROW,
        "edit_view_metrics" => host_call::EDIT_VIEW_METRICS,
        "edit_cursor_status" => host_call::EDIT_CURSOR_STATUS,
        "edit_total_lines" => host_call::EDIT_TOTAL_LINES,
        "edit_wrap" => host_call::EDIT_WRAP,
        "edit_hscroll_view" => host_call::EDIT_HSCROLL_VIEW,
        "dir_list" => host_call::DIR_LIST,
        "edit_reserve_rows" => host_call::EDIT_RESERVE_ROWS,
        "edit_configure" => host_call::EDIT_CONFIGURE,
        _ => return None,
    })
}

fn split_mnemonic(line: &str) -> (&str, &str) {
    match line.split_once(char::is_whitespace) {
        Some((mnemonic, rest)) => (mnemonic, rest.trim()),
        None => (line, ""),
    }
}

/// Remove a trailing `;`/`#` comment, ignoring comment chars inside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            ';' | '#' => return &line[..index],
            _ => {}
        }
    }
    line
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
        && !text.chars().next().unwrap().is_ascii_digit()
}

fn parse_imm16(text: &str, line: usize) -> Result<u16, AsmError> {
    let value = parse_int(text, line)?;
    if (-32768..=65535).contains(&value) {
        Ok(value as u16)
    } else {
        Err(AsmError::new(
            line,
            format!("`{text}` is out of 16-bit range"),
        ))
    }
}

fn parse_u8(text: &str, line: usize) -> Result<u8, AsmError> {
    let value = parse_int(text, line)?;
    u8::try_from(value).map_err(|_| AsmError::new(line, format!("`{text}` is out of 8-bit range")))
}

fn parse_u16(text: &str, line: usize) -> Result<u16, AsmError> {
    let value = parse_int(text, line)?;
    u16::try_from(value).map_err(|_| AsmError::new(line, format!("`{text}` is out of u16 range")))
}

fn parse_u32(text: &str, line: usize) -> Result<u32, AsmError> {
    let value = parse_int(text, line)?;
    u32::try_from(value).map_err(|_| AsmError::new(line, format!("`{text}` is out of u32 range")))
}

/// Parse a `.rodata` operand: a contiguous string of hex digit pairs, each a byte
/// (KOTO-0139). The compiler emits the const heap image this way.
fn parse_hex_bytes(text: &str, line: usize) -> Result<Vec<u8>, AsmError> {
    if text.is_empty() {
        return Err(AsmError::new(line, ".rodata needs hex bytes"));
    }
    if !text.len().is_multiple_of(2) {
        return Err(AsmError::new(
            line,
            ".rodata needs an even number of hex digits",
        ));
    }
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = pair[0].to_digit(16);
        let lo = pair[1].to_digit(16);
        match (hi, lo) {
            (Some(hi), Some(lo)) => out.push((hi * 16 + lo) as u8),
            _ => {
                return Err(AsmError::new(
                    line,
                    format!("`{}{}` is not a hex byte", pair[0], pair[1]),
                ))
            }
        }
    }
    Ok(out)
}

fn parse_int(text: &str, line: usize) -> Result<i64, AsmError> {
    let text = text.trim();
    if text.starts_with('\'') {
        return parse_char_literal(text, line).map(|ch| ch as i64);
    }
    let parsed = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
    } else {
        text.parse::<i64>()
    };
    parsed.map_err(|_| AsmError::new(line, format!("expected a number, got `{text}`")))
}

fn parse_char_literal(text: &str, line: usize) -> Result<char, AsmError> {
    let inner = text
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .ok_or_else(|| AsmError::new(line, "unterminated char literal"))?;
    let mut chars = inner.chars();
    let first = chars
        .next()
        .ok_or_else(|| AsmError::new(line, "empty char literal"))?;
    let ch = if first == '\\' {
        let escape = chars
            .next()
            .ok_or_else(|| AsmError::new(line, "dangling escape in char literal"))?;
        unescape(escape).ok_or_else(|| AsmError::new(line, "unknown char escape"))?
    } else {
        first
    };
    if chars.next().is_some() {
        return Err(AsmError::new(
            line,
            "char literal holds more than one character",
        ));
    }
    Ok(ch)
}

fn parse_string(text: &str, line: usize) -> Result<Vec<u8>, AsmError> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| AsmError::new(line, "string must be quoted"))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escape = chars
                .next()
                .ok_or_else(|| AsmError::new(line, "dangling escape in string"))?;
            out.push(unescape(escape).ok_or_else(|| AsmError::new(line, "unknown string escape"))?);
        } else {
            out.push(ch);
        }
    }
    Ok(out.into_bytes())
}

fn unescape(escape: char) -> Option<char> {
    Some(match escape {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '0' => '\0',
        '\\' => '\\',
        '"' => '"',
        '\'' => '\'',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_core::{verify_kbc, RuntimeLimits};

    fn verified(bytes: &[u8]) {
        verify_kbc(bytes, RuntimeLimits::simulator_default()).expect("assembled bytecode verifies");
    }

    #[test]
    fn assembles_and_verifies_minimal_program() {
        let bytes = assemble(
            "
            .stack 8
            .calls 4
            .heap 0
            push_i16 7
            host_call exit
            halt
            ",
        )
        .unwrap();
        verified(&bytes);
        // entry word 0, code = 3 words.
        assert_eq!(&bytes[0..4], b"KBC1");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 12);
    }

    #[test]
    fn resolves_labels_for_branches() {
        let bytes = assemble(
            "
            .stack 8
            .calls 4
            top:
            push_i16 0
            br_if_zero done
            br top
            done:
            push_i16 0
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);
        // `br top` targets word 0; `br_if_zero done` targets word 3 (push,br_if,br,push...).
        // Word layout: 0 push, 1 br_if_zero, 2 br, 3 push, 4 host_call.
        let br_if = &bytes[KBC_HEADER_SIZE + 4..KBC_HEADER_SIZE + 8];
        assert_eq!(u16::from_le_bytes([br_if[0], br_if[1]]), 3);
        let br = &bytes[KBC_HEADER_SIZE + 8..KBC_HEADER_SIZE + 12];
        assert_eq!(u16::from_le_bytes([br[0], br[1]]), 0);
    }

    #[test]
    fn store_str_expands_to_byte_stores() {
        let bytes = assemble(
            "
            .stack 8
            .heap 64
            store_str 0, \"hi\\n\"
            push_i16 0
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);
        // 3 bytes * 3 words + push + host_call = 11 words.
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            11 * 4
        );
    }

    #[test]
    fn rodata_directive_emits_const_heap_image() {
        let bytes = assemble(
            "
            .stack 8
            .heap 64
            .rodata f00044440f00
            push_i16 0
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);
        let code_size = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
        let rodata_offset = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
        let rodata_size = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
        // rodata sits immediately after the code segment, sized to the hex bytes.
        assert_eq!(rodata_offset, KBC_HEADER_SIZE + code_size);
        assert_eq!(rodata_size, 6);
        assert_eq!(
            &bytes[rodata_offset..rodata_offset + rodata_size],
            &[0xf0, 0x00, 0x44, 0x44, 0x0f, 0x00]
        );
    }

    #[test]
    fn rodata_appends_across_directives() {
        let bytes = assemble(
            "
            .heap 64
            .rodata 0102
            .rodata 0304
            push_i16 0
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);
        let rodata_offset = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
        let rodata_size = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
        assert_eq!(rodata_size, 4);
        assert_eq!(
            &bytes[rodata_offset..rodata_offset + rodata_size],
            &[1, 2, 3, 4]
        );
    }

    #[test]
    fn rejects_odd_length_rodata() {
        let error = assemble(".rodata abc\npush_i16 0\nhost_call exit").unwrap_err();
        assert!(error.message.contains("even"));
    }

    #[test]
    fn rejects_undefined_label() {
        let error = assemble(
            "
            br nowhere
            ",
        )
        .unwrap_err();
        assert!(error.message.contains("nowhere"));
    }

    #[test]
    fn rejects_unknown_mnemonic() {
        let error = assemble("frobnicate 1").unwrap_err();
        assert!(error.message.contains("frobnicate"));
    }

    #[test]
    fn memo_assembly_fixture_assembles_and_verifies() {
        // `apps/memo/memo.kbc.asm` is the assembler's reference fixture. The
        // shipped `sdcard_mock/bytecode/memo.kbc` is now compiled from the
        // high-level `apps/memo/src/main.koto` (see the build loop), so this only
        // checks the assembly fixture still assembles to verifier-valid bytecode.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let source = std::fs::read_to_string(root.join("apps/memo/memo.kbc.asm")).unwrap();
        let assembled = assemble(&source).unwrap();
        verified(&assembled);
    }

    #[test]
    fn char_literal_and_hex_immediates() {
        let bytes = assemble(
            "
            .stack 8
            push_i16 'k'
            push_i16 0x10
            drop
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);
        let first = &bytes[KBC_HEADER_SIZE..KBC_HEADER_SIZE + 4];
        assert_eq!(u16::from_le_bytes([first[0], first[1]]), u16::from(b'k'));
    }

    #[test]
    fn emits_debug_section_from_file_and_loc_directives() {
        let bytes = assemble(
            "
            .debug_file \"test.koto\"
            .loc 3 5
            push_i16 1
            .loc 4 7
            push_i16 0
            div_i32
            host_call exit
            ",
        )
        .unwrap();
        verified(&bytes);

        let map = koto_core::debug_map(&bytes).unwrap().unwrap();

        assert_eq!(map.lookup_pc(0).unwrap().file, "test.koto");
        assert_eq!(map.lookup_pc(0).unwrap().line, 3);
        assert_eq!(map.lookup_pc(2).unwrap().line, 4);
        assert_eq!(map.lookup_pc(2).unwrap().col, 7);
    }

    #[test]
    fn assembles_without_debug_when_no_loc_is_present() {
        let bytes = assemble(
            "
            push_i16 0
            host_call exit
            ",
        )
        .unwrap();

        assert!(koto_core::debug_map(&bytes).unwrap().is_none());
    }
}
