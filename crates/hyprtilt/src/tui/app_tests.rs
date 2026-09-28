// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

use hyprtilt_core::layout::Layout;
use hyprtilt_core::model::{Mode, Scale};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect as Area;

use super::*;
use crate::tui::canvas;
use crate::tui::testing::{app, block_rules, monitors, press, type_keys};

const HDMI: usize = 0;
const EDP: usize = 1;
const DP: usize = 2;

fn x(app: &App, i: usize) -> i32 {
    app.layout.rect(i).unwrap().x
}

fn message(app: &App) -> String {
    app.message
        .as_ref()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn ctrl(app: &mut App, c: char) -> Effect {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

#[test]
fn the_outputs_are_ordered_from_left_to_right() {
    let a = app();
    let names: Vec<&str> = a
        .layout
        .outputs
        .iter()
        .map(|o| o.info.name.as_str())
        .collect();
    assert_eq!(names, ["HDMI-A-1", "eDP-1", "DP-1"]);
    assert!(!a.modified() && a.overlay == Overlay::None && a.message.is_none());
}

#[test]
fn selection_moves_with_tab_and_digits() {
    let mut a = app();
    press(&mut a, KeyCode::Tab);
    assert_eq!(a.selected, EDP);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Tab);
    assert_eq!(a.selected, HDMI);
    press(&mut a, KeyCode::BackTab);
    assert_eq!(a.selected, DP);
    type_keys(&mut a, "2");
    assert_eq!(a.selected, EDP);
    type_keys(&mut a, "9");
    assert_eq!(a.selected, EDP);
    assert_eq!(a.name(), "eDP-1");
}

#[test]
fn moving_can_be_undone_and_redone() {
    let mut a = app();
    type_keys(&mut a, "3l");
    assert_eq!(x(&a, DP), 3370);
    assert!(a.modified());
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(x(&a, DP), 3470);
    type_keys(&mut a, "L");
    assert_eq!(x(&a, DP), 3570);
    type_keys(&mut a, "uuu");
    assert_eq!(x(&a, DP), 3360);
    assert!(!a.modified());
    type_keys(&mut a, "u");
    assert_eq!(message(&a), "nothing to undo");
    type_keys(&mut a, "U");
    assert_eq!(x(&a, DP), 3370);
    ctrl(&mut a, 'r');
    assert_eq!(x(&a, DP), 3470);
    ctrl(&mut a, 'r');
    ctrl(&mut a, 'r');
    assert_eq!(message(&a), "nothing to redo");
}

#[test]
fn snapping_can_be_turned_off() {
    let mut a = app();
    type_keys(&mut a, "g");
    assert!(!a.snap);
    assert!(message(&a).contains("off"));
    type_keys(&mut a, "g");
    assert!(a.snap);
}

#[test]
fn brackets_step_through_the_refresh_rates() {
    let mut a = app();
    type_keys(&mut a, "3]");
    assert_eq!(
        message(&a),
        "already the highest refresh rate of this resolution"
    );
    assert!(!a.modified());
    type_keys(&mut a, "[");
    assert_eq!(message(&a), "DP-1: 164.98 Hz");
    assert!((a.layout.refresh(DP) - 164.98).abs() < 0.01);
    type_keys(&mut a, "[[");
    assert!((a.layout.refresh(DP) - 59.95).abs() < 0.01);
    type_keys(&mut a, "[");
    assert!(message(&a).starts_with("already the lowest"));
    type_keys(&mut a, "]]]");
    assert!((a.layout.refresh(DP) - 179.95).abs() < 0.01);
}

#[test]
fn overlaps_block_applying_and_writing() {
    let mut a = app();
    a.layout.move_to(EDP, 0, 0);
    assert_eq!(type_keys(&mut a, "a"), Effect::None);
    assert!(message(&a).contains("overlap"));
    assert_eq!(type_keys(&mut a, "w"), Effect::None);
    a.layout.move_to(EDP, 1440, 1335);
    assert_eq!(type_keys(&mut a, "a"), Effect::ApplyLive);
    assert_eq!(type_keys(&mut a, "w"), Effect::Write);
}

#[test]
fn nothing_is_applied_live_offline() {
    let mut a = app();
    a.offline = true;
    assert_eq!(type_keys(&mut a, "a"), Effect::None);
    assert!(message(&a).contains("not reachable"));
    assert_eq!(type_keys(&mut a, "w"), Effect::Write);
}

#[test]
fn quitting_asks_about_changes() {
    let mut a = app();
    assert_eq!(type_keys(&mut a, "q"), Effect::Quit);
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::Quit);
    assert_eq!(ctrl(&mut a, 'c'), Effect::Quit);
    type_keys(&mut a, "1f");
    assert_eq!(type_keys(&mut a, "q"), Effect::None);
    assert_eq!(a.overlay, Overlay::Quit { live: false });
    // Reverting live changes is offered only when there are some.
    assert_eq!(type_keys(&mut a, "r"), Effect::None);
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::None);
    assert_eq!(a.overlay, Overlay::None);
    type_keys(&mut a, "q");
    assert_eq!(type_keys(&mut a, "w"), Effect::WriteThenQuit);
    type_keys(&mut a, "q");
    assert_eq!(type_keys(&mut a, "q"), Effect::Quit);
    a.live_unsaved = true;
    type_keys(&mut a, "q");
    assert_eq!(a.overlay, Overlay::Quit { live: true });
    assert_eq!(type_keys(&mut a, "r"), Effect::RevertAndQuit);
}

