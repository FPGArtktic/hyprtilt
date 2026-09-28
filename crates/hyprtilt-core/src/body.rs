// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Rewriting the body of a managed block with minimal changes.
//!
//! A backend splits the body into pieces: rule statements (with the rule
//! they define) and comments. This module groups the pieces into items of
//! whole lines and rebuilds the body for a new list of rules:
//!
//! - a rule whose value did not change keeps its text byte for byte,
//!   including the user's column alignment and trailing comment;
//! - a changed rule is regenerated in place, keeping the indentation and a
//!   trailing comment of its line;
//! - a removed rule is dropped together with the comment lines directly
//!   above it;
//! - a new rule is appended at the end of the body.
//!
//! Rules are matched by their selector string, which is how Hyprland
//! identifies a rule.

use std::ops::Range;

use crate::block;
use crate::document::ConfigError;
use crate::model::MonitorRule;

/// A statement or comment inside a block body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    /// Byte range in the file, without surrounding whitespace.
    pub(crate) span: Range<usize>,
    /// The rule the statement defines; `None` for a comment.
    pub(crate) rule: Option<MonitorRule>,
}

/// What an item of the body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemKind {
    /// An empty or whitespace-only line.
    Blank,
    /// Lines holding only comments.
    Comment,
    /// Lines holding one or more rule statements.
    Rules,
}

/// A run of whole lines of the body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    /// Byte range of the lines, line endings included.
    pub(crate) span: Range<usize>,
    /// What the lines hold.
    pub(crate) kind: ItemKind,
    /// The rules defined in the lines, in order.
    pub(crate) rules: Vec<MonitorRule>,
    /// 1-based line of each rule.
    pub(crate) rule_lines: Vec<usize>,
    /// Indentation of the first line.
    pub(crate) prefix: String,
    /// Text after the last rule statement up to the end of the last line
    /// (a trailing comment), without the line ending.
    pub(crate) suffix: String,
}

/// Group the pieces of the body `body` of `src` into items. Every line of
/// the body belongs to exactly one item.
///
/// # Errors
///
/// Returns [`ConfigError::UnsupportedInBlock`] if a line holds text that is
/// not covered by any piece, which means the backend did not recognise it.
pub(crate) fn items(
    src: &str,
    body: &Range<usize>,
    pieces: &[Piece],
) -> Result<Vec<Item>, ConfigError> {
    let lines: Vec<block::Line> = block::lines(src)
        .into_iter()
        .filter(|l| l.full.start >= body.start && l.full.end <= body.end)
        .collect();
    let line_index = |offset: usize| -> usize {
        lines
            .iter()
            .position(|l| offset < l.full.end)
            .unwrap_or(lines.len().saturating_sub(1))
    };

    // Groups of consecutive lines touched by pieces: (first, last, pieces).
    let mut groups: Vec<(usize, usize, Vec<&Piece>)> = Vec::new();
    for piece in pieces {
        let first = line_index(piece.span.start);
        let last = line_index(piece.span.end.saturating_sub(1).max(piece.span.start));
        match groups.last_mut() {
            Some(group) if first <= group.1 => {
                group.1 = group.1.max(last);
                group.2.push(piece);
            }
            _ => groups.push((first, last, vec![piece])),
        }
    }

    let mut out = Vec::new();
    let mut next_line = 0;
    for (first, last, group) in groups {
        for line in &lines[next_line..first] {
            out.push(blank_item(src, line)?);
        }
        out.push(group_item(src, &lines[first..=last], &group));
        next_line = last + 1;
    }
    for line in &lines[next_line..] {
        out.push(blank_item(src, line)?);
    }
    Ok(out)
}

fn blank_item(src: &str, line: &block::Line) -> Result<Item, ConfigError> {
    let text = &src[line.content.clone()];
    if !text.trim().is_empty() {
        return Err(ConfigError::UnsupportedInBlock {
            line: line.number,
            message: text.trim().to_owned(),
        });
    }
    Ok(Item {
        span: line.full.clone(),
        kind: ItemKind::Blank,
        rules: Vec::new(),
        rule_lines: Vec::new(),
        prefix: String::new(),
        suffix: String::new(),
    })
}

fn group_item(src: &str, lines: &[block::Line], pieces: &[&Piece]) -> Item {
    let first = &lines[0];
    let last = &lines[lines.len() - 1];
    let rules: Vec<MonitorRule> = pieces.iter().filter_map(|p| p.rule.clone()).collect();
    let rule_lines = pieces
        .iter()
        .filter(|p| p.rule.is_some())
        .map(|p| block::line_of(src, p.span.start))
        .collect();
    let indent_end = src[first.content.clone()]
        .find(|c: char| !c.is_whitespace())
        .map_or(first.content.end, |i| first.content.start + i);
    let suffix = pieces
        .iter()
        .rev()
        .find(|p| p.rule.is_some())
        .map(|p| src[p.span.end.min(last.content.end)..last.content.end].to_owned())
        .unwrap_or_default();
    Item {
        span: first.full.start..last.full.end,
        kind: if rules.is_empty() {
            ItemKind::Comment
        } else {
            ItemKind::Rules
        },
        rules,
        rule_lines,
        prefix: src[first.content.start..indent_end].to_owned(),
        suffix,
    }
}

