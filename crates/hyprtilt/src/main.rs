// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! hyprtilt: TUI and CLI for Hyprland monitor layout.

#![forbid(unsafe_code)]

mod cli;
mod exit;

use std::io::Write;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use crate::cli::{Cli, Command};
use crate::exit::Exit;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "hyprtilt",
                &mut std::io::stdout(),
            );
            Exit::Ok.into()
        }
        Some(Command::Man) => print_man(),
        other => {
            let what =
                other.map_or_else(|| "the terminal interface".to_owned(), |c| format!("{c:?}"));
            eprintln!("hyprtilt: not implemented yet: {what}");
            Exit::Error.into()
        }
    }
}

/// Print the man page generated from the command line definition.
fn print_man() -> ExitCode {
    let man = clap_mangen::Man::new(Cli::command());
    let mut out = Vec::new();
    if let Err(e) = man.render(&mut out) {
        eprintln!("hyprtilt: cannot render the man page: {e}");
        return Exit::Error.into();
    }
    if std::io::stdout().write_all(&out).is_err() {
        return Exit::Error.into();
    }
    Exit::Ok.into()
}
