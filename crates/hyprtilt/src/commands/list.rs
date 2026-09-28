// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `hyprtilt list`: the live state and what the file says.

use std::path::PathBuf;

use hyprtilt_core::layout::{Layout, Mismatch, Origin, compare};
use hyprtilt_core::model::{MonitorRule, format_refresh};
use serde::Serialize;

use super::{Workspace, print_json, reason};
use crate::context::Context;
use crate::error::AppError;

#[derive(Serialize)]
struct Report {
    target: TargetInfo,
    hyprland: Option<HyprlandInfo>,
    block: Option<BlockInfo>,
    outputs: Vec<OutputInfo>,
    detached_rules: Vec<MonitorRule>,
    outside_rules: Vec<OutsideRule>,
}

#[derive(Serialize)]
struct TargetInfo {
    path: PathBuf,
    backend: String,
    reason: &'static str,
}

#[derive(Serialize)]
struct HyprlandInfo {
    version: String,
    provider: Option<String>,
}

#[derive(Serialize)]
struct BlockInfo {
    begin_line: usize,
    end_line: usize,
}

#[derive(Serialize)]
struct OutputInfo {
    name: String,
    description: String,
    enabled: bool,
    mode: String,
    refresh_rate: f64,
    position: [i32; 2],
    scale: f64,
    transform: u8,
    rule: Option<MonitorRule>,
    rule_line: Option<usize>,
    differences: Vec<Mismatch>,
}

#[derive(Serialize)]
struct OutsideRule {
    line: usize,
    rule: Option<MonitorRule>,
    adoptable: bool,
    reason: Option<String>,
}

/// Run `list`.
pub(crate) fn run(ctx: &Context) -> Result<(), AppError> {
    let ws = Workspace::open(ctx)?;
    let ipc = ctx.ipc();
    let monitors = match &ipc {
        Ok(ipc) => Some(ipc.monitors()?),
        Err(_) => None,
    };
    let hyprland = ipc
        .ok()
        .and_then(|ipc| ipc.version().ok())
        .map(|v| HyprlandInfo {
            version: v.version,
            provider: ctx.status().map(|s| s.config_provider),
        });
    let layout = match &monitors {
        Some(m) => Layout::new(m, ws.block_rules(), false),
        None => Layout::offline(ws.block_rules()),
    };
    let expectations = layout.expectations();
    let lines = ws
        .doc
        .block
        .as_ref()
        .map(|b| b.rule_lines.clone())
        .unwrap_or_default();
    let outputs = layout
        .outputs
        .iter()
        .zip(&expectations)
        .map(|(o, e)| {
            let block_index = match o.origin {
                Origin::Block(i) => Some(i),
                Origin::New => None,
            };
            let differences = match (&monitors, block_index) {
                (Some(m), Some(_)) => compare(std::slice::from_ref(e), m),
                _ => Vec::new(),
            };
            OutputInfo {
                name: o.info.name.clone(),
                description: o.info.description.clone(),
                enabled: !o.info.disabled,
                mode: format!("{}x{}", o.info.width, o.info.height),
                refresh_rate: o.info.refresh_rate,
                position: [o.info.x, o.info.y],
                scale: o.info.scale,
                transform: o.info.transform,
                rule: block_index.map(|_| o.rule.clone()),
                rule_line: block_index.and_then(|i| lines.get(i).copied()),
                differences,
            }
        })
        .collect();
    let report = Report {
        target: TargetInfo {
            path: ws.target.path.clone(),
            backend: ws.target.backend.to_string(),
            reason: reason(&ws.target),
        },
        hyprland,
        block: ws.doc.block.as_ref().map(|b| BlockInfo {
            begin_line: b.begin_line,
            end_line: b.end_line,
        }),
        outputs,
        detached_rules: layout.detached.iter().map(|(_, r)| r.clone()).collect(),
        outside_rules: ws
            .doc
            .outside
            .iter()
            .map(|f| OutsideRule {
                line: f.line,
                rule: f.rule.clone(),
                adoptable: f.is_adoptable(),
                reason: f.not_adoptable.clone(),
            })
            .collect(),
    };
    if ctx.global.json {
        return print_json(&report);
    }
    print!("{}", render(&report, monitors.is_some()));
    Ok(())
}

fn render(r: &Report, connected: bool) -> String {
    let mut out = format!(
        "File:  {} ({}, {})\n",
        r.target.path.display(),
        r.target.backend,
        r.target.reason
    );
    match &r.block {
        Some(b) => out.push_str(&format!("Block: lines {}-{}\n", b.begin_line, b.end_line)),
        None => out.push_str("Block: none yet (`hyprtilt save` or `hyprtilt adopt` creates it)\n"),
    }
    match &r.hyprland {
        Some(h) => out.push_str(&format!(
            "Hyprland {} ({})\n",
            h.version,
            h.provider.as_deref().unwrap_or("hyprlang")
        )),
        None if !connected => {
            out.push_str("Hyprland is not reachable; the outputs below come from the block\n");
        }
        None => {}
    }
    out.push('\n');
    let header = [
        "OUTPUT",
        "STATE",
        "MODE",
        "POSITION",
        "SCALE",
        "TRANSFORM",
        "RULE",
    ];
    let mut rows: Vec<[String; 7]> = vec![header.map(str::to_owned)];
    for o in &r.outputs {
        let rule = match (&o.rule, o.rule_line) {
            (Some(rule), line) => {
                let mut text =
                    line.map_or_else(|| "block".to_owned(), |l| format!("block, line {l}"));
                if let Some(v) = rule.vrr {
                    text.push_str(&format!(", vrr {v}"));
                }
                if !o.differences.is_empty() {
                    text.push_str(", differs");
                }
                text
            }
            (None, _) => "none".to_owned(),
        };
        rows.push([
            o.name.clone(),
            if o.enabled { "on" } else { "off" }.to_owned(),
            format!("{}@{}", o.mode, format_refresh(o.refresh_rate)),
            format!("{}x{}", o.position[0], o.position[1]),
            hyprtilt_core::geometry::format_scale(o.scale),
            transform_name(o.transform),
            rule,
        ]);
    }
    let widths: Vec<usize> = (0..7)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    for row in &rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| format!("{cell:<width$}", width = widths[c]))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    for o in r.outputs.iter().filter(|o| !o.differences.is_empty()) {
        for d in &o.differences {
            out.push_str(&format!("  ! {d} (the file says otherwise)\n"));
        }
    }
    for rule in &r.detached_rules {
        out.push_str(&format!(
            "  rule for {:?} (not connected)\n",
            rule.output.as_str()
        ));
    }
    let adoptable: Vec<String> = r
        .outside_rules
        .iter()
        .filter(|f| f.adoptable)
        .map(|f| f.line.to_string())
        .collect();
    if !adoptable.is_empty() {
        out.push_str(&format!(
            "\n{} rule(s) outside the block (line {}); `hyprtilt adopt` moves them in\n",
            adoptable.len(),
            adoptable.join(", ")
        ));
    }
    out
}

/// A transform as rotation and flip, such as `90°` or `flipped 180°`.
pub(crate) fn transform_name(t: u8) -> String {
    let degrees = u16::from(t & 3) * 90;
    if t & 4 == 0 {
        format!("{degrees}°")
    } else {
        format!("flipped {degrees}°")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms() {
        assert_eq!(transform_name(0), "0°");
        assert_eq!(transform_name(1), "90°");
        assert_eq!(transform_name(6), "flipped 180°");
    }
}
