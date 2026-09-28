// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `hyprtilt doctor`: checks of the setup, as data.
//!
//! The binary gathers [`Facts`] (IPC replies, files, `/proc`), and
//! [`diagnose`] turns them into [`Check`]s without touching the system, so
//! every conclusion is testable. Whether Hyprland really loads the target
//! file is judged from three independent signals
//! (`docs/hyprland-lua-api.md`, section 10): the inotify watches of the
//! compositor, the configuration errors, and a comparison of the live
//! state with the managed block.

use std::path::Path;

use serde::Serialize;

use crate::config::{MainConfig, MainConfigSource, Target, TargetReason};
use crate::document::{Backend, ConfigDocument};
use crate::ipc::MonitorInfo;
use crate::layout::{Layout, Origin, Problem, compare};
use crate::lua::syntax::Luac;
use crate::model::SelectorKind;
use crate::proc::ProcessInfo;
use crate::version::Version;

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Fine.
    Ok,
    /// For information.
    Info,
    /// Probably a problem.
    Warn,
    /// A problem.
    Fail,
}

impl Status {
    /// Whether the check counts as a problem (for the exit code).
    #[must_use]
    pub fn is_problem(self) -> bool {
        self >= Status::Warn
    }
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable identifier, for scripts.
    pub id: &'static str,
    /// The outcome.
    pub status: Status,
    /// One line.
    pub title: String,
    /// More detail or what to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What the running compositor reported.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    /// The version string.
    pub version: String,
    /// `lua` or `hyprlang`; `None` before 0.55.
    pub provider: Option<String>,
    /// The monitors.
    pub monitors: Vec<MonitorInfo>,
    /// The configuration errors.
    pub config_errors: Vec<String>,
}

/// Everything the checks look at.
#[derive(Debug, Clone)]
pub struct Facts {
    /// The compositor, or why it could not be reached.
    pub hyprland: Result<Live, String>,
    /// Hyprland's main configuration.
    pub main_config: MainConfig,
    /// The file hyprtilt edits.
    pub target: Target,
    /// Its content (`None` when it does not exist), or a read error.
    pub target_content: Result<Option<String>, String>,
    /// Its inode number.
    pub target_inode: Option<u64>,
    /// Why it cannot be written, if it cannot.
    pub read_only: Option<String>,
    /// The compositor's process.
    pub process: Option<ProcessInfo>,
    /// The Lua compiler used for syntax checks.
    pub luac: Option<Luac>,
}

fn check(
    id: &'static str,
    status: Status,
    title: impl Into<String>,
    detail: Option<String>,
) -> Check {
    Check {
        id,
        status,
        title: title.into(),
        detail,
    }
}

/// Run every check.
#[must_use]
pub fn diagnose(facts: &Facts) -> Vec<Check> {
    let mut out = Vec::new();
    let live = facts.hyprland.as_ref().ok();
    hyprland(facts, &mut out);
    if let Some(live) = live {
        backend(facts, live, &mut out);
    }
    out.push(main_config(&facts.main_config));
    target(facts, &mut out);
    let doc = document(facts, &mut out);
    if let Some(reason) = &facts.read_only {
        out.push(check(
            "writable",
            Status::Fail,
            format!("{} cannot be written", facts.target.path.display()),
            Some(reason.clone()),
        ));
    }
    if let Some(doc) = &doc {
        outside(facts.target.backend, doc, &mut out);
    }
    loaded(facts, &mut out);
    if let Some(live) = live {
        errors(facts, live, &mut out);
        if let Some(doc) = &doc {
            live_state(doc, live, &mut out);
        }
    }
    if facts.target.backend == Backend::Lua {
        out.push(match &facts.luac {
            Some(l) => check(
                "syntax-check",
                Status::Ok,
                format!(
                    "Lua {} checks new content before it is written ({})",
                    l.version,
                    l.path.display()
                ),
                None,
            ),
            None => check(
                "syntax-check",
                Status::Info,
                "no Lua compiler found for an extra syntax check before writing",
                Some("install Lua 5.4 or 5.5 (luac) to enable it".to_owned()),
            ),
        });
    }
    out
}

fn hyprland(facts: &Facts, out: &mut Vec<Check>) {
    out.push(match &facts.hyprland {
        Ok(live) => check(
            "hyprland",
            Status::Ok,
            format!(
                "Hyprland {} is running with a {} configuration and {} monitor(s)",
                live.version,
                live.provider.as_deref().unwrap_or("hyprlang"),
                live.monitors.len()
            ),
            None,
        ),
        Err(e) => check(
            "hyprland",
            Status::Fail,
            "Hyprland cannot be reached",
            Some(format!("{e}; the file checks below still run")),
        ),
    });
}

