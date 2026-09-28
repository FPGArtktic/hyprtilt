// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Named monitor layouts stored as TOML files in
//! `~/.config/hyprtilt/profiles/<name>.toml`.
//!
//! A profile is a complete set of monitor rules, optionally with the file
//! and language it belongs to. TOML is human-editable and allows comments
//! (ADR 0001, D11):
//!
//! ```toml
//! backend = "lua"
//! target = "~/.config/caelestia/hypr-user.lua"
//!
//! [[monitor]]
//! output = "desc:Samsung Electric Company Odyssey G50F SERIAL0001"
//! mode = "2560x1440@144"
//! position = "0x0"
//! scale = 1.0
//! transform = 1
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::document::Backend;
use crate::fsio;
use crate::model::MonitorRule;
use crate::settings::expand_tilde;

/// A saved layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Language of the target file, if the profile belongs to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<Backend>,
    /// The file the profile is applied to, if not the usual one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    /// The monitor rules.
    #[serde(default, rename = "monitor")]
    pub monitors: Vec<MonitorRule>,
}

/// Why a profile cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The name contains characters that do not belong in a file name.
    #[error(
        "invalid profile name {0:?}: use letters, digits, '.', '_' and '-', not starting with '.'"
    )]
    InvalidName(String),
    /// No profile of that name.
    #[error("no profile named {0:?}")]
    NotFound(String),
    /// The file is not a valid profile.
    #[error("{path}: {message}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
    /// Reading, writing or listing failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// Writing the file failed.
    #[error(transparent)]
    Write(#[from] fsio::WriteError),
}

/// The profiles in one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileStore {
    dir: PathBuf,
}

/// Check that `name` can be a file name on its own.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidName`] otherwise.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::profile::validate_name;
///
/// assert!(validate_name("home-office_2").is_ok());
/// assert!(validate_name("../x").is_err());
/// assert!(validate_name(".hidden").is_err());
/// ```
pub fn validate_name(name: &str) -> Result<(), ProfileError> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(ProfileError::InvalidName(name.to_owned()))
    }
}

impl ProfileStore {
    /// The profiles in `dir`.
    #[must_use]
    pub fn new(dir: PathBuf) -> ProfileStore {
        ProfileStore { dir }
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of a profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::InvalidName`] for an invalid name.
    pub fn path(&self, name: &str) -> Result<PathBuf, ProfileError> {
        validate_name(name)?;
        Ok(self.dir.join(format!("{name}.toml")))
    }

    /// The names of all profiles, sorted. A missing directory has none.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::Io`] when the directory cannot be listed.
    pub fn list(&self) -> Result<Vec<String>, ProfileError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ProfileError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".toml").map(str::to_owned)
            })
            .filter(|n| validate_name(n).is_ok())
            .collect();
        names.sort();
        Ok(names)
    }

    /// Read a profile. A `target` starting with `~` is expanded with
    /// `home`.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an invalid name, a missing profile, or a
    /// file that is not a valid profile (including two rules for the same
    /// output).
    pub fn load(&self, name: &str, home: Option<&str>) -> Result<Profile, ProfileError> {
        let path = self.path(name)?;
        let text = std::fs::read_to_string(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ProfileError::NotFound(name.to_owned())
            } else {
                ProfileError::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let mut profile = Profile::from_toml(&text).map_err(|message| ProfileError::Parse {
            path: path.clone(),
            message,
        })?;
        profile.target = profile.target.map(|t| expand_tilde(&t, home));
        Ok(profile)
    }

    /// Write a profile, replacing one of the same name.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an invalid name or a failed write.
    pub fn save(
        &self,
        name: &str,
        profile: &Profile,
        now: SystemTime,
    ) -> Result<PathBuf, ProfileError> {
        let path = self.path(name)?;
        let snapshot = fsio::snapshot(&path).map_err(|source| ProfileError::Io {
            path: path.clone(),
            source,
        })?;
        fsio::replace(&snapshot, &profile.to_toml(name), None, now)?;
        Ok(path)
    }

    /// Delete a profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::NotFound`] if there is no such profile.
    pub fn delete(&self, name: &str) -> Result<(), ProfileError> {
        let path = self.path(name)?;
        std::fs::remove_file(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ProfileError::NotFound(name.to_owned())
            } else {
                ProfileError::Io { path, source }
            }
        })
    }
}

