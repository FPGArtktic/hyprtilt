// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The layout being edited: every connected output with its live state
//! and the rule hyprtilt writes for it.
//!
//! The configuration file is the source of truth: an output takes its rule
//! from the managed block when a block rule matches it, and a rule built
//! from its live state otherwise. Block rules for outputs that are not
//! connected are kept untouched. Every edit changes the rule; geometry is
//! derived from the rule the way Hyprland derives it (see [`crate::geometry`]).

use serde::Serialize;

use crate::geometry::{self, Align, Direction, EffectiveScale, Rect, ScaleFit};
use crate::ipc::MonitorInfo;
use crate::model::{Mode, MonitorRule, Position, Scale, SelectorKind, Transform, format_refresh};

/// Where an output's rule comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "index")]
pub enum Origin {
    /// The managed block, at this index among its rules.
    Block(usize),
    /// Built from the live state; not in the file yet.
    New,
}

/// One connected output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Output {
    /// What Hyprland reports.
    pub info: MonitorInfo,
    /// The rule hyprtilt writes for it.
    pub rule: MonitorRule,
    /// Where the rule comes from.
    pub origin: Origin,
}

/// The layout: connected outputs and block rules for absent ones.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Layout {
    /// Connected outputs, left to right, disabled ones last.
    pub outputs: Vec<Output>,
    /// Block rules that match no connected output, with their index.
    pub detached: Vec<(usize, MonitorRule)>,
}

/// Something wrong with a layout.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Problem {
    /// Two outputs overlap; Hyprland only warns, so hyprtilt refuses to
    /// write or apply it.
    Overlap {
        /// First output.
        a: String,
        /// Second output.
        b: String,
    },
    /// The outputs form separate groups that the pointer cannot cross.
    Gap {
        /// The outputs of each group.
        groups: Vec<Vec<String>>,
    },
    /// Hyprland will not use the requested scale.
    Scale {
        /// The output.
        output: String,
        /// Requested scale.
        requested: f64,
        /// The scale Hyprland uses.
        effective: f32,
        /// Whether Hyprland shows a warning about it.
        notification: bool,
    },
    /// The mode is not in the output's list; Hyprland will try a custom
    /// mode.
    CustomMode {
        /// The output.
        output: String,
        /// The mode.
        mode: String,
    },
}

impl Problem {
    /// Whether the problem forbids writing or applying the layout.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        matches!(self, Problem::Overlap { .. })
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Overlap { a, b } => write!(f, "{a} and {b} overlap"),
            Problem::Gap { groups } => {
                let groups: Vec<String> = groups.iter().map(|g| g.join(" + ")).collect();
                write!(f, "gap between {}", groups.join(" and "))
            }
            Problem::Scale {
                output,
                requested,
                effective,
                notification,
            } => {
                write!(
                    f,
                    "{output}: Hyprland uses scale {} instead of {}",
                    format_scale32(*effective),
                    geometry::format_scale(*requested)
                )?;
                if *notification {
                    f.write_str(" and shows a warning")?;
                }
                Ok(())
            }
            Problem::CustomMode { output, mode } => {
                write!(f, "{output}: {mode} is not in the monitor's mode list")
            }
        }
    }
}

fn format_scale32(scale: f32) -> String {
    geometry::format_scale(f64::from(scale))
}

/// Why an edit is not possible.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// No output with that name or selector.
    #[error("no output {0:?}")]
    NoSuchOutput(String),
    /// The output has no refresh rates to choose from.
    #[error("{0}: the monitor reports no modes for its resolution")]
    NoRates(String),
}

/// A refresh rate to set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefreshTarget {
    /// The available rate closest to this one.
    Hz(f64),
    /// The highest available rate.
    Max,
    /// The lowest available rate.
    Min,
}

