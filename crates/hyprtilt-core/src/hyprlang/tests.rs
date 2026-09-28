// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the hyprlang backend.

use super::*;

const CONF: &str = include_str!("../../tests/fixtures/hyprlang/hyprland.conf");
const OPTS: SaveOptions = SaveOptions { hyprland: None };

fn rule(output: &str, mode: &str, pos: &str) -> MonitorRule {
    let mut r = MonitorRule::new(output);
    r.mode = Some(mode.parse().unwrap());
    r.position = Some(pos.parse().unwrap());
    r.scale = Some(Scale::Factor(1.0));
    r
}

fn version(v: &str) -> SaveOptions {
    SaveOptions {
        hyprland: Some(v.parse().unwrap()),
    }
}

#[test]
fn fixture_rules() {
    let doc = parse(CONF).unwrap();
    assert!(doc.block.is_none());
    let found: Vec<(usize, bool)> = doc
        .outside
        .iter()
        .map(|f| (f.line, f.is_adoptable()))
        .collect();
    assert_eq!(
        found,
        [
            (5, true),
            (6, true),
            (7, true),
            (8, false),
            (9, true),
            (11, true)
        ]
    );
    assert_eq!(
        doc.outside[3].not_adoptable.as_deref(),
        Some("uses hyprlang variables or expressions")
    );
    let dp = doc.outside[2].rule.as_ref().unwrap();
    assert_eq!(dp.vrr, Some(2));
    assert_eq!(
        doc.outside[2].text,
        "monitor = DP-1, 2560x1440@179.95, 3360x975, 1, vrr, 2"
    );
    let fallback = doc.outside[4].rule.as_ref().unwrap();
    assert_eq!(fallback.output.as_str(), "");
    let v2 = doc.outside[5].rule.as_ref().unwrap();
    assert_eq!(
        v2.output.as_str(),
        "desc:Samsung Electric Company Odyssey G50F SERIAL0002"
    );
    assert_eq!(v2.sdr_eotf.as_deref(), Some("gamma22"));
    assert!(doc.outside[5].text.starts_with("monitorv2 {\n") && doc.outside[5].text.ends_with('}'));
}

#[test]
fn adopting_across_a_patch_is_refused() {
    // `monitor = $laptop, addreserved, ...` patches a rule that may be one
    // of those being adopted.
    assert_eq!(
        adopt(CONF, &[]).unwrap_err(),
        ConfigError::AdoptCrossing { line: 8 }
    );
}

#[test]
fn adopt_in_place_and_unmanage() {
    let edit = adopt(CONF, &[5, 6, 7]).unwrap();
    let lines = "monitor = HDMI-A-1, 2560x1440@144, 0x0, 1, transform, 1\nmonitor=eDP-1,1920x1080@144,1440x1335,1\nmonitor = DP-1, 2560x1440@179.95, 3360x975, 1, vrr, 2 # fast panel\n";
    let kept = "monitor = HDMI-A-1, 2560x1440@144, 0x0, 1, transform, 1\nmonitor=eDP-1,1920x1080@144,1440x1335,1\nmonitor = DP-1, 2560x1440@179.95, 3360x975, 1, vrr, 2\n";
    let expected = CONF.replace(
        lines,
        &format!("# BEGIN hyprtilt (managed)\n{kept}# END hyprtilt\n"),
    );
    assert_eq!(edit.content, expected);
    let doc = parse(&edit.content).unwrap();
    assert_eq!(doc.block_rules().len(), 3);
    assert_eq!(doc.block.as_ref().unwrap().rule_lines, [6, 7, 8]);
    // Saving the same rules changes nothing.
    let rules = doc.block_rules().to_vec();
    assert!(!save(&edit.content, &rules, &OPTS).unwrap().changed);
    // The block's own trailing comment survives a change of its rule.
    let unmanaged = unmanage(&edit.content).unwrap().content;
    assert_eq!(unmanaged, CONF.replace(lines, kept));
}

