// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The apply, verify and rollback state machine. Time is an input, so
//! the countdown is testable without sleeping.
//!
//! A [`Session`] goes through these phases (ADR 0001, D8):
//!
//! ```text
//! Applying -> Verifying -> Confirming -> Finished(kept)
//!     \            \            \
//!      +------------+------------+--> RollingBack -> Finished(rolled back)
//! ```
//!
//! The session never does anything itself. It takes [`Input`]s (the result
//! of the last command, a tick of the clock, the user's answer, a signal)
//! and returns [`Command`]s for the caller to carry out. [`step`] does that
//! with an [`Effects`] implementation; [`IpcChange`] and [`FileChange`] are
//! the two kinds of change hyprtilt makes.
//!
//! Hyprland answers `eval` before the monitors change, so success is never
//! assumed: the monitor list is polled until it matches the expectation or
//! the verification time runs out.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::document::{Backend, ConfigError};
use crate::fsio::{self, BackupPolicy, Snapshot};
use crate::ipc::{HyprlandIpc, MonitorInfo};
use crate::layout::{Expectation, Mismatch, compare};
use crate::model::MonitorRule;

/// Timing of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyConfig {
    /// How long the user has to confirm; `None` keeps a verified change
    /// without asking.
    pub confirm: Option<Duration>,
    /// How long to wait for Hyprland to show the expected state.
    pub verify_timeout: Duration,
    /// How often to ask for the monitor list while verifying.
    pub poll_interval: Duration,
}

impl Default for ApplyConfig {
    fn default() -> Self {
        ApplyConfig {
            confirm: Some(Duration::from_secs(15)),
            verify_timeout: Duration::from_secs(3),
            poll_interval: Duration::from_millis(100),
        }
    }
}

impl ApplyConfig {
    /// The default timing with a confirmation time in seconds; 0 means no
    /// confirmation.
    #[must_use]
    pub fn with_confirm_seconds(seconds: u64) -> ApplyConfig {
        ApplyConfig {
            confirm: (seconds > 0).then(|| Duration::from_secs(seconds)),
            ..ApplyConfig::default()
        }
    }
}

/// Something that happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The last `Apply` or `Restore` finished.
    Done(Result<(), String>),
    /// The monitor list asked for with `Query`.
    Observed(Result<Vec<MonitorInfo>, String>),
    /// Time passed.
    Tick,
    /// The user keeps the change.
    Confirm,
    /// The user rejects the change.
    Cancel,
    /// The process got `SIGINT` or `SIGTERM`.
    Signal,
}

/// What the caller has to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Make the change.
    Apply,
    /// Ask Hyprland for the monitor list.
    Query,
    /// Undo the change.
    Restore,
    /// The session is over.
    Finish(Outcome),
}

/// Why a change was undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "reason", content = "details")]
pub enum Reason {
    /// Nobody confirmed it in time.
    NotConfirmed,
    /// The user rejected it.
    Cancelled,
    /// hyprtilt was interrupted or terminated.
    Signal,
    /// Making the change failed.
    ApplyFailed(String),
    /// Hyprland did not show the expected state in time.
    Verification(Vec<Mismatch>),
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::NotConfirmed => f.write_str("not confirmed in time"),
            Reason::Cancelled => f.write_str("rejected"),
            Reason::Signal => f.write_str("interrupted"),
            Reason::ApplyFailed(e) => write!(f, "applying failed: {e}"),
            Reason::Verification(m) => {
                let list: Vec<String> = m.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "Hyprland does not show the expected layout: {}",
                    list.join("; ")
                )
            }
        }
    }
}

/// How a session ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum Outcome {
    /// The change stays.
    Kept,
    /// The change was undone.
    RolledBack {
        /// Why.
        reason: Reason,
    },
    /// Undoing the change failed; the state is unknown.
    RollbackFailed {
        /// Why it was undone.
        reason: Reason,
        /// What went wrong while undoing it.
        error: String,
    },
}

/// Where a session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// Not started.
    Ready,
    /// Waiting for the change to be made.
    Applying,
    /// Waiting for Hyprland to show the expected state.
    Verifying,
    /// Waiting for the user.
    Confirming,
    /// Waiting for the change to be undone.
    RollingBack,
    /// Over.
    Finished,
}

/// One apply, verify and confirm cycle.
#[derive(Debug, Clone)]
pub struct Session {
    config: ApplyConfig,
    expected: Vec<Expectation>,
    phase: Phase,
    deadline: Duration,
    next_poll: Duration,
    querying: bool,
    mismatches: Vec<Mismatch>,
    reason: Option<Reason>,
    outcome: Option<Outcome>,
}

