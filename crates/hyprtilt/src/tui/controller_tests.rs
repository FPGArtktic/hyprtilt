// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

use std::path::PathBuf;

use hyprtilt_core::apply::Phase;
use hyprtilt_core::config::TargetReason;
use hyprtilt_core::ipc::fake::{FakeHyprland, FakeSetup};

use super::*;
use crate::tui::testing::{HYPR_USER, managed, monitors, type_keys};

/// A temporary directory with `hypr-user.lua` and a compositor that has
/// loaded it.
struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
    fake: FakeHyprland,
}

impl Fixture {
    fn new(content: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hypr-user.lua");
        std::fs::write(&path, content).unwrap();
        let fake = FakeHyprland::new(FakeSetup {
            version: "0.56.2".to_owned(),
            provider: Backend::Lua,
            config: Some(path.clone()),
            base_rules: Vec::new(),
            monitors: monitors(),
        });
        fake.reload().unwrap();
        Fixture { dir, path, fake }
    }

    fn env(&self, connected: bool) -> Env<'_> {
        Env {
            ipc: connected.then_some(&self.fake as &dyn HyprlandIpc),
            target: Target {
                path: self.path.clone(),
                backend: Backend::Lua,
                reason: TargetReason::Explicit,
                warnings: Vec::new(),
            },
            settings: Settings::default(),
            profiles: ProfileStore::new(self.dir.path().join("profiles")),
            home: None,
            dry_run: false,
            live_backend: Backend::Lua,
            save_options: SaveOptions::default(),
            luac: None,
        }
    }

    fn content(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap()
    }

    fn backups(&self) -> usize {
        std::fs::read_dir(self.dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".bak.")
            })
            .count()
    }

    fn live_refresh(&self, name: &str) -> f64 {
        self.fake
            .current()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap()
            .refresh_rate
    }
}

const T0: Duration = Duration::ZERO;

fn message(app: &App) -> (Level, String) {
    let m = app.message.clone().unwrap();
    (m.level, m.text)
}

fn phase(app: &App) -> Option<Phase> {
    match &app.overlay {
        Overlay::Countdown(c) => Some(c.phase),
        _ => None,
    }
}

#[test]
fn loading_builds_the_interface() {
    let f = Fixture::new(&managed());
    let c = Controller::new(f.env(true));
    let app = c.load().unwrap();
    assert_eq!(app.layout.outputs.len(), 3);
    assert!(!app.offline && !app.modified());
    assert_eq!(app.file.lines.len(), 3);
    assert_eq!(app.overlay, Overlay::None);
    let c = Controller::new(f.env(false));
    let app = c.load().unwrap();
    assert!(app.offline);
    assert_eq!(app.layout.outputs.len(), 3);
    // Without a block the rules outside are offered for adoption.
    let f = Fixture::new(HYPR_USER);
    let app = Controller::new(f.env(true)).load().unwrap();
    assert_eq!(app.overlay, Overlay::Adopt(vec![5, 6, 7]));
}

#[test]
fn a_live_change_is_kept_when_confirmed() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    assert_eq!(c.perform(&mut app, Effect::ApplyLive, T0), Flow::Continue);
    assert_eq!(phase(&app), Some(Phase::Confirming));
    assert!((f.live_refresh("DP-1") - 164.98).abs() < 0.01);
    assert!(
        f.fake
            .requests()
            .iter()
            .any(|r| r.starts_with("/eval hl.monitor("))
    );
    let t = Duration::from_secs(5);
    assert_eq!(c.tick(&mut app, t), Flow::Continue);
    let Overlay::Countdown(countdown) = &app.overlay else {
        panic!()
    };
    assert_eq!(countdown.remaining, Some(Duration::from_secs(10)));
    assert_eq!(c.perform(&mut app, Effect::Confirm, t), Flow::Continue);
    assert_eq!(app.overlay, Overlay::None);
    assert!(app.live_unsaved);
    assert_eq!(message(&app).0, Level::Success);
    assert!((f.live_refresh("DP-1") - 164.98).abs() < 0.01);
    // The file is untouched.
    assert_eq!(f.content(), managed());
    assert!(app.modified());
}

#[test]
fn a_live_change_is_rolled_back_without_confirmation() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    c.perform(&mut app, Effect::ApplyLive, T0);
    c.tick(&mut app, Duration::from_secs(16));
    assert_eq!(app.overlay, Overlay::None);
    let (level, text) = message(&app);
    assert_eq!(level, Level::Error);
    assert_eq!(
        text,
        "the live change was rolled back: not confirmed in time"
    );
    assert!((f.live_refresh("DP-1") - 179.95).abs() < 0.01);
    assert!(!app.live_unsaved);
    // The edit stays in the editor.
    assert!(app.modified());
}

