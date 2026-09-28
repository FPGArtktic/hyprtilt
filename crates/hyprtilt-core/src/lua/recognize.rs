// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Recognising `hl.monitor` calls and the statement structure of a Lua file
//! without executing it.
//!
//! On top of the token stream this module tracks two depths: the block
//! depth (`function`, `if`, `do` and `repeat` open a block; `end` and
//! `until` close one; `while` and `for` open through their `do`) and the
//! bracket depth. Together with the tokens around a position they decide
//! whether a line start is a statement boundary at the top level, which is
//! where a managed block may be inserted.
//!
//! A monitor rule is recognised only as `hl.monitor(<table>)` or
//! `hl.monitor<table>` whose table holds literals: strings, numbers with an
//! optional minus sign, booleans, `nil`, and nested tables of literals.
//! The values are checked against the types Hyprland 0.56 accepts
//! (`docs/hyprland-lua-api.md`, sections 1.2 and 1.3).

use std::ops::Range;

use crate::block;
use crate::document::ConfigError;
use crate::lua::lexer::{self, Number, Token, TokenKind};
use std::ops::RangeInclusive;

use crate::model::{ColorManagement, ExtraField, MonitorRule, Reserved, Scale, Transform};

/// Symbols after which an expression goes on, so a call before them is not
/// a statement of its own.
const CONTINUATIONS: &[&str] = &[
    ".", ":", "(", "[", "{", "..", "+", "-", "*", "/", "//", "%", "^", "==", "~=", "<", "<=", ">",
    ">=", "&", "|", "~", "<<", ">>", "=", ",",
];

/// A tokenised Lua file with depths.
pub(crate) struct Source<'a> {
    pub(crate) src: &'a str,
    /// Every token, comments included.
    pub(crate) tokens: Vec<Token>,
    /// Indices into `tokens` of the tokens that are not comments.
    code: Vec<usize>,
    /// Block depth before each code token; one extra entry for the end.
    block_depth: Vec<i32>,
    /// Bracket depth before each code token; one extra entry for the end.
    bracket_depth: Vec<i32>,
}

/// A recognised `hl.monitor` call.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    /// Bytes from `hl` to the closing bracket, plus a following `;`.
    pub(crate) span: Range<usize>,
    /// Block depth of the call; 0 is the top level.
    pub(crate) depth: i32,
    /// Whether the call is a statement of its own rather than part of an
    /// expression.
    pub(crate) statement: bool,
    /// The rule, or why the argument is not a literal rule.
    pub(crate) rule: Result<MonitorRule, String>,
}

