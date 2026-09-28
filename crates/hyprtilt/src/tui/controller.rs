// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Carrying out what the interface asks for: live changes and file writes
//! with verification and a countdown, adoption, profiles and hotplug.
//!
//! The controller gets its clock from the caller, so tests drive it with
//! the in-memory Hyprland and a fake time.

use std::time::{Duration, SystemTime};

use hyprtilt_core::apply::{
    Action, ApplyConfig, Effects, FileChange, Input, IpcChange, Outcome, Session, step,
};
use hyprtilt_core::config::Target;
use hyprtilt_core::document::{Backend, SaveOptions};
use hyprtilt_core::fsio;
use hyprtilt_core::ipc::events::Event;
use hyprtilt_core::ipc::{HyprlandIpc, MonitorInfo};
use hyprtilt_core::layout::{Layout, compare};
use hyprtilt_core::lua::syntax::Luac;
use hyprtilt_core::model::MonitorRule;
use hyprtilt_core::profile::{Profile, ProfileStore};
use hyprtilt_core::settings::Settings;

use super::app::{App, ChangeKind, Countdown, Effect, FileInfo, Level, Overlay};
use crate::commands::{Workspace, running_backend};
use crate::context::Context;
use crate::error::AppError;

/// What the controller works with.
pub(crate) struct Env<'a> {
    /// The connection to Hyprland, if it is reachable.
    pub(crate) ipc: Option<&'a dyn HyprlandIpc>,
    /// The file to edit.
    pub(crate) target: Target,
    /// Settings from config.toml.
    pub(crate) settings: Settings,
    /// The profile directory.
    pub(crate) profiles: ProfileStore,
    /// `$HOME`, for `~` in profiles.
    pub(crate) home: Option<String>,
    /// `--dry-run`: change nothing.
    pub(crate) dry_run: bool,
    /// The language of live changes.
    pub(crate) live_backend: Backend,
    /// What the writers need to know about Hyprland.
    pub(crate) save_options: SaveOptions,
    /// The Lua compiler for syntax checks.
    pub(crate) luac: Option<Luac>,
}

impl<'a> Env<'a> {
    /// Everything from the command's context.
    pub(crate) fn from_context(ctx: &'a Context) -> Result<Env<'a>, AppError> {
        let target = ctx.target();
        ctx.check_backend(&target)?;
        Ok(Env {
            ipc: ctx.ipc().ok(),
            live_backend: running_backend(ctx, target.backend),
            target,
            settings: ctx.settings.clone(),
            profiles: ctx.profiles(),
            home: ctx.home.clone(),
            dry_run: ctx.global.dry_run,
            save_options: ctx.save_options(),
            luac: Context::luac(),
        })
    }
}

/// Whether to go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Quit,
}

enum Change<'a> {
    Live(IpcChange<'a>),
    File(FileChange<'a>),
}

impl Change<'_> {
    fn effects(&mut self) -> &mut dyn Effects {
        match self {
            Change::Live(c) => c,
            Change::File(c) => c,
        }
    }
}

struct Running<'a> {
    kind: ChangeKind,
    session: Session,
    change: Change<'a>,
    quit_after: bool,
}

/// Carries out the effects of [`App`].
pub(crate) struct Controller<'a> {
    env: Env<'a>,
    running: Option<Running<'a>>,
    hotplug_pending: bool,
}

