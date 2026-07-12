//! Tokenizer for the Koto app language. Tracks 1-based line/column for
//! diagnostics and rejects malformed tokens with a [`Diag`].

use crate::Diag;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Str(Vec<u8>),
    // keywords
    Fn,
    Let,
    Const,
    Buf,
    Data,
    If,
    Else,
    While,
    Loop,
    Break,
    Continue,
    Return,
    True,
    False,
    KwInt,
    KwBool,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Arrow,
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Shl,
    Shr,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eq,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

/// Advance the cursor past `chars[*index]`, updating line/column.
fn advance(chars: &[char], index: &mut usize, line: &mut usize, col: &mut usize) {
    if chars[*index] == '\n' {
        *line += 1;
        *col = 1;
    } else {
        *col += 1;
    }
    *index += 1;
}

pub fn lex(source: &str) -> Result<Vec<Token>, Diag> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut index = 0;
    let mut line = 1;
    let mut col = 1;

    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            advance(&chars, &mut index, &mut line, &mut col);
            continue;
        }
        if ch == '/' && index + 1 < chars.len() && chars[index + 1] == '/' {
            while index < chars.len() && chars[index] != '\n' {
                advance(&chars, &mut index, &mut line, &mut col);
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut name = String::new();
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                name.push(chars[index]);
                advance(&chars, &mut index, &mut line, &mut col);
            }
            tokens.push(Token {
                tok: keyword_or_ident(name),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        if ch.is_ascii_digit() {
            let value = lex_number(
                &chars, &mut index, &mut line, &mut col, start_line, start_col,
            )?;
            tokens.push(Token {
                tok: Tok::Int(value),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        if ch == '"' {
            let bytes = lex_string(&chars, &mut index, &mut line, &mut col)?;
            tokens.push(Token {
                tok: Tok::Str(bytes),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        if ch == '\'' {
            let value = lex_char(&chars, &mut index, &mut line, &mut col)?;
            tokens.push(Token {
                tok: Tok::Int(value),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        let next = if index + 1 < chars.len() {
            Some(chars[index + 1])
        } else {
            None
        };
        let (tok, width) = match (ch, next) {
            ('-', Some('>')) => (Tok::Arrow, 2),
            ('<', Some('<')) => (Tok::Shl, 2),
            ('>', Some('>')) => (Tok::Shr, 2),
            ('=', Some('=')) => (Tok::EqEq, 2),
            ('!', Some('=')) => (Tok::Ne, 2),
            ('<', Some('=')) => (Tok::Le, 2),
            ('>', Some('=')) => (Tok::Ge, 2),
            ('&', Some('&')) => (Tok::AndAnd, 2),
            ('|', Some('|')) => (Tok::OrOr, 2),
            _ => {
                let single = match ch {
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    '{' => Tok::LBrace,
                    '}' => Tok::RBrace,
                    '[' => Tok::LBracket,
                    ']' => Tok::RBracket,
                    ',' => Tok::Comma,
                    ';' => Tok::Semi,
                    ':' => Tok::Colon,
                    '+' => Tok::Plus,
                    '-' => Tok::Minus,
                    '*' => Tok::Star,
                    '/' => Tok::Slash,
                    '%' => Tok::Percent,
                    '&' => Tok::Amp,
                    '|' => Tok::Pipe,
                    '^' => Tok::Caret,
                    '<' => Tok::Lt,
                    '>' => Tok::Gt,
                    '!' => Tok::Bang,
                    '=' => Tok::Eq,
                    other => {
                        return Err(Diag::new(
                            start_line,
                            start_col,
                            format!("unexpected character `{other}`"),
                        ))
                    }
                };
                (single, 1)
            }
        };
        for _ in 0..width {
            advance(&chars, &mut index, &mut line, &mut col);
        }
        tokens.push(Token {
            tok,
            line: start_line,
            col: start_col,
        });
    }

    Ok(tokens)
}

fn keyword_or_ident(name: String) -> Tok {
    match name.as_str() {
        "fn" => Tok::Fn,
        "let" => Tok::Let,
        "const" => Tok::Const,
        "buf" => Tok::Buf,
        "data" => Tok::Data,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "loop" => Tok::Loop,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "return" => Tok::Return,
        "true" => Tok::True,
        "false" => Tok::False,
        "int" => Tok::KwInt,
        "bool" => Tok::KwBool,
        _ => Tok::Ident(name),
    }
}

fn lex_number(
    chars: &[char],
    index: &mut usize,
    line: &mut usize,
    col: &mut usize,
    start_line: usize,
    start_col: usize,
) -> Result<i64, Diag> {
    let mut text = String::new();
    let hex = chars[*index] == '0'
        && *index + 1 < chars.len()
        && (chars[*index + 1] == 'x' || chars[*index + 1] == 'X');
    if hex {
        advance(chars, index, line, col);
        advance(chars, index, line, col);
        while *index < chars.len() && (chars[*index].is_ascii_hexdigit() || chars[*index] == '_') {
            if chars[*index] != '_' {
                text.push(chars[*index]);
            }
            advance(chars, index, line, col);
        }
        return i64::from_str_radix(&text, 16)
            .map_err(|_| Diag::new(start_line, start_col, "invalid hex literal".to_string()));
    }
    while *index < chars.len() && (chars[*index].is_ascii_digit() || chars[*index] == '_') {
        if chars[*index] != '_' {
            text.push(chars[*index]);
        }
        advance(chars, index, line, col);
    }
    text.parse::<i64>().map_err(|_| {
        Diag::new(
            start_line,
            start_col,
            format!("invalid integer literal `{text}`"),
        )
    })
}

fn lex_string(
    chars: &[char],
    index: &mut usize,
    line: &mut usize,
    col: &mut usize,
) -> Result<Vec<u8>, Diag> {
    let start_line = *line;
    let start_col = *col;
    advance(chars, index, line, col); // opening quote
    let mut out = String::new();
    while *index < chars.len() && chars[*index] != '"' {
        if chars[*index] == '\\' {
            advance(chars, index, line, col);
            if *index >= chars.len() {
                break;
            }
            let escape = unescape(chars[*index])
                .ok_or_else(|| Diag::new(*line, *col, "unknown string escape".to_string()))?;
            out.push(escape);
            advance(chars, index, line, col);
        } else {
            out.push(chars[*index]);
            advance(chars, index, line, col);
        }
    }
    if *index >= chars.len() {
        return Err(Diag::new(
            start_line,
            start_col,
            "unterminated string".to_string(),
        ));
    }
    advance(chars, index, line, col); // closing quote
    Ok(out.into_bytes())
}

fn lex_char(
    chars: &[char],
    index: &mut usize,
    line: &mut usize,
    col: &mut usize,
) -> Result<i64, Diag> {
    let start_line = *line;
    let start_col = *col;
    advance(chars, index, line, col); // opening quote
    if *index >= chars.len() {
        return Err(Diag::new(
            start_line,
            start_col,
            "unterminated char literal".to_string(),
        ));
    }
    let ch = if chars[*index] == '\\' {
        advance(chars, index, line, col);
        if *index >= chars.len() {
            return Err(Diag::new(
                start_line,
                start_col,
                "dangling escape".to_string(),
            ));
        }
        let value = unescape(chars[*index])
            .ok_or_else(|| Diag::new(*line, *col, "unknown char escape".to_string()))?;
        advance(chars, index, line, col);
        value
    } else {
        let value = chars[*index];
        advance(chars, index, line, col);
        value
    };
    if *index >= chars.len() || chars[*index] != '\'' {
        return Err(Diag::new(
            start_line,
            start_col,
            "char literal must hold one character".to_string(),
        ));
    }
    advance(chars, index, line, col); // closing quote
    Ok(ch as i64)
}

fn unescape(ch: char) -> Option<char> {
    Some(match ch {
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
