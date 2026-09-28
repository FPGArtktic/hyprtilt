// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Tests of the apply state machine: countdown, confirmation, missing
//! confirmation, signals, failed verification and IPC errors, first as a
//! pure machine, then end to end with the in-memory Hyprland.

use super::*;
use crate::ipc::fake::{FakeHyprland, FakeSetup};
use crate::layout::{Layout, RefreshTarget};

const MONITORS: &str = include_str!("../../tests/fixtures/ipc/monitors-all-0.56.2.json");
const HYPR_USER: &str = include_str!("../../tests/fixtures/lua/caelestia-hypr-user.lua");

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn monitors() -> Vec<MonitorInfo> {
    serde_json::from_str(MONITORS).unwrap()
}

/// Expect DP-1 at 0x0.
fn expect_dp_at_origin() -> Vec<Expectation> {
    vec![Expectation {
        output: "DP-1".to_owned(),
        enabled: true,
        pixels: None,
        refresh: None,
        position: Some((0, 0)),
        scale: None,
        transform: None,
    }]
}

fn moved() -> Vec<MonitorInfo> {
    let mut m = monitors();
    m[2].x = 0;
    m[2].y = 0;
    m
}

#[test]
fn confirmed_in_time() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    assert_eq!(s.phase(), Phase::Ready);
    assert_eq!(s.start(ms(0)), [Command::Apply]);
    assert_eq!(s.start(ms(0)), [], "starting twice does nothing");
    assert_eq!(s.handle(Input::Done(Ok(())), ms(10)), [Command::Query]);
    assert_eq!(s.phase(), Phase::Verifying);
    assert_eq!(s.handle(Input::Observed(Ok(moved())), ms(20)), []);
    assert_eq!(s.phase(), Phase::Confirming);
    assert_eq!(s.remaining(ms(5020)), Some(ms(10_000)));
    assert_eq!(s.handle(Input::Tick, ms(5020)), []);
    assert_eq!(
        s.handle(Input::Confirm, ms(6000)),
        [Command::Finish(Outcome::Kept)]
    );
    assert_eq!(s.outcome(), Some(&Outcome::Kept));
    assert_eq!(
        s.handle(Input::Cancel, ms(7000)),
        [],
        "finished sessions ignore input"
    );
}

#[test]
fn not_confirmed_rolls_back_after_the_countdown() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    s.start(ms(0));
    s.handle(Input::Done(Ok(())), ms(0));
    s.handle(Input::Observed(Ok(moved())), ms(0));
    assert_eq!(s.handle(Input::Tick, ms(14_999)), []);
    assert_eq!(s.handle(Input::Tick, ms(15_000)), [Command::Restore]);
    assert_eq!(s.phase(), Phase::RollingBack);
    assert_eq!(s.remaining(ms(15_000)), None);
    assert_eq!(s.handle(Input::Confirm, ms(15_001)), [], "too late");
    assert_eq!(
        s.handle(Input::Done(Ok(())), ms(15_100)),
        [Command::Finish(Outcome::RolledBack {
            reason: Reason::NotConfirmed
        })]
    );
}

#[test]
fn without_confirmation_a_verified_change_is_kept() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::with_confirm_seconds(0));
    s.start(ms(0));
    s.handle(Input::Done(Ok(())), ms(0));
    assert_eq!(
        s.handle(Input::Observed(Ok(moved())), ms(0)),
        [Command::Finish(Outcome::Kept)]
    );
}

#[test]
fn verification_polls_until_the_deadline() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    s.start(ms(0));
    s.handle(Input::Done(Ok(())), ms(0));
    // Not yet applied: keep polling every 100 ms.
    assert_eq!(s.handle(Input::Observed(Ok(monitors())), ms(0)), []);
    assert_eq!(s.mismatches().len(), 1);
    assert_eq!(s.handle(Input::Tick, ms(50)), []);
    assert_eq!(s.handle(Input::Tick, ms(100)), [Command::Query]);
    assert_eq!(s.handle(Input::Tick, ms(150)), [], "a query is pending");
    assert_eq!(
        s.handle(Input::Observed(Err("timeout".into())), ms(160)),
        []
    );
    assert_eq!(s.mismatches()[0].field, "monitor list");
    assert_eq!(s.handle(Input::Tick, ms(200)), [Command::Query]);
    assert_eq!(s.handle(Input::Observed(Ok(monitors())), ms(210)), []);
    let restore = s.handle(Input::Tick, ms(3000));
    assert_eq!(restore, [Command::Restore]);
    let finish = s.handle(Input::Done(Ok(())), ms(3100));
    let Command::Finish(Outcome::RolledBack {
        reason: Reason::Verification(m),
    }) = &finish[0]
    else {
        panic!("{finish:?}");
    };
    assert_eq!(m[0].to_string(), "DP-1: position is 3360x975, expected 0x0");
}

