// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the doctor's conclusions.

use super::*;
use std::path::PathBuf;

const MONITORS: &str = include_str!("../../tests/fixtures/ipc/monitors-all-0.56.2.json");
const HYPR_USER: &str = include_str!("../../tests/fixtures/lua/caelestia-hypr-user.lua");

const TARGET: &str = "/home/u/.config/caelestia/hypr-user.lua";

fn healthy() -> Facts {
    let adopted = crate::lua::adopt(HYPR_USER, &[]).unwrap().content;
    Facts {
        hyprland: Ok(Live {
            version: "0.56.2".to_owned(),
            provider: Some("lua".to_owned()),
            monitors: serde_json::from_str(MONITORS).unwrap(),
            config_errors: Vec::new(),
        }),
        main_config: MainConfig {
            path: PathBuf::from("/home/u/.config/hypr/hyprland.lua"),
            source: MainConfigSource::Xdg,
            backend: Backend::Lua,
        },
        target: Target {
            path: PathBuf::from(TARGET),
            backend: Backend::Lua,
            reason: TargetReason::Caelestia,
            warnings: Vec::new(),
        },
        target_content: Ok(Some(adopted)),
        target_inode: Some(0xa1_58fc),
        read_only: None,
        process: Some(ProcessInfo {
            pid: 3436,
            watched_inodes: Some(vec![0x9e_222a, 0xa1_58fc]),
            ..ProcessInfo::default()
        }),
        luac: None,
    }
}

fn find<'c>(checks: &'c [Check], id: &str) -> Vec<&'c Check> {
    checks.iter().filter(|c| c.id == id).collect()
}

fn status(checks: &[Check], id: &str) -> Status {
    find(checks, id)[0].status
}

#[test]
fn a_healthy_caelestia_setup() {
    let checks = diagnose(&healthy());
    let problems: Vec<&Check> = checks.iter().filter(|c| c.status.is_problem()).collect();
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(status(&checks, "loaded"), Status::Ok);
    assert_eq!(status(&checks, "live-state"), Status::Ok);
    assert_eq!(
        find(&checks, "target-file")[0].title,
        "managed block on lines 5-9 with 3 rule(s)"
    );
    assert!(
        find(&checks, "target")[0]
            .title
            .contains("Caelestia preset")
    );
    assert_eq!(status(&checks, "syntax-check"), Status::Info);
}

#[test]
fn an_unloaded_file_is_reported_three_ways() {
    let mut facts = healthy();
    facts.process.as_mut().unwrap().watched_inodes = Some(vec![0x9e_222a]);
    if let Ok(live) = &mut facts.hyprland {
        live.monitors[1].transform = 0;
        live.config_errors = vec![format!("{TARGET}:9: hl.monitor: unknown field 'x'")];
    }
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "loaded"), Status::Warn);
    assert_eq!(status(&checks, "config-errors"), Status::Fail);
    let live = find(&checks, "live-state")[0];
    assert_eq!(live.status, Status::Warn);
    assert!(
        live.detail
            .as_ref()
            .unwrap()
            .starts_with("HDMI-A-1: transform is 0, expected 1")
    );
    // Unrelated errors only warn.
    if let Ok(live) = &mut facts.hyprland {
        live.config_errors = vec!["/elsewhere.lua:1: oops".to_owned()];
    }
    assert_eq!(status(&diagnose(&facts), "config-errors"), Status::Warn);
    // Nothing watched at all, or no process: cannot tell.
    facts.process.as_mut().unwrap().watched_inodes = Some(Vec::new());
    assert_eq!(status(&diagnose(&facts), "loaded"), Status::Info);
    facts.process = None;
    assert_eq!(status(&diagnose(&facts), "loaded"), Status::Info);
}

#[test]
fn backend_mismatches() {
    let mut facts = healthy();
    facts.target.backend = Backend::Hyprlang;
    facts.target.path = PathBuf::from("/home/u/.config/hypr/hyprland.conf");
    facts.target_content = Ok(Some("monitor = DP-1, preferred, auto, 1\n".to_owned()));
    assert_eq!(status(&diagnose(&facts), "backend"), Status::Fail);
    if let Ok(live) = &mut facts.hyprland {
        live.provider = Some("hyprlang".to_owned());
    }
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "backend"), Status::Ok);
    assert_eq!(status(&checks, "hyprlang-deprecated"), Status::Info);
    let mut old = healthy();
    if let Ok(live) = &mut old.hyprland {
        live.version = "0.53.3".to_owned();
        live.provider = None;
    }
    let checks = diagnose(&old);
    assert_eq!(status(&checks, "backend"), Status::Fail);
    assert!(
        find(&checks, "backend")[0]
            .title
            .contains("never reads a Lua file")
    );
}

