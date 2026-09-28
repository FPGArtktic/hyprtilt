// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Reading and writing monitor rules in hyprlang configuration files.
//!
//! The managed block holds `monitor=` lines, `monitorv2 { }` blocks with
//! literal values, comments and blank lines, between
//! `# BEGIN hyprtilt (managed)` and `# END hyprtilt`. Variables (`$name`),
//! expressions (`{{...}}`), `source=` and `# hyprlang` directives are
//! refused inside the block and opaque outside it.
//!
//! hyprtilt writes one `monitor=` line per rule, always in the full form
//! `monitor = SEL, MODE, POSITION, SCALE[, key, value]...` (or
//! `monitor = SEL, disable`), and a `monitorv2` block only for fields the
//! line form cannot express (`sdr_eotf`, `supports_*`, the luminances,
//! `reserved`). It never writes the `transform` or `addreserved` short
//! forms: they patch an earlier rule and depend on its position.
//! `docs/hyprland-lua-api.md`, section 8, has the grammar.

mod scan;

use std::fmt::Write as _;
use std::ops::Range;

use crate::adopt::{self, check_unique};
use crate::block::{self, BlockLocation, HYPRLANG_MARKERS};
use crate::body::{self, Item, Piece};
use crate::document::{ConfigDocument, ConfigError, Edit, FoundRule, ManagedBlock, SaveOptions};
use crate::model::{
    ColorManagement, Mode, MonitorRule, Position, Reserved, SDR_EOTF_NAMES, Scale, Transform,
};
use crate::version::Version;
use scan::{LineKind, Logical};

/// The keywords a `monitor=` line accepts after the scale; any other
/// keyword makes Hyprland ignore the whole line.
const LINE_KEYWORDS: &[&str] = &[
    "mirror",
    "bitdepth",
    "cm",
    "sdrsaturation",
    "sdrbrightness",
    "transform",
    "vrr",
    "icc",
];

/// Read the managed block and the monitor rules outside it.
///
/// # Errors
///
/// Returns [`ConfigError`] when the markers are malformed or the block
/// holds anything but literal monitor rules, comments and blank lines.
///
/// # Examples
///
/// ```
/// let doc = hyprtilt_core::hyprlang::parse("monitor = DP-1, 1920x1080@60, 0x0, 1\n").unwrap();
/// assert_eq!(doc.outside.len(), 1);
/// assert!(doc.outside[0].is_adoptable());
/// ```
pub fn parse(src: &str) -> Result<ConfigDocument, ConfigError> {
    Ok(analyze(src)?.doc)
}

/// Write `rules` into the managed block, creating it after the last
/// monitor rule outside it, or at the end of the file. Unchanged rules keep
/// their text; if nothing changed, the edit is unchanged.
///
/// # Errors
///
/// Returns [`ConfigError`] if the file cannot be read (see [`parse`]), if
/// two rules have the same selector, or if a rule cannot be written in
/// hyprlang or for the Hyprland version in `options`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::document::SaveOptions;
/// use hyprtilt_core::model::MonitorRule;
///
/// let mut rule = MonitorRule::new("DP-1");
/// rule.mode = Some("2560x1440@144".parse().unwrap());
/// let edit = hyprtilt_core::hyprlang::save("", &[rule], &SaveOptions::default()).unwrap();
/// assert_eq!(
///     edit.content,
///     "# BEGIN hyprtilt (managed)\nmonitor = DP-1, 2560x1440@144, auto, auto\n# END hyprtilt\n"
/// );
/// ```
pub fn save(src: &str, rules: &[MonitorRule], options: &SaveOptions) -> Result<Edit, ConfigError> {
    check_unique(rules)?;
    let rules: Vec<MonitorRule> = rules.iter().map(canonical).collect();
    let format = |rule: &MonitorRule| format_rule(rule, options);
    let a = analyze(src)?;
    let eol = block::line_ending(src);
    let content = match &a.location {
        Some(location) => {
            let new_body = body::rewrite(src, &a.items, &rules, &format, eol)?;
            block::replace_body(src, location, &new_body)
        }
        None if rules.is_empty() => src.to_owned(),
        None => {
            let at = new_block_position(src, &a)?;
            let text: String = rules
                .iter()
                .map(|r| format(r).map(|t| t + "\n"))
                .collect::<Result<_, _>>()?;
            block::insert_block(src, at, &text, &HYPRLANG_MARKERS)
        }
    };
    verify(&content, &rules)?;
    Ok(Edit::new(src, content))
}