#[test]
fn saving_a_change_keeps_a_trailing_comment() {
    let src = "# BEGIN hyprtilt (managed)\nmonitor = DP-1, 2560x1440@179.95, 3360x975, 1, vrr, 2 # fast panel\n# END hyprtilt\n";
    let mut dp = rule("DP-1", "2560x1440@179.95", "4480x975");
    dp.vrr = Some(2);
    let edit = save(src, &[dp], &OPTS).unwrap();
    assert_eq!(
        edit.content,
        "# BEGIN hyprtilt (managed)\nmonitor = DP-1, 2560x1440@179.95, 4480x975, 1, vrr, 2 # fast panel\n# END hyprtilt\n"
    );
}

#[test]
fn a_new_block_goes_after_the_last_rule() {
    let edit = save(CONF, &[rule("DP-2", "1920x1080@60", "5920x0")], &OPTS).unwrap();
    let begin = edit.content.find(HYPRLANG_MARKERS.begin).unwrap();
    assert!(begin > edit.content.find("    sdr_eotf = gamma22\n}").unwrap());
    assert!(begin < edit.content.find("# A literal hash").unwrap());
    let loc = block::locate(&edit.content, &HYPRLANG_MARKERS)
        .unwrap()
        .unwrap();
    let rest = format!(
        "{}{}",
        &edit.content[..loc.span.start],
        &edit.content[loc.span.end..]
    );
    assert_eq!(rest.replace("\n\n\n", "\n\n"), CONF);
    // Without rules, at the end.
    let edit = save("general {\n}\n", &[rule("a", "1x1@60", "0x0")], &OPTS).unwrap();
    assert!(edit.content.starts_with("general {\n}\n\n# BEGIN"));
}

#[test]
fn line_and_v2_forms_round_trip() {
    let mut plain = rule("desc:Acme #1, Inc", "2560x1440@179.952", "-1920x0");
    plain.transform = Some(Transform::new(3).unwrap());
    plain.vrr = Some(-1);
    plain.mirror = Some("eDP-1".to_owned());
    plain.bitdepth = Some(10);
    plain.cm = Some(ColorManagement::Wide);
    plain.sdrbrightness = Some(1.2);
    plain.sdrsaturation = Some(0.9);
    plain.icc = Some("/x.icc".to_owned());
    let text = format_rule(&plain, &OPTS).unwrap();
    assert_eq!(
        text,
        "monitor = desc:Acme ##1\\, Inc, 2560x1440@179.95, -1920x0, 1, transform, 3, vrr, -1, mirror, eDP-1, bitdepth, 10, cm, wide, sdrbrightness, 1.2, sdrsaturation, 0.9, icc, /x.icc"
    );
    let mut v2 = rule("DP-1", "2560x1440@144", "0x0");
    v2.disabled = Some(false);
    v2.sdr_eotf = Some("gamma22".to_owned());
    v2.supports_wide_color = Some(1);
    v2.supports_hdr = Some(-1);
    v2.sdr_min_luminance = Some(0.005);
    v2.sdr_max_luminance = Some(250);
    v2.min_luminance = Some(0.0);
    v2.max_luminance = Some(1000);
    v2.max_avg_luminance = Some(400);
    v2.reserved = Some(Reserved {
        top: 30,
        right: 4,
        bottom: 0,
        left: 2,
    });
    let text = format_rule(&v2, &OPTS).unwrap();
    assert!(text.starts_with("monitorv2 {\n    output = DP-1\n    mode = 2560x1440@144\n"));
    assert!(text.contains("\n    disabled = 0\n"));
    assert!(text.contains("\n    addreserved = 30, 0, 2, 4\n"));
    assert!(text.ends_with("\n}"));
    let edit = save("", &[plain.clone(), v2.clone()], &OPTS).unwrap();
    let back = parse(&edit.content).unwrap().block_rules().to_vec();
    assert_eq!(back, [plain.normalized(), v2.clone()]);
    // A second save is a fixed point.
    assert!(!save(&edit.content, &back, &OPTS).unwrap().changed);
    let mut disabled = MonitorRule::new("HDMI-A-1");
    disabled.disabled = Some(true);
    disabled.mode = Some(Mode::Preferred);
    assert_eq!(
        format_rule(&disabled, &OPTS).unwrap(),
        "monitor = HDMI-A-1, disable"
    );
    let back = parse(&save("", &[disabled], &OPTS).unwrap().content).unwrap();
    let mut expected = MonitorRule::new("HDMI-A-1");
    expected.disabled = Some(true);
    assert_eq!(back.block_rules(), [expected]);
}

