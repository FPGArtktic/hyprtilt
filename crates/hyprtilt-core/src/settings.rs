// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Application settings from `~/.config/hyprtilt/config.toml`.
//!
//! ```toml
//! # File to edit instead of the one hyprtilt finds, and its language.
//! target = "~/.config/caelestia/hypr-user.lua"
//! backend = "lua"
//!
//! # Seconds to wait for confirmation after applying; 0 keeps changes
//! # without asking (they are still verified).
//! confirm_timeout = 15
//!
//! # Snap to neighbouring edges when moving in the terminal interface.
//! snap = true
//!
//! # Write desc: selectors (make, model, serial) for new rules instead of
//! # connector names, so that rules survive connector renames.
//! use_descriptions = false
//!
//! [backup]
//! dir = "~/.local/state/hyprtilt/backups"   # next to the file when unset
//! keep = 10                                  # 0 disables backups
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Backend;
use crate::fsio::BackupPolicy;

/// Application settings. A missing file or key means the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// File to edit instead of the one hyprtilt finds.
    pub target: Option<PathBuf>,
    /// Language of `target`, when its extension is misleading.
    pub backend: Option<Backend>,
    /// Seconds to wait for confirmation after applying; 0 disables it.
    pub confirm_timeout: u64,
    /// Snapping in the terminal interface.
    pub snap: bool,
    /// New rules use `desc:` selectors instead of connector names.
    pub use_descriptions: bool,
    /// Backups made before every write.
    pub backup: BackupSettings,
}

/// The `[backup]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackupSettings {
    /// Directory for backups; next to the file when unset.
    pub dir: Option<PathBuf>,
    /// How many backups of a file to keep; 0 disables backups.
    pub keep: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            target: None,
            backend: None,
            confirm_timeout: 15,
            snap: true,
            use_descriptions: false,
            backup: BackupSettings::default(),
        }
    }
}

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings {
            dir: None,
            keep: 10,
        }
    }
}

/// Why the settings file cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The file exists but cannot be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// The file is not valid TOML or has unknown keys.
    #[error("{path}: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

/// The configuration directory of hyprtilt: `$XDG_CONFIG_HOME/hyprtilt`, or
/// `~/.config/hyprtilt`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::settings::config_dir;
/// use std::path::PathBuf;
///
/// assert_eq!(config_dir(None, Some("/home/u")), PathBuf::from("/home/u/.config/hyprtilt"));
/// assert_eq!(config_dir(Some("/x"), Some("/home/u")), PathBuf::from("/x/hyprtilt"));
/// ```
#[must_use]
pub fn config_dir(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    match xdg_config_home.filter(|d| d.starts_with('/')) {
        Some(dir) => Path::new(dir).join("hyprtilt"),
        None => Path::new(home.unwrap_or("/")).join(".config/hyprtilt"),
    }
}

/// Replace a leading `~` with `home`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::settings::expand_tilde;
/// use std::path::{Path, PathBuf};
///
/// assert_eq!(expand_tilde(Path::new("~/a.lua"), Some("/home/u")), PathBuf::from("/home/u/a.lua"));
/// assert_eq!(expand_tilde(Path::new("/b.lua"), Some("/home/u")), PathBuf::from("/b.lua"));
/// ```
#[must_use]
pub fn expand_tilde(path: &Path, home: Option<&str>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => Path::new(home).join(rest),
        _ => path.to_path_buf(),
    }
}

impl Settings {
    /// Read the settings from `path`; a missing file gives the defaults.
    /// Paths starting with `~` are expanded with `home`.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] for an unreadable or invalid file.
    pub fn load(path: &Path, home: Option<&str>) -> Result<Settings, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(source) => {
                return Err(SettingsError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let mut settings: Settings = toml::from_str(&text).map_err(|e| SettingsError::Parse {
            path: path.to_path_buf(),
            message: e.message().to_owned(),
        })?;
        settings.target = settings.target.map(|p| expand_tilde(&p, home));
        settings.backup.dir = settings.backup.dir.map(|p| expand_tilde(&p, home));
        Ok(settings)
    }

    /// The backup policy for file writes.
    #[must_use]
    pub fn backup_policy(&self) -> BackupPolicy {
        BackupPolicy {
            dir: self.backup.dir.clone(),
            keep: self.backup.keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_a_missing_file() {
        let s = Settings::load(Path::new("/nonexistent/config.toml"), None).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.confirm_timeout, 15);
        assert!(s.snap);
        assert_eq!(s.backup_policy(), BackupPolicy::default());
    }

    #[test]
    fn full_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "target = \"~/.config/caelestia/hypr-user.lua\"\nbackend = \"lua\"\nconfirm_timeout = 0\nsnap = false\nuse_descriptions = true\n[backup]\ndir = \"~/b\"\nkeep = 3\n",
        )
        .unwrap();
        let s = Settings::load(&path, Some("/home/u")).unwrap();
        assert_eq!(
            s.target,
            Some(PathBuf::from("/home/u/.config/caelestia/hypr-user.lua"))
        );
        assert_eq!(s.backend, Some(Backend::Lua));
        assert_eq!(s.confirm_timeout, 0);
        assert!(!s.snap && s.use_descriptions);
        assert_eq!(
            s.backup_policy(),
            BackupPolicy {
                dir: Some(PathBuf::from("/home/u/b")),
                keep: 3
            }
        );
    }

    #[test]
    fn unknown_keys_and_bad_values_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "tagret = \"x\"\n").unwrap();
        let err = Settings::load(&path, None).unwrap_err();
        assert!(err.to_string().contains("unknown field `tagret`"), "{err}");
        std::fs::write(&path, "backend = \"python\"\n").unwrap();
        assert!(Settings::load(&path, None).is_err());
        let err = Settings::load(dir.path(), None).unwrap_err();
        assert!(matches!(err, SettingsError::Read { .. }), "{err}");
    }

    #[test]
    fn directories() {
        assert_eq!(
            config_dir(Some("relative"), Some("/h")),
            PathBuf::from("/h/.config/hyprtilt")
        );
        assert_eq!(config_dir(None, None), PathBuf::from("/.config/hyprtilt"));
        assert_eq!(expand_tilde(Path::new("~/x"), None), PathBuf::from("~/x"));
    }
}
