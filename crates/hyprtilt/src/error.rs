// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Errors of the binary, each with its documented exit code.

use hyprtilt_core::document::ConfigError;
use hyprtilt_core::fsio::WriteError;
use hyprtilt_core::ipc::IpcError;
use hyprtilt_core::layout::LayoutError;
use hyprtilt_core::profile::ProfileError;
use hyprtilt_core::settings::SettingsError;

use crate::exit::Exit;

/// Why a command failed.
#[derive(Debug)]
pub(crate) enum AppError {
    /// Hyprland is not running or IPC failed.
    Ipc(String),
    /// The configuration file cannot be read, parsed or written as asked.
    Config(String),
    /// The requested output does not exist.
    NoSuchOutput(String),
    /// The change was rolled back.
    RolledBack(String),
    /// `doctor` found problems (already printed).
    DoctorProblems,
    /// Anything else.
    Other(String),
}

impl AppError {
    /// The exit code.
    pub(crate) fn exit(&self) -> Exit {
        match self {
            AppError::Ipc(_) => Exit::Ipc,
            AppError::Config(_) => Exit::Config,
            AppError::NoSuchOutput(_) => Exit::NoSuchOutput,
            AppError::RolledBack(_) => Exit::RolledBack,
            AppError::DoctorProblems => Exit::DoctorProblems,
            AppError::Other(_) => Exit::Error,
        }
    }

    /// The message for standard error, if any.
    pub(crate) fn message(&self) -> Option<&str> {
        match self {
            AppError::Ipc(m)
            | AppError::Config(m)
            | AppError::NoSuchOutput(m)
            | AppError::RolledBack(m)
            | AppError::Other(m) => Some(m),
            AppError::DoctorProblems => None,
        }
    }
}

impl From<IpcError> for AppError {
    fn from(e: IpcError) -> Self {
        AppError::Ipc(e.to_string())
    }
}

impl From<ConfigError> for AppError {
    fn from(e: ConfigError) -> Self {
        AppError::Config(e.to_string())
    }
}

impl From<WriteError> for AppError {
    fn from(e: WriteError) -> Self {
        AppError::Config(e.to_string())
    }
}

impl From<SettingsError> for AppError {
    fn from(e: SettingsError) -> Self {
        AppError::Config(e.to_string())
    }
}

impl From<ProfileError> for AppError {
    fn from(e: ProfileError) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<LayoutError> for AppError {
    fn from(e: LayoutError) -> Self {
        match e {
            LayoutError::NoSuchOutput(_) => AppError::NoSuchOutput(e.to_string()),
            LayoutError::NoRates(_) => AppError::Other(e.to_string()),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes() {
        let cases = [
            (AppError::Ipc("x".into()), Exit::Ipc),
            (AppError::Config("x".into()), Exit::Config),
            (AppError::NoSuchOutput("x".into()), Exit::NoSuchOutput),
            (AppError::RolledBack("x".into()), Exit::RolledBack),
            (AppError::DoctorProblems, Exit::DoctorProblems),
            (AppError::Other("x".into()), Exit::Error),
        ];
        for (e, exit) in cases {
            assert_eq!(e.exit(), exit);
            assert_eq!(e.message().is_some(), exit != Exit::DoctorProblems);
        }
        let e: AppError = LayoutError::NoSuchOutput("DP-9".into()).into();
        assert_eq!(e.exit(), Exit::NoSuchOutput);
        let e: AppError = LayoutError::NoRates("DP-1".into()).into();
        assert_eq!(e.exit(), Exit::Error);
        let e: AppError = ConfigError::NoBlock.into();
        assert_eq!(e.message(), Some("the file has no managed block"));
        let e: AppError = IpcError::NotRunning("gone".into()).into();
        assert_eq!(e.exit(), Exit::Ipc);
        let e: AppError = std::io::Error::other("disk").into();
        assert_eq!(e.exit(), Exit::Error);
    }
}
