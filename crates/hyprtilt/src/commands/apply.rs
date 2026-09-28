// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `apply` and `profile apply`: apply the configuration file or a profile
//! with verification and a confirmation countdown.

use hyprtilt_core::apply::{Action, ApplyConfig};
use hyprtilt_core::config::{Target, TargetReason};
use hyprtilt_core::document::Backend;
use hyprtilt_core::layout::{Layout, Origin};
use serde::Serialize;

use super::{
    Workspace, Written, check_layout, live_change, print_json, running_backend, write_and_verify,
};
use crate::context::Context;
use crate::error::AppError;

#[derive(Serialize)]
struct Report<'a> {
    profile: Option<&'a str>,
    dry_run: bool,
    #[serde(flatten)]
    written: Written,
}

/// The confirmation timing from the options and the settings.
fn timing(ctx: &Context, confirm_timeout: Option<u64>, no_confirm: bool) -> ApplyConfig {
    let seconds = if no_confirm {
        0
    } else {
        confirm_timeout.unwrap_or(ctx.settings.confirm_timeout)
    };
    ApplyConfig::with_confirm_seconds(seconds)
}

/// Run `apply`.
pub(crate) fn run(
    ctx: &Context,
    profile: Option<&str>,
    confirm_timeout: Option<u64>,
    no_confirm: bool,
) -> Result<(), AppError> {
    let config = timing(ctx, confirm_timeout, no_confirm);
    let written = match profile {
        Some(name) => apply_profile(ctx, name, config)?,
        None => apply_file(ctx, config)?,
    };
    if ctx.global.json {
        return print_json(&Report {
            profile,
            dry_run: ctx.global.dry_run,
            written,
        });
    }
    if !ctx.global.dry_run {
        match profile {
            Some(name) if !written.changed => {
                println!("nothing to change: the file already holds profile {name:?}");
            }
            Some(name) => println!("profile {name:?} applied and kept"),
            None => println!("the configuration file is applied and kept"),
        }
    }
    Ok(())
}

/// Write the profile's rules into the block of its target.
fn apply_profile(ctx: &Context, name: &str, config: ApplyConfig) -> Result<Written, AppError> {
    let profile = ctx.profiles().load(name, ctx.home.as_deref())?;
    let mut target = ctx.target();
    if let Some(path) = &profile.target {
        target = Target {
            backend: profile.backend.unwrap_or_else(|| Backend::for_path(path)),
            path: path.clone(),
            reason: TargetReason::Explicit,
            warnings: Vec::new(),
        };
    } else if let Some(backend) = profile.backend {
        target.backend = backend;
    }
    let ws = Workspace::open_target(target)?;
    let monitors = ctx.ipc()?.monitors()?;
    let layout = Layout::new(&monitors, &profile.monitors, false);
    check_layout(&layout)?;
    let expected = layout
        .expectations()
        .into_iter()
        .zip(&layout.outputs)
        .filter(|(_, o)| matches!(o.origin, Origin::Block(_)))
        .map(|(e, _)| e)
        .collect();
    let content = ws
        .target
        .backend
        .save(ws.snapshot.text(), &profile.monitors, &ctx.save_options())?
        .content;
    write_and_verify(ctx, &ws, &content, expected, config)
}

/// Reload the file as it is, verify that Hyprland shows the block, and go
/// back to the previous live layout unless confirmed.
fn apply_file(ctx: &Context, config: ApplyConfig) -> Result<Written, AppError> {
    let ws = Workspace::open(ctx)?;
    ctx.check_backend(&ws.target)?;
    let monitors = ctx.ipc()?.monitors()?;
    let layout = Layout::new(&monitors, ws.block_rules(), false);
    let expected = layout
        .expectations()
        .into_iter()
        .zip(&layout.outputs)
        .filter(|(_, o)| matches!(o.origin, Origin::Block(_)))
        .map(|(e, _)| e)
        .collect();
    // The layout before the reload, as complete rules.
    let current = Layout::new(&monitors, &[], false);
    let previous: Vec<_> = (0..current.outputs.len())
        .map(|i| current.live_rule(i))
        .collect();
    let restore = Action::set(running_backend(ctx, ws.target.backend), &previous)?;
    live_change(ctx, Action::Reload, restore, expected, config)
}
