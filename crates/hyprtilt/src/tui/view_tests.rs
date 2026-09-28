// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

use std::time::Duration;

use hyprtilt_core::apply::Phase;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::tui::app::{ChangeKind, Countdown, Level};
use crate::tui::testing::{app, press, type_keys};

/// Draw `app` on a `width` x `height` terminal; returns the screen as
/// text and the placement.
fn render(app: &App, width: u16, height: u16) -> (String, Placement) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut placement = None;
    terminal
        .draw(|frame| placement = Some(draw(frame, app)))
        .unwrap();
    (screen(&terminal), placement.unwrap())
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn the_standard_terminal_shows_every_monitor_and_the_details() {
    let a = app();
    let (text, placement) = render(&a, 80, 24);
    for name in ["HDMI-A-1", "eDP-1", "DP-1"] {
        assert!(text.contains(name), "{name} missing:\n{text}");
    }
    assert_eq!(placement.boxes.len(), 3);
    assert!(text.contains("hyprtilt"));
    assert!(text.contains("hypr-user.lua (lua)"));
    // The details show the refresh rate and the available rates.
    assert!(text.contains("Refresh   144 Hz"), "{text}");
    assert!(text.contains("Rotation  90°"), "{text}");
    assert!(text.contains("Rule      line 6"), "{text}");
    assert!(text.contains("? shows every key"));
    assert!(text.contains("[ ] Hz"));
}

#[test]
fn the_selected_monitor_shows_its_rates() {
    let mut a = app();
    type_keys(&mut a, "3[");
    let (text, _) = render(&a, 100, 30);
    assert!(text.contains("Refresh   164.98 Hz"), "{text}");
    assert!(text.contains("Rates     59.95 120 164.98"), "{text}");
    assert!(text.contains("Rule      line 8, edited"), "{text}");
    assert!(text.contains("[modified]"));
    assert!(text.contains("DP-1: 164.98 Hz"));
    assert!(text.contains("Running   2560x1440@179.95 at"), "{text}");
}

#[test]
fn problems_and_flags_are_shown() {
    let mut a = app();
    a.layout.move_to(1, 0, 0);
    a.live_unsaved = true;
    a.offline = true;
    a.snap = false;
    a.say(Level::Error, "something failed");
    let (text, _) = render(&a, 120, 40);
    assert!(text.contains("HDMI-A-1 and eDP-1 overlap"), "{text}");
    assert!(text.contains("[live change not in the file]"));
    assert!(text.contains("[offline]"));
    assert!(text.contains("[snapping off]"));
    assert!(text.contains("something failed"));
    assert!(!text.contains("Running"));
}

#[test]
fn narrow_terminals_put_the_details_below() {
    let a = app();
    let (text, placement) = render(&a, 60, 30);
    assert_eq!(placement.boxes.len(), 3);
    let canvas_bottom = placement
        .boxes
        .iter()
        .map(|(_, cells)| cells.bottom())
        .max()
        .unwrap();
    let details_row = text.lines().position(|l| l.contains("Monitor")).unwrap();
    assert!(usize::from(canvas_bottom) <= details_row, "{text}");
}

#[test]
fn a_resize_rescales_the_canvas() {
    let a = app();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut small = None;
    terminal.draw(|f| small = Some(draw(f, &a))).unwrap();
    terminal.backend_mut().resize(160, 50);
    terminal
        .resize(ratatui::layout::Rect::new(0, 0, 160, 50))
        .unwrap();
    let mut large = None;
    terminal.draw(|f| large = Some(draw(f, &a))).unwrap();
    let (small, large) = (small.unwrap(), large.unwrap());
    assert!(large.per_column < small.per_column);
    assert_eq!(large.boxes.len(), 3);
    assert!(screen(&terminal).contains("2560x1440"));
}

