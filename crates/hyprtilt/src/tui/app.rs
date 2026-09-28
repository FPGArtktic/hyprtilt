// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The state of the terminal interface and what keys do to it.
//!
//! [`App`] never touches the terminal, the files or Hyprland. A key either
//! changes the edited layout, opens or closes a dialog, or returns an
//! [`Effect`] that the controller carries out. This keeps every key
//! testable.

use std::time::Duration;

use hyprtilt_core::apply::Phase;
use hyprtilt_core::document::Backend;
use hyprtilt_core::geometry::{Align, Direction};
use hyprtilt_core::ipc::MonitorInfo;
use hyprtilt_core::layout::{Layout, Origin};
use hyprtilt_core::model::{Mode, MonitorRule, Scale, format_refresh};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::canvas::Placement;

/// Pixels per small step.
pub(crate) const STEP: i32 = 10;
/// Pixels per large step, and the range of edge snapping.
pub(crate) const BIG_STEP: i32 = 100;

/// The keys, for the help popup and the documentation.
pub(crate) const KEYS: &[(&str, &str)] = &[
    (
        "Tab, Shift+Tab, 1-9",
        "select the next, previous or n-th monitor",
    ),
    (
        "h j k l, arrows",
        "move by 10 px, snapping to neighbouring edges",
    ),
    ("H J K L, Shift+arrows", "move by 100 px"),
    ("g", "turn snapping on or off"),
    (
        "b t c",
        "align bottom, top or centre with the nearest neighbour",
    ),
    ("r R", "rotate right, rotate left"),
    ("f", "flip"),
    ("m", "choose the mode (resolution and refresh rate)"),
    ("[ ]", "lower or raise the refresh rate"),
    ("s", "choose the scale"),
    (
        "v",
        "cycle VRR: unset, off, on, fullscreen, games and video",
    ),
    ("e", "enable or disable the monitor"),
    ("u, U or Ctrl+r", "undo, redo"),
    ("a", "apply live; kept only when confirmed"),
    (
        "w",
        "write into the file and reload; kept only when confirmed",
    ),
    ("o", "adopt monitor rules from outside the managed block"),
    ("p", "profiles: load or save"),
    ("mouse", "click selects a monitor, dragging moves it"),
    ("?", "this help"),
    ("q, Esc", "quit"),
];

/// Something the controller has to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Nothing.
    None,
    /// Apply the layout to the running session, with a countdown.
    ApplyLive,
    /// Write the layout into the file, reload, verify, with a countdown.
    Write,
    /// Like [`Effect::Write`], and quit once the change is kept.
    WriteThenQuit,
    /// Keep the change being counted down.
    Confirm,
    /// Undo the change being counted down.
    Cancel,
    /// Move the rules outside the block into it.
    Adopt,
    /// Read the profile names and show them.
    ListProfiles,
    /// Put a profile's rules into the editor.
    LoadProfile(String),
    /// Save the edited layout as a profile.
    SaveProfile(String),
    /// Quit.
    Quit,
    /// Reload the file to drop live changes that are not in it, then quit.
    RevertAndQuit,
}

/// How important a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// Neutral.
    Info,
    /// Something worked.
    Success,
    /// Something is wrong.
    Error,
}

/// A line in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) text: String,
    pub(crate) level: Level,
}

/// A list to choose from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Picker<T> {
    pub(crate) title: String,
    pub(crate) items: Vec<(String, T)>,
    pub(crate) cursor: usize,
}

enum PickerKey {
    Moved,
    Chosen,
    Closed,
}

impl<T> Picker<T> {
    fn key(&mut self, code: KeyCode) -> PickerKey {
        let last = self.items.len().saturating_sub(1);
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::PageDown => self.cursor = (self.cursor + 10).min(last),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(10),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = last,
            KeyCode::Enter => return PickerKey::Chosen,
            KeyCode::Esc | KeyCode::Char('q') => return PickerKey::Closed,
            _ => {}
        }
        PickerKey::Moved
    }
}