/// Move monitor rules from outside the block into it: every adoptable rule
/// when `lines` is empty, otherwise the rules starting on `lines`. A later
/// `monitor=` line for the same selector replaces an earlier one, and a
/// `monitorv2` block replaces a `monitor=` line wherever it is, as in
/// Hyprland.
///
/// # Errors
///
/// Returns [`ConfigError::NotAdoptable`] or [`ConfigError::AdoptCrossing`]
/// when the requested rules cannot be moved, and the errors of [`save`].
///
/// # Examples
///
/// ```
/// let src = "monitor = DP-1, 1920x1080@60, 0x0, 1\n";
/// let edit = hyprtilt_core::hyprlang::adopt(src, &[]).unwrap();
/// assert_eq!(
///     edit.content,
///     "# BEGIN hyprtilt (managed)\nmonitor = DP-1, 1920x1080@60, 0x0, 1\n# END hyprtilt\n"
/// );
/// ```
pub fn adopt(src: &str, lines: &[usize]) -> Result<Edit, ConfigError> {
    let a = analyze(src)?;
    let selected = adopt::select(&a.doc.outside, lines)?;
    if selected.is_empty() {
        return Ok(Edit::new(src, src.to_owned()));
    }
    let block_start = a.location.as_ref().map(|l| l.span.start);
    adopt::check_crossing(&a.doc.outside, &selected, block_start)?;
    let without = adopt::remove_spans(src, &selected);
    // monitorv2 rules are applied after every monitor= line.
    let mut ordered: Vec<&FoundRule> = selected.iter().copied().filter(|f| !is_v2(f)).collect();
    ordered.extend(selected.iter().copied().filter(|f| is_v2(f)));
    let options = SaveOptions::default();
    if let Some(start) = block_start {
        let rules = adopt::merge_into_block(a.doc.block_rules(), &ordered, start, adopt::replace);
        let edit = save(&without, &rules, &options)?;
        return Ok(Edit::new(src, edit.content));
    }

    let mut winners: Vec<&FoundRule> = Vec::new();
    for found in ordered {
        winners.retain(|w| {
            w.rule.as_ref().map(|r| &r.output) != found.rule.as_ref().map(|r| &r.output)
        });
        winners.push(found);
    }
    let text: String = winners
        .iter()
        .flat_map(|f| [f.text.as_str(), "\n"])
        .collect();
    let rules: Vec<MonitorRule> = winners.iter().filter_map(|f| f.rule.clone()).collect();
    let last = selected[selected.len() - 1];
    let removed_before: usize = selected[..selected.len() - 1]
        .iter()
        .map(|f| f.span.len())
        .sum();
    let at = last.span.start - removed_before;
    let a2 = analyze(&without)?;
    let content = if is_insertion_point(&a2, at) {
        block::insert_block_in_place(&without, at, &text, &HYPRLANG_MARKERS)
    } else {
        let at = new_block_position(&without, &a2)?;
        block::insert_block(&without, at, &text, &HYPRLANG_MARKERS)
    };
    verify(&content, &rules)?;
    Ok(Edit::new(src, content))
}

fn is_v2(found: &FoundRule) -> bool {
    found.text.trim_start().starts_with("monitorv2")
}

/// Remove the two marker lines and leave the block's rules as ordinary
/// configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NoBlock`] for a file without a block, and the
/// errors of [`parse`].
pub fn unmanage(src: &str) -> Result<Edit, ConfigError> {
    let a = analyze(src)?;
    let location = a.location.ok_or(ConfigError::NoBlock)?;
    Ok(Edit::new(src, block::remove_markers(src, &location)))
}

/// The hyprlang text of one rule: a `monitor=` line, or a `monitorv2`
/// block (lines separated by `\n`) when a field needs it.
///
/// # Errors
///
/// Returns [`ConfigError::Unrepresentable`] for a field hyprlang does not
/// have, a value hyprlang would change (a `$`, surrounding whitespace, a
/// trailing backslash), or a field the Hyprland version in `options` does
/// not support.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::document::SaveOptions;
/// use hyprtilt_core::model::{MonitorRule, Transform};
///
/// let mut rule = MonitorRule::new("HDMI-A-1");
/// rule.mode = Some("2560x1440@144".parse().unwrap());
/// rule.position = Some("0x0".parse().unwrap());
/// rule.scale = Some("1".parse().unwrap());
/// rule.transform = Some(Transform::new(1).unwrap());
/// assert_eq!(
///     hyprtilt_core::hyprlang::format_rule(&rule, &SaveOptions::default()).unwrap(),
///     "monitor = HDMI-A-1, 2560x1440@144, 0x0, 1, transform, 1"
/// );
/// ```
pub fn format_rule(rule: &MonitorRule, options: &SaveOptions) -> Result<String, ConfigError> {
    check_representable(rule, options)?;
    if let Some(field) = needs_v2(rule) {
        if let Some(v) = options.hyprland
            && !v.at_least(Version::MONITORV2)
        {
            return Err(unrepresentable(
                rule,
                &format!("`{field}` needs a monitorv2 block, which needs Hyprland 0.50 or newer"),
            ));
        }
        return v2_text(rule);
    }
    Ok(format!("monitor = {}", line_value(rule, ", ")?))
}

