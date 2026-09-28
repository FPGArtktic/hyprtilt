// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Command line definition.
//!
//! The same definition generates the man page and the shell completions,
//! so help texts here are user documentation.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Arrange Hyprland monitors from the terminal, editing your own Lua or
/// hyprlang configuration in place.
///
/// Without a subcommand, hyprtilt opens the terminal user interface.
#[derive(Debug, Parser)]
#[command(name = "hyprtilt", version, about, long_about)]
pub(crate) struct Cli {
    /// Options shared by all commands.
    #[command(flatten)]
    pub(crate) global: GlobalArgs,

    /// What to do; the terminal interface when omitted.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Options shared by all commands.
#[derive(Debug, Args)]
pub(crate) struct GlobalArgs {
    /// Configuration file to edit, instead of the one hyprtilt finds.
    #[arg(long, short = 'f', global = true, value_name = "PATH")]
    pub(crate) file: Option<PathBuf>,

    /// Configuration language of the file, instead of guessing it from the
    /// file name.
    #[arg(long, global = true, value_enum)]
    pub(crate) backend: Option<BackendArg>,

    /// Show what would change (file diff and IPC requests) without
    /// changing anything.
    #[arg(long, short = 'n', global = true)]
    pub(crate) dry_run: bool,

    /// Print results as JSON.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Use an in-memory Hyprland described by a JSON file instead of the
    /// running compositor (for demonstrations and tests).
    #[arg(long, global = true, hide = true, value_name = "SETUP.json")]
    pub(crate) fake_hyprland: Option<PathBuf>,
}

/// Configuration language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BackendArg {
    /// Lua (`hl.monitor`), Hyprland 0.55 and later.
    Lua,
    /// hyprlang (`monitor=`).
    Hyprlang,
}

/// Rotation step for `rotate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Angle {
    /// 90° clockwise.
    #[value(name = "90")]
    Cw90,
    /// 180°.
    #[value(name = "180")]
    Half,
    /// 270° clockwise, the same as 90° counter-clockwise.
    #[value(name = "270")]
    Cw270,
    /// 90° counter-clockwise.
    #[value(name = "-90")]
    Ccw90,
}

/// Options of the per-output commands.
#[derive(Debug, Args)]
pub(crate) struct OutputChange {
    /// Output to change: a connector name such as `DP-1`, or the selector
    /// used in the configuration (`desc:...`).
    pub(crate) output: String,

    /// Change only the running session, not the configuration file. The
    /// change is lost on the next reload.
    #[arg(long)]
    pub(crate) live: bool,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show the monitors: live state from Hyprland and the rules in the
    /// configuration file.
    List,

    /// Rotate an output by a step of 90°.
    Rotate {
        #[command(flatten)]
        change: OutputChange,
        /// Rotation: 90, 180, 270 or -90.
        #[arg(value_enum, allow_hyphen_values = true)]
        angle: Angle,
    },

    /// Move an output to a position in logical pixels.
    Move {
        #[command(flatten)]
        change: OutputChange,
        /// New top-left corner, `XxY` (for example `1440x1335`).
        position: String,
    },

    /// Set the scale of an output.
    Scale {
        #[command(flatten)]
        change: OutputChange,
        /// Scale factor (snapped the way Hyprland does), or `auto`.
        factor: String,
    },

    /// Set the mode of an output.
    Mode {
        #[command(flatten)]
        change: OutputChange,
        /// `WxH@Hz`, `preferred`, `highres` or `highrr`.
        mode: String,
    },

    /// Set the refresh rate of an output, keeping its resolution.
    Refresh {
        #[command(flatten)]
        change: OutputChange,
        /// Refresh rate in Hz (the nearest rate the monitor offers is
        /// used), `max`, `min`, or `up`/`down` for the next rate.
        rate: String,
    },

    /// Enable an output.
    Enable {
        #[command(flatten)]
        change: OutputChange,
    },

    /// Disable an output.
    Disable {
        #[command(flatten)]
        change: OutputChange,
    },

    /// Apply the configuration file (or a profile) to the running session,
    /// verify it and keep it only when confirmed.
    Apply {
        /// Apply this profile instead of the configuration file.
        #[arg(long, short = 'p', value_name = "NAME")]
        profile: Option<String>,
        /// Seconds to wait for confirmation before rolling back; 0 keeps
        /// the change without asking.
        #[arg(long, value_name = "SECONDS")]
        confirm_timeout: Option<u64>,
        /// Keep the change without asking (verification still runs).
        #[arg(long, conflicts_with = "confirm_timeout")]
        no_confirm: bool,
    },

    /// Write the running layout into the managed block.
    Save,

    /// Move monitor rules outside the managed block into it.
    Adopt {
        /// Adopt only the rule starting on this line (repeatable).
        #[arg(long, value_name = "LINE")]
        line: Vec<usize>,
    },

    /// Remove the block markers and leave the rules as ordinary
    /// configuration.
    Unmanage,

    /// Manage saved layouts.
    #[command(subcommand)]
    Profile(ProfileCommand),

    /// Check the setup: Hyprland, the target file, whether Hyprland loads
    /// it, and conflicting rules.
    Doctor,

    /// Print shell completions.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Print the man page (roff).
    #[command(hide = true)]
    Man,
}

/// `profile` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    /// List saved profiles.
    List,
    /// Save the current layout as a profile.
    Save {
        /// Profile name.
        name: String,
        /// Select monitors by description (make, model, serial) instead
        /// of connector name, so the profile survives connector renames.
        #[arg(long)]
        desc: bool,
    },
    /// Apply a profile (same as `apply --profile`).
    Apply {
        /// Profile name.
        name: String,
    },
    /// Delete a profile.
    Delete {
        /// Profile name.
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn negative_angle_parses() {
        let cli = Cli::try_parse_from(["hyprtilt", "rotate", "HDMI-A-1", "-90"]).unwrap();
        match cli.command {
            Some(Command::Rotate { change, angle }) => {
                assert_eq!(change.output, "HDMI-A-1");
                assert_eq!(angle, Angle::Ccw90);
                assert!(!change.live);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn global_options_after_subcommand() {
        let cli = Cli::try_parse_from([
            "hyprtilt",
            "move",
            "DP-1",
            "3360x975",
            "--dry-run",
            "--file",
            "/x.lua",
            "--live",
        ])
        .unwrap();
        assert!(cli.global.dry_run);
        assert_eq!(
            cli.global.file.as_deref(),
            Some(std::path::Path::new("/x.lua"))
        );
        assert!(matches!(cli.command, Some(Command::Move { ref change, .. }) if change.live));
    }

    #[test]
    fn confirm_options_conflict() {
        assert!(
            Cli::try_parse_from([
                "hyprtilt",
                "apply",
                "--no-confirm",
                "--confirm-timeout",
                "5"
            ])
            .is_err()
        );
    }

    #[test]
    fn refresh_and_global_json() {
        let cli = Cli::try_parse_from(["hyprtilt", "refresh", "DP-1", "max", "--json"]).unwrap();
        assert!(cli.global.json);
        assert!(matches!(cli.command, Some(Command::Refresh { ref rate, .. }) if rate == "max"));
    }

    #[test]
    fn no_subcommand_means_tui() {
        let cli = Cli::try_parse_from(["hyprtilt"]).unwrap();
        assert!(cli.command.is_none());
    }
}
