//! Recursive-descent parser with precedence climbing for the Koto app language.
//! Produces the AST consumed by codegen. Errors carry source line/column.

use crate::assets::AssetResolver;
use crate::lexer::{Tok, Token};
use crate::Diag;
use koto_core::ui_session::{
    UI_DATA_CAPACITY, UI_MAX_LIST_ROWS, UI_MAX_MOUNT_BYTES, UI_MAX_NODES, UI_MAX_UPDATE_BYTES,
    UI_MAX_UPDATE_RECORDS, UI_MOUNT_HEADER_SIZE, UI_NODE_RECORD_SIZE, UI_UPDATE_HEADER_SIZE,
    UI_UPDATE_RECORD_SIZE,
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Struct(String),
}

#[derive(Clone, Debug)]
pub struct Program {
    pub consts: Vec<ConstDef>,
    pub enums: Vec<EnumDef>,
    pub data: Vec<DataDef>,
    pub structs: Vec<StructDef>,
    pub statics: Vec<StaticDef>,
    pub methods: Vec<MethodDef>,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<StructField>,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub struct StructField {
    pub name: String,
    pub kind: StructFieldKind,
    pub name_line: usize,
    pub name_col: usize,
}

/// A struct field is either a 32-bit scalar (KOTO-0228) or a fixed-size byte
/// region (`name: buf[N]`, KOTO-0235). Buffer sizes are folded at parse time
/// through the same size grammar as local `buf` declarations, so capacity
/// helpers and prior consts carry identical diagnostics at both sites.
#[derive(Clone, Debug)]
pub enum StructFieldKind {
    Scalar(Type),
    Buffer(usize),
}

#[derive(Clone, Debug)]
pub struct StaticDef {
    pub name: String,
    pub ty: String,
    pub fields: Vec<StaticFieldInit>,
    pub line: usize,
    pub col: usize,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub struct StaticFieldInit {
    pub name: String,
    pub value: Expr,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct MethodDef {
    pub target: String,
    pub function: Function,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub members: Vec<EnumMember>,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub struct EnumMember {
    pub name: String,
    pub value: i64,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub struct ConstDef {
    pub name: String,
    pub value: i64,
    pub ty: Type,
    pub name_line: usize,
    pub name_col: usize,
}

/// The element width of a `data` literal array, which fixes its little-endian byte
/// layout in the const heap image (KOTO-0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataWidth {
    U8,
    U16,
}

/// A top-level const-initialized buffer (`data NAME = u16[...]`). Its bytes are
/// emitted into the KBC `rodata` segment and copied into the bottom of the app heap
/// once at load, replacing runtime `heap_set_*` table baking (KOTO-0139).
#[derive(Clone, Debug)]
pub struct DataDef {
    pub name: String,
    pub width: DataWidth,
    pub values: Vec<i64>,
    pub line: usize,
    pub col: usize,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Vec<Stmt>,
    pub line: usize,
    pub col: usize,
    pub name_line: usize,
    pub name_col: usize,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: String,
        annotation: Option<Type>,
        value: Expr,
        line: usize,
        col: usize,
    },
    BufDecl {
        name: String,
        size: usize,
        line: usize,
        col: usize,
    },
    Assign {
        name: String,
        value: Expr,
        line: usize,
        col: usize,
    },
    BufStore {
        name: String,
        index: Expr,
        value: Expr,
        line: usize,
        col: usize,
    },
    FieldAssign {
        receiver: Expr,
        field: String,
        value: Expr,
        line: usize,
        col: usize,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then: Vec<Stmt>,
        otherwise: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Loop {
        body: Vec<Stmt>,
    },
    Break {
        line: usize,
        col: usize,
    },
    Continue {
        line: usize,
        col: usize,
    },
    Return {
        value: Option<Expr>,
        line: usize,
        col: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LAnd,
    LOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int {
        value: i64,
        line: usize,
        col: usize,
    },
    Bool {
        value: bool,
        line: usize,
        col: usize,
    },
    Str {
        bytes: Vec<u8>,
        line: usize,
        col: usize,
    },
    Ident {
        name: String,
        line: usize,
        col: usize,
    },
    EnumMember {
        enum_name: String,
        member_name: String,
        line: usize,
        col: usize,
    },
    BufIndex {
        name: String,
        index: Box<Expr>,
        line: usize,
        col: usize,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
        line: usize,
        col: usize,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        line: usize,
        col: usize,
    },
    Call {
        name: String,
        args: Vec<Expr>,
        line: usize,
        col: usize,
    },
    Field {
        receiver: Box<Expr>,
        name: String,
        line: usize,
        col: usize,
    },
    MethodCall {
        receiver: Box<Expr>,
        name: String,
        args: Vec<Expr>,
        line: usize,
        col: usize,
    },
}

impl Expr {
    pub fn position(&self) -> (usize, usize) {
        match self {
            Expr::Int { line, col, .. }
            | Expr::Bool { line, col, .. }
            | Expr::Str { line, col, .. }
            | Expr::Ident { line, col, .. }
            | Expr::EnumMember { line, col, .. }
            | Expr::BufIndex { line, col, .. }
            | Expr::Unary { line, col, .. }
            | Expr::Binary { line, col, .. }
            | Expr::Call { line, col, .. } => (*line, *col),
            Expr::Field { line, col, .. } | Expr::MethodCall { line, col, .. } => (*line, *col),
        }
    }
}

/// The KotoUI capacity helpers the compiler folds at compile time when they
/// appear in a `const` initializer, a `buf` size, or a struct buffer-field
/// size (KOTO-0232/0233/0236): the two packet constructors plus the two SDK
/// storage constructors for `TextResource` and `UiListRowsBuilder` regions.
fn is_capacity_helper(name: &str) -> bool {
    name == "ui_mount_capacity"
        || name == "ui_update_capacity"
        || name == "ui_text_resource_capacity"
        || name == "ui_list_rows_capacity"
}

fn is_text_asset_helper(name: &str) -> bool {
    name == "asset_text_line_count"
        || name == "asset_text_max_range_bytes"
        || name == "asset_text_max_line_bytes"
}

/// SDK-private storage facts the two KOTO-0236 helpers fold so applications
/// never restate them: `TextResource::parse` reserves four table bytes per
/// line, `UiListRowsBuilder::begin` reserves a 12-byte record per row, and
/// both representations index with u16 offsets (`sdk/koto_ui/resources.koto`).
const UI_TEXT_RESOURCE_LINE_STRIDE: i64 = 4;
const UI_TEXT_RESOURCE_MAX_LINES: i64 = 16383;
const UI_LIST_ROW_RECORD_STRIDE: i64 = 12;
const UI_STORAGE_MAX_BYTES: i64 = 65535;

fn text_asset_line_lengths(bytes: &[u8]) -> Result<Vec<usize>, String> {
    std::str::from_utf8(bytes).map_err(|_| "asset is not valid UTF-8".to_string())?;
    let mut lengths = Vec::new();
    let mut line_len = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lengths.push(line_len);
                line_len = 0;
                index += 1;
            }
            b'\r' => {
                if bytes.get(index + 1) != Some(&b'\n') {
                    return Err("asset contains a bare CR line ending".to_string());
                }
                lengths.push(line_len);
                line_len = 0;
                index += 2;
            }
            _ => {
                line_len += 1;
                index += 1;
            }
        }
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        lengths.push(line_len);
    }
    Ok(lengths)
}

pub fn parse(tokens: &[Token], assets: &mut dyn AssetResolver) -> Result<Program, Diag> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        const_values: HashMap::new(),
        assets,
    };
    parser.program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    const_values: HashMap<String, (i64, Type)>,
    /// KOTO-0236: `asset_len` byte sizes, injected alongside include loading.
    assets: &'a mut dyn AssetResolver,
}