fn backend(facts: &Facts, live: &Live, out: &mut Vec<Check>) {
    let version: Option<Version> = live.version.parse().ok();
    let provider = live.provider.as_deref().unwrap_or("hyprlang");
    let target = facts.target.backend;
    let status = match (provider, target) {
        ("lua", Backend::Hyprlang) => {
            Some("Hyprland runs a Lua configuration and never reads a hyprlang file")
        }
        ("hyprlang", Backend::Lua) => {
            Some("Hyprland runs a hyprlang configuration and never reads a Lua file")
        }
        _ => None,
    };
    if let Some(problem) = status {
        out.push(check(
            "backend",
            Status::Fail,
            problem,
            Some(format!("the target is {}", facts.target.path.display())),
        ));
    } else if target == Backend::Lua && version.is_some_and(|v| !v.at_least(Version::LUA)) {
        out.push(check(
            "backend",
            Status::Fail,
            "the Lua configuration needs Hyprland 0.55 or newer",
            None,
        ));
    } else {
        out.push(check(
            "backend",
            Status::Ok,
            format!("the target's language ({target}) is the one Hyprland runs"),
            None,
        ));
    }
    if provider == "hyprlang" && version.is_some_and(|v| v.at_least(Version::HYPRLANG_DEPRECATED)) {
        out.push(check(
            "hyprlang-deprecated",
            Status::Info,
            "Hyprland announces that the .conf format will be removed in 0.57",
            Some("hyprtilt edits Lua configurations the same way".to_owned()),
        ));
    }
}

fn main_config(main: &MainConfig) -> Check {
    let how = match main.source {
        MainConfigSource::CommandLine => "given with --config",
        MainConfigSource::Environment => "from $HYPRLAND_CONFIG",
        MainConfigSource::Xdg => "found in the configuration directories",
        MainConfigSource::Default => "does not exist yet",
    };
    check(
        "main-config",
        Status::Info,
        format!(
            "Hyprland's main configuration: {} ({how})",
            main.path.display()
        ),
        None,
    )
}

fn target(facts: &Facts, out: &mut Vec<Check>) {
    let t = &facts.target;
    let why = match &t.reason {
        TargetReason::Explicit => "given with --file".to_owned(),
        TargetReason::Settings => "from config.toml".to_owned(),
        TargetReason::Caelestia => {
            "the Caelestia preset: the file require(\"hypr-user\") loads".to_owned()
        }
        TargetReason::MainConfig(_) => "Hyprland's main configuration".to_owned(),
    };
    out.push(check(
        "target",
        Status::Ok,
        format!("hyprtilt edits {} ({}; {why})", t.path.display(), t.backend),
        None,
    ));
    for w in &t.warnings {
        out.push(check("target", Status::Warn, w.clone(), None));
    }
}

fn document(facts: &Facts, out: &mut Vec<Check>) -> Option<ConfigDocument> {
    let path = facts.target.path.display();
    let content = match &facts.target_content {
        Ok(Some(text)) => text,
        Ok(None) => {
            out.push(check(
                "target-file",
                Status::Info,
                format!("{path} does not exist yet"),
                Some("hyprtilt creates it on the first save".to_owned()),
            ));
            return None;
        }
        Err(e) => {
            out.push(check(
                "target-file",
                Status::Fail,
                format!("{path} cannot be read"),
                Some(e.clone()),
            ));
            return None;
        }
    };
    match facts.target.backend.parse(content) {
        Ok(doc) => {
            out.push(match &doc.block {
                Some(b) => check(
                    "target-file",
                    Status::Ok,
                    format!(
                        "managed block on lines {}-{} with {} rule(s)",
                        b.begin_line,
                        b.end_line,
                        b.rules.len()
                    ),
                    None,
                ),
                None => check(
                    "target-file",
                    Status::Info,
                    "no managed block yet",
                    Some("`hyprtilt save` or `hyprtilt adopt` creates it".to_owned()),
                ),
            });
            Some(doc)
        }
        Err(e) => {
            out.push(check(
                "target-file",
                Status::Fail,
                format!("{path} cannot be edited"),
                Some(e.to_string()),
            ));
            None
        }
    }
}

