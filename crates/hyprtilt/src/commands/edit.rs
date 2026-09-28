// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The per-output commands: `rotate`, `move`, `scale`, `mode`, `refresh`,
//! `enable` and `disable`. They are meant for keybindings: the change is
//! written into the managed block, reloaded and verified, and rolled back
//! if Hyprland does not show it; `--live` changes only the running
//! session. There is no confirmation prompt.

use hyprtilt_core::apply::{Action, ApplyConfig};
use hyprtilt_core::layout::{Layout, RefreshChange, RefreshTarget};
use hyprtilt_core::model::{Mode, MonitorRule, Position, Scale, format_refresh};
use serde::Serialize;

use super::{
    Workspace, Written, check_layout, describe_change, layout, live_change, print_json,
    running_backend, write_and_verify,
};
use crate::cli::Angle;
use crate::context::Context;
use crate::error::AppError;

/// What to change.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Change {
    /// Rotate by a step.
    Rotate(Angle),
    /// Move to a position.
    Move(i32, i32),
    /// Set the scale.
    Scale(Scale),
    /// Set the mode.
    Mode(Mode),
    /// Set the refresh rate.
    Refresh(RefreshArg),
    /// Enable or disable.
    Enable(bool),
}

/// The argument of `refresh`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RefreshArg {
    /// A rate or `max`/`min`.
    Target(RefreshTarget),
    /// The next higher (`up`) or lower rate.
    Step(bool),
}

/// Parse the arguments of the per-output commands.
pub(crate) fn parse_position(text: &str) -> Result<(i32, i32), AppError> {
    match text.parse::<Position>() {
        Ok(Position::At { x, y }) => Ok((x, y)),
        _ => Err(AppError::Other(format!(
            "invalid position {text:?}: expected XxY, such as 1440x1335"
        ))),
    }
}

/// Parse a scale argument.
pub(crate) fn parse_scale(text: &str) -> Result<Scale, AppError> {
    text.parse().map_err(|_| {
        AppError::Other(format!(
            "invalid scale {text:?}: expected a number of at least 0.25, or auto"
        ))
    })
}

/// Parse a mode argument.
pub(crate) fn parse_mode(text: &str) -> Result<Mode, AppError> {
    text.parse().map_err(|_| {
        AppError::Other(format!(
            "invalid mode {text:?}: expected WxH@Hz, preferred, highres or highrr"
        ))
    })
}

/// Parse a refresh argument.
pub(crate) fn parse_refresh(text: &str) -> Result<RefreshArg, AppError> {
    let t = text.trim().to_ascii_lowercase();
    Ok(match t.as_str() {
        "max" => RefreshArg::Target(RefreshTarget::Max),
        "min" => RefreshArg::Target(RefreshTarget::Min),
        "up" | "+" => RefreshArg::Step(true),
        "down" | "-" => RefreshArg::Step(false),
        _ => {
            let hz: f64 = t
                .trim_end_matches("hz")
                .parse()
                .ok()
                .filter(|h: &f64| h.is_finite() && *h > 0.0)
                .ok_or_else(|| {
                    AppError::Other(format!("invalid refresh rate {text:?}: expected Hz (such as 144), max, min, up or down"))
                })?;
            RefreshArg::Target(RefreshTarget::Hz(hz))
        }
    })
}

#[derive(Serialize)]
struct Report {
    output: String,
    changes: Vec<String>,
    moved: Vec<String>,
    rule: MonitorRule,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh: Option<RefreshChange>,
    live: bool,
    dry_run: bool,
    #[serde(flatten)]
    written: Written,
}

fn apply_change(
    layout: &mut Layout,
    i: usize,
    change: &Change,
) -> Result<Option<RefreshChange>, AppError> {
    match change {
        Change::Rotate(angle) => {
            let steps = match angle {
                Angle::Cw90 => 1,
                Angle::Half => 2,
                Angle::Cw270 | Angle::Ccw90 => 3,
            };
            for _ in 0..steps {
                layout.rotate(i, true);
            }
        }
        Change::Move(x, y) => layout.move_to(i, *x, *y),
        Change::Scale(s) => layout.set_scale(i, *s),
        Change::Mode(m) => layout.set_mode(i, m),
        Change::Enable(on) => layout.set_enabled(i, *on),
        Change::Refresh(RefreshArg::Target(t)) => return Ok(Some(layout.set_refresh(i, *t)?)),
        Change::Refresh(RefreshArg::Step(up)) => {
            let from = layout.outputs[i].rule.mode.as_ref().and_then(Mode::refresh);
            let Some(to) = layout.step_refresh(i, *up) else {
                let edge = if *up { "highest" } else { "lowest" };
                return Err(AppError::Other(format!(
                    "{} already runs at its {edge} refresh rate ({} Hz)",
                    layout.outputs[i].info.name,
                    format_refresh(layout.refresh(i))
                )));
            };
            return Ok(Some(RefreshChange {
                from,
                to,
                custom: false,
            }));
        }
    }
    Ok(None)
}