impl<'a> Source<'a> {
    /// Tokenise `src` and compute the depths.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Syntax`] for a lexical error.
    pub(crate) fn new(src: &'a str) -> Result<Self, ConfigError> {
        let tokens = lexer::tokenize(src).map_err(|e| ConfigError::Syntax {
            line: block::line_of(src, e.offset),
            message: e.message,
        })?;
        let code: Vec<usize> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind != TokenKind::Comment)
            .map(|(i, _)| i)
            .collect();
        let mut block_depth = Vec::with_capacity(code.len() + 1);
        let mut bracket_depth = Vec::with_capacity(code.len() + 1);
        let (mut blocks, mut brackets) = (0i32, 0i32);
        for &i in &code {
            block_depth.push(blocks);
            bracket_depth.push(brackets);
            let t = &tokens[i];
            let text = t.text(src);
            match t.kind {
                TokenKind::Keyword => match text {
                    "function" | "if" | "do" | "repeat" => blocks += 1,
                    "end" | "until" => blocks -= 1,
                    _ => {}
                },
                TokenKind::Symbol => match text {
                    "(" | "{" | "[" => brackets += 1,
                    ")" | "}" | "]" => brackets -= 1,
                    _ => {}
                },
                _ => {}
            }
        }
        block_depth.push(blocks);
        bracket_depth.push(brackets);
        Ok(Source {
            src,
            tokens,
            code,
            block_depth,
            bracket_depth,
        })
    }

    fn tok(&self, k: usize) -> Option<&Token> {
        self.code.get(k).map(|&i| &self.tokens[i])
    }

    fn is(&self, k: usize, s: &str) -> bool {
        self.tok(k).is_some_and(|t| t.is(self.src, s))
    }

    /// Index of the code token closing the bracket opened at `open`.
    fn matching(&self, open: usize) -> Option<usize> {
        let base = self.bracket_depth[open];
        (open + 1..self.code.len())
            .find(|&k| self.bracket_depth[k + 1] == base && self.bracket_depth[k] == base + 1)
    }

    /// The comment tokens that start inside `range`.
    pub(crate) fn comments_in(&self, range: &Range<usize>) -> Vec<Range<usize>> {
        self.tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Comment && range.contains(&t.span.start))
            .map(|t| t.span.clone())
            .collect()
    }

    /// The code tokens that start inside `range`, as byte ranges.
    pub(crate) fn code_in(&self, range: &Range<usize>) -> Vec<Range<usize>> {
        self.code
            .iter()
            .map(|&i| &self.tokens[i])
            .filter(|t| range.contains(&t.span.start))
            .map(|t| t.span.clone())
            .collect()
    }

    /// Whether a token (string, comment, ...) covers `offset` from both
    /// sides, so that inserting text there would change that token.
    fn inside_token(&self, offset: usize) -> bool {
        self.tokens
            .iter()
            .any(|t| t.span.start < offset && offset < t.span.end)
    }

    /// Number of code tokens that end at or before `offset`.
    fn code_before(&self, offset: usize) -> usize {
        self.code
            .iter()
            .take_while(|&&i| self.tokens[i].span.end <= offset)
            .count()
    }

    /// Whether `offset`, a line start, is a boundary between two top-level
    /// statements, where a new statement can be inserted without changing
    /// the meaning of the code around it.
    pub(crate) fn is_top_level_boundary(&self, offset: usize) -> bool {
        if self.inside_token(offset) {
            return false;
        }
        let k = self.code_before(offset);
        if self.block_depth[k] != 0 || self.bracket_depth[k] != 0 {
            return false;
        }
        let prev_ok = k == 0 || self.tok(k - 1).is_some_and(|t| self.can_end_statement(t));
        let next_ok = self.tok(k).is_none_or(|t| self.can_start_statement(t));
        prev_ok && next_ok
    }

    fn can_end_statement(&self, t: &Token) -> bool {
        match t.kind {
            TokenKind::Name | TokenKind::Str(_) | TokenKind::Number(_) => true,
            // A statement may also start right after the keywords that open
            // a block body.
            TokenKind::Keyword => matches!(
                t.text(self.src),
                "end" | "true" | "false" | "nil" | "break" | "then" | "do" | "else" | "repeat"
            ),
            TokenKind::Symbol => matches!(t.text(self.src), ")" | "]" | "}" | "..." | ";" | "::"),
            TokenKind::Comment => false,
        }
    }

    fn can_start_statement(&self, t: &Token) -> bool {
        match t.kind {
            TokenKind::Name => true,
            TokenKind::Keyword => matches!(
                t.text(self.src),
                "local"
                    | "function"
                    | "if"
                    | "while"
                    | "for"
                    | "repeat"
                    | "do"
                    | "return"
                    | "break"
                    | "goto"
            ),
            TokenKind::Symbol => matches!(t.text(self.src), ";" | "::"),
            _ => false,
        }
    }

    /// Byte offsets of the `return` keywords at the top level.
    pub(crate) fn top_level_returns(&self) -> Vec<usize> {
        (0..self.code.len())
            .filter(|&k| self.block_depth[k] == 0 && self.is(k, "return"))
            .filter_map(|k| self.tok(k).map(|t| t.span.start))
            .collect()
    }

    /// Every `hl.monitor` call in the file.
    pub(crate) fn calls(&self) -> Vec<Call> {
        (0..self.code.len())
            .filter_map(|k| self.call_at(k))
            .collect()
    }

    fn call_at(&self, k: usize) -> Option<Call> {
        let is_hl = self.tok(k).is_some_and(|t| t.is_name(self.src, "hl"));
        let member = k > 0 && (self.is(k - 1, ".") || self.is(k - 1, ":"));
        if !is_hl || member || !self.is(k + 1, ".") {
            return None;
        }
        if !self
            .tok(k + 2)
            .is_some_and(|t| t.is_name(self.src, "monitor"))
        {
            return None;
        }
        let open = k + 3;
        let (last, rule) = if self.is(open, "(") {
            let close = self.matching(open)?;
            let single_table = self.is(open + 1, "{") && self.matching(open + 1) == Some(close - 1);
            let rule = if single_table {
                self.table_rule(open + 1, close - 1)
            } else {
                Err("the argument is not a table literal".to_owned())
            };
            (close, rule)
        } else if self.is(open, "{") {
            let close = self.matching(open)?;
            (close, self.table_rule(open, close))
        } else if matches!(self.tok(open).map(|t| &t.kind), Some(TokenKind::Str(_))) {
            (open, Err("the argument must be a table".to_owned()))
        } else {
            return None;
        };
        let start = self.tok(k)?.span.start;
        let mut end_token = last;
        if self.is(last + 1, ";") {
            end_token = last + 1;
        }
        let end = self.tok(end_token)?.span.end;
        let prev_ok = k == 0 || self.tok(k - 1).is_some_and(|t| self.can_end_statement(t));
        let next_ok = self
            .tok(last + 1)
            .is_none_or(|t| !self.continues_expression(t));
        Some(Call {
            span: start..end,
            depth: self.block_depth[k],
            statement: prev_ok && next_ok,
            rule,
        })
    }

    fn continues_expression(&self, t: &Token) -> bool {
        match t.kind {
            TokenKind::Str(_) => true,
            TokenKind::Symbol => CONTINUATIONS.contains(&t.text(self.src)),
            TokenKind::Keyword => matches!(t.text(self.src), "and" | "or"),
            _ => false,
        }
    }

    /// Source text of the code tokens `from..=to`.
    fn text_of(&self, from: usize, to: usize) -> &'a str {
        match (self.tok(from), self.tok(to)) {
            (Some(a), Some(b)) if a.span.start <= b.span.end => &self.src[a.span.start..b.span.end],
            _ => "",
        }
    }

    fn table_rule(&self, open: usize, close: usize) -> Result<MonitorRule, String> {
        let fields = self.table(open, close, false)?;
        rule_from_fields(fields)
    }

    /// Parse the fields of the table constructor `open..=close`. Positional
    /// values are allowed only in nested tables, where they get their index
    /// as the key.
    fn table(&self, open: usize, close: usize, positional: bool) -> Result<Vec<Field>, String> {
        let mut fields = Vec::new();
        let mut k = open + 1;
        while k < close {
            let (key, value_at) =
                if self.tok(k).is_some_and(|t| t.kind == TokenKind::Name) && self.is(k + 1, "=") {
                    (self.text_of(k, k).to_owned(), k + 2)
                } else if self.is(k, "[") {
                    let end = self
                        .matching(k)
                        .filter(|&e| e < close)
                        .ok_or("unbalanced brackets")?;
                    let key = match self.tok(k + 1).map(|t| &t.kind) {
                        Some(TokenKind::Str(bytes)) if end == k + 2 => {
                            String::from_utf8_lossy(bytes).into_owned()
                        }
                        _ => {
                            return Err(format!(
                                "the key `{}` is not a string literal",
                                self.text_of(k, end)
                            ));
                        }
                    };
                    if !self.is(end + 1, "=") {
                        return Err(format!("expected `=` after `{}`", self.text_of(k, end)));
                    }
                    (key, end + 2)
                } else if positional {
                    ((fields.len() + 1).to_string(), k)
                } else {
                    return Err(format!(
                        "positional value `{}` is not supported",
                        self.text_of(k, self.expression_end(k, close))
                    ));
                };
            let (value, next) = self
                .field_value(value_at, close)
                .map_err(|e| format!("{key}: {e}"))?;
            fields.push(Field {
                key,
                raw: self.text_of(value_at, next - 1).to_owned(),
                value,
            });
            k = next;
            if self.is(k, ",") || self.is(k, ";") {
                k += 1;
            }
        }
        Ok(fields)
    }

    /// A literal value followed by a field separator or the end of the
    /// table, so that `1 + x` is not mistaken for the literal `1`.
    fn field_value(&self, k: usize, close: usize) -> Result<(Value, usize), String> {
        let (value, next) = self.value(k, close)?;
        if next == close || self.is(next, ",") || self.is(next, ";") {
            Ok((value, next))
        } else {
            Err(format!(
                "`{}` is not a literal",
                self.text_of(k, self.expression_end(k, close))
            ))
        }
    }

    /// The last code token of the expression starting at `k` inside a table
    /// that closes at `close`.
    fn expression_end(&self, k: usize, close: usize) -> usize {
        let base = self.bracket_depth[k];
        let mut e = k;
        while e + 1 < close {
            let next = e + 1;
            if self.bracket_depth[next] == base && (self.is(next, ",") || self.is(next, ";")) {
                break;
            }
            e = next;
        }
        e
    }

    /// Parse a literal value starting at code token `k`; returns the value
    /// and the index after it.
    fn value(&self, k: usize, close: usize) -> Result<(Value, usize), String> {
        let t = self.tok(k).filter(|_| k < close).ok_or("missing value")?;
        let not_literal = || {
            format!(
                "`{}` is not a literal",
                self.text_of(k, self.expression_end(k, close))
            )
        };
        let value = match &t.kind {
            TokenKind::Str(bytes) => Value::Str(String::from_utf8_lossy(bytes).into_owned()),
            TokenKind::Number(n) => Value::Num(*n),
            TokenKind::Keyword => match t.text(self.src) {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "nil" => Value::Nil,
                _ => return Err(not_literal()),
            },
            TokenKind::Symbol if t.is(self.src, "-") => match self.tok(k + 1).map(|n| &n.kind) {
                Some(TokenKind::Number(n)) if k + 1 < close => {
                    let negated = match *n {
                        Number::Int(i) => Number::Int(i.wrapping_neg()),
                        Number::Float(f) => Number::Float(-f),
                    };
                    return Ok((Value::Num(negated), k + 2));
                }
                _ => return Err(not_literal()),
            },
            TokenKind::Symbol if t.is(self.src, "{") => {
                let end = self
                    .matching(k)
                    .filter(|&e| e < close)
                    .ok_or("unbalanced braces")?;
                return Ok((Value::Table(self.table(k, end, true)?), end + 1));
            }
            _ => return Err(not_literal()),
        };
        Ok((value, k + 1))
    }
}

