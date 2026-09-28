// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Process exit codes (ADR 0001, D13). They are part of the interface:
//! scripts and keybindings rely on them, so a code never changes meaning.

use std::process::ExitCode;

/// Why hyprtilt exits.
// Most codes are returned by commands that are not implemented yet.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    /// Success.
    Ok = 0,
    /// Any error without a more specific code.
    Error = 1,
    /// Invalid command line (clap uses 2 as well).
    Usage = 2,
    /// Hyprland is not running, or IPC failed.
    Ipc = 3,
    /// The configuration file cannot be parsed or is refused.
    Config = 4,
    /// The requested output does not exist.
    NoSuchOutput = 5,
    /// The change was rolled back (not confirmed, or verification failed).
    RolledBack = 6,
    /// `doctor` found problems.
    DoctorProblems = 7,
}

impl From<Exit> for ExitCode {
    fn from(e: Exit) -> ExitCode {
        ExitCode::from(e as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let codes = [
            (Exit::Ok, 0),
            (Exit::Error, 1),
            (Exit::Usage, 2),
            (Exit::Ipc, 3),
            (Exit::Config, 4),
            (Exit::NoSuchOutput, 5),
            (Exit::RolledBack, 6),
            (Exit::DoctorProblems, 7),
        ];
        for (exit, code) in codes {
            assert_eq!(exit as u8, code);
            assert_eq!(ExitCode::from(exit), ExitCode::from(code));
        }
    }
}
