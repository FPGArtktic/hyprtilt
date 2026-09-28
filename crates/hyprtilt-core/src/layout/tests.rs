// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the editable layout on the maintainer's three monitors.

use super::*;

const MONITORS: &str = include_str!("../../tests/fixtures/ipc/monitors-all-0.56.2.json");
const HYPR_USER: &str = include_str!("../../tests/fixtures/lua/caelestia-hypr-user.lua");

fn monitors() -> Vec<MonitorInfo> {
    serde_json::from_str(MONITORS).unwrap()
}

fn block() -> Vec<MonitorRule> {
    let adopted = crate::lua::adopt(HYPR_USER, &[]).unwrap().content;
    crate::lua::parse(&adopted).unwrap().block_rules().to_vec()
}

fn layout() -> Layout {
    Layout::new(&monitors(), &block(), false)
}

fn rect_of(l: &Layout, name: &str) -> Rect {
    l.rect(l.index(name).unwrap()).unwrap()
}

#[test]
fn rules_come_from_the_block_and_match_the_live_state() {
    let l = layout();
    let names: Vec<&str> = l.outputs.iter().map(|o| o.info.name.as_str()).collect();
    assert_eq!(names, ["HDMI-A-1", "eDP-1", "DP-1"], "left to right");
    assert!(
        l.outputs
            .iter()
            .all(|o| matches!(o.origin, Origin::Block(_)))
    );
    assert_eq!(rect_of(&l, "HDMI-A-1"), Rect::new(0, 0, 1440, 2560));
    assert_eq!(rect_of(&l, "eDP-1"), Rect::new(1440, 1335, 1920, 1080));
    assert_eq!(rect_of(&l, "DP-1"), Rect::new(3360, 975, 2560, 1440));
    assert!(l.problems().is_empty(), "{:?}", l.problems());
    assert_eq!(l.rules(), block());
    assert!(compare(&l.expectations(), &monitors()).is_empty());
}

#[test]
fn outputs_without_a_rule_get_one_from_the_live_state() {
    let mut rules = block();
    let dp = rules.pop().unwrap();
    let mut absent = MonitorRule::new("HDMI-A-2");
    absent.mode = Some("1920x1080@60".parse().unwrap());
    rules.insert(0, absent.clone());
    let l = Layout::new(&monitors(), &rules, false);
    let i = l.index("DP-1").unwrap();
    assert_eq!(l.outputs[i].origin, Origin::New);
    assert_eq!(l.outputs[i].rule.mode, dp.mode);
    assert_eq!(l.outputs[i].rule.position, dp.position);
    assert_eq!(
        l.outputs[i].rule.vrr, None,
        "the live vrr flag is not the configured mode"
    );
    assert_eq!(l.detached, [(0, absent.clone())]);
    // Block order is kept, detached rules stay, new rules come last.
    let order: Vec<String> = l.rules().iter().map(|r| r.output.to_string()).collect();
    assert_eq!(order, ["HDMI-A-2", "HDMI-A-1", "eDP-1", "DP-1"]);
    // With descriptions preferred, a unique description is used.
    let l = Layout::new(&monitors(), &[], true);
    let i = l.index("DP-1").unwrap();
    assert_eq!(
        l.outputs[i].rule.output.as_str(),
        "desc:Samsung Electric Company Odyssey G50F SERIAL0002"
    );
    assert_eq!(
        l.find("desc:Samsung Electric Company Odyssey G50F SERIAL0002"),
        Some(i)
    );
    assert_eq!(
        l.find("Samsung Electric Company Odyssey G50F SERIAL0002"),
        Some(i)
    );
    assert!(l.index("DP-9").is_err());
}

#[test]
fn a_description_rule_serves_one_output_only() {
    let mut shared = MonitorRule::new("desc:Samsung Electric Company Odyssey G50F");
    shared.scale = Some(Scale::Factor(1.0));
    let l = Layout::new(&monitors(), &[shared], false);
    let origins: Vec<Origin> = l.outputs.iter().map(|o| o.origin).collect();
    assert_eq!(
        origins.iter().filter(|o| **o == Origin::Block(0)).count(),
        1
    );
}

