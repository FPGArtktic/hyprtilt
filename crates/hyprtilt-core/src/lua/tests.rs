// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the Lua backend: byte preservation, idempotence, placement,
//! adoption and refusals.

use super::*;
use crate::model::{ColorManagement, ExtraField, Mode, Position, Reserved, Transform};

const HYPR_USER: &str = include_str!("../../tests/fixtures/lua/caelestia-hypr-user.lua");
const CAELESTIA_MAIN: &str = include_str!("../../tests/fixtures/lua/caelestia-hyprland.lua");

const HDMI: &str = r#"hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144",    position = "0x0",       scale = 1, transform = 1 })"#;
const EDP: &str = r#"hl.monitor({ output = "eDP-1",    mode = "1920x1080@144",    position = "1440x1335", scale = 1 })"#;
const DP: &str = r#"hl.monitor({ output = "DP-1",     mode = "2560x1440@179.95", position = "3360x975",  scale = 1, vrr = 2 })"#;

fn rule(output: &str, mode: &str, x: i32, y: i32) -> MonitorRule {
    let mut r = MonitorRule::new(output);
    r.mode = Some(mode.parse().unwrap());
    r.position = Some(Position::At { x, y });
    r.scale = Some(Scale::Factor(1.0));
    r
}

/// The maintainer's three rules, as the fixture writes them.
fn maintainer_rules() -> Vec<MonitorRule> {
    let mut hdmi = rule("HDMI-A-1", "2560x1440@144", 0, 0);
    hdmi.transform = Some(Transform::new(1).unwrap());
    let edp = rule("eDP-1", "1920x1080@144", 1440, 1335);
    let mut dp = rule("DP-1", "2560x1440@179.95", 3360, 975);
    dp.vrr = Some(2);
    vec![hdmi, edp, dp]
}

/// Check that everything outside the block of `new` is `old` byte for
/// byte, except for blank lines added around a new block.
fn assert_outside_preserved(old: &str, new: &str) {
    let loc = block::locate(new, &LUA_MARKERS).unwrap().unwrap();
    let before = &new[..loc.span.start];
    let after = &new[loc.span.end..];
    if let Some(old_loc) = block::locate(old, &LUA_MARKERS).unwrap() {
        assert_eq!(before, &old[..old_loc.span.start]);
        assert_eq!(after, &old[old_loc.span.end..]);
    } else {
        let b = before.trim_end_matches(['\n', '\r']);
        let a = after.trim_start_matches(['\n', '\r']);
        assert!(old.starts_with(b), "prefix changed");
        assert!(old.ends_with(a), "suffix changed");
        let middle = &old[b.len()..old.len() - a.len()];
        assert!(middle.trim().is_empty(), "text lost: {middle:?}");
    }
}

/// Compile the content with a real Lua compiler, when one is installed.
fn assert_compiles(content: &str) {
    if let Some(luac) = syntax::Luac::find(&std::env::var("PATH").unwrap_or_default()) {
        luac.check(content)
            .unwrap_or_else(|e| panic!("{e}\n{content}"));
    }
}

#[test]
fn fixture_rules_are_found_outside() {
    let doc = parse(HYPR_USER).unwrap();
    assert!(doc.block.is_none());
    let lines: Vec<usize> = doc.outside.iter().map(|f| f.line).collect();
    assert_eq!(lines, [5, 6, 7]);
    assert!(doc.outside.iter().all(FoundRule::is_adoptable));
    let rules: Vec<MonitorRule> = doc.outside.iter().filter_map(|f| f.rule.clone()).collect();
    assert_eq!(rules, maintainer_rules());
    assert_eq!(doc.outside[0].text, HDMI);
}

#[test]
fn adopt_replaces_the_rules_in_place_and_unmanage_undoes_it() {
    let edit = adopt(HYPR_USER, &[]).unwrap();
    assert!(edit.changed);
    let expected = HYPR_USER.replace(
        &format!("{HDMI}\n{EDP}\n{DP}\n"),
        &format!("-- BEGIN hyprtilt (managed)\n{HDMI}\n{EDP}\n{DP}\n-- END hyprtilt\n"),
    );
    assert_eq!(edit.content, expected);
    let doc = parse(&edit.content).unwrap();
    let block = doc.block.unwrap();
    assert_eq!(block.rules, maintainer_rules());
    assert_eq!(block.rule_lines, [6, 7, 8]);
    assert!(doc.outside.is_empty());
    assert_compiles(&edit.content);
    assert_eq!(unmanage(&edit.content).unwrap().content, HYPR_USER);
}

