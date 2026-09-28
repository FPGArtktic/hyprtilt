// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The monitor rule model shared by both configuration backends.
//!
//! A [`MonitorRule`] is one `hl.monitor({...})` call or one `monitor=` line.
//! Every field except the selector is optional: `None` means "not written",
//! which matters because Hyprland merges Lua rules with the same `output`
//! string field by field (see `docs/hyprland-lua-api.md`). The typed values
//! mirror what Hyprland 0.56 accepts; anything hyprtilt does not model is
//! kept verbatim in [`MonitorRule::extra`].

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Which monitors a rule applies to: the `output` key in Lua, the first
/// field of `monitor=` in hyprlang.
///
/// The original string is kept, so that a rule written as `desc:  Samsung`
/// round-trips unchanged. Hyprland compares selector strings exactly when it
/// merges rules, so hyprtilt does too.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Selector(String);

/// The kind of a [`Selector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind<'a> {
    /// `""`: the fallback rule, used only when no other rule matches.
    Fallback,
    /// A connector name such as `HDMI-A-1`, matched exactly.
    Connector(&'a str),
    /// `desc:<prefix>`: the trimmed prefix is matched against the start of
    /// the monitor description.
    Description(&'a str),
}

impl Selector {
    /// Create a selector from the string written in the configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::{Selector, SelectorKind};
    ///
    /// assert_eq!(Selector::new("DP-1").kind(), SelectorKind::Connector("DP-1"));
    /// assert_eq!(Selector::new("desc: Samsung").kind(), SelectorKind::Description("Samsung"));
    /// assert_eq!(Selector::new("").kind(), SelectorKind::Fallback);
    /// ```
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Selector(s.into())
    }

    /// The selector exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What kind of selector this is.
    #[must_use]
    pub fn kind(&self) -> SelectorKind<'_> {
        if self.0.is_empty() {
            SelectorKind::Fallback
        } else if let Some(prefix) = self.0.strip_prefix("desc:") {
            SelectorKind::Description(prefix.trim())
        } else {
            SelectorKind::Connector(&self.0)
        }
    }

    /// Whether the selector matches a monitor, the way Hyprland's
    /// `matchesStaticSelector` decides it. `description` is the description
    /// reported by `hyprctl monitors -j` (make, model and serial).
    ///
    /// The fallback selector never matches here: Hyprland uses it only when
    /// no other rule matched.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Selector;
    ///
    /// let desc = "Samsung Electric Company Odyssey G50F HNAYC01385";
    /// assert!(Selector::new("HDMI-A-1").matches("HDMI-A-1", desc));
    /// assert!(Selector::new("desc:Samsung Electric Company Odyssey").matches("DP-1", desc));
    /// assert!(!Selector::new("").matches("DP-1", desc));
    /// ```
    #[must_use]
    pub fn matches(&self, connector: &str, description: &str) -> bool {
        match self.kind() {
            SelectorKind::Fallback => false,
            SelectorKind::Connector(name) => name == connector,
            SelectorKind::Description(prefix) => description.replace(',', "").starts_with(prefix),
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The `mode` of a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Mode {
    /// `preferred`: the monitor's preferred mode.
    Preferred,
    /// `highrr`: the highest refresh rate.
    HighRefreshRate,
    /// `highres`: the highest resolution.
    HighResolution,
    /// `maxwidth`: the widest mode.
    MaxWidth,
    /// `WxH` or `WxH@Hz`. Without a refresh rate Hyprland targets 60 Hz,
    /// so hyprtilt always writes one.
    Resolution {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Refresh rate in Hz, if written.
        refresh: Option<f64>,
    },
    /// `modeline ...`, kept verbatim (Hyprland truncates the clock, so it is
    /// never regenerated).
    Modeline(String),
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Preferred => f.write_str("preferred"),
            Mode::HighRefreshRate => f.write_str("highrr"),
            Mode::HighResolution => f.write_str("highres"),
            Mode::MaxWidth => f.write_str("maxwidth"),
            Mode::Resolution {
                width,
                height,
                refresh: None,
            } => write!(f, "{width}x{height}"),
            Mode::Resolution {
                width,
                height,
                refresh: Some(hz),
            } => write!(f, "{width}x{height}@{}", format_refresh(*hz)),
            Mode::Modeline(s) => f.write_str(s),
        }
    }
}

