//! Bounded, allocation-free, incremental JSON decoder for Koto applications
//! (KOTO-0246).
//!
//! The decoder consumes caller-owned input chunks split at any byte boundary
//! and emits a stable stream of tokens for object/array boundaries, keys,
//! strings, numbers, booleans, and null. It never builds an in-memory document
//! tree: all cross-chunk state lives in a fixed-size struct, and completed
//! string/number token bytes live in a decoder-owned scratch buffer, so a
//! caller can pause after any token and resume without retaining a pointer into
//! a previous VM buffer.
//!
//! Every limit is a public constant with a deterministic failure result rather
//! than hidden heap behavior. Malformed, truncated, over-deep, and oversized
//! input fails with a fixed [`JsonError`] and a bounded byte offset. This is the
//! portable core shared by KotoSim and device builds; host manifest/tooling JSON
//! parsing stays separate and no dynamically allocated DOM is linked here.

/// Maximum object/array nesting depth. A container that would exceed this fails
/// with [`JsonError::DepthExceeded`].
pub const MAX_JSON_DEPTH: usize = 16;

/// Maximum decoded byte length of a single string or key token. Escapes and
/// `\u` sequences are decoded to UTF-8 before this bound is applied; exceeding
/// it fails with [`JsonError::TokenTooLong`].
pub const MAX_JSON_TOKEN_BYTES: usize = 256;

/// Maximum raw byte length of a single numeric literal. Exceeding it fails with
/// [`JsonError::NumberTooLong`].
pub const MAX_JSON_NUMBER_BYTES: usize = 40;

/// Fixed decoder error set. Each value has a stable `#[repr(u8)]` code for the
/// Runtime ABI and application code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum JsonError {
    /// A byte was not valid in the current structural position.
    UnexpectedByte = 1,
    /// A numeric literal violated the JSON number grammar.
    InvalidNumber = 2,
    /// A string contained an unescaped control byte.
    InvalidString = 3,
    /// A `\` escape named an undefined escape character.
    InvalidEscape = 4,
    /// A `\u` escape was malformed or formed an invalid surrogate pairing.
    InvalidUnicode = 5,
    /// A raw multibyte sequence was not valid UTF-8.
    InvalidUtf8 = 6,
    /// Nesting exceeded [`MAX_JSON_DEPTH`].
    DepthExceeded = 7,
    /// A decoded string/key exceeded [`MAX_JSON_TOKEN_BYTES`].
    TokenTooLong = 8,
    /// A numeric literal exceeded [`MAX_JSON_NUMBER_BYTES`].
    NumberTooLong = 9,
    /// Non-whitespace bytes followed a complete top-level value.
    TrailingData = 10,
    /// Input ended while a value or container was still open.
    UnexpectedEnd = 11,
}

impl JsonError {
    /// Stable numeric code for the Runtime ABI.
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// One decoder event. Token payload bytes for [`JsonEvent::Key`],
/// [`JsonEvent::Str`], and [`JsonEvent::Number`] are read separately from
/// [`JsonDecoder::token`] so this enum stays small and `Copy`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonEvent {
    /// The supplied input was fully consumed without completing a token; feed
    /// the next chunk, or call [`JsonDecoder::finish`] at end of input.
    NeedMore,
    /// `{`
    BeginObject,
    /// `}`
    EndObject,
    /// `[`
    BeginArray,
    /// `]`
    EndArray,
    /// An object member name; bytes in [`JsonDecoder::token`].
    Key,
    /// A string value; decoded UTF-8 bytes in [`JsonDecoder::token`].
    Str,
    /// A numeric value; raw literal bytes in [`JsonDecoder::token`].
    Number,
    /// `true` / `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// A complete top-level value followed by only whitespace.
    EndDocument,
    /// Decoding failed at the given byte offset; the decoder stays failed until
    /// [`JsonDecoder::reset`].
    Error(JsonError, u32),
}

/// The kind of value a value-starting event begins. Lets application code check
/// a selected field's type without inspecting the raw event, so a wrong-type
/// field (for example a string where a number was expected) is distinguishable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonValueKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

impl JsonEvent {
    /// The [`JsonValueKind`] this event begins, or `None` for structural, key,
    /// and control events (`EndObject`/`EndArray`/`Key`/`NeedMore`/
    /// `EndDocument`/`Error`).
    pub const fn value_kind(self) -> Option<JsonValueKind> {
        match self {
            JsonEvent::BeginObject => Some(JsonValueKind::Object),
            JsonEvent::BeginArray => Some(JsonValueKind::Array),
            JsonEvent::Str => Some(JsonValueKind::String),
            JsonEvent::Number => Some(JsonValueKind::Number),
            JsonEvent::Bool(_) => Some(JsonValueKind::Bool),
            JsonEvent::Null => Some(JsonValueKind::Null),
            _ => None,
        }
    }

    /// True if this event begins a value (scalar or container).
    pub const fn is_value(self) -> bool {
        self.value_kind().is_some()
    }

    /// Stable Runtime ABI code for this event (see [`event_code`]). `Bool`
    /// splits into [`event_code::FALSE`]/[`event_code::TRUE`] so application
    /// code reads the boolean value without a token fetch; `Error` maps to
    /// [`event_code::ERROR`] and the error/offset pair is read separately.
    pub const fn code(self) -> i32 {
        match self {
            JsonEvent::NeedMore => event_code::NEED_MORE,
            JsonEvent::BeginObject => event_code::BEGIN_OBJECT,
            JsonEvent::EndObject => event_code::END_OBJECT,
            JsonEvent::BeginArray => event_code::BEGIN_ARRAY,
            JsonEvent::EndArray => event_code::END_ARRAY,
            JsonEvent::Key => event_code::KEY,
            JsonEvent::Str => event_code::STR,
            JsonEvent::Number => event_code::NUMBER,
            JsonEvent::Bool(false) => event_code::FALSE,
            JsonEvent::Bool(true) => event_code::TRUE,
            JsonEvent::Null => event_code::NULL,
            JsonEvent::EndDocument => event_code::END_DOCUMENT,
            JsonEvent::Error(_, _) => event_code::ERROR,
        }
    }
}

