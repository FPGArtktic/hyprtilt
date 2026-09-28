// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Talking to Hyprland.
//!
//! [`HyprlandIpc`] has one required method: send a raw request and return
//! the raw reply. Typed helpers build on it, so the socket implementation
//! and the test double only have to move bytes. See `docs/hyprland-lua-api.md`
//! section 3 for the wire format.

pub mod events;
pub mod socket;

use serde::{Deserialize, Serialize};

/// Why a request failed.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// No Hyprland instance to talk to (environment variables missing, or
    /// the socket does not exist).
    #[error("Hyprland is not running or its socket was not found: {0}")]
    NotRunning(String),
    /// Reading or writing the socket failed.
    #[error("IPC I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The reply could not be parsed.
    #[error("unexpected reply to {request:?}: {message}")]
    BadReply {
        /// The request that was sent.
        request: String,
        /// What was wrong with the reply.
        message: String,
    },
    /// Hyprland answered with an error message.
    #[error("Hyprland refused {request:?}: {reply}")]
    Refused {
        /// The request that was sent.
        request: String,
        /// Hyprland's reply.
        reply: String,
    },
}

/// A connection to Hyprland's request socket.
///
/// Every request carries a flag prefix (`j/monitors all`, `/eval ...`), so
/// that a `/` inside Lua code is never taken for the flag separator.
pub trait HyprlandIpc {
    /// Send one raw request, including its flag prefix (for example
    /// `j/monitors all` or `/reload`), and return the complete reply.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] when Hyprland cannot be reached.
    fn request(&self, request: &str) -> Result<String, IpcError>;

    /// Every monitor, disabled and mirrored ones included (`monitors all`).
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] when Hyprland cannot be reached or the reply is
    /// not the expected JSON.
    fn monitors(&self) -> Result<Vec<MonitorInfo>, IpcError> {
        let reply = self.request("j/monitors all")?;
        if reply.trim() == UNKNOWN_REQUEST {
            // Hyprland answers this way when there is nothing to list.
            return Ok(Vec::new());
        }
        json("j/monitors all", &reply)
    }

    /// The compositor's version.
    ///
    /// # Errors
    ///
    /// As [`HyprlandIpc::monitors`].
    fn version(&self) -> Result<VersionInfo, IpcError> {
        json("j/version", &self.request("j/version")?)
    }

    /// The configuration provider (`lua` or `hyprlang`); `None` before
    /// Hyprland 0.55, which has no `status` request and only hyprlang.
    ///
    /// # Errors
    ///
    /// As [`HyprlandIpc::monitors`].
    fn status(&self) -> Result<Option<StatusInfo>, IpcError> {
        let reply = self.request("j/status")?;
        if reply.trim() == UNKNOWN_REQUEST {
            return Ok(None);
        }
        json("j/status", &reply).map(Some)
    }

    /// The configuration errors of the last load, empty when there are
    /// none. Reliable only right after a reload: `eval` clears the list.
    ///
    /// # Errors
    ///
    /// As [`HyprlandIpc::monitors`].
    fn config_errors(&self) -> Result<Vec<String>, IpcError> {
        let lines: Vec<String> = json("j/configerrors", &self.request("j/configerrors")?)?;
        Ok(lines.into_iter().filter(|l| !l.trim().is_empty()).collect())
    }

    /// Re-read the configuration files. Hyprland reloads synchronously and
    /// always answers `ok`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::Refused`] for any other reply.
    fn reload(&self) -> Result<(), IpcError> {
        expect_ok(self, "/reload")
    }

    /// Run Lua code in the configuration's Lua state (Hyprland 0.55 and
    /// later, Lua configuration only). The reply comes before the monitors
    /// change; poll [`HyprlandIpc::monitors`] to see the effect.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::Refused`] with Hyprland's message for anything
    /// but `ok`. Note that rules may still have been added in part.
    fn eval(&self, code: &str) -> Result<(), IpcError> {
        expect_ok(self, &format!("/eval {code}"))
    }

    /// Set a keyword in the running session (hyprlang configuration only),
    /// such as `keyword monitor DP-1,preferred,auto,1`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::Refused`] for anything but `ok`.
    fn keyword(&self, key: &str, value: &str) -> Result<(), IpcError> {
        expect_ok(self, &format!("/keyword {key} {value}"))
    }
}

/// Hyprland's reply to a request it does not know, or one with an empty
/// result.
pub const UNKNOWN_REQUEST: &str = "unknown request";