/// The result of setting a refresh rate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RefreshChange {
    /// The rate before.
    pub from: Option<f64>,
    /// The rate written.
    pub to: f64,
    /// Whether no available mode is within 1 Hz, so Hyprland will try a
    /// custom mode.
    pub custom: bool,
}

/// What Hyprland should report for an output after a change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Expectation {
    /// Connector name.
    pub output: String,
    /// Whether it is enabled.
    pub enabled: bool,
    /// Pixel size of the mode, before the transform.
    pub pixels: Option<(u32, u32)>,
    /// Refresh rate.
    pub refresh: Option<f64>,
    /// Logical position.
    pub position: Option<(i32, i32)>,
    /// Effective scale.
    pub scale: Option<f32>,
    /// Transform.
    pub transform: Option<u8>,
}

/// A difference between an expectation and what Hyprland reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    /// The output.
    pub output: String,
    /// The field.
    pub field: &'static str,
    /// The expected value.
    pub expected: String,
    /// What Hyprland reports.
    pub observed: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} is {}, expected {}",
            self.output, self.field, self.observed, self.expected
        )
    }
}

/// Compare expectations with the monitors Hyprland reports. Modes match
/// within 1 pixel and 1 Hz, as Hyprland's own mode selection does; scales
/// within 0.001.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::layout::{compare, Expectation};
///
/// let e = Expectation {
///     output: "DP-1".into(), enabled: true, pixels: None, refresh: None,
///     position: Some((0, 0)), scale: None, transform: None,
/// };
/// let m = compare(&[e], &[]);
/// assert_eq!(m[0].field, "presence");
/// ```
#[must_use]
pub fn compare(expected: &[Expectation], observed: &[MonitorInfo]) -> Vec<Mismatch> {
    let mut out = Vec::new();
    for e in expected {
        let mut mismatch = |field: &'static str, expected: String, observed: String| {
            out.push(Mismatch {
                output: e.output.clone(),
                field,
                expected,
                observed,
            });
        };
        let Some(m) = observed.iter().find(|m| m.name == e.output) else {
            mismatch(
                "presence",
                "connected".to_owned(),
                "not reported".to_owned(),
            );
            continue;
        };
        if e.enabled == m.disabled {
            let state = |on: bool| if on { "enabled" } else { "disabled" }.to_owned();
            mismatch("state", state(e.enabled), state(!m.disabled));
            continue;
        }
        if !e.enabled {
            continue;
        }
        if let Some((w, h)) = e.pixels
            && (w.abs_diff(m.width) > 1 || h.abs_diff(m.height) > 1)
        {
            mismatch(
                "mode",
                format!("{w}x{h}"),
                format!("{}x{}", m.width, m.height),
            );
        }
        if let Some(hz) = e.refresh
            && (hz - m.refresh_rate).abs() >= 1.0
        {
            mismatch(
                "refresh rate",
                format!("{} Hz", format_refresh(hz)),
                format!("{} Hz", format_refresh(m.refresh_rate)),
            );
        }
        if let Some((x, y)) = e.position
            && (x, y) != (m.x, m.y)
        {
            mismatch("position", format!("{x}x{y}"), format!("{}x{}", m.x, m.y));
        }
        if let Some(s) = e.scale
            && (f64::from(s) - m.scale).abs() > 1e-3
        {
            mismatch("scale", format_scale32(s), geometry::format_scale(m.scale));
        }
        if let Some(t) = e.transform
            && t != m.transform
        {
            mismatch("transform", t.to_string(), m.transform.to_string());
        }
    }
    out
}

/// Refresh rates that differ by less than this are the same rate.
const SAME_RATE: f64 = 0.005;