#[test]
fn versions_gate_fields() {
    let mut r = rule("DP-1", "1920x1080@60", "0x0");
    r.sdr_eotf = Some("srgb".to_owned());
    assert!(format_rule(&r, &version("0.53.3")).is_err());
    assert!(format_rule(&r, &version("0.54.0")).is_ok());
    let mut r = rule("DP-1", "1920x1080@60", "0x0");
    r.reserved = Some(Reserved::default());
    assert!(format_rule(&r, &version("0.49.0")).is_err());
    assert!(format_rule(&r, &version("0.50.0")).is_ok());
    let mut r = rule("DP-1", "1920x1080@60", "0x0");
    r.icc = Some("/x.icc".to_owned());
    assert!(format_rule(&r, &version("0.54.0")).is_err());
    assert!(format_rule(&r, &version("0.55.0")).is_ok());
}

#[test]
fn unwritable_values() {
    for bad in ["$laptop", "a{{1+1}}", " lead", "trail\\", "tab\there"] {
        let r = rule(bad, "1920x1080@60", "0x0");
        assert!(
            matches!(
                format_rule(&r, &OPTS),
                Err(ConfigError::Unrepresentable { .. })
            ),
            "{bad}"
        );
    }
    let mut r = rule("a", "1x1@60", "0x0");
    r.extra.push(crate::model::ExtraField {
        key: "future".to_owned(),
        raw: "1".to_owned(),
    });
    let err = format_rule(&r, &OPTS).unwrap_err();
    assert!(err.to_string().contains("hyprlang has no field `future`"));
    let mut r = rule("a", "1x1@60", "0x0");
    r.sdrbrightness = Some(f64::INFINITY);
    assert!(format_rule(&r, &OPTS).is_err());
}

#[test]
fn keyword_values() {
    let mut r = rule("DP-1", "2560x1440@144", "0x0");
    r.transform = Some(Transform::new(1).unwrap());
    assert_eq!(
        keyword_value(&r).unwrap(),
        "DP-1,2560x1440@144,0x0,1,transform,1"
    );
    r.reserved = Some(Reserved::default());
    assert!(keyword_value(&r).is_err());
}

#[test]
fn monitor_line_forms() {
    let parsed = |v: &str| parse_monitor(v);
    let Parsed::Rule(r) = parsed("DP-1").clone() else {
        panic!("a bare selector is a rule with defaults");
    };
    assert_eq!(
        (r.mode, r.position, r.scale),
        (
            Some(Mode::Preferred),
            Some(Position::Auto(None)),
            Some(Scale::Auto)
        )
    );
    let Parsed::Rule(r) = parsed("DP-1, disabled") else {
        panic!("disable form");
    };
    assert!(r.is_disabled());
    let opaque = |v: &str| match parsed(v) {
        Parsed::Opaque(reason) => reason,
        Parsed::Rule(r) => panic!("{v} parsed as {r:?}"),
    };
    assert_eq!(
        opaque("DP-1, transform, 1"),
        "the `transform` form patches an earlier rule for `DP-1`"
    );
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, sdr_eotf, 1"),
        "unknown keyword `sdr_eotf`, which makes Hyprland ignore the whole line"
    );
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, , vrr, 1"),
        "field 5 is empty, which makes Hyprland ignore the rest of the line"
    );
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, vrr, 1, vrr, 2"),
        "the keyword `vrr` appears twice"
    );
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, vrr, 4"),
        "`vrr` must be from -1 to 3, not 4"
    );
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, transform, 1.0"),
        "`transform` must be an integer from 0 to 7, not \"1.0\""
    );
    assert_eq!(opaque("DP-1, wide"), "invalid mode: \"wide\"");
    assert_eq!(opaque("DP-1, preferred, auto, .5"), "invalid scale: \".5\"");
    assert_eq!(
        opaque("DP-1, preferred, auto, 1, cm, sRGB"),
        "invalid cm: \"sRGB\""
    );
}

