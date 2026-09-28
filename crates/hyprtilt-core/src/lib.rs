// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Core library of hyprtilt.
//!
//! The crate holds everything that does not depend on a terminal: the monitor
//! model, geometry, parsing and generating Hyprland monitor rules in Lua and
//! hyprlang inside a managed block, profiles, Hyprland IPC and the
//! apply/rollback state machine. The `hyprtilt` binary is a thin layer over it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod apply;
pub mod block;
pub mod config;
pub mod document;
pub mod fsio;
pub mod geometry;
pub mod hyprlang;
pub mod ipc;
pub mod lua;
pub mod model;
pub mod profile;
pub mod settings;

/// Version of the crate, as declared in `Cargo.toml`.
///
/// # Examples
///
/// ```
/// assert!(!hyprtilt_core::VERSION.is_empty());
/// ```
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