/// Rebuild the body from `items` for the new list of `rules`. `format`
/// produces the text of one rule without indentation or line ending; `eol`
/// is the line ending of the file.
///
/// # Errors
///
/// Returns the error of `format` for a rule the backend cannot write.
pub(crate) fn rewrite(
    src: &str,
    items: &[Item],
    rules: &[MonitorRule],
    format: &dyn Fn(&MonitorRule) -> Result<String, ConfigError>,
    eol: &str,
) -> Result<String, ConfigError> {
    // Assign every new rule to the first unused old rule with the same
    // selector: slot (item, position in item) -> index into `rules`.
    let mut slots: Vec<Vec<Option<usize>>> =
        items.iter().map(|i| vec![None; i.rules.len()]).collect();
    let mut owner: Vec<Option<usize>> = vec![None; rules.len()];
    for (n, rule) in rules.iter().enumerate() {
        let slot = items.iter().enumerate().find_map(|(i, item)| {
            item.rules
                .iter()
                .enumerate()
                .position(|(k, old)| old.output == rule.output && slots[i][k].is_none())
                .map(|k| (i, k))
        });
        if let Some((i, k)) = slot {
            slots[i][k] = Some(n);
            owner[n] = Some(i);
        }
    }
    // A new rule goes before the item of the next existing rule in the
    // requested order, so that the order of evaluation is kept; without one
    // it is appended.
    let mut before: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    let mut appended = Vec::new();
    for n in (0..rules.len()).filter(|&n| owner[n].is_none()) {
        match owner[n + 1..].iter().flatten().next() {
            Some(&i) => before[i].push(n),
            None => appended.push(n),
        }
    }

    let mut out: Vec<(ItemKind, String)> = Vec::new();
    let mut indent = String::new();
    for (i, (item, slot)) in items.iter().zip(&slots).enumerate() {
        let text = &src[item.span.clone()];
        if item.kind != ItemKind::Rules {
            out.push((item.kind, text.to_owned()));
            continue;
        }
        indent.clone_from(&item.prefix);
        let kept: Vec<usize> = slot.iter().flatten().copied().collect();
        if kept.is_empty() {
            // Drop the comment lines directly above the removed rules.
            while out
                .last()
                .is_some_and(|(kind, _)| *kind == ItemKind::Comment)
            {
                out.pop();
            }
        }
        // New rules go above the comments that belong to this item.
        let comments = out
            .iter()
            .rev()
            .take_while(|(kind, _)| *kind == ItemKind::Comment)
            .count();
        let at = out.len() - comments;
        for (offset, &n) in before[i].iter().enumerate() {
            let text = format!("{}{}{eol}", item.prefix, format(&rules[n])?);
            out.insert(at + offset, (ItemKind::Rules, text));
        }
        if kept.is_empty() {
            continue;
        }
        let unchanged = slot
            .iter()
            .zip(&item.rules)
            .all(|(n, old)| n.is_some_and(|n| rules[n] == *old));
        if unchanged {
            out.push((ItemKind::Rules, text.to_owned()));
            continue;
        }
        let mut regenerated = String::new();
        for (k, n) in kept.iter().enumerate() {
            regenerated.push_str(&item.prefix);
            regenerated.push_str(&format(&rules[*n])?);
            if k + 1 == kept.len() {
                regenerated.push_str(&item.suffix);
            }
            regenerated.push_str(eol);
        }
        out.push((ItemKind::Rules, regenerated));
    }
    for n in appended {
        let text = format!("{indent}{}{eol}", format(&rules[n])?);
        out.push((ItemKind::Rules, text));
    }
    Ok(out.into_iter().map(|(_, text)| text).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy backend: one rule per line, `NAME=VALUE` sets the mode text.
    fn toy_pieces(src: &str, body: &Range<usize>) -> Vec<Piece> {
        let mut pieces = Vec::new();
        for line in block::lines(src) {
            if line.full.start < body.start || line.full.end > body.end {
                continue;
            }
            let text = &src[line.content.clone()];
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let start = line.content.start + text.find(trimmed).unwrap_or(0);
            if trimmed.starts_with('#') {
                pieces.push(Piece {
                    span: start..start + trimmed.len(),
                    rule: None,
                });
                continue;
            }
            let stmt = trimmed.split(" #").next().unwrap_or(trimmed).trim_end();
            let (name, mode) = stmt.split_once('=').unwrap();
            let mut rule = MonitorRule::new(name);
            rule.mode = Some(mode.parse().unwrap());
            pieces.push(Piece {
                span: start..start + stmt.len(),
                rule: Some(rule),
            });
        }
        pieces
    }

    fn toy_text(rule: &MonitorRule) -> String {
        let mode = rule.mode.as_ref().map(ToString::to_string);
        format!("{}={}", rule.output, mode.unwrap_or_default())
    }

    /// The formatter callback in the shape `rewrite` expects.
    const TOY_FORMAT: &dyn Fn(&MonitorRule) -> Result<String, ConfigError> = &|r| Ok(toy_text(r));

    fn rule(name: &str, mode: &str) -> MonitorRule {
        let mut r = MonitorRule::new(name);
        r.mode = Some(mode.parse().unwrap());
        r
    }

    fn run(src: &str, rules: &[MonitorRule]) -> String {
        let body = 0..src.len();
        let pieces = toy_pieces(src, &body);
        let items = items(src, &body, &pieces).unwrap();
        rewrite(src, &items, rules, TOY_FORMAT, "\n").unwrap()
    }

    const SRC: &str = "# laptop\nA=1920x1080@60   # aligned\n\n# desk\nB=2560x1440@144\n";

    #[test]
    fn unchanged_rules_keep_their_text() {
        let rules = [rule("A", "1920x1080@60"), rule("B", "2560x1440@144")];
        assert_eq!(run(SRC, &rules), SRC);
    }

    #[test]
    fn changed_rule_keeps_indent_and_trailing_comment() {
        let src = "  A=1920x1080@60   # aligned\n";
        let out = run(src, &[rule("A", "1280x720@60")]);
        assert_eq!(out, "  A=1280x720@60   # aligned\n");
    }

    #[test]
    fn removed_rule_takes_its_comment_along() {
        let out = run(SRC, &[rule("A", "1920x1080@60")]);
        assert_eq!(out, "# laptop\nA=1920x1080@60   # aligned\n\n");
        // A blank line protects a comment that is not directly above.
        let out = run("# keep\n\nA=1x1@60\n", &[]);
        assert_eq!(out, "# keep\n\n");
    }

    #[test]
    fn new_rules_are_appended_with_the_last_indent() {
        let src = "    A=1x1@60\n";
        let out = run(src, &[rule("A", "1x1@60"), rule("C", "2x2@60")]);
        assert_eq!(out, "    A=1x1@60\n    C=2x2@60\n");
        assert_eq!(run("", &[rule("C", "2x2@60")]), "C=2x2@60\n");
    }

    #[test]
    fn new_rules_keep_the_requested_order() {
        let src = "# a\nA=1x1@60\nB=1x1@60\n";
        let out = run(
            src,
            &[
                rule("Z", "2x2@60"),
                rule("A", "1x1@60"),
                rule("Y", "3x3@60"),
                rule("B", "1x1@60"),
            ],
        );
        assert_eq!(out, "Z=2x2@60\n# a\nA=1x1@60\nY=3x3@60\nB=1x1@60\n");
        // Before a removed item, the new rule takes its place.
        let out = run(
            "# a\nA=1x1@60\nB=1x1@60\n",
            &[rule("Z", "2x2@60"), rule("B", "1x1@60")],
        );
        assert_eq!(out, "Z=2x2@60\nB=1x1@60\n");
    }

    #[test]
    fn duplicate_selectors_collapse_to_the_first() {
        let src = "A=1x1@60\nA=2x2@60\n";
        let out = run(src, &[rule("A", "1x1@60")]);
        assert_eq!(out, "A=1x1@60\n");
    }

    #[test]
    fn items_cover_every_line() {
        let body = 0..SRC.len();
        let pieces = toy_pieces(SRC, &body);
        let items = items(SRC, &body, &pieces).unwrap();
        let kinds: Vec<_> = items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            [
                ItemKind::Comment,
                ItemKind::Rules,
                ItemKind::Blank,
                ItemKind::Comment,
                ItemKind::Rules
            ]
        );
        assert_eq!(items[1].suffix, "   # aligned");
        assert_eq!(items[1].rule_lines, [2]);
        let joined: String = items.iter().map(|i| &SRC[i.span.clone()]).collect();
        assert_eq!(joined, SRC);
    }

    #[test]
    fn uncovered_text_is_refused() {
        let src = "junk\n";
        let err = items(src, &(0..src.len()), &[]).unwrap_err();
        assert_eq!(
            err,
            ConfigError::UnsupportedInBlock {
                line: 1,
                message: "junk".to_owned()
            }
        );
    }

    #[test]
    fn pieces_sharing_lines_form_one_item() {
        let src = "A=1x1@60\n";
        let pieces = vec![
            Piece {
                span: 0..3,
                rule: Some(rule("A", "1x1@60")),
            },
            Piece {
                span: 4..8,
                rule: Some(rule("B", "1x1@60")),
            },
        ];
        let items = items(src, &(0..src.len()), &pieces).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rules.len(), 2);
        // Changing one regenerates both, one per line.
        let out = rewrite(
            src,
            &items,
            &[rule("A", "1x1@60"), rule("B", "2x2@60")],
            TOY_FORMAT,
            "\n",
        )
        .unwrap();
        assert_eq!(out, "A=1x1@60\nB=2x2@60\n");
    }
}