fn json<T: serde::de::DeserializeOwned>(request: &str, reply: &str) -> Result<T, IpcError> {
    serde_json::from_str(reply).map_err(|e| IpcError::BadReply {
        request: request.to_owned(),
        message: format!("{e}: {}", reply.chars().take(200).collect::<String>()),
    })
}

fn expect_ok<I: HyprlandIpc + ?Sized>(ipc: &I, request: &str) -> Result<(), IpcError> {
    let reply = ipc.request(request)?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(IpcError::Refused {
            request: request.chars().take(120).collect(),
            reply: reply.trim().to_owned(),
        })
    }
}

/// The active workspace of a monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkspaceRef {
    /// Workspace ID.
    #[serde(default)]
    pub id: i64,
    /// Workspace name.
    #[serde(default)]
    pub name: String,
}

/// One entry of `monitors all -j`. Unknown fields are ignored, because
/// newer Hyprland versions add fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    /// Monitor ID (reused per connector name).
    pub id: i64,
    /// Connector name, such as `DP-1`.
    pub name: String,
    /// `make model serial`, commas removed: what `desc:` matches.
    #[serde(default)]
    pub description: String,
    /// Manufacturer.
    #[serde(default)]
    pub make: String,
    /// Model.
    #[serde(default)]
    pub model: String,
    /// Serial number.
    #[serde(default)]
    pub serial: String,
    /// Width of the current mode in pixels, before the transform.
    pub width: u32,
    /// Height of the current mode in pixels, before the transform.
    pub height: u32,
    /// Physical width in millimetres.
    #[serde(default)]
    pub physical_width: u32,
    /// Physical height in millimetres.
    #[serde(default)]
    pub physical_height: u32,
    /// Current refresh rate in Hz.
    pub refresh_rate: f64,
    /// Left edge in logical layout coordinates.
    pub x: i32,
    /// Top edge in logical layout coordinates.
    pub y: i32,
    /// The workspace shown on this monitor.
    #[serde(default)]
    pub active_workspace: Option<WorkspaceRef>,
    /// Effective scale (after Hyprland's snapping).
    pub scale: f64,
    /// `wl_output_transform`, 0–7.
    pub transform: u8,
    /// Whether the monitor has focus.
    #[serde(default)]
    pub focused: bool,
    /// Whether adaptive sync is active right now (not the configured mode).
    #[serde(default)]
    pub vrr: bool,
    /// Whether the output is disabled.
    #[serde(default)]
    pub disabled: bool,
    /// Pixel format, such as `XRGB8888` or `XRGB2101010`.
    #[serde(default)]
    pub current_format: String,
    /// ID of the mirrored monitor as a string, or `none`.
    #[serde(default)]
    pub mirror_of: String,
    /// Available modes, formatted `WxH@R.RRHz`; may contain duplicates.
    #[serde(default)]
    pub available_modes: Vec<String>,
    /// Colour management preset in use.
    #[serde(default)]
    pub color_management_preset: String,
}

/// The reply of `version -j` (fields hyprtilt uses).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Version, such as `0.56.2`.
    pub version: String,
    /// Git tag, such as `v0.56.2`.
    #[serde(default)]
    pub tag: String,
    /// Git commit.
    #[serde(default)]
    pub commit: String,
    /// Git branch.
    #[serde(default)]
    pub branch: String,
}

/// The reply of `status -j` (Hyprland 0.55 and later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    /// `lua` or `hyprlang`.
    pub config_provider: String,
    /// `drm` or `wayland`.
    #[serde(default)]
    pub backend: String,
}

impl MonitorInfo {
    /// The connector name of the monitor this one mirrors, resolved from
    /// the numeric `mirrorOf` through the other entries.
    #[must_use]
    pub fn mirror_name<'a>(&self, all: &'a [MonitorInfo]) -> Option<&'a str> {
        let id: i64 = self.mirror_of.parse().ok()?;
        all.iter().find(|m| m.id == id).map(|m| m.name.as_str())
    }

    /// The current pixel size, before the transform.
    #[must_use]
    pub fn pixels(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The available modes without duplicates, in Hyprland's order.
    #[must_use]
    pub fn modes(&self) -> Vec<crate::model::Mode> {
        let mut out: Vec<crate::model::Mode> = Vec::new();
        for text in &self.available_modes {
            if let Ok(mode) = text.parse::<crate::model::Mode>()
                && !out.contains(&mode)
            {
                out.push(mode);
            }
        }
        out
    }
}

impl VersionInfo {
    /// The version as a number, if it parses.
    #[must_use]
    pub fn parsed(&self) -> Option<crate::version::Version> {
        self.version.parse().ok()
    }
}

#[cfg(test)]
mod tests;
