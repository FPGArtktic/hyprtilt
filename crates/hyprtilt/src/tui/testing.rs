// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Fixtures shared by the tests of the terminal interface: the
//! maintainer's three monitors and their rules.

use hyprtilt_core::document::Backend;
use hyprtilt_core::ipc::MonitorInfo;
use hyprtilt_core::layout::Layout;
use hyprtilt_core::model::MonitorRule;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Effect, FileInfo};

/// `monitors all -j` of Hyprland 0.56.2 with three monitors.
pub(super) const MONITORS: &str =
    include_str!("../../../hyprtilt-core/tests/fixtures/ipc/monitors-all-0.56.2.json");
/// Caelestia's `hypr-user.lua` with the rules outside any block.
pub(super) const HYPR_USER: &str =
    include_str!("../../../hyprtilt-core/tests/fixtures/lua/caelestia-hypr-user.lua");

/// The connected monitors.
pub(super) fn monitors() -> Vec<MonitorInfo> {
    serde_json::from_str(MONITORS).unwrap()
}

/// `hypr-user.lua` with its rules adopted into the block.
pub(super) fn managed() -> String {
    Backend::Lua.adopt(HYPR_USER, &[]).unwrap().content
}

/// The rules of the block of [`managed`].
pub(super) fn block_rules() -> Vec<MonitorRule> {
    Backend::Lua
        .parse(&managed())
        .unwrap()
        .block_rules()
        .to_vec()
}

/// An interface on the three monitors with the rules in the block.
/// Outputs by position: 0 HDMI-A-1, 1 eDP-1, 2 DP-1.
pub(super) fn app() -> App {
    let rules = block_rules();
    let layout = Layout::new(&monitors(), &rules, false);
    let file = FileInfo {
        path: "hypr-user.lua".to_owned(),
        backend: Backend::Lua,
        lines: vec![6, 7, 8],
        rules,
        outside: Vec::new(),
    };
    App::new(layout, file, false, true)
}

/// Press a key without modifiers.
pub(super) fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Type characters one by one; returns the last effect.
pub(super) fn type_keys(app: &mut App, keys: &str) -> Effect {
    let mut effect = Effect::None;
    for c in keys.chars() {
        effect = press(app, KeyCode::Char(c));
    }
    effect
}