/// Which change is being counted down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    /// A live change (`a`).
    Live,
    /// A file write (`w`).
    Write,
}

/// The state of a running apply session, as the controller reports it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Countdown {
    pub(crate) kind: ChangeKind,
    pub(crate) phase: Phase,
    pub(crate) remaining: Option<Duration>,
}

/// The profile dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfileDialog {
    pub(crate) names: Vec<String>,
    pub(crate) cursor: usize,
    /// The name being typed for "save as", if the input line is open.
    pub(crate) input: Option<String>,
}

/// A dialog or popup over the canvas.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Overlay {
    None,
    Help,
    Modes(Picker<Mode>),
    Scales(Picker<Scale>),
    Adopt(Vec<usize>),
    Profiles(ProfileDialog),
    Quit { live: bool },
    Countdown(Countdown),
}

/// The file being edited, as it was last read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileInfo {
    /// Its path, for the title.
    pub(crate) path: String,
    /// Its language.
    pub(crate) backend: Backend,
    /// The rules of its managed block.
    pub(crate) rules: Vec<MonitorRule>,
    /// The line of each rule.
    pub(crate) lines: Vec<usize>,
    /// Lines of adoptable rules outside the block.
    pub(crate) outside: Vec<usize>,
}

impl FileInfo {
    /// The line of the block rule for `rule`'s output, and whether the
    /// rule differs from it.
    pub(crate) fn line_of(&self, rule: &MonitorRule) -> Option<(usize, bool)> {
        let k = self.rules.iter().rposition(|r| r.output == rule.output)?;
        Some((
            self.lines.get(k).copied().unwrap_or(0),
            self.rules[k] != *rule,
        ))
    }
}

/// A drag of a monitor with the mouse.
#[derive(Debug, Clone)]
struct Drag {
    index: usize,
    column: u16,
    row: u16,
    origin: (i32, i32),
    before: Layout,
}

/// The terminal interface's state.
pub(crate) struct App {
    pub(crate) layout: Layout,
    /// The rules of the layout when it last matched the file.
    baseline: Vec<MonitorRule>,
    /// A change applied live and kept is not in the file.
    pub(crate) live_unsaved: bool,
    pub(crate) selected: usize,
    pub(crate) snap: bool,
    undo: Vec<Layout>,
    redo: Vec<Layout>,
    pub(crate) overlay: Overlay,
    pub(crate) message: Option<Message>,
    pub(crate) file: FileInfo,
    /// Hyprland is not reachable: editing the file only.
    pub(crate) offline: bool,
    /// Where the canvas drew the monitors last time, for the mouse.
    pub(crate) placement: Option<Placement>,
    drag: Option<Drag>,
}

impl App {
    /// A new interface for `layout` built from `file`.
    pub(crate) fn new(layout: Layout, file: FileInfo, offline: bool, snap: bool) -> App {
        let overlay = if file.outside.is_empty() {
            Overlay::None
        } else {
            Overlay::Adopt(file.outside.clone())
        };
        let message = offline.then(|| Message {
            text: "Hyprland is not reachable: w writes the file; nothing is applied or verified"
                .to_owned(),
            level: Level::Error,
        });
        App {
            baseline: layout.rules(),
            layout,
            live_unsaved: false,
            selected: 0,
            snap,
            undo: Vec::new(),
            redo: Vec::new(),
            overlay,
            message,
            file,
            offline,
            placement: None,
            drag: None,
        }
    }

    /// Whether the layout differs from the file.
    pub(crate) fn modified(&self) -> bool {
        self.layout.rules() != self.baseline
    }

    /// Record that the file now holds the layout.
    pub(crate) fn mark_saved(&mut self, file: FileInfo) {
        self.baseline = self.layout.rules();
        self.live_unsaved = false;
        self.file = file;
    }