#[test]
fn saving_unchanged_rules_writes_nothing() {
    let adopted = adopt(HYPR_USER, &[]).unwrap().content;
    let edit = save(&adopted, &maintainer_rules()).unwrap();
    assert!(!edit.changed);
    assert_eq!(edit.content, adopted);
}

#[test]
fn saving_a_change_rewrites_only_that_rule() {
    let adopted = adopt(HYPR_USER, &[]).unwrap().content;
    let mut rules = maintainer_rules();
    rules[0].transform = Some(Transform::NORMAL);
    rules[1].position = Some(Position::At { x: 2560, y: 1335 });
    rules[2].position = Some(Position::At { x: 4480, y: 975 });
    let edit = save(&adopted, &rules).unwrap();
    assert_outside_preserved(&adopted, &edit.content);
    let changed: Vec<&str> = edit
        .content
        .lines()
        .zip(adopted.lines())
        .filter(|(a, b)| a != b)
        .map(|(a, _)| a)
        .collect();
    assert_eq!(
        changed,
        [
            r#"hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144", position = "0x0", scale = 1, transform = 0 })"#,
            r#"hl.monitor({ output = "eDP-1", mode = "1920x1080@144", position = "2560x1335", scale = 1 })"#,
            r#"hl.monitor({ output = "DP-1", mode = "2560x1440@179.95", position = "4480x975", scale = 1, vrr = 2 })"#,
        ]
    );
    assert_eq!(parse(&edit.content).unwrap().block_rules(), rules);
    assert_compiles(&edit.content);
    // Saving the result again is a fixed point.
    assert!(!save(&edit.content, &rules).unwrap().changed);
}

#[test]
fn a_new_block_goes_after_existing_rules() {
    let rules = vec![rule("DP-2", "1920x1080@60", 5920, 0)];
    let edit = save(HYPR_USER, &rules).unwrap();
    assert_outside_preserved(HYPR_USER, &edit.content);
    let block_line = edit
        .content
        .lines()
        .position(|l| l == LUA_MARKERS.begin)
        .unwrap();
    let dp_line = edit.content.lines().position(|l| l == DP).unwrap();
    assert_eq!(
        block_line,
        dp_line + 2,
        "after the last rule and a blank line"
    );
    assert_compiles(&edit.content);
}

#[test]
fn a_new_block_goes_before_the_top_level_return() {
    let src = "hl.config({})\n\nlocal function f()\n  return 1\nend\n\nreturn {\n  a = 1,\n}\n";
    let edit = save(src, &maintainer_rules()).unwrap();
    assert!(edit.content.contains("-- END hyprtilt\n\nreturn {\n"));
    assert!(
        edit.content
            .starts_with("hl.config({})\n\nlocal function f()\n  return 1\nend\n\n-- BEGIN")
    );
    assert_outside_preserved(src, &edit.content);
    assert_compiles(&edit.content);
}

#[test]
fn a_new_block_goes_at_the_end_otherwise() {
    let edit = save("hl.config({})", &maintainer_rules()[..1]).unwrap();
    assert!(
        edit.content
            .starts_with("hl.config({})\n\n-- BEGIN hyprtilt (managed)\n")
    );
    assert!(edit.content.ends_with("-- END hyprtilt\n"));
    let edit = save("", &maintainer_rules()[..1]).unwrap();
    assert!(edit.content.starts_with("-- BEGIN"));
    // No rules and no block: nothing to write.
    assert!(!save("x = 1\n", &[]).unwrap().changed);
}

#[test]
fn a_return_sharing_its_line_is_refused() {
    let err = save("x = 1 return {}\n", &maintainer_rules()).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 1, .. }), "{err}");
}