/// The value for `hyprctl keyword monitor <value>`, which changes the
/// running session until the next reload (hyprlang configurations only).
///
/// # Errors
///
/// Returns [`ConfigError::Unrepresentable`] for a rule that needs a
/// `monitorv2` block (which `keyword` cannot apply before a reload) or
/// cannot be written at all.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::model::MonitorRule;
///
/// let mut rule = MonitorRule::new("DP-1");
/// rule.disabled = Some(true);
/// assert_eq!(hyprtilt_core::hyprlang::keyword_value(&rule).unwrap(), "DP-1,disable");
/// ```
pub fn keyword_value(rule: &MonitorRule) -> Result<String, ConfigError> {
    check_representable(rule, &SaveOptions::default())?;
    if let Some(field) = needs_v2(rule) {
        return Err(unrepresentable(
            rule,
            &format!("`{field}` cannot be applied live in hyprlang; write the file instead"),
        ));
    }
    line_value(&canonical(rule), ",")
}

/// The first field that only a `monitorv2` block can express.
fn needs_v2(rule: &MonitorRule) -> Option<&'static str> {
    [
        ("sdr_eotf", rule.sdr_eotf.is_some()),
        ("supports_wide_color", rule.supports_wide_color.is_some()),
        ("supports_hdr", rule.supports_hdr.is_some()),
        ("sdr_min_luminance", rule.sdr_min_luminance.is_some()),
        ("sdr_max_luminance", rule.sdr_max_luminance.is_some()),
        ("min_luminance", rule.min_luminance.is_some()),
        ("max_luminance", rule.max_luminance.is_some()),
        ("max_avg_luminance", rule.max_avg_luminance.is_some()),
        ("reserved", rule.reserved.is_some()),
    ]
    .into_iter()
    .find_map(|(name, set)| set.then_some(name))
}

fn unrepresentable(rule: &MonitorRule, message: &str) -> ConfigError {
    ConfigError::Unrepresentable {
        output: rule.output.to_string(),
        message: message.to_owned(),
    }
}

fn check_representable(rule: &MonitorRule, options: &SaveOptions) -> Result<(), ConfigError> {
    if let Some(extra) = rule.extra.first() {
        return Err(unrepresentable(
            rule,
            &format!("hyprlang has no field `{}`", extra.key),
        ));
    }
    if let Some(v) = options.hyprland {
        if rule.sdr_eotf.is_some() && !v.at_least(Version::SDR_EOTF_NAMES) {
            return Err(unrepresentable(
                rule,
                "`sdr_eotf` needs Hyprland 0.54 or newer",
            ));
        }
        if rule.icc.is_some() && !v.at_least(Version::ICC) {
            return Err(unrepresentable(rule, "`icc` needs Hyprland 0.55 or newer"));
        }
    }
    Ok(())
}

/// The value a rule reads back as after hyprtilt writes it in hyprlang: the
/// line form always has mode, position and scale, and a disabled line has
/// nothing else.
fn canonical(rule: &MonitorRule) -> MonitorRule {
    let rule = rule.normalized();
    if needs_v2(&rule).is_some() {
        return rule;
    }
    if rule.is_disabled() {
        let mut disabled = MonitorRule::new(rule.output.as_str());
        disabled.disabled = Some(true);
        return disabled;
    }
    let mut r = rule;
    r.disabled = None;
    r.mode.get_or_insert(Mode::Preferred);
    r.position.get_or_insert(Position::Auto(None));
    r.scale.get_or_insert(Scale::Auto);
    r
}

/// `SEL, MODE, POSITION, SCALE, ...` (or `SEL, disable`) with `separator`
/// between the fields.
fn line_value(rule: &MonitorRule, separator: &str) -> Result<String, ConfigError> {
    let rule = canonical(rule);
    let arg = |s: &str| value_text(&rule, s).map(|v| v.replace(',', "\\,"));
    let mut fields = vec![arg(rule.output.as_str())?];
    if rule.is_disabled() {
        fields.push("disable".to_owned());
        return Ok(fields.join(separator));
    }
    let text = |v: Option<String>| v.unwrap_or_default();
    fields.push(arg(&text(rule.mode.as_ref().map(ToString::to_string)))?);
    fields.push(arg(&text(rule.position.map(|p| p.to_string())))?);
    fields.push(arg(&text(rule.scale.map(|s| s.to_string())))?);
    let mut extra = |key: &str, value: Option<String>| -> Result<(), ConfigError> {
        if let Some(v) = value {
            fields.push(key.to_owned());
            fields.push(arg(&v)?);
        }
        Ok(())
    };
    extra("transform", rule.transform.map(|t| t.value().to_string()))?;
    extra("vrr", rule.vrr.map(|v| v.to_string()))?;
    extra("mirror", rule.mirror.clone())?;
    extra("bitdepth", rule.bitdepth.map(|v| v.to_string()))?;
    extra("cm", rule.cm.map(|v| v.to_string()))?;
    extra(
        "sdrbrightness",
        rule.sdrbrightness
            .map(|v| float_text(&rule, v))
            .transpose()?,
    )?;
    extra(
        "sdrsaturation",
        rule.sdrsaturation
            .map(|v| float_text(&rule, v))
            .transpose()?,
    )?;
    extra("icc", rule.icc.clone())?;
    Ok(fields.join(separator))
}