#[test]
fn a_late_mismatch_after_the_deadline_rolls_back_at_once() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    s.start(ms(0));
    s.handle(Input::Done(Ok(())), ms(0));
    assert_eq!(
        s.handle(Input::Observed(Ok(monitors())), ms(3500)),
        [Command::Restore]
    );
}

#[test]
fn failures_signals_and_cancellation() {
    let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    s.start(ms(0));
    assert_eq!(
        s.handle(Input::Done(Err("refused".into())), ms(0)),
        [Command::Restore]
    );
    let finish = s.handle(Input::Done(Err("gone".into())), ms(0));
    assert_eq!(
        finish,
        [Command::Finish(Outcome::RollbackFailed {
            reason: Reason::ApplyFailed("refused".into()),
            error: "gone".into()
        })]
    );
    for (input, reason) in [
        (Input::Signal, Reason::Signal),
        (Input::Cancel, Reason::Cancelled),
    ] {
        let mut s = Session::new(expect_dp_at_origin(), ApplyConfig::default());
        s.start(ms(0));
        s.handle(Input::Done(Ok(())), ms(0));
        s.handle(Input::Observed(Ok(moved())), ms(0));
        assert_eq!(s.handle(input, ms(1)), [Command::Restore]);
        assert_eq!(s.handle(Input::Signal, ms(2)), [], "already rolling back");
        assert_eq!(
            s.handle(Input::Done(Ok(())), ms(3)),
            [Command::Finish(Outcome::RolledBack { reason })]
        );
    }
    let mut idle = Session::new(Vec::new(), ApplyConfig::default());
    assert_eq!(idle.handle(Input::Tick, ms(0)), []);
}

#[test]
fn reasons_read_well() {
    assert_eq!(Reason::NotConfirmed.to_string(), "not confirmed in time");
    assert_eq!(Reason::Cancelled.to_string(), "rejected");
    assert_eq!(Reason::Signal.to_string(), "interrupted");
    assert_eq!(
        Reason::ApplyFailed("x".into()).to_string(),
        "applying failed: x"
    );
}

fn fake(config: Option<std::path::PathBuf>) -> FakeHyprland {
    FakeHyprland::new(FakeSetup {
        version: "0.56.2".to_owned(),
        provider: Backend::Lua,
        config,
        base_rules: Vec::new(),
        monitors: monitors(),
    })
}

/// Run a session to its end, ticking every 100 ms, answering with
/// `answer` once confirmation is asked for.
fn drive(session: &mut Session, effects: &mut dyn Effects, answer: Option<&Input>) -> Outcome {
    let mut now = Duration::ZERO;
    if let Some(done) = step(session, None, now, effects) {
        return done;
    }
    for _ in 0..1000 {
        now += ms(100);
        let input = match (session.phase(), &answer) {
            (Phase::Confirming, Some(a)) => (*a).clone(),
            _ => Input::Tick,
        };
        if let Some(done) = step(session, Some(input), now, effects) {
            return done;
        }
    }
    panic!("the session did not end");
}

#[test]
fn live_refresh_change_is_verified_and_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hypr-user.lua");
    std::fs::write(&file, HYPR_USER).unwrap();
    let hyprland = fake(Some(file));
    hyprland.reload().unwrap();
    hyprland.set_lag(3);
    let block = crate::lua::parse(HYPR_USER).unwrap();
    let rules: Vec<MonitorRule> = block
        .outside
        .iter()
        .filter_map(|f| f.rule.clone())
        .collect();
    let mut layout = Layout::new(&hyprland.monitors().unwrap(), &rules, false);
    let dp = layout.index("DP-1").unwrap();
    layout.set_refresh(dp, RefreshTarget::Hz(120.0)).unwrap();
    let apply = Action::set(Backend::Lua, &[layout.live_rule(dp)]).unwrap();
    assert!(matches!(&apply, Action::Eval(code) if code.contains("2560x1440@120")));
    let mut effects = IpcChange {
        ipc: &hyprland,
        apply,
        restore: Action::Reload,
    };
    let mut session = Session::new(layout.expectations(), ApplyConfig::default());
    assert_eq!(
        drive(&mut session, &mut effects, Some(&Input::Confirm)),
        Outcome::Kept
    );
    let dp_now = hyprland
        .current()
        .into_iter()
        .find(|m| m.name == "DP-1")
        .unwrap();
    assert!((dp_now.refresh_rate - 120.0).abs() < 0.01);
}