#[test]
fn rotating_back_moves_the_neighbours() {
    let mut l = layout();
    let hdmi = l.index("HDMI-A-1").unwrap();
    l.rotate(hdmi, false);
    assert_eq!(l.outputs[hdmi].rule.transform, Some(Transform::NORMAL));
    assert_eq!(rect_of(&l, "HDMI-A-1"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect_of(&l, "eDP-1"), Rect::new(2560, 1335, 1920, 1080));
    assert_eq!(rect_of(&l, "DP-1"), Rect::new(4480, 975, 2560, 1440));
    assert!(l.problems().iter().all(|p| !p.is_blocking()));
    l.rotate(hdmi, true);
    assert_eq!(rect_of(&l, "eDP-1"), Rect::new(1440, 1335, 1920, 1080));
    l.flip(hdmi);
    assert_eq!(
        l.outputs[hdmi].rule.transform,
        Some(Transform::new(5).unwrap())
    );
}

#[test]
fn refresh_rates_step_and_snap_to_available_modes() {
    let mut l = layout();
    let dp = l.index("DP-1").unwrap();
    let rates = l.refresh_rates(dp);
    assert_eq!(rates.first().copied(), Some(59.95));
    assert_eq!(rates.last().copied(), Some(179.95));
    assert_eq!(l.step_refresh(dp, true), None, "already the highest");
    assert_eq!(l.step_refresh(dp, false), Some(164.98));
    assert_eq!(
        l.outputs[dp].rule.mode.as_ref().unwrap().to_string(),
        "2560x1440@164.98"
    );
    assert_eq!(l.step_refresh(dp, false), Some(120.0));
    let change = l.set_refresh(dp, RefreshTarget::Hz(165.0)).unwrap();
    assert_eq!(
        change,
        RefreshChange {
            from: Some(120.0),
            to: 164.98,
            custom: false
        }
    );
    let change = l.set_refresh(dp, RefreshTarget::Hz(144.0)).unwrap();
    assert!(change.custom);
    assert!(
        l.problems()
            .iter()
            .any(|p| matches!(p, Problem::CustomMode { .. }))
    );
    assert_eq!(l.set_refresh(dp, RefreshTarget::Max).unwrap().to, 179.95);
    assert_eq!(l.set_refresh(dp, RefreshTarget::Min).unwrap().to, 59.95);
    // A size change of the mode reflows too.
    let hdmi = l.index("HDMI-A-1").unwrap();
    l.set_mode(hdmi, &"1920x1080@120".parse().unwrap());
    assert_eq!(rect_of(&l, "HDMI-A-1"), Rect::new(0, 0, 1080, 1920));
    assert_eq!(rect_of(&l, "eDP-1").x, 1080);
    let mut empty = l.clone();
    empty.outputs[hdmi].info.available_modes.clear();
    assert!(matches!(
        empty.set_refresh(hdmi, RefreshTarget::Max),
        Err(LayoutError::NoRates(_))
    ));
}

#[test]
fn scales_and_their_problems() {
    let mut l = layout();
    let dp = l.index("DP-1").unwrap();
    assert!(l.scales(dp).contains(&1.25));
    l.set_scale(dp, Scale::Factor(1.25));
    assert_eq!(rect_of(&l, "DP-1"), Rect::new(3360, 975, 2048, 1152));
    l.set_scale(dp, Scale::Factor(1.5));
    let problem = l
        .problems()
        .into_iter()
        .find(|p| matches!(p, Problem::Scale { .. }))
        .unwrap();
    assert_eq!(
        problem.to_string(),
        "DP-1: Hyprland uses scale 1.6 instead of 1.5 and shows a warning"
    );
    let edp = l.index("eDP-1").unwrap();
    l.set_scale(edp, Scale::Auto);
    assert!(
        (l.scale(edp).scale - 1.5).abs() < f32::EPSILON,
        "eDP-1 is 143 ppi"
    );
}

#[test]
fn overlaps_block_and_gaps_warn() {
    let mut l = layout();
    let dp = l.index("DP-1").unwrap();
    l.move_to(dp, 3000, 975);
    let problems = l.problems();
    assert_eq!(
        problems[0],
        Problem::Overlap {
            a: "eDP-1".into(),
            b: "DP-1".into()
        }
    );
    assert!(problems[0].is_blocking());
    assert_eq!(problems[0].to_string(), "eDP-1 and DP-1 overlap");
    l.move_to(dp, 4000, 975);
    let gap = l.problems().remove(0);
    assert_eq!(gap.to_string(), "gap between HDMI-A-1 + eDP-1 and DP-1");
    assert!(!gap.is_blocking());
}

#[test]
fn moving_snapping_and_aligning() {
    let mut l = layout();
    let dp = l.index("DP-1").unwrap();
    l.nudge(dp, Direction::Right, 10, false, 0);
    assert_eq!(rect_of(&l, "DP-1").x, 3370);
    // Snapping goes back to eDP-1's edge first.
    l.nudge(dp, Direction::Left, 10, true, 100);
    assert_eq!(rect_of(&l, "DP-1").x, 3360);
    let edp = l.index("eDP-1").unwrap();
    assert!(l.align(edp, Align::Bottom));
    assert_eq!(rect_of(&l, "eDP-1").y, 2560 - 1080);
    assert!(!l.align(edp, Align::Bottom), "already aligned");
    l.set_enabled(dp, false);
    assert!(!l.align(dp, Align::Top));
    assert_eq!(l.rect(dp), None);
    l.nudge(dp, Direction::Up, 10, false, 0);
    l.set_enabled(dp, true);
    assert_eq!(l.outputs[dp].rule.disabled, None);
}

#[test]
fn vrr_cycles_through_all_values() {
    let mut l = layout();
    let edp = l.index("eDP-1").unwrap();
    let seen: Vec<Option<i8>> = (0..5).map(|_| l.cycle_vrr(edp)).collect();
    assert_eq!(seen, [Some(0), Some(1), Some(2), Some(3), None]);
}

#[test]
fn live_rules_are_complete() {
    let mut rules = block();
    rules[1] = MonitorRule::new("eDP-1");
    let l = Layout::new(&monitors(), &rules, false);
    let edp = l.index("eDP-1").unwrap();
    let live = l.live_rule(edp);
    assert_eq!(live.disabled, Some(false));
    assert_eq!(live.mode, Some(Mode::Preferred));
    assert_eq!(live.position, Some(Position::At { x: 1440, y: 1335 }));
    assert_eq!(live.scale, Some(Scale::Auto));
    assert_eq!(live.transform, Some(Transform::NORMAL));
    let mut off = l.clone();
    off.set_enabled(edp, false);
    assert!(off.live_rule(edp).is_disabled());
}

#[test]
fn comparing_with_the_live_state() {
    let mut l = layout();
    let dp = l.index("DP-1").unwrap();
    l.move_to(dp, 0, 0);
    l.set_scale(dp, Scale::Factor(2.0));
    l.set_refresh(dp, RefreshTarget::Min).unwrap();
    let hdmi = l.index("HDMI-A-1").unwrap();
    l.set_enabled(hdmi, false);
    let mismatches = compare(&l.expectations(), &monitors());
    let fields: Vec<(&str, &str)> = mismatches
        .iter()
        .map(|m| (m.output.as_str(), m.field))
        .collect();
    assert_eq!(
        fields,
        [
            ("HDMI-A-1", "state"),
            ("DP-1", "refresh rate"),
            ("DP-1", "position"),
            ("DP-1", "scale")
        ]
    );
    assert_eq!(
        mismatches[2].to_string(),
        "DP-1: position is 3360x975, expected 0x0"
    );
    let mut reported = monitors();
    reported[2].width = 1920;
    reported[2].transform = 2;
    let fields: Vec<&str> = compare(&layout().expectations(), &reported)
        .iter()
        .map(|m| m.field)
        .collect();
    assert_eq!(fields, ["mode", "transform"]);
}

#[test]
fn offline_layouts_come_from_the_block() {
    let mut rules = block();
    rules.push(MonitorRule::new("DP-2"));
    let l = Layout::offline(&rules);
    assert_eq!(l.outputs.len(), 3);
    assert_eq!(l.detached.len(), 1);
    assert_eq!(rect_of(&l, "HDMI-A-1"), Rect::new(0, 0, 1440, 2560));
    assert_eq!(l.rules(), rules);
}