#[test]
fn caelestia_main_file() {
    let edit = save(CAELESTIA_MAIN, &maintainer_rules()).unwrap();
    assert_outside_preserved(CAELESTIA_MAIN, &edit.content);
    // After the catch-all rule, before the modules.
    let begin = edit.content.find(LUA_MARKERS.begin).unwrap();
    assert!(begin > edit.content.find("scale    = 1,\n})").unwrap());
    assert!(begin < edit.content.find("require(\"hyprland.general\")").unwrap());
    assert_compiles(&edit.content);
    // The catch-all is adoptable too.
    let doc = parse(CAELESTIA_MAIN).unwrap();
    assert_eq!(doc.outside.len(), 1);
    assert_eq!(doc.outside[0].rule.as_ref().unwrap().output.as_str(), "");
}

#[test]
fn crlf_files_stay_crlf() {
    let src = HYPR_USER.replace('\n', "\r\n");
    let edit = adopt(&src, &[]).unwrap();
    assert!(!edit.content.replace("\r\n", "").contains('\n'));
    let mut rules = maintainer_rules();
    rules.push(rule("DP-2", "1920x1080@60", 5920, 0));
    let edit = save(&edit.content, &rules).unwrap();
    assert!(!edit.content.replace("\r\n", "").contains('\n'));
    assert_eq!(parse(&edit.content).unwrap().block_rules(), rules);
}

#[test]
fn removal_spans_keep_neighbouring_code() {
    let src = "x = 1 hl.monitor({ output = \"a\" })\nhl.monitor({ output = \"b\" }) y = 2\nhl.monitor({ output = \"c\" }) -- note\n";
    let doc = parse(src).unwrap();
    let after: Vec<String> = doc
        .outside
        .iter()
        .map(|f| block::remove_range(src, &f.span))
        .collect();
    assert_eq!(
        after[0],
        src.replacen(" hl.monitor({ output = \"a\" })", "", 1)
    );
    assert_eq!(
        after[1],
        src.replacen("hl.monitor({ output = \"b\" }) ", "", 1)
    );
    assert_eq!(
        after[2],
        src.replacen("hl.monitor({ output = \"c\" }) -- note\n", "", 1)
    );
}

#[test]
fn adopt_selected_lines_merges_and_refuses() {
    let src = "hl.monitor({ output = \"a\", scale = 1 })\nif x then hl.monitor({ output = \"b\" }) end\nhl.monitor({ output = \"a\", vrr = 1 })\nhl.monitor(t)\n";
    let err = adopt(src, &[2]).unwrap_err();
    assert_eq!(
        err,
        ConfigError::NotAdoptable {
            line: 2,
            reason: "inside a function or a conditional block".to_owned()
        }
    );
    assert!(matches!(
        adopt(src, &[9]).unwrap_err(),
        ConfigError::NotAdoptable { line: 9, .. }
    ));
    assert!(matches!(
        adopt(src, &[4]).unwrap_err(),
        ConfigError::NotAdoptable { line: 4, .. }
    ));
    let edit = adopt(src, &[1, 3]).unwrap();
    let doc = parse(&edit.content).unwrap();
    let mut merged = MonitorRule::new("a");
    merged.scale = Some(Scale::Factor(1.0));
    merged.vrr = Some(1);
    assert_eq!(doc.block_rules(), [merged]);
    // Merged rules are regenerated, and the block sits where the last one was.
    assert_eq!(
        edit.content,
        "if x then hl.monitor({ output = \"b\" }) end\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"a\", scale = 1, vrr = 1 })\n-- END hyprtilt\nhl.monitor(t)\n"
    );
    // Nothing adoptable: nothing changes.
    assert!(!adopt("hl.monitor(t)\n", &[]).unwrap().changed);
}

#[test]
fn adopt_into_an_existing_block_respects_order() {
    let src = "hl.monitor({ output = \"a\", scale = 2, vrr = 1 })\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"a\", scale = 1 })\n-- END hyprtilt\nhl.monitor({ output = \"b\", scale = 1 })\nhl.monitor({ output = \"a\", transform = 1 })\n";
    let edit = adopt(src, &[]).unwrap();
    let doc = parse(&edit.content).unwrap();
    assert!(doc.outside.is_empty());
    let rules = doc.block_rules();
    // The earlier rule is overridden by the block, the later one overrides it.
    assert_eq!(rules[0].output.as_str(), "a");
    assert_eq!(rules[0].scale, Some(Scale::Factor(1.0)));
    assert_eq!(rules[0].vrr, Some(1));
    assert_eq!(rules[0].transform, Some(Transform::new(1).unwrap()));
    assert_eq!(rules[1].output.as_str(), "b");
    // A rule before the block with a new selector goes first.
    let src = "hl.monitor({ output = \"z\" })\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"a\" })\n-- END hyprtilt\n";
    let rules = parse(&adopt(src, &[]).unwrap().content)
        .unwrap()
        .block_rules()
        .to_vec();
    let order: Vec<&str> = rules.iter().map(|r| r.output.as_str()).collect();
    assert_eq!(order, ["z", "a"]);
}