#[test]
fn tiny_terminals_do_not_break() {
    let a = app();
    for (w, h) in [(1, 1), (10, 4), (20, 8), (40, 12)] {
        let (_, placement) = render(&a, w, h);
        for (_, cells) in placement.boxes {
            assert!(cells.right() <= w && cells.bottom() <= h);
        }
    }
    let mut empty = app();
    empty.layout.outputs.clear();
    let (text, _) = render(&empty, 80, 24);
    assert!(text.contains("no active monitor"));
    assert!(text.contains("no monitors"));
}

#[test]
fn boxes_keep_their_proportions() {
    let a = app();
    let (_, placement) = render(&a, 120, 40);
    let size = |i: usize| {
        let cells = placement.boxes.iter().find(|(k, _)| *k == i).unwrap().1;
        (f64::from(cells.width), f64::from(cells.height) * 2.0)
    };
    // HDMI-A-1 is portrait, DP-1 landscape; both are 2560 px on the long
    // side.
    let (hw, hh) = size(0);
    let (dw, dh) = size(2);
    assert!(hh > hw && dw > dh);
    assert!((hh - dw).abs() <= 2.0, "{hh} {dw}");
    // Adjacent monitors touch without overlapping.
    let cells = |i: usize| placement.boxes.iter().find(|(k, _)| *k == i).unwrap().1;
    assert_eq!(cells(0).right(), cells(1).x);
    assert_eq!(cells(1).right(), cells(2).x);
    assert_eq!(placement.hit(cells(2).x, cells(2).y), Some(2));
    assert_eq!(placement.hit(0, 0), None);
}

#[test]
fn the_help_lists_every_key() {
    let mut a = app();
    type_keys(&mut a, "?");
    let (text, _) = render(&a, 100, 30);
    assert!(text.contains("lower or raise the refresh rate"), "{text}");
    assert!(text.contains("rotate right, rotate left"));
    assert!(text.contains("any key closes this help"));
}

#[test]
fn pickers_show_the_cursor() {
    let mut a = app();
    type_keys(&mut a, "3m");
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Mode of DP-1"), "{text}");
    assert!(text.contains("> 2560x1440 @ 179.95 Hz"), "{text}");
    press(&mut a, KeyCode::Esc);
    type_keys(&mut a, "s");
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Scale of DP-1"), "{text}");
}

#[test]
fn dialogs_are_drawn() {
    let mut a = app();
    a.overlay = Overlay::Adopt(vec![5, 6, 7]);
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("3 monitor rule(s) outside the managed block (line 5, 6, 7)."));
    a.overlay = Overlay::Quit { live: true };
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("A change applied live is not in the file."));
    assert!(text.contains("r reload the file"));
    a.overlay = Overlay::Quit { live: false };
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("The layout differs from the file."));
    a.show_profiles(vec!["desk".to_owned(), "home".to_owned()]);
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("> desk"), "{text}");
    type_keys(&mut a, "swork");
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Save as: work_"), "{text}");
    a.show_profiles(Vec::new());
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("no profiles yet"));
}

#[test]
fn the_countdown_shows_the_seconds_left() {
    let mut a = app();
    let mut countdown = Countdown {
        kind: ChangeKind::Write,
        phase: Phase::Confirming,
        remaining: Some(Duration::from_millis(11_200)),
    };
    a.overlay = Overlay::Countdown(countdown.clone());
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Keep this layout?"), "{text}");
    assert!(text.contains("Reverting in 12 s"), "{text}");
    countdown.phase = Phase::Verifying;
    a.overlay = Overlay::Countdown(countdown.clone());
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Writing the file and reloading..."), "{text}");
    countdown.kind = ChangeKind::Live;
    countdown.phase = Phase::Applying;
    a.overlay = Overlay::Countdown(countdown.clone());
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Applying the layout live..."), "{text}");
    countdown.phase = Phase::RollingBack;
    a.overlay = Overlay::Countdown(countdown);
    let (text, _) = render(&a, 80, 24);
    assert!(text.contains("Rolling back..."), "{text}");
}
