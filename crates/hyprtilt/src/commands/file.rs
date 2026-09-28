// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `save`, `adopt` and `unmanage`: commands that change the file without
//! changing the layout.

use std::time::SystemTime;

use hyprtilt_core::apply::ApplyConfig;
use hyprtilt_core::document::{Backend, Edit};
use hyprtilt_core::fsio;
use hyprtilt_core::layout::Origin;
use serde::Serialize;

use super::{Workspace, Written, check_layout, layout, print_json, write_and_verify};
use crate::context::Context;
use crate::error::AppError;

#[derive(Serialize)]
struct Report {
    command: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lines: Vec<usize>,
    dry_run: bool,
    #[serde(flatten)]
    written: Written,
}

fn report(
    ctx: &Context,
    command: &'static str,
    lines: Vec<usize>,
    written: Written,
    human: &str,
) -> Result<(), AppError> {
    if ctx.global.json {
        return print_json(&Report {
            command,
            lines,
            dry_run: ctx.global.dry_run,
            written,
        });
    }
    if !ctx.global.dry_run {
        println!("{human}");
    }
    Ok(())
}

/// `save`: write the running layout into the block.
pub(crate) fn save(ctx: &Context) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let (mut layout, _) = layout(ctx, &ws)?;
    layout.take_live_state();
    check_layout(&layout)?;
    let content = ws
        .target
        .backend
        .save(ws.snapshot.text(), &layout.rules(), &ctx.save_options())?
        .content;
    // The file now says what Hyprland shows, so nothing needs confirming.
    let written = write_and_verify(
        ctx,
        &ws,
        &content,
        layout.expectations(),
        ApplyConfig::with_confirm_seconds(0),
    )?;
    let new = layout
        .outputs
        .iter()
        .filter(|o| o.origin == Origin::New)
        .count();
    let human = if written.changed {
        format!(
            "wrote the running layout of {} output(s) into {}{}",
            layout.outputs.len(),
            ws.display(),
            if new > 0 {
                format!(" ({new} new rule(s))")
            } else {
                String::new()
            }
        )
    } else {
        format!("{} already describes the running layout", ws.display())
    };
    report(ctx, "save", Vec::new(), written, &human)
}

/// Write a file edit that does not change the layout (`adopt`,
/// `unmanage`): back up, write, and reload so that Hyprland watches the
/// file as it is.
fn write_plain(ctx: &Context, ws: &Workspace, edit: &Edit) -> Result<Written, AppError> {
    let mut written = Written {
        file: Some(ws.target.path.clone()),
        changed: edit.changed,
        ..Written::default()
    };
    if !edit.changed {
        return Ok(written);
    }
    if ctx.global.dry_run {
        if !ctx.global.json {
            print!(
                "{}",
                fsio::diff(ws.snapshot.text(), &edit.content, &ws.display())
            );
        }
        return Ok(written);
    }
    if ws.target.backend == Backend::Lua
        && let Some(luac) = Context::luac()
    {
        luac.check_regression(ws.snapshot.text(), &edit.content)
            .map_err(|e| {
                AppError::Config(format!(
                    "the new content of {} does not compile: {e}",
                    ws.display()
                ))
            })?;
    }
    let report = fsio::replace(
        &ws.snapshot,
        &edit.content,
        Some(&ctx.settings.backup_policy()),
        SystemTime::now(),
    )?;
    written.backup = report.backup;
    if let Ok(ipc) = ctx.ipc() {
        // The layout is the same; a failed reload is not worth failing for.
        let _ = ipc.reload();
        written.requests.push("/reload".to_owned());
    }
    Ok(written)
}

/// `adopt`: move rules from outside the block into it.
pub(crate) fn adopt(ctx: &Context, lines: &[usize]) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let moved: Vec<usize> = if lines.is_empty() {
        ws.doc
            .outside
            .iter()
            .filter(|f| f.is_adoptable())
            .map(|f| f.line)
            .collect()
    } else {
        lines.to_vec()
    };
    let edit = ws.target.backend.adopt(ws.snapshot.text(), lines)?;
    let written = write_plain(ctx, &ws, &edit)?;
    let human = if edit.changed {
        let list: Vec<String> = moved.iter().map(ToString::to_string).collect();
        format!(
            "adopted {} rule(s) from line {} into the block of {}",
            moved.len(),
            list.join(", "),
            ws.display()
        )
    } else {
        "no monitor rules to adopt".to_owned()
    };
    report(ctx, "adopt", moved, written, &human)
}

/// `unmanage`: remove the block markers.
pub(crate) fn unmanage(ctx: &Context) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let edit = ws.target.backend.unmanage(ws.snapshot.text())?;
    let written = write_plain(ctx, &ws, &edit)?;
    let human = format!(
        "removed the managed block markers from {}; the rules stay as ordinary configuration",
        ws.display()
    );
    report(ctx, "unmanage", Vec::new(), written, &human)
}
