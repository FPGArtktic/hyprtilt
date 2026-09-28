// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The subcommands. Each one is a thin sequence of core operations: read
//! the target, build the layout, change it, write and verify.

pub(crate) mod apply;
pub(crate) mod doctor;
pub(crate) mod edit;
pub(crate) mod file;
pub(crate) mod list;
pub(crate) mod profile;

use std::path::PathBuf;
use std::time::SystemTime;

use hyprtilt_core::apply::{Action, ApplyConfig, FileChange, IpcChange, Outcome, Session};
use hyprtilt_core::config::{Target, TargetReason};
use hyprtilt_core::document::{Backend, ConfigDocument};
use hyprtilt_core::fsio::{self, Snapshot};
use hyprtilt_core::ipc::MonitorInfo;
use hyprtilt_core::layout::{Expectation, Layout};
use hyprtilt_core::model::MonitorRule;
use serde::Serialize;

use crate::context::Context;
use crate::error::AppError;

/// The target file, read and parsed.
pub(crate) struct Workspace {
    /// The file and its language.
    pub(crate) target: Target,
    /// Its content when it was read.
    pub(crate) snapshot: Snapshot,
    /// Its block and the rules outside it.
    pub(crate) doc: ConfigDocument,
}

impl Workspace {
    /// Read and parse the target file of `ctx`.
    pub(crate) fn open(ctx: &Context) -> Result<Workspace, AppError> {
        Workspace::open_target(ctx.target())
    }

    /// Read and parse `target`.
    pub(crate) fn open_target(target: Target) -> Result<Workspace, AppError> {
        let snapshot = fsio::snapshot(&target.path)
            .map_err(|e| AppError::Config(format!("{}: {e}", target.path.display())))?;
        let doc = target
            .backend
            .parse(snapshot.text())
            .map_err(|e| AppError::Config(format!("{}: {e}", target.path.display())))?;
        Ok(Workspace {
            target,
            snapshot,
            doc,
        })
    }

    /// The rules of the managed block.
    pub(crate) fn block_rules(&self) -> &[MonitorRule] {
        self.doc.block_rules()
    }

    /// A short name of the file for messages.
    pub(crate) fn display(&self) -> String {
        self.target.path.display().to_string()
    }
}

/// How the target was chosen, in words.
pub(crate) fn reason(target: &Target) -> &'static str {
    match target.reason {
        TargetReason::Explicit => "--file",
        TargetReason::Settings => "config.toml",
        TargetReason::Caelestia => "Caelestia preset",
        TargetReason::MainConfig(_) => "Hyprland's main configuration",
    }
}

/// The live monitors and the layout built from them and the block. With
/// `--dry-run` and no Hyprland, the layout comes from the block alone.
pub(crate) fn layout(
    ctx: &Context,
    ws: &Workspace,
) -> Result<(Layout, Vec<MonitorInfo>), AppError> {
    let monitors = match ctx.ipc().and_then(|ipc| Ok(ipc.monitors()?)) {
        Ok(m) => m,
        Err(_) if ctx.global.dry_run => {
            eprintln!(
                "hyprtilt: Hyprland is not reachable; showing the change from the file alone"
            );
            return Ok((Layout::offline(ws.block_rules()), Vec::new()));
        }
        Err(e) => return Err(e),
    };
    let layout = Layout::new(&monitors, ws.block_rules(), ctx.settings.use_descriptions);
    Ok((layout, monitors))
}

/// Refuse a layout Hyprland would show broken.
pub(crate) fn check_layout(layout: &Layout) -> Result<(), AppError> {
    let blocking: Vec<String> = layout
        .problems()
        .iter()
        .filter(|p| p.is_blocking())
        .map(ToString::to_string)
        .collect();
    if blocking.is_empty() {
        Ok(())
    } else {
        Err(AppError::Config(format!(
            "refusing a layout with overlapping monitors: {}",
            blocking.join("; ")
        )))
    }
}

/// What a write did, for the report.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Written {
    /// The file.
    pub(crate) file: Option<PathBuf>,
    /// Its backup.
    pub(crate) backup: Option<PathBuf>,
    /// Whether the file changed.
    pub(crate) changed: bool,
    /// The requests sent instead, for live changes.
    pub(crate) requests: Vec<String>,
    /// Whether the change was verified.
    pub(crate) verified: bool,
}

fn outcome(result: Outcome) -> Result<(), AppError> {
    match result {
        Outcome::Kept => Ok(()),
        Outcome::RolledBack { reason } => Err(AppError::RolledBack(format!(
            "the change was rolled back: {reason}"
        ))),
        Outcome::RollbackFailed { reason, error } => Err(AppError::Other(format!(
            "the change was not kept ({reason}), and rolling it back failed: {error}"
        ))),
    }
}