#[test]
fn unmanage_needs_a_block() {
    assert_eq!(unmanage("x = 1\n").unwrap_err(), ConfigError::NoBlock);
}

#[test]
fn block_content_is_checked() {
    let wrap = |body: &str| format!("x = 1\n-- BEGIN hyprtilt (managed)\n{body}-- END hyprtilt\n");
    let err = parse(&wrap("local y = 2\n")).unwrap_err();
    assert_eq!(
        err,
        ConfigError::UnsupportedInBlock {
            line: 3,
            message: "only literal hl.monitor calls are allowed, found `local y = 2`".to_owned()
        }
    );
    let err = parse(&wrap("hl.monitor({ output = name })\n")).unwrap_err();
    assert_eq!(
        err,
        ConfigError::UnsupportedInBlock {
            line: 3,
            message: "output: `name` is not a literal".to_owned()
        }
    );
    // Comments, blank lines, semicolons and several calls on a line are fine.
    let doc = parse(&wrap(
        "-- mine\n\n  hl.monitor{ output = \"a\" }; hl.monitor({ output = \"b\" })\n--[[ long\ncomment ]]\n",
    ))
    .unwrap();
    assert_eq!(doc.block_rules().len(), 2);
    // An expression left open before the block swallows the first call.
    let src =
        "x = 1 +\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"a\" })\n-- END hyprtilt\n";
    assert!(matches!(
        parse(src).unwrap_err(),
        ConfigError::UnsupportedInBlock { line: 3, .. }
    ));
}

#[test]
fn markers_inside_strings_are_refused() {
    let src = "s = [[\n-- BEGIN hyprtilt (managed)\n]]\n-- END hyprtilt\n";
    let err = parse(src).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 2, .. }), "{err}");
    let src = "-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"a\",\n-- END hyprtilt\n})\n";
    let err = parse(src).unwrap_err();
    assert!(
        matches!(err, ConfigError::UnsupportedInBlock { line: 2, .. }),
        "{err}"
    );
    let src = "hl.monitor({ output = \"a\",\n-- BEGIN hyprtilt (managed)\n})\n-- END hyprtilt\n";
    let err = parse(src).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 1, .. }), "{err}");
}

#[test]
fn malformed_markers_and_lexical_errors() {
    let src = format!("{}\n", LUA_MARKERS.begin);
    assert!(matches!(parse(&src).unwrap_err(), ConfigError::Block(_)));
    assert!(matches!(
        parse("x = \"").unwrap_err(),
        ConfigError::Syntax { line: 1, .. }
    ));
}

#[test]
fn duplicate_selectors_are_refused() {
    let rules = [MonitorRule::new("a"), MonitorRule::new("a")];
    assert!(matches!(
        save("", &rules).unwrap_err(),
        ConfigError::Unrepresentable { .. }
    ));
}

#[test]
fn removing_a_rule_takes_its_comment() {
    let src = "-- BEGIN hyprtilt (managed)\n-- laptop\nhl.monitor({ output = \"eDP-1\" })\nhl.monitor({ output = \"DP-1\" })\n-- END hyprtilt\n";
    let edit = save(src, &[MonitorRule::new("DP-1")]).unwrap();
    assert_eq!(
        edit.content,
        "-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\" })\n-- END hyprtilt\n"
    );
    let edit = save(src, &[]).unwrap();
    assert_eq!(
        edit.content,
        "-- BEGIN hyprtilt (managed)\n-- END hyprtilt\n"
    );
}

