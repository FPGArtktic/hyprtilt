// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! End-to-end tests of the `hyprtilt` binary against the in-memory
//! Hyprland (`--fake-hyprland`), in a temporary home directory. The real
//! configuration of the machine running the tests is never touched.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MONITORS: &str =
    include_str!("../../hyprtilt-core/tests/fixtures/ipc/monitors-all-0.56.2.json");
const HYPR_USER: &str =
    include_str!("../../hyprtilt-core/tests/fixtures/lua/caelestia-hypr-user.lua");
const HYPRLAND_CONF: &str =
    include_str!("../../hyprtilt-core/tests/fixtures/hyprlang/hyprland.conf");

/// A temporary home with a configuration file and a fake compositor.
struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Setup {
    fn lua() -> Setup {
        Setup::new("hypr-user.lua", HYPR_USER, "lua", "0.56.2")
    }

    fn hyprlang() -> Setup {
        Setup::new("hyprland.conf", HYPRLAND_CONF, "hyprlang", "0.53.3")
    }

    fn new(name: &str, content: &str, provider: &str, version: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(name);
        std::fs::write(&config, content).unwrap();
        let setup = format!(
            r#"{{ "version": "{version}", "provider": "{provider}", "config": "{name}", "monitors": {MONITORS} }}"#
        );
        std::fs::write(dir.path().join("fake.json"), setup).unwrap();
        std::fs::create_dir(dir.path().join("home")).unwrap();
        std::fs::create_dir(dir.path().join("run")).unwrap();
        Setup { dir, config }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn content(&self) -> String {
        std::fs::read_to_string(&self.config).unwrap()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hyprtilt"));
        cmd.arg("--fake-hyprland").arg(self.path("fake.json"));
        cmd.args(args);
        isolate(&mut cmd, self.dir.path());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).stdin(Stdio::null()).output().unwrap()
    }

    fn run_with_input(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn isolate(cmd: &mut Command, dir: &Path) {
    cmd.env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home/.config"))
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("HYPRLAND_CONFIG");
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", stdout(out)))
}

#[test]
fn version_prints_crate_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_hyprtilt"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        stdout(&out),
        format!("hyprtilt {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn list_shows_live_state_and_rules() {
    let s = Setup::lua();
    let out = s.run(&["list"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("HDMI-A-1  on     2560x1440@144     0x0        1      90°        none"),
        "{text}"
    );
    assert!(
        text.contains("3 rule(s) outside the block (line 5, 6, 7)"),
        "{text}"
    );
    let out = s.run(&["list", "--json"]);
    let v = json(&out);
    assert_eq!(v["outputs"].as_array().unwrap().len(), 3);
    assert_eq!(v["outside_rules"][0]["line"], 5);
    assert_eq!(v["hyprland"]["provider"], "lua");
}

#[test]
fn adopt_then_edit_keeps_the_rest_of_the_file() {
    let s = Setup::lua();
    let out = s.run(&["adopt"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).starts_with("adopted 3 rule(s) from line 5, 6, 7"));
    let adopted = s.content();
    assert!(adopted.contains("-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"HDMI-A-1\""));
    let v = json(&s.run(&["list", "--json"]));
    assert_eq!(v["block"]["begin_line"], 5);
    assert_eq!(v["outputs"][0]["rule_line"], 6);

    let out = s.run(&["rotate", "HDMI-A-1", "-90"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("HDMI-A-1: transform 1 -> 0"), "{text}");
    assert!(text.contains("eDP-1 moved to 2560x1335"), "{text}");
    let rotated = s.content();
    let tail = |t: &str| t[t.find("-- END hyprtilt").unwrap()..].to_owned();
    let head = |t: &str| t[..t.find("-- BEGIN").unwrap()].to_owned();
    assert_eq!(
        tail(&rotated),
        tail(
            HYPR_USER
                .replace(
                    &format!("{}\n", HYPR_USER.lines().nth(6).unwrap()),
                    &format!("{}\n-- END hyprtilt\n", HYPR_USER.lines().nth(6).unwrap()),
                )
                .as_str()
        )
    );
    assert_eq!(head(&rotated), head(&adopted));
    let backups: Vec<_> = std::fs::read_dir(s.dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("hypr-user.lua.bak.")
        })
        .collect();
    assert_eq!(backups.len(), 2);
}

#[test]
fn refresh_rates() {
    let s = Setup::lua();
    assert_eq!(code(&s.run(&["adopt"])), 0);
    let out = s.run(&["refresh", "DP-1", "down", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["refresh"]["from"], 179.95);
    assert_eq!(v["refresh"]["to"], 164.98);
    assert_eq!(v["verified"], true);
    assert!(s.content().contains("\"2560x1440@164.98\""));
    let out = s.run(&["refresh", "DP-1", "max"]);
    assert_eq!(
        stdout(&out).lines().next(),
        Some("DP-1: mode 2560x1440@164.98 -> 2560x1440@179.95")
    );
    let out = s.run(&["refresh", "DP-1", "up"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("already runs at its highest refresh rate (179.95 Hz)"));
    let out = s.run(&["refresh", "eDP-1", "60"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(s.content().contains("\"1920x1080@60\""));
    // No 144 Hz mode on DP-1: the change is tried, not verified, rolled back.
    let before = s.content();
    let out = s.run(&["refresh", "DP-1", "144"]);
    assert_eq!(code(&out), 6, "{}", stderr(&out));
    assert!(stderr(&out).contains("will try a custom mode"));
    assert!(
        stderr(&out).contains("DP-1: refresh rate is 164.98 Hz, expected 144 Hz"),
        "{}",
        stderr(&out)
    );
    assert_eq!(s.content(), before, "the file was restored");
    assert_eq!(code(&s.run(&["refresh", "DP-1", "fast"])), 1);
}

#[test]
fn every_per_output_command() {
    let s = Setup::lua();
    let cases: &[(&[&str], &str)] = &[
        (&["scale", "eDP-1", "1.25"], "scale 1 -> 1.25"),
        (
            &["mode", "eDP-1", "1920x1080@60"],
            "mode 1920x1080@144 -> 1920x1080@60",
        ),
        (&["move", "DP-1", "4000x0"], "-> 4000x0"),
        (&["rotate", "DP-1", "180"], "transform 0 -> 2"),
        (&["disable", "HDMI-A-1"], "disabled unset -> true"),
        (&["enable", "HDMI-A-1"], "disabled true -> unset"),
    ];
    for (args, change) in cases {
        let out = s.run(args);
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
        assert!(stdout(&out).contains(change), "{args:?}: {}", stdout(&out));
    }
    // The rules that had no block got one.
    assert!(s.content().contains("-- BEGIN hyprtilt (managed)"));
    let out = s.run(&["enable", "DP-9"]);
    assert_eq!(code(&out), 5);
    assert_eq!(stderr(&out), "hyprtilt: no output \"DP-9\"\n");
    assert_eq!(code(&s.run(&["move", "DP-1", "auto"])), 1);
    assert_eq!(code(&s.run(&["scale", "DP-1", "0.1"])), 1);
    assert_eq!(code(&s.run(&["mode", "DP-1", "wide"])), 1);
    assert_eq!(code(&s.run(&["rotate", "DP-1", "45"])), 2);
}

#[test]
fn overlaps_are_refused_and_dry_run_changes_nothing() {
    let s = Setup::lua();
    let out = s.run(&["move", "DP-1", "3000x975"]);
    assert_eq!(code(&out), 4);
    assert!(stderr(&out).contains("eDP-1 and DP-1 overlap"));
    assert_eq!(s.content(), HYPR_USER);
    let out = s.run(&["rotate", "HDMI-A-1", "90", "--dry-run"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("+-- BEGIN hyprtilt (managed)"), "{text}");
    assert!(text.ends_with("then: /reload\n"), "{text}");
    assert_eq!(s.content(), HYPR_USER);
    let v = json(&s.run(&["adopt", "--dry-run", "--json"]));
    assert_eq!(v["changed"], true);
    assert_eq!(v["dry_run"], true);
    assert_eq!(s.content(), HYPR_USER);
}

#[test]
fn live_changes_leave_the_file_alone() {
    let s = Setup::lua();
    let v = json(&s.run(&["scale", "eDP-1", "2", "--live", "--json"]));
    assert_eq!(v["live"], true);
    assert_eq!(v["verified"], true);
    assert!(
        v["requests"][0]
            .as_str()
            .unwrap()
            .starts_with("/eval hl.monitor({ output = \"eDP-1\"")
    );
    assert_eq!(s.content(), HYPR_USER);
    let out = s.run(&["disable", "eDP-1", "--live", "--dry-run"]);
    assert!(stdout(&out).contains("disabled = true"), "{}", stdout(&out));
}

#[test]
fn doctor_reports_a_file_hyprland_does_not_load() {
    let s = Setup::lua();
    let out = s.run(&["doctor"]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert!(stdout(&out).ends_with("No problems found.\n"));
    let other = s.path("other.lua");
    std::fs::write(&other, "-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\", mode = \"2560x1440@179.95\", position = \"0x0\", scale = 1 })\n-- END hyprtilt\n").unwrap();
    let out = s.run(&["doctor", "--file", other.to_str().unwrap(), "--json"]);
    assert_eq!(code(&out), 7);
    let v = json(&out);
    let live = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "live-state")
        .unwrap();
    assert_eq!(live["status"], "warn");
    assert!(v["problems"].as_u64().unwrap() >= 1);
}

#[test]
fn hyprlang_configuration() {
    let s = Setup::hyprlang();
    let out = s.run(&["adopt", "--line", "5", "--line", "6", "--line", "7"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = s.run(&["adopt"]);
    assert_eq!(code(&out), 4, "the addreserved patch is in the way");
    assert!(
        stderr(&out).contains("line 10: this monitor rule stays outside the block"),
        "{}",
        stderr(&out)
    );
    let out = s.run(&["refresh", "HDMI-A-1", "120"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        s.content()
            .contains("monitor = HDMI-A-1, 2560x1440@120, 0x0, 1, transform, 1"),
        "{}",
        s.content()
    );
    // DP-1 is also matched by the monitorv2 block on line 13, which
    // Hyprland applies last: the change cannot take effect and is undone,
    // and doctor names the rule.
    let before = s.content();
    let out = s.run(&["refresh", "DP-1", "120"]);
    assert_eq!(code(&out), 6, "{}", stderr(&out));
    assert_eq!(s.content(), before);
    let out = s.run(&["doctor"]);
    assert_eq!(code(&out), 7);
    assert!(stdout(&out).contains("line 13: the rule for \"desc:Samsung Electric Company Odyssey G50F SERIAL0002\" also applies to DP-1"), "{}", stdout(&out));
    let v = json(&s.run(&["move", "eDP-1", "1440x1480", "--live", "--json"]));
    assert_eq!(
        v["requests"][0],
        "/keyword monitor eDP-1,1920x1080@144,1440x1480,1,transform,0"
    );
    // A Lua file under a hyprlang compositor is refused.
    let lua = s.path("x.lua");
    std::fs::write(&lua, "").unwrap();
    let out = s.run(&["rotate", "DP-1", "90", "--file", lua.to_str().unwrap()]);
    assert_eq!(code(&out), 4);
    assert!(
        stderr(&out).contains("Hyprland runs a hyprlang configuration"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn profiles_with_confirmation() {
    let s = Setup::lua();
    assert_eq!(
        stdout(&s.run(&["profile", "list"])).lines().count(),
        1,
        "no profiles yet"
    );
    let out = s.run(&["profile", "save", "desk", "--desc"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        json(&s.run(&["profile", "list", "--json"])),
        serde_json::json!(["desk"])
    );
    // Rejected: the file is restored and the exit code says so.
    let out = s.run_with_input(
        &["apply", "--profile", "desk", "--confirm-timeout", "5"],
        "n\n",
    );
    assert_eq!(code(&out), 6, "{}", stderr(&out));
    assert!(stderr(&out).contains("rolled back: rejected"));
    assert_eq!(s.content(), HYPR_USER);
    // Confirmed.
    let out = s.run_with_input(&["profile", "apply", "desk"], "y\n");
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        s.content()
            .contains("desc:Samsung Electric Company Odyssey G50F SERIAL0001")
    );
    // No answer: the countdown runs out.
    let out = s.run(&["apply", "--confirm-timeout", "1"]);
    assert_eq!(code(&out), 6, "{}", stderr(&out));
    assert!(stderr(&out).contains("not confirmed in time"));
    assert_eq!(code(&s.run(&["apply", "--no-confirm"])), 0);
    assert_eq!(code(&s.run(&["profile", "delete", "desk"])), 0);
    assert_eq!(code(&s.run(&["profile", "delete", "desk"])), 1);
    assert_eq!(code(&s.run(&["profile", "apply", "../x"])), 1);
}

#[test]
fn a_signal_during_the_countdown_rolls_back() {
    let s = Setup::lua();
    assert_eq!(code(&s.run(&["profile", "save", "desk"])), 0);
    assert_eq!(code(&s.run(&["rotate", "HDMI-A-1", "-90"])), 0);
    let before = s.content();
    let mut child = s
        .command(&["profile", "apply", "desk"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the prompt, then send SIGTERM.
    let mut err = child.stderr.take().unwrap();
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 256];
    while !String::from_utf8_lossy(&seen).contains("Keep this layout?") {
        assert!(Instant::now() < deadline, "no prompt");
        let n = err.read(&mut buf).unwrap();
        seen.extend_from_slice(&buf[..n]);
    }
    assert!(
        Command::new("kill")
            .arg("-TERM")
            .arg(child.id().to_string())
            .status()
            .unwrap()
            .success()
    );
    let status = child.wait().unwrap();
    let mut rest = String::new();
    err.read_to_string(&mut rest).unwrap();
    assert_eq!(status.code(), Some(6), "{rest}");
    assert!(rest.contains("rolled back: interrupted"), "{rest}");
    assert_eq!(s.content(), before);
}

#[test]
fn save_and_unmanage() {
    let s = Setup::lua();
    let out = s.run(&["save"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("wrote the running layout of 3 output(s)"),
        "{}",
        stdout(&out)
    );
    let out = s.run(&["save"]);
    assert!(
        stdout(&out).contains("already describes the running layout"),
        "{}",
        stdout(&out)
    );
    let out = s.run(&["unmanage"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!s.content().contains("hyprtilt"));
    let out = s.run(&["unmanage"]);
    assert_eq!(code(&out), 4);
    assert_eq!(stderr(&out), "hyprtilt: the file has no managed block\n");
}

#[test]
fn without_hyprland() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    let file = dir.path().join("hypr-user.lua");
    std::fs::write(&file, crate_adopted()).unwrap();
    let run = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hyprtilt"));
        cmd.args(args).arg("--file").arg(&file).stdin(Stdio::null());
        isolate(&mut cmd, dir.path());
        cmd.output().unwrap()
    };
    let out = run(&["list"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("Hyprland is not reachable"),
        "{}",
        stdout(&out)
    );
    let out = run(&["rotate", "HDMI-A-1", "-90"]);
    assert_eq!(code(&out), 3, "{}", stderr(&out));
    let out = run(&["rotate", "HDMI-A-1", "-90", "--dry-run"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains(
            "+hl.monitor({ output = \"eDP-1\", mode = \"1920x1080@144\", position = \"2560x1335\""
        ),
        "{}",
        stdout(&out)
    );
    let out = run(&["doctor"]);
    assert_eq!(code(&out), 7);
    assert!(stdout(&out).starts_with("[FAIL] Hyprland cannot be reached"));
}

fn crate_adopted() -> String {
    let lines: Vec<&str> = HYPR_USER.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i == 4 {
            out.push("-- BEGIN hyprtilt (managed)".to_owned());
        }
        out.push((*l).to_owned());
        if i == 6 {
            out.push("-- END hyprtilt".to_owned());
        }
    }
    out.join("\n") + "\n"
}

#[test]
fn completions_and_man_page() {
    for shell in ["bash", "zsh", "fish"] {
        let out = Command::new(env!("CARGO_BIN_EXE_hyprtilt"))
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(stdout(&out).contains("refresh"), "{shell}");
    }
    let out = Command::new(env!("CARGO_BIN_EXE_hyprtilt"))
        .arg("man")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).starts_with(".ie"), "{}", &stdout(&out)[..40]);
}