impl Layout {
    /// The layout of the connected `monitors` with the rules of the
    /// managed block. With `prefer_descriptions`, rules created for
    /// outputs without one use `desc:` selectors when the description is
    /// unique among the connected monitors.
    #[must_use]
    pub fn new(
        monitors: &[MonitorInfo],
        block: &[MonitorRule],
        prefer_descriptions: bool,
    ) -> Layout {
        let mut claimed = vec![false; block.len()];
        let mut outputs: Vec<Output> = monitors
            .iter()
            .map(|m| {
                // The last matching rule wins, as in Hyprland; a rule
                // serves only the first output it matches, so editing one
                // output never changes another.
                let found = block
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(i, r)| !claimed[*i] && r.output.matches(&m.name, &m.description));
                if let Some((i, rule)) = found {
                    claimed[i] = true;
                    return Output {
                        info: m.clone(),
                        rule: rule.clone(),
                        origin: Origin::Block(i),
                    };
                }
                let unique = monitors
                    .iter()
                    .filter(|o| o.description == m.description)
                    .count()
                    == 1;
                Output {
                    info: m.clone(),
                    rule: rule_from_live(m, monitors, prefer_descriptions && unique),
                    origin: Origin::New,
                }
            })
            .collect();
        outputs.sort_by_key(|o| (o.rule.is_disabled(), o.info.x, o.info.y));
        let detached = block
            .iter()
            .enumerate()
            .filter(|(i, _)| !claimed[*i])
            .map(|(i, r)| (i, r.clone()))
            .collect();
        Layout { outputs, detached }
    }

    /// A layout from the block alone, for editing without Hyprland: every
    /// rule with an explicit resolution becomes an output.
    #[must_use]
    pub fn offline(block: &[MonitorRule]) -> Layout {
        let mut monitors = Vec::new();
        for (id, rule) in block.iter().enumerate() {
            let Some(Mode::Resolution {
                width,
                height,
                refresh,
            }) = rule.mode.clone()
            else {
                continue;
            };
            let (x, y) = match rule.position {
                Some(Position::At { x, y }) => (x, y),
                _ => (0, 0),
            };
            let name = match rule.output.kind() {
                SelectorKind::Description(d) => d.to_owned(),
                _ => rule.output.to_string(),
            };
            let refresh = refresh.unwrap_or(60.0);
            monitors.push(MonitorInfo {
                id: id as i64,
                name,
                description: String::new(),
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                width,
                height,
                physical_width: 0,
                physical_height: 0,
                refresh_rate: refresh,
                x,
                y,
                active_workspace: None,
                scale: match rule.scale {
                    Some(Scale::Factor(f)) => f,
                    _ => 1.0,
                },
                transform: rule.transform_or_default().value(),
                focused: false,
                vrr: false,
                disabled: rule.is_disabled(),
                current_format: String::new(),
                mirror_of: "none".to_owned(),
                available_modes: vec![format!("{width}x{height}@{refresh:.2}Hz")],
                color_management_preset: String::new(),
            });
        }
        let mut layout = Layout::new(&[], block, false);
        layout.detached.clear();
        let mut used = vec![false; block.len()];
        for m in monitors {
            let index = usize::try_from(m.id).unwrap_or(0);
            used[index] = true;
            layout.outputs.push(Output {
                rule: block[index].clone(),
                info: m,
                origin: Origin::Block(index),
            });
        }
        layout.detached = block
            .iter()
            .enumerate()
            .filter(|(i, _)| !used[*i])
            .map(|(i, r)| (i, r.clone()))
            .collect();
        layout
    }

    /// The rules for the managed block: rules from the block in their
    /// order (with the edits), then the rules of new outputs.
    #[must_use]
    pub fn rules(&self) -> Vec<MonitorRule> {
        let mut from_block: Vec<(usize, MonitorRule)> = self
            .outputs
            .iter()
            .filter_map(|o| match o.origin {
                Origin::Block(i) => Some((i, o.rule.clone())),
                Origin::New => None,
            })
            .chain(self.detached.iter().cloned())
            .collect();
        from_block.sort_by_key(|(i, _)| *i);
        let mut rules: Vec<MonitorRule> = from_block.into_iter().map(|(_, r)| r).collect();
        rules.extend(
            self.outputs
                .iter()
                .filter(|o| o.origin == Origin::New)
                .map(|o| o.rule.clone()),
        );
        rules
    }

    /// The output with this connector name, rule selector or description.
    #[must_use]
    pub fn find(&self, query: &str) -> Option<usize> {
        let q = query.strip_prefix("desc:").unwrap_or(query).trim();
        self.outputs
            .iter()
            .position(|o| o.info.name == query || o.rule.output.as_str() == query)
            .or_else(|| {
                self.outputs
                    .iter()
                    .position(|o| !q.is_empty() && o.info.description.starts_with(q))
            })
    }

    /// Like [`Layout::find`], with an error.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::NoSuchOutput`].
    pub fn index(&self, query: &str) -> Result<usize, LayoutError> {
        self.find(query)
            .ok_or_else(|| LayoutError::NoSuchOutput(query.to_owned()))
    }

    /// The pixel size of the output's mode, before the transform. For
    /// `preferred` and the other keywords it is the live size when the
    /// output runs that rule, otherwise the first listed mode.
    #[must_use]
    pub fn pixels(&self, i: usize) -> (u32, u32) {
        let o = &self.outputs[i];
        if let Some(size) = o.rule.mode.as_ref().and_then(Mode::resolution) {
            return size;
        }
        if o.info.width > 0 && o.info.height > 0 {
            return o.info.pixels();
        }
        o.info
            .modes()
            .first()
            .and_then(Mode::resolution)
            .unwrap_or((1920, 1080))
    }

    /// The refresh rate the rule asks for, or the live one.
    #[must_use]
    pub fn refresh(&self, i: usize) -> f64 {
        let o = &self.outputs[i];
        o.rule
            .mode
            .as_ref()
            .and_then(Mode::refresh)
            .unwrap_or(o.info.refresh_rate)
    }

    /// The scale Hyprland will use for the output.
    #[must_use]
    pub fn scale(&self, i: usize) -> EffectiveScale {
        let o = &self.outputs[i];
        let pixels = self.pixels(i);
        let auto = geometry::auto_scale(pixels, (o.info.physical_width, o.info.physical_height));
        match o.rule.scale {
            Some(Scale::Factor(f)) => geometry::effective_scale(pixels, f as f32, false, auto),
            _ => geometry::effective_scale(pixels, auto, true, auto),
        }
    }

    /// Whether the output takes part in the layout (enabled and not a
    /// mirror).
    #[must_use]
    pub fn is_active(&self, i: usize) -> bool {
        let rule = &self.outputs[i].rule;
        !rule.is_disabled() && rule.mirror.as_deref().is_none_or(str::is_empty)
    }

    /// The output's rectangle in logical coordinates, if it is active.
    #[must_use]
    pub fn rect(&self, i: usize) -> Option<Rect> {
        if !self.is_active(i) {
            return None;
        }
        let output = &self.outputs[i];
        let (width, height) = geometry::logical_size(
            self.pixels(i),
            output.rule.transform_or_default(),
            self.scale(i).scale,
        );
        let (left, top) = match output.rule.position {
            Some(Position::At { x, y }) => (x, y),
            _ => (output.info.x, output.info.y),
        };
        Some(Rect::new(left, top, width, height))
    }

    /// The rectangles of the active outputs, with their index.
    #[must_use]
    pub fn rects(&self) -> Vec<(usize, Rect)> {
        (0..self.outputs.len())
            .filter_map(|i| self.rect(i).map(|r| (i, r)))
            .collect()
    }

    /// Everything wrong with the layout.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let rects = self.rects();
        let only: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let name = |k: usize| self.outputs[rects[k].0].info.name.clone();
        let mut problems: Vec<Problem> = geometry::overlapping_pairs(&only)
            .into_iter()
            .map(|(a, b)| Problem::Overlap {
                a: name(a),
                b: name(b),
            })
            .collect();
        let groups = geometry::connected_groups(&only);
        if groups.len() > 1 {
            problems.push(Problem::Gap {
                groups: groups
                    .iter()
                    .map(|g| g.iter().map(|&k| name(k)).collect())
                    .collect(),
            });
        }
        for (i, o) in self.outputs.iter().enumerate() {
            if !self.is_active(i) {
                continue;
            }
            let s = self.scale(i);
            if let (Some(Scale::Factor(requested)), ScaleFit::Adjusted | ScaleFit::Fallback) =
                (o.rule.scale, s.fit)
            {
                problems.push(Problem::Scale {
                    output: o.info.name.clone(),
                    requested,
                    effective: s.scale,
                    notification: s.notification,
                });
            }
            if let Some(mode @ Mode::Resolution { .. }) = &o.rule.mode {
                let listed = o.info.modes().iter().any(|m| same_mode(m, mode));
                if !listed && !o.info.available_modes.is_empty() {
                    problems.push(Problem::CustomMode {
                        output: o.info.name.clone(),
                        mode: mode.to_string(),
                    });
                }
            }
        }
        problems
    }

    /// Move an output's top-left corner.
    pub fn move_to(&mut self, i: usize, x: i32, y: i32) {
        self.outputs[i].rule.position = Some(Position::At { x, y });
    }

    /// Move an output by `step` pixels in `dir`; with `snap`, stop at the
    /// next edge of a neighbour when one is within `snap_range`.
    pub fn nudge(&mut self, i: usize, dir: Direction, step: i32, snap: bool, snap_range: i32) {
        let Some(me) = self.rect(i) else {
            return;
        };
        let target = if snap {
            let others: Vec<Rect> = self
                .rects()
                .into_iter()
                .filter(|(j, _)| *j != i)
                .map(|(_, r)| r)
                .collect();
            geometry::next_snap(me, &others, dir, snap_range)
        } else {
            None
        };
        let (dx, dy) = dir.delta();
        let (x, y) = target.unwrap_or((me.x + dx * step, me.y + dy * step));
        self.move_to(i, x, y);
    }

    /// Apply `change` to output `i` and move its neighbours if its size
    /// changed, so that the layout keeps its shape.
    fn resize_with(&mut self, i: usize, change: impl FnOnce(&mut MonitorRule)) {
        let before = self.rect(i);
        change(&mut self.outputs[i].rule);
        let (Some(old), Some(new)) = (before, self.rect(i)) else {
            return;
        };
        self.move_to(i, old.x, old.y);
        if (old.w, old.h) == (new.w, new.h) {
            return;
        }
        let active = self.rects();
        let rects: Vec<Rect> = active.iter().map(|(_, r)| *r).collect();
        let Some(me) = active.iter().position(|(j, _)| *j == i) else {
            return;
        };
        for (k, r) in geometry::reflow(&rects, me, old) {
            self.move_to(active[k].0, r.x, r.y);
        }
    }

    /// Set the transform.
    pub fn set_transform(&mut self, i: usize, t: Transform) {
        self.resize_with(i, |r| r.transform = Some(t));
    }

    /// Rotate by 90°: clockwise (the physical monitor turned right) or
    /// counter-clockwise, keeping a flip.
    pub fn rotate(&mut self, i: usize, clockwise: bool) {
        let t = self.outputs[i].rule.transform_or_default();
        self.set_transform(
            i,
            if clockwise {
                t.rotated_right()
            } else {
                t.rotated_left()
            },
        );
    }

    /// Toggle the flip.
    pub fn flip(&mut self, i: usize) {
        let t = self.outputs[i].rule.transform_or_default();
        self.set_transform(i, t.flipped());
    }

    /// Set the scale, as it reads back after writing.
    pub fn set_scale(&mut self, i: usize, scale: Scale) {
        self.resize_with(i, |r| r.scale = Some(scale.normalized()));
    }

    /// Set the mode, as it reads back after writing.
    pub fn set_mode(&mut self, i: usize, mode: &Mode) {
        let mode = mode.normalized();
        self.resize_with(i, |r| r.mode = Some(mode));
    }

    /// The modes the monitor offers, without duplicates.
    #[must_use]
    pub fn modes(&self, i: usize) -> Vec<Mode> {
        self.outputs[i]
            .info
            .modes()
            .into_iter()
            .map(|m| m.normalized())
            .collect()
    }

    /// The refresh rates available for the output's current resolution,
    /// lowest first.
    #[must_use]
    pub fn refresh_rates(&self, i: usize) -> Vec<f64> {
        let (w, h) = self.pixels(i);
        let mut rates: Vec<f64> = self
            .modes(i)
            .iter()
            .filter(|m| m.resolution() == Some((w, h)))
            .filter_map(Mode::refresh)
            .collect();
        rates.sort_by(f64::total_cmp);
        rates.dedup_by(|a, b| (*a - *b).abs() < SAME_RATE);
        rates
    }

    /// Set the refresh rate, keeping the resolution. The rate is taken
    /// from the available modes when one is within 1 Hz.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::NoRates`] for `Max` or `Min` when the monitor
    /// lists no mode for its resolution.
    pub fn set_refresh(
        &mut self,
        i: usize,
        target: RefreshTarget,
    ) -> Result<RefreshChange, LayoutError> {
        let rates = self.refresh_rates(i);
        let no_rates = || LayoutError::NoRates(self.outputs[i].info.name.clone());
        let (to, custom) = match target {
            RefreshTarget::Max => (*rates.last().ok_or_else(no_rates)?, false),
            RefreshTarget::Min => (*rates.first().ok_or_else(no_rates)?, false),
            RefreshTarget::Hz(hz) => match rates
                .iter()
                .min_by(|a, b| (*a - hz).abs().total_cmp(&(*b - hz).abs()))
            {
                Some(&near) if (near - hz).abs() < 1.0 => (near, false),
                _ => (hz, true),
            },
        };
        let from = self.outputs[i].rule.mode.as_ref().and_then(Mode::refresh);
        let (width, height) = self.pixels(i);
        self.set_mode(
            i,
            &Mode::Resolution {
                width,
                height,
                refresh: Some(to),
            },
        );
        Ok(RefreshChange { from, to, custom })
    }

    /// Step to the next higher or lower available refresh rate; `None` at
    /// the end of the list.
    pub fn step_refresh(&mut self, i: usize, up: bool) -> Option<f64> {
        let rates = self.refresh_rates(i);
        let now = self.refresh(i);
        let next = if up {
            rates.iter().find(|&&r| r > now + SAME_RATE).copied()
        } else {
            rates.iter().rev().find(|&&r| r < now - SAME_RATE).copied()
        }?;
        self.set_refresh(i, RefreshTarget::Hz(next))
            .ok()
            .map(|c| c.to)
    }

    /// The scales Hyprland keeps unchanged for the output's mode, 0.5 to 3.
    #[must_use]
    pub fn scales(&self, i: usize) -> Vec<f64> {
        geometry::valid_scales(self.pixels(i), 0.5, 3.0)
    }

    /// Enable or disable an output. Enabling removes `disabled` from the
    /// rule, so that the file says nothing it does not need to.
    pub fn set_enabled(&mut self, i: usize, enabled: bool) {
        self.outputs[i].rule.disabled = if enabled { None } else { Some(true) };
    }

    /// Cycle VRR: unset, off, on, fullscreen, fullscreen games and video,
    /// back to unset. Returns the new value.
    pub fn cycle_vrr(&mut self, i: usize) -> Option<i8> {
        let rule = &mut self.outputs[i].rule;
        rule.vrr = match rule.vrr {
            None | Some(-1) => Some(0),
            Some(v @ 0..=2) => Some(v + 1),
            Some(_) => None,
        };
        rule.vrr
    }

    /// Align the output vertically with its nearest neighbour. Returns
    /// whether it moved.
    pub fn align(&mut self, i: usize, align: Align) -> bool {
        let active = self.rects();
        let rects: Vec<Rect> = active.iter().map(|(_, r)| *r).collect();
        let Some(me) = active.iter().position(|(j, _)| *j == i) else {
            return false;
        };
        let Some(other) = geometry::nearest(&rects, me) else {
            return false;
        };
        let y = geometry::aligned_y(rects[me], rects[other], align);
        if y == rects[me].y {
            return false;
        }
        self.move_to(i, rects[me].x, y);
        true
    }

    /// The complete rule for a live change: every field hyprtilt owns is
    /// written, because Lua merges a live rule into the existing one.
    #[must_use]
    pub fn live_rule(&self, i: usize) -> MonitorRule {
        let o = &self.outputs[i];
        let mut rule = o.rule.clone();
        if rule.is_disabled() {
            return rule;
        }
        rule.disabled = Some(false);
        rule.mode.get_or_insert(Mode::Preferred);
        if rule.position.is_none() || matches!(rule.position, Some(Position::Auto(_))) {
            let r = self.rect(i);
            rule.position = r.map(|r| Position::At { x: r.x, y: r.y });
        }
        rule.scale.get_or_insert(Scale::Auto);
        rule.transform.get_or_insert(Transform::NORMAL);
        rule
    }

    /// What Hyprland should report for each output after applying.
    #[must_use]
    pub fn expectations(&self) -> Vec<Expectation> {
        (0..self.outputs.len())
            .map(|i| {
                let o = &self.outputs[i];
                let enabled = !o.rule.is_disabled();
                let explicit = o.rule.mode.as_ref().and_then(Mode::resolution).is_some();
                let active = self.is_active(i);
                Expectation {
                    output: o.info.name.clone(),
                    enabled,
                    pixels: explicit.then(|| self.pixels(i)),
                    refresh: o.rule.mode.as_ref().and_then(Mode::refresh),
                    position: match o.rule.position {
                        Some(Position::At { x, y }) if active => Some((x, y)),
                        _ => None,
                    },
                    scale: active.then(|| self.scale(i).scale),
                    transform: Some(o.rule.transform_or_default().value()),
                }
            })
            .collect()
    }
}