#[test]
fn argument_splitting() {
    assert_eq!(split_args("a, b ,c"), ["a", "b", "c"]);
    assert_eq!(split_args("a\\, b, c"), ["a, b", "c"]);
    assert_eq!(split_args("a\\\\, b"), ["a, b"]);
    assert_eq!(split_args("a,,b,"), ["a", "", "b"]);
    assert_eq!(split_args(""), [""]);
    assert_eq!(split_args(", preferred"), ["", "preferred"]);
}

#[test]
fn monitorv2_blocks() {
    let doc = |body: &str| parse(&format!("monitorv2 {{\n{body}}}\n")).unwrap();
    let reason = |body: &str| doc(body).outside[0].not_adoptable.clone().unwrap();
    assert_eq!(
        reason("mode = preferred\n"),
        "the first field of a monitorv2 block must be `output`"
    );
    assert_eq!(
        reason("output = a\nfoo = 1\n"),
        "unknown monitorv2 field `foo`"
    );
    assert_eq!(
        reason("output = a\nmode = preferred\nmode = highrr\n"),
        "the field `mode` appears twice"
    );
    assert_eq!(
        reason("output = $x\n"),
        "uses hyprlang variables or expressions"
    );
    assert_eq!(
        reason("output = a\nsub {\n}\n"),
        "line 3: only `key = value` lines are allowed in a monitorv2 block"
    );
    assert_eq!(
        reason("# only a comment\n"),
        "the monitorv2 block has no `output`"
    );
    assert_eq!(
        reason("output = a\naddreserved = 1, 2\n"),
        "`addreserved` must be four integers, not \"1, 2\""
    );
    assert_eq!(
        reason("output = a\ndisabled = maybe\n"),
        "`disabled` must be 0 or 1, not \"maybe\""
    );
    assert_eq!(
        reason("output = a\nsdr_eotf = pq\n"),
        "invalid sdr_eotf: \"pq\" (use default, auto, srgb, gamma22, gamma22force)"
    );
    let r = doc("output = a\ndisabled = yes\nsdr_eotf = 0\nsdrbrightness = 1.5\nicc = /p\n")
        .outside[0]
        .rule
        .clone()
        .unwrap();
    assert_eq!(r.disabled, Some(true));
    assert_eq!(r.sdr_eotf.as_deref(), Some("auto"));
    let unclosed = parse("monitorv2 {\noutput = a\n").unwrap();
    assert_eq!(
        unclosed.outside[0].not_adoptable.as_deref(),
        Some("the monitorv2 block is not closed")
    );
    let other = parse("monitorv2[DP-1]:transform = 1\nmonitorv2[DP-1] {\n}\n").unwrap();
    assert_eq!(other.outside.len(), 2);
    assert!(other.outside.iter().all(|f| !f.is_adoptable()));
}