fn float_text(rule: &MonitorRule, v: f64) -> Result<String, ConfigError> {
    if v.is_finite() {
        Ok(format!("{v}"))
    } else {
        Err(unrepresentable(rule, "a number is not finite"))
    }
}

/// A value as hyprlang reads it back unchanged: `#` doubled; `$`, `{{`,
/// control characters, surrounding whitespace and a trailing backslash
/// cannot be written.
fn value_text(rule: &MonitorRule, s: &str) -> Result<String, ConfigError> {
    let problem = if s.contains('$') {
        Some("`$` would be expanded as a hyprlang variable")
    } else if s.contains("{{") {
        Some("`{{` would be evaluated as a hyprlang expression")
    } else if s.chars().any(char::is_control) {
        Some("control characters cannot be written in hyprlang")
    } else if s != s.trim() {
        Some("hyprlang trims leading and trailing whitespace")
    } else if s.ends_with('\\') {
        Some("a trailing backslash would join the next line")
    } else {
        None
    };
    match problem {
        Some(p) => Err(unrepresentable(rule, &format!("{s:?}: {p}"))),
        None => Ok(s.replace('#', "##")),
    }
}

fn v2_text(rule: &MonitorRule) -> Result<String, ConfigError> {
    let mut out = String::from("monitorv2 {\n");
    let mut field = |key: &str, value: Option<String>| -> Result<(), ConfigError> {
        if let Some(v) = value {
            // Writing to a String cannot fail.
            let _ = writeln!(out, "    {key} = {}", value_text(rule, &v)?);
        }
        Ok(())
    };
    let float = |v: Option<f64>| v.map(|v| float_text(rule, v)).transpose();
    field("output", Some(rule.output.to_string()))?;
    field("mode", rule.mode.as_ref().map(ToString::to_string))?;
    field("position", rule.position.map(|p| p.to_string()))?;
    field("scale", rule.scale.map(|s| s.to_string()))?;
    field("transform", rule.transform.map(|t| t.value().to_string()))?;
    field("disabled", rule.disabled.map(|d| u8::from(d).to_string()))?;
    field("vrr", rule.vrr.map(|v| v.to_string()))?;
    field("mirror", rule.mirror.clone())?;
    field("bitdepth", rule.bitdepth.map(|v| v.to_string()))?;
    field("cm", rule.cm.map(|v| v.to_string()))?;
    field("sdr_eotf", rule.sdr_eotf.clone())?;
    field("sdrbrightness", float(rule.sdrbrightness)?)?;
    field("sdrsaturation", float(rule.sdrsaturation)?)?;
    field(
        "supports_wide_color",
        rule.supports_wide_color.map(|v| v.to_string()),
    )?;
    field("supports_hdr", rule.supports_hdr.map(|v| v.to_string()))?;
    field("sdr_min_luminance", float(rule.sdr_min_luminance)?)?;
    field(
        "sdr_max_luminance",
        rule.sdr_max_luminance.map(|v| v.to_string()),
    )?;
    field("min_luminance", float(rule.min_luminance)?)?;
    field("max_luminance", rule.max_luminance.map(|v| v.to_string()))?;
    field(
        "max_avg_luminance",
        rule.max_avg_luminance.map(|v| v.to_string()),
    )?;
    field(
        "addreserved",
        rule.reserved
            .map(|r| format!("{}, {}, {}, {}", r.top, r.bottom, r.left, r.right)),
    )?;
    field("icc", rule.icc.clone())?;
    out.push('}');
    Ok(out)
}

/// What a monitor statement defines.
#[derive(Debug, Clone, PartialEq)]
enum Parsed {
    /// A complete rule with literal values.
    Rule(Box<MonitorRule>),
    /// Something hyprtilt does not take over, with the reason.
    Opaque(String),
}

