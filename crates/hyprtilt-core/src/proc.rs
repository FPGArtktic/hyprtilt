// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! What the compositor's process shows in `/proc`: its command line and
//! environment (which decide the main configuration file), its working
//! directory (which relative `package.path` entries use), and the files it
//! watches for changes.
//!
//! No IPC request lists the files a configuration loaded, but Hyprland
//! watches every loaded file with inotify, and the watches are listed in
//! `/proc/<pid>/fdinfo` with their inode numbers
//! (`docs/hyprland-lua-api.md`, section 10.1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Facts about a process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProcessInfo {
    /// Process ID.
    pub pid: u32,
    /// Command line.
    pub cmdline: Vec<String>,
    /// Environment at the time the process started.
    pub environ: HashMap<String, String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Inode numbers of the inotify watches, if they could be read.
    pub watched_inodes: Option<Vec<u64>>,
}

/// Read what `/proc` (at `proc_root`) shows about `pid`. Missing parts stay
/// empty; a process of another user shows little.
#[must_use]
pub fn inspect(proc_root: &Path, pid: u32) -> ProcessInfo {
    let dir = proc_root.join(pid.to_string());
    let split = |blob: Vec<u8>| -> Vec<String> {
        blob.split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect()
    };
    let cmdline = std::fs::read(dir.join("cmdline"))
        .map(split)
        .unwrap_or_default();
    let environ = std::fs::read(dir.join("environ"))
        .map(split)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect();
    ProcessInfo {
        pid,
        cmdline,
        environ,
        cwd: std::fs::read_link(dir.join("cwd")).ok(),
        watched_inodes: watched_inodes(&dir),
    }
}

fn watched_inodes(dir: &Path) -> Option<Vec<u64>> {
    let mut inodes = Vec::new();
    for entry in std::fs::read_dir(dir.join("fd"))
        .ok()?
        .filter_map(Result::ok)
    {
        let is_inotify = std::fs::read_link(entry.path())
            .is_ok_and(|target| target.as_os_str() == "anon_inode:inotify");
        if !is_inotify {
            continue;
        }
        let info = dir.join("fdinfo").join(entry.file_name());
        if let Ok(text) = std::fs::read_to_string(info) {
            inodes.extend(parse_inotify(&text));
        }
    }
    Some(inodes)
}

/// The inode numbers in an inotify fd's `fdinfo`, one per watch:
/// `inotify wd:1 ino:a158fc sdev:1d mask:8 ...` (hexadecimal).
///
/// # Examples
///
/// ```
/// let text = "pos:\t0\nflags:\t02000000\ninotify wd:2 ino:a158fc sdev:1d mask:8 ignored_mask:0\n";
/// assert_eq!(hyprtilt_core::proc::parse_inotify(text), [0xa158fc]);
/// ```
#[must_use]
pub fn parse_inotify(fdinfo: &str) -> Vec<u64> {
    fdinfo
        .lines()
        .filter(|l| l.starts_with("inotify "))
        .filter_map(|l| {
            l.split_whitespace()
                .find_map(|field| field.strip_prefix("ino:"))
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_fake_proc_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("42");
        std::fs::create_dir_all(dir.join("fd")).unwrap();
        std::fs::create_dir_all(dir.join("fdinfo")).unwrap();
        std::fs::write(dir.join("cmdline"), b"Hyprland\0--watchdog-fd\x004\0").unwrap();
        std::fs::write(
            dir.join("environ"),
            b"HOME=/home/u\0XDG_RUNTIME_DIR=/run/user/1000\0junk\0",
        )
        .unwrap();
        symlink("/home/u", dir.join("cwd")).unwrap();
        symlink("anon_inode:inotify", dir.join("fd/7")).unwrap();
        symlink("/dev/null", dir.join("fd/0")).unwrap();
        std::fs::write(
            dir.join("fdinfo/7"),
            "inotify wd:1 ino:9e222a sdev:1d mask:8 ignored_mask:0\ninotify wd:2 ino:a158fc sdev:1d mask:8\n",
        )
        .unwrap();
        let info = inspect(root.path(), 42);
        assert_eq!(info.cmdline, ["Hyprland", "--watchdog-fd", "4"]);
        assert_eq!(
            info.environ.get("HOME").map(String::as_str),
            Some("/home/u")
        );
        assert_eq!(info.environ.len(), 2);
        assert_eq!(info.cwd, Some(PathBuf::from("/home/u")));
        assert_eq!(info.watched_inodes, Some(vec![0x9e_222a, 0xa1_58fc]));
    }

    #[test]
    fn missing_process() {
        let info = inspect(Path::new("/nonexistent"), 1);
        assert!(info.cmdline.is_empty() && info.watched_inodes.is_none());
        assert!(parse_inotify("inotify wd:1 ino:zz\n").is_empty());
    }

    #[test]
    fn our_own_process() {
        let info = inspect(Path::new("/proc"), std::process::id());
        assert!(!info.cmdline.is_empty());
        assert!(info.watched_inodes.is_some());
    }
}