/// Run a per-output command.
pub(crate) fn run(
    ctx: &Context,
    output: &str,
    live: bool,
    change: &Change,
) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let (mut layout, _) = layout(ctx, &ws)?;
    let i = layout.index(output)?;
    let before: Vec<MonitorRule> = layout.outputs.iter().map(|o| o.rule.clone()).collect();
    let refresh = apply_change(&mut layout, i, change)?;
    check_layout(&layout)?;
    let changes = describe_change(&before[i], &layout.outputs[i].rule);
    let touched: Vec<usize> = (0..layout.outputs.len())
        .filter(|&k| layout.outputs[k].rule != before[k])
        .collect();
    let moved: Vec<String> = touched
        .iter()
        .filter(|&&k| k != i)
        .map(|&k| {
            let o = &layout.outputs[k];
            let at = o.rule.position.map(|p| p.to_string()).unwrap_or_default();
            format!("{} moved to {at}", o.info.name)
        })
        .collect();
    let expectations: Vec<_> = layout
        .expectations()
        .into_iter()
        .enumerate()
        .filter(|(k, _)| touched.contains(k))
        .map(|(_, e)| e)
        .collect();
    if let Some(r) = refresh
        && r.custom
    {
        eprintln!(
            "hyprtilt: warning: {} lists no mode within 1 Hz of {} Hz; Hyprland will try a custom mode",
            layout.outputs[i].info.name,
            format_refresh(r.to)
        );
    }
    let config = ApplyConfig::with_confirm_seconds(0);
    let written = if touched.is_empty() {
        Written::default()
    } else if live {
        let backend = running_backend(ctx, ws.target.backend);
        let rules: Vec<MonitorRule> = touched.iter().map(|&k| layout.live_rule(k)).collect();
        let apply = Action::set(backend, &rules)?;
        live_change(ctx, apply, Action::Reload, expectations, config)?
    } else {
        let content = ws
            .target
            .backend
            .save(ws.snapshot.text(), &layout.rules(), &ctx.save_options())?
            .content;
        write_and_verify(ctx, &ws, &content, expectations, config)?
    };
    let report = Report {
        output: layout.outputs[i].info.name.clone(),
        changes,
        moved,
        rule: layout.outputs[i].rule.clone(),
        refresh,
        live,
        dry_run: ctx.global.dry_run,
        written,
    };
    if ctx.global.json {
        return print_json(&report);
    }
    if !ctx.global.dry_run {
        print_summary(&report, &ws);
    }
    Ok(())
}

fn print_summary(r: &Report, ws: &Workspace) {
    if r.changes.is_empty() {
        println!("{}: nothing to change", r.output);
        return;
    }
    println!("{}: {}", r.output, r.changes.join(", "));
    for m in &r.moved {
        println!("{m}");
    }
    if r.live {
        println!("applied to the running session (until the next reload)");
    } else if r.written.changed {
        let backup = r
            .written
            .backup
            .as_ref()
            .map(|b| format!(" (backup: {})", b.display()))
            .unwrap_or_default();
        println!("wrote {}{backup}", ws.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        assert_eq!(parse_position("1440x1335").unwrap(), (1440, 1335));
        assert!(parse_position("auto").is_err());
        assert_eq!(parse_scale("1.25").unwrap(), Scale::Factor(1.25));
        assert!(parse_scale("0.1").is_err());
        assert!(parse_mode("2560x1440@144").is_ok());
        assert!(parse_mode("wide").is_err());
        assert_eq!(
            parse_refresh("MAX").unwrap(),
            RefreshArg::Target(RefreshTarget::Max)
        );
        assert_eq!(
            parse_refresh("min").unwrap(),
            RefreshArg::Target(RefreshTarget::Min)
        );
        assert_eq!(parse_refresh("up").unwrap(), RefreshArg::Step(true));
        assert_eq!(parse_refresh("-").unwrap(), RefreshArg::Step(false));
        assert_eq!(
            parse_refresh("144Hz").unwrap(),
            RefreshArg::Target(RefreshTarget::Hz(144.0))
        );
        assert!(parse_refresh("fast").is_err());
        assert!(parse_refresh("0").is_err());
    }
}
