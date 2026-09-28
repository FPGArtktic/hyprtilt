// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! An in-memory Hyprland for tests and demonstrations.
//!
//! [`FakeHyprland`] answers the requests hyprtilt sends the way Hyprland
//! 0.56 does: `reload` reads the configuration file with hyprtilt's own
//! parsers, `eval` and `keyword` add rules to the running session, and the
//! monitor list is computed from the rules with Hyprland's precedence (the
//! last matching rule wins, `""` is the fallback), scale snapping and
//! logical sizes. Failures and delayed application can be scripted, so the
//! apply and rollback logic can be tested without a compositor.
//!
//! The simulation is deliberately simple where hyprtilt does not depend on
//! the details: automatic positions are placed to the right of the others,
//! and a mode is taken from the available list by closeness.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{HyprlandIpc, IpcError, MonitorInfo, StatusInfo, UNKNOWN_REQUEST, VersionInfo};
use crate::document::Backend;
use crate::geometry::{self, Rect};
use crate::model::{Mode, MonitorRule, Position, Scale};
use crate::version::Version;

/// What the fake compositor looks like; also the format of the JSON file
/// behind `hyprtilt --fake-hyprland`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FakeSetup {
    /// Reported version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Configuration provider.
    #[serde(default = "default_provider")]
    pub provider: Backend,
    /// The file a reload reads, relative to the setup file.
    #[serde(default)]
    pub config: Option<PathBuf>,
    /// Rules evaluated before the configuration file, such as the fallback
    /// rule of Caelestia's main configuration.
    #[serde(default)]
    pub base_rules: Vec<MonitorRule>,
    /// The connected monitors with their hardware and initial state.
    pub monitors: Vec<MonitorInfo>,
}

fn default_version() -> String {
    "0.56.2".to_owned()
}

fn default_provider() -> Backend {
    Backend::Lua
}

struct State {
    setup: FakeSetup,
    monitors: Vec<MonitorInfo>,
    /// The state `monitors` reports while a change is still pending.
    shown: Option<Vec<MonitorInfo>>,
    lag: usize,
    pending: usize,
    rules: Vec<MonitorRule>,
    errors: Vec<String>,
    requests: Vec<String>,
    scripted: VecDeque<(String, Result<String, String>)>,
}

/// An in-memory Hyprland (see the module documentation).
pub struct FakeHyprland {
    state: Mutex<State>,
}

impl FakeHyprland {
    /// A compositor with the given setup. The configuration file, if any,
    /// is not read until the first `reload`.
    #[must_use]
    pub fn new(setup: FakeSetup) -> FakeHyprland {
        let monitors = setup.monitors.clone();
        FakeHyprland {
            state: Mutex::new(State {
                setup,
                monitors,
                shown: None,
                lag: 0,
                pending: 0,
                rules: Vec::new(),
                errors: Vec::new(),
                requests: Vec::new(),
                scripted: VecDeque::new(),
            }),
        }
    }

    /// Load a setup from a JSON file; a relative `config` is resolved
    /// against the file's directory.
    ///
    /// # Errors
    ///
    /// Returns a message when the file cannot be read or parsed.
    pub fn from_file(path: &Path) -> Result<FakeHyprland, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut setup: FakeSetup =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(config) = &setup.config
            && config.is_relative()
        {
            let base = path.parent().unwrap_or_else(|| Path::new("."));
            setup.config = Some(base.join(config));
        }
        Ok(FakeHyprland::new(setup))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The configuration file a reload reads, if any.
    #[must_use]
    pub fn config_path(&self) -> Option<PathBuf> {
        self.lock().setup.config.clone()
    }

    /// The configuration provider.
    #[must_use]
    pub fn provider(&self) -> Backend {
        self.lock().setup.provider
    }

