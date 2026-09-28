// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Reading and writing monitor rules in Lua configuration files.
//!
//! The managed block holds `hl.monitor({...})` calls with literal values,
//! comments and blank lines, between `-- BEGIN hyprtilt (managed)` and
//! `-- END hyprtilt`. Everything else in the file is opaque text that is
//! copied byte for byte. Nothing is ever executed.
//!
//! A new block goes after the last monitor rule outside it (so that its
//! rules come later and win), otherwise before the last top-level
//! `return` (code after a `return` is a syntax error, and Caelestia's
//! `hypr-user.lua` ends with one), otherwise at the end of the file. Every
//! insertion point is a statement boundary at the top level.

pub mod lexer;
mod recognize;
pub mod syntax;

use std::fmt::Write as _;

use crate::block::{self, BlockLocation, LUA_MARKERS};
use crate::body::{self, Item, Piece};
use crate::document::{ConfigDocument, ConfigError, Edit, FoundRule, ManagedBlock};
use crate::model::{MonitorRule, Scale};
use recognize::{Call, Source};

/// Read the managed block and the monitor rules outside it.
///
/// # Errors
///
/// Returns [`ConfigError`] when the file cannot be tokenised, the markers
/// are malformed, or the block holds anything but literal `hl.monitor`
/// calls, comments and blank lines.
///
/// # Examples
///
/// ```
/// let src = "hl.monitor({ output = \"DP-1\", scale = 1 })\n";
/// let doc = hyprtilt_core::lua::parse(src).unwrap();
/// assert!(doc.block.is_none());
/// assert_eq!(doc.outside.len(), 1);
/// assert!(doc.outside[0].is_adoptable());
/// ```
pub fn parse(src: &str) -> Result<ConfigDocument, ConfigError> {
    Ok(analyze(src)?.doc)
}

/// Write `rules` into the managed block, creating the block if the file has
/// none. Rules whose value did not change keep their text; if nothing
/// changed at all, the returned edit is unchanged.
///
/// # Errors
///
/// Returns [`ConfigError`] if the file cannot be read (see [`parse`]), if two
/// rules have the same selector, if a value cannot be written, or if the
/// block cannot be placed.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::model::MonitorRule;
///
/// let src = "hl.config({})\nreturn {}\n";
/// let mut rule = MonitorRule::new("DP-1");
/// rule.scale = Some(hyprtilt_core::model::Scale::Factor(1.0));
/// let edit = hyprtilt_core::lua::save(src, &[rule.clone()]).unwrap();
/// assert!(edit.changed);
/// assert!(edit.content.ends_with("-- END hyprtilt\n\nreturn {}\n"));
/// // Saving the same rules again changes nothing.
/// assert!(!hyprtilt_core::lua::save(&edit.content, &[rule]).unwrap().changed);
/// ```
pub fn save(src: &str, rules: &[MonitorRule]) -> Result<Edit, ConfigError> {
    check_unique(rules)?;
    let a = analyze(src)?;
    let eol = block::line_ending(src);
    let content = match &a.location {
        Some(location) => {
            let new_body = body::rewrite(src, &a.items, rules, &format_rule, eol)?;
            block::replace_body(src, location, &new_body)
        }
        None if rules.is_empty() => src.to_owned(),
        None => {
            let at = new_block_position(src, &a.source, &a.doc.outside)?;
            block::insert_block(src, at, &rules_text(rules)?, &LUA_MARKERS)
        }
    };
    verify(&content, rules)?;
    Ok(Edit::new(src, content))
}