/// Format a refresh rate with at most two decimals and no trailing zeros,
/// matching the precision of `availableModes` (`179.95`, `144`).
#[must_use]
pub fn format_refresh(hz: f64) -> String {
    let s = format!("{hz:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// Error for a value that Hyprland would reject.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what}: {value:?}")]
pub struct InvalidValue {
    /// Which field or value kind.
    pub what: &'static str,
    /// The offending text.
    pub value: String,
}

impl InvalidValue {
    fn new(what: &'static str, value: &str) -> Self {
        InvalidValue {
            what,
            value: value.to_owned(),
        }
    }
}

impl FromStr for Mode {
    type Err = InvalidValue;

    /// Parse a mode the way Hyprland's `parseMode` does, but strictly:
    /// keyword prefixes (`pref...`) are accepted as Hyprland does, while
    /// malformed resolutions are errors instead of silent fallbacks.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Mode;
    ///
    /// let m: Mode = "2560x1440@179.95".parse().unwrap();
    /// assert_eq!(m, Mode::Resolution { width: 2560, height: 1440, refresh: Some(179.95) });
    /// assert_eq!("preferred".parse::<Mode>().unwrap(), Mode::Preferred);
    /// assert!("wide".parse::<Mode>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.is_empty() || t.starts_with("pref") {
            return Ok(Mode::Preferred);
        }
        if t.starts_with("highrr") {
            return Ok(Mode::HighRefreshRate);
        }
        if t.starts_with("highres") {
            return Ok(Mode::HighResolution);
        }
        if t.starts_with("maxwidth") {
            return Ok(Mode::MaxWidth);
        }
        if t.len() >= 8 && t[..8].eq_ignore_ascii_case("modeline") {
            return Ok(Mode::Modeline(t.to_owned()));
        }
        let (size, refresh) = match t.split_once('@') {
            Some((size, hz)) => {
                let hz = hz.trim().trim_end_matches("Hz").trim_end_matches("hz");
                let hz: f64 = hz.parse().map_err(|_| InvalidValue::new("mode", s))?;
                if !(hz.is_finite() && hz > 0.0) {
                    return Err(InvalidValue::new("mode", s));
                }
                (size, Some(hz))
            }
            None => (t, None),
        };
        let (w, h) = size
            .split_once('x')
            .ok_or_else(|| InvalidValue::new("mode", s))?;
        let width = w.trim().parse().map_err(|_| InvalidValue::new("mode", s))?;
        let height = h.trim().parse().map_err(|_| InvalidValue::new("mode", s))?;
        if width == 0 || height == 0 {
            return Err(InvalidValue::new("mode", s));
        }
        Ok(Mode::Resolution {
            width,
            height,
            refresh,
        })
    }
}

impl Mode {
    /// The pixel size, for an explicit resolution.
    #[must_use]
    pub fn resolution(&self) -> Option<(u32, u32)> {
        match self {
            Mode::Resolution { width, height, .. } => Some((*width, *height)),
            _ => None,
        }
    }

    /// The refresh rate, for an explicit resolution that has one.
    #[must_use]
    pub fn refresh(&self) -> Option<f64> {
        match self {
            Mode::Resolution { refresh, .. } => *refresh,
            _ => None,
        }
    }

    /// The mode as it reads back after being written: the refresh rate
    /// with at most two decimals.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Mode;
    ///
    /// let m = Mode::Resolution { width: 2560, height: 1440, refresh: Some(179.952) };
    /// assert_eq!(m.normalized().to_string(), "2560x1440@179.95");
    /// assert_eq!(m.normalized().refresh(), Some(179.95));
    /// ```
    #[must_use]
    pub fn normalized(&self) -> Mode {
        match self {
            Mode::Resolution {
                width,
                height,
                refresh: Some(hz),
            } => Mode::Resolution {
                width: *width,
                height: *height,
                refresh: Some(format_refresh(*hz).parse().unwrap_or(*hz)),
            },
            other => other.clone(),
        }
    }
}

impl From<Mode> for String {
    fn from(m: Mode) -> String {
        m.to_string()
    }
}