    /// Show a message in the status bar.
    pub(crate) fn say(&mut self, level: Level, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            level,
        });
    }

    /// The name of the selected output.
    pub(crate) fn name(&self) -> String {
        self.layout
            .outputs
            .get(self.selected)
            .map(|o| o.info.name.clone())
            .unwrap_or_default()
    }

    /// Rebuild the layout for a changed set of monitors, keeping the
    /// edits. Rules made for monitors that are gone are dropped; the undo
    /// history is cleared because it belongs to the old set.
    pub(crate) fn rebuild(&mut self, monitors: &[MonitorInfo], prefer_descriptions: bool) {
        let was_modified = self.modified();
        let name = self.name();
        self.layout
            .outputs
            .retain(|o| o.origin != Origin::New || monitors.iter().any(|m| m.name == o.info.name));
        self.layout = Layout::new(monitors, &self.layout.rules(), prefer_descriptions);
        if !was_modified {
            self.baseline = self.layout.rules();
        }
        self.undo.clear();
        self.redo.clear();
        self.drag = None;
        self.selected = self
            .layout
            .outputs
            .iter()
            .position(|o| o.info.name == name)
            .unwrap_or(0);
    }

    /// Replace the live facts of the outputs (after a change or a reload)
    /// without touching their rules.
    pub(crate) fn update_live(&mut self, monitors: &[MonitorInfo]) {
        for o in &mut self.layout.outputs {
            if let Some(m) = monitors.iter().find(|m| m.name == o.info.name) {
                o.info = m.clone();
            }
        }
    }

    /// Put a profile's layout into the editor.
    pub(crate) fn load_layout(&mut self, layout: Layout, name: &str) {
        let before = std::mem::replace(&mut self.layout, layout);
        self.undo.push(before);
        self.redo.clear();
        self.selected = 0;
        self.say(
            Level::Info,
            format!("profile {name:?} loaded: a applies it live, w writes it"),
        );
    }

    /// Show the profile names the controller read.
    pub(crate) fn show_profiles(&mut self, names: Vec<String>) {
        self.overlay = Overlay::Profiles(ProfileDialog {
            names,
            cursor: 0,
            input: None,
        });
    }

    fn edit(&mut self, change: impl FnOnce(&mut Layout, usize)) {
        if self.layout.outputs.is_empty() {
            return;
        }
        let before = self.layout.clone();
        change(&mut self.layout, self.selected);
        if self.layout != before {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    /// Handle a key.
    pub(crate) fn key(&mut self, key: KeyEvent) -> Effect {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None if ctrl_c => self.quit(),
            Overlay::None => self.normal_key(key),
            Overlay::Help => Effect::None,
            Overlay::Modes(picker) => self.modes_key(picker, key.code),
            Overlay::Scales(picker) => self.scales_key(picker, key.code),
            Overlay::Adopt(lines) => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => Effect::Adopt,
                KeyCode::Char('n' | 'q') | KeyCode::Esc => Effect::None,
                _ => {
                    self.overlay = Overlay::Adopt(lines);
                    Effect::None
                }
            },
            Overlay::Profiles(dialog) => self.profiles_key(dialog, key),
            Overlay::Quit { live } => match key.code {
                KeyCode::Char('w') => self.checked(Effect::WriteThenQuit),
                KeyCode::Char('q') => Effect::Quit,
                KeyCode::Char('r') if live => Effect::RevertAndQuit,
                KeyCode::Esc | KeyCode::Char('c') => Effect::None,
                _ => {
                    self.overlay = Overlay::Quit { live };
                    Effect::None
                }
            },
            Overlay::Countdown(countdown) => {
                let effect = match key.code {
                    _ if ctrl_c => Effect::Cancel,
                    KeyCode::Char('y') | KeyCode::Enter => Effect::Confirm,
                    KeyCode::Char('n') | KeyCode::Esc => Effect::Cancel,
                    _ => Effect::None,
                };
                self.overlay = Overlay::Countdown(countdown);
                effect
            }
        }
    }

    fn quit(&mut self) -> Effect {
        if self.modified() || self.live_unsaved {
            self.overlay = Overlay::Quit {
                live: self.live_unsaved,
            };
            Effect::None
        } else {
            Effect::Quit
        }
    }

    fn normal_key(&mut self, key: KeyEvent) -> Effect {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let step = if shift { BIG_STEP } else { STEP };
        let count = self.layout.outputs.len().max(1);
        match key.code {
            KeyCode::Tab => self.selected = (self.selected + 1) % count,
            KeyCode::BackTab => self.selected = (self.selected + count - 1) % count,
            KeyCode::Char(c @ '1'..='9') => {
                let n = c as usize - '1' as usize;
                if n < self.layout.outputs.len() {
                    self.selected = n;
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.nudge(Direction::Left, step),
            KeyCode::Char('l') | KeyCode::Right => self.nudge(Direction::Right, step),
            KeyCode::Char('k') | KeyCode::Up => self.nudge(Direction::Up, step),
            KeyCode::Char('j') | KeyCode::Down => self.nudge(Direction::Down, step),
            KeyCode::Char('H') => self.nudge(Direction::Left, BIG_STEP),
            KeyCode::Char('L') => self.nudge(Direction::Right, BIG_STEP),
            KeyCode::Char('K') => self.nudge(Direction::Up, BIG_STEP),
            KeyCode::Char('J') => self.nudge(Direction::Down, BIG_STEP),
            KeyCode::Char('g') => {
                self.snap = !self.snap;
                let state = if self.snap { "on" } else { "off" };
                self.say(
                    Level::Info,
                    format!("snapping to neighbouring edges is {state}"),
                );
            }
            KeyCode::Char('b') => self.align(Align::Bottom),
            KeyCode::Char('t') => self.align(Align::Top),
            KeyCode::Char('c') => self.align(Align::Center),
            KeyCode::Char('r') if ctrl => self.redo(),
            KeyCode::Char('r') => self.edit(|l, i| l.rotate(i, true)),
            KeyCode::Char('R') => self.edit(|l, i| l.rotate(i, false)),
            KeyCode::Char('f') => self.edit(Layout::flip),
            KeyCode::Char('m') => self.open_modes(),
            KeyCode::Char('s') => self.open_scales(),
            KeyCode::Char('[') => self.step_refresh(false),
            KeyCode::Char(']') => self.step_refresh(true),
            KeyCode::Char('v') => self.cycle_vrr(),
            KeyCode::Char('e') => self.toggle_enabled(),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('U') => self.redo(),
            KeyCode::Char('o') if self.file.outside.is_empty() => {
                self.say(Level::Info, "there are no monitor rules outside the block");
            }
            KeyCode::Char('o') => self.overlay = Overlay::Adopt(self.file.outside.clone()),
            KeyCode::Char('p') => return Effect::ListProfiles,
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('a') => return self.checked(Effect::ApplyLive),
            KeyCode::Char('w') => return self.checked(Effect::Write),
            KeyCode::Char('q') | KeyCode::Esc => return self.quit(),
            _ => {}
        }
        Effect::None
    }

    /// Refuse to apply or write a layout with overlaps.
    fn checked(&mut self, effect: Effect) -> Effect {
        let blocking: Vec<String> = self
            .layout
            .problems()
            .iter()
            .filter(|p| p.is_blocking())
            .map(ToString::to_string)
            .collect();
        if !blocking.is_empty() {
            self.say(Level::Error, format!("fix first: {}", blocking.join("; ")));
            return Effect::None;
        }
        if effect == Effect::ApplyLive && self.offline {
            self.say(
                Level::Error,
                "Hyprland is not reachable, so nothing can be applied live",
            );
            return Effect::None;
        }
        effect
    }

    fn nudge(&mut self, dir: Direction, step: i32) {
        let snap = self.snap;
        self.edit(|l, i| l.nudge(i, dir, step, snap, BIG_STEP));
    }

    fn align(&mut self, align: Align) {
        let mut moved = false;
        self.edit(|l, i| moved = l.align(i, align));
        if !moved {
            self.say(Level::Info, "nothing to align with, or already aligned");
        }
    }

    fn step_refresh(&mut self, up: bool) {
        let mut result = None;
        self.edit(|l, i| result = l.step_refresh(i, up));
        if let Some(hz) = result {
            let name = self.name();
            self.say(Level::Info, format!("{name}: {} Hz", format_refresh(hz)));
        } else {
            let edge = if up { "highest" } else { "lowest" };
            self.say(
                Level::Info,
                format!("already the {edge} refresh rate of this resolution"),
            );
        }
    }

    fn cycle_vrr(&mut self) {
        let mut value = None;
        self.edit(|l, i| value = l.cycle_vrr(i));
        self.say(Level::Info, format!("VRR {}", vrr_name(value)));
    }

    fn toggle_enabled(&mut self) {
        let Some(output) = self.layout.outputs.get(self.selected) else {
            return;
        };
        let enabled = !output.rule.is_disabled();
        let others = (0..self.layout.outputs.len())
            .filter(|&k| k != self.selected && self.layout.is_active(k))
            .count();
        if enabled && others == 0 {
            self.say(Level::Error, "refusing to disable the only active monitor");
            return;
        }
        self.edit(|l, i| l.set_enabled(i, !enabled));
    }

    fn undo(&mut self) {
        match self.undo.pop() {
            Some(previous) => {
                self.redo
                    .push(std::mem::replace(&mut self.layout, previous));
                self.selected = self
                    .selected
                    .min(self.layout.outputs.len().saturating_sub(1));
            }
            None => self.say(Level::Info, "nothing to undo"),
        }
    }

    fn redo(&mut self) {
        match self.redo.pop() {
            Some(next) => self.undo.push(std::mem::replace(&mut self.layout, next)),
            None => self.say(Level::Info, "nothing to redo"),
        }
    }

    fn open_modes(&mut self) {
        let Some(output) = self.layout.outputs.get(self.selected) else {
            return;
        };
        let current = output.rule.mode.clone();
        let mut items: Vec<(String, Mode)> = self
            .layout
            .modes(self.selected)
            .into_iter()
            .map(|m| (mode_label(&m), m))
            .collect();
        for (label, keyword) in [
            ("preferred", Mode::Preferred),
            ("highest refresh rate (highrr)", Mode::HighRefreshRate),
            ("highest resolution (highres)", Mode::HighResolution),
        ] {
            items.push((label.to_owned(), keyword));
        }
        let cursor = current
            .as_ref()
            .and_then(|c| {
                items.iter().position(|(_, m)| {
                    m == c
                        || (m.resolution().is_some()
                            && m.resolution() == c.resolution()
                            && m.refresh()
                                .zip(c.refresh())
                                .is_some_and(|(a, b)| (a - b).abs() < 1.0))
                })
            })
            .unwrap_or(0);
        self.overlay = Overlay::Modes(Picker {
            title: format!("Mode of {}", output.info.name),
            items,
            cursor,
        });
    }

    fn modes_key(&mut self, mut picker: Picker<Mode>, code: KeyCode) -> Effect {
        match picker.key(code) {
            PickerKey::Moved => self.overlay = Overlay::Modes(picker),
            PickerKey::Closed => {}
            PickerKey::Chosen => {
                if let Some((_, mode)) = picker.items.get(picker.cursor).cloned() {
                    self.edit(|l, i| l.set_mode(i, &mode));
                }
            }
        }
        Effect::None
    }

    fn open_scales(&mut self) {
        let Some(output) = self.layout.outputs.get(self.selected) else {
            return;
        };
        let pixels = self.layout.pixels(self.selected);
        let transform = output.rule.transform_or_default();
        let mut items: Vec<(String, Scale)> = self
            .layout
            .scales(self.selected)
            .into_iter()
            .map(|s| {
                let (w, h) = hyprtilt_core::geometry::logical_size(pixels, transform, s as f32);
                (
                    format!("{:<9} {w}x{h}", hyprtilt_core::geometry::format_scale(s)),
                    Scale::Factor(s),
                )
            })
            .collect();
        items.push(("auto (by pixel density)".to_owned(), Scale::Auto));
        let effective = f64::from(self.layout.scale(self.selected).scale);
        let cursor = items
            .iter()
            .position(|(_, s)| match (s, output.rule.scale) {
                (Scale::Auto, None | Some(Scale::Auto)) => true,
                (Scale::Factor(f), Some(Scale::Factor(_))) => (f - effective).abs() < 1e-6,
                _ => false,
            })
            .unwrap_or(0);
        self.overlay = Overlay::Scales(Picker {
            title: format!("Scale of {} (scale, logical size)", output.info.name),
            items,
            cursor,
        });
    }

    fn scales_key(&mut self, mut picker: Picker<Scale>, code: KeyCode) -> Effect {
        match picker.key(code) {
            PickerKey::Moved => self.overlay = Overlay::Scales(picker),
            PickerKey::Closed => {}
            PickerKey::Chosen => {
                if let Some((_, scale)) = picker.items.get(picker.cursor).cloned() {
                    self.edit(|l, i| l.set_scale(i, scale));
                }
            }
        }
        Effect::None
    }

    fn profiles_key(&mut self, mut dialog: ProfileDialog, key: KeyEvent) -> Effect {
        if let Some(input) = &mut dialog.input {
            match key.code {
                KeyCode::Enter if !input.is_empty() => {
                    return Effect::SaveProfile(input.clone());
                }
                KeyCode::Esc => dialog.input = None,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') => {
                    input.push(c);
                }
                _ => {}
            }
            self.overlay = Overlay::Profiles(dialog);
            return Effect::None;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.cursor = (dialog.cursor + 1).min(dialog.names.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => dialog.cursor = dialog.cursor.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(name) = dialog.names.get(dialog.cursor) {
                    return Effect::LoadProfile(name.clone());
                }
            }
            KeyCode::Char('s' | 'n') => dialog.input = Some(String::new()),
            KeyCode::Esc | KeyCode::Char('q') => return Effect::None,
            _ => {}
        }
        self.overlay = Overlay::Profiles(dialog);
        Effect::None
    }

    /// Handle a mouse event: a click selects the monitor under the
    /// pointer, a drag moves it on a 10 px grid.
    pub(crate) fn mouse(&mut self, event: MouseEvent) {
        if self.overlay != Overlay::None {
            return;
        }
        let Some(placement) = &self.placement else {
            return;
        };
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(index) = placement.hit(event.column, event.row) else {
                    return;
                };
                self.selected = index;
                let Some(rect) = self.layout.rect(index) else {
                    return;
                };
                self.drag = Some(Drag {
                    index,
                    column: event.column,
                    row: event.row,
                    origin: (rect.x, rect.y),
                    before: self.layout.clone(),
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(drag) = &self.drag else {
                    return;
                };
                let (dx, dy) =
                    placement.delta((drag.column, drag.row), (event.column, event.row), STEP);
                let (x, y) = (drag.origin.0 + dx, drag.origin.1 + dy);
                let index = drag.index;
                self.layout.move_to(index, x, y);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(drag) = self.drag.take()
                    && drag.before != self.layout
                {
                    self.undo.push(drag.before);
                    self.redo.clear();
                }
            }
            _ => {}
        }
    }
}

/// `2560x1440 @ 179.95 Hz`.
pub(crate) fn mode_label(m: &Mode) -> String {
    match (m.resolution(), m.refresh()) {
        (Some((w, h)), Some(hz)) => format!("{w}x{h} @ {} Hz", format_refresh(hz)),
        _ => m.to_string(),
    }
}

/// A VRR value in words.
pub(crate) fn vrr_name(value: Option<i8>) -> &'static str {
    match value {
        None | Some(-1) => "unset (follows misc:vrr)",
        Some(0) => "off",
        Some(1) => "on",
        Some(2) => "fullscreen only",
        Some(_) => "fullscreen games and video",
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