/// A literal value in a table.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Num(Number),
    Bool(bool),
    Nil,
    Table(Vec<Field>),
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Str(_) => "a string",
            Value::Num(Number::Int(_)) => "an integer",
            Value::Num(Number::Float(_)) => "a float",
            Value::Bool(_) => "a boolean",
            Value::Nil => "nil",
            Value::Table(_) => "a table",
        }
    }
}

/// A `key = value` field of a table.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    key: String,
    value: Value,
    /// The value's source text.
    raw: String,
}

/// Build a rule from the fields of an `hl.monitor` table, accepting the
/// types Hyprland accepts and refusing values it would reject or
/// silently replace.
fn rule_from_fields(fields: Vec<Field>) -> Result<MonitorRule, String> {
    let mut seen: Vec<&str> = Vec::new();
    for f in &fields {
        if seen.contains(&f.key.as_str()) {
            return Err(format!("the key `{}` appears twice", f.key));
        }
        seen.push(&f.key);
    }
    if seen.contains(&"reserved") && seen.contains(&"reserved_area") {
        return Err("both `reserved` and `reserved_area` are given".to_owned());
    }
    let output = fields
        .iter()
        .find(|f| f.key == "output")
        .ok_or("`output` is missing")?;
    let output = match &output.value {
        Value::Str(s) => s.clone(),
        Value::Num(Number::Int(i)) => i.to_string(),
        other => return Err(format!("`output` must be a string, not {}", other.kind())),
    };
    let mut rule = MonitorRule::new(output);
    for f in fields {
        if f.key == "output" || f.value == Value::Nil {
            continue;
        }
        apply_field(&mut rule, &f)?;
    }
    Ok(rule)
}