/// Move monitor rules from outside the block into it. With `lines` empty,
/// every adoptable rule is moved; otherwise the rules starting on the given
/// lines. Rules for the same selector are merged the way Hyprland merges
/// them. Without a block, the block takes the place of the last adopted
/// rule, so that the evaluation order stays the same; adopted rules keep
/// their text unless they had to be merged.
///
/// # Errors
///
/// Returns [`ConfigError::NotAdoptable`] for a requested line without an
/// adoptable rule, and the errors of [`parse`] and [`save`].
///
/// # Examples
///
/// ```
/// let src = "-- screens\nhl.monitor({ output = \"DP-1\", scale = 1 })\nreturn {}\n";
/// let edit = hyprtilt_core::lua::adopt(src, &[]).unwrap();
/// assert_eq!(
///     edit.content,
///     "-- screens\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\", scale = 1 })\n-- END hyprtilt\nreturn {}\n"
/// );
/// ```
pub fn adopt(src: &str, lines: &[usize]) -> Result<Edit, ConfigError> {
    let a = analyze(src)?;
    let selected = select(&a.doc.outside, lines)?;
    if selected.is_empty() {
        return Ok(Edit::new(src, src.to_owned()));
    }
    let without = remove_spans(src, &selected);
    if let Some(location) = &a.location {
        let rules = merge_into_block(a.doc.block_rules(), &selected, location);
        let edit = save(&without, &rules)?;
        return Ok(Edit::new(src, edit.content));
    }

    let mut merged: Vec<(MonitorRule, Vec<&FoundRule>)> = Vec::new();
    for found in &selected {
        let rule = found.rule.clone().unwrap_or_else(|| MonitorRule::new(""));
        match merged.iter().position(|(r, _)| r.output == rule.output) {
            Some(i) => {
                let (mut earlier, mut sources) = merged.remove(i);
                earlier.overlay(&rule);
                sources.push(found);
                merged.push((earlier, sources));
            }
            None => merged.push((rule, vec![found])),
        }
    }
    let mut body_text = String::new();
    for (rule, sources) in &merged {
        match sources.as_slice() {
            [single] => body_text.push_str(&single.text),
            _ => body_text.push_str(&format_rule(rule)?),
        }
        body_text.push('\n');
    }
    let rules: Vec<MonitorRule> = merged.into_iter().map(|(r, _)| r).collect();

    let last = selected[selected.len() - 1];
    let removed_before: usize = selected[..selected.len() - 1]
        .iter()
        .map(|f| f.span.len())
        .sum();
    let mut at = last.span.start - removed_before;
    if at > 0 && without.as_bytes()[at - 1] != b'\n' {
        at = without[at..]
            .find('\n')
            .map_or(without.len(), |i| at + i + 1);
    }
    let source = Source::new(&without)?;
    let content = if source.is_top_level_boundary(at) {
        block::insert_block_in_place(&without, at, &body_text, &LUA_MARKERS)
    } else {
        let doc = analyze(&without)?.doc;
        let at = new_block_position(&without, &source, &doc.outside)?;
        block::insert_block(&without, at, &body_text, &LUA_MARKERS)
    };
    verify(&content, &rules)?;
    Ok(Edit::new(src, content))
}

/// Remove the two marker lines and leave the block's rules as ordinary
/// configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NoBlock`] for a file without a block, and the
/// errors of [`parse`].
///
/// # Examples
///
/// ```
/// let src = "-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\" })\n-- END hyprtilt\n";
/// let edit = hyprtilt_core::lua::unmanage(src).unwrap();
/// assert_eq!(edit.content, "hl.monitor({ output = \"DP-1\" })\n");
/// ```
pub fn unmanage(src: &str) -> Result<Edit, ConfigError> {
    let a = analyze(src)?;
    let location = a.location.ok_or(ConfigError::NoBlock)?;
    Ok(Edit::new(src, block::remove_markers(src, &location)))
}