    /// Every request received so far.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.lock().requests.clone()
    }

    /// Answer the next request that starts with `prefix` with `reply`
    /// (`Err` becomes an I/O error), without acting on it.
    pub fn script(&self, prefix: &str, reply: Result<String, String>) {
        self.lock().scripted.push_back((prefix.to_owned(), reply));
    }

    /// Keep reporting the previous monitor state for `polls` more
    /// `monitors` requests after each change, as Hyprland does until the
    /// next frame.
    pub fn set_lag(&self, polls: usize) {
        self.lock().lag = polls;
    }

    /// The monitor state right now, ignoring any lag.
    #[must_use]
    pub fn current(&self) -> Vec<MonitorInfo> {
        self.lock().monitors.clone()
    }

    /// Replace the connected monitors (a hotplug).
    pub fn set_monitors(&self, monitors: Vec<MonitorInfo>) {
        let mut s = self.lock();
        s.setup.monitors.clone_from(&monitors);
        s.monitors = monitors;
        s.recompute();
    }
}

impl State {
    fn version(&self) -> Option<Version> {
        self.setup.version.parse().ok()
    }

    fn change(&mut self, rules: Vec<MonitorRule>) {
        self.rules = rules;
        let before = self.monitors.clone();
        self.recompute();
        if self.lag > 0 && self.monitors != before {
            self.shown = Some(before);
            self.pending = self.lag;
        }
    }

    fn recompute(&mut self) {
        self.monitors = apply(&self.setup.monitors, &self.rules);
    }

    fn reload(&mut self) {
        self.errors.clear();
        let mut rules = self.setup.base_rules.clone();
        if let Some(path) = self.setup.config.clone() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match file_rules(self.setup.provider, &text) {
                    Ok(found) => rules.extend(found),
                    Err(e) => self.errors.push(format!("{}: {e}", path.display())),
                },
                Err(e) => self.errors.push(format!("{}: {e}", path.display())),
            }
        }
        let lua = self.setup.provider == Backend::Lua;
        self.change(merge(Vec::new(), rules, lua));
    }

    fn handle(&mut self, request: &str) -> String {
        let Some((flags, command)) = request.split_once('/') else {
            return UNKNOWN_REQUEST.to_owned();
        };
        let json = flags.contains('j');
        let (name, args) = command.split_once(' ').unwrap_or((command, ""));
        match name {
            "monitors" if json => {
                let shown = if self.pending > 0 {
                    self.pending -= 1;
                    self.shown.clone().unwrap_or_else(|| self.monitors.clone())
                } else {
                    self.monitors.clone()
                };
                to_json(&shown)
            }
            "version" if json => to_json(&VersionInfo {
                version: self.setup.version.clone(),
                tag: format!("v{}", self.setup.version),
                commit: "fake".to_owned(),
                branch: format!("v{}", self.setup.version),
            }),
            "status" if json && self.version().is_some_and(|v| v.at_least(Version::LUA)) => {
                to_json(&StatusInfo {
                    config_provider: self.setup.provider.to_string(),
                    backend: "drm".to_owned(),
                })
            }
            "configerrors" if json => {
                if self.errors.is_empty() {
                    to_json(&[""])
                } else {
                    to_json(&self.errors)
                }
            }
            "reload" => {
                self.reload();
                "ok".to_owned()
            }
            "eval" if self.setup.provider != Backend::Lua => {
                "eval is only supported with the lua config manager".to_owned()
            }
            "eval" => {
                self.errors.clear();
                match crate::lua::parse(args) {
                    Ok(doc) => {
                        let rules = doc.outside.into_iter().filter_map(|f| f.rule).collect();
                        let merged = merge(self.rules.clone(), rules, true);
                        self.change(merged);
                        "ok".to_owned()
                    }
                    Err(e) => format!("error: {e}"),
                }
            }
            "keyword" if self.setup.provider == Backend::Lua => {
                "keyword can't work with non-legacy parsers. Use eval.".to_owned()
            }
            "keyword" => {
                let Some(("monitor", value)) = args.split_once(' ') else {
                    return "ok".to_owned();
                };
                match crate::hyprlang::parse(&format!("monitor = {value}\n")) {
                    Ok(doc) => {
                        let rules = doc.outside.into_iter().filter_map(|f| f.rule).collect();
                        let merged = merge(self.rules.clone(), rules, false);
                        self.change(merged);
                        "ok".to_owned()
                    }
                    Err(e) => format!("error: {e}"),
                }
            }
            _ => UNKNOWN_REQUEST.to_owned(),
        }
    }
}