fn apply_field(rule: &mut MonitorRule, f: &Field) -> Result<(), String> {
    let key = f.key.as_str();
    let v = &f.value;
    let any = i64::MIN..=i64::MAX;
    match key {
        "mode" => rule.mode = Some(parse(&string(v, key)?)?),
        "position" => rule.position = Some(parse(&string(v, key)?)?),
        "scale" => rule.scale = Some(scale(v)?),
        "transform" => {
            let t = int_in(v, key, 0..=7)?;
            rule.transform = Some(Transform::new(t as u8).map_err(|e| e.to_string())?);
        }
        "disabled" => rule.disabled = Some(boolean(v, key)?),
        "vrr" => rule.vrr = Some(int_in(v, key, -1..=3)? as i8),
        "mirror" => rule.mirror = Some(string(v, key)?),
        "bitdepth" => rule.bitdepth = Some(int_in(v, key, any)?),
        "cm" => rule.cm = Some(parse::<ColorManagement>(&string(v, key)?)?),
        "sdr_eotf" => rule.sdr_eotf = Some(string(v, key)?),
        "sdrbrightness" => rule.sdrbrightness = Some(float(v, key)?),
        "sdrsaturation" => rule.sdrsaturation = Some(float(v, key)?),
        "sdr_min_luminance" => rule.sdr_min_luminance = Some(float(v, key)?),
        "min_luminance" => rule.min_luminance = Some(float(v, key)?),
        "icc" => {
            let s = string(v, key)?;
            if s.is_empty() {
                return Err("`icc` must not be empty".to_owned());
            }
            rule.icc = Some(s);
        }
        "supports_wide_color" => rule.supports_wide_color = Some(int_in(v, key, -1..=1)? as i8),
        "supports_hdr" => rule.supports_hdr = Some(int_in(v, key, -1..=1)? as i8),
        "sdr_max_luminance" => rule.sdr_max_luminance = Some(int_in(v, key, any)?),
        "max_luminance" => rule.max_luminance = Some(int_in(v, key, any)?),
        "max_avg_luminance" => rule.max_avg_luminance = Some(int_in(v, key, any)?),
        "reserved" | "reserved_area" => rule.reserved = Some(reserved(v, key)?),
        _ => rule.extra.push(ExtraField {
            key: f.key.clone(),
            raw: f.raw.clone(),
        }),
    }
    Ok(())
}