#[test]
fn block_content_is_checked() {
    let wrap = |body: &str| format!("x = 1\n# BEGIN hyprtilt (managed)\n{body}# END hyprtilt\n");
    let refused = |body: &str| parse(&wrap(body)).unwrap_err();
    assert_eq!(
        refused("$x = 1\n"),
        ConfigError::UnsupportedInBlock {
            line: 3,
            message: "only monitor rules are allowed, found `$x = 1`".to_owned()
        }
    );
    assert!(matches!(
        refused("source = a.conf\n"),
        ConfigError::UnsupportedInBlock { line: 3, .. }
    ));
    assert!(matches!(
        refused("# hyprlang if X\n"),
        ConfigError::UnsupportedInBlock { line: 3, .. }
    ));
    assert!(matches!(
        refused("monitor = $x, preferred, auto, 1\n"),
        ConfigError::UnsupportedInBlock { line: 3, .. }
    ));
    // Comments, blank lines and both rule forms are fine.
    let doc = parse(&wrap(
        "# mine\n\nmonitor = a, preferred, auto, 1\nmonitorv2 {\n  output = b\n}\n",
    ))
    .unwrap();
    assert_eq!(doc.block_rules().len(), 2);
    // A backslash before a marker joins the marker to the previous line.
    let err = parse("x = 1 \\\n# BEGIN hyprtilt (managed)\n# END hyprtilt\n").unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 2, .. }), "{err}");
    let err = parse("a {\n# BEGIN hyprtilt (managed)\n# END hyprtilt\n}\n").unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 2, .. }), "{err}");
    let err = parse("monitorv2 {\n# BEGIN hyprtilt (managed)\n}\n# END hyprtilt\n").unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { line: 2, .. }), "{err}");
    let err = parse("# BEGIN hyprtilt (managed)\nmonitorv2 {\n# END hyprtilt\n}\n").unwrap_err();
    assert!(
        matches!(err, ConfigError::UnsupportedInBlock { line: 2, .. }),
        "{err}"
    );
}

#[test]
fn placement_edge_cases() {
    let err = save("x = 1 \\", &[rule("a", "1x1@60", "0x0")], &OPTS).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { .. }), "{err}");
    let err = save("general {\n", &[rule("a", "1x1@60", "0x0")], &OPTS).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { .. }), "{err}");
    // A rule inside a category: the block goes after the category.
    let edit = save(
        "general {\n  monitor = a, preferred, auto, 1\n}\nx = 1\n",
        &[rule("b", "1x1@60", "0x0")],
        &OPTS,
    )
    .unwrap();
    assert!(
        edit.content
            .starts_with("general {\n  monitor = a, preferred, auto, 1\n}\n\n# BEGIN")
    );
    // Rules inside `# hyprlang if` are not adoptable.
    let doc =
        parse("# hyprlang if X\nmonitor = a, preferred, auto, 1\n# hyprlang endif\n").unwrap();
    assert_eq!(
        doc.outside[0].not_adoptable.as_deref(),
        Some("inside a `# hyprlang if` block")
    );
}

#[test]
fn adopt_with_an_existing_block_and_v2_precedence() {
    let src = "monitorv2 {\n  output = a\n  scale = 2\n}\nmonitor = a, preferred, auto, 1\n# BEGIN hyprtilt (managed)\n# END hyprtilt\n";
    let edit = adopt(src, &[]).unwrap();
    let doc = parse(&edit.content).unwrap();
    assert!(doc.outside.is_empty());
    // The monitorv2 block wins over the monitor= line although it comes first.
    assert_eq!(doc.block_rules().len(), 1);
    assert_eq!(doc.block_rules()[0].scale, Some(Scale::Factor(2.0)));
    // Without a block, the later monitor= line replaces the earlier one.
    let edit = adopt(
        "monitor = a, preferred, auto, 1\nmonitor = a, preferred, auto, 2\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        edit.content,
        "# BEGIN hyprtilt (managed)\nmonitor = a, preferred, auto, 2\n# END hyprtilt\n"
    );
    assert_eq!(unmanage("x\n").unwrap_err(), ConfigError::NoBlock);
    assert!(
        !adopt("monitor = $x, preferred, auto, 1\n", &[])
            .unwrap()
            .changed
    );
}

#[test]
fn crlf_files_stay_crlf() {
    let src = CONF.replace('\n', "\r\n");
    let edit = adopt(&src, &[5, 6, 7]).unwrap();
    assert!(!edit.content.replace("\r\n", "").contains('\n'));
    let mut v2 = rule("DP-3", "1x1@60", "0x0");
    v2.reserved = Some(Reserved::default());
    let mut rules = parse(&edit.content).unwrap().block_rules().to_vec();
    rules.push(v2);
    let edit = save(&edit.content, &rules, &OPTS).unwrap();
    assert!(!edit.content.replace("\r\n", "").contains('\n'));
    assert_eq!(parse(&edit.content).unwrap().block_rules().len(), 4);
}