impl TryFrom<String> for Mode {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Direction of an automatic position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoDirection {
    /// `auto-right` (also what plain `auto` means).
    Right,
    /// `auto-left`.
    Left,
    /// `auto-up`.
    Up,
    /// `auto-down`.
    Down,
    /// `auto-center-right`.
    CenterRight,
    /// `auto-center-left`.
    CenterLeft,
    /// `auto-center-up`.
    CenterUp,
    /// `auto-center-down`.
    CenterDown,
}

/// The `position` of a rule, in logical (scaled) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Position {
    /// `auto` (`None`) or `auto-<direction>`.
    Auto(Option<AutoDirection>),
    /// `XxY`. Negative coordinates are allowed.
    At {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
    },
}

const AUTO_NAMES: &[(&str, AutoDirection)] = &[
    ("auto-right", AutoDirection::Right),
    ("auto-left", AutoDirection::Left),
    ("auto-up", AutoDirection::Up),
    ("auto-down", AutoDirection::Down),
    ("auto-center-right", AutoDirection::CenterRight),
    ("auto-center-left", AutoDirection::CenterLeft),
    ("auto-center-up", AutoDirection::CenterUp),
    ("auto-center-down", AutoDirection::CenterDown),
];

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Position::Auto(None) => f.write_str("auto"),
            Position::Auto(Some(dir)) => {
                let name = AUTO_NAMES
                    .iter()
                    .find(|(_, d)| d == dir)
                    .map_or("auto", |(n, _)| n);
                f.write_str(name)
            }
            Position::At { x, y } => write!(f, "{x}x{y}"),
        }
    }
}

impl FromStr for Position {
    type Err = InvalidValue;

    /// Parse `auto`, `auto-<direction>` or `XxY`.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Position;
    ///
    /// assert_eq!("1440x1335".parse::<Position>().unwrap(), Position::At { x: 1440, y: 1335 });
    /// assert_eq!("-1920x0".parse::<Position>().unwrap(), Position::At { x: -1920, y: 0 });
    /// assert_eq!("auto".parse::<Position>().unwrap(), Position::Auto(None));
    /// assert!("auto-sideways".parse::<Position>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.is_empty() || t == "auto" {
            return Ok(Position::Auto(None));
        }
        if t.starts_with("auto") {
            return AUTO_NAMES
                .iter()
                .find(|(n, _)| *n == t)
                .map(|(_, d)| Position::Auto(Some(*d)))
                .ok_or_else(|| InvalidValue::new("position", s));
        }
        // Split at the first 'x' after the first character, so that a
        // leading minus sign stays with x.
        let split = t
            .char_indices()
            .skip(1)
            .find(|&(_, c)| c == 'x')
            .map(|(i, _)| i)
            .ok_or_else(|| InvalidValue::new("position", s))?;
        let x = t[..split]
            .trim()
            .parse()
            .map_err(|_| InvalidValue::new("position", s))?;
        let y = t[split + 1..]
            .trim()
            .parse()
            .map_err(|_| InvalidValue::new("position", s))?;
        Ok(Position::At { x, y })
    }
}

impl From<Position> for String {
    fn from(p: Position) -> String {
        p.to_string()
    }
}

impl TryFrom<String> for Position {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// The `scale` of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    /// `auto`: Hyprland picks 1, 1.5 or 2 from the pixel density.
    #[serde(with = "auto_string")]
    Auto,
    /// An explicit scale factor. Hyprland snaps it to a multiple of 1/120
    /// that gives an integral logical size (see `geometry`).
    Factor(f64),
}

mod auto_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("auto")
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<(), D::Error> {
        let s = String::deserialize(d)?;
        if s == "auto" {
            Ok(())
        } else {
            Err(serde::de::Error::custom("expected \"auto\""))
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scale::Auto => f.write_str("auto"),
            Scale::Factor(v) => f.write_str(&crate::geometry::format_scale(*v)),
        }
    }
}

/// Hyprland's `isNumber(s, true)` from hyprutils: an optional leading
/// minus, digits, at most one dot that is not the first character, and a
/// digit at the end. `".5"` and `"1."` are not numbers.
fn hyprland_is_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut dot = false;
    for (i, &c) in bytes.iter().enumerate() {
        if i == 0 && c == b'-' {
            continue;
        }
        if c.is_ascii_digit() {
            continue;
        }
        if c != b'.' || i == 0 || dot {
            return false;
        }
        dot = true;
    }
    bytes.last().is_some_and(u8::is_ascii_digit)
}

