// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Lexer for Lua 5.4 source text.
//!
//! hyprtilt never executes Lua. To find `hl.monitor` calls, top-level
//! `return` statements and the managed block, it only needs to know where
//! tokens start and end, so that strings and comments are never mistaken for
//! code. The lexer covers the complete lexical grammar of Lua 5.4 (section
//! 3.1 of the reference manual): names, keywords, all symbols, short and long
//! strings with every escape sequence, decimal and hexadecimal numbers, short
//! and long comments.
//!
//! Every token carries its byte span in the source, so callers can copy the
//! original text verbatim.

use std::fmt;
use std::ops::Range;

/// Lua reserved words.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Lua symbols, longest first so that the lexer matches greedily.
const SYMBOLS: &[&str] = &[
    "...", "..", "==", "~=", "<=", ">=", "<<", ">>", "//", "::", "+", "-", "*", "/", "%", "^", "#",
    "&", "~", "|", "<", ">", "=", "(", ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

/// The kind of a token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier that is not a keyword.
    Name,
    /// A reserved word, such as `return` or `function`.
    Keyword,
    /// A short or long string literal, with its decoded bytes.
    Str(Vec<u8>),
    /// A numeric literal.
    Number(Number),
    /// An operator or punctuation symbol.
    Symbol,
    /// A short (`-- ...`) or long (`--[[ ... ]]`) comment.
    Comment,
}

/// The value of a numeric literal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    /// An integer literal (Lua subtype integer).
    Int(i64),
    /// A float literal (Lua subtype float).
    Float(f64),
}

impl Number {
    /// The value as a float, as Lua converts it for arithmetic.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        match self {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }
    }
}

/// A token with its byte span in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// Byte range of the token in the source text.
    pub span: Range<usize>,
}

impl Token {
    /// The source text of the token.
    #[must_use]
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.clone()]
    }

    /// Whether the token is the keyword or symbol `s`.
    #[must_use]
    pub fn is(&self, src: &str, s: &str) -> bool {
        matches!(self.kind, TokenKind::Keyword | TokenKind::Symbol) && self.text(src) == s
    }

    /// Whether the token is the name `s`.
    #[must_use]
    pub fn is_name(&self, src: &str, s: &str) -> bool {
        self.kind == TokenKind::Name && self.text(src) == s
    }
}

/// A lexical error, with the byte offset where it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    /// Byte offset of the error in the source.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for LexError {}

