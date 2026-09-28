// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! `hyprtilt doctor`: gather the facts and print the checks.

use std::os::unix::fs::MetadataExt;

use hyprtilt_core::doctor::{self, Check, Facts, Live, Status};
use hyprtilt_core::fsio;
use hyprtilt_core::ipc::HyprlandIpc;
use serde::Serialize;

use super::print_json;
use crate::context::Context;
use crate::error::AppError;

#[derive(Serialize)]
struct Report<'a> {
    checks: &'a [Check],
    problems: usize,
}

/// Run `doctor`.
pub(crate) fn run(ctx: &Context) -> Result<(), AppError> {
    let checks = doctor::diagnose(&facts(ctx));
    let problems = checks.iter().filter(|c| c.status.is_problem()).count();
    if ctx.global.json {
        print_json(&Report {
            checks: &checks,
            problems,
        })?;
    } else {
        print!("{}", render(&checks));
    }
    if problems > 0 {
        return Err(AppError::DoctorProblems);
    }
    Ok(())
}

fn live(ipc: &dyn HyprlandIpc) -> Result<Live, String> {
    let version = ipc.version().map_err(|e| e.to_string())?.version;
    Ok(Live {
        provider: ipc
            .status()
            .map_err(|e| e.to_string())?
            .map(|s| s.config_provider),
        monitors: ipc.monitors().map_err(|e| e.to_string())?,
        config_errors: ipc.config_errors().map_err(|e| e.to_string())?,
        version,
    })
}

fn facts(ctx: &Context) -> Facts {
    let target = ctx.target();
    let snapshot = fsio::snapshot(&target.path);
    let (target_content, real) = match snapshot {
        Ok(s) => (Ok(s.content.clone()), Some(s.real)),
        Err(e) => (Err(e.to_string()), None),
    };
    let target_inode = real
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.ino());
    let read_only = real.as_ref().and_then(|p| {
        if p.starts_with("/nix/store") {
            Some("it is in the Nix store; change your Nix configuration instead".to_owned())
        } else if std::fs::metadata(p).is_ok_and(|m| m.permissions().readonly()) {
            Some("the file is read-only".to_owned())
        } else {
            None
        }
    });
    Facts {
        hyprland: ctx
            .ipc()
            .map_err(|e| e.message().unwrap_or_default().to_owned())
            .and_then(live),
        main_config: ctx.main_config(),
        target,
        target_content,
        target_inode,
        read_only,
        process: ctx.process(),
        luac: Context::luac(),
    }
}

fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let tag = match c.status {
            Status::Ok => "[ ok ]",
            Status::Info => "[info]",
            Status::Warn => "[warn]",
            Status::Fail => "[FAIL]",
        };
        out.push_str(&format!("{tag} {}\n", c.title));
        if let Some(detail) = &c.detail {
            for line in detail.lines() {
                out.push_str(&format!("       {line}\n"));
            }
        }
    }
    let problems = checks.iter().filter(|c| c.status.is_problem()).count();
    out.push_str(&match problems {
        0 => "\nNo problems found.\n".to_owned(),
        n => format!("\n{n} problem(s) found.\n"),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering() {
        let checks = [
            Check {
                id: "a",
                status: Status::Ok,
                title: "fine".to_owned(),
                detail: None,
            },
            Check {
                id: "b",
                status: Status::Warn,
                title: "hmm".to_owned(),
                detail: Some("one\ntwo".to_owned()),
            },
        ];
        assert_eq!(
            render(&checks),
            "[ ok ] fine\n[warn] hmm\n       one\n       two\n\n1 problem(s) found.\n"
        );
        assert!(render(&checks[..1]).ends_with("No problems found.\n"));
    }
}