impl HyprlandIpc for FakeHyprland {
    fn request(&self, request: &str) -> Result<String, IpcError> {
        let mut s = self.lock();
        s.requests.push(request.to_owned());
        let scripted = s
            .scripted
            .iter()
            .position(|(p, _)| request.starts_with(p.as_str()))
            .and_then(|i| s.scripted.remove(i));
        if let Some((_, reply)) = scripted {
            return reply.map_err(|e| IpcError::Io(std::io::Error::other(e)));
        }
        Ok(s.handle(request))
    }
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The literal rules of a configuration file in evaluation order.
fn file_rules(backend: Backend, text: &str) -> Result<Vec<MonitorRule>, String> {
    let doc = backend.parse(text).map_err(|e| e.to_string())?;
    let mut ordered: Vec<(usize, MonitorRule)> = doc
        .outside
        .into_iter()
        .filter_map(|f| f.rule.map(|r| (f.line, r)))
        .collect();
    if let Some(block) = doc.block {
        ordered.extend(block.rule_lines.into_iter().zip(block.rules));
    }
    ordered.sort_by_key(|(line, _)| *line);
    Ok(ordered.into_iter().map(|(_, r)| r).collect())
}

/// Add `new` to the rule list as Hyprland's rule manager does: a rule
/// with an existing selector replaces it (merging fields in Lua) and moves
/// to the end.
fn merge(mut list: Vec<MonitorRule>, new: Vec<MonitorRule>, lua: bool) -> Vec<MonitorRule> {
    for rule in new {
        let merged = match list.iter().position(|r| r.output == rule.output) {
            Some(i) => {
                let mut old = list.remove(i);
                if lua {
                    old.overlay(&rule);
                    old
                } else {
                    rule
                }
            }
            None => rule,
        };
        list.push(merged);
    }
    list
}

/// The rule Hyprland uses for a monitor: the last one that matches, else
/// the first fallback rule, else a default.
#[must_use]
pub fn rule_for<'r>(
    rules: &'r [MonitorRule],
    name: &str,
    description: &str,
) -> Option<&'r MonitorRule> {
    rules
        .iter()
        .rev()
        .find(|r| r.output.matches(name, description))
        .or_else(|| rules.iter().find(|r| r.output.as_str().is_empty()))
}

/// The monitor state that `rules` produce on `hardware`.
fn apply(hardware: &[MonitorInfo], rules: &[MonitorRule]) -> Vec<MonitorInfo> {
    let mut out: Vec<MonitorInfo> = Vec::with_capacity(hardware.len());
    let mut auto: Vec<usize> = Vec::new();
    for hw in hardware {
        let default_rule = MonitorRule::new("");
        let rule = rule_for(rules, &hw.name, &hw.description).unwrap_or(&default_rule);
        let mut m = hw.clone();
        m.disabled = rule.is_disabled();
        let (w, h, hz) = pick_mode(hw, rule.mode.as_ref());
        m.width = w;
        m.height = h;
        m.refresh_rate = hz;
        let physical = (hw.physical_width, hw.physical_height);
        let fallback = geometry::auto_scale((w, h), physical);
        m.scale = match rule.scale {
            Some(Scale::Factor(f)) => {
                f64::from(geometry::effective_scale((w, h), f as f32, false, fallback).scale)
            }
            _ => f64::from(geometry::effective_scale((w, h), fallback, true, fallback).scale),
        };
        m.transform = rule.transform_or_default().value();
        m.vrr = rule.vrr == Some(1);
        match rule.position {
            Some(Position::At { x, y }) => {
                m.x = x;
                m.y = y;
            }
            _ => auto.push(out.len()),
        }
        out.push(m);
    }
    for i in auto {
        let placed: Vec<Rect> = out
            .iter()
            .enumerate()
            .filter(|(j, m)| *j != i && !m.disabled)
            .map(|(_, m)| logical_rect(m))
            .collect();
        let right = geometry::bounds(&placed).map_or(0, Rect::right);
        out[i].x = right.max(0);
        out[i].y = 0;
    }
    out
}

fn logical_rect(m: &MonitorInfo) -> Rect {
    let t = crate::model::Transform::new(m.transform).unwrap_or_default();
    let (w, h) = geometry::logical_size(m.pixels(), t, m.scale as f32);
    Rect::new(m.x, m.y, w, h)
}