impl Session {
    /// A session that expects `expected` after the change.
    #[must_use]
    pub fn new(expected: Vec<Expectation>, config: ApplyConfig) -> Session {
        Session {
            config,
            expected,
            phase: Phase::Ready,
            deadline: Duration::ZERO,
            next_poll: Duration::ZERO,
            querying: false,
            mismatches: Vec::new(),
            reason: None,
            outcome: None,
        }
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Time left to confirm, while confirming.
    #[must_use]
    pub fn remaining(&self, now: Duration) -> Option<Duration> {
        (self.phase == Phase::Confirming).then(|| self.deadline.saturating_sub(now))
    }

    /// The differences found by the last check of the monitor list.
    #[must_use]
    pub fn mismatches(&self) -> &[Mismatch] {
        &self.mismatches
    }

    /// How the session ended, once it has.
    #[must_use]
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    /// Start at time `now`.
    pub fn start(&mut self, now: Duration) -> Vec<Command> {
        if self.phase != Phase::Ready {
            return Vec::new();
        }
        self.phase = Phase::Applying;
        self.deadline = now;
        vec![Command::Apply]
    }

    /// React to `input` at time `now` (measured from any fixed point).
    pub fn handle(&mut self, input: Input, now: Duration) -> Vec<Command> {
        match (self.phase, input) {
            (Phase::RollingBack, Input::Done(result)) => {
                let reason = self.reason.clone().unwrap_or(Reason::Cancelled);
                self.finish(match result {
                    Ok(()) => Outcome::RolledBack { reason },
                    Err(error) => Outcome::RollbackFailed { reason, error },
                })
            }
            (Phase::Ready | Phase::Finished | Phase::RollingBack, _) => Vec::new(),
            (_, Input::Signal) => self.roll_back(Reason::Signal),
            (_, Input::Cancel) => self.roll_back(Reason::Cancelled),
            (Phase::Applying, Input::Done(Ok(()))) => {
                self.phase = Phase::Verifying;
                self.deadline = now + self.config.verify_timeout;
                self.query(now)
            }
            (Phase::Applying, Input::Done(Err(e))) => self.roll_back(Reason::ApplyFailed(e)),
            (Phase::Verifying, Input::Observed(result)) => self.observed(result, now),
            (Phase::Verifying, Input::Tick) => {
                if now >= self.deadline && !self.querying {
                    self.roll_back(Reason::Verification(self.mismatches.clone()))
                } else if now >= self.next_poll && !self.querying {
                    self.query(now)
                } else {
                    Vec::new()
                }
            }
            (Phase::Confirming, Input::Confirm) => self.finish(Outcome::Kept),
            (Phase::Confirming, Input::Tick) if now >= self.deadline => {
                self.roll_back(Reason::NotConfirmed)
            }
            _ => Vec::new(),
        }
    }

    fn query(&mut self, now: Duration) -> Vec<Command> {
        self.querying = true;
        self.next_poll = now + self.config.poll_interval;
        vec![Command::Query]
    }

    fn observed(
        &mut self,
        result: Result<Vec<MonitorInfo>, String>,
        now: Duration,
    ) -> Vec<Command> {
        self.querying = false;
        self.mismatches = match result {
            Ok(monitors) => compare(&self.expected, &monitors),
            Err(e) => vec![Mismatch {
                output: String::new(),
                field: "monitor list",
                expected: "a reply".to_owned(),
                observed: e,
            }],
        };
        if self.mismatches.is_empty() {
            return match self.config.confirm {
                Some(wait) => {
                    self.phase = Phase::Confirming;
                    self.deadline = now + wait;
                    Vec::new()
                }
                None => self.finish(Outcome::Kept),
            };
        }
        if now >= self.deadline {
            return self.roll_back(Reason::Verification(self.mismatches.clone()));
        }
        Vec::new()
    }

    fn roll_back(&mut self, reason: Reason) -> Vec<Command> {
        self.phase = Phase::RollingBack;
        self.reason = Some(reason);
        vec![Command::Restore]
    }

    fn finish(&mut self, outcome: Outcome) -> Vec<Command> {
        self.phase = Phase::Finished;
        self.outcome = Some(outcome.clone());
        vec![Command::Finish(outcome)]
    }
}

/// What carrying out a session's commands means.
pub trait Effects {
    /// Make the change.
    ///
    /// # Errors
    ///
    /// Returns a message when the change could not be made.
    fn apply(&mut self) -> Result<(), String>;
    /// Undo the change.
    ///
    /// # Errors
    ///
    /// Returns a message when the change could not be undone.
    fn restore(&mut self) -> Result<(), String>;
    /// The monitor list.
    ///
    /// # Errors
    ///
    /// Returns a message when Hyprland cannot be asked.
    fn query(&mut self) -> Result<Vec<MonitorInfo>, String>;
}

/// Start the session (`input` is `None`) or feed it `input`, and carry out
/// the commands that follow until none are left. Returns the outcome once
/// the session is over.
pub fn step(
    session: &mut Session,
    input: Option<Input>,
    now: Duration,
    effects: &mut dyn Effects,
) -> Option<Outcome> {
    let mut commands: VecDeque<Command> = match input {
        Some(input) => session.handle(input, now),
        None => session.start(now),
    }
    .into();
    while let Some(command) = commands.pop_front() {
        let next = match command {
            Command::Apply => Input::Done(effects.apply()),
            Command::Restore => Input::Done(effects.restore()),
            Command::Query => Input::Observed(effects.query()),
            Command::Finish(outcome) => return Some(outcome),
        };
        commands.extend(session.handle(next, now));
    }
    None
}

/// A request that changes the running session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `reload`: back to what the files say.
    Reload,
    /// `eval` of Lua code (Lua configurations).
    Eval(String),
    /// `keyword monitor <value>` for each value (hyprlang configurations).
    Keywords(Vec<String>),
    /// Nothing.
    Nothing,
}

