// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Hyprland versions and the features that depend on them
//! (`docs/hyprland-lua-api.md`, sections 6 and 7).
//!
//! Version numbers alone never decide whether hyprlang works: git builds of
//! Hyprland's `main` branch report 0.56.0 and have no hyprlang at all. The
//! running compositor's `configProvider` decides that; versions only gate
//! optional fields and requests.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

/// A Hyprland version, `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Version {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version.
    pub patch: u32,
}

impl Version {
    /// A version from its parts.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// The Lua configuration and `hyprctl eval` (0.55.0).
    pub const LUA: Version = Version::new(0, 55, 0);
    /// `monitorv2 { }` blocks in hyprlang (0.50.0).
    pub const MONITORV2: Version = Version::new(0, 50, 0);
    /// `sdr_eotf` as a name in `monitorv2` (0.54.0; an integer before).
    pub const SDR_EOTF_NAMES: Version = Version::new(0, 54, 0);
    /// `icc` in `monitor=` and `monitorv2` (0.55.0).
    pub const ICC: Version = Version::new(0, 55, 0);
    /// `disabled = false` re-enables an output in Lua (0.56.0).
    pub const DISABLED_FALSE: Version = Version::new(0, 56, 0);
    /// Omitted `mode` and `position` mean preferred and auto (0.56.0).
    pub const PREFERRED_DEFAULTS: Version = Version::new(0, 56, 0);
    /// The `.conf` format is announced to be removed in 0.57 (0.56.1).
    pub const HYPRLANG_DEPRECATED: Version = Version::new(0, 56, 1);

    /// Whether this version is at least `other`.
    #[must_use]
    pub fn at_least(self, other: Version) -> bool {
        self >= other
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A string that does not start with a version number.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a version: {0:?}")]
pub struct BadVersion(pub String);

impl FromStr for Version {
    type Err = BadVersion;

    /// Parse the leading `major.minor.patch` of `0.56.2`, `v0.56.2` or
    /// `v0.56.0-209-g4bb6844b`. A missing patch is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::version::Version;
    ///
    /// assert_eq!("0.56.2".parse::<Version>().unwrap(), Version::new(0, 56, 2));
    /// assert_eq!("v0.56.0-209-g4bb6844b".parse::<Version>().unwrap(), Version::new(0, 56, 0));
    /// assert!("unknown".parse::<Version>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadVersion(s.to_owned());
        let t = s.trim().trim_start_matches('v');
        let numeric: String = t
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = numeric.split('.').filter(|p| !p.is_empty());
        let major = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let minor = parts.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let patch = match parts.next() {
            Some(p) => p.parse().map_err(|_| bad())?,
            None => 0,
        };
        Ok(Version::new(major, minor, patch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_order() {
        assert_eq!("0.55".parse::<Version>().unwrap(), Version::new(0, 55, 0));
        assert!("".parse::<Version>().is_err());
        assert!("0".parse::<Version>().is_err());
        assert!(Version::new(0, 56, 2).at_least(Version::LUA));
        assert!(!Version::new(0, 53, 3).at_least(Version::LUA));
        assert!(Version::new(0, 53, 3).at_least(Version::MONITORV2));
        assert_eq!(Version::new(0, 56, 2).to_string(), "0.56.2");
        assert_eq!(
            BadVersion("x".to_owned()).to_string(),
            "not a version: \"x\""
        );
    }
}