fn parse<T: std::str::FromStr<Err = crate::model::InvalidValue>>(s: &str) -> Result<T, String> {
    s.parse()
        .map_err(|e: crate::model::InvalidValue| e.to_string())
}

/// A string value; integers are converted as Lua's `tostring` does.
fn string(v: &Value, key: &str) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        Value::Num(Number::Int(i)) => Ok(i.to_string()),
        other => Err(format!("`{key}` must be a string, not {}", other.kind())),
    }
}

/// An integer or boolean in `range`. Floats are refused: Hyprland rejects
/// `1.0` for integer keys.
fn int_in(v: &Value, key: &str, range: RangeInclusive<i64>) -> Result<i64, String> {
    let i = match v {
        Value::Num(Number::Int(i)) => *i,
        Value::Bool(b) => i64::from(*b),
        other => return Err(format!("`{key}` must be an integer, not {}", other.kind())),
    };
    if range.contains(&i) {
        Ok(i)
    } else {
        Err(format!(
            "`{key}` must be from {} to {}, not {i}",
            range.start(),
            range.end()
        ))
    }
}

/// A boolean, or a number (or numeric string) equal to 0 or 1.
fn boolean(v: &Value, key: &str) -> Result<bool, String> {
    let number = match v {
        Value::Bool(b) => return Ok(*b),
        Value::Num(n) => Some(n.as_f64()),
        Value::Str(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    if number == Some(0.0) {
        Ok(false)
    } else if number == Some(1.0) {
        Ok(true)
    } else {
        Err(format!("`{key}` must be true or false, not {}", v.kind()))
    }
}

/// A number, boolean or numeric string.
fn float(v: &Value, key: &str) -> Result<f64, String> {
    let f = match v {
        Value::Num(n) => Some(n.as_f64()),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Str(s) => s.trim().parse().ok(),
        _ => None,
    };
    f.filter(|f: &f64| f.is_finite())
        .ok_or_else(|| format!("`{key}` must be a number, not {}", v.kind()))
}

fn scale(v: &Value) -> Result<Scale, String> {
    match v {
        Value::Str(s) => Scale::parse_hyprland(s).map_err(|e| e.to_string()),
        Value::Num(n) if n.as_f64() >= 0.25 => Ok(Scale::Factor(n.as_f64())),
        Value::Num(n) => Err(format!(
            "invalid scale: {} (the minimum is 0.25)",
            n.as_f64()
        )),
        other => Err(format!(
            "`scale` must be a number or \"auto\", not {}",
            other.kind()
        )),
    }
}

fn reserved(v: &Value, key: &str) -> Result<Reserved, String> {
    let side = |v: &Value, name: &str| match v {
        Value::Num(Number::Int(i)) => {
            i32::try_from(*i).map_err(|_| format!("`{name}` is out of range"))
        }
        other => Err(format!("`{name}` must be an integer, not {}", other.kind())),
    };
    match v {
        Value::Num(_) => {
            let all = side(v, key)?;
            Ok(Reserved {
                top: all,
                right: all,
                bottom: all,
                left: all,
            })
        }
        Value::Table(fields) => {
            let mut r = Reserved::default();
            for f in fields {
                let value = side(&f.value, &f.key)?;
                match f.key.as_str() {
                    "top" => r.top = value,
                    "right" => r.right = value,
                    "bottom" => r.bottom = value,
                    "left" => r.left = value,
                    other => return Err(format!("unknown side `{other}` in `{key}`")),
                }
            }
            Ok(r)
        }
        other => Err(format!(
            "`{key}` must be an integer or a table {{ top, right, bottom, left }}, not {}",
            other.kind()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Position};

    fn calls(src: &str) -> Vec<Call> {
        Source::new(src).unwrap().calls()
    }

    fn rule(src: &str) -> MonitorRule {
        let c = calls(src);
        assert_eq!(c.len(), 1, "{src}");
        c[0].rule.clone().unwrap()
    }

    fn error(src: &str) -> String {
        let c = calls(src);
        assert_eq!(c.len(), 1, "{src}");
        c[0].rule.clone().unwrap_err()
    }

    #[test]
    fn maintainer_rule() {
        let r = rule(
            r#"hl.monitor({ output = "DP-1",     mode = "2560x1440@179.95", position = "3360x975",  scale = 1, vrr = 2 })"#,
        );
        assert_eq!(r.output.as_str(), "DP-1");
        assert_eq!(
            r.mode,
            Some(Mode::Resolution {
                width: 2560,
                height: 1440,
                refresh: Some(179.95)
            })
        );
        assert_eq!(r.position, Some(Position::At { x: 3360, y: 975 }));
        assert_eq!(r.scale, Some(Scale::Factor(1.0)));
        assert_eq!(r.vrr, Some(2));
    }

    #[test]
    fn every_modelled_field() {
        let r = rule(
            r#"hl.monitor{
                output = "desc:Samsung", mode = "preferred", position = "auto-left",
                scale = "auto", transform = 5, disabled = false, vrr = -1,
                mirror = "eDP-1", bitdepth = 10, cm = "hdr", sdr_eotf = "gamma22",
                sdrbrightness = 1.2, sdrsaturation = "0.9", icc = "/x.icc",
                supports_wide_color = 1, supports_hdr = true, sdr_min_luminance = 0,
                sdr_max_luminance = 200, min_luminance = 0.01, max_luminance = 800,
                max_avg_luminance = 400, ["reserved"] = { top = 30, left = -2 },
                future_key = { 1, "a" }; }"#,
        );
        assert_eq!(r.transform, Some(Transform::new(5).unwrap()));
        assert_eq!(r.disabled, Some(false));
        assert_eq!(r.vrr, Some(-1));
        assert_eq!(r.scale, Some(Scale::Auto));
        assert_eq!(r.cm, Some(ColorManagement::Hdr));
        assert_eq!(r.sdrsaturation, Some(0.9));
        assert_eq!(r.supports_hdr, Some(1));
        assert_eq!(r.sdr_min_luminance, Some(0.0));
        assert_eq!(
            r.reserved,
            Some(Reserved {
                top: 30,
                right: 0,
                bottom: 0,
                left: -2
            })
        );
        assert_eq!(
            r.extra,
            [ExtraField {
                key: "future_key".to_owned(),
                raw: "{ 1, \"a\" }".to_owned()
            }]
        );
        let all =
            rule(r#"hl.monitor({ output = 1, reserved_area = 5, disabled = 1, bitdepth = true })"#);
        assert_eq!(all.output.as_str(), "1");
        assert_eq!(all.reserved.unwrap().right, 5);
        assert_eq!(all.disabled, Some(true));
        assert_eq!(all.bitdepth, Some(1));
    }

    /// Calls Hyprland rejects or silently changes, with hyprtilt's reason.
    const REJECTED: &[(&str, &str)] = &[
        (
            r#"hl.monitor({ output = "a", transform = 1.0 })"#,
            "`transform` must be an integer, not a float",
        ),
        (
            r#"hl.monitor({ output = "a", transform = 8 })"#,
            "`transform` must be from 0 to 7, not 8",
        ),
        (
            r#"hl.monitor({ output = "a", vrr = "2" })"#,
            "`vrr` must be an integer, not a string",
        ),
        (
            r#"hl.monitor({ output = "a", mode = "wide" })"#,
            "invalid mode: \"wide\"",
        ),
        (
            r#"hl.monitor({ output = "a", position = { x = 0 } })"#,
            "`position` must be a string, not a table",
        ),
        (
            r#"hl.monitor({ output = "a", scale = 0.1 })"#,
            "invalid scale: 0.1 (the minimum is 0.25)",
        ),
        (
            r#"hl.monitor({ output = "a", scale = ".5" })"#,
            "invalid scale: \".5\"",
        ),
        (
            r#"hl.monitor({ output = "a", cm = "sRGB" })"#,
            "invalid cm: \"sRGB\"",
        ),
        (
            r#"hl.monitor({ output = "a", disabled = 2 })"#,
            "`disabled` must be true or false, not an integer",
        ),
        (
            r#"hl.monitor({ output = "a", mirror = false })"#,
            "`mirror` must be a string, not a boolean",
        ),
        (
            r#"hl.monitor({ output = "a", icc = "" })"#,
            "`icc` must not be empty",
        ),
        (
            r#"hl.monitor({ output = "a", reserved = { up = 1 } })"#,
            "unknown side `up` in `reserved`",
        ),
        (
            r#"hl.monitor({ output = "a", reserved = 1, reserved_area = 2 })"#,
            "both `reserved` and `reserved_area` are given",
        ),
        (
            r#"hl.monitor({ output = "a", output = "b" })"#,
            "the key `output` appears twice",
        ),
        (
            r#"hl.monitor({ mode = "preferred" })"#,
            "`output` is missing",
        ),
        (
            r#"hl.monitor({ output = true })"#,
            "`output` must be a string, not a boolean",
        ),
        (
            r#"hl.monitor({ "DP-1" })"#,
            "positional value `\"DP-1\"` is not supported",
        ),
        (
            r#"hl.monitor({ output = name })"#,
            "output: `name` is not a literal",
        ),
        (
            r#"hl.monitor({ output = "a" .. b, scale = 1 })"#,
            "output: `\"a\" .. b` is not a literal",
        ),
        (
            r#"hl.monitor({ output = "a", sdrbrightness = "x" })"#,
            "`sdrbrightness` must be a number, not a string",
        ),
        (
            r#"hl.monitor({ output = "a", reserved = { top = 1.5 } })"#,
            "`top` must be an integer, not a float",
        ),
        (
            r#"hl.monitor({ output = "a", reserved = "1" })"#,
            "`reserved` must be an integer or a table { top, right, bottom, left }, not a string",
        ),
        (
            r#"hl.monitor({ output = "a", scale = true })"#,
            "`scale` must be a number or \"auto\", not a boolean",
        ),
        (
            r#"hl.monitor({ output = "a", mode = 1920 })"#,
            "invalid mode: \"1920\"",
        ),
        (
            r#"hl.monitor({ output = "a", scale = f(1) })"#,
            "scale: `f(1)` is not a literal",
        ),
        (
            r#"hl.monitor({ [k] = "a" })"#,
            "the key `[k]` is not a string literal",
        ),
        (r#"hl.monitor(spec)"#, "the argument is not a table literal"),
        (
            r#"hl.monitor({ output = "a" }, 2)"#,
            "the argument is not a table literal",
        ),
        (r#"hl.monitor "DP-1""#, "the argument must be a table"),
    ];

    #[test]
    fn values_hyprland_rejects_are_refused() {
        for &(src, message) in REJECTED {
            assert_eq!(error(src), message, "{src}");
        }
    }

    #[test]
    fn nil_values_are_absent_and_negatives_parse() {
        let r = rule(r#"hl.monitor({ output = "a", mode = nil, vrr = -1, sdrbrightness = -0.5 })"#);
        assert_eq!(r.mode, None);
        assert_eq!(r.vrr, Some(-1));
        assert_eq!(r.sdrbrightness, Some(-0.5));
    }

    #[test]
    fn statement_context_and_depth() {
        let src = r#"
local m = hl.monitor({ output = "a" })
hl.monitor({ output = "b" }).x = 1
if laptop then
  hl.monitor({ output = "c" })
end
local function f() hl.monitor{ output = "d" } end
x = 1 hl.monitor({ output = "e" }); y = 2
return hl.monitor({ output = "f" })
"#;
        let c = calls(src);
        let summary: Vec<(String, bool, i32)> = c
            .iter()
            .map(|c| {
                (
                    c.rule.as_ref().unwrap().output.to_string(),
                    c.statement,
                    c.depth,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("a".to_owned(), false, 0),
                ("b".to_owned(), false, 0),
                ("c".to_owned(), true, 1),
                ("d".to_owned(), true, 1),
                ("e".to_owned(), true, 0),
                ("f".to_owned(), false, 0),
            ]
        );
        assert_eq!(&src[c[4].span.clone()], r#"hl.monitor({ output = "e" });"#);
    }

    #[test]
    fn not_calls() {
        assert!(calls("local m = hl.monitor").is_empty());
        assert!(calls("x.hl.monitor({ output = 'a' })").is_empty());
        assert!(calls("hl.monitors({ output = 'a' })").is_empty());
        assert!(calls("-- hl.monitor({ output = 'a' })").is_empty());
        assert!(calls("s = [[hl.monitor({ output = 'a' })]]").is_empty());
        assert!(calls("hl.monitor({ output = 'a' ").is_empty());
    }

    #[test]
    fn top_level_returns_and_boundaries() {
        let src =
            "local t = {\n  a = 1,\n}\nif x then\n  return 1\nend\nlocal y =\n  2\nreturn t\n";
        let s = Source::new(src).unwrap();
        assert_eq!(s.top_level_returns(), [src.find("return t").unwrap()]);
        let starts: Vec<usize> = block::lines(src).iter().map(|l| l.full.start).collect();
        let boundaries: Vec<bool> = starts.iter().map(|&o| s.is_top_level_boundary(o)).collect();
        assert_eq!(
            boundaries,
            [true, false, false, true, false, false, true, false, true]
        );
        assert!(s.is_top_level_boundary(src.len()));
        // Inside a long string or comment is never a boundary.
        let src = "x = [[\nhl\n]]\n--[[\nq\n]]\n";
        let s = Source::new(src).unwrap();
        let starts: Vec<usize> = block::lines(src).iter().map(|l| l.full.start).collect();
        let boundaries: Vec<bool> = starts.iter().map(|&o| s.is_top_level_boundary(o)).collect();
        assert_eq!(boundaries, [true, false, false, true, false, false]);
    }

    #[test]
    fn lexical_errors_become_syntax_errors() {
        let Err(err) = Source::new("x = 1\ny = \"open\n") else {
            panic!("expected an error");
        };
        assert_eq!(
            err,
            ConfigError::Syntax {
                line: 2,
                message: "unfinished string".to_owned()
            }
        );
    }

    #[test]
    fn comment_queries() {
        let src = "-- a\nx = 1 -- b\n";
        let s = Source::new(src).unwrap();
        let comments = s.comments_in(&(4..src.len()));
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0], 11..15);
        assert_eq!(s.code_in(&(4..src.len())).len(), 3);
    }
}