impl Scale {
    /// Parse a scale exactly as Hyprland's `parseScale` reads a string:
    /// empty or starting with `auto` is automatic, otherwise it must be a
    /// number in Hyprland's strict sense and at least 0.25. No whitespace
    /// is trimmed.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue`] for anything Hyprland would not use.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Scale;
    ///
    /// assert_eq!(Scale::parse_hyprland("1.5").unwrap(), Scale::Factor(1.5));
    /// assert_eq!(Scale::parse_hyprland("auto").unwrap(), Scale::Auto);
    /// assert!(Scale::parse_hyprland(".5").is_err());
    /// assert!(Scale::parse_hyprland(" 1").is_err());
    /// ```
    pub fn parse_hyprland(s: &str) -> Result<Scale, InvalidValue> {
        if s.is_empty() || s.starts_with("auto") {
            return Ok(Scale::Auto);
        }
        if !hyprland_is_number(s) {
            return Err(InvalidValue::new("scale", s));
        }
        match s.parse::<f64>() {
            Ok(v) if v.is_finite() && v >= 0.25 => Ok(Scale::Factor(v)),
            _ => Err(InvalidValue::new("scale", s)),
        }
    }

    /// The scale as it reads back after being written: at most six
    /// decimals. Written rules are a fixed point of reading and writing.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Scale;
    ///
    /// assert_eq!(Scale::Factor(4.0 / 3.0).normalized(), Scale::Factor(1.333333));
    /// assert_eq!(Scale::Auto.normalized(), Scale::Auto);
    /// ```
    #[must_use]
    pub fn normalized(self) -> Scale {
        match self {
            Scale::Auto => Scale::Auto,
            Scale::Factor(v) => {
                Scale::Factor(crate::geometry::format_scale(v).parse().unwrap_or(v))
            }
        }
    }
}

impl FromStr for Scale {
    type Err = InvalidValue;

    /// Parse `auto` or a factor of at least 0.25, the minimum Hyprland
    /// accepts.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Scale;
    ///
    /// assert_eq!("1.25".parse::<Scale>().unwrap(), Scale::Factor(1.25));
    /// assert_eq!("auto".parse::<Scale>().unwrap(), Scale::Auto);
    /// assert!("0.1".parse::<Scale>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Scale::parse_hyprland(s.trim()).map_err(|_| InvalidValue::new("scale", s))
    }
}

/// An output transform, `wl_output_transform` 0–7.
///
/// Values 1–3 rotate the content by 90°, 180° and 270° counter-clockwise;
/// 4–7 flip around the vertical axis first. Odd values swap width and
/// height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Transform(u8);

impl Transform {
    /// No rotation, no flip.
    pub const NORMAL: Transform = Transform(0);

    /// Create a transform from its number.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue`] for values above 7.
    pub fn new(value: u8) -> Result<Self, InvalidValue> {
        if value <= 7 {
            Ok(Transform(value))
        } else {
            Err(InvalidValue::new("transform", &value.to_string()))
        }
    }

    /// The number written in the configuration.
    #[must_use]
    pub fn value(self) -> u8 {
        self.0
    }

    /// Whether width and height are swapped (90° or 270°).
    #[must_use]
    pub fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }

    /// Whether the transform includes a flip.
    #[must_use]
    pub fn is_flipped(self) -> bool {
        self.0 & 4 != 0
    }

    /// The next transform in the direction of `r` in the TUI: one step of
    /// 90° that keeps the flip.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::Transform;
    ///
    /// let t = Transform::new(3).unwrap();
    /// assert_eq!(t.rotated_right().value(), 0);
    /// assert_eq!(Transform::new(7).unwrap().rotated_right().value(), 4);
    /// ```
    #[must_use]
    pub fn rotated_right(self) -> Self {
        Transform((self.0 & 4) | ((self.0 + 1) & 3))
    }

    /// The inverse of [`Transform::rotated_right`].
    #[must_use]
    pub fn rotated_left(self) -> Self {
        Transform((self.0 & 4) | ((self.0 + 3) & 3))
    }

    /// The same rotation with the flip toggled.
    #[must_use]
    pub fn flipped(self) -> Self {
        Transform(self.0 ^ 4)
    }

    /// Rotation in degrees (0, 90, 180, 270), ignoring the flip.
    #[must_use]
    pub fn degrees(self) -> u16 {
        u16::from(self.0 & 3) * 90
    }
}

