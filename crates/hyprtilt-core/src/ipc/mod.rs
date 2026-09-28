// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Talking to Hyprland.
//!
//! [`HyprlandIpc`] has one required method: send a raw request and return
//! the raw reply. Typed helpers build on it, so the socket implementation
//! and the test mock only have to move bytes. See `docs/hyprland-lua-api.md`
//! section 3 for the wire format.

use serde::Deserialize;

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
pub trait HyprlandIpc {
    /// Send one raw request, including its flag prefix (for example
    /// `j/monitors all` or `/reload`), and return the complete reply.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError`] when Hyprland cannot be reached.
    fn request(&self, request: &str) -> Result<String, IpcError>;
}

/// The active workspace of a monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    /// `lua` or `hyprlang`.
    pub config_provider: String,
    /// `drm` or `wayland`.
    #[serde(default)]
    pub backend: String,
}