#[test]
fn the_mode_picker_sets_the_mode() {
    let mut a = app();
    type_keys(&mut a, "3m");
    let Overlay::Modes(picker) = &a.overlay else {
        panic!("{:?}", a.overlay)
    };
    assert_eq!(picker.cursor, 0);
    assert_eq!(picker.items[0].0, "2560x1440 @ 179.95 Hz");
    assert!(picker.items.iter().any(|(l, _)| l == "preferred"));
    type_keys(&mut a, "j");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.overlay, Overlay::None);
    assert!((a.layout.refresh(DP) - 164.98).abs() < 0.01);
    // The cursor opens on the current mode; Esc changes nothing.
    type_keys(&mut a, "m");
    let Overlay::Modes(picker) = &a.overlay else {
        panic!()
    };
    assert_eq!(picker.cursor, 1);
    for code in [
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::Up,
    ] {
        press(&mut a, code);
    }
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.overlay, Overlay::None);
    assert!((a.layout.refresh(DP) - 164.98).abs() < 0.01);
    type_keys(&mut a, "mG");
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.outputs[DP].rule.mode, Some(Mode::HighResolution));
}

#[test]
fn the_scale_picker_shows_logical_sizes() {
    let mut a = app();
    type_keys(&mut a, "1s");
    let Overlay::Scales(picker) = &a.overlay else {
        panic!("{:?}", a.overlay)
    };
    // HDMI-A-1 is rotated: scale 1 is 1440x2560.
    let one = picker
        .items
        .iter()
        .find(|(l, _)| l.starts_with("1 "))
        .unwrap();
    assert!(one.0.ends_with("1440x2560"), "{}", one.0);
    assert_eq!(picker.items[picker.cursor].1, Scale::Factor(1.0));
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.layout.outputs[HDMI].rule.scale, Some(Scale::Auto));
    type_keys(&mut a, "s");
    let Overlay::Scales(picker) = &a.overlay else {
        panic!()
    };
    assert_eq!(picker.items[picker.cursor].1, Scale::Auto);
    type_keys(&mut a, "q");
    assert_eq!(a.overlay, Overlay::None);
}

#[test]
fn the_last_active_monitor_stays_enabled() {
    let mut a = app();
    type_keys(&mut a, "1e2e");
    assert!(a.layout.outputs[HDMI].rule.is_disabled());
    assert!(a.layout.outputs[EDP].rule.is_disabled());
    type_keys(&mut a, "3e");
    assert!(!a.layout.outputs[DP].rule.is_disabled());
    assert_eq!(message(&a), "refusing to disable the only active monitor");
    type_keys(&mut a, "1e");
    assert!(!a.layout.outputs[HDMI].rule.is_disabled());
}