/// Replace the file with `content`, reload, verify `expected`, and wait
/// for confirmation as `config` says; restore the file otherwise. With
/// `--dry-run`, print the diff instead.
pub(crate) fn write_and_verify(
    ctx: &Context,
    ws: &Workspace,
    content: &str,
    expected: Vec<Expectation>,
    config: ApplyConfig,
) -> Result<Written, AppError> {
    let old = ws.snapshot.text();
    if old == content {
        return Ok(Written {
            file: Some(ws.target.path.clone()),
            ..Written::default()
        });
    }
    if ctx.global.dry_run {
        if !ctx.global.json {
            print!("{}", fsio::diff(old, content, &ws.display()));
            println!("then: /reload");
        }
        return Ok(Written {
            file: Some(ws.target.path.clone()),
            changed: true,
            requests: vec!["/reload".to_owned()],
            ..Written::default()
        });
    }
    ctx.check_backend(&ws.target)?;
    if ws.target.backend == Backend::Lua
        && let Some(luac) = Context::luac()
    {
        luac.check_regression(old, content).map_err(|e| {
            AppError::Config(format!(
                "the new content of {} does not compile: {e}",
                ws.display()
            ))
        })?;
    }
    let ipc = ctx.ipc()?;
    let mut change = FileChange::new(
        ipc,
        ws.snapshot.clone(),
        content.to_owned(),
        Some(ctx.settings.backup_policy()),
        SystemTime::now(),
    );
    let mut session = Session::new(expected, config);
    let result = crate::run::run(&mut session, &mut change);
    let backup = change.backup.clone();
    outcome(result)?;
    Ok(Written {
        file: Some(ws.target.path.clone()),
        backup,
        changed: true,
        requests: vec!["/reload".to_owned()],
        verified: true,
    })
}

/// Send `apply`, verify `expected`, wait for confirmation as `config`
/// says, and send `restore` otherwise. With `--dry-run`, print the
/// requests instead.
pub(crate) fn live_change(
    ctx: &Context,
    apply: Action,
    restore: Action,
    expected: Vec<Expectation>,
    config: ApplyConfig,
) -> Result<Written, AppError> {
    let requests = apply.requests();
    if ctx.global.dry_run {
        if !ctx.global.json {
            for r in &requests {
                println!("{r}");
            }
        }
        return Ok(Written {
            requests,
            ..Written::default()
        });
    }
    let ipc = ctx.ipc()?;
    let mut change = IpcChange {
        ipc,
        apply,
        restore,
    };
    let mut session = Session::new(expected, config);
    outcome(crate::run::run(&mut session, &mut change))?;
    Ok(Written {
        requests,
        verified: true,
        ..Written::default()
    })
}

/// The backend Hyprland runs, for live changes: its provider, else the
/// target's language.
pub(crate) fn running_backend(ctx: &Context, fallback: Backend) -> Backend {
    match ctx.status().map(|s| s.config_provider) {
        Some(p) if p == "lua" => Backend::Lua,
        Some(p) if p == "hyprlang" => Backend::Hyprlang,
        _ => fallback,
    }
}

/// Print `value` as pretty JSON.
pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<(), AppError> {
    let text = serde_json::to_string_pretty(value).map_err(|e| AppError::Other(e.to_string()))?;
    println!("{text}");
    Ok(())
}

/// The differences between two rules in the fields a user sees, as
/// `field old -> new`.
pub(crate) fn describe_change(before: &MonitorRule, after: &MonitorRule) -> Vec<String> {
    let show = |v: Option<String>| v.unwrap_or_else(|| "unset".to_owned());
    let fields: [(&str, Option<String>, Option<String>); 6] = [
        (
            "mode",
            before.mode.as_ref().map(ToString::to_string),
            after.mode.as_ref().map(ToString::to_string),
        ),
        (
            "position",
            before.position.map(|p| p.to_string()),
            after.position.map(|p| p.to_string()),
        ),
        (
            "scale",
            before.scale.map(|s| s.to_string()),
            after.scale.map(|s| s.to_string()),
        ),
        (
            "transform",
            before.transform.map(|t| t.value().to_string()),
            after.transform.map(|t| t.value().to_string()),
        ),
        (
            "disabled",
            before.disabled.map(|d| d.to_string()),
            after.disabled.map(|d| d.to_string()),
        ),
        (
            "vrr",
            before.vrr.map(|v| v.to_string()),
            after.vrr.map(|v| v.to_string()),
        ),
    ];
    fields
        .into_iter()
        .filter(|(_, a, b)| a != b)
        .map(|(name, a, b)| format!("{name} {} -> {}", show(a), show(b)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprtilt_core::model::{Position, Transform};

    #[test]
    fn changes_are_described_field_by_field() {
        let mut a = MonitorRule::new("DP-1");
        a.mode = Some("2560x1440@179.95".parse().unwrap());
        a.position = Some(Position::At { x: 0, y: 0 });
        let mut b = a.clone();
        b.mode = Some("2560x1440@144".parse().unwrap());
        b.transform = Some(Transform::new(1).unwrap());
        assert_eq!(
            describe_change(&a, &b),
            [
                "mode 2560x1440@179.95 -> 2560x1440@144",
                "transform unset -> 1"
            ]
        );
        assert!(describe_change(&a, &a).is_empty());
    }

    #[test]
    fn outcomes_become_exit_codes() {
        use hyprtilt_core::apply::Reason;
        assert!(outcome(Outcome::Kept).is_ok());
        let e = outcome(Outcome::RolledBack {
            reason: Reason::NotConfirmed,
        })
        .unwrap_err();
        assert_eq!(e.exit(), crate::exit::Exit::RolledBack);
        let e = outcome(Outcome::RollbackFailed {
            reason: Reason::Signal,
            error: "gone".into(),
        })
        .unwrap_err();
        assert_eq!(e.exit(), crate::exit::Exit::Error);
    }
}