#[test]
fn unreachable_hyprland_still_checks_the_file() {
    let mut facts = healthy();
    facts.hyprland = Err("no socket".to_owned());
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "hyprland"), Status::Fail);
    assert!(find(&checks, "backend").is_empty());
    assert_eq!(status(&checks, "target-file"), Status::Ok);
}

#[test]
fn file_problems() {
    let mut facts = healthy();
    facts.target_content = Ok(Some(
        "hl.monitor({ output = \"DP-1\", vrr = 1 })\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\", scale = 1 })\n-- END hyprtilt\nif x then hl.monitor({ output = \"eDP-1\" }) end\n".to_owned(),
    ));
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "conflicting-rule"), Status::Warn);
    assert_eq!(find(&checks, "outside-rules").len(), 2);
    facts.target_content = Ok(Some(
        "-- BEGIN hyprtilt (managed)\nx = 1\n-- END hyprtilt\n".to_owned(),
    ));
    assert_eq!(status(&diagnose(&facts), "target-file"), Status::Fail);
    facts.target_content = Err("permission denied".to_owned());
    assert_eq!(status(&diagnose(&facts), "target-file"), Status::Fail);
    facts.target_content = Ok(None);
    facts.target_inode = None;
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "target-file"), Status::Info);
    assert_eq!(status(&checks, "loaded"), Status::Info);
    facts.read_only = Some("managed by Nix".to_owned());
    assert_eq!(status(&diagnose(&facts), "writable"), Status::Fail);
    facts.target.warnings.push("shadowed".to_owned());
    assert!(
        diagnose(&facts)
            .iter()
            .any(|c| c.id == "target" && c.status == Status::Warn)
    );
}

#[test]
fn layout_and_selector_problems() {
    let mut facts = healthy();
    facts.target_content = Ok(Some(
        "-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"desc:Samsung Electric Company Odyssey G50F\", position = \"0x0\", scale = 1 })\nhl.monitor({ output = \"eDP-1\", position = \"0x0\", scale = 1 })\n-- END hyprtilt\n".to_owned(),
    ));
    let checks = diagnose(&facts);
    assert_eq!(status(&checks, "selectors"), Status::Warn);
    assert!(
        find(&checks, "layout")
            .iter()
            .any(|c| c.status == Status::Fail)
    );
}

#[test]
fn syntax_checker_and_status_order() {
    let mut facts = healthy();
    facts.luac = Some(Luac {
        path: PathBuf::from("/usr/bin/luac5.5"),
        version: "5.5.1".to_owned(),
    });
    assert_eq!(status(&diagnose(&facts), "syntax-check"), Status::Ok);
    assert!(Status::Fail.is_problem() && Status::Warn.is_problem());
    assert!(!Status::Info.is_problem() && !Status::Ok.is_problem());
    let json = serde_json::to_string(&diagnose(&facts)[0]).unwrap();
    assert!(json.contains("\"status\":\"ok\""), "{json}");
}

#[test]
fn rules_that_win_over_the_block() {
    let mut facts = healthy();
    let adopted = crate::lua::adopt(HYPR_USER, &[]).unwrap().content;
    facts.target_content = Ok(Some(format!(
        "{adopted}hl.monitor({{ output = \"desc:Samsung Electric Company Odyssey G50F SERIAL0002\", scale = 2 }})\n"
    )));
    let checks = diagnose(&facts);
    let found = find(&checks, "shadowing-rule");
    assert_eq!(found.len(), 1);
    assert!(
        found[0].title.contains("also applies to DP-1"),
        "{}",
        found[0].title
    );
    // In hyprlang a monitorv2 block wins wherever it is.
    let mut facts = healthy();
    facts.target.backend = Backend::Hyprlang;
    if let Ok(live) = &mut facts.hyprland {
        live.provider = Some("hyprlang".to_owned());
    }
    facts.target_content = Ok(Some(
        "monitorv2 {\n  output = desc:Samsung Electric Company Odyssey G50F SERIAL0002\n}\n# BEGIN hyprtilt (managed)\nmonitor = DP-1, preferred, auto, 1\n# END hyprtilt\n".to_owned(),
    ));
    assert_eq!(find(&diagnose(&facts), "shadowing-rule").len(), 1);
}