/// The Lua code of one rule: `hl.monitor({ output = "...", ... })`, on one
/// line, with the fields in a fixed order and every value as Hyprland
/// expects its type.
///
/// # Errors
///
/// Returns [`ConfigError::Unrepresentable`] for a number that is not finite.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::model::{MonitorRule, Position, Scale, Transform};
///
/// let mut rule = MonitorRule::new("HDMI-A-1");
/// rule.mode = Some("2560x1440@144".parse().unwrap());
/// rule.position = Some(Position::At { x: 0, y: 0 });
/// rule.scale = Some(Scale::Factor(1.0));
/// rule.transform = Some(Transform::new(1).unwrap());
/// assert_eq!(
///     hyprtilt_core::lua::format_rule(&rule).unwrap(),
///     r#"hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144", position = "0x0", scale = 1, transform = 1 })"#
/// );
/// ```
pub fn format_rule(rule: &MonitorRule) -> Result<String, ConfigError> {
    let number = |key: &str, v: f64| lua_number(rule, key, v);
    let mut fields = vec![format!("output = {}", lua_string(rule.output.as_str()))];
    let mut push = |key: &str, value: String| fields.push(format!("{key} = {value}"));
    if let Some(m) = &rule.mode {
        push("mode", lua_string(&m.to_string()));
    }
    if let Some(p) = &rule.position {
        push("position", lua_string(&p.to_string()));
    }
    match rule.scale {
        Some(Scale::Factor(v)) if v.fract() == 0.0 => push("scale", number("scale", v)?),
        Some(s) => push("scale", lua_string(&s.to_string())),
        None => {}
    }
    if let Some(t) = rule.transform {
        push("transform", t.value().to_string());
    }
    if let Some(d) = rule.disabled {
        push("disabled", d.to_string());
    }
    if let Some(v) = rule.vrr {
        push("vrr", v.to_string());
    }
    if let Some(m) = &rule.mirror {
        push("mirror", lua_string(m));
    }
    if let Some(b) = rule.bitdepth {
        push("bitdepth", b.to_string());
    }
    if let Some(cm) = rule.cm {
        push("cm", lua_string(&cm.to_string()));
    }
    if let Some(e) = &rule.sdr_eotf {
        push("sdr_eotf", lua_string(e));
    }
    if let Some(v) = rule.sdrbrightness {
        push("sdrbrightness", number("sdrbrightness", v)?);
    }
    if let Some(v) = rule.sdrsaturation {
        push("sdrsaturation", number("sdrsaturation", v)?);
    }
    if let Some(icc) = &rule.icc {
        push("icc", lua_string(icc));
    }
    if let Some(v) = rule.supports_wide_color {
        push("supports_wide_color", v.to_string());
    }
    if let Some(v) = rule.supports_hdr {
        push("supports_hdr", v.to_string());
    }
    if let Some(v) = rule.sdr_min_luminance {
        push("sdr_min_luminance", number("sdr_min_luminance", v)?);
    }
    if let Some(v) = rule.sdr_max_luminance {
        push("sdr_max_luminance", v.to_string());
    }
    if let Some(v) = rule.min_luminance {
        push("min_luminance", number("min_luminance", v)?);
    }
    if let Some(v) = rule.max_luminance {
        push("max_luminance", v.to_string());
    }
    if let Some(v) = rule.max_avg_luminance {
        push("max_avg_luminance", v.to_string());
    }
    if let Some(r) = rule.reserved {
        push(
            "reserved",
            format!(
                "{{ top = {}, right = {}, bottom = {}, left = {} }}",
                r.top, r.right, r.bottom, r.left
            ),
        );
    }
    for extra in &rule.extra {
        push(&lua_key(&extra.key), extra.raw.clone());
    }
    Ok(format!("hl.monitor({{ {} }})", fields.join(", ")))
}

/// The Lua code for `hyprctl eval` that applies `rules` in one request.
///
/// # Errors
///
/// Returns the errors of [`format_rule`].
pub fn eval_code(rules: &[MonitorRule]) -> Result<String, ConfigError> {
    let calls: Vec<String> = rules.iter().map(format_rule).collect::<Result<_, _>>()?;
    Ok(calls.join(" "))
}

