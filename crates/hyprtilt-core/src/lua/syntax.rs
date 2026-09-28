// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! A syntax check with an external Lua compiler, as a safety net before a
//! file is replaced.
//!
//! A syntax error in a file loaded with `require` makes Hyprland clear all
//! monitor rules on the next reload (`docs/hyprland-lua-api.md`, section
//! 5.8), so a broken write would rearrange the user's screens. hyprtilt's
//! own checks keep the structure intact; `luac -p` confirms it when a
//! compiler is installed. Lua 5.5 is what Hyprland runs; a 5.4 compiler is
//! accepted too, because the check only refuses a write that turns a file
//! that compiled into one that does not.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Compiler names to look for, in order of preference.
pub const CANDIDATES: &[&str] = &["luac5.5", "luac", "luac5.4"];

/// A Lua compiler that can check syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Luac {
    /// Path of the executable.
    pub path: PathBuf,
    /// Version reported by `luac -v`, such as `5.5.1`.
    pub version: String,
}

impl Luac {
    /// The first usable compiler in the directories of `path_var`, a
    /// `PATH`-style list.
    #[must_use]
    pub fn find(path_var: &str) -> Option<Luac> {
        CANDIDATES.iter().find_map(|name| {
            path_var
                .split(':')
                .filter(|d| !d.is_empty())
                .map(|d| Path::new(d).join(name))
                .filter(|p| p.is_file())
                .find_map(|p| Luac::probe(&p))
        })
    }

    /// Ask `path -v` for its version; `None` unless it is Lua 5.4 or 5.5.
    #[must_use]
    pub fn probe(path: &Path) -> Option<Luac> {
        let out = Command::new(path)
            .arg("-v")
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        let version = parse_version(&text)?;
        Some(Luac {
            path: path.to_path_buf(),
            version,
        })
    }

    /// Compile `src` without running it.
    ///
    /// # Errors
    ///
    /// Returns the compiler's message, such as `3: <eof> expected near 'hl'`,
    /// when the source does not compile or the compiler cannot be run.
    pub fn check(&self, src: &str) -> Result<(), String> {
        let mut child = Command::new(&self.path)
            .args(["-p", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.path.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A write error means the compiler exited early; its status
            // and message below say why.
            let _ = stdin.write_all(src.as_bytes());
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("cannot run {}: {e}", self.path.display()))?;
        if out.status.success() {
            return Ok(());
        }
        let message = String::from_utf8_lossy(&out.stderr);
        let message = message.trim();
        // "luac5.4: stdin:3: <eof> expected near 'hl'" -> "line 3: ..."
        let message = match message.split_once("stdin:") {
            Some((_, rest)) => format!("line {rest}"),
            None => message.to_owned(),
        };
        Err(message)
    }

    /// Refuse `new` if it does not compile while `old` did. A file that
    /// already failed to compile (for example because it uses syntax of a
    /// newer Lua than this compiler) is not held against the new content.
    ///
    /// # Errors
    ///
    /// Returns the compiler's message for the new content.
    pub fn check_regression(&self, old: &str, new: &str) -> Result<(), String> {
        let Err(e) = self.check(new) else {
            return Ok(());
        };
        if self.check(old).is_ok() {
            Err(e)
        } else {
            Ok(())
        }
    }
}

/// `Lua 5.5.1  Copyright ...` -> `5.5.1`, only for 5.4 and 5.5.
fn parse_version(text: &str) -> Option<String> {
    let rest = text.split("Lua ").nth(1)?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (version.starts_with("5.4") || version.starts_with("5.5")).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("Lua 5.5.1  Copyright (C) 1994-2025 Lua.org, PUC-Rio"),
            Some("5.5.1".to_owned())
        );
        assert_eq!(
            parse_version("Lua 5.4.6  Copyright"),
            Some("5.4.6".to_owned())
        );
        assert_eq!(parse_version("Lua 5.3.6  Copyright"), None);
        assert_eq!(parse_version("LuaJIT 2.1"), None);
    }

    #[test]
    fn missing_compiler() {
        assert_eq!(Luac::find("/nonexistent:"), None);
        assert_eq!(Luac::probe(Path::new("/nonexistent/luac")), None);
        let fake = Luac {
            path: PathBuf::from("/nonexistent/luac"),
            version: "5.5.0".to_owned(),
        };
        assert!(fake.check("x = 1").unwrap_err().starts_with("cannot run"));
    }

    #[test]
    fn real_compiler_when_installed() {
        let Some(luac) = Luac::find(&std::env::var("PATH").unwrap_or_default()) else {
            eprintln!("no luac 5.4/5.5 installed; skipping");
            return;
        };
        assert!(
            luac.check("hl.monitor({ output = \"DP-1\" })\nreturn {}\n")
                .is_ok()
        );
        let err = luac.check("return {}\nhl.monitor({})\n").unwrap_err();
        assert!(err.starts_with("line 2:"), "{err}");
        assert!(luac.check_regression("x = 1", "x = ").is_err());
        assert!(luac.check_regression("x = ", "y = ").is_ok());
    }
}