/// Whether two modes are the same resolution and refresh rate.
fn same_mode(a: &Mode, b: &Mode) -> bool {
    a.resolution() == b.resolution()
        && match (a.refresh(), b.refresh()) {
            (Some(x), Some(y)) => (x - y).abs() < 1.0,
            _ => true,
        }
}

/// A rule that describes the live state of a monitor.
fn rule_from_live(m: &MonitorInfo, all: &[MonitorInfo], description: bool) -> MonitorRule {
    let selector =
        if description && !m.description.is_empty() && !m.description.contains(['$', '#', ',']) {
            format!("desc:{}", m.description)
        } else {
            m.name.clone()
        };
    let mut rule = MonitorRule::new(selector);
    let (width, height) = if m.width > 0 && m.height > 0 {
        m.pixels()
    } else {
        m.modes()
            .first()
            .and_then(Mode::resolution)
            .unwrap_or((1920, 1080))
    };
    rule.mode = Some(
        Mode::Resolution {
            width,
            height,
            refresh: Some(m.refresh_rate),
        }
        .normalized(),
    );
    rule.position = Some(Position::At { x: m.x, y: m.y });
    rule.scale = Some(Scale::Factor(m.scale).normalized());
    rule.transform = Transform::new(m.transform).ok();
    if m.disabled {
        rule.disabled = Some(true);
    }
    rule.mirror = m.mirror_name(all).map(str::to_owned);
    rule
}

#[cfg(test)]
mod tests;