/// A Lua string literal. Printable text is written as is; quotes,
/// backslashes and control characters are escaped.
fn lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii_control() => {
                // Writing to a String cannot fail.
                let _ = write!(out, "\\{:03}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A table key: a plain name, or `["..."]` when the key is not a valid
/// identifier.
fn lua_key(key: &str) -> String {
    let identifier = key
        .chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && key.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !lexer::is_keyword(key);
    if identifier {
        key.to_owned()
    } else {
        format!("[{}]", lua_string(key))
    }
}

fn lua_number(rule: &MonitorRule, key: &str, v: f64) -> Result<String, ConfigError> {
    if !v.is_finite() {
        return Err(ConfigError::Unrepresentable {
            output: rule.output.to_string(),
            message: format!("`{key}` is not a finite number"),
        });
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Ok(format!("{}", v as i64))
    } else {
        Ok(format!("{v}"))
    }
}

fn rules_text(rules: &[MonitorRule]) -> Result<String, ConfigError> {
    rules
        .iter()
        .map(|r| format_rule(r).map(|t| t + "\n"))
        .collect()
}

/// A tokenised file with its block, its items and the document.
struct Analysis<'a> {
    source: Source<'a>,
    location: Option<BlockLocation>,
    items: Vec<Item>,
    doc: ConfigDocument,
}

fn analyze(src: &str) -> Result<Analysis<'_>, ConfigError> {
    let source = Source::new(src)?;
    let location = block::locate(src, &LUA_MARKERS)?;
    if let Some(loc) = &location {
        check_marker(&source, src, loc.begin_marker.clone(), loc.begin_line)?;
        check_marker(&source, src, loc.end_marker.clone(), loc.end_line)?;
    }
    let (inside, outside): (Vec<Call>, Vec<Call>) = source.calls().into_iter().partition(|c| {
        location
            .as_ref()
            .is_some_and(|l| l.body.contains(&c.span.start))
    });
    let mut found = Vec::new();
    for call in outside {
        if let Some(loc) = &location
            && call.span.start < loc.span.end
            && call.span.end > loc.span.start
        {
            return Err(ConfigError::Syntax {
                line: block::line_of(src, call.span.start),
                message: format!(
                    "a statement crosses the managed block marker on line {}",
                    loc.begin_line
                ),
            });
        }
        found.push(found_rule(src, &source, call));
    }
    let (block, items) = match &location {
        Some(loc) => {
            let (block, items) = block_contents(src, &source, loc, inside)?;
            (Some(block), items)
        }
        None => (None, Vec::new()),
    };
    Ok(Analysis {
        source,
        location,
        items,
        doc: ConfigDocument {
            block,
            outside: found,
        },
    })
}

/// A marker line must be a real comment, not text inside a string or a
/// long comment.
fn check_marker(
    source: &Source<'_>,
    src: &str,
    line: std::ops::Range<usize>,
    number: usize,
) -> Result<(), ConfigError> {
    let text = &src[line.clone()];
    let start = line.start + (text.len() - text.trim_start().len());
    let end = line.start + text.trim_end().len();
    let is_comment = source
        .comments_in(&(start..start + 1))
        .iter()
        .any(|c| c.start == start && c.end >= end && c.end <= line.end);
    if is_comment {
        Ok(())
    } else {
        Err(ConfigError::Syntax {
            line: number,
            message: "the managed block marker is inside a string or a long comment".to_owned(),
        })
    }
}