#[test]
fn rotation_flip_vrr_and_alignment() {
    let mut a = app();
    type_keys(&mut a, "1r");
    assert_eq!(
        a.layout.outputs[HDMI].rule.transform_or_default().value(),
        2
    );
    type_keys(&mut a, "RR");
    assert_eq!(
        a.layout.outputs[HDMI].rule.transform_or_default().value(),
        0
    );
    type_keys(&mut a, "f");
    assert_eq!(
        a.layout.outputs[HDMI].rule.transform_or_default().value(),
        4
    );
    type_keys(&mut a, "3v");
    assert_eq!(message(&a), "VRR fullscreen games and video");
    type_keys(&mut a, "v");
    assert_eq!(message(&a), "VRR unset (follows misc:vrr)");
    // eDP-1 and DP-1 share their bottom edge already.
    type_keys(&mut a, "b");
    assert_eq!(message(&a), "nothing to align with, or already aligned");
    type_keys(&mut a, "t");
    assert_eq!(a.layout.rect(DP).unwrap().y, 1335);
    type_keys(&mut a, "c");
    assert!(a.modified());
}

#[test]
fn help_closes_on_any_key() {
    let mut a = app();
    type_keys(&mut a, "?");
    assert_eq!(a.overlay, Overlay::Help);
    assert_eq!(type_keys(&mut a, "x"), Effect::None);
    assert_eq!(a.overlay, Overlay::None);
}

#[test]
fn profiles_are_listed_loaded_and_saved() {
    let mut a = app();
    assert_eq!(type_keys(&mut a, "p"), Effect::ListProfiles);
    a.show_profiles(vec!["desk".to_owned(), "home".to_owned()]);
    type_keys(&mut a, "jj");
    assert_eq!(
        press(&mut a, KeyCode::Enter),
        Effect::LoadProfile("home".to_owned())
    );
    a.show_profiles(vec!["desk".to_owned()]);
    type_keys(&mut a, "k");
    assert_eq!(type_keys(&mut a, "s"), Effect::None);
    type_keys(&mut a, "work 1/");
    press(&mut a, KeyCode::Backspace);
    let Overlay::Profiles(dialog) = &a.overlay else {
        panic!()
    };
    assert_eq!(dialog.input.as_deref(), Some("work"));
    assert_eq!(
        press(&mut a, KeyCode::Enter),
        Effect::SaveProfile("work".to_owned())
    );
    a.show_profiles(Vec::new());
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    type_keys(&mut a, "n");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Esc);
    assert!(matches!(
        a.overlay,
        Overlay::Profiles(ProfileDialog { input: None, .. })
    ));
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.overlay, Overlay::None);
}

#[test]
fn a_loaded_profile_can_be_undone() {
    let mut a = app();
    let mut other = a.layout.clone();
    other.move_to(DP, 3360, 1120);
    a.load_layout(other, "desk");
    assert!(a.modified());
    assert!(message(&a).contains("\"desk\" loaded"));
    type_keys(&mut a, "u");
    assert!(!a.modified());
}

#[test]
fn rules_outside_the_block_are_offered_for_adoption() {
    let mut a = app();
    type_keys(&mut a, "o");
    assert_eq!(message(&a), "there are no monitor rules outside the block");
    let mut file = a.file.clone();
    file.outside = vec![5, 6, 7];
    let mut a = App::new(a.layout.clone(), file, false, true);
    assert_eq!(a.overlay, Overlay::Adopt(vec![5, 6, 7]));
    assert_eq!(type_keys(&mut a, "x"), Effect::None);
    assert_eq!(type_keys(&mut a, "n"), Effect::None);
    assert_eq!(a.overlay, Overlay::None);
    type_keys(&mut a, "o");
    assert_eq!(type_keys(&mut a, "y"), Effect::Adopt);
}

#[test]
fn the_countdown_takes_only_keep_and_revert() {
    let mut a = app();
    a.overlay = Overlay::Countdown(Countdown {
        kind: ChangeKind::Live,
        phase: Phase::Confirming,
        remaining: Some(Duration::from_secs(9)),
    });
    assert_eq!(type_keys(&mut a, "q"), Effect::None);
    assert_eq!(type_keys(&mut a, "y"), Effect::Confirm);
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::Confirm);
    assert_eq!(type_keys(&mut a, "n"), Effect::Cancel);
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::Cancel);
    assert_eq!(ctrl(&mut a, 'c'), Effect::Cancel);
    assert!(matches!(a.overlay, Overlay::Countdown(_)));
}

fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    app.mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

#[test]
fn the_mouse_selects_and_drags() {
    let mut a = app();
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), 0, 0);
    assert_eq!(a.selected, HDMI);
    let placement = canvas::place(&a.layout, Area::new(0, 0, 62, 20), a.selected);
    let dp = placement.boxes.iter().find(|(i, _)| *i == DP).unwrap().1;
    a.placement = Some(placement.clone());
    let (column, row) = (dp.x + 2, dp.y + 1);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), column, row);
    assert_eq!(a.selected, DP);
    mouse(
        &mut a,
        MouseEventKind::Drag(MouseButton::Left),
        column + 2,
        row,
    );
    let (dx, _) = placement.delta((0, 0), (2, 0), STEP);
    assert!(dx > 0);
    assert_eq!(x(&a, DP), 3360 + dx);
    mouse(
        &mut a,
        MouseEventKind::Up(MouseButton::Left),
        column + 2,
        row,
    );
    assert!(a.modified());
    type_keys(&mut a, "u");
    assert_eq!(x(&a, DP), 3360);
    // A click without a drag changes nothing; clicks outside do nothing.
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), column, row);
    mouse(&mut a, MouseEventKind::Up(MouseButton::Left), column, row);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), column, row);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), 0, 0);
    mouse(&mut a, MouseEventKind::Moved, 0, 0);
    assert!(!a.modified());
    a.overlay = Overlay::Help;
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), 0, 0);
    assert_eq!(a.selected, DP);
}

#[test]
fn a_hotplug_keeps_the_edits() {
    let mut a = app();
    type_keys(&mut a, "3[");
    let mut fewer = monitors();
    fewer.retain(|m| m.name != "HDMI-A-1");
    a.rebuild(&fewer, false);
    assert_eq!(a.layout.outputs.len(), 2);
    assert_eq!(a.name(), "DP-1");
    assert!((a.layout.refresh(a.selected) - 164.98).abs() < 0.01);
    assert!(a.modified());
    // The rule of the absent monitor stays in the block.
    assert_eq!(a.layout.detached.len(), 1);
    type_keys(&mut a, "u");
    assert_eq!(message(&a), "nothing to undo");
}

#[test]
fn rules_made_for_unplugged_monitors_are_dropped() {
    let mut rules = block_rules();
    rules.retain(|r| r.output.as_str() != "HDMI-A-1");
    let layout = Layout::new(&monitors(), &rules, false);
    let mut a = app();
    a.layout = layout;
    let before = a.layout.rules().len();
    assert_eq!(before, 3);
    let mut fewer = monitors();
    fewer.retain(|m| m.name != "HDMI-A-1");
    a.rebuild(&fewer, false);
    assert_eq!(a.layout.rules().len(), 2);
    assert!(a.layout.detached.is_empty());
    // Nothing was edited, so nothing is modified after the hotplug.
    let mut b = app();
    b.rebuild(&monitors(), false);
    assert!(!b.modified());
}

#[test]
fn live_facts_are_updated_in_place() {
    let mut a = app();
    let mut live = monitors();
    live[2].refresh_rate = 120.0;
    a.update_live(&live);
    assert!(
        a.layout
            .outputs
            .iter()
            .any(|o| (o.info.refresh_rate - 120.0).abs() < 1e-9)
    );
    assert!(!a.modified());
}

#[test]
fn labels() {
    assert_eq!(
        mode_label(&"1920x1080@60".parse().unwrap()),
        "1920x1080 @ 60 Hz"
    );
    assert_eq!(mode_label(&Mode::Preferred), "preferred");
    assert_eq!(vrr_name(Some(0)), "off");
    assert_eq!(vrr_name(Some(1)), "on");
    assert_eq!(vrr_name(Some(2)), "fullscreen only");
    assert_eq!(vrr_name(Some(-1)), vrr_name(None));
    let file = app().file;
    let rules = block_rules();
    assert_eq!(file.line_of(&rules[0]), Some((6, false)));
    let mut edited = rules[2].clone();
    edited.vrr = None;
    assert_eq!(file.line_of(&edited), Some((8, true)));
    edited.output = hyprtilt_core::model::Selector::new("HDMI-A-2");
    assert_eq!(file.line_of(&edited), None);
}