#[test]
fn unconfirmed_file_change_restores_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hypr-user.lua");
    std::fs::write(&file, HYPR_USER).unwrap();
    let hyprland = fake(Some(file.clone()));
    hyprland.reload().unwrap();
    let before = fsio::snapshot(&file).unwrap();
    let mut layout = Layout::new(&hyprland.monitors().unwrap(), &[], false);
    let hdmi = layout.index("HDMI-A-1").unwrap();
    layout.rotate(hdmi, false);
    let new = crate::lua::save(before.text(), &layout.rules())
        .unwrap()
        .content;
    let mut effects = FileChange::new(
        &hyprland,
        before,
        new,
        Some(BackupPolicy::default()),
        SystemTime::UNIX_EPOCH,
    );
    let mut session = Session::new(layout.expectations(), ApplyConfig::with_confirm_seconds(2));
    let outcome = drive(&mut session, &mut effects, None);
    assert_eq!(
        outcome,
        Outcome::RolledBack {
            reason: Reason::NotConfirmed
        }
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), HYPR_USER);
    assert!(effects.backup.is_some());
    let hdmi_now = hyprland
        .current()
        .into_iter()
        .find(|m| m.name == "HDMI-A-1")
        .unwrap();
    assert_eq!(
        hdmi_now.transform, 1,
        "the reload restored the portrait layout"
    );
}

#[test]
fn a_file_hyprland_does_not_load_fails_verification() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-loaded.lua");
    std::fs::write(&file, "").unwrap();
    // The compositor reads another file (none at all here).
    let hyprland = fake(None);
    let before = fsio::snapshot(&file).unwrap();
    let mut layout = Layout::new(&hyprland.monitors().unwrap(), &[], false);
    let dp = layout.index("DP-1").unwrap();
    layout.move_to(dp, 3360, 0);
    let new = crate::lua::save("", &layout.rules()).unwrap().content;
    let mut effects = FileChange::new(&hyprland, before, new, None, SystemTime::UNIX_EPOCH);
    let mut session = Session::new(layout.expectations(), ApplyConfig::default());
    let outcome = drive(&mut session, &mut effects, Some(&Input::Confirm));
    let Outcome::RolledBack {
        reason: Reason::Verification(m),
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(m[0].field, "position");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
}

#[test]
fn ipc_errors_during_apply_roll_back() {
    let hyprland = fake(None);
    hyprland.script("/eval", Err("connection reset".to_owned()));
    let mut effects = IpcChange {
        ipc: &hyprland,
        apply: Action::Eval("hl.monitor({ output = \"DP-1\" })".to_owned()),
        restore: Action::Reload,
    };
    let mut session = Session::new(expect_dp_at_origin(), ApplyConfig::default());
    let outcome = drive(&mut session, &mut effects, None);
    assert!(
        matches!(
            outcome,
            Outcome::RolledBack {
                reason: Reason::ApplyFailed(_)
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(hyprland.requests().last().unwrap(), "/reload");
}

#[test]
fn actions() {
    let mut rule = MonitorRule::new("DP-1");
    rule.mode = Some("2560x1440@144".parse().unwrap());
    assert_eq!(Action::set(Backend::Lua, &[]).unwrap(), Action::Nothing);
    let keywords = Action::set(Backend::Hyprlang, std::slice::from_ref(&rule)).unwrap();
    assert_eq!(
        keywords.requests(),
        ["/keyword monitor DP-1,2560x1440@144,auto,auto"]
    );
    assert_eq!(Action::Reload.requests(), ["/reload"]);
    assert!(Action::Nothing.requests().is_empty());
    assert!(Action::Eval("x".into()).requests()[0].starts_with("/eval "));
    let mut setup_rule = rule.clone();
    setup_rule.reserved = Some(crate::model::Reserved::default());
    assert!(Action::set(Backend::Hyprlang, &[setup_rule]).is_err());
    let mut hyprlang = FakeSetup {
        version: "0.53.3".to_owned(),
        provider: Backend::Hyprlang,
        config: None,
        base_rules: Vec::new(),
        monitors: monitors(),
    };
    hyprlang.monitors.truncate(3);
    let fake = FakeHyprland::new(hyprlang);
    keywords.run(&fake).unwrap();
    assert!(Action::Nothing.run(&fake).is_ok());
    assert!(Action::Eval("x".into()).run(&fake).is_err());
}