/// Split Lua source text into tokens, including comments.
///
/// Whitespace is skipped; its extent follows from the spans of the tokens
/// around it. A leading `#!` line (shebang), which the Lua interpreter
/// skips, is skipped too.
///
/// # Errors
///
/// Returns a [`LexError`] for unterminated strings or comments, invalid
/// escape sequences, malformed numbers and characters that cannot start a
/// token.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::lua::lexer::{tokenize, TokenKind};
///
/// let src = r#"hl.monitor({ output = "DP-1" }) -- right"#;
/// let tokens = tokenize(src).unwrap();
/// assert_eq!(tokens[0].text(src), "hl");
/// assert_eq!(tokens.last().unwrap().kind, TokenKind::Comment);
/// ```
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        src: src.as_bytes(),
        pos: 0,
    };
    if src.starts_with("#!") {
        lexer.pos = src.find('\n').unwrap_or(src.len());
    }
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token()? {
        tokens.push(token);
    }
    Ok(tokens)
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.pos + ahead).copied()
    }

    fn error<T>(offset: usize, message: impl Into<String>) -> Result<T, LexError> {
        Err(LexError {
            offset,
            message: message.into(),
        })
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek(0) {
            if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_whitespace();
        let start = self.pos;
        let Some(c) = self.peek(0) else {
            return Ok(None);
        };
        let kind = match c {
            b'-' if self.peek(1) == Some(b'-') => self.comment()?,
            b'"' | b'\'' => self.short_string()?,
            b'[' if self.long_bracket_level().is_some() => {
                let level = self.long_bracket_level().unwrap_or(0);
                TokenKind::Str(self.long_bracket(level, "string")?)
            }
            b'0'..=b'9' => self.number()?,
            b'.' if self.peek(1).is_some_and(|d| d.is_ascii_digit()) => self.number()?,
            c if c == b'_' || c.is_ascii_alphabetic() => self.name(),
            _ => self.symbol()?,
        };
        Ok(Some(Token {
            kind,
            span: start..self.pos,
        }))
    }

    fn name(&mut self) -> TokenKind {
        let start = self.pos;
        while self
            .peek(0)
            .is_some_and(|c| c == b'_' || c.is_ascii_alphanumeric())
        {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        if KEYWORDS.iter().any(|k| k.as_bytes() == text) {
            TokenKind::Keyword
        } else {
            TokenKind::Name
        }
    }

    fn symbol(&mut self) -> Result<TokenKind, LexError> {
        let rest = &self.src[self.pos..];
        for s in SYMBOLS {
            if rest.starts_with(s.as_bytes()) {
                self.pos += s.len();
                return Ok(TokenKind::Symbol);
            }
        }
        let shown = std::str::from_utf8(&rest[..rest.len().min(4)])
            .ok()
            .and_then(|s| s.chars().next())
            .map_or_else(|| format!("byte 0x{:02x}", rest[0]), |ch| format!("{ch:?}"));
        Self::error(self.pos, format!("unexpected character {shown}"))
    }

    /// The level of a long bracket (`[[` is 0, `[==[` is 2) starting at the
    /// current position, if there is one.
    fn long_bracket_level(&self) -> Option<usize> {
        if self.peek(0) != Some(b'[') {
            return None;
        }
        let mut level = 0;
        while self.peek(1 + level) == Some(b'=') {
            level += 1;
        }
        (self.peek(1 + level) == Some(b'[')).then_some(level)
    }

    /// Consume a long bracket of the given level and return its content. A
    /// newline right after the opening bracket is not part of the content.
    fn long_bracket(&mut self, level: usize, what: &str) -> Result<Vec<u8>, LexError> {
        let start = self.pos;
        self.pos += level + 2;
        // Skip the first newline (any of \n, \r, \r\n, \n\r).
        match (self.peek(0), self.peek(1)) {
            (Some(b'\r'), Some(b'\n')) | (Some(b'\n'), Some(b'\r')) => self.pos += 2,
            (Some(b'\r' | b'\n'), _) => self.pos += 1,
            _ => {}
        }
        let content_start = self.pos;
        while self.pos < self.src.len() {
            if self.src[self.pos] == b']' {
                let eqs = self.src[self.pos + 1..]
                    .iter()
                    .take_while(|&&c| c == b'=')
                    .count();
                if eqs == level && self.src.get(self.pos + 1 + level) == Some(&b']') {
                    let content = self.src[content_start..self.pos].to_vec();
                    self.pos += level + 2;
                    return Ok(content);
                }
            }
            self.pos += 1;
        }
        Self::error(start, format!("unfinished long {what}"))
    }

    fn comment(&mut self) -> Result<TokenKind, LexError> {
        self.pos += 2;
        if let Some(level) = self.long_bracket_level() {
            self.long_bracket(level, "comment")?;
        } else {
            while self.peek(0).is_some_and(|c| c != b'\n' && c != b'\r') {
                self.pos += 1;
            }
        }
        Ok(TokenKind::Comment)
    }

    fn short_string(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let quote = self.src[self.pos];
        self.pos += 1;
        let mut value = Vec::new();
        loop {
            let Some(c) = self.peek(0) else {
                return Self::error(start, "unfinished string");
            };
            match c {
                b'\n' | b'\r' => return Self::error(start, "unfinished string"),
                b'\\' => self.escape(&mut value)?,
                c if c == quote => {
                    self.pos += 1;
                    return Ok(TokenKind::Str(value));
                }
                c => {
                    value.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn escape(&mut self, value: &mut Vec<u8>) -> Result<(), LexError> {
        let at = self.pos;
        self.pos += 1;
        let Some(c) = self.peek(0) else {
            return Self::error(at, "unfinished string");
        };
        self.pos += 1;
        match c {
            b'a' => value.push(0x07),
            b'b' => value.push(0x08),
            b'f' => value.push(0x0c),
            b'n' => value.push(b'\n'),
            b'r' => value.push(b'\r'),
            b't' => value.push(b'\t'),
            b'v' => value.push(0x0b),
            b'\\' | b'"' | b'\'' => value.push(c),
            b'\n' | b'\r' => {
                // An escaped line break is a newline; \r\n and \n\r count once.
                if matches!(self.peek(0), Some(n) if (n == b'\n' || n == b'\r') && n != c) {
                    self.pos += 1;
                }
                value.push(b'\n');
            }
            b'z' => self.skip_whitespace(),
            b'x' => {
                let hex = self.src.get(self.pos..self.pos + 2).unwrap_or_default();
                let byte = std::str::from_utf8(hex)
                    .ok()
                    .filter(|h| h.len() == 2)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                let Some(byte) = byte else {
                    return Self::error(at, "hexadecimal digit expected in \\x escape");
                };
                value.push(byte);
                self.pos += 2;
            }
            b'u' => self.utf8_escape(at, value)?,
            b'0'..=b'9' => {
                let mut n = u32::from(c - b'0');
                for _ in 0..2 {
                    match self.peek(0) {
                        Some(d @ b'0'..=b'9') => {
                            n = n * 10 + u32::from(d - b'0');
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                let Ok(byte) = u8::try_from(n) else {
                    return Self::error(at, "decimal escape too large");
                };
                value.push(byte);
            }
            _ => return Self::error(at, "invalid escape sequence"),
        }
        Ok(())
    }

    /// `\u{XXX}`: Lua 5.4 accepts code points up to 2^31 and encodes them
    /// with the original, up to six byte, UTF-8 scheme.
    fn utf8_escape(&mut self, at: usize, value: &mut Vec<u8>) -> Result<(), LexError> {
        if self.peek(0) != Some(b'{') {
            return Self::error(at, "missing '{' in \\u{xxxx}");
        }
        self.pos += 1;
        let digits_start = self.pos;
        while self.peek(0).is_some_and(|c| c.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits = std::str::from_utf8(&self.src[digits_start..self.pos]).unwrap_or_default();
        if digits.is_empty() || self.peek(0) != Some(b'}') {
            return Self::error(at, "malformed \\u{xxxx} escape");
        }
        self.pos += 1;
        let code = match u64::from_str_radix(digits, 16) {
            Ok(code) if code < (1 << 31) => code as u32,
            _ => return Self::error(at, "UTF-8 value too large"),
        };
        encode_utf8_lua(code, value);
        Ok(())
    }

    fn number(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let hex = self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'x' | b'X'));
        if hex {
            self.pos += 2;
        }
        let (exp_lower, exp_upper) = if hex { (b'p', b'P') } else { (b'e', b'E') };
        // Consume like Lua's read_numeral: digits, dots, exponent marks with
        // an optional sign, then validate the whole literal.
        loop {
            match self.peek(0) {
                Some(c) if c == exp_lower || c == exp_upper => {
                    self.pos += 1;
                    if matches!(self.peek(0), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                }
                Some(c) if c.is_ascii_hexdigit() || c == b'.' => self.pos += 1,
                _ => break,
            }
        }
        if self
            .peek(0)
            .is_some_and(|c| c == b'_' || c.is_ascii_alphanumeric())
        {
            return Self::error(start, "malformed number");
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or_default();
        match parse_number(text) {
            Some(n) => Ok(TokenKind::Number(n)),
            None => Self::error(start, format!("malformed number near '{text}'")),
        }
    }
}

/// Encode a code point the way Lua's `luaO_utf8esc` does (up to 6 bytes).
fn encode_utf8_lua(mut code: u32, out: &mut Vec<u8>) {
    if code < 0x80 {
        out.push(code as u8);
        return;
    }
    let mut buf = [0u8; 8];
    let mut n = 1;
    let mut mfb: u32 = 0x3f; // maximum that fits in the first byte
    loop {
        buf[8 - n] = 0x80 | (code & 0x3f) as u8;
        n += 1;
        code >>= 6;
        mfb >>= 1;
        if code <= mfb {
            break;
        }
    }
    buf[8 - n] = ((!mfb << 1) | code) as u8;
    out.extend_from_slice(&buf[8 - n..]);
}

/// Parse a numeric literal as Lua does: integers that overflow become
/// floats (decimal) or wrap around (hexadecimal).
fn parse_number(text: &str) -> Option<Number> {
    let lower = text.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return parse_hex(hex);
    }
    if lower.bytes().all(|c| c.is_ascii_digit()) {
        return Some(match lower.parse::<i64>() {
            Ok(i) => Number::Int(i),
            Err(_) => Number::Float(lower.parse::<f64>().ok()?),
        });
    }
    // Rust's float grammar is a superset of Lua's decimal floats except for
    // forms like "inf" or "nan", which cannot reach here (digits only).
    let valid = lower
        .bytes()
        .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'+' | b'-'));
    if !valid || lower.matches('.').count() > 1 {
        return None;
    }
    lower.parse::<f64>().ok().map(Number::Float)
}

fn parse_hex(hex: &str) -> Option<Number> {
    let (mantissa, exponent) = match hex.split_once('p') {
        Some((m, e)) => (m, Some(e)),
        None => (hex, None),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    if int_part.is_empty() && frac_part.is_none_or(str::is_empty) {
        return None;
    }
    let all_hex = |s: &str| s.bytes().all(|c| c.is_ascii_hexdigit());
    if !all_hex(int_part) || !frac_part.is_none_or(all_hex) {
        return None;
    }
    if frac_part.is_none() && exponent.is_none() {
        // Hexadecimal integers wrap around modulo 2^64.
        let mut value: u64 = 0;
        for c in int_part.bytes() {
            let digit = u64::from((c as char).to_digit(16)?);
            value = value.wrapping_mul(16).wrapping_add(digit);
        }
        return Some(Number::Int(value as i64));
    }
    let mut value = 0f64;
    for c in int_part.bytes() {
        value = value * 16.0 + f64::from((c as char).to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.unwrap_or_default().bytes() {
        value += f64::from((c as char).to_digit(16)?) * scale;
        scale /= 16.0;
    }
    if let Some(e) = exponent {
        let e: i32 = e.parse().ok()?;
        value *= 2f64.powi(e);
    }
    Some(Number::Float(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(TokenKind, &str)> {
        tokenize(src)
            .unwrap()
            .into_iter()
            .map(|t| {
                let text = t.text(src);
                (t.kind, text)
            })
            .collect()
    }

    fn string_value(src: &str) -> Vec<u8> {
        match tokenize(src).unwrap().remove(0).kind {
            TokenKind::Str(v) => v,
            other => panic!("not a string: {other:?}"),
        }
    }

    fn number_value(src: &str) -> Number {
        match tokenize(src).unwrap().remove(0).kind {
            TokenKind::Number(n) => n,
            other => panic!("not a number: {other:?}"),
        }
    }

    #[test]
    fn monitor_call_tokens() {
        let src = r#"hl.monitor({ output = "HDMI-A-1", scale = 1.5, transform = 1 })"#;
        let texts: Vec<_> = kinds(src).into_iter().map(|(_, t)| t).collect();
        assert_eq!(
            texts,
            [
                "hl",
                ".",
                "monitor",
                "(",
                "{",
                "output",
                "=",
                "\"HDMI-A-1\"",
                ",",
                "scale",
                "=",
                "1.5",
                ",",
                "transform",
                "=",
                "1",
                "}",
                ")"
            ]
        );
    }

    #[test]
    fn keywords_and_names() {
        let k = kinds("return returned local _x x1");
        assert_eq!(k[0].0, TokenKind::Keyword);
        assert_eq!(k[1].0, TokenKind::Name);
        assert_eq!(k[2].0, TokenKind::Keyword);
        assert_eq!(k[3].0, TokenKind::Name);
        assert_eq!(k[4].0, TokenKind::Name);
    }

    #[test]
    fn symbols_are_greedy() {
        let texts: Vec<_> = kinds("a...b..c==d~=e<=f>=g<<h>>i//j::k")
            .into_iter()
            .filter(|(k, _)| *k == TokenKind::Symbol)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(
            texts,
            ["...", "..", "==", "~=", "<=", ">=", "<<", ">>", "//", "::"]
        );
    }

    #[test]
    fn short_and_long_comments() {
        let src = "a -- short\n--[[ long\n comment ]] b --[==[ x ]] ]==] c";
        let k = kinds(src);
        assert_eq!(k[1], (TokenKind::Comment, "-- short"));
        assert_eq!(k[2], (TokenKind::Comment, "--[[ long\n comment ]]"));
        assert_eq!(k[4], (TokenKind::Comment, "--[==[ x ]] ]==]"));
        assert_eq!(k[5].1, "c");
    }

    #[test]
    fn comment_that_looks_like_long_bracket_but_is_not() {
        // "--[=" without a second "[" is a short comment.
        let k = kinds("--[= not long\nx");
        assert_eq!(k[0], (TokenKind::Comment, "--[= not long"));
        assert_eq!(k[1].1, "x");
    }

    #[test]
    fn code_inside_strings_and_comments_is_not_code() {
        let src = r#"s = "hl.monitor({})" -- hl.monitor({})
t = [[ return x ]]"#;
        let names: Vec<_> = kinds(src)
            .into_iter()
            .filter(|(k, _)| matches!(k, TokenKind::Name | TokenKind::Keyword))
            .map(|(_, t)| t)
            .collect();
        assert_eq!(names, ["s", "t"]);
    }

    #[test]
    fn string_escapes() {
        assert_eq!(string_value(r#""a\tb\n\\\"\'""#), b"a\tb\n\\\"'");
        assert_eq!(string_value(r#""\65\066\0671""#), b"ABC1");
        assert_eq!(string_value(r#""\x41\x6a""#), b"Aj");
        assert_eq!(string_value(r#""\u{48}\u{20AC}""#), "H\u{20ac}".as_bytes());
        assert_eq!(string_value("\"a\\z   \n  b\""), b"ab");
        assert_eq!(string_value("'line\\\nnext'"), b"line\nnext");
        assert_eq!(string_value("'crlf\\\r\nnext'"), b"crlf\nnext");
    }

    #[test]
    fn utf8_escape_beyond_unicode_uses_lua_encoding() {
        // Lua encodes 0x7FFFFFFF as six bytes.
        assert_eq!(
            string_value(r#""\u{7FFFFFFF}""#),
            [0xfd, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf]
        );
    }

    #[test]
    fn long_strings_skip_first_newline() {
        assert_eq!(string_value("[[\nabc]]"), b"abc");
        assert_eq!(string_value("[==[\r\na]]b]==]"), b"a]]b");
        assert_eq!(string_value("[[x\\n]]"), b"x\\n");
    }

    #[test]
    fn numbers() {
        assert_eq!(number_value("144"), Number::Int(144));
        assert_eq!(number_value("179.95"), Number::Float(179.95));
        assert_eq!(number_value(".5"), Number::Float(0.5));
        assert_eq!(number_value("3."), Number::Float(3.0));
        assert_eq!(number_value("1e3"), Number::Float(1000.0));
        assert_eq!(number_value("2E-2"), Number::Float(0.02));
        assert_eq!(number_value("0xff"), Number::Int(255));
        assert_eq!(number_value("0x10p1"), Number::Float(32.0));
        assert_eq!(number_value("0x.8"), Number::Float(0.5));
        assert_eq!(number_value("0xA.8p0"), Number::Float(10.5));
        // Decimal overflow becomes a float, hexadecimal wraps around.
        assert_eq!(
            number_value("9223372036854775808"),
            Number::Float(9_223_372_036_854_775_808.0)
        );
        assert_eq!(number_value("0xffffffffffffffff"), Number::Int(-1));
    }

    #[test]
    fn negative_numbers_are_minus_then_number() {
        let k = kinds("-5");
        assert_eq!(k[0], (TokenKind::Symbol, "-"));
        assert_eq!(k[1].0, TokenKind::Number(Number::Int(5)));
    }

    #[test]
    fn malformed_numbers() {
        assert!(tokenize("3x").is_err());
        assert!(tokenize("1..2").is_err());
        assert!(tokenize("0x").is_err());
        assert!(tokenize("1e").is_err());
        assert!(tokenize("0xg").is_err());
    }

    #[test]
    fn errors_report_offsets() {
        let err = tokenize("x = \"open").unwrap_err();
        assert_eq!(err.offset, 4);
        assert_eq!(err.message, "unfinished string");
        assert!(tokenize("'a\nb'").is_err());
        assert!(tokenize("--[[ never closed").is_err());
        assert!(tokenize("[==[ wrong ]=]").is_err());
        assert!(tokenize(r#""\q""#).is_err());
        assert!(tokenize(r#""\256""#).is_err());
        assert!(tokenize(r#""\x4""#).is_err());
        assert!(tokenize(r#""\u{}""#).is_err());
        assert!(tokenize(r#""\u{80000000}""#).is_err());
        assert!(tokenize(r#""\u12""#).is_err());
        let err = tokenize("a = $").unwrap_err();
        assert_eq!(err.to_string(), "unexpected character '$' at byte 4");
    }

    #[test]
    fn shebang_is_skipped() {
        let k = kinds("#!/usr/bin/lua\nx");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].1, "x");
    }

    #[test]
    fn spans_cover_original_text() {
        let src = "local t = { [\"k\"] = 0x1F, } -- c\n";
        for t in tokenize(src).unwrap() {
            assert_eq!(&src[t.span.clone()], t.text(src));
            assert!(!t.text(src).trim().is_empty());
        }
    }

    #[test]
    fn number_as_f64() {
        assert!((Number::Int(2).as_f64() - 2.0).abs() < f64::EPSILON);
        assert!((Number::Float(1.25).as_f64() - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn token_predicates() {
        let src = "return hl";
        let t = tokenize(src).unwrap();
        assert!(t[0].is(src, "return"));
        assert!(!t[0].is_name(src, "return"));
        assert!(t[1].is_name(src, "hl"));
        assert!(!t[1].is(src, "hl"));
    }
}