impl TryFrom<u8> for Transform {
    type Error = InvalidValue;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Transform::new(v)
    }
}

impl From<Transform> for u8 {
    fn from(t: Transform) -> u8 {
        t.0
    }
}

/// Colour management preset (`cm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorManagement {
    /// `auto`.
    Auto,
    /// `srgb`, the default.
    Srgb,
    /// `wide`.
    Wide,
    /// `edid`.
    Edid,
    /// `hdr`.
    Hdr,
    /// `hdredid`.
    Hdredid,
    /// `dcip3`.
    Dcip3,
    /// `dp3`.
    Dp3,
    /// `adobe`.
    Adobe,
}

const CM_NAMES: &[(&str, ColorManagement)] = &[
    ("auto", ColorManagement::Auto),
    ("srgb", ColorManagement::Srgb),
    ("wide", ColorManagement::Wide),
    ("edid", ColorManagement::Edid),
    ("hdr", ColorManagement::Hdr),
    ("hdredid", ColorManagement::Hdredid),
    ("dcip3", ColorManagement::Dcip3),
    ("dp3", ColorManagement::Dp3),
    ("adobe", ColorManagement::Adobe),
];

impl fmt::Display for ColorManagement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = CM_NAMES
            .iter()
            .find(|(_, c)| c == self)
            .map_or("srgb", |(n, _)| n);
        f.write_str(name)
    }
}

impl FromStr for ColorManagement {
    type Err = InvalidValue;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        CM_NAMES
            .iter()
            .find(|(n, _)| *n == s)
            .map(|(_, c)| *c)
            .ok_or_else(|| InvalidValue::new("cm", s))
    }
}

/// The transfer function names `sdr_eotf` accepts. hyprtilt stores these
/// names only: the digit forms mean different functions in Lua and in
/// hyprlang (`docs/hyprland-lua-api.md`, section 8.2), so each backend
/// translates digits when it reads them.
pub const SDR_EOTF_NAMES: &[&str] = &["default", "auto", "srgb", "gamma22", "gamma22force"];

/// Reserved area in logical pixels (`reserved` in Lua, `addreserved` in
/// hyprlang).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Reserved {
    /// Top edge.
    pub top: i32,
    /// Right edge.
    pub right: i32,
    /// Bottom edge.
    pub bottom: i32,
    /// Left edge.
    pub left: i32,
}

/// A value hyprtilt does not model, kept as the exact source text of the
/// value (Lua expression or hyprlang value) so that it can be written back
/// unchanged by the same backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraField {
    /// The key.
    pub key: String,
    /// The value's source text.
    pub raw: String,
}

/// One monitor rule. `None` means the field is not written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorRule {
    /// `output`: which monitors the rule applies to.
    pub output: Selector,
    /// `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// `position`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// `scale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
    /// `transform`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// `disabled`. `Some(false)` re-enables an output (Hyprland ≥ 0.56).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// `vrr`: -1 follows `misc:vrr`, 0 off, 1 on, 2 fullscreen, 3
    /// fullscreen games and video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr: Option<i8>,
    /// `mirror`: the output to mirror (`""` means none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    /// `bitdepth`: only 10 has an effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitdepth: Option<i64>,
    /// `cm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cm: Option<ColorManagement>,
    /// `sdr_eotf`, kept as written (`default`, `auto`, `srgb`, `gamma22`,
    /// `gamma22force` or a digit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_eotf: Option<String>,
    /// `sdrbrightness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdrbrightness: Option<f64>,
    /// `sdrsaturation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdrsaturation: Option<f64>,
    /// `icc`: path of an ICC profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icc: Option<String>,
    /// `supports_wide_color`: -1, 0 or 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_wide_color: Option<i8>,
    /// `supports_hdr`: -1, 0 or 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_hdr: Option<i8>,
    /// `sdr_min_luminance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_min_luminance: Option<f64>,
    /// `sdr_max_luminance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_max_luminance: Option<i64>,
    /// `min_luminance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_luminance: Option<f64>,
    /// `max_luminance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_luminance: Option<i64>,
    /// `max_avg_luminance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_avg_luminance: Option<i64>,
    /// `reserved` / `addreserved`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved: Option<Reserved>,
    /// Keys hyprtilt does not model, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<ExtraField>,
}

