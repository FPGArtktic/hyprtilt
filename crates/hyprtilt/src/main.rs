// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! hyprtilt: TUI and CLI for Hyprland monitor layout.

#![forbid(unsafe_code)]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("hyprtilt {}", hyprtilt_core::VERSION);
    }
}
