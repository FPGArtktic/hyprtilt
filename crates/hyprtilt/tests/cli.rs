// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! End-to-end tests of the `hyprtilt` binary.

use std::process::Command;

#[test]
fn version_prints_crate_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_hyprtilt"))
        .arg("--version")
        .output()
        .expect("run hyprtilt");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(stdout, format!("hyprtilt {}\n", env!("CARGO_PKG_VERSION")));
}
