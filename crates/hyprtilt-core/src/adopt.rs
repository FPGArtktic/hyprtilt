// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Language-independent parts of editing a file: choosing the rules to
//! adopt, taking them out of the file, and merging rules the way Hyprland
//! evaluates them.

use crate::document::{ConfigError, FoundRule};
use crate::model::{MonitorRule, SelectorKind};

/// How a later rule for the same selector combines with an earlier one:
/// field by field in Lua, wholesale in hyprlang.
pub(crate) type Merge = fn(&mut MonitorRule, &MonitorRule);

/// Lua semantics: the later call replaces only the fields it writes.
pub(crate) fn overlay(earlier: &mut MonitorRule, later: &MonitorRule) {
    earlier.overlay(later);
}

/// hyprlang semantics: the later `monitor=` line replaces the rule.
pub(crate) fn replace(earlier: &mut MonitorRule, later: &MonitorRule) {
    earlier.clone_from(later);
}

/// The rules to adopt: every adoptable rule when `lines` is empty,
/// otherwise the rules starting on `lines`, sorted by position.
///
/// # Errors
///
/// Returns [`ConfigError::NotAdoptable`] for a line without a rule or with
/// a rule that cannot be adopted.
pub(crate) fn select<'d>(
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

/// Refuse to move rules across a monitor rule that stays outside the block
/// and may concern the same monitor: its place in the evaluation order
/// relative to them would change. `block_start` is where an existing block
/// begins.
///
/// # Errors
///
/// Returns [`ConfigError::AdoptCrossing`] naming the rule in the way.
pub(crate) fn check_crossing(
    outside: &[FoundRule],
    selected: &[&FoundRule],
    block_start: Option<usize>,
) -> Result<(), ConfigError> {
    let positions = selected.iter().map(|f| f.span.start).chain(block_start);
    let (Some(low), Some(high)) = (positions.clone().min(), positions.max()) else {
        return Ok(());
    };
    for f in outside {
        let between = f.span.start > low && f.span.start < high;
        let chosen = selected.iter().any(|s| s.span == f.span);
        if between && !chosen && selected.iter().any(|s| may_interact(f, s)) {
            return Err(ConfigError::AdoptCrossing { line: f.line });
        }
    }
    Ok(())
}

/// Whether two rules may apply to the same monitor. Without a literal
/// selector, or with a `desc:` selector on either side, it cannot be ruled
/// out.
fn may_interact(a: &FoundRule, b: &FoundRule) -> bool {
    let (Some(x), Some(y)) = (&a.rule, &b.rule) else {
        return true;
    };
    let by_description = |r: &MonitorRule| matches!(r.output.kind(), SelectorKind::Description(_));
    x.output == y.output || by_description(x) || by_description(y)
}

/// `src` without the removal spans of `rules`, which are sorted and do not
/// overlap.
pub(crate) fn remove_spans(src: &str, rules: &[&FoundRule]) -> String {
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for f in rules {
        out.push_str(&src[at..f.span.start]);
        at = f.span.end;
    }
    out.push_str(&src[at..]);
    out
}

/// Merge rules with the same selector the way Hyprland's rule list does:
/// the merged rule takes the position of the later one.
pub(crate) fn merge_in_order(
    rules: impl IntoIterator<Item = MonitorRule>,
    merge: Merge,
) -> Vec<MonitorRule> {
    let mut out: Vec<MonitorRule> = Vec::new();
    for rule in rules {
        match out.iter().position(|r| r.output == rule.output) {
            Some(i) => {
                let mut merged = out.remove(i);
                merge(&mut merged, &rule);
                out.push(merged);
            }
            None => out.push(rule),
        }
    }
    out
}

/// The block's rules after adopting `selected`: a rule before the block
/// (`block_start`) is overridden by the block's rule for the same selector,
/// a rule after it overrides the block's rule. New selectors from before
/// the block go first, those from after it last.
pub(crate) fn merge_into_block(
    block_rules: &[MonitorRule],
    selected: &[&FoundRule],
    block_start: usize,
    merge: Merge,
) -> Vec<MonitorRule> {
    let mut rules = merge_in_order(block_rules.iter().cloned(), merge);
    let mut earlier = Vec::new();
    for found in selected {
        let Some(rule) = found.rule.clone() else {
            continue;
        };
        let existing = rules.iter_mut().find(|b| b.output == rule.output);
        match (found.span.end <= block_start, existing) {
            (true, Some(block_rule)) => {
                let mut merged = rule;
                merge(&mut merged, block_rule);
                *block_rule = merged;
            }
            (true, None) => earlier.push(rule),
            (false, Some(block_rule)) => merge(block_rule, &rule),
            (false, None) => rules.push(rule),
        }
    }
    let mut all = merge_in_order(earlier, merge);
    all.extend(rules);
    all
}

