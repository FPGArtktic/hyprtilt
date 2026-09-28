// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the typed IPC helpers against replies captured from a real
//! Hyprland 0.56.2 session.

use super::*;
use std::collections::HashMap;

const MONITORS: &str = include_str!("../../tests/fixtures/ipc/monitors-all-0.56.2.json");
const VERSION: &str = include_str!("../../tests/fixtures/ipc/version-0.56.2.json");
const STATUS: &str = include_str!("../../tests/fixtures/ipc/status-0.56.2.json");
const NO_ERRORS: &str = include_str!("../../tests/fixtures/ipc/configerrors-empty-0.56.2.json");

/// Fixed replies by request.
struct Canned(HashMap<&'static str, &'static str>);

impl HyprlandIpc for Canned {
    fn request(&self, request: &str) -> Result<String, IpcError> {
        Ok(self
            .0
            .get(request)
            .copied()
            .unwrap_or(UNKNOWN_REQUEST)
            .to_owned())
    }
}

fn live() -> Canned {
    Canned(HashMap::from([
        ("j/monitors all", MONITORS),
        ("j/version", VERSION),
        ("j/status", STATUS),
        ("j/configerrors", NO_ERRORS),
        ("/reload", "ok"),
        (
            "/eval hl.monitor({})",
            "error: hl.monitor: 'output' field is required and must be a string\n",
        ),
        (
            "/keyword monitor x",
            "keyword can't work with non-legacy parsers. Use eval.",
        ),
    ]))
}

#[test]
fn captured_replies_parse() {
    let ipc = live();
    let monitors = ipc.monitors().unwrap();
    let names: Vec<&str> = monitors.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["eDP-1", "HDMI-A-1", "DP-1"]);
    let hdmi = &monitors[1];
    assert_eq!(
        (hdmi.pixels(), hdmi.transform, hdmi.x, hdmi.y),
        ((2560, 1440), 1, 0, 0)
    );
    assert_eq!(hdmi.physical_width, 700);
    assert_eq!(hdmi.mirror_name(&monitors), None);
    // availableModes lists duplicates; modes() removes them.
    assert_eq!(hdmi.available_modes.len(), 35);
    assert_eq!(hdmi.modes().len(), 34);
    assert_eq!(hdmi.modes()[0].to_string(), "2560x1440@144");
    let dp = &monitors[2];
    assert!((dp.refresh_rate - 179.952).abs() < 1e-9);
    assert_eq!(dp.active_workspace.as_ref().unwrap().name, "3");
    let version = ipc.version().unwrap();
    assert_eq!(
        version.parsed(),
        Some(crate::version::Version::new(0, 56, 2))
    );
    assert_eq!(ipc.status().unwrap().unwrap().config_provider, "lua");
    assert!(ipc.config_errors().unwrap().is_empty());
    ipc.reload().unwrap();
}

#[test]
fn refusals_and_old_versions() {
    let ipc = live();
    let err = ipc.eval("hl.monitor({})").unwrap_err();
    assert_eq!(
        err.to_string(),
        "Hyprland refused \"/eval hl.monitor({})\": error: hl.monitor: 'output' field is required and must be a string"
    );
    assert!(matches!(
        ipc.keyword("monitor", "x"),
        Err(IpcError::Refused { .. })
    ));
    let old = Canned(HashMap::from([("j/configerrors", "[\"a\", \"\", \"b\"]")]));
    assert_eq!(old.status().unwrap(), None);
    assert!(old.monitors().unwrap().is_empty());
    assert_eq!(old.config_errors().unwrap(), ["a", "b"]);
    let broken = Canned(HashMap::from([("j/version", "{")]));
    assert!(matches!(broken.version(), Err(IpcError::BadReply { .. })));
}

#[test]
fn mirrors_resolve_to_names() {
    let mut monitors: Vec<MonitorInfo> = serde_json::from_str(MONITORS).unwrap();
    monitors[0].mirror_of = "2".to_owned();
    let all = monitors.clone();
    assert_eq!(monitors[0].mirror_name(&all), Some("DP-1"));
    monitors[0].mirror_of = "9".to_owned();
    assert_eq!(monitors[0].mirror_name(&all), None);
}
