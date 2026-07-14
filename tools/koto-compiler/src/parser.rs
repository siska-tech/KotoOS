//! Recursive-descent parser with precedence climbing for the Koto app language.
//! Produces the AST consumed by codegen. Errors carry source line/column.

use crate::lexer::{Tok, Token};
use crate::Diag;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
}

#[derive(Clone, Debug)]
pub struct Program {
    pub consts: Vec<ConstDef>,
    pub data: Vec<DataDef>,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug)]
pub struct ConstDef {
    pub name: String,
    pub value: i64,
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
}

impl Expr {
    pub fn position(&self) -> (usize, usize) {
        match self {
            Expr::Int { line, col, .. }
            | Expr::Bool { line, col, .. }
            | Expr::Str { line, col, .. }
            | Expr::Ident { line, col, .. }
            | Expr::BufIndex { line, col, .. }
            | Expr::Unary { line, col, .. }
            | Expr::Binary { line, col, .. }
            | Expr::Call { line, col, .. } => (*line, *col),
        }
    }
}

pub fn parse(tokens: &[Token]) -> Result<Program, Diag> {
    let mut parser = Parser { tokens, pos: 0 };
    parser.program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn program(&mut self) -> Result<Program, Diag> {
        let mut consts = Vec::new();
        let mut data = Vec::new();
        let mut functions = Vec::new();
        while !self.at_end() {
            match self.peek_tok() {
                Tok::Const => consts.push(self.const_def()?),
                Tok::Data => data.push(self.data_def()?),
                Tok::Fn => functions.push(self.function()?),
                _ => {
                    let (line, col) = self.position();
                    return Err(Diag::new(
                        line,
                        col,
                        "expected `fn`, `const`, or `data` at top level".to_string(),
                    ));
                }
            }
        }
        Ok(Program {
            consts,
            data,
            functions,
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
        let value = match self.next_tok()? {
            Tok::Int(value) => value,
            Tok::True => 1,
            Tok::False => 0,
            Tok::Minus => match self.next_tok()? {
                Tok::Int(value) => -value,
                _ => {
                    return Err(Diag::new(
                        line,
                        col,
                        "const value must be a literal".to_string(),
                    ))
                }
            },
            _ => {
                return Err(Diag::new(
                    line,
                    col,
                    "const value must be a literal".to_string(),
                ))
            }
        };
        self.expect(Tok::Semi)?;
        Ok(ConstDef {
            name,
            value,
            name_line,
            name_col,
        })
    }

    fn function(&mut self) -> Result<Function, Diag> {
        let (line, col) = self.position();
        self.expect(Tok::Fn)?;
        let (name_line, name_col) = self.position();
        let name = self.ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            let (param_line, param_col) = self.position();
            let pname = self.ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.ty()?;
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
            _ => Err(Diag::new(
                line,
                col,
                "expected a type (`int` or `bool`)".to_string(),
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
                // An optional `: type` annotation is accepted but not yet used.
                if self.eat(&Tok::Colon) {
                    let _ = self.ty()?;
                }
                self.expect(Tok::Eq)?;
                let value = self.expr()?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Let {
                    name,
                    value,
                    line,
                    col,
                })
            }
            Tok::Buf => {
                self.next_tok()?;
                let name = self.ident()?;
                self.expect(Tok::LBracket)?;
                let (sline, scol) = self.position();
                let size = match self.next_tok()? {
                    Tok::Int(value) if value > 0 => value as usize,
                    _ => {
                        return Err(Diag::new(
                            sline,
                            scol,
                            "buffer size must be a positive integer".to_string(),
                        ))
                    }
                };
                self.expect(Tok::RBracket)?;
                self.expect(Tok::Semi)?;
                Ok(Stmt::BufDecl {
                    name,
                    size,
                    line,
                    col,
                })
            }
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
        match self.next_tok()? {
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
                if self.eat(&Tok::LParen) {
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
        }
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