/// Refuse two rules for the same selector: Hyprland would merge them, and
/// the block must say exactly one thing per output.
///
/// # Errors
///
/// Returns [`ConfigError::Unrepresentable`] for the second rule.
pub(crate) fn check_unique(rules: &[MonitorRule]) -> Result<(), ConfigError> {
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

/// Compare the rules read back from a new block with the requested ones.
/// `expected` gives the value a requested rule reads back as.
///
/// # Errors
///
/// Returns [`ConfigError::Unrepresentable`] describing the difference.
pub(crate) fn verify(
    got: &[MonitorRule],
    rules: &[MonitorRule],
    expected: &dyn Fn(&MonitorRule) -> MonitorRule,
) -> Result<(), ConfigError> {
    let internal = |output: &str, what: &str| ConfigError::Unrepresentable {
        output: output.to_owned(),
        message: format!("{what} (this is a bug in hyprtilt; nothing was written)"),
    };
    if got.len() != rules.len() {
        return Err(internal("", "the block does not hold the requested rules"));
    }
    for rule in rules {
        let want = expected(rule);
        let ok = got
            .iter()
            .any(|g| g.output == rule.output && (*g == want || g == rule));
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
mod tests {
    use super::*;

    fn found(output: Option<&str>, start: usize, line: usize) -> FoundRule {
        FoundRule {
            rule: output.map(MonitorRule::new),
            text: String::new(),
            span: start..start + 1,
            line,
            not_adoptable: output.is_none().then(|| "uses variables".to_owned()),
        }
    }

    #[test]
    fn crossing_rules_that_may_interact_is_refused() {
        let outside = [
            found(Some("DP-1"), 0, 1),
            found(None, 10, 2),
            found(Some("HDMI-A-1"), 20, 3),
            found(Some("DP-1"), 30, 4),
        ];
        let all: Vec<&FoundRule> = vec![&outside[0], &outside[2], &outside[3]];
        assert_eq!(
            check_crossing(&outside, &all, None),
            Err(ConfigError::AdoptCrossing { line: 2 })
        );
        // The rules after the unknown one can move freely.
        assert_eq!(check_crossing(&outside, &all[1..], None), Ok(()));
        // A different connector does not interact.
        let pair = [&outside[0], &outside[3]];
        let without_unknown = [outside[0].clone(), outside[2].clone(), outside[3].clone()];
        assert_eq!(check_crossing(&without_unknown, &pair, None), Ok(()));
        // An existing block counts as a position.
        assert_eq!(
            check_crossing(&outside, &[&outside[0]], Some(15)),
            Err(ConfigError::AdoptCrossing { line: 2 })
        );
        assert_eq!(check_crossing(&outside, &[], None), Ok(()));
    }

    #[test]
    fn description_selectors_may_match_anything() {
        let a = found(Some("desc:Samsung"), 0, 1);
        let b = found(Some("DP-1"), 5, 2);
        assert!(may_interact(&a, &b));
        assert!(!may_interact(&b, &found(Some("DP-2"), 0, 1)));
    }

    #[test]
    fn merging_follows_the_language() {
        let mut a = MonitorRule::new("DP-1");
        a.vrr = Some(1);
        let mut b = MonitorRule::new("DP-1");
        b.transform = Some(crate::model::Transform::new(1).unwrap());
        let lua = merge_in_order([a.clone(), b.clone()], overlay);
        assert_eq!((lua[0].vrr, lua[0].transform.is_some()), (Some(1), true));
        let hyprlang = merge_in_order([a, b.clone()], replace);
        assert_eq!(hyprlang, [b]);
    }

    #[test]
    fn verification_reports_differences() {
        let rule = MonitorRule::new("DP-1");
        let same = |r: &MonitorRule| r.clone();
        assert!(
            verify(
                std::slice::from_ref(&rule),
                std::slice::from_ref(&rule),
                &same
            )
            .is_ok()
        );
        assert!(verify(&[], std::slice::from_ref(&rule), &same).is_err());
        assert!(verify(&[MonitorRule::new("DP-2")], &[rule], &same).is_err());
    }
}