impl<'a> Parser<'a> {
    fn program(&mut self) -> Result<Program, Diag> {
        let mut consts = Vec::new();
        let mut enums = Vec::new();
        let mut data = Vec::new();
        let mut structs = Vec::new();
        let mut statics = Vec::new();
        let mut methods = Vec::new();
        let mut functions = Vec::new();
        while !self.at_end() {
            match self.peek_tok() {
                Tok::Const => {
                    let def = self.const_def()?;
                    self.const_values
                        .insert(def.name.clone(), (def.value, def.ty.clone()));
                    consts.push(def);
                }
                Tok::Enum => enums.push(self.enum_def()?),
                Tok::Data => data.push(self.data_def()?),
                Tok::Struct => structs.push(self.struct_def()?),
                Tok::Static => statics.push(self.static_def()?),
                Tok::Impl => methods.extend(self.impl_def()?),
                Tok::Fn => functions.push(self.function()?),
                _ => {
                    let (line, col) = self.position();
                    return Err(Diag::new(
                        line,
                        col,
                        "expected `fn`, `const`, `enum`, `data`, `struct`, `static`, or `impl` at top level".to_string(),
                    ));
                }
            }
        }
        Ok(Program {
            consts,
            enums,
            data,
            structs,
            statics,
            methods,
            functions,
        })
    }