/// A `monitor=` line or a `monitorv2` block.
#[derive(Debug, Clone)]
struct Statement {
    /// The statement's bytes (a piece of the block body).
    span: Range<usize>,
    /// The physical lines it covers.
    full: Range<usize>,
    /// 1-based first line.
    line: usize,
    /// Inside a `# hyprlang if`.
    conditional: bool,
    parsed: Parsed,
}

/// A file split into logical lines and statements, with its block.
struct Analysis {
    logicals: Vec<Logical>,
    /// Category depth and `if` depth at the start of each logical line.
    depths: Vec<(usize, usize)>,
    dangling: bool,
    location: Option<BlockLocation>,
    items: Vec<Item>,
    doc: ConfigDocument,
}

fn analyze(src: &str) -> Result<Analysis, ConfigError> {
    let (logicals, dangling) = scan::logical_lines(src);
    let (statements, depths) = statements(&logicals);
    let location = block::locate(src, &HYPRLANG_MARKERS)?;
    if let Some(loc) = &location {
        check_markers(&logicals, &depths, loc)?;
    }
    let in_block = |s: &Statement| {
        location
            .as_ref()
            .is_some_and(|l| l.body.contains(&s.span.start))
    };
    let mut outside = Vec::new();
    let mut inside = Vec::new();
    for s in statements {
        if in_block(&s) {
            inside.push(s);
            continue;
        }
        if let Some(loc) = &location
            && s.full.start < loc.span.end
            && s.full.end > loc.span.start
        {
            return Err(ConfigError::Syntax {
                line: s.line,
                message: format!(
                    "a statement crosses the managed block marker on line {}",
                    loc.begin_line
                ),
            });
        }
        outside.push(found_rule(src, &s));
    }
    let (block, items) = match &location {
        Some(loc) => {
            let (b, items) = block_contents(src, &logicals, loc, inside)?;
            (Some(b), items)
        }
        None => (None, Vec::new()),
    };
    Ok(Analysis {
        logicals,
        depths,
        dangling,
        location,
        items,
        doc: ConfigDocument { block, outside },
    })
}

/// Find the monitor statements and the depths at each logical line.
fn statements(logicals: &[Logical]) -> (Vec<Statement>, Vec<(usize, usize)>) {
    let mut out = Vec::new();
    let mut depths = Vec::with_capacity(logicals.len());
    let (mut depth, mut conditional) = (0usize, 0usize);
    let mut i = 0;
    while i < logicals.len() {
        depths.push((depth, conditional));
        let l = &logicals[i];
        match &l.kind {
            LineKind::If => conditional += 1,
            LineKind::EndIf => conditional = conditional.saturating_sub(1),
            LineKind::Close => depth = depth.saturating_sub(1),
            LineKind::Open(name) if name == "monitorv2" => {
                // The matching `}`, counting nested blocks (which
                // `parse_v2` refuses).
                let mut level = 0usize;
                let mut end = None;
                for (j, n) in logicals.iter().enumerate().skip(i + 1) {
                    match n.kind {
                        LineKind::Open(_) => level += 1,
                        LineKind::Close if level == 0 => {
                            end = Some(j);
                            break;
                        }
                        LineKind::Close => level -= 1,
                        _ => {}
                    }
                }
                let last = end.unwrap_or(logicals.len() - 1);
                let parsed = match end {
                    Some(e) => parse_v2(&logicals[i + 1..e]),
                    None => Parsed::Opaque("the monitorv2 block is not closed".to_owned()),
                };
                for _ in i + 1..=last {
                    depths.push((depth + 1, conditional));
                }
                out.push(Statement {
                    span: l.stmt.start..logicals[last].stmt.end,
                    full: l.full.start..logicals[last].full.end,
                    line: l.line,
                    conditional: conditional > 0,
                    parsed,
                });
                i = last + 1;
                continue;
            }
            LineKind::Open(name) => {
                if name.starts_with("monitorv2") {
                    out.push(opaque_statement(
                        l,
                        conditional,
                        "keyed monitorv2 blocks are not supported",
                    ));
                }
                depth += 1;
            }
            LineKind::Assign { lhs, rhs } if lhs == "monitor" => {
                out.push(Statement {
                    span: l.stmt.clone(),
                    full: l.full.clone(),
                    line: l.line,
                    conditional: conditional > 0,
                    parsed: parse_monitor(rhs),
                });
            }
            LineKind::Assign { lhs, .. } if lhs.starts_with("monitorv2[") => {
                out.push(opaque_statement(
                    l,
                    conditional,
                    "inline monitorv2 settings are not supported",
                ));
            }
            _ => {}
        }
        i += 1;
    }
    (out, depths)
}

fn opaque_statement(l: &Logical, conditional: usize, reason: &str) -> Statement {
    Statement {
        span: l.stmt.clone(),
        full: l.full.clone(),
        line: l.line,
        conditional: conditional > 0,
        parsed: Parsed::Opaque(reason.to_owned()),
    }
}

