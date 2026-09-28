// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `profile list|save|apply|delete`.

use std::time::SystemTime;

use hyprtilt_core::profile::Profile;
use serde::Serialize;

use super::{Workspace, layout, print_json};
use crate::context::Context;
use crate::error::AppError;

/// `profile list`.
pub(crate) fn list(ctx: &Context) -> Result<(), AppError> {
    let store = ctx.profiles();
    let names = store.list()?;
    if ctx.global.json {
        return print_json(&names);
    }
    if names.is_empty() {
        println!("no profiles in {}", store.dir().display());
    }
    for name in names {
        println!("{name}");
    }
    Ok(())
}

#[derive(Serialize)]
struct Saved {
    profile: String,
    file: std::path::PathBuf,
    monitors: usize,
}

/// `profile save`: the running layout, with the other fields of the
/// block's rules.
pub(crate) fn save(ctx: &Context, name: &str, descriptions: bool) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let (mut layout, _) = layout(ctx, &ws)?;
    layout.take_live_state();
    if descriptions {
        layout.use_descriptions();
    }
    let profile = Profile {
        backend: Some(ws.target.backend),
        target: None,
        monitors: layout.outputs.iter().map(|o| o.rule.clone()).collect(),
    };
    let store = ctx.profiles();
    if ctx.global.dry_run {
        print!("{}", profile.to_toml(name));
        return Ok(());
    }
    let file = store.save(name, &profile, SystemTime::now())?;
    if ctx.global.json {
        return print_json(&Saved {
            profile: name.to_owned(),
            file,
            monitors: profile.monitors.len(),
        });
    }
    println!(
        "saved {} monitor(s) as profile {name:?} in {}",
        profile.monitors.len(),
        file.display()
    );
    Ok(())
}

/// `profile delete`.
pub(crate) fn delete(ctx: &Context, name: &str) -> Result<(), AppError> {
    let store = ctx.profiles();
    if !ctx.global.dry_run {
        store.delete(name)?;
    }
    if !ctx.global.json {
        println!("deleted profile {name:?}");
    }
    Ok(())
}