#[test]
fn a_signal_rolls_back_and_cancel_too() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    c.perform(&mut app, Effect::ApplyLive, T0);
    c.signal(&mut app, T0);
    assert!((f.live_refresh("DP-1") - 179.95).abs() < 0.01);
    assert!(message(&app).1.contains("rolled back"));
    c.perform(&mut app, Effect::ApplyLive, T0);
    c.perform(&mut app, Effect::Cancel, T0);
    assert!((f.live_refresh("DP-1") - 179.95).abs() < 0.01);
    // Confirming with nothing running does nothing.
    assert_eq!(c.perform(&mut app, Effect::Confirm, T0), Flow::Continue);
    assert_eq!(c.tick(&mut app, T0), Flow::Continue);
}

#[test]
fn writing_is_kept_when_confirmed() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    c.perform(&mut app, Effect::Write, T0);
    assert_eq!(phase(&app), Some(Phase::Confirming));
    assert!(f.content().contains("2560x1440@164.98"));
    c.perform(&mut app, Effect::Confirm, T0);
    assert_eq!(app.overlay, Overlay::None);
    let (level, text) = message(&app);
    assert_eq!(level, Level::Success, "{text}");
    assert!(text.contains("(backup: "), "{text}");
    assert!(!app.modified());
    assert_eq!(f.backups(), 1);
    assert!(!app.file.line_of(&app.layout.outputs[2].rule).unwrap().1);
    // Writing again changes nothing.
    c.perform(&mut app, Effect::Write, T0);
    assert_eq!(message(&app).1, "the file already holds this layout");
}

#[test]
fn a_cancelled_write_restores_the_file() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "1r");
    c.perform(&mut app, Effect::Write, T0);
    assert_ne!(f.content(), managed());
    c.perform(&mut app, Effect::Cancel, T0);
    assert_eq!(f.content(), managed());
    assert_eq!(message(&app).1, "the file was restored: rejected");
    assert!(app.modified());
}

#[test]
fn writing_the_running_layout_needs_no_confirmation() {
    let f = Fixture::new(HYPR_USER);
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    app.overlay = Overlay::None;
    // Every output is new: the block does not exist yet.
    assert!(app.file.rules.is_empty());
    c.perform(&mut app, Effect::Write, T0);
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(message(&app).0, Level::Success);
    assert!(f.content().contains("-- BEGIN hyprtilt (managed)"));
    assert_eq!(app.file.rules.len(), 3);
}

#[test]
fn write_then_quit_quits_once_kept() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    assert_eq!(
        c.perform(&mut app, Effect::WriteThenQuit, T0),
        Flow::Continue
    );
    assert_eq!(c.perform(&mut app, Effect::Confirm, T0), Flow::Quit);
    // Nothing to write: quit at once.
    let mut app = c.load().unwrap();
    assert_eq!(c.perform(&mut app, Effect::WriteThenQuit, T0), Flow::Quit);
}

#[test]
fn offline_writes_go_straight_to_the_file() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(false));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3l");
    assert_eq!(c.perform(&mut app, Effect::Write, T0), Flow::Continue);
    assert!(f.content().contains("3370x975"), "{}", f.content());
    assert!(message(&app).1.contains("nothing was applied"));
    assert!(!app.modified());
    assert_eq!(c.perform(&mut app, Effect::ApplyLive, T0), Flow::Continue);
    assert_eq!(message(&app).1, "Hyprland is not reachable");
    type_keys(&mut app, "l");
    assert_eq!(c.perform(&mut app, Effect::WriteThenQuit, T0), Flow::Quit);
}

#[test]
fn a_dry_run_changes_nothing() {
    let f = Fixture::new(HYPR_USER);
    let mut env = f.env(true);
    env.dry_run = true;
    let mut c = Controller::new(env);
    let mut app = c.load().unwrap();
    app.overlay = Overlay::None;
    type_keys(&mut app, "3[");
    let before = f.fake.requests().len();
    c.perform(&mut app, Effect::ApplyLive, T0);
    assert!(
        message(&app)
            .1
            .starts_with("--dry-run: would send /eval hl.monitor(")
    );
    c.perform(&mut app, Effect::Write, T0);
    assert!(message(&app).1.contains("line(s) of"), "{:?}", app.message);
    c.perform(&mut app, Effect::SaveProfile("desk".to_owned()), T0);
    assert!(message(&app).1.contains("not saved"));
    type_keys(&mut app, "u");
    c.perform(&mut app, Effect::Adopt, T0);
    assert_eq!(message(&app).1, "--dry-run: the rules are not adopted");
    assert_eq!(f.content(), HYPR_USER);
    assert!(
        f.fake.requests()[before..]
            .iter()
            .all(|r| r.starts_with('j'))
    );
}