    fn struct_def(&mut self) -> Result<StructDef, Diag> {
        self.expect(Tok::Struct)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(&Tok::RBrace) {
            let (field_line, field_col) = self.position();
            let field = self.ident()?;
            self.expect(Tok::Colon)?;
            let kind = if self.eat(&Tok::Buf) {
                self.expect(Tok::LBracket)?;
                let size = self.buf_size()?;
                self.expect(Tok::RBracket)?;
                StructFieldKind::Buffer(size)
            } else {
                StructFieldKind::Scalar(self.ty()?)
            };
            fields.push(StructField {
                name: field,
                kind,
                name_line: field_line,
                name_col: field_col,
            });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        self.eat(&Tok::Semi);
        if fields.is_empty() {
            return Err(Diag::new(
                name_line,
                name_col,
                format!("struct `{name}` must have at least one field"),
            ));
        }
        Ok(StructDef {
            name,
            fields,
            name_line,
            name_col,
        })
    }

    fn static_def(&mut self) -> Result<StaticDef, Diag> {
        let (line, col) = self.position();
        self.expect(Tok::Static)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::Colon)?;
        let ty = self.ident()?;
        self.expect(Tok::Eq)?;
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(&Tok::RBrace) {
            let (field_line, field_col) = self.position();
            let field = self.ident()?;
            self.expect(Tok::Colon)?;
            let value = self.expr()?;
            fields.push(StaticFieldInit {
                name: field,
                value,
                line: field_line,
                col: field_col,
            });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        self.expect(Tok::Semi)?;
        Ok(StaticDef {
            name,
            ty,
            fields,
            line,
            col,
            name_line,
            name_col,
        })
    }

    fn impl_def(&mut self) -> Result<Vec<MethodDef>, Diag> {
        self.expect(Tok::Impl)?;
        let target = self.ident()?;
        self.expect(Tok::LBrace)?;
        let mut methods = Vec::new();
        while !self.at(&Tok::RBrace) {
            methods.push(MethodDef {
                target: target.clone(),
                function: self.function_for(Some(&target))?,
            });
        }
        self.expect(Tok::RBrace)?;
        Ok(methods)
    }

    fn enum_def(&mut self) -> Result<EnumDef, Diag> {
        let (line, col) = self.position();
        self.expect(Tok::Enum)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::LBrace)?;
        let mut members = Vec::new();
        let mut next = 0i64;
        while !self.at(&Tok::RBrace) {
            let (member_line, member_col) = self.position();
            let member_name = self.ident()?;
            let explicit = self.eat(&Tok::Eq);
            let value = if explicit {
                let (value_line, value_col) = self.position();
                let negative = self.eat(&Tok::Minus);
                match self.next_tok()? {
                    Tok::Int(value) => {
                        let value = if negative {
                            value.checked_neg()
                        } else {
                            Some(value)
                        };
                        value.ok_or_else(|| {
                            Diag::new(
                                value_line,
                                value_col,
                                "enum value is out of range".to_string(),
                            )
                        })?
                    }
                    _ => {
                        return Err(Diag::new(
                            value_line,
                            value_col,
                            "enum value must be a signed integer literal".to_string(),
                        ))
                    }
                }
            } else {
                next
            };
            if i32::try_from(value).is_err() {
                return Err(Diag::new(
                    member_line,
                    member_col,
                    if explicit {
                        "enum value is out of 32-bit range".to_string()
                    } else {
                        "enum implicit value overflows 32-bit int".to_string()
                    },
                ));
            }
            members.push(EnumMember {
                name: member_name,
                value,
                name_line: member_line,
                name_col: member_col,
            });
            next = value.checked_add(1).ok_or_else(|| {
                Diag::new(
                    member_line,
                    member_col,
                    "enum implicit value overflows".to_string(),
                )
            })?;
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        // Match other top-level declarations while permitting the common `};` spelling.
        self.eat(&Tok::Semi);
        if members.is_empty() {
            return Err(Diag::new(
                line,
                col,
                format!("enum `{name}` must have at least one member"),
            ));
        }
        Ok(EnumDef {
            name,
            members,
            name_line,
            name_col,
        })
    }

    fn data_def(&mut self) -> Result<DataDef, Diag> {
        let (line, col) = self.position();
        self.expect(Tok::Data)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::Eq)?;
        // Element width keyword (`u8`/`u16`) read as an identifier, then a bracketed,
        // comma-separated list of integer literals.
        let (wline, wcol) = self.position();
        let width = match self.ident()?.as_str() {
            "u8" => DataWidth::U8,
            "u16" => DataWidth::U16,
            other => {
                return Err(Diag::new(
                    wline,
                    wcol,
                    format!("`data` element type must be `u8` or `u16`, got `{other}`"),
                ))
            }
        };
        self.expect(Tok::LBracket)?;
        let mut values = Vec::new();
        while !self.at(&Tok::RBracket) {
            let (vline, vcol) = self.position();
            let value = match self.next_tok()? {
                Tok::Int(value) => value,
                Tok::Minus => match self.next_tok()? {
                    Tok::Int(value) => -value,
                    _ => {
                        return Err(Diag::new(
                            vline,
                            vcol,
                            "`data` values must be integer literals".to_string(),
                        ))
                    }
                },
                _ => {
                    return Err(Diag::new(
                        vline,
                        vcol,
                        "`data` values must be integer literals".to_string(),
                    ))
                }
            };
            values.push(value);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBracket)?;
        self.expect(Tok::Semi)?;
        if values.is_empty() {
            return Err(Diag::new(
                line,
                col,
                format!("`data {name}` must have at least one value"),
            ));
        }
        Ok(DataDef {
            name,
            width,
            values,
            line,
            col,
            name_line,
            name_col,
        })
    }