/// The mode a rule selects from a monitor's available modes.
fn pick_mode(hw: &MonitorInfo, mode: Option<&Mode>) -> (u32, u32, f64) {
    let modes: Vec<(u32, u32, f64)> = hw
        .modes()
        .into_iter()
        .filter_map(|m| {
            m.resolution()
                .map(|(w, h)| (w, h, m.refresh().unwrap_or(60.0)))
        })
        .collect();
    let first = modes
        .first()
        .copied()
        .unwrap_or((hw.width, hw.height, hw.refresh_rate));
    match mode {
        Some(Mode::Resolution {
            width,
            height,
            refresh,
        }) => {
            let hz = refresh.unwrap_or(60.0);
            modes
                .iter()
                .copied()
                .filter(|&(w, h, _)| w == *width && h == *height)
                .min_by(|a, b| (a.2 - hz).abs().total_cmp(&(b.2 - hz).abs()))
                .unwrap_or((*width, *height, hz))
        }
        Some(Mode::HighRefreshRate) => modes
            .iter()
            .copied()
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap_or(first),
        Some(Mode::HighResolution | Mode::MaxWidth) => modes
            .iter()
            .copied()
            .max_by_key(|&(w, h, _)| (u64::from(w) * u64::from(h), w))
            .unwrap_or(first),
        Some(Mode::Modeline(_)) => (hw.width, hw.height, hw.refresh_rate),
        Some(Mode::Preferred) | None => first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITORS: &str = include_str!("../../tests/fixtures/ipc/monitors-all-0.56.2.json");
    const HYPR_USER: &str = include_str!("../../tests/fixtures/lua/caelestia-hypr-user.lua");

    fn setup(config: Option<PathBuf>) -> FakeSetup {
        FakeSetup {
            version: "0.56.2".to_owned(),
            provider: Backend::Lua,
            config,
            base_rules: Vec::new(),
            monitors: serde_json::from_str(MONITORS).unwrap(),
        }
    }

    #[test]
    fn reload_reproduces_the_live_state_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hypr-user.lua");
        std::fs::write(&file, HYPR_USER).unwrap();
        let live: Vec<MonitorInfo> = serde_json::from_str(MONITORS).unwrap();
        let mut scrambled = setup(Some(file));
        for m in &mut scrambled.monitors {
            m.x = 0;
            m.y = 0;
            m.transform = 0;
        }
        let fake = FakeHyprland::new(scrambled);
        fake.reload().unwrap();
        let now = fake.monitors().unwrap();
        for (a, b) in now.iter().zip(&live) {
            assert_eq!(
                (a.name.as_str(), a.x, a.y, a.transform),
                (b.name.as_str(), b.x, b.y, b.transform)
            );
            assert!((a.refresh_rate - b.refresh_rate).abs() < 0.01, "{}", a.name);
        }
        assert!(fake.config_errors().unwrap().is_empty());
        assert_eq!(
            fake.requests(),
            ["/reload", "j/monitors all", "j/configerrors"]
        );
    }

    #[test]
    fn eval_merges_and_reload_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hypr-user.lua");
        std::fs::write(&file, HYPR_USER).unwrap();
        let fake = FakeHyprland::new(setup(Some(file)));
        fake.reload().unwrap();
        fake.eval(r#"hl.monitor({ output = "DP-1", position = "4000x0" })"#)
            .unwrap();
        let dp = fake
            .current()
            .into_iter()
            .find(|m| m.name == "DP-1")
            .unwrap();
        // Merged: the mode and refresh rate of the file's rule are kept.
        assert_eq!((dp.x, dp.y), (4000, 0));
        assert!((dp.refresh_rate - 179.952).abs() < 0.01);
        fake.reload().unwrap();
        let dp = fake
            .current()
            .into_iter()
            .find(|m| m.name == "DP-1")
            .unwrap();
        assert_eq!((dp.x, dp.y), (3360, 975));
        assert!(fake.keyword("monitor", "DP-1,preferred,auto,1").is_err());
    }

    #[test]
    fn hyprlang_keyword_and_version_gates() {
        let mut s = setup(None);
        s.provider = Backend::Hyprlang;
        s.version = "0.53.3".to_owned();
        let fake = FakeHyprland::new(s);
        assert_eq!(fake.status().unwrap(), None);
        assert!(fake.eval("x").is_err());
        fake.keyword("monitor", "DP-1,1920x1080@60,100x100,2")
            .unwrap();
        let dp = fake
            .current()
            .into_iter()
            .find(|m| m.name == "DP-1")
            .unwrap();
        assert_eq!((dp.width, dp.x, dp.y), (1920, 100, 100));
        assert!((dp.scale - 2.0).abs() < f64::EPSILON);
        assert_eq!(fake.version().unwrap().version, "0.53.3");
    }

    #[test]
    fn lag_and_scripted_failures() {
        let fake = FakeHyprland::new(setup(None));
        fake.set_lag(2);
        fake.eval(
            r#"hl.monitor({ output = "eDP-1", position = "0x5000", mode = "1920x1080@144" })"#,
        )
        .unwrap();
        let edp = |ms: Vec<MonitorInfo>| ms.into_iter().find(|m| m.name == "eDP-1").unwrap().y;
        assert_eq!(edp(fake.monitors().unwrap()), 1335);
        assert_eq!(edp(fake.monitors().unwrap()), 1335);
        assert_eq!(edp(fake.monitors().unwrap()), 5000);
        fake.script("/reload", Err("broken pipe".to_owned()));
        assert!(matches!(fake.reload(), Err(IpcError::Io(_))));
        fake.script("/eval", Ok("error: nope".to_owned()));
        assert!(matches!(fake.eval("x"), Err(IpcError::Refused { .. })));
        assert_eq!(fake.request("nonsense").unwrap(), UNKNOWN_REQUEST);
        assert_eq!(fake.request("/unknown").unwrap(), UNKNOWN_REQUEST);
    }

    #[test]
    fn errors_and_hotplug() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("broken.lua");
        std::fs::write(&file, "x = \"").unwrap();
        let fake = FakeHyprland::new(setup(Some(file)));
        fake.reload().unwrap();
        assert_eq!(fake.config_errors().unwrap().len(), 1);
        let mut monitors = fake.current();
        monitors.pop();
        fake.set_monitors(monitors);
        assert_eq!(fake.monitors().unwrap().len(), 2);
    }

    #[test]
    fn setup_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.json");
        let text = format!(r#"{{ "config": "hypr-user.lua", "monitors": {MONITORS} }}"#);
        std::fs::write(&path, text).unwrap();
        std::fs::write(dir.path().join("hypr-user.lua"), HYPR_USER).unwrap();
        let fake = FakeHyprland::from_file(&path).unwrap();
        assert_eq!(fake.config_path(), Some(dir.path().join("hypr-user.lua")));
        assert_eq!(fake.provider(), Backend::Lua);
        fake.reload().unwrap();
        assert!(fake.config_errors().unwrap().is_empty());
        assert!(FakeHyprland::from_file(&dir.path().join("missing.json")).is_err());
        std::fs::write(&path, "{").unwrap();
        assert!(FakeHyprland::from_file(&path).is_err());
    }

    #[test]
    fn mode_selection() {
        let monitors: Vec<MonitorInfo> = serde_json::from_str(MONITORS).unwrap();
        let dp = &monitors[2];
        assert_eq!(pick_mode(dp, None).0, 2560);
        assert!((pick_mode(dp, Some(&Mode::HighRefreshRate)).2 - 179.95).abs() < 0.01);
        let r = pick_mode(dp, Some(&"1920x1080@165".parse().unwrap()));
        assert_eq!((r.0, r.1), (1920, 1080));
        assert!((r.2 - 164.92).abs() < 0.01);
        assert_eq!(pick_mode(dp, Some(&Mode::HighResolution)).0, 2560);
        assert_eq!(
            pick_mode(dp, Some(&"1234x567@60".parse().unwrap())),
            (1234, 567, 60.0)
        );
        let rules = [MonitorRule::new(""), MonitorRule::new("desc:Samsung")];
        assert_eq!(
            rule_for(&rules, "DP-1", &dp.description)
                .unwrap()
                .output
                .as_str(),
            "desc:Samsung"
        );
        assert_eq!(
            rule_for(&rules, "eDP-1", "Other").unwrap().output.as_str(),
            ""
        );
    }
}