impl MonitorRule {
    /// A rule for `output` with no other field written.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::MonitorRule;
    ///
    /// let rule = MonitorRule::new("DP-1");
    /// assert_eq!(rule.output.as_str(), "DP-1");
    /// assert!(rule.mode.is_none());
    /// ```
    #[must_use]
    pub fn new(output: impl Into<String>) -> Self {
        MonitorRule {
            output: Selector::new(output),
            mode: None,
            position: None,
            scale: None,
            transform: None,
            disabled: None,
            vrr: None,
            mirror: None,
            bitdepth: None,
            cm: None,
            sdr_eotf: None,
            sdrbrightness: None,
            sdrsaturation: None,
            icc: None,
            supports_wide_color: None,
            supports_hdr: None,
            sdr_min_luminance: None,
            sdr_max_luminance: None,
            min_luminance: None,
            max_luminance: None,
            max_avg_luminance: None,
            reserved: None,
            extra: Vec::new(),
        }
    }

    /// Whether the rule disables its output.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled == Some(true)
    }

    /// The transform, or normal when not written.
    #[must_use]
    pub fn transform_or_default(&self) -> Transform {
        self.transform.unwrap_or_default()
    }

    /// The rule as it reads back after being written (refresh rate and
    /// scale rounded to the precision hyprtilt writes).
    #[must_use]
    pub fn normalized(&self) -> MonitorRule {
        let mut rule = self.clone();
        rule.mode = rule.mode.as_ref().map(Mode::normalized);
        rule.scale = rule.scale.map(Scale::normalized);
        rule
    }

    /// Apply a later rule for the same output the way Hyprland merges Lua
    /// rules: every field the later rule writes replaces this rule's
    /// value, every other field is kept. Unmodelled keys merge by name.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::model::{MonitorRule, Transform};
    ///
    /// let mut earlier = MonitorRule::new("DP-1");
    /// earlier.vrr = Some(2);
    /// earlier.transform = Some(Transform::new(1).unwrap());
    /// let mut later = MonitorRule::new("DP-1");
    /// later.transform = Some(Transform::NORMAL);
    /// earlier.overlay(&later);
    /// assert_eq!(earlier.vrr, Some(2));
    /// assert_eq!(earlier.transform, Some(Transform::NORMAL));
    /// ```
    pub fn overlay(&mut self, later: &MonitorRule) {
        fn take<T: Clone>(field: &mut Option<T>, later: Option<&T>) {
            if let Some(value) = later {
                *field = Some(value.clone());
            }
        }
        take(&mut self.mode, later.mode.as_ref());
        take(&mut self.position, later.position.as_ref());
        take(&mut self.scale, later.scale.as_ref());
        take(&mut self.transform, later.transform.as_ref());
        take(&mut self.disabled, later.disabled.as_ref());
        take(&mut self.vrr, later.vrr.as_ref());
        take(&mut self.mirror, later.mirror.as_ref());
        take(&mut self.bitdepth, later.bitdepth.as_ref());
        take(&mut self.cm, later.cm.as_ref());
        take(&mut self.sdr_eotf, later.sdr_eotf.as_ref());
        take(&mut self.sdrbrightness, later.sdrbrightness.as_ref());
        take(&mut self.sdrsaturation, later.sdrsaturation.as_ref());
        take(&mut self.icc, later.icc.as_ref());
        take(
            &mut self.supports_wide_color,
            later.supports_wide_color.as_ref(),
        );
        take(&mut self.supports_hdr, later.supports_hdr.as_ref());
        take(
            &mut self.sdr_min_luminance,
            later.sdr_min_luminance.as_ref(),
        );
        take(
            &mut self.sdr_max_luminance,
            later.sdr_max_luminance.as_ref(),
        );
        take(&mut self.min_luminance, later.min_luminance.as_ref());
        take(&mut self.max_luminance, later.max_luminance.as_ref());
        take(
            &mut self.max_avg_luminance,
            later.max_avg_luminance.as_ref(),
        );
        take(&mut self.reserved, later.reserved.as_ref());
        for field in &later.extra {
            match self.extra.iter_mut().find(|f| f.key == field.key) {
                Some(existing) => existing.raw.clone_from(&field.raw),
                None => self.extra.push(field.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_kinds_and_matching() {
        let desc = "Samsung Electric Company Odyssey G50F HNAYC01385";
        assert_eq!(Selector::new("").kind(), SelectorKind::Fallback);
        assert_eq!(
            Selector::new("desc:  Samsung ").kind(),
            SelectorKind::Description("Samsung")
        );
        assert!(Selector::new("desc:Samsung").matches("DP-1", desc));
        assert!(!Selector::new("desc:LG").matches("DP-1", desc));
        assert!(!Selector::new("DP-2").matches("DP-1", desc));
        // Commas are removed from the description before matching.
        assert!(Selector::new("desc:Acme Inc 27").matches("DP-1", "Acme, Inc 27"));
        assert_eq!(Selector::new("DP-1").to_string(), "DP-1");
    }

    #[test]
    fn mode_parsing_and_display() {
        let cases = [
            ("2560x1440@144", "2560x1440@144"),
            ("2560x1440@179.95", "2560x1440@179.95"),
            ("1920x1080@120Hz", "1920x1080@120"),
            ("3840x2160@60.0", "3840x2160@60"),
            ("1920x1080", "1920x1080"),
            ("preferred", "preferred"),
            ("pref", "preferred"),
            ("", "preferred"),
            ("highrr", "highrr"),
            ("highres", "highres"),
            ("maxwidth", "maxwidth"),
        ];
        for (input, shown) in cases {
            let m: Mode = input.parse().unwrap();
            assert_eq!(m.to_string(), shown, "{input}");
        }
        let modeline = "modeline 1071.101 3840 3848 3880 3920 2160 2263 2271 2277 +hsync -vsync";
        assert_eq!(
            modeline.parse::<Mode>().unwrap(),
            Mode::Modeline(modeline.to_owned())
        );
        for bad in [
            "1920",
            "axb",
            "0x1080",
            "1920x1080@",
            "1920x1080@-5",
            "wide",
        ] {
            assert!(bad.parse::<Mode>().is_err(), "{bad}");
        }
    }

    #[test]
    fn refresh_formatting() {
        assert_eq!(format_refresh(144.0), "144");
        assert_eq!(format_refresh(179.952), "179.95");
        assert_eq!(format_refresh(59.94), "59.94");
        assert_eq!(format_refresh(60.5), "60.5");
    }

    #[test]
    fn position_parsing_and_display() {
        for s in [
            "0x0",
            "1440x1335",
            "-1920x0",
            "0x-1080",
            "auto",
            "auto-right",
            "auto-center-down",
        ] {
            assert_eq!(s.parse::<Position>().unwrap().to_string(), s);
        }
        assert_eq!("".parse::<Position>().unwrap(), Position::Auto(None));
        for bad in ["auto-sideways", "10", "x10", "10x", "axb"] {
            assert!(bad.parse::<Position>().is_err(), "{bad}");
        }
    }

    #[test]
    fn scale_parsing() {
        assert_eq!("1".parse::<Scale>().unwrap(), Scale::Factor(1.0));
        assert_eq!("auto".parse::<Scale>().unwrap(), Scale::Auto);
        assert_eq!("".parse::<Scale>().unwrap(), Scale::Auto);
        assert!("0.2".parse::<Scale>().is_err());
        assert!("big".parse::<Scale>().is_err());
        assert!("NaN".parse::<Scale>().is_err());
        assert_eq!(Scale::Factor(1.5).to_string(), "1.5");
        assert_eq!(Scale::Factor(1.0).to_string(), "1");
        assert_eq!(Scale::Auto.to_string(), "auto");
    }

    #[test]
    fn transform_steps() {
        let all: Vec<u8> = (0..8).collect();
        for &v in &all {
            let t = Transform::new(v).unwrap();
            assert_eq!(t.rotated_right().rotated_left(), t);
            assert_eq!(t.flipped().flipped(), t);
            assert_eq!(t.rotated_right().is_flipped(), t.is_flipped());
            assert_eq!(t.swaps_axes(), v % 2 == 1);
        }
        let t0 = Transform::NORMAL;
        let four = t0
            .rotated_right()
            .rotated_right()
            .rotated_right()
            .rotated_right();
        assert_eq!(four, t0);
        assert_eq!(Transform::new(1).unwrap().degrees(), 90);
        assert_eq!(Transform::new(6).unwrap().degrees(), 180);
        assert!(Transform::new(8).is_err());
    }

    #[test]
    fn color_management_names() {
        for (name, cm) in CM_NAMES {
            assert_eq!(name.parse::<ColorManagement>().unwrap(), *cm);
            assert_eq!(cm.to_string(), *name);
        }
        assert!("sRGB".parse::<ColorManagement>().is_err());
    }

    #[test]
    fn serde_round_trip() {
        let mut rule = MonitorRule::new("HDMI-A-1");
        rule.mode = Some("2560x1440@144".parse().unwrap());
        rule.position = Some(Position::At { x: 0, y: 0 });
        rule.scale = Some(Scale::Factor(1.0));
        rule.transform = Some(Transform::new(1).unwrap());
        rule.vrr = Some(2);
        let json = serde_json::to_string(&rule).unwrap();
        assert_eq!(
            json,
            r#"{"output":"HDMI-A-1","mode":"2560x1440@144","position":"0x0","scale":1.0,"transform":1,"vrr":2}"#
        );
        let back: MonitorRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rule);

        let mut auto = MonitorRule::new("");
        auto.scale = Some(Scale::Auto);
        let json = serde_json::to_string(&auto).unwrap();
        assert_eq!(json, r#"{"output":"","scale":"auto"}"#);
        assert_eq!(serde_json::from_str::<MonitorRule>(&json).unwrap(), auto);
        assert!(serde_json::from_str::<MonitorRule>(r#"{"output":"x","transform":9}"#).is_err());
    }

    #[test]
    fn overlay_merges_like_lua() {
        let mut base = MonitorRule::new("DP-1");
        base.mode = Some("1920x1080@60".parse().unwrap());
        base.extra.push(ExtraField {
            key: "a".to_owned(),
            raw: "1".to_owned(),
        });
        let mut later = MonitorRule::new("DP-1");
        later.scale = Some(Scale::Factor(2.0));
        later.extra.push(ExtraField {
            key: "a".to_owned(),
            raw: "2".to_owned(),
        });
        later.extra.push(ExtraField {
            key: "b".to_owned(),
            raw: "3".to_owned(),
        });
        base.overlay(&later);
        assert_eq!(base.mode.unwrap().to_string(), "1920x1080@60");
        assert_eq!(base.scale, Some(Scale::Factor(2.0)));
        let extra: Vec<_> = base
            .extra
            .iter()
            .map(|f| (f.key.as_str(), f.raw.as_str()))
            .collect();
        assert_eq!(extra, [("a", "2"), ("b", "3")]);
    }

    #[test]
    fn normalization_is_idempotent() {
        let mut rule = MonitorRule::new("DP-1");
        rule.mode = Some(Mode::Resolution {
            width: 2560,
            height: 1440,
            refresh: Some(179.952_001),
        });
        rule.scale = Some(Scale::Factor(5.0 / 3.0));
        let once = rule.normalized();
        assert_eq!(once.normalized(), once);
        assert_eq!(once.scale.unwrap().to_string(), "1.666667");
        assert_eq!(Mode::Preferred.normalized(), Mode::Preferred);
        assert_eq!(Mode::Preferred.resolution(), None);
        assert_eq!(Mode::Preferred.refresh(), None);
    }

    #[test]
    fn rule_helpers() {
        let mut rule = MonitorRule::new("eDP-1");
        assert!(!rule.is_disabled());
        assert_eq!(rule.transform_or_default(), Transform::NORMAL);
        rule.disabled = Some(true);
        assert!(rule.is_disabled());
    }
}