    fn const_def(&mut self) -> Result<ConstDef, Diag> {
        self.expect(Tok::Const)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::Eq)?;
        let (line, col) = self.position();
        let (value, ty) = match self.next_tok()? {
            Tok::Int(value) => (value, Type::Int),
            Tok::True => (1, Type::Bool),
            Tok::False => (0, Type::Bool),
            Tok::Minus => match self.next_tok()? {
                Tok::Int(value) => (-value, Type::Int),
                _ => {
                    return Err(Diag::new(
                        line,
                        col,
                        "const value must be a literal".to_string(),
                    ))
                }
            },
            Tok::Ident(helper) if is_capacity_helper(&helper) => {
                (self.capacity_helper_call(&helper, line, col)?, Type::Int)
            }
            Tok::Ident(name) if name == "asset_len" && self.at(&Tok::LParen) => {
                (self.asset_len_call(line, col)?, Type::Int)
            }
            Tok::Ident(name) if is_text_asset_helper(&name) && self.at(&Tok::LParen) => {
                (self.text_asset_helper_call(&name, line, col)?, Type::Int)
            }
            Tok::Ident(name) => match self.const_values.get(&name) {
                Some((value, ty)) => (*value, ty.clone()),
                None => {
                    return Err(Diag::new(
                        line,
                        col,
                        "const value must be a literal, prior const, or capacity helper"
                            .to_string(),
                    ))
                }
            },
            _ => {
                return Err(Diag::new(
                    line,
                    col,
                    "const value must be a literal, prior const, or capacity helper".to_string(),
                ))
            }
        };
        // KOTO-0238: an integer initializer may continue as an additive chain
        // (`const TOTAL = TITLE_BYTES + STATUS_BYTES + ...`); bool consts fall
        // through to the `;` and keep their existing diagnostic.
        let value = if ty == Type::Int {
            self.const_additive_tail(value)?.0
        } else {
            value
        };
        self.expect(Tok::Semi)?;
        Ok(ConstDef {
            name,
            value,
            ty,
            name_line,
            name_col,
        })
    }

    /// Fold one two-argument KotoUI capacity helper at compile time, with the
    /// caller positioned just past the helper identifier. Shared by top-level
    /// `const` initializers, local `buf` sizes (KOTO-0233), and struct
    /// buffer-field sizes (KOTO-0235) so every site gets the same KotoUI v1
    /// boundary diagnostics. The packet constructors (KOTO-0232) check the
    /// mount/update wire capacities; the storage constructors (KOTO-0236)
    /// check the `TextResource::parse` / `UiListRowsBuilder::begin` bounds.
    fn capacity_helper_call(&mut self, helper: &str, line: usize, col: usize) -> Result<i64, Diag> {
        self.expect(Tok::LParen)?;
        let records = self.const_int_argument()?;
        self.expect(Tok::Comma)?;
        let data_capacity = self.const_int_argument()?;
        self.expect(Tok::RParen)?;
        let (header, stride, max_records, max_data, max_bytes, domain) = match helper {
            "ui_mount_capacity" => (
                UI_MOUNT_HEADER_SIZE as i64,
                UI_NODE_RECORD_SIZE as i64,
                UI_MAX_NODES as i64,
                UI_DATA_CAPACITY as i64,
                UI_MAX_MOUNT_BYTES as i64,
                "packet",
            ),
            "ui_update_capacity" => (
                UI_UPDATE_HEADER_SIZE as i64,
                UI_UPDATE_RECORD_SIZE as i64,
                UI_MAX_UPDATE_RECORDS as i64,
                UI_DATA_CAPACITY as i64,
                UI_MAX_UPDATE_BYTES as i64,
                "packet",
            ),
            "ui_text_resource_capacity" => (
                0,
                UI_TEXT_RESOURCE_LINE_STRIDE,
                UI_TEXT_RESOURCE_MAX_LINES,
                UI_STORAGE_MAX_BYTES,
                UI_STORAGE_MAX_BYTES,
                "text resource",
            ),
            _ => (
                0,
                UI_LIST_ROW_RECORD_STRIDE,
                UI_MAX_LIST_ROWS as i64,
                UI_STORAGE_MAX_BYTES,
                UI_STORAGE_MAX_BYTES,
                "list rows",
            ),
        };
        let total = records
            .checked_mul(stride)
            .and_then(|records_bytes| header.checked_add(records_bytes))
            .and_then(|records_end| records_end.checked_add(data_capacity));
        if records < 1
            || records > max_records
            || data_capacity < 0
            || data_capacity > max_data
            || total.is_none_or(|total| total > max_bytes)
        {
            return Err(Diag::new(
                line,
                col,
                format!("{helper} arguments exceed the KotoUI v1 {domain} capacities"),
            ));
        }
        Ok(total.unwrap())
    }

    /// Fold `asset_len("path", ...)` (KOTO-0236) at compile time, with the
    /// caller positioned just past the identifier: the byte size of one
    /// manifest-declared package asset, or the maximum across several. Paths
    /// are string literals in the `asset_load` namespace — position-independent
    /// package paths, not include-style source-relative ones.
    fn asset_len_call(&mut self, line: usize, col: usize) -> Result<i64, Diag> {
        self.expect(Tok::LParen)?;
        if self.at(&Tok::RParen) {
            return Err(Diag::new(
                line,
                col,
                "`asset_len` takes at least one package asset path".to_string(),
            ));
        }
        let mut max = 0i64;
        loop {
            let (aline, acol) = self.position();
            match self.next_tok()? {
                Tok::Str(bytes) => {
                    // Source text is UTF-8, so a literal path always re-encodes.
                    let path = String::from_utf8(bytes).map_err(|_| {
                        Diag::new(
                            aline,
                            acol,
                            "`asset_len` paths must be valid UTF-8".to_string(),
                        )
                    })?;
                    let size = self.assets.asset_len(&path).map_err(|message| {
                        Diag::new(aline, acol, format!("asset_len: {message}"))
                    })?;
                    if size > i32::MAX as u64 {
                        return Err(Diag::new(
                            aline,
                            acol,
                            format!("asset_len: \"{path}\" exceeds the 32-bit size domain"),
                        ));
                    }
                    max = max.max(size as i64);
                }
                _ => {
                    return Err(Diag::new(
                        aline,
                        acol,
                        "`asset_len` arguments must be string-literal package asset paths"
                            .to_string(),
                    ))
                }
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        Ok(max)
    }

    fn text_asset_helper_call(
        &mut self,
        helper: &str,
        line: usize,
        col: usize,
    ) -> Result<i64, Diag> {
        self.expect(Tok::LParen)?;
        let range = if helper != "asset_text_line_count" {
            let first_line = self.const_int_argument()?;
            self.expect(Tok::Comma)?;
            let line_count = self.const_int_argument()?;
            if self.at(&Tok::RParen) {
                return Err(Diag::new(
                    line,
                    col,
                    format!("`{helper}` takes at least one package asset path"),
                ));
            }
            self.expect(Tok::Comma)?;
            let end = first_line.checked_add(line_count);
            if first_line < 0 || line_count < 1 || end.is_none_or(|end| end > i32::MAX as i64) {
                return Err(Diag::new(
                    line,
                    col,
                    format!(
                        "`{helper}` requires a non-negative first line and a \
                         positive line count in the 32-bit integer domain"
                    ),
                ));
            }
            Some((first_line as usize, end.unwrap() as usize))
        } else {
            None
        };
        if self.at(&Tok::RParen) {
            return Err(Diag::new(
                line,
                col,
                format!("`{helper}` takes at least one package asset path"),
            ));
        }
        let mut expected = None;
        let mut max_range_bytes = 0usize;
        loop {
            let (asset_line, asset_col) = self.position();
            let Tok::Str(path_bytes) = self.next_tok()? else {
                return Err(Diag::new(
                    asset_line,
                    asset_col,
                    format!(
                        "`{helper}` asset arguments must be string-literal package asset paths"
                    ),
                ));
            };
            let path = String::from_utf8(path_bytes).map_err(|_| {
                Diag::new(
                    asset_line,
                    asset_col,
                    format!("`{helper}` paths must be valid UTF-8"),
                )
            })?;
            let bytes = self.assets.asset_bytes(&path).map_err(|message| {
                Diag::new(asset_line, asset_col, format!("{helper}: {message}"))
            })?;
            let lengths = text_asset_line_lengths(&bytes).map_err(|message| {
                Diag::new(
                    asset_line,
                    asset_col,
                    format!("{helper}: \"{path}\" {message}"),
                )
            })?;
            if let Some((first_line, end)) = range {
                if end > lengths.len() {
                    return Err(Diag::new(
                        asset_line,
                        asset_col,
                        format!(
                            "{helper}: \"{path}\" range {first_line}..{end} exceeds its {} lines",
                            lengths.len()
                        ),
                    ));
                }
                // The two range helpers answer distinct sizing questions:
                // `max_range_bytes` bounds the payload one packet copies (the
                // largest per-asset range *sum*), `max_line_bytes` bounds a
                // retained slot that holds one-of-N indexed lines (the largest
                // *single* line across assets).
                let range_bytes = if helper == "asset_text_max_line_bytes" {
                    lengths[first_line..end].iter().copied().max()
                } else {
                    lengths[first_line..end]
                        .iter()
                        .try_fold(0usize, |sum, length| sum.checked_add(*length))
                };
                if range_bytes.is_none_or(|bytes| bytes > i32::MAX as usize) {
                    return Err(Diag::new(
                        asset_line,
                        asset_col,
                        format!("{helper}: \"{path}\" range exceeds the 32-bit size domain"),
                    ));
                }
                max_range_bytes = max_range_bytes.max(range_bytes.unwrap());
            } else {
                let count = lengths.len();
                if count > i32::MAX as usize {
                    return Err(Diag::new(
                        asset_line,
                        asset_col,
                        format!("{helper}: \"{path}\" exceeds the 32-bit line-count domain"),
                    ));
                }
                if let Some(expected) = expected {
                    if count != expected {
                        return Err(Diag::new(
                            asset_line,
                            asset_col,
                            format!("{helper}: \"{path}\" has {count} lines; expected {expected}"),
                        ));
                    }
                } else {
                    expected = Some(count);
                }
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        Ok(range.map_or(expected.unwrap_or(0), |_| max_range_bytes) as i64)
    }

    /// Parse the size of a `buf[...]` declaration, with the caller positioned
    /// just past the `[`. Shared by statement-level `buf` locals (KOTO-0233)
    /// and struct buffer fields (KOTO-0235) so both sites accept the same
    /// forms — positive integer literal, prior integer `const`, a folded
    /// capacity-helper call, or a folded `asset_len` call (KOTO-0236) — with
    /// identical diagnostics.
    fn buf_size(&mut self) -> Result<usize, Diag> {
        let (sline, scol) = self.position();
        let generic = || {
            Diag::new(
                sline,
                scol,
                "buffer size must be a positive integer literal, prior integer const, capacity helper, or `asset_len`"
                    .to_string(),
            )
        };
        // First atom; the positivity check moves after the KOTO-0238 additive
        // tail so a zero atom may still contribute to a positive folded total
        // (`buf tmp[NOTE_DOC_BYTES + 4]`), while lone zero atoms keep their
        // existing per-form diagnostics.
        let (first, zero_diag) = match self.next_tok()? {
            Tok::Int(value) => (value, None),
            // KOTO-0233: a packet buffer states its record/data facts at
            // the declaration instead of a file-scope `*_BYTES` const.
            // The folded total is always >= the record table, so it is
            // positive by construction.
            Tok::Ident(helper) if is_capacity_helper(&helper) && self.at(&Tok::LParen) => {
                (self.capacity_helper_call(&helper, sline, scol)?, None)
            }
            // KOTO-0236: a buffer sized by its packaged asset bytes. Unlike
            // the helpers, an empty asset folds to zero, which cannot size a
            // region.
            Tok::Ident(name) if name == "asset_len" && self.at(&Tok::LParen) => (
                self.asset_len_call(sline, scol)?,
                Some(Diag::new(
                    sline,
                    scol,
                    "`asset_len` folded to 0 bytes; buffer sizes must be positive".to_string(),
                )),
            ),
            Tok::Ident(name) if is_text_asset_helper(&name) && self.at(&Tok::LParen) => {
                let size = self.text_asset_helper_call(&name, sline, scol)?;
                (
                    size,
                    Some(Diag::new(
                        sline,
                        scol,
                        format!("`{name}` folded to 0; buffer sizes must be positive"),
                    )),
                )
            }
            Tok::Ident(name) => match self.const_values.get(&name) {
                Some((value, Type::Int)) => (*value, None),
                _ => return Err(generic()),
            },
            _ => return Err(generic()),
        };
        let (size, chained) = self.const_additive_tail(first)?;
        if size > 0 {
            return Ok(size as usize);
        }
        if chained {
            return Err(Diag::new(
                sline,
                scol,
                format!("buffer size folded to {size}; buffer sizes must be positive"),
            ));
        }
        Err(match zero_diag {
            Some(diag) if size == 0 => diag,
            _ => generic(),
        })
    }

    fn const_int_argument(&mut self) -> Result<i64, Diag> {
        let first = self.const_int_atom()?;
        Ok(self.const_additive_tail(first)?.0)
    }

    fn const_int_atom(&mut self) -> Result<i64, Diag> {
        let (line, col) = self.position();
        match self.next_tok()? {
            Tok::Int(value) => Ok(value),
            Tok::Minus => match self.next_tok()? {
                Tok::Int(value) => Ok(-value),
                _ => Err(Self::const_argument_diag(line, col)),
            },
            // KOTO-0236: compile-time forms compose — a helper argument may
            // itself be a folded `asset_len` or capacity-helper call, so a
            // derived sizing like `ui_text_resource_capacity(22, asset_len(...))`
            // needs no intermediate const.
            Tok::Ident(name) if name == "asset_len" && self.at(&Tok::LParen) => {
                self.asset_len_call(line, col)
            }
            Tok::Ident(name) if is_text_asset_helper(&name) && self.at(&Tok::LParen) => {
                self.text_asset_helper_call(&name, line, col)
            }
            Tok::Ident(helper) if is_capacity_helper(&helper) && self.at(&Tok::LParen) => {
                self.capacity_helper_call(&helper, line, col)
            }
            Tok::Ident(name) => match self.const_values.get(&name) {
                Some((value, Type::Int)) => Ok(*value),
                _ => Err(Self::const_argument_diag(line, col)),
            },
            _ => Err(Self::const_argument_diag(line, col)),
        }
    }

    /// KOTO-0238: fold a left-associative `+`/`-` chain over the compile-time
    /// integer atoms, checking the signed 32-bit Koto integer domain at every
    /// step. Shared by the three compile-time integer positions — `const`
    /// initializers, `buf`/buffer-field sizes, and helper integer arguments —
    /// so arena totals compose from independently derived capacities without
    /// hand-computed literals. Returns the folded value and whether at least
    /// one operator was consumed (single atoms keep their existing checks).
    fn const_additive_tail(&mut self, first: i64) -> Result<(i64, bool), Diag> {
        let mut value = first;
        let mut chained = false;
        while self.at(&Tok::Plus) || self.at(&Tok::Minus) {
            let (op_line, op_col) = self.position();
            let overflow = || Diag::new(op_line, op_col, Self::ADDITIVE_DOMAIN_MESSAGE.to_string());
            if !chained && i32::try_from(value).is_err() {
                return Err(overflow());
            }
            let subtract = matches!(self.next_tok()?, Tok::Minus);
            let rhs = self.const_int_atom()?;
            let step = if subtract {
                value.checked_sub(rhs)
            } else {
                value.checked_add(rhs)
            };
            value = match step {
                Some(step) if i32::try_from(step).is_ok() => step,
                _ => return Err(overflow()),
            };
            chained = true;
        }
        Ok((value, chained))
    }

    const ADDITIVE_DOMAIN_MESSAGE: &'static str =
        "compile-time `+`/`-` folding leaves the signed 32-bit integer domain";

    fn const_argument_diag(line: usize, col: usize) -> Diag {
        Diag::new(
            line,
            col,
            "capacity helper arguments must be integer literals, prior integer consts, \
                 capacity helper calls, `asset_len`, or a text asset helper"
                .to_string(),
        )
    }

    fn function(&mut self) -> Result<Function, Diag> {
        self.function_for(None)
    }

    fn function_for(&mut self, receiver: Option<&str>) -> Result<Function, Diag> {
        let (line, col) = self.position();
        self.expect(Tok::Fn)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            let (param_line, param_col) = self.position();
            let pname = self.ident()?;
            let implicit_receiver =
                receiver.filter(|_| params.is_empty() && pname == "self" && !self.at(&Tok::Colon));
            let ty = if let Some(receiver_name) = implicit_receiver {
                Type::Struct(receiver_name.to_string())
            } else {
                self.expect(Tok::Colon)?;
                self.ty()?
            };
            params.push(Param {
                name: pname,
                ty,
                line: param_line,
                col: param_col,
            });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.ty()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(Function {
            name,
            params,
            ret,
            body,
            line,
            col,
            name_line,
            name_col,
        })
    }

    fn ty(&mut self) -> Result<Type, Diag> {
        let (line, col) = self.position();
        match self.next_tok()? {
            Tok::KwInt => Ok(Type::Int),
            Tok::KwBool => Ok(Type::Bool),
            Tok::Ident(name) => Ok(Type::Struct(name)),
            _ => Err(Diag::new(
                line,
                col,
                "expected a type (`int`, `bool`, or a struct name)".to_string(),
            )),
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Diag> {
        self.expect(Tok::LBrace)?;
        let mut stmts = Vec::new();
        while !self.at(&Tok::RBrace) && !self.at_end() {
            stmts.push(self.statement()?);
        }
        self.expect(Tok::RBrace)?;
        Ok(stmts)
    }

    fn statement(&mut self) -> Result<Stmt, Diag> {
        let (line, col) = self.position();
        match self.peek_tok() {
            Tok::Let => {
                self.next_tok()?;
                let name = self.ident()?;
                let annotation = if self.eat(&Tok::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect(Tok::Eq)?;
                let value = self.expr()?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Let {
                    name,
                    annotation,
                    value,
                    line,
                    col,
                })
            }
            Tok::Buf => {
                self.next_tok()?;
                let name = self.ident()?;
                self.expect(Tok::LBracket)?;
                let size = self.buf_size()?;
                self.expect(Tok::RBracket)?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::BufDecl {
                    name,
                    size,
                    line,
                    col,
                })
            }
            Tok::Static => Err(Diag::new(
                line,
                col,
                "`static` is only allowed at top level".to_string(),
            )),
            Tok::If => self.if_statement(),
            Tok::While => {
                self.next_tok()?;
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::Loop => {
                self.next_tok()?;
                let body = self.block()?;
                Ok(Stmt::Loop { body })
            }
            Tok::Break => {
                self.next_tok()?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Break { line, col })
            }
            Tok::Continue => {
                self.next_tok()?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Continue { line, col })
            }
            Tok::Return => {
                self.next_tok()?;
                let value = if self.at(&Tok::Semi) {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.expect(Tok::Semi)?;
                Ok(Stmt::Return { value, line, col })
            }
            // assignment or expression statement
            Tok::Ident(_) => self.assign_or_expr(line, col),
            _ => {
                let value = self.expr()?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Expr(value))
            }
        }
    }

    fn assign_or_expr(&mut self, line: usize, col: usize) -> Result<Stmt, Diag> {
        // Look ahead: IDENT = ...  | IDENT [ idx ] = ...  | expression ;
        let name = match self.peek_tok().clone() {
            Tok::Ident(name) => name,
            _ => unreachable!("assign_or_expr entered without ident"),
        };
        // simple assignment: ident '='
        if matches!(self.peek_at(1), Some(Tok::Eq)) {
            self.next_tok()?; // ident
            self.next_tok()?; // '='
            let value = self.expr()?;
            self.expect(Tok::Semi)?;
            return Ok(Stmt::Assign {
                name,
                value,
                line,
                col,
            });
        }
        // buffer store: ident '[' expr ']' '='
        if matches!(self.peek_at(1), Some(Tok::LBracket)) {
            // Tentatively parse `ident [ expr ]`; if followed by `=`, it is a store.
            let save = self.pos;
            self.next_tok()?; // ident
            self.next_tok()?; // '['
            let index = self.expr()?;
            self.expect(Tok::RBracket)?;
            if self.eat(&Tok::Eq) {
                let value = self.expr()?;
                self.expect(Tok::Semi)?;
                return Ok(Stmt::BufStore {
                    name,
                    index,
                    value,
                    line,
                    col,
                });
            }
            // not a store; rewind and parse as a normal expression statement.
            self.pos = save;
        }
        if matches!(self.peek_at(1), Some(Tok::Dot)) {
            let save = self.pos;
            let field_expr = self.primary()?;
            if self.eat(&Tok::Eq) {
                if let Expr::Field { receiver, name, .. } = field_expr {
                    let value = self.expr()?;
                    self.expect(Tok::Semi)?;
                    return Ok(Stmt::FieldAssign {
                        receiver: *receiver,
                        field: name,
                        value,
                        line,
                        col,
                    });
                }
            }
            self.pos = save;
        }
        let value = self.expr()?;
        self.expect(Tok::Semi)?;
        Ok(Stmt::Expr(value))
    }

    fn if_statement(&mut self) -> Result<Stmt, Diag> {
        self.expect(Tok::If)?;
        let cond = self.expr()?;
        let then = self.block()?;
        let otherwise = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                vec![self.if_statement()?]
            } else {
                self.block()?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt::If {
            cond,
            then,
            otherwise,
        })
    }

    // Expression parsing via precedence climbing.
    fn expr(&mut self) -> Result<Expr, Diag> {
        self.binary(0)
    }

    fn binary(&mut self, min_prec: u8) -> Result<Expr, Diag> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = self.peek_binop() {
            if prec < min_prec {
                break;
            }
            let (line, col) = self.position();
            self.next_tok()?;
            let rhs = self.binary(prec + 1)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                line,
                col,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Diag> {
        let (line, col) = self.position();
        match self.peek_tok() {
            Tok::Minus => {
                self.next_tok()?;
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(self.unary()?),
                    line,
                    col,
                })
            }
            Tok::Bang => {
                self.next_tok()?;
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(self.unary()?),
                    line,
                    col,
                })
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, Diag> {
        let (line, col) = self.position();
        let mut expr = match self.next_tok()? {
            Tok::Int(value) => Ok(Expr::Int { value, line, col }),
            Tok::True => Ok(Expr::Bool {
                value: true,
                line,
                col,
            }),
            Tok::False => Ok(Expr::Bool {
                value: false,
                line,
                col,
            }),
            Tok::Str(bytes) => Ok(Expr::Str { bytes, line, col }),
            Tok::LParen => {
                let inner = self.expr()?;
                self.expect(Tok::RParen)?;
                Ok(inner)
            }
            Tok::Ident(name) => {
                if self.eat(&Tok::ColonColon) {
                    let member_name = self.ident()?;
                    Ok(Expr::EnumMember {
                        enum_name: name,
                        member_name,
                        line,
                        col,
                    })
                } else if self.eat(&Tok::LParen) {
                    // KOTO-0236: unlike the capacity helpers, `asset_len` has
                    // no runtime SDK form — sizes exist only at compile time.
                    if name == "asset_len" || is_text_asset_helper(&name) {
                        return Err(Diag::new(
                            line,
                            col,
                            format!(
                                "`{name}` is compile-time only: use it in a `const` initializer, \
                                 a `buf` or buffer-field size, or a capacity helper argument"
                            ),
                        ));
                    }
                    let mut args = Vec::new();
                    while !self.at(&Tok::RParen) {
                        args.push(self.expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    Ok(Expr::Call {
                        name,
                        args,
                        line,
                        col,
                    })
                } else if self.eat(&Tok::LBracket) {
                    let index = self.expr()?;
                    self.expect(Tok::RBracket)?;
                    Ok(Expr::BufIndex {
                        name,
                        index: Box::new(index),
                        line,
                        col,
                    })
                } else {
                    Ok(Expr::Ident { name, line, col })
                }
            }
            other => Err(Diag::new(
                line,
                col,
                format!("unexpected token in expression: {other:?}"),
            )),
        }?;
        while self.eat(&Tok::Dot) {
            let (member_line, member_col) = self.position();
            let name = self.ident()?;
            if self.eat(&Tok::LParen) {
                let mut args = Vec::new();
                while !self.at(&Tok::RParen) {
                    args.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen)?;
                expr = Expr::MethodCall {
                    receiver: Box::new(expr),
                    name,
                    args,
                    line: member_line,
                    col: member_col,
                };
            } else {
                expr = Expr::Field {
                    receiver: Box::new(expr),
                    name,
                    line: member_line,
                    col: member_col,
                };
            }
        }
        Ok(expr)
    }

    fn peek_binop(&self) -> Option<(BinOp, u8)> {
        let op = match self.tokens.get(self.pos)?.tok {
            Tok::OrOr => (BinOp::LOr, 1),
            Tok::AndAnd => (BinOp::LAnd, 2),
            Tok::Pipe => (BinOp::Or, 3),
            Tok::Caret => (BinOp::Xor, 4),
            Tok::Amp => (BinOp::And, 5),
            Tok::EqEq => (BinOp::Eq, 6),
            Tok::Ne => (BinOp::Ne, 6),
            Tok::Lt => (BinOp::Lt, 7),
            Tok::Le => (BinOp::Le, 7),
            Tok::Gt => (BinOp::Gt, 7),
            Tok::Ge => (BinOp::Ge, 7),
            Tok::Shl => (BinOp::Shl, 8),
            Tok::Shr => (BinOp::Shr, 8),
            Tok::Plus => (BinOp::Add, 9),
            Tok::Minus => (BinOp::Sub, 9),
            Tok::Star => (BinOp::Mul, 10),
            Tok::Slash => (BinOp::Div, 10),
            Tok::Percent => (BinOp::Mod, 10),
            _ => return None,
        };
        Some(op)
    }

    // token helpers

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek_tok(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_at(&self, offset: usize) -> Option<&Tok> {
        self.tokens.get(self.pos + offset).map(|token| &token.tok)
    }

    fn position(&self) -> (usize, usize) {
        match self.tokens.get(self.pos) {
            Some(token) => (token.line, token.col),
            None => self
                .tokens
                .last()
                .map(|token| (token.line, token.col))
                .unwrap_or((1, 1)),
        }
    }

    fn next_tok(&mut self) -> Result<Tok, Diag> {
        if self.at_end() {
            let (line, col) = self.position();
            return Err(Diag::new(line, col, "unexpected end of input".to_string()));
        }
        let tok = self.tokens[self.pos].tok.clone();
        self.pos += 1;
        Ok(tok)
    }

    fn at(&self, tok: &Tok) -> bool {
        !self.at_end() && self.peek_tok() == tok
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Tok) -> Result<(), Diag> {
        if self.at(&tok) {
            self.pos += 1;
            Ok(())
        } else {
            let (line, col) = self.position();
            Err(Diag::new(line, col, format!("expected {tok:?}")))
        }
    }

    fn ident(&mut self) -> Result<String, Diag> {
        let (line, col) = self.position();
        match self.next_tok()? {
            Tok::Ident(name) => Ok(name),
            other => Err(Diag::new(
                line,
                col,
                format!("expected an identifier, found {other:?}"),
            )),
        }
    }
}