#[test]
fn adopting_moves_the_rules_into_the_block() {
    let f = Fixture::new(HYPR_USER);
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    let effect = type_keys(&mut app, "y");
    assert_eq!(effect, Effect::Adopt);
    c.perform(&mut app, effect, T0);
    assert_eq!(message(&app).0, Level::Success, "{:?}", app.message);
    assert_eq!(f.content(), managed());
    assert!(app.file.outside.is_empty());
    assert_eq!(app.file.lines.len(), 3);
    assert!(!app.modified());
    assert_eq!(f.backups(), 1);
    c.perform(&mut app, Effect::Adopt, T0);
    assert_eq!(message(&app).1, "no monitor rules to adopt");
    type_keys(&mut app, "1r");
    c.perform(&mut app, Effect::Adopt, T0);
    assert!(
        message(&app)
            .1
            .starts_with("write or undo the changes first")
    );
}

#[test]
fn profiles_round_trip() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    c.perform(&mut app, Effect::ListProfiles, T0);
    assert!(matches!(&app.overlay, Overlay::Profiles(d) if d.names.is_empty()));
    type_keys(&mut app, "3[");
    let edited = app.layout.rules();
    c.perform(&mut app, Effect::SaveProfile("desk".to_owned()), T0);
    assert_eq!(message(&app).0, Level::Success);
    assert!(f.dir.path().join("profiles/desk.toml").exists());
    c.perform(&mut app, Effect::ListProfiles, T0);
    assert!(matches!(&app.overlay, Overlay::Profiles(d) if d.names == ["desk"]));
    type_keys(&mut app, "u");
    assert!(!app.modified());
    c.perform(&mut app, Effect::LoadProfile("desk".to_owned()), T0);
    assert_eq!(app.layout.rules(), edited);
    c.perform(&mut app, Effect::LoadProfile("nope".to_owned()), T0);
    assert_eq!(message(&app).0, Level::Error);
    c.perform(&mut app, Effect::SaveProfile("../x".to_owned()), T0);
    assert_eq!(message(&app).0, Level::Error);
}

#[test]
fn hotplug_rebuilds_the_layout() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    let mut fewer = monitors();
    fewer.retain(|m| m.name != "HDMI-A-1");
    f.fake.set_monitors(fewer);
    c.event(&mut app, &Event::MonitorRemoved("HDMI-A-1".to_owned()));
    assert_eq!(app.layout.outputs.len(), 2);
    assert_eq!(message(&app).1, "HDMI-A-1 was disconnected");
    f.fake.set_monitors(monitors());
    c.event(&mut app, &Event::MonitorAdded("HDMI-A-1".to_owned()));
    assert_eq!(app.layout.outputs.len(), 3);
    assert_eq!(message(&app).1, "HDMI-A-1 was connected");
    // A reload or a disabled monitor only updates the live facts.
    c.event(&mut app, &Event::ConfigReloaded);
    c.event(&mut app, &Event::MonitorRemoved("DP-1".to_owned()));
    assert_eq!(app.layout.outputs.len(), 3);
    assert!(!app.modified());
}

#[test]
fn hotplug_during_a_change_waits_for_its_end() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    c.perform(&mut app, Effect::ApplyLive, T0);
    let mut fewer = monitors();
    fewer.retain(|m| m.name != "HDMI-A-1");
    c.event(&mut app, &Event::MonitorRemoved("HDMI-A-1".to_owned()));
    c.event(&mut app, &Event::ConfigReloaded);
    assert_eq!(app.layout.outputs.len(), 3);
    f.fake.set_monitors(fewer);
    c.perform(&mut app, Effect::Confirm, T0);
    assert_eq!(app.layout.outputs.len(), 2);
}

#[test]
fn quitting() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    assert_eq!(c.perform(&mut app, Effect::None, T0), Flow::Continue);
    assert_eq!(c.perform(&mut app, Effect::Quit, T0), Flow::Quit);
    assert_eq!(c.perform(&mut app, Effect::RevertAndQuit, T0), Flow::Quit);
    assert_eq!(
        f.fake.requests().last().map(String::as_str),
        Some("/reload")
    );
    f.fake.script("/reload", Err("broken".to_owned()));
    assert_eq!(
        c.perform(&mut app, Effect::RevertAndQuit, T0),
        Flow::Continue
    );
    assert!(message(&app).1.starts_with("reloading the file failed"));
}

#[test]
fn a_failed_apply_is_reported() {
    let f = Fixture::new(&managed());
    let mut c = Controller::new(f.env(true));
    let mut app = c.load().unwrap();
    type_keys(&mut app, "3[");
    f.fake.script("/eval", Err("syntax error".to_owned()));
    c.perform(&mut app, Effect::ApplyLive, T0);
    assert_eq!(app.overlay, Overlay::None);
    let (level, text) = message(&app);
    assert_eq!(level, Level::Error);
    assert!(text.contains("syntax error"), "{text}");
}