fn block_contents(
    src: &str,
    source: &Source<'_>,
    loc: &BlockLocation,
    inside: Vec<Call>,
) -> Result<(ManagedBlock, Vec<Item>), ConfigError> {
    let unsupported = |offset: usize, message: String| ConfigError::UnsupportedInBlock {
        line: block::line_of(src, offset),
        message,
    };
    for token in source.code_in(&loc.body) {
        let covered = inside.iter().any(|c| c.span.contains(&token.start));
        if !covered {
            let line = block::lines(src)
                .into_iter()
                .find(|l| l.full.contains(&token.start))
                .map(|l| src[l.content].trim().to_owned())
                .unwrap_or_default();
            return Err(unsupported(
                token.start,
                format!("only literal hl.monitor calls are allowed, found `{line}`"),
            ));
        }
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for call in inside {
        if call.span.end > loc.body.end {
            return Err(unsupported(
                call.span.start,
                "the statement crosses the end marker".to_owned(),
            ));
        }
        if !call.statement {
            return Err(unsupported(
                call.span.start,
                "the hl.monitor call is not a statement of its own (check the line before the block)"
                    .to_owned(),
            ));
        }
        let rule = call.rule.map_err(|e| unsupported(call.span.start, e))?;
        pieces.push(Piece {
            span: call.span,
            rule: Some(rule),
        });
    }
    let comments: Vec<Piece> = source
        .comments_in(&loc.body)
        .into_iter()
        .filter(|c| !pieces.iter().any(|p| p.span.contains(&c.start)))
        .map(|span| Piece { span, rule: None })
        .collect();
    pieces.extend(comments);
    pieces.sort_by_key(|p| p.span.start);
    let items = body::items(src, &loc.body, &pieces)?;
    let rules = items.iter().flat_map(|i| i.rules.clone()).collect();
    let rule_lines = items.iter().flat_map(|i| i.rule_lines.clone()).collect();
    Ok((
        ManagedBlock {
            span: loc.span.clone(),
            begin_line: loc.begin_line,
            end_line: loc.end_line,
            rules,
            rule_lines,
        },
        items,
    ))
}

fn found_rule(src: &str, source: &Source<'_>, call: Call) -> FoundRule {
    let span = removal_span(src, source, &call.span);
    let (rule, reason) = match call.rule {
        Ok(rule) => {
            let reason = if call.depth > 0 {
                Some("inside a function or a conditional block".to_owned())
            } else if !call.statement {
                Some("part of a larger expression".to_owned())
            } else {
                None
            };
            (Some(rule), reason)
        }
        Err(e) => (None, Some(e)),
    };
    FoundRule {
        rule,
        text: src[call.span.clone()].to_owned(),
        line: block::line_of(src, call.span.start),
        span,
        not_adoptable: reason,
    }
}

/// The bytes to remove to take a statement out of the file: whole lines
/// (with a trailing comment) when it stands alone on them, otherwise just
/// the statement and the whitespace that separated it.
fn removal_span(
    src: &str,
    source: &Source<'_>,
    stmt: &std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let lines = block::lines(src);
    let first = lines
        .iter()
        .find(|l| l.full.contains(&stmt.start))
        .map_or(0..src.len(), |l| l.full.clone());
    let last = lines
        .iter()
        .find(|l| l.full.contains(&(stmt.end - 1)))
        .map_or(0..src.len(), |l| l.full.clone());
    let last_content_end = src[last.clone()].trim_end_matches(['\n', '\r']).len() + last.start;
    let before = &src[first.start..stmt.start];
    let after = &src[stmt.end..last_content_end];
    let trailing_comment = || {
        let comments = source.comments_in(&(stmt.end..last_content_end));
        comments.first().is_some_and(|c| {
            src[stmt.end..c.start].trim().is_empty()
                && c.end <= last_content_end
                && src[c.end..last_content_end].trim().is_empty()
        })
    };
    if before.trim().is_empty() && (after.trim().is_empty() || trailing_comment()) {
        first.start..last.end
    } else if before.trim().is_empty() {
        let ws = after.len() - after.trim_start().len();
        stmt.start..stmt.end + ws
    } else {
        let ws = before.len() - before.trim_end().len();
        stmt.start - ws..stmt.end
    }
}

/// Where a new block goes (see the module documentation).
fn new_block_position(
    src: &str,
    source: &Source<'_>,
    outside: &[FoundRule],
) -> Result<usize, ConfigError> {
    let returns = source.top_level_returns();
    let limit = returns.last().copied().unwrap_or(src.len());
    if let Some(last) = outside.last() {
        let candidates = block::lines(src)
            .into_iter()
            .map(|l| l.full.start)
            .chain(std::iter::once(src.len()))
            .filter(|&o| o >= last.span.end && o <= limit);
        for offset in candidates {
            if source.is_top_level_boundary(offset) {
                return Ok(offset);
            }
        }
    }
    if let Some(&ret) = returns.last() {
        let line_start = src[..ret].rfind('\n').map_or(0, |i| i + 1);
        if src[line_start..ret].trim().is_empty() && source.is_top_level_boundary(line_start) {
            return Ok(line_start);
        }
        return Err(ConfigError::Syntax {
            line: block::line_of(src, ret),
            message: "the top-level `return` shares its line with other code; \
                      put it on a line of its own so that the managed block can go before it"
                .to_owned(),
        });
    }
    Ok(src.len())
}

fn select<'d>(
    outside: &'d [FoundRule],
    lines: &[usize],
) -> Result<Vec<&'d FoundRule>, ConfigError> {
    if lines.is_empty() {
        return Ok(outside.iter().filter(|f| f.is_adoptable()).collect());
    }
    let mut selected = Vec::new();
    for &line in lines {
        let found =
            outside
                .iter()
                .find(|f| f.line == line)
                .ok_or_else(|| ConfigError::NotAdoptable {
                    line,
                    reason: "no monitor rule starts on this line".to_owned(),
                })?;
        if let Some(reason) = &found.not_adoptable {
            return Err(ConfigError::NotAdoptable {
                line,
                reason: reason.clone(),
            });
        }
        selected.push(found);
    }
    selected.sort_by_key(|f| f.span.start);
    selected.dedup_by_key(|f| f.span.start);
    Ok(selected)
}