impl<'a> Controller<'a> {
    pub(crate) fn new(env: Env<'a>) -> Controller<'a> {
        Controller {
            env,
            running: None,
            hotplug_pending: false,
        }
    }

    fn open(&self) -> Result<Workspace, AppError> {
        Workspace::open_target(self.env.target.clone())
    }

    fn monitors(&self) -> Option<Vec<MonitorInfo>> {
        self.env.ipc.and_then(|ipc| ipc.monitors().ok())
    }

    fn file_info(ws: &Workspace) -> FileInfo {
        FileInfo {
            path: ws.display(),
            backend: ws.target.backend,
            rules: ws.block_rules().to_vec(),
            lines: ws
                .doc
                .block
                .as_ref()
                .map(|b| b.rule_lines.clone())
                .unwrap_or_default(),
            outside: ws
                .doc
                .outside
                .iter()
                .filter(|f| f.is_adoptable())
                .map(|f| f.line)
                .collect(),
        }
    }

    fn build(&self, rules: &[MonitorRule], monitors: Option<&[MonitorInfo]>) -> Layout {
        match monitors {
            Some(m) => Layout::new(m, rules, self.env.settings.use_descriptions),
            None => Layout::offline(rules),
        }
    }

    /// Read the file and the monitors and build the interface.
    pub(crate) fn load(&self) -> Result<App, AppError> {
        let ws = self.open()?;
        let monitors = match self.env.ipc {
            Some(ipc) => Some(ipc.monitors()?),
            None => None,
        };
        let layout = self.build(ws.block_rules(), monitors.as_deref());
        Ok(App::new(
            layout,
            Controller::file_info(&ws),
            monitors.is_none(),
            self.env.settings.snap,
        ))
    }

    fn confirm_config(&self) -> ApplyConfig {
        ApplyConfig::with_confirm_seconds(self.env.settings.confirm_timeout)
    }

    /// Carry out `effect` at time `now`.
    pub(crate) fn perform(&mut self, app: &mut App, effect: Effect, now: Duration) -> Flow {
        match effect {
            Effect::None => Flow::Continue,
            Effect::ApplyLive => {
                self.apply_live(app, now);
                Flow::Continue
            }
            Effect::Write => self.write(app, now, false),
            Effect::WriteThenQuit => self.write(app, now, true),
            Effect::Confirm => self.feed(app, Input::Confirm, now),
            Effect::Cancel => self.feed(app, Input::Cancel, now),
            Effect::Adopt => {
                self.adopt(app);
                Flow::Continue
            }
            Effect::ListProfiles => {
                match self.env.profiles.list() {
                    Ok(names) => app.show_profiles(names),
                    Err(e) => app.say(Level::Error, e.to_string()),
                }
                Flow::Continue
            }
            Effect::LoadProfile(name) => {
                self.load_profile(app, &name);
                Flow::Continue
            }
            Effect::SaveProfile(name) => {
                self.save_profile(app, &name);
                Flow::Continue
            }
            Effect::Quit => Flow::Quit,
            Effect::RevertAndQuit => match self.env.ipc.map(HyprlandIpc::reload) {
                Some(Err(e)) => {
                    app.say(Level::Error, format!("reloading the file failed: {e}"));
                    Flow::Continue
                }
                _ => Flow::Quit,
            },
        }
    }

    /// Let the clock move on: time out a countdown or poll Hyprland.
    pub(crate) fn tick(&mut self, app: &mut App, now: Duration) -> Flow {
        self.feed(app, Input::Tick, now)
    }

    /// A signal asks to stop: roll back a pending change and quit.
    pub(crate) fn signal(&mut self, app: &mut App, now: Duration) {
        self.feed(app, Input::Signal, now);
    }

    fn feed(&mut self, app: &mut App, input: Input, now: Duration) -> Flow {
        let Some(mut running) = self.running.take() else {
            return Flow::Continue;
        };
        let outcome = step(
            &mut running.session,
            Some(input),
            now,
            running.change.effects(),
        );
        self.proceed(app, running, outcome, now)
    }

    /// Finish the session with `outcome`, or keep it running and show
    /// where it is.
    fn proceed(
        &mut self,
        app: &mut App,
        running: Running<'a>,
        outcome: Option<Outcome>,
        now: Duration,
    ) -> Flow {
        if let Some(outcome) = outcome {
            return self.finish(app, &running, outcome);
        }
        app.overlay = Overlay::Countdown(Countdown {
            kind: running.kind,
            phase: running.session.phase(),
            remaining: running.session.remaining(now),
        });
        self.running = Some(running);
        Flow::Continue
    }

    fn start(
        &mut self,
        app: &mut App,
        kind: ChangeKind,
        mut change: Change<'a>,
        config: ApplyConfig,
        now: Duration,
        quit_after: bool,
    ) -> Flow {
        let mut session = Session::new(app.layout.expectations(), config);
        let outcome = step(&mut session, None, now, change.effects());
        let running = Running {
            kind,
            session,
            change,
            quit_after,
        };
        self.proceed(app, running, outcome, now)
    }

    fn finish(&mut self, app: &mut App, running: &Running<'a>, outcome: Outcome) -> Flow {
        if matches!(app.overlay, Overlay::Countdown(_)) {
            app.overlay = Overlay::None;
        }
        let mut flow = Flow::Continue;
        match (&running.change, outcome) {
            (Change::Live(_), Outcome::Kept) => {
                app.live_unsaved = true;
                app.say(
                    Level::Success,
                    "applied live and kept; w writes it into the file",
                );
            }
            (Change::File(change), Outcome::Kept) => {
                match self.open() {
                    Ok(ws) => app.mark_saved(Controller::file_info(&ws)),
                    Err(e) => app.say(Level::Error, e.message().unwrap_or_default()),
                }
                let backup = change
                    .backup
                    .as_ref()
                    .map(|b| format!(" (backup: {})", b.display()))
                    .unwrap_or_default();
                app.say(Level::Success, format!("wrote {}{backup}", app.file.path));
                if running.quit_after {
                    flow = Flow::Quit;
                }
            }
            (_, Outcome::RolledBack { reason }) => {
                let what = match running.kind {
                    ChangeKind::Live => "the live change was rolled back",
                    ChangeKind::Write => "the file was restored",
                };
                app.say(Level::Error, format!("{what}: {reason}"));
            }
            (_, Outcome::RollbackFailed { reason, error }) => {
                app.say(
                    Level::Error,
                    format!(
                        "the change was not kept ({reason}) and rolling it back failed: {error}"
                    ),
                );
            }
        }
        if let Some(monitors) = self.monitors() {
            if self.hotplug_pending || !same_outputs(app, &monitors) {
                app.rebuild(&monitors, self.env.settings.use_descriptions);
            } else {
                app.update_live(&monitors);
            }
        }
        self.hotplug_pending = false;
        flow
    }

    fn apply_live(&mut self, app: &mut App, now: Duration) {
        let Some(ipc) = self.env.ipc else {
            app.say(Level::Error, "Hyprland is not reachable");
            return;
        };
        let result = (|| -> Result<(Action, Action), AppError> {
            let current = Layout::new(&ipc.monitors()?, &[], false);
            let previous: Vec<MonitorRule> = (0..current.outputs.len())
                .map(|i| current.live_rule(i))
                .collect();
            let rules: Vec<MonitorRule> = (0..app.layout.outputs.len())
                .map(|i| app.layout.live_rule(i))
                .collect();
            Ok((
                Action::set(self.env.live_backend, &rules)?,
                Action::set(self.env.live_backend, &previous)?,
            ))
        })();
        let (apply, restore) = match result {
            Ok(actions) => actions,
            Err(e) => {
                app.say(Level::Error, e.message().unwrap_or_default());
                return;
            }
        };
        if self.env.dry_run {
            app.say(
                Level::Info,
                format!("--dry-run: would send {}", apply.requests().join("; ")),
            );
            return;
        }
        let config = self.confirm_config();
        let change = Change::Live(IpcChange {
            ipc,
            apply,
            restore,
        });
        self.start(app, ChangeKind::Live, change, config, now, false);
    }

    fn write(&mut self, app: &mut App, now: Duration, quit_after: bool) -> Flow {
        let result = (|| -> Result<(Workspace, String), AppError> {
            let ws = self.open()?;
            let content = ws
                .target
                .backend
                .save(
                    ws.snapshot.text(),
                    &app.layout.rules(),
                    &self.env.save_options,
                )?
                .content;
            Ok((ws, content))
        })();
        let (ws, content) = match result {
            Ok(r) => r,
            Err(e) => {
                app.say(Level::Error, e.message().unwrap_or_default());
                return Flow::Continue;
            }
        };
        if content == ws.snapshot.text() {
            app.mark_saved(Controller::file_info(&ws));
            app.say(Level::Info, "the file already holds this layout");
            return if quit_after {
                Flow::Quit
            } else {
                Flow::Continue
            };
        }
        if self.env.dry_run {
            let changed = fsio::diff(ws.snapshot.text(), &content, "")
                .lines()
                .filter(|l| {
                    l.starts_with(['+', '-']) && !l.starts_with("+++") && !l.starts_with("---")
                })
                .count();
            app.say(
                Level::Info,
                format!(
                    "--dry-run: {changed} line(s) of {} would change",
                    ws.display()
                ),
            );
            return Flow::Continue;
        }
        if ws.target.backend == Backend::Lua
            && let Some(luac) = &self.env.luac
            && let Err(e) = luac.check_regression(ws.snapshot.text(), &content)
        {
            app.say(
                Level::Error,
                format!("the new content of {} does not compile: {e}", ws.display()),
            );
            return Flow::Continue;
        }
        let backups = self.env.settings.backup_policy();
        let Some(ipc) = self.env.ipc else {
            return match fsio::replace(&ws.snapshot, &content, Some(&backups), SystemTime::now()) {
                Ok(_) => {
                    if let Ok(ws) = self.open() {
                        app.mark_saved(Controller::file_info(&ws));
                    }
                    app.say(
                        Level::Success,
                        format!(
                            "wrote {}; Hyprland is not reachable, so nothing was applied",
                            ws.display()
                        ),
                    );
                    if quit_after {
                        Flow::Quit
                    } else {
                        Flow::Continue
                    }
                }
                Err(e) => {
                    app.say(Level::Error, e.to_string());
                    Flow::Continue
                }
            };
        };
        // A layout that is already running needs no confirmation.
        let running = self
            .monitors()
            .is_some_and(|m| compare(&app.layout.expectations(), &m).is_empty());
        let config = if running {
            ApplyConfig::with_confirm_seconds(0)
        } else {
            self.confirm_config()
        };
        let change = Change::File(FileChange::new(
            ipc,
            ws.snapshot.clone(),
            content,
            Some(backups),
            SystemTime::now(),
        ));
        self.start(app, ChangeKind::Write, change, config, now, quit_after)
    }

    fn adopt(&mut self, app: &mut App) {
        if app.modified() {
            app.say(
                Level::Error,
                "write or undo the changes first; adopting reads the layout from the file again",
            );
            return;
        }
        let result = (|| -> Result<Option<String>, AppError> {
            let ws = self.open()?;
            let edit = ws.target.backend.adopt(ws.snapshot.text(), &[])?;
            if !edit.changed {
                return Ok(None);
            }
            if self.env.dry_run {
                return Ok(Some("--dry-run: the rules are not adopted".to_owned()));
            }
            if ws.target.backend == Backend::Lua
                && let Some(luac) = &self.env.luac
            {
                luac.check_regression(ws.snapshot.text(), &edit.content)
                    .map_err(|e| {
                        AppError::Config(format!("the adopted file does not compile: {e}"))
                    })?;
            }
            let report = fsio::replace(
                &ws.snapshot,
                &edit.content,
                Some(&self.env.settings.backup_policy()),
                SystemTime::now(),
            )?;
            if let Some(ipc) = self.env.ipc {
                // The layout is the same; a failed reload is not worth failing for.
                let _ = ipc.reload();
            }
            let backup = report
                .backup
                .map(|b| format!(" (backup: {})", b.display()))
                .unwrap_or_default();
            Ok(Some(format!("adopted the rules into the block{backup}")))
        })();
        match result {
            Ok(None) => app.say(Level::Info, "no monitor rules to adopt"),
            Ok(Some(text)) if self.env.dry_run => app.say(Level::Info, text),
            Ok(Some(text)) => match self.open() {
                Ok(ws) => {
                    let monitors = self.monitors();
                    let layout = self.build(ws.block_rules(), monitors.as_deref());
                    let selected = app.selected;
                    app.layout = layout;
                    app.selected = selected.min(app.layout.outputs.len().saturating_sub(1));
                    app.mark_saved(Controller::file_info(&ws));
                    app.say(Level::Success, text);
                }
                Err(e) => app.say(Level::Error, e.message().unwrap_or_default()),
            },
            Err(e) => app.say(Level::Error, e.message().unwrap_or_default()),
        }
    }

    fn load_profile(&mut self, app: &mut App, name: &str) {
        match self.env.profiles.load(name, self.env.home.as_deref()) {
            Ok(profile) => {
                let layout = self.build(&profile.monitors, self.monitors().as_deref());
                app.load_layout(layout, name);
            }
            Err(e) => app.say(Level::Error, e.to_string()),
        }
    }

    fn save_profile(&mut self, app: &mut App, name: &str) {
        let profile = Profile {
            backend: Some(self.env.target.backend),
            target: None,
            monitors: app.layout.outputs.iter().map(|o| o.rule.clone()).collect(),
        };
        if self.env.dry_run {
            app.say(
                Level::Info,
                format!("--dry-run: profile {name:?} is not saved"),
            );
            return;
        }
        match self.env.profiles.save(name, &profile, SystemTime::now()) {
            Ok(path) => app.say(
                Level::Success,
                format!("saved profile {name:?} in {}", path.display()),
            ),
            Err(e) => app.say(Level::Error, e.to_string()),
        }
    }

    /// A monitor was connected or disconnected, or the configuration was
    /// reloaded by someone else.
    pub(crate) fn event(&mut self, app: &mut App, event: &Event) {
        if self.running.is_some() {
            // The session's own reloads end up here too; rebuild after it.
            if !matches!(event, Event::ConfigReloaded) {
                self.hotplug_pending = true;
            }
            return;
        }
        let Some(monitors) = self.monitors() else {
            return;
        };
        // Disabling a monitor also sends monitorremoved, and `monitors
        // all` still lists it: only a changed set of outputs is a hotplug.
        if same_outputs(app, &monitors) {
            app.update_live(&monitors);
            return;
        }
        app.rebuild(&monitors, self.env.settings.use_descriptions);
        match event {
            Event::MonitorAdded(name) => app.say(Level::Info, format!("{name} was connected")),
            Event::MonitorRemoved(name) => {
                app.say(Level::Info, format!("{name} was disconnected"));
            }
            Event::ConfigReloaded => {}
        }
    }
}

fn same_outputs(app: &App, monitors: &[MonitorInfo]) -> bool {
    app.layout.outputs.len() == monitors.len()
        && app
            .layout
            .outputs
            .iter()
            .all(|o| monitors.iter().any(|m| m.name == o.info.name))
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