/// Split a `monitor=` value the way hyprutils' `CVarList2` does after
/// hyprlang's escape pass: on commas not preceded by a backslash, trimmed,
/// empty fields kept, no empty field after a trailing comma.
fn split_args(value: &str) -> Vec<String> {
    let value = value.replace("\\\\", "\\");
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                chars.next();
                current.push(',');
            }
            ',' => args.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() || args.is_empty() {
        args.push(current);
    }
    args.into_iter().map(|a| a.trim().to_owned()).collect()
}

fn parse_monitor(value: &str) -> Parsed {
    if value.contains('$') || value.contains("{{") {
        return Parsed::Opaque("uses hyprlang variables or expressions".to_owned());
    }
    let args = split_args(value);
    let selector = args[0].clone();
    match args.get(1).map(String::as_str) {
        Some("disable" | "disabled") => {
            let mut rule = MonitorRule::new(selector);
            rule.disabled = Some(true);
            return Parsed::Rule(Box::new(rule));
        }
        Some(form @ ("addreserved" | "transform")) => {
            return Parsed::Opaque(format!(
                "the `{form}` form patches an earlier rule for `{selector}`"
            ));
        }
        _ => {}
    }
    match line_rule(selector, &args) {
        Ok(rule) => Parsed::Rule(Box::new(rule)),
        Err(e) => Parsed::Opaque(e),
    }
}

fn line_rule(selector: String, args: &[String]) -> Result<MonitorRule, String> {
    let arg = |i: usize| args.get(i).map_or("", String::as_str);
    let mut rule = MonitorRule::new(selector);
    rule.mode = Some(arg(1).parse::<Mode>().map_err(|e| e.to_string())?);
    rule.position = Some(arg(2).parse::<Position>().map_err(|e| e.to_string())?);
    rule.scale = Some(Scale::parse_hyprland(arg(3)).map_err(|e| e.to_string())?);
    let mut seen: Vec<&str> = Vec::new();
    let mut i = 4;
    while i < args.len() {
        let key = args[i].as_str();
        if key.is_empty() {
            return Err(format!(
                "field {} is empty, which makes Hyprland ignore the rest of the line",
                i + 1
            ));
        }
        if !LINE_KEYWORDS.contains(&key) {
            return Err(format!(
                "unknown keyword `{key}`, which makes Hyprland ignore the whole line"
            ));
        }
        if seen.contains(&key) {
            return Err(format!("the keyword `{key}` appears twice"));
        }
        seen.push(key);
        set_field(&mut rule, key, arg(i + 1))?;
        i += 2;
    }
    Ok(rule)
}

/// Set a field from its hyprlang text; the parsers are strict where
/// Hyprland would silently fall back.
fn set_field(rule: &mut MonitorRule, key: &str, value: &str) -> Result<(), String> {
    let int = |min: i64, max: i64| -> Result<i64, String> {
        let v =
            int_value(value).ok_or_else(|| format!("`{key}` must be an integer, not {value:?}"))?;
        if (min..=max).contains(&v) {
            Ok(v)
        } else {
            Err(format!("`{key}` must be from {min} to {max}, not {v}"))
        }
    };
    let float = || -> Result<f64, String> {
        value
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .ok_or_else(|| format!("`{key}` must be a number, not {value:?}"))
    };
    let any = (i64::MIN, i64::MAX);
    match key {
        "mode" => {
            rule.mode = Some(
                value
                    .parse()
                    .map_err(|e: crate::model::InvalidValue| e.to_string())?,
            );
        }
        "position" => {
            rule.position = Some(
                value
                    .parse()
                    .map_err(|e: crate::model::InvalidValue| e.to_string())?,
            );
        }
        "scale" => rule.scale = Some(Scale::parse_hyprland(value).map_err(|e| e.to_string())?),
        "transform" => {
            let t = value
                .parse::<u8>()
                .ok()
                .filter(|t| *t <= 7)
                .ok_or_else(|| {
                    format!("`transform` must be an integer from 0 to 7, not {value:?}")
                })?;
            rule.transform = Some(Transform::new(t).map_err(|e| e.to_string())?);
        }
        "disabled" => {
            rule.disabled = Some(match int_value(value) {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(format!("`disabled` must be 0 or 1, not {value:?}")),
            });
        }
        "vrr" => rule.vrr = Some(int(-1, 3)? as i8),
        "mirror" => rule.mirror = Some(value.to_owned()),
        "bitdepth" => rule.bitdepth = Some(int(any.0, any.1)?),
        "cm" => {
            rule.cm = Some(
                value
                    .parse::<ColorManagement>()
                    .map_err(|e| e.to_string())?,
            );
        }
        "sdr_eotf" => rule.sdr_eotf = Some(sdr_eotf(value)?),
        "sdrbrightness" => rule.sdrbrightness = Some(float()?),
        "sdrsaturation" => rule.sdrsaturation = Some(float()?),
        "sdr_min_luminance" => rule.sdr_min_luminance = Some(float()?),
        "min_luminance" => rule.min_luminance = Some(float()?),
        "supports_wide_color" => rule.supports_wide_color = Some(int(-1, 1)? as i8),
        "supports_hdr" => rule.supports_hdr = Some(int(-1, 1)? as i8),
        "sdr_max_luminance" => rule.sdr_max_luminance = Some(int(any.0, any.1)?),
        "max_luminance" => rule.max_luminance = Some(int(any.0, any.1)?),
        "max_avg_luminance" => rule.max_avg_luminance = Some(int(any.0, any.1)?),
        "addreserved" => rule.reserved = Some(reserved(value)?),
        "icc" => {
            if value.is_empty() {
                return Err("`icc` must not be empty".to_owned());
            }
            rule.icc = Some(value.to_owned());
        }
        other => return Err(format!("unknown monitorv2 field `{other}`")),
    }
    Ok(())
}