impl Action {
    /// The action that sets `rules` in the running session.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Unrepresentable`] for a rule that cannot be
    /// set live.
    pub fn set(backend: Backend, rules: &[MonitorRule]) -> Result<Action, ConfigError> {
        if rules.is_empty() {
            return Ok(Action::Nothing);
        }
        Ok(match backend {
            Backend::Lua => Action::Eval(crate::lua::eval_code(rules)?),
            Backend::Hyprlang => Action::Keywords(
                rules
                    .iter()
                    .map(crate::hyprlang::keyword_value)
                    .collect::<Result<_, _>>()?,
            ),
        })
    }

    /// The raw requests the action sends, for `--dry-run`.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        match self {
            Action::Reload => vec!["/reload".to_owned()],
            Action::Eval(code) => vec![format!("/eval {code}")],
            Action::Keywords(values) => values
                .iter()
                .map(|v| format!("/keyword monitor {v}"))
                .collect(),
            Action::Nothing => Vec::new(),
        }
    }

    /// Send the requests.
    ///
    /// # Errors
    ///
    /// Returns Hyprland's error message.
    pub fn run(&self, ipc: &dyn HyprlandIpc) -> Result<(), String> {
        match self {
            Action::Reload => ipc.reload(),
            Action::Eval(code) => ipc.eval(code),
            Action::Keywords(values) => values.iter().try_for_each(|v| ipc.keyword("monitor", v)),
            Action::Nothing => Ok(()),
        }
        .map_err(|e| e.to_string())
    }
}

/// A change made with requests only: a live preview, or a reload of a
/// file edited by hand.
pub struct IpcChange<'a> {
    /// The connection.
    pub ipc: &'a dyn HyprlandIpc,
    /// What makes the change.
    pub apply: Action,
    /// What undoes it.
    pub restore: Action,
}

impl Effects for IpcChange<'_> {
    fn apply(&mut self) -> Result<(), String> {
        self.apply.run(self.ipc)
    }

    fn restore(&mut self) -> Result<(), String> {
        self.restore.run(self.ipc)
    }

    fn query(&mut self) -> Result<Vec<MonitorInfo>, String> {
        self.ipc.monitors().map_err(|e| e.to_string())
    }
}

/// A persistent change: write the configuration file and reload; undone
/// by writing the previous content back and reloading.
pub struct FileChange<'a> {
    ipc: &'a dyn HyprlandIpc,
    before: Snapshot,
    content: String,
    backups: Option<BackupPolicy>,
    now: SystemTime,
    written: Option<Snapshot>,
    /// The backup made by the write, if any.
    pub backup: Option<std::path::PathBuf>,
}

impl<'a> FileChange<'a> {
    /// Write `content` over the file of `before` and reload.
    #[must_use]
    pub fn new(
        ipc: &'a dyn HyprlandIpc,
        before: Snapshot,
        content: String,
        backups: Option<BackupPolicy>,
        now: SystemTime,
    ) -> FileChange<'a> {
        FileChange {
            ipc,
            before,
            content,
            backups,
            now,
            written: None,
            backup: None,
        }
    }
}

impl Effects for FileChange<'_> {
    fn apply(&mut self) -> Result<(), String> {
        let report = fsio::replace(&self.before, &self.content, self.backups.as_ref(), self.now)
            .map_err(|e| e.to_string())?;
        self.backup = report.backup;
        self.written = Some(Snapshot {
            content: Some(self.content.clone()),
            ..self.before.clone()
        });
        self.ipc.reload().map_err(|e| e.to_string())
    }

    fn restore(&mut self) -> Result<(), String> {
        if let Some(written) = self.written.take() {
            fsio::replace(&written, self.before.text(), None, self.now)
                .map_err(|e| e.to_string())?;
        }
        self.ipc.reload().map_err(|e| e.to_string())
    }

    fn query(&mut self) -> Result<Vec<MonitorInfo>, String> {
        self.ipc.monitors().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