/// `src` without the removal spans of `rules` (which are sorted and do not
/// overlap).
fn remove_spans(src: &str, rules: &[&FoundRule]) -> String {
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for f in rules {
        out.push_str(&src[at..f.span.start]);
        at = f.span.end;
    }
    out.push_str(&src[at..]);
    out
}

/// The block's rules after adopting `selected`: a rule before the block is
/// overridden by the block's rule for the same selector, a rule after it
/// overrides the block's rule, exactly as Hyprland evaluates them.
fn merge_into_block(
    block_rules: &[MonitorRule],
    selected: &[&FoundRule],
    location: &BlockLocation,
) -> Vec<MonitorRule> {
    let mut rules = merge_in_order(block_rules.iter().cloned());
    let mut earlier = Vec::new();
    for found in selected {
        let Some(rule) = found.rule.clone() else {
            continue;
        };
        let existing = rules.iter_mut().find(|b| b.output == rule.output);
        match (found.span.end <= location.span.start, existing) {
            (true, Some(block_rule)) => {
                let mut merged = rule;
                merged.overlay(block_rule);
                *block_rule = merged;
            }
            (true, None) => earlier.push(rule),
            (false, Some(block_rule)) => block_rule.overlay(&rule),
            (false, None) => rules.push(rule),
        }
    }
    let mut all = merge_in_order(earlier);
    all.extend(rules);
    all
}

/// Merge rules with the same selector the way Hyprland's rule list does:
/// the merged rule takes the position of the later one.
fn merge_in_order(rules: impl IntoIterator<Item = MonitorRule>) -> Vec<MonitorRule> {
    let mut out: Vec<MonitorRule> = Vec::new();
    for rule in rules {
        match out.iter().position(|r| r.output == rule.output) {
            Some(i) => {
                let mut merged = out.remove(i);
                merged.overlay(&rule);
                out.push(merged);
            }
            None => out.push(rule),
        }
    }
    out
}

fn check_unique(rules: &[MonitorRule]) -> Result<(), ConfigError> {
    for (i, rule) in rules.iter().enumerate() {
        if rules[..i].iter().any(|r| r.output == rule.output) {
            return Err(ConfigError::Unrepresentable {
                output: rule.output.to_string(),
                message: "more than one rule for the same output".to_owned(),
            });
        }
    }
    Ok(())
}

/// Read the new content back and check that its block holds exactly the
/// requested rules. This guards against writing a file that does not say
/// what hyprtilt meant.
fn verify(content: &str, rules: &[MonitorRule]) -> Result<(), ConfigError> {
    let internal = |output: &str, what: &str| ConfigError::Unrepresentable {
        output: output.to_owned(),
        message: format!("{what} (this is a bug in hyprtilt; nothing was written)"),
    };
    let doc = analyze(content)
        .map_err(|e| internal("", &format!("the new content does not read back: {e}")))?
        .doc;
    let got = doc.block_rules();
    if got.len() != rules.len() {
        return Err(internal("", "the block does not hold the requested rules"));
    }
    for rule in rules {
        let ok = got
            .iter()
            .any(|g| g.output == rule.output && (g == rule || *g == rule.normalized()));
        if !ok {
            return Err(internal(
                rule.output.as_str(),
                "the rule does not read back as written",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