/// An integer as hyprlang's `configStringToInt` reads plain values: a
/// decimal number, or a boolean word.
fn int_value(value: &str) -> Option<i64> {
    match value {
        "true" | "yes" | "on" => Some(1),
        "false" | "no" | "off" => Some(0),
        v => v.parse().ok(),
    }
}

/// hyprlang's `sdr_eotf` digits (0 auto, 1 srgb, 2 gamma22) and names.
fn sdr_eotf(value: &str) -> Result<String, String> {
    let name = match value {
        "0" => "auto",
        "1" | "3" => "srgb",
        "2" => "gamma22",
        other => other,
    };
    if SDR_EOTF_NAMES.contains(&name) {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "invalid sdr_eotf: {value:?} (use {})",
            SDR_EOTF_NAMES.join(", ")
        ))
    }
}

/// `addreserved = TOP, BOTTOM, LEFT, RIGHT`.
fn reserved(value: &str) -> Result<Reserved, String> {
    let sides: Vec<i32> = value
        .split(',')
        .map(|s| s.trim().parse::<i32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("`addreserved` must be four integers, not {value:?}"))?;
    let [top, bottom, left, right] = sides[..] else {
        return Err(format!(
            "`addreserved` must be four integers, not {value:?}"
        ));
    };
    Ok(Reserved {
        top,
        right,
        bottom,
        left,
    })
}

fn parse_v2(lines: &[Logical]) -> Parsed {
    let mut rule: Option<MonitorRule> = None;
    let mut seen: Vec<String> = Vec::new();
    for l in lines {
        match &l.kind {
            LineKind::Blank | LineKind::Comment => {}
            LineKind::Assign { lhs, rhs } => {
                if rhs.contains('$') || rhs.contains("{{") {
                    return Parsed::Opaque("uses hyprlang variables or expressions".to_owned());
                }
                if seen.contains(lhs) {
                    return Parsed::Opaque(format!("the field `{lhs}` appears twice"));
                }
                seen.push(lhs.clone());
                match &mut rule {
                    None if lhs == "output" => rule = Some(MonitorRule::new(rhs.as_str())),
                    None => {
                        return Parsed::Opaque(
                            "the first field of a monitorv2 block must be `output`".to_owned(),
                        );
                    }
                    Some(r) => {
                        if let Err(e) = set_field(r, lhs, rhs) {
                            return Parsed::Opaque(e);
                        }
                    }
                }
            }
            _ => {
                return Parsed::Opaque(format!(
                    "line {}: only `key = value` lines are allowed in a monitorv2 block",
                    l.line
                ));
            }
        }
    }
    match rule {
        Some(r) => Parsed::Rule(Box::new(r)),
        None => Parsed::Opaque("the monitorv2 block has no `output`".to_owned()),
    }
}

fn found_rule(src: &str, s: &Statement) -> FoundRule {
    let (rule, mut reason) = match &s.parsed {
        Parsed::Rule(r) => (Some(r.as_ref().clone()), None),
        Parsed::Opaque(e) => (None, Some(e.clone())),
    };
    if reason.is_none() && s.conditional {
        reason = Some("inside a `# hyprlang if` block".to_owned());
    }
    FoundRule {
        rule,
        text: src[s.span.clone()].to_owned(),
        span: s.full.clone(),
        line: s.line,
        not_adoptable: reason,
    }
}