fn full_rule() -> MonitorRule {
    let mut r = MonitorRule::new("desc:Samsung \"Odyssey\"\\G5");
    r.mode = Some(Mode::Resolution {
        width: 2560,
        height: 1440,
        refresh: Some(179.952),
    });
    r.position = Some(Position::At { x: -1920, y: 0 });
    r.scale = Some(Scale::Factor(4.0 / 3.0));
    r.transform = Some(Transform::new(7).unwrap());
    r.disabled = Some(false);
    r.vrr = Some(-1);
    r.mirror = Some("eDP-1".to_owned());
    r.bitdepth = Some(10);
    r.cm = Some(ColorManagement::Hdr);
    r.sdr_eotf = Some("gamma22".to_owned());
    r.sdrbrightness = Some(1.2);
    r.sdrsaturation = Some(0.95);
    r.icc = Some("/usr/share/color/icc/x.icc".to_owned());
    r.supports_wide_color = Some(1);
    r.supports_hdr = Some(-1);
    r.sdr_min_luminance = Some(0.005);
    r.sdr_max_luminance = Some(250);
    r.min_luminance = Some(0.0);
    r.max_luminance = Some(1000);
    r.max_avg_luminance = Some(400);
    r.reserved = Some(Reserved {
        top: 30,
        right: 0,
        bottom: 0,
        left: 0,
    });
    r.extra.push(ExtraField {
        key: "future".to_owned(),
        raw: "{ 1, 2 }".to_owned(),
    });
    r.extra.push(ExtraField {
        key: "odd key".to_owned(),
        raw: "true".to_owned(),
    });
    r
}

#[test]
fn every_field_round_trips() {
    let rule = full_rule();
    let text = format_rule(&rule).unwrap();
    assert_eq!(
        text,
        r#"hl.monitor({ output = "desc:Samsung \"Odyssey\"\\G5", mode = "2560x1440@179.95", position = "-1920x0", scale = "1.333333", transform = 7, disabled = false, vrr = -1, mirror = "eDP-1", bitdepth = 10, cm = "hdr", sdr_eotf = "gamma22", sdrbrightness = 1.2, sdrsaturation = 0.95, icc = "/usr/share/color/icc/x.icc", supports_wide_color = 1, supports_hdr = -1, sdr_min_luminance = 0.005, sdr_max_luminance = 250, min_luminance = 0, max_luminance = 1000, max_avg_luminance = 400, reserved = { top = 30, right = 0, bottom = 0, left = 0 }, future = { 1, 2 }, ["odd key"] = true })"#
    );
    let edit = save("", std::slice::from_ref(&rule)).unwrap();
    let back = parse(&edit.content).unwrap().block_rules()[0].clone();
    assert_eq!(back, rule.normalized());
    assert_eq!(
        format_rule(&back).unwrap(),
        text,
        "generation is a fixed point"
    );
    assert_compiles(&edit.content);
}

#[test]
fn strings_and_numbers() {
    assert_eq!(lua_string("a\u{1}b\tc\r\n"), r#""a\001b\tc\r\n""#);
    assert_eq!(lua_string("zażółć"), "\"zażółć\"");
    assert_eq!(lua_key("end"), r#"["end"]"#);
    assert_eq!(lua_key("_x1"), "_x1");
    assert_eq!(lua_key("1x"), r#"["1x"]"#);
    let mut r = MonitorRule::new("a");
    r.sdrbrightness = Some(f64::NAN);
    assert!(matches!(
        format_rule(&r).unwrap_err(),
        ConfigError::Unrepresentable { .. }
    ));
    r.sdrbrightness = Some(1e20);
    assert!(
        format_rule(&r)
            .unwrap()
            .contains("sdrbrightness = 100000000000000000000")
    );
    r.sdrbrightness = None;
    r.scale = Some(Scale::Auto);
    assert!(format_rule(&r).unwrap().contains(r#"scale = "auto""#));
}

#[test]
fn eval_code_joins_calls() {
    let code = eval_code(&maintainer_rules()[..2]).unwrap();
    assert_eq!(
        code,
        r#"hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144", position = "0x0", scale = 1, transform = 1 }) hl.monitor({ output = "eDP-1", mode = "1920x1080@144", position = "1440x1335", scale = 1 })"#
    );
    assert_eq!(eval_code(&[]).unwrap(), "");
}