fn outside(backend: Backend, doc: &ConfigDocument, out: &mut Vec<Check>) {
    let lines = |rules: &[&crate::document::FoundRule]| -> String {
        rules
            .iter()
            .map(|f| f.line.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let adoptable: Vec<_> = doc.outside.iter().filter(|f| f.is_adoptable()).collect();
    if !adoptable.is_empty() {
        out.push(check(
            "outside-rules",
            Status::Info,
            format!(
                "{} monitor rule(s) outside the block (line {})",
                adoptable.len(),
                lines(&adoptable)
            ),
            Some("`hyprtilt adopt` moves them into the block".to_owned()),
        ));
    }
    for f in doc.outside.iter().filter(|f| !f.is_adoptable()) {
        out.push(check(
            "outside-rules",
            Status::Info,
            format!("line {}: a monitor rule hyprtilt leaves alone", f.line),
            f.not_adoptable.clone(),
        ));
    }
    let effect = match backend {
        Backend::Lua => "merges with",
        Backend::Hyprlang => "competes with",
    };
    for f in doc.conflicting_outside() {
        let selector = f
            .rule
            .as_ref()
            .map(|r| r.output.to_string())
            .unwrap_or_default();
        out.push(check(
            "conflicting-rule",
            Status::Warn,
            format!(
                "line {}: a rule for {selector:?} outside the block {effect} the block's rule",
                f.line
            ),
            Some("adopt it, or remove it".to_owned()),
        ));
    }
}

fn loaded(facts: &Facts, out: &mut Vec<Check>) {
    let path = facts.target.path.display();
    let watched = facts
        .process
        .as_ref()
        .and_then(|p| p.watched_inodes.as_ref());
    out.push(match (watched, facts.target_inode) {
        (Some(list), Some(inode)) if list.contains(&inode) => {
            check("loaded", Status::Ok, format!("Hyprland watches {path}, so it loads it"), None)
        }
        (Some(list), Some(_)) if list.is_empty() => check(
            "loaded",
            Status::Info,
            "Hyprland watches no files, so whether it loads the target cannot be told",
            Some("misc:disable_autoreload is probably set".to_owned()),
        ),
        (Some(_), Some(_)) => check(
            "loaded",
            Status::Warn,
            format!("Hyprland does not watch {path}: it probably does not load it"),
            Some("check that your configuration loads this file (require, source, dofile)".to_owned()),
        ),
        (_, None) => check("loaded", Status::Info, format!("{path} does not exist, so Hyprland cannot load it yet"), None),
        (None, _) => check(
            "loaded",
            Status::Info,
            "the Hyprland process cannot be inspected, so whether it loads the target cannot be told",
            None,
        ),
    });
}

fn errors(facts: &Facts, live: &Live, out: &mut Vec<Check>) {
    if live.config_errors.is_empty() {
        out.push(check(
            "config-errors",
            Status::Ok,
            "Hyprland reports no configuration errors",
            None,
        ));
        return;
    }
    let target = facts.target.path.to_string_lossy();
    let name = Path::new(target.as_ref())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let about_target = live
        .config_errors
        .iter()
        .any(|e| e.contains(target.as_ref()) || (!name.is_empty() && e.contains(&name)));
    out.push(check(
        "config-errors",
        if about_target {
            Status::Fail
        } else {
            Status::Warn
        },
        format!(
            "Hyprland reports {} configuration error line(s)",
            live.config_errors.len()
        ),
        Some(live.config_errors.join("\n")),
    ));
}

fn live_state(doc: &ConfigDocument, live: &Live, out: &mut Vec<Check>) {
    let Some(block) = &doc.block else {
        return;
    };
    let layout = Layout::new(&live.monitors, &block.rules, false);
    let from_block: Vec<_> = layout
        .expectations()
        .into_iter()
        .zip(&layout.outputs)
        .filter(|(_, o)| matches!(o.origin, Origin::Block(_)))
        .map(|(e, _)| e)
        .collect();
    let mismatches = compare(&from_block, &live.monitors);
    out.push(if mismatches.is_empty() {
        check(
            "live-state",
            Status::Ok,
            format!("the live state matches the block for {} output(s)", from_block.len()),
            None,
        )
    } else {
        check(
            "live-state",
            Status::Warn,
            "the live state differs from the block",
            Some(format!(
                "{}\nThe file may not be loaded or reloaded yet, a rule elsewhere may override it, or an output-management tool (kanshi, wlr-randr, nwg-displays) may be running.",
                mismatches.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
            )),
        )
    });
    for problem in layout.problems() {
        let status = match problem {
            Problem::Overlap { .. } => Status::Fail,
            Problem::CustomMode { .. } => Status::Info,
            _ => Status::Warn,
        };
        out.push(check("layout", status, problem.to_string(), None));
    }
    for rule in &block.rules {
        if let SelectorKind::Description(_) = rule.output.kind() {
            let matching: Vec<&str> = live
                .monitors
                .iter()
                .filter(|m| rule.output.matches(&m.name, &m.description))
                .map(|m| m.name.as_str())
                .collect();
            if matching.len() > 1 {
                out.push(check(
                    "selectors",
                    Status::Warn,
                    format!(
                        "{:?} matches {}",
                        rule.output.as_str(),
                        matching.join(" and ")
                    ),
                    Some("write the full description, serial number included".to_owned()),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