/// A marker must be a comment line of its own, not joined to the line
/// before it, and the block must be at the top level.
fn check_markers(
    logicals: &[Logical],
    depths: &[(usize, usize)],
    loc: &BlockLocation,
) -> Result<(), ConfigError> {
    for (line, what) in [(loc.begin_line, "begin"), (loc.end_line, "end")] {
        let standalone = logicals
            .iter()
            .position(|l| l.line == line && !l.joined && l.kind == LineKind::Comment);
        let Some(index) = standalone else {
            return Err(ConfigError::Syntax {
                line,
                message: format!(
                    "the {what} marker is joined to the line before it by a trailing backslash"
                ),
            });
        };
        if what == "begin" && depths[index].0 > 0 {
            return Err(ConfigError::Syntax {
                line,
                message: "the managed block is inside a category block".to_owned(),
            });
        }
    }
    Ok(())
}

fn block_contents(
    src: &str,
    logicals: &[Logical],
    loc: &BlockLocation,
    inside: Vec<Statement>,
) -> Result<(ManagedBlock, Vec<Item>), ConfigError> {
    let unsupported =
        |line: usize, message: String| ConfigError::UnsupportedInBlock { line, message };
    let mut pieces = Vec::new();
    for s in inside {
        if s.full.end > loc.body.end {
            return Err(unsupported(
                s.line,
                "the statement crosses the end marker".to_owned(),
            ));
        }
        match s.parsed {
            Parsed::Rule(rule) => pieces.push(Piece {
                span: s.span,
                rule: Some(*rule),
            }),
            Parsed::Opaque(reason) => return Err(unsupported(s.line, reason)),
        }
    }
    for l in logicals.iter().filter(|l| loc.body.contains(&l.full.start)) {
        let covered = pieces.iter().any(|p| p.span.contains(&l.stmt.start));
        match &l.kind {
            LineKind::Blank => {}
            LineKind::Comment => pieces.push(Piece {
                span: l.stmt.clone(),
                rule: None,
            }),
            _ if covered => {}
            LineKind::If | LineKind::EndIf | LineKind::Directive => {
                return Err(unsupported(
                    l.line,
                    "hyprlang directives are not allowed".to_owned(),
                ));
            }
            _ => {
                return Err(unsupported(
                    l.line,
                    format!(
                        "only monitor rules are allowed, found `{}`",
                        src[l.stmt.clone()].trim()
                    ),
                ));
            }
        }
    }
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

/// Whether a new block may start at `offset`: the start of a logical line
/// (or the end of the file) at the top level, outside `# hyprlang if`.
fn is_insertion_point(a: &Analysis, offset: usize) -> bool {
    if let Some(i) = a.logicals.iter().position(|l| l.full.start == offset) {
        return a.depths[i] == (0, 0);
    }
    let at_end = a.logicals.last().is_none_or(|l| l.full.end == offset);
    at_end && !a.dangling && depth_at_end(a) == (0, 0)
}

/// Category and `if` depth after the last logical line.
fn depth_at_end(a: &Analysis) -> (usize, usize) {
    let (Some(last), Some(&(mut depth, mut conditional))) = (a.logicals.last(), a.depths.last())
    else {
        return (0, 0);
    };
    match &last.kind {
        LineKind::Open(_) => depth += 1,
        LineKind::Close => depth = depth.saturating_sub(1),
        LineKind::If => conditional += 1,
        LineKind::EndIf => conditional = conditional.saturating_sub(1),
        _ => {}
    }
    (depth, conditional)
}

/// Where a new block goes: after the last monitor rule outside it, or at
/// the end of the file.
fn new_block_position(src: &str, a: &Analysis) -> Result<usize, ConfigError> {
    if let Some(last) = a.doc.outside.last() {
        let candidate = a
            .logicals
            .iter()
            .map(|l| l.full.start)
            .chain(std::iter::once(src.len()))
            .find(|&o| o >= last.span.end && is_insertion_point(a, o));
        if let Some(offset) = candidate {
            return Ok(offset);
        }
    }
    if a.dangling {
        return Err(ConfigError::Syntax {
            line: block::line_of(src, src.len()),
            message: "the last line ends with a backslash, which would swallow the managed block"
                .to_owned(),
        });
    }
    if is_insertion_point(a, src.len()) {
        return Ok(src.len());
    }
    Err(ConfigError::Syntax {
        line: block::line_of(src, src.len()),
        message: "the file ends inside a category or `# hyprlang if` block".to_owned(),
    })
}

fn verify(content: &str, rules: &[MonitorRule]) -> Result<(), ConfigError> {
    let doc = analyze(content)
        .map_err(|e| ConfigError::Unrepresentable {
            output: String::new(),
            message: format!(
                "the new content does not read back: {e} (this is a bug in hyprtilt; nothing was written)"
            ),
        })?
        .doc;
    adopt::verify(doc.block_rules(), rules, &canonical)
}

#[cfg(test)]
mod tests;
