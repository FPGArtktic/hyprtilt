// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! hyprtilt: TUI and CLI for Hyprland monitor layout.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod context;
mod error;
mod exit;
mod run;

use std::io::Write;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use crate::cli::{Cli, Command, ProfileCommand};
use crate::commands::edit::{self, Change};
use crate::commands::{apply, doctor, file, list, profile};
use crate::context::Context;
use crate::error::AppError;
use crate::exit::Exit;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(()) => Exit::Ok.into(),
        Err(e) => {
            if let Some(message) = e.message() {
                eprintln!("hyprtilt: {message}");
            }
            e.exit().into()
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), AppError> {
    match &cli.command {
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                *shell,
                &mut Cli::command(),
                "hyprtilt",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
        Some(Command::Man) => return print_man(),
        _ => {}
    }
    let ctx = Context::new(cli.global)?;
    let Some(command) = cli.command else {
        return Err(AppError::Other(
            "the terminal interface is not implemented yet".to_owned(),
        ));
    };
    match command {
        Command::List => list::run(&ctx),
        Command::Rotate { change, angle } => {
            edit::run(&ctx, &change.output, change.live, &Change::Rotate(angle))
        }
        Command::Move { change, position } => {
            let (x, y) = edit::parse_position(&position)?;
            edit::run(&ctx, &change.output, change.live, &Change::Move(x, y))
        }
        Command::Scale { change, factor } => {
            let scale = edit::parse_scale(&factor)?;
            edit::run(&ctx, &change.output, change.live, &Change::Scale(scale))
        }
        Command::Mode { change, mode } => {
            let mode = edit::parse_mode(&mode)?;
            edit::run(&ctx, &change.output, change.live, &Change::Mode(mode))
        }
        Command::Refresh { change, rate } => {
            let rate = edit::parse_refresh(&rate)?;
            edit::run(&ctx, &change.output, change.live, &Change::Refresh(rate))
        }
        Command::Enable { change } => {
            edit::run(&ctx, &change.output, change.live, &Change::Enable(true))
        }
        Command::Disable { change } => {
            edit::run(&ctx, &change.output, change.live, &Change::Enable(false))
        }
        Command::Apply {
            profile,
            confirm_timeout,
            no_confirm,
        } => apply::run(&ctx, profile.as_deref(), confirm_timeout, no_confirm),
        Command::Save => file::save(&ctx),
        Command::Adopt { line } => file::adopt(&ctx, &line),
        Command::Unmanage => file::unmanage(&ctx),
        Command::Profile(ProfileCommand::List) => profile::list(&ctx),
        Command::Profile(ProfileCommand::Save { name, desc }) => profile::save(&ctx, &name, desc),
        Command::Profile(ProfileCommand::Apply { name }) => {
            apply::run(&ctx, Some(&name), None, false)
        }
        Command::Profile(ProfileCommand::Delete { name }) => profile::delete(&ctx, &name),
        Command::Doctor => doctor::run(&ctx),
        Command::Completions { .. } | Command::Man => Ok(()),
    }
}

/// Print the man page generated from the command line definition.
fn print_man() -> Result<(), AppError> {
    let man = clap_mangen::Man::new(Cli::command());
    let mut out = Vec::new();
    man.render(&mut out)
        .map_err(|e| AppError::Other(format!("cannot render the man page: {e}")))?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}