impl Profile {
    /// Parse a profile, refusing two rules for the same output.
    ///
    /// # Errors
    ///
    /// Returns a message describing the problem.
    pub fn from_toml(text: &str) -> Result<Profile, String> {
        let profile: Profile = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        for (i, rule) in profile.monitors.iter().enumerate() {
            if profile.monitors[..i]
                .iter()
                .any(|r| r.output == rule.output)
            {
                return Err(format!(
                    "more than one [[monitor]] for {:?}",
                    rule.output.as_str()
                ));
            }
        }
        Ok(profile)
    }

    /// The profile as TOML, with a comment naming it.
    #[must_use]
    pub fn to_toml(&self, name: &str) -> String {
        let body = toml::to_string_pretty(self).unwrap_or_default();
        format!(
            "# hyprtilt profile {name:?}; the format is described in\n# https://hyprtilt.readthedocs.io/en/latest/guide/profiles/\n\n{body}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Position, Scale, Transform};
    use std::time::UNIX_EPOCH;

    fn sample() -> Profile {
        let mut hdmi = MonitorRule::new("desc:Samsung Electric Company Odyssey G50F SERIAL0001");
        hdmi.mode = Some("2560x1440@144".parse().unwrap());
        hdmi.position = Some(Position::At { x: 0, y: 0 });
        hdmi.scale = Some(Scale::Factor(1.0));
        hdmi.transform = Some(Transform::new(1).unwrap());
        let mut edp = MonitorRule::new("eDP-1");
        edp.scale = Some(Scale::Auto);
        edp.vrr = Some(2);
        Profile {
            backend: Some(Backend::Lua),
            target: Some(PathBuf::from("/home/u/.config/caelestia/hypr-user.lua")),
            monitors: vec![hdmi, edp],
        }
    }

    #[test]
    fn save_list_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles"));
        assert!(store.list().unwrap().is_empty());
        let profile = sample();
        let path = store.save("home", &profile, UNIX_EPOCH).unwrap();
        assert_eq!(path, dir.path().join("profiles/home.toml"));
        store
            .save(
                "laptop",
                &Profile {
                    backend: None,
                    target: None,
                    monitors: vec![],
                },
                UNIX_EPOCH,
            )
            .unwrap();
        std::fs::write(dir.path().join("profiles/notes.txt"), "x").unwrap();
        assert_eq!(store.list().unwrap(), ["home", "laptop"]);
        assert_eq!(store.load("home", None).unwrap(), profile);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# hyprtilt profile \"home\""));
        assert!(text.contains("[[monitor]]\noutput = \"desc:Samsung"));
        store.delete("home").unwrap();
        assert!(matches!(
            store.load("home", None),
            Err(ProfileError::NotFound(_))
        ));
        assert!(matches!(
            store.delete("home"),
            Err(ProfileError::NotFound(_))
        ));
        assert_eq!(store.dir(), dir.path().join("profiles"));
    }

    #[test]
    fn hand_written_profiles() {
        let text = r#"
target = "~/hypr.lua"
[[monitor]]
output = "DP-1"
mode = "2560x1440@179.95"
position = "3360x975"
scale = 1
vrr = 2
"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("desk.toml"), text).unwrap();
        let store = ProfileStore::new(dir.path().to_path_buf());
        let p = store.load("desk", Some("/home/u")).unwrap();
        assert_eq!(p.target, Some(PathBuf::from("/home/u/hypr.lua")));
        assert_eq!(p.monitors[0].scale, Some(Scale::Factor(1.0)));
        assert_eq!(p.monitors[0].vrr, Some(2));
    }

    #[test]
    fn invalid_profiles() {
        assert!(Profile::from_toml("[[monitor]]\nmode = \"preferred\"\n").is_err());
        assert!(Profile::from_toml("[[monitor]]\noutput = \"a\"\ntransform = 9\n").is_err());
        assert!(Profile::from_toml("[[monitor]]\noutput = \"a\"\ncolour = 1\n").is_err());
        let err = Profile::from_toml("[[monitor]]\noutput = \"a\"\n[[monitor]]\noutput = \"a\"\n")
            .unwrap_err();
        assert_eq!(err, "more than one [[monitor]] for \"a\"");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.toml"), "[[monitor]]\n").unwrap();
        let store = ProfileStore::new(dir.path().to_path_buf());
        assert!(matches!(
            store.load("bad", None),
            Err(ProfileError::Parse { .. })
        ));
        assert!(matches!(
            store.load("../etc/passwd", None),
            Err(ProfileError::InvalidName(_))
        ));
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
    }
}