/// Stable numeric [`JsonEvent`] codes for the Runtime ABI (`json_next` /
/// `json_finish` results). These values are frozen; the SDK constants in the
/// compiler prelude are sourced from here so they cannot drift.
pub mod event_code {
    pub const NEED_MORE: i32 = 0;
    pub const BEGIN_OBJECT: i32 = 1;
    pub const END_OBJECT: i32 = 2;
    pub const BEGIN_ARRAY: i32 = 3;
    pub const END_ARRAY: i32 = 4;
    pub const KEY: i32 = 5;
    pub const STR: i32 = 6;
    pub const NUMBER: i32 = 7;
    pub const FALSE: i32 = 8;
    pub const TRUE: i32 = 9;
    pub const NULL: i32 = 10;
    pub const END_DOCUMENT: i32 = 11;
    pub const ERROR: i32 = 12;
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Container {
    Object,
    Array,
}

/// What the parser expects at the next structural position.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Next {
    /// A value is required (document root, array element after `,`, object
    /// value after `:`).
    Value,
    /// The first array element, or `]` for an empty array.
    ArrayValueOrEnd,
    /// The first object key, or `}` for an empty object.
    KeyOrEnd,
    /// An object key (after `,`).
    Key,
    /// `:` after an object key.
    Colon,
    /// `,` or the matching close bracket after a value inside a container.
    CommaOrEnd,
    /// The top-level value is complete; only whitespace may follow.
    DocEnd,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LexKind {
    None,
    Str,
    Num,
    Kw,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StrState {
    Normal,
    Escape,
    Unicode { acc: u16, count: u8 },
    SurHigh { high: u16 },
    SurBackslash { high: u16 },
    SurLow { high: u16, acc: u16, count: u8 },
    Utf8 { buf: [u8; 4], need: u8, seen: u8 },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NumState {
    Sign,
    IntZero,
    IntDigits,
    DotFirst,
    Frac,
    ExpSign,
    ExpFirst,
    ExpDigits,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Keyword {
    True,
    False,
    Null,
}

impl Keyword {
    const fn bytes(self) -> &'static [u8] {
        match self {
            Keyword::True => b"true",
            Keyword::False => b"false",
            Keyword::Null => b"null",
        }
    }
}

enum Step {
    /// Consume the current byte and continue.
    Advance,
    /// Consume the current byte and return this event.
    Emit(JsonEvent),
    /// Return this event without consuming the current byte (re-dispatched on
    /// the next call). Used when a value terminator closes a number.
    EmitHold(JsonEvent),
    /// Fail at the current byte.
    Fail(JsonError),
}

/// Incremental JSON tokenizer. Fixed size; holds no reference to caller input.
pub struct JsonDecoder {
    scratch: [u8; MAX_JSON_TOKEN_BYTES],
    scratch_len: usize,
    stack: [Container; MAX_JSON_DEPTH],
    depth: usize,
    next: Next,
    lex: LexKind,
    str_state: StrState,
    str_is_key: bool,
    num_state: NumState,
    kw: Keyword,
    kw_matched: u8,
    offset: u32,
    failed: Option<(JsonError, u32)>,
}

impl JsonDecoder {
    pub const fn new() -> Self {
        Self {
            scratch: [0; MAX_JSON_TOKEN_BYTES],
            scratch_len: 0,
            stack: [Container::Object; MAX_JSON_DEPTH],
            depth: 0,
            next: Next::Value,
            lex: LexKind::None,
            str_state: StrState::Normal,
            str_is_key: false,
            num_state: NumState::Sign,
            kw: Keyword::True,
            kw_matched: 0,
            offset: 0,
            failed: None,
        }
    }

    /// Bytes of the token reported by the most recent [`JsonEvent::Key`],
    /// [`JsonEvent::Str`], or [`JsonEvent::Number`]. Valid until the next call
    /// that starts a new token.
    pub fn token(&self) -> &[u8] {
        &self.scratch[..self.scratch_len]
    }

    /// Current container nesting depth. Callers skip an unknown value by noting
    /// the depth before it and reading tokens until the depth returns to that
    /// level (for scalars the depth is unchanged and the single value token is
    /// the whole value).
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// The failure, if the decoder has failed.
    pub const fn failed(&self) -> Option<(JsonError, u32)> {
        self.failed
    }

    /// Total bytes consumed across all calls.
    pub const fn byte_offset(&self) -> u32 {
        self.offset
    }

    /// Clear all state and reuse the buffer for a fresh document.
    pub fn reset(&mut self) {
        self.scratch.fill(0);
        *self = Self::new();
    }

    /// Feed the next input chunk. Returns the number of bytes consumed from
    /// `input` and the resulting event. When the event is [`JsonEvent::NeedMore`]
    /// the whole chunk was consumed; supply more input or call [`finish`]. For
    /// any other event fewer bytes may be consumed, so advance `input` by the
    /// returned count and call again to pull the next token.
    ///
    /// [`finish`]: JsonDecoder::finish
    pub fn next(&mut self, input: &[u8]) -> (usize, JsonEvent) {
        if let Some((error, offset)) = self.failed {
            return (0, JsonEvent::Error(error, offset));
        }
        let mut i = 0;
        while i < input.len() {
            let b = input[i];
            let step = match self.lex {
                LexKind::None => self.step_between(b),
                LexKind::Str => self.step_string(b),
                LexKind::Num => self.step_number(b),
                LexKind::Kw => self.step_keyword(b),
            };
            match step {
                Step::Advance => {
                    i += 1;
                    self.offset = self.offset.saturating_add(1);
                }
                Step::Emit(event) => {
                    i += 1;
                    self.offset = self.offset.saturating_add(1);
                    return (i, event);
                }
                Step::EmitHold(event) => return (i, event),
                Step::Fail(error) => return (i, self.fail(error)),
            }
        }
        (i, JsonEvent::NeedMore)
    }

    /// Signal end of input. Flushes a trailing bare number, then validates that
    /// the document is complete. Call repeatedly until it returns
    /// [`JsonEvent::EndDocument`] or [`JsonEvent::Error`]: a flushed trailing
    /// number is reported as [`JsonEvent::Number`] and requires one more call.
    pub fn finish(&mut self) -> JsonEvent {
        if let Some((error, offset)) = self.failed {
            return JsonEvent::Error(error, offset);
        }
        match self.lex {
            LexKind::Num => {
                if self.num_accepting() {
                    self.lex = LexKind::None;
                    self.after_value();
                    JsonEvent::Number
                } else {
                    self.fail(JsonError::UnexpectedEnd)
                }
            }
            LexKind::Str | LexKind::Kw => self.fail(JsonError::UnexpectedEnd),
            LexKind::None => match self.next {
                Next::DocEnd => JsonEvent::EndDocument,
                _ => self.fail(JsonError::UnexpectedEnd),
            },
        }
    }

    fn fail(&mut self, error: JsonError) -> JsonEvent {
        let at = self.offset;
        self.failed = Some((error, at));
        JsonEvent::Error(error, at)
    }

    fn after_value(&mut self) {
        self.next = if self.depth == 0 {
            Next::DocEnd
        } else {
            Next::CommaOrEnd
        };
    }

    fn step_between(&mut self, b: u8) -> Step {
        if is_ws(b) {
            return Step::Advance;
        }
        match self.next {
            Next::Value => self.start_value(b),
            Next::ArrayValueOrEnd => {
                if b == b']' {
                    self.close(Container::Array)
                } else {
                    self.start_value(b)
                }
            }
            Next::KeyOrEnd => {
                if b == b'}' {
                    self.close(Container::Object)
                } else if b == b'"' {
                    self.begin_string(true)
                } else {
                    Step::Fail(JsonError::UnexpectedByte)
                }
            }
            Next::Key => {
                if b == b'"' {
                    self.begin_string(true)
                } else {
                    Step::Fail(JsonError::UnexpectedByte)
                }
            }
            Next::Colon => {
                if b == b':' {
                    self.next = Next::Value;
                    Step::Advance
                } else {
                    Step::Fail(JsonError::UnexpectedByte)
                }
            }
            Next::CommaOrEnd => match b {
                b',' => {
                    self.next = if self.stack[self.depth - 1] == Container::Object {
                        Next::Key
                    } else {
                        Next::Value
                    };
                    Step::Advance
                }
                b'}' => self.close(Container::Object),
                b']' => self.close(Container::Array),
                _ => Step::Fail(JsonError::UnexpectedByte),
            },
            Next::DocEnd => Step::Fail(JsonError::TrailingData),
        }
    }

    fn start_value(&mut self, b: u8) -> Step {
        match b {
            b'{' => self.open(Container::Object),
            b'[' => self.open(Container::Array),
            b'"' => self.begin_string(false),
            b'-' | b'0'..=b'9' => self.begin_number(b),
            b't' => self.begin_keyword(Keyword::True),
            b'f' => self.begin_keyword(Keyword::False),
            b'n' => self.begin_keyword(Keyword::Null),
            _ => Step::Fail(JsonError::UnexpectedByte),
        }
    }

    fn open(&mut self, kind: Container) -> Step {
        if self.depth == MAX_JSON_DEPTH {
            return Step::Fail(JsonError::DepthExceeded);
        }
        self.stack[self.depth] = kind;
        self.depth += 1;
        match kind {
            Container::Object => {
                self.next = Next::KeyOrEnd;
                Step::Emit(JsonEvent::BeginObject)
            }
            Container::Array => {
                self.next = Next::ArrayValueOrEnd;
                Step::Emit(JsonEvent::BeginArray)
            }
        }
    }

    fn close(&mut self, kind: Container) -> Step {
        if self.depth == 0 || self.stack[self.depth - 1] != kind {
            return Step::Fail(JsonError::UnexpectedByte);
        }
        self.depth -= 1;
        self.after_value();
        Step::Emit(match kind {
            Container::Object => JsonEvent::EndObject,
            Container::Array => JsonEvent::EndArray,
        })
    }

    fn begin_string(&mut self, is_key: bool) -> Step {
        self.lex = LexKind::Str;
        self.str_state = StrState::Normal;
        self.str_is_key = is_key;
        self.scratch_len = 0;
        Step::Advance
    }

    fn begin_number(&mut self, b: u8) -> Step {
        self.lex = LexKind::Num;
        self.scratch_len = 0;
        self.scratch[0] = b;
        self.scratch_len = 1;
        self.num_state = match b {
            b'-' => NumState::Sign,
            b'0' => NumState::IntZero,
            _ => NumState::IntDigits,
        };
        Step::Advance
    }

    fn begin_keyword(&mut self, kw: Keyword) -> Step {
        self.lex = LexKind::Kw;
        self.kw = kw;
        self.kw_matched = 1;
        Step::Advance
    }

    fn step_keyword(&mut self, b: u8) -> Step {
        let word = self.kw.bytes();
        if b != word[self.kw_matched as usize] {
            return Step::Fail(JsonError::UnexpectedByte);
        }
        self.kw_matched += 1;
        if self.kw_matched as usize == word.len() {
            self.lex = LexKind::None;
            self.after_value();
            Step::Emit(match self.kw {
                Keyword::True => JsonEvent::Bool(true),
                Keyword::False => JsonEvent::Bool(false),
                Keyword::Null => JsonEvent::Null,
            })
        } else {
            Step::Advance
        }
    }

    fn step_number(&mut self, b: u8) -> Step {
        if !is_number_char(b) {
            return if self.num_accepting() {
                self.lex = LexKind::None;
                self.after_value();
                Step::EmitHold(JsonEvent::Number)
            } else {
                Step::Fail(JsonError::InvalidNumber)
            };
        }
        let next = match (self.num_state, b) {
            (NumState::Sign, b'0') => NumState::IntZero,
            (NumState::Sign, b'1'..=b'9') => NumState::IntDigits,
            (NumState::IntZero, b'.') => NumState::DotFirst,
            (NumState::IntZero, b'e' | b'E') => NumState::ExpSign,
            (NumState::IntDigits, b'0'..=b'9') => NumState::IntDigits,
            (NumState::IntDigits, b'.') => NumState::DotFirst,
            (NumState::IntDigits, b'e' | b'E') => NumState::ExpSign,
            (NumState::DotFirst, b'0'..=b'9') => NumState::Frac,
            (NumState::Frac, b'0'..=b'9') => NumState::Frac,
            (NumState::Frac, b'e' | b'E') => NumState::ExpSign,
            (NumState::ExpSign, b'+' | b'-') => NumState::ExpFirst,
            (NumState::ExpSign, b'0'..=b'9') => NumState::ExpDigits,
            (NumState::ExpFirst, b'0'..=b'9') => NumState::ExpDigits,
            (NumState::ExpDigits, b'0'..=b'9') => NumState::ExpDigits,
            _ => return Step::Fail(JsonError::InvalidNumber),
        };
        if self.scratch_len >= MAX_JSON_NUMBER_BYTES {
            return Step::Fail(JsonError::NumberTooLong);
        }
        self.scratch[self.scratch_len] = b;
        self.scratch_len += 1;
        self.num_state = next;
        Step::Advance
    }

    const fn num_accepting(&self) -> bool {
        matches!(
            self.num_state,
            NumState::IntZero | NumState::IntDigits | NumState::Frac | NumState::ExpDigits
        )
    }

    fn push_token(&mut self, b: u8) -> Result<(), JsonError> {
        if self.scratch_len >= MAX_JSON_TOKEN_BYTES {
            return Err(JsonError::TokenTooLong);
        }
        self.scratch[self.scratch_len] = b;
        self.scratch_len += 1;
        Ok(())
    }

    fn push_code_point(&mut self, cp: u32) -> Result<(), JsonError> {
        if cp < 0x80 {
            self.push_token(cp as u8)
        } else if cp < 0x800 {
            self.push_token((0xC0 | (cp >> 6)) as u8)?;
            self.push_token((0x80 | (cp & 0x3F)) as u8)
        } else if cp < 0x1_0000 {
            self.push_token((0xE0 | (cp >> 12)) as u8)?;
            self.push_token((0x80 | ((cp >> 6) & 0x3F)) as u8)?;
            self.push_token((0x80 | (cp & 0x3F)) as u8)
        } else {
            self.push_token((0xF0 | (cp >> 18)) as u8)?;
            self.push_token((0x80 | ((cp >> 12) & 0x3F)) as u8)?;
            self.push_token((0x80 | ((cp >> 6) & 0x3F)) as u8)?;
            self.push_token((0x80 | (cp & 0x3F)) as u8)
        }
    }

    fn step_string(&mut self, b: u8) -> Step {
        match self.str_state {
            StrState::Normal => {
                if b == b'"' {
                    self.lex = LexKind::None;
                    return if self.str_is_key {
                        self.next = Next::Colon;
                        Step::Emit(JsonEvent::Key)
                    } else {
                        self.after_value();
                        Step::Emit(JsonEvent::Str)
                    };
                }
                if b == b'\\' {
                    self.str_state = StrState::Escape;
                    return Step::Advance;
                }
                if b < 0x20 {
                    return Step::Fail(JsonError::InvalidString);
                }
                if b < 0x80 {
                    return match self.push_token(b) {
                        Ok(()) => Step::Advance,
                        Err(e) => Step::Fail(e),
                    };
                }
                match utf8_need(b) {
                    Some(need) => {
                        let mut buf = [0u8; 4];
                        buf[0] = b;
                        self.str_state = StrState::Utf8 { buf, need, seen: 1 };
                        Step::Advance
                    }
                    None => Step::Fail(JsonError::InvalidUtf8),
                }
            }
            StrState::Escape => {
                let decoded = match b {
                    b'"' => 0x22,
                    b'\\' => 0x5C,
                    b'/' => 0x2F,
                    b'b' => 0x08,
                    b'f' => 0x0C,
                    b'n' => 0x0A,
                    b'r' => 0x0D,
                    b't' => 0x09,
                    b'u' => {
                        self.str_state = StrState::Unicode { acc: 0, count: 0 };
                        return Step::Advance;
                    }
                    _ => return Step::Fail(JsonError::InvalidEscape),
                };
                self.str_state = StrState::Normal;
                match self.push_token(decoded) {
                    Ok(()) => Step::Advance,
                    Err(e) => Step::Fail(e),
                }
            }
            StrState::Unicode { acc, count } => match hex(b) {
                Some(nibble) => {
                    let acc = (acc << 4) | nibble as u16;
                    let count = count + 1;
                    if count < 4 {
                        self.str_state = StrState::Unicode { acc, count };
                        Step::Advance
                    } else if (0xD800..=0xDBFF).contains(&acc) {
                        self.str_state = StrState::SurHigh { high: acc };
                        Step::Advance
                    } else if (0xDC00..=0xDFFF).contains(&acc) {
                        Step::Fail(JsonError::InvalidUnicode)
                    } else {
                        self.str_state = StrState::Normal;
                        match self.push_code_point(acc as u32) {
                            Ok(()) => Step::Advance,
                            Err(e) => Step::Fail(e),
                        }
                    }
                }
                None => Step::Fail(JsonError::InvalidUnicode),
            },
            StrState::SurHigh { high } => {
                if b == b'\\' {
                    self.str_state = StrState::SurBackslash { high };
                    Step::Advance
                } else {
                    Step::Fail(JsonError::InvalidUnicode)
                }
            }
            StrState::SurBackslash { high } => {
                if b == b'u' {
                    self.str_state = StrState::SurLow {
                        high,
                        acc: 0,
                        count: 0,
                    };
                    Step::Advance
                } else {
                    Step::Fail(JsonError::InvalidUnicode)
                }
            }
            StrState::SurLow { high, acc, count } => match hex(b) {
                Some(nibble) => {
                    let acc = (acc << 4) | nibble as u16;
                    let count = count + 1;
                    if count < 4 {
                        self.str_state = StrState::SurLow { high, acc, count };
                        Step::Advance
                    } else if (0xDC00..=0xDFFF).contains(&acc) {
                        let cp =
                            0x1_0000 + (((high - 0xD800) as u32) << 10) + (acc - 0xDC00) as u32;
                        self.str_state = StrState::Normal;
                        match self.push_code_point(cp) {
                            Ok(()) => Step::Advance,
                            Err(e) => Step::Fail(e),
                        }
                    } else {
                        Step::Fail(JsonError::InvalidUnicode)
                    }
                }
                None => Step::Fail(JsonError::InvalidUnicode),
            },
            StrState::Utf8 {
                mut buf,
                need,
                seen,
            } => {
                buf[seen as usize] = b;
                let seen = seen + 1;
                if seen < need {
                    self.str_state = StrState::Utf8 { buf, need, seen };
                    return Step::Advance;
                }
                if core::str::from_utf8(&buf[..need as usize]).is_err() {
                    return Step::Fail(JsonError::InvalidUtf8);
                }
                self.str_state = StrState::Normal;
                for &raw in &buf[..need as usize] {
                    if let Err(e) = self.push_token(raw) {
                        return Step::Fail(e);
                    }
                }
                Step::Advance
            }
        }
    }
}

impl Default for JsonDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress of a [`JsonValueSkip`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipProgress {
    /// The value is not yet fully consumed; keep feeding events.
    More,
    /// The whole value has been consumed; the decoder is positioned after it.
    Done,
}

/// Consumes exactly one JSON value — a scalar or a whole object/array subtree —
/// across chunk boundaries, so application code can skip an unknown or unwanted
/// field without allocating or recursing.
///
/// Feed every event from [`JsonDecoder::next`] into [`feed`](JsonValueSkip::feed),
/// starting with the value's own first event (the `BeginObject`/`BeginArray` or
/// the scalar `Str`/`Number`/`Bool`/`Null`), until it returns
/// [`SkipProgress::Done`]. Control (`NeedMore`) and object `Key` events are
/// counted as part of the current subtree and left to the caller's input loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonValueSkip {
    open: u32,
    done: bool,
}

impl JsonValueSkip {
    pub const fn new() -> Self {
        Self {
            open: 0,
            done: false,
        }
    }

    /// True once the value has been fully skipped.
    pub const fn is_done(&self) -> bool {
        self.done
    }

    /// Advance the skip with the next decoder event.
    pub fn feed(&mut self, event: JsonEvent) -> SkipProgress {
        if self.done {
            return SkipProgress::Done;
        }
        match event {
            JsonEvent::BeginObject | JsonEvent::BeginArray => self.open += 1,
            JsonEvent::EndObject | JsonEvent::EndArray => {
                self.open = self.open.saturating_sub(1);
                if self.open == 0 {
                    self.done = true;
                }
            }
            // A scalar that is not inside a container being skipped is the
            // whole value.
            JsonEvent::Str | JsonEvent::Number | JsonEvent::Bool(_) | JsonEvent::Null
                if self.open == 0 =>
            {
                self.done = true;
            }
            // Keys, control events, and scalars nested inside the skipped
            // subtree do not change subtree depth.
            _ => {}
        }
        if self.done {
            SkipProgress::Done
        } else {
            SkipProgress::More
        }
    }
}

impl Default for JsonValueSkip {
    fn default() -> Self {
        Self::new()
    }
}

/// The host-owned decoder state behind the `json_*` host calls, shared verbatim
/// by the KotoSim and device hosts so both runtimes expose byte-identical ABI
/// behavior. It pairs one [`JsonDecoder`] with the consumed-byte count of the
/// most recent `next` call, because `json_next` is not idempotent: the VM host
/// call returns only the event code, and the app reads `(consumed, depth)`
/// afterwards through the idempotent `json_status` call.
pub struct JsonHostSession {
    decoder: JsonDecoder,
    last_consumed: u32,
}

impl JsonHostSession {
    pub const fn new() -> Self {
        Self {
            decoder: JsonDecoder::new(),
            last_consumed: 0,
        }
    }

    /// `json_reset`: clear all state for a fresh document.
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.last_consumed = 0;
    }

    /// `json_next`: feed a caller-owned chunk, record how many bytes were
    /// consumed, and return the resulting stable event code.
    pub fn next(&mut self, input: &[u8]) -> i32 {
        let (consumed, event) = self.decoder.next(input);
        self.last_consumed = consumed as u32;
        event.code()
    }

    /// `json_finish`: signal end of input. A flushed trailing bare number is
    /// reported as [`event_code::NUMBER`] and requires one more call. Consumes
    /// no chunk bytes, so the recorded consumed count drops to zero.
    pub fn finish(&mut self) -> i32 {
        self.last_consumed = 0;
        self.decoder.finish().code()
    }

    /// `json_token`: bytes of the most recent `Key`/`Str`/`Number` token.
    pub fn token(&self) -> &[u8] {
        self.decoder.token()
    }

    /// `json_error`: `(error_code, byte_offset)` of the sticky failure, or
    /// `(0, 0)` while the decoder has not failed.
    pub fn error(&self) -> (i32, i32) {
        match self.decoder.failed() {
            Some((error, offset)) => (i32::from(error.code()), offset.min(i32::MAX as u32) as i32),
            None => (0, 0),
        }
    }

    /// `json_status`: `(consumed, depth)` — bytes consumed by the most recent
    /// `json_next` and the current container nesting depth.
    pub fn status(&self) -> (i32, i32) {
        (self.last_consumed as i32, self.decoder.depth() as i32)
    }
}

impl Default for JsonHostSession {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for JsonHostSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The decoder's scratch/stack internals are noise; report the
        // observable session state instead.
        f.debug_struct("JsonHostSession")
            .field("depth", &self.decoder.depth())
            .field("byte_offset", &self.decoder.byte_offset())
            .field("failed", &self.decoder.failed())
            .field("last_consumed", &self.last_consumed)
            .finish()
    }
}

const fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

const fn is_number_char(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E')
}

const fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Number of bytes in a UTF-8 sequence given its leading byte, or `None` for a
/// byte that cannot begin a valid multibyte sequence.
const fn utf8_need(b: u8) -> Option<u8> {
    match b {
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Owned {
        BeginObject,
        EndObject,
        BeginArray,
        EndArray,
        Key(Vec<u8>),
        Str(Vec<u8>),
        Number(Vec<u8>),
        Bool(bool),
        Null,
    }

    fn record(out: &mut Vec<Owned>, decoder: &JsonDecoder, event: JsonEvent) {
        match event {
            JsonEvent::BeginObject => out.push(Owned::BeginObject),
            JsonEvent::EndObject => out.push(Owned::EndObject),
            JsonEvent::BeginArray => out.push(Owned::BeginArray),
            JsonEvent::EndArray => out.push(Owned::EndArray),
            JsonEvent::Key => out.push(Owned::Key(decoder.token().to_vec())),
            JsonEvent::Str => out.push(Owned::Str(decoder.token().to_vec())),
            JsonEvent::Number => out.push(Owned::Number(decoder.token().to_vec())),
            JsonEvent::Bool(v) => out.push(Owned::Bool(v)),
            JsonEvent::Null => out.push(Owned::Null),
            _ => unreachable!("unexpected event: {event:?}"),
        }
    }

    fn drain_finish(
        decoder: &mut JsonDecoder,
        out: &mut Vec<Owned>,
    ) -> Result<(), (JsonError, u32)> {
        loop {
            match decoder.finish() {
                JsonEvent::EndDocument => return Ok(()),
                JsonEvent::Error(e, off) => return Err((e, off)),
                JsonEvent::Number => out.push(Owned::Number(decoder.token().to_vec())),
                other => unreachable!("unexpected finish event: {other:?}"),
            }
        }
    }

    /// Feed `input` in fixed-size chunks, re-presenting unconsumed bytes with
    /// each newly revealed chunk so decoding is exercised across boundaries.
    fn collect_chunked(input: &[u8], chunk: usize) -> Result<Vec<Owned>, (JsonError, u32)> {
        let mut decoder = JsonDecoder::new();
        let mut out = Vec::new();
        let mut consumed = 0usize;
        let mut revealed = 0usize;
        while revealed < input.len() {
            revealed = (revealed + chunk).min(input.len());
            loop {
                let (n, event) = decoder.next(&input[consumed..revealed]);
                consumed += n;
                match event {
                    JsonEvent::NeedMore => break,
                    JsonEvent::Error(e, off) => return Err((e, off)),
                    other => record(&mut out, &decoder, other),
                }
            }
        }
        drain_finish(&mut decoder, &mut out)?;
        Ok(out)
    }

    fn collect(input: &[u8]) -> Result<Vec<Owned>, (JsonError, u32)> {
        collect_chunked(input, input.len().max(1))
    }

    fn key(s: &str) -> Owned {
        Owned::Key(s.as_bytes().to_vec())
    }
    fn str_(s: &str) -> Owned {
        Owned::Str(s.as_bytes().to_vec())
    }
    fn num(s: &str) -> Owned {
        Owned::Number(s.as_bytes().to_vec())
    }

    #[test]
    fn scalars_objects_and_arrays_decode() {
        let events = collect(br#"{"a":1,"b":[true,false,null],"c":"hi"}"#).unwrap();
        assert_eq!(
            events,
            vec![
                Owned::BeginObject,
                key("a"),
                num("1"),
                key("b"),
                Owned::BeginArray,
                Owned::Bool(true),
                Owned::Bool(false),
                Owned::Null,
                Owned::EndArray,
                key("c"),
                str_("hi"),
                Owned::EndObject,
            ]
        );
    }

    #[test]
    fn number_grammar_variants_decode() {
        for literal in [
            "0",
            "-0",
            "42",
            "-42",
            "3.14",
            "-3.14",
            "0.5",
            "1e10",
            "1E10",
            "1e+10",
            "1e-10",
            "-2.5e-3",
            "12345678901234567890",
        ] {
            let events = collect(literal.as_bytes()).unwrap();
            assert_eq!(events, vec![num(literal)], "literal {literal}");
        }
    }

    #[test]
    fn empty_containers_and_whitespace() {
        assert_eq!(
            collect(b"  {\n\t}  ").unwrap(),
            vec![Owned::BeginObject, Owned::EndObject]
        );
        assert_eq!(
            collect(b"[\r\n]").unwrap(),
            vec![Owned::BeginArray, Owned::EndArray]
        );
    }

    #[test]
    fn string_escapes_decode_to_utf8() {
        let events = collect(br#""a\"b\\c\/d\n\t\r\b\f""#).unwrap();
        assert_eq!(events, vec![str_("a\"b\\c/d\n\t\r\u{08}\u{0C}")]);
    }

    #[test]
    fn unicode_escape_and_surrogate_pair_decode() {
        // é (U+00E9) via \u, and 😀 (U+1F600) via a surrogate pair. Raw byte
        // strings keep the backslashes literal, so these are JSON escapes.
        let events = collect(b"\"caf\\u00e9 \\ud83d\\ude00\"").unwrap();
        assert_eq!(events, vec![str_("caf\u{e9} \u{1f600}")]);
    }

    #[test]
    fn raw_multibyte_utf8_passes_through() {
        let events = collect("\"日本語\"".as_bytes()).unwrap();
        assert_eq!(events, vec![str_("日本語")]);
    }

    #[test]
    fn every_byte_boundary_yields_identical_tokens() {
        let input = b"{\"nested\":{\"list\":[1,-2.5e3,\"x\\u00e9y\",true],\"n\":null},\"end\":0}";
        let whole = collect(input).unwrap();
        for chunk in 1..=input.len() {
            assert_eq!(
                collect_chunked(input, chunk).unwrap(),
                whole,
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn maximum_depth_is_accepted_then_rejected() {
        let mut ok = vec![b'['; MAX_JSON_DEPTH];
        ok.extend(core::iter::repeat_n(b']', MAX_JSON_DEPTH));
        assert!(collect(&ok).is_ok());

        let mut deep = ok.clone();
        deep.insert(MAX_JSON_DEPTH, b'[');
        let (error, _) = collect(&deep).unwrap_err();
        assert_eq!(error, JsonError::DepthExceeded);
    }

    #[test]
    fn oversized_token_and_number_fail_deterministically() {
        let mut long_string = Vec::new();
        long_string.push(b'"');
        long_string.extend(core::iter::repeat_n(b'a', MAX_JSON_TOKEN_BYTES + 1));
        long_string.push(b'"');
        assert_eq!(
            collect(&long_string).unwrap_err().0,
            JsonError::TokenTooLong
        );

        let mut long_number = Vec::new();
        long_number.push(b'1');
        long_number.extend(core::iter::repeat_n(b'0', MAX_JSON_NUMBER_BYTES + 1));
        assert_eq!(
            collect(&long_number).unwrap_err().0,
            JsonError::NumberTooLong
        );
    }

    #[test]
    fn adversarial_inputs_fail_with_bounded_offset() {
        let cases: &[(&[u8], JsonError)] = &[
            (b"01", JsonError::InvalidNumber),
            (b"-", JsonError::UnexpectedEnd),
            (b"1.", JsonError::UnexpectedEnd),
            (b"1.2.3", JsonError::InvalidNumber),
            (b"1e", JsonError::UnexpectedEnd),
            (b"1e+", JsonError::UnexpectedEnd),
            (br#""\x""#, JsonError::InvalidEscape),
            (br#""\u12g4""#, JsonError::InvalidUnicode),
            (br#""\ud83d""#, JsonError::InvalidUnicode),
            (br#""\ud83dx""#, JsonError::InvalidUnicode),
            (br#""\ud83d"#, JsonError::UnexpectedEnd),
            (br#""\udc00""#, JsonError::InvalidUnicode),
            (b"\"\x01\"", JsonError::InvalidString),
            (b"\"\xff\"", JsonError::InvalidUtf8),
            (b"{\"a\":1,}", JsonError::UnexpectedByte),
            (b"[1,]", JsonError::UnexpectedByte),
            (b"[1 2]", JsonError::UnexpectedByte),
            (b"{\"a\" 1}", JsonError::UnexpectedByte),
            (b"[1}", JsonError::UnexpectedByte),
            (b"tru", JsonError::UnexpectedEnd),
            (b"nul", JsonError::UnexpectedEnd),
            (b"truex", JsonError::TrailingData),
            (b"1 2", JsonError::TrailingData),
            (b"{}x", JsonError::TrailingData),
            (b"", JsonError::UnexpectedEnd),
            (b"}", JsonError::UnexpectedByte),
        ];
        for (input, expected) in cases {
            let (error, offset) = collect(input).unwrap_err();
            assert_eq!(error, *expected, "input {:?}", core::str::from_utf8(input));
            assert!(
                offset as usize <= input.len(),
                "offset {offset} out of bounds for {input:?}"
            );
        }
    }

    #[test]
    fn error_state_is_sticky_until_reset() {
        let mut decoder = JsonDecoder::new();
        // Drive `[1,]` to its failure.
        let bad = b"[1,]";
        let mut pos = 0;
        loop {
            let (n, event) = decoder.next(&bad[pos..]);
            pos += n;
            match event {
                JsonEvent::Error(..) => break,
                JsonEvent::NeedMore => {
                    assert!(matches!(decoder.finish(), JsonEvent::Error(..)));
                    break;
                }
                _ => {}
            }
        }
        assert!(matches!(
            decoder.failed(),
            Some((JsonError::UnexpectedByte, _))
        ));
        // Any further input keeps returning the same error.
        assert!(matches!(
            decoder.next(b"whatever"),
            (0, JsonEvent::Error(JsonError::UnexpectedByte, _))
        ));
        assert!(matches!(
            decoder.finish(),
            JsonEvent::Error(JsonError::UnexpectedByte, _)
        ));

        // Recovery: reset and parse a fresh document with the same buffer.
        decoder.reset();
        assert!(decoder.failed().is_none());
        let mut out = Vec::new();
        let mut consumed = 0;
        loop {
            let (n, e) = decoder.next(&b"[1,2]"[consumed..]);
            consumed += n;
            match e {
                JsonEvent::NeedMore => break,
                JsonEvent::Error(err, off) => panic!("unexpected error {err:?}@{off}"),
                other => record(&mut out, &decoder, other),
            }
        }
        drain_finish(&mut decoder, &mut out).unwrap();
        assert_eq!(
            out,
            vec![Owned::BeginArray, num("1"), num("2"), Owned::EndArray]
        );
    }

    /// Representative bounded Weather payload: select named fields while safely
    /// skipping objects and arrays the app does not care about, using `depth()`.
    #[test]
    fn weather_fixture_named_field_selection_with_skip() {
        let payload = br#"{
            "location":{"name":"Kyoto","lat":35.0,"lon":135.8},
            "current":{"temp_c":24.5,"humidity":60,"conditions":["cloudy","mild"]},
            "hourly":[{"t":0,"temp_c":24.0},{"t":1,"temp_c":23.5}],
            "ok":true
        }"#;

        let mut decoder = JsonDecoder::new();
        let mut consumed = 0usize;
        let mut top_key: Option<Vec<u8>> = None;
        let mut temp_c: Option<Vec<u8>> = None;
        let mut ok: Option<bool> = None;

        loop {
            let (n, event) = decoder.next(&payload[consumed..]);
            consumed += n;
            match event {
                JsonEvent::NeedMore => break,
                JsonEvent::Error(e, off) => panic!("weather parse failed {e:?}@{off}"),
                JsonEvent::Key if decoder.depth() == 1 => {
                    top_key = Some(decoder.token().to_vec());
                }
                JsonEvent::BeginObject | JsonEvent::BeginArray
                    if decoder.depth() >= 2 && top_key.as_deref() != Some(b"current") =>
                {
                    // Skip an unknown container wholesale by tracking depth.
                    let target = decoder.depth() - 1;
                    while decoder.depth() > target {
                        let (m, ev) = decoder.next(&payload[consumed..]);
                        consumed += m;
                        if let JsonEvent::Error(e, off) = ev {
                            panic!("skip failed {e:?}@{off}");
                        }
                        if let JsonEvent::NeedMore = ev {
                            unreachable!("fully buffered");
                        }
                    }
                }
                JsonEvent::Key if decoder.depth() == 2 && decoder.token() == b"temp_c" => {
                    let (m, ev) = decoder.next(&payload[consumed..]);
                    consumed += m;
                    assert_eq!(ev, JsonEvent::Number);
                    if temp_c.is_none() {
                        temp_c = Some(decoder.token().to_vec());
                    }
                }
                JsonEvent::Bool(v) if top_key.as_deref() == Some(b"ok") => ok = Some(v),
                _ => {}
            }
        }
        assert!(matches!(decoder.finish(), JsonEvent::EndDocument));
        assert_eq!(temp_c.as_deref(), Some(&b"24.5"[..]));
        assert_eq!(ok, Some(true));
    }

    /// Representative bounded MQTT-style telemetry payload arriving one byte at
    /// a time, as a broker might deliver it.
    #[test]
    fn mqtt_fixture_decodes_byte_by_byte() {
        let payload = br#"{"device":"sensor-07","seq":1024,"metrics":{"t":21.3,"rh":48},"alarms":[],"online":true}"#;
        let whole = collect(payload).unwrap();
        assert_eq!(collect_chunked(payload, 1).unwrap(), whole);
        assert_eq!(
            whole,
            vec![
                Owned::BeginObject,
                key("device"),
                str_("sensor-07"),
                key("seq"),
                num("1024"),
                key("metrics"),
                Owned::BeginObject,
                key("t"),
                num("21.3"),
                key("rh"),
                num("48"),
                Owned::EndObject,
                key("alarms"),
                Owned::BeginArray,
                Owned::EndArray,
                key("online"),
                Owned::Bool(true),
                Owned::EndObject,
            ]
        );
    }

    /// Pull the next meaningful event, resolving end-of-buffer via `finish`.
    fn pull(decoder: &mut JsonDecoder, input: &[u8], pos: &mut usize) -> JsonEvent {
        let (n, event) = decoder.next(&input[*pos..]);
        *pos += n;
        match event {
            JsonEvent::NeedMore => decoder.finish(),
            other => other,
        }
    }

    /// Skip one whole value beginning with `first`, using `JsonValueSkip`.
    fn skip_value(decoder: &mut JsonDecoder, input: &[u8], pos: &mut usize, first: JsonEvent) {
        let mut skip = JsonValueSkip::new();
        if skip.feed(first) == SkipProgress::Done {
            return;
        }
        loop {
            let event = pull(decoder, input, pos);
            if let JsonEvent::Error(e, off) = event {
                panic!("skip hit error {e:?}@{off}");
            }
            if skip.feed(event) == SkipProgress::Done {
                return;
            }
        }
    }

    #[test]
    fn value_skip_consumes_scalars_and_subtrees() {
        // Each value is skipped starting from its first event; the decoder is
        // then positioned to see the array separators / end.
        let input = br#"[123,"str",{"a":[1,2],"b":{"c":3}},[[[]]],true,null]"#;
        let mut decoder = JsonDecoder::new();
        let mut pos = 0;
        assert_eq!(pull(&mut decoder, input, &mut pos), JsonEvent::BeginArray);
        let mut skipped = 0;
        loop {
            let event = pull(&mut decoder, input, &mut pos);
            match event {
                JsonEvent::EndArray => break,
                other => {
                    assert!(other.is_value(), "expected a value, got {other:?}");
                    skip_value(&mut decoder, input, &mut pos, other);
                    skipped += 1;
                }
            }
        }
        assert_eq!(skipped, 6);
        assert!(matches!(decoder.finish(), JsonEvent::EndDocument));
    }

    #[test]
    fn value_skip_holds_across_every_byte_boundary() {
        let input = br#"{"skip":{"deep":[1,{"x":2}],"z":true},"keep":42}"#;
        for chunk in 1..=input.len() {
            let mut decoder = JsonDecoder::new();
            let mut consumed = 0usize;
            let mut revealed = 0usize;
            let mut skip = JsonValueSkip::new();
            let mut phase = 0; // 0=await root, 1=await "skip" key, 2=skipping, 3=await "keep" number
            let mut kept: Option<Vec<u8>> = None;
            while revealed < input.len() {
                revealed = (revealed + chunk).min(input.len());
                loop {
                    let (n, event) = decoder.next(&input[consumed..revealed]);
                    consumed += n;
                    match (phase, event) {
                        (_, JsonEvent::NeedMore) => break,
                        (_, JsonEvent::Error(e, off)) => panic!("chunk {chunk}: {e:?}@{off}"),
                        (0, JsonEvent::BeginObject) => phase = 1,
                        (1, JsonEvent::Key) => {
                            assert_eq!(decoder.token(), b"skip");
                            phase = 2;
                        }
                        (2, ev) => {
                            if skip.feed(ev) == SkipProgress::Done {
                                phase = 3;
                            }
                        }
                        (3, JsonEvent::Key) => assert_eq!(decoder.token(), b"keep"),
                        (3, JsonEvent::Number) => kept = Some(decoder.token().to_vec()),
                        _ => {}
                    }
                }
            }
            assert!(
                matches!(decoder.finish(), JsonEvent::EndDocument),
                "chunk {chunk}"
            );
            assert_eq!(kept.as_deref(), Some(&b"42"[..]), "chunk {chunk}");
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    enum Selected {
        Ok(Vec<u8>),
        WrongType,
        Missing,
        Duplicate,
    }

    /// Select a top-level object field of an expected kind, distinguishing
    /// missing, duplicate, and wrong-type outcomes. Scalar values return their
    /// token bytes; container/bool/null values return empty bytes.
    fn select_top_field(input: &[u8], target: &[u8], expect: JsonValueKind) -> Selected {
        let mut decoder = JsonDecoder::new();
        let mut pos = 0;
        assert_eq!(pull(&mut decoder, input, &mut pos), JsonEvent::BeginObject);
        let mut matches = 0u32;
        let mut result = Selected::Missing;
        loop {
            match pull(&mut decoder, input, &mut pos) {
                JsonEvent::EndObject => break,
                JsonEvent::Key => {
                    let is_target = decoder.token() == target;
                    let value = pull(&mut decoder, input, &mut pos);
                    if !is_target {
                        skip_value(&mut decoder, input, &mut pos, value);
                        continue;
                    }
                    matches += 1;
                    if matches >= 2 {
                        result = Selected::Duplicate;
                    } else if value.value_kind() == Some(expect) {
                        let bytes = match value {
                            JsonEvent::Str | JsonEvent::Number => decoder.token().to_vec(),
                            _ => Vec::new(),
                        };
                        result = Selected::Ok(bytes);
                    } else if !matches!(result, Selected::Duplicate) {
                        result = Selected::WrongType;
                    }
                    skip_value(&mut decoder, input, &mut pos, value);
                }
                JsonEvent::Error(e, off) => panic!("select error {e:?}@{off}"),
                _ => {}
            }
        }
        result
    }

    #[test]
    fn named_field_selection_distinguishes_missing_duplicate_wrong_type() {
        let doc = br#"{"name":"Kyoto","temp":24,"tags":["a","b"],"name":"Osaka","ok":true}"#;

        assert_eq!(
            select_top_field(doc, b"temp", JsonValueKind::Number),
            Selected::Ok(b"24".to_vec())
        );
        assert_eq!(
            select_top_field(doc, b"ok", JsonValueKind::Bool),
            Selected::Ok(Vec::new())
        );
        assert_eq!(
            select_top_field(doc, b"tags", JsonValueKind::Array),
            Selected::Ok(Vec::new())
        );
        // "temp" is a number, not a string -> wrong type.
        assert_eq!(
            select_top_field(doc, b"temp", JsonValueKind::String),
            Selected::WrongType
        );
        // "name" appears twice -> duplicate.
        assert_eq!(
            select_top_field(doc, b"name", JsonValueKind::String),
            Selected::Duplicate
        );
        // absent key -> missing.
        assert_eq!(
            select_top_field(doc, b"humidity", JsonValueKind::Number),
            Selected::Missing
        );
    }

    #[test]
    fn parser_state_size_is_bounded() {
        // The parser state is fixed at 320 bytes on the host layout (see the
        // KOTO-0246 progress note); this guards against unbounded growth.
        assert!(core::mem::size_of::<JsonDecoder>() <= 512);
    }

    #[test]
    fn abi_event_codes_are_frozen() {
        // These values are the Runtime ABI (host ABI minor 20); changing any of
        // them breaks every compiled app that branches on JSON_* constants.
        let frozen: [(JsonEvent, i32); 13] = [
            (JsonEvent::NeedMore, 0),
            (JsonEvent::BeginObject, 1),
            (JsonEvent::EndObject, 2),
            (JsonEvent::BeginArray, 3),
            (JsonEvent::EndArray, 4),
            (JsonEvent::Key, 5),
            (JsonEvent::Str, 6),
            (JsonEvent::Number, 7),
            (JsonEvent::Bool(false), 8),
            (JsonEvent::Bool(true), 9),
            (JsonEvent::Null, 10),
            (JsonEvent::EndDocument, 11),
            (JsonEvent::Error(JsonError::UnexpectedByte, 0), 12),
        ];
        for (event, code) in frozen {
            assert_eq!(event.code(), code, "{event:?}");
        }
    }

    #[test]
    fn host_session_walks_a_document_via_abi_codes() {
        let doc = br#"{"temp":-3.5,"ok":true}"#;
        let mut session = JsonHostSession::new();
        let mut pos = 0usize;
        let mut codes = Vec::new();
        let mut tokens = Vec::new();
        loop {
            let code = session.next(&doc[pos..]);
            let (consumed, _) = session.status();
            pos += consumed as usize;
            if code == event_code::NEED_MORE {
                break;
            }
            codes.push(code);
            if matches!(code, event_code::KEY | event_code::STR | event_code::NUMBER) {
                tokens.push(session.token().to_vec());
            }
            if code == event_code::END_DOCUMENT || code == event_code::ERROR {
                break;
            }
        }
        // The document ends without trailing whitespace, so EndDocument is
        // reported by finish() after the input runs dry.
        assert_eq!(session.finish(), event_code::END_DOCUMENT);
        assert_eq!(
            codes,
            [
                event_code::BEGIN_OBJECT,
                event_code::KEY,
                event_code::NUMBER,
                event_code::KEY,
                event_code::TRUE,
                event_code::END_OBJECT,
            ]
        );
        assert_eq!(tokens, [b"temp".to_vec(), b"-3.5".to_vec(), b"ok".to_vec()]);
        assert_eq!(session.error(), (0, 0));
        assert_eq!(session.status(), (0, 0));
    }

    #[test]
    fn host_session_surfaces_sticky_error_and_reset() {
        let mut session = JsonHostSession::new();
        assert_eq!(session.next(b"{,"), event_code::BEGIN_OBJECT);
        let (consumed, _) = session.status();
        assert_eq!(consumed, 1);
        assert_eq!(session.next(&b"{,"[consumed as usize..]), event_code::ERROR);
        let (code, offset) = session.error();
        assert_eq!(code, i32::from(JsonError::UnexpectedByte.code()));
        assert_eq!(offset, 1);
        // Sticky until reset, including through finish().
        assert_eq!(session.next(b"1"), event_code::ERROR);
        assert_eq!(session.finish(), event_code::ERROR);
        session.reset();
        assert_eq!(session.error(), (0, 0));
        assert_eq!(session.next(b"7 "), event_code::NUMBER);
        assert_eq!(session.finish(), event_code::END_DOCUMENT);
    }

    #[test]
    fn host_session_reports_depth_for_skip_patterns() {
        let mut session = JsonHostSession::new();
        let doc = br#"{"a":[1,2]}"#;
        let mut pos = 0usize;
        let mut max_depth = 0;
        loop {
            let code = session.next(&doc[pos..]);
            let (consumed, depth) = session.status();
            pos += consumed as usize;
            max_depth = max_depth.max(depth);
            if code == event_code::NEED_MORE || code == event_code::END_DOCUMENT {
                break;
            }
        }
        assert_eq!(max_depth, 2);
        let (_, depth) = session.status();
        assert_eq!(depth, 0);
    }
}
