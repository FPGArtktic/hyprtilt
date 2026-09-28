// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Monitor geometry: logical sizes, Hyprland's scale snapping, layout
//! rectangles, reflow after rotation or scale changes, snapping, alignment,
//! and overlap and gap detection.
//!
//! The scale and size rules replicate Hyprland 0.56 exactly (see
//! `docs/hyprland-lua-api.md`, section 4): the requested scale is stored as
//! an `f32`, divisions happen in `f64`, the logical size is rounded half away
//! from zero, and a scale that does not give an integral logical size is
//! moved to a multiple of 1/120 that does. hyprtilt computes the same result
//! before writing, so what it shows is what Hyprland will do.
//!
//! Layout coordinates are logical pixels (scaled and transformed), the same
//! space as the `position` of a rule. Hyprland never fixes overlaps or gaps;
//! detecting them is hyprtilt's job.

use serde::Serialize;

use crate::model::Transform;

/// A rectangle in logical layout coordinates. Edges are half-open: two
/// rectangles that share an edge do not overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Rect {
    /// A rectangle from its top-left corner and size.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The right edge (exclusive).
    #[must_use]
    pub fn right(self) -> i32 {
        self.x.saturating_add(self.w)
    }

    /// The bottom edge (exclusive).
    #[must_use]
    pub fn bottom(self) -> i32 {
        self.y.saturating_add(self.h)
    }

    /// The same size at another position.
    #[must_use]
    pub fn at(self, x: i32, y: i32) -> Rect {
        Rect { x, y, ..self }
    }

    /// Whether the two rectangles share an area. Touching edges and corners
    /// do not count, exactly as in Hyprland's overlap check.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::geometry::Rect;
    ///
    /// let a = Rect::new(0, 0, 1440, 2560);
    /// assert!(!a.overlaps(Rect::new(1440, 1335, 1920, 1080)));
    /// assert!(a.overlaps(Rect::new(1439, 1335, 1920, 1080)));
    /// ```
    #[must_use]
    pub fn overlaps(self, other: Rect) -> bool {
        overlap_len(self.x, self.right(), other.x, other.right()) > 0
            && overlap_len(self.y, self.bottom(), other.y, other.bottom()) > 0
    }

    /// Whether the two rectangles share a piece of an edge of positive
    /// length, so that the pointer can move from one to the other. Corner
    /// contact does not count.
    ///
    /// # Examples
    ///
    /// ```
    /// use hyprtilt_core::geometry::Rect;
    ///
    /// let a = Rect::new(0, 0, 100, 100);
    /// assert!(a.touches(Rect::new(100, 50, 100, 100)));
    /// assert!(!a.touches(Rect::new(100, 100, 100, 100)));
    /// ```
    #[must_use]
    pub fn touches(self, other: Rect) -> bool {
        let vertical_edge = (self.right() == other.x || other.right() == self.x)
            && overlap_len(self.y, self.bottom(), other.y, other.bottom()) > 0;
        let horizontal_edge = (self.bottom() == other.y || other.bottom() == self.y)
            && overlap_len(self.x, self.right(), other.x, other.right()) > 0;
        vertical_edge || horizontal_edge
    }

    /// Squared distance between the closest points of the two rectangles;
    /// zero when they touch or overlap.
    #[must_use]
    pub fn gap_squared(self, other: Rect) -> i64 {
        let dx = i64::from(self.x.max(other.x)) - i64::from(self.right().min(other.right()));
        let dy = i64::from(self.y.max(other.y)) - i64::from(self.bottom().min(other.bottom()));
        let dx = dx.max(0);
        let dy = dy.max(0);
        dx * dx + dy * dy
    }
}

/// Length of the intersection of two half-open intervals, 0 if disjoint.
fn overlap_len(a0: i32, a1: i32, b0: i32, b1: i32) -> i64 {
    (i64::from(a1.min(b1)) - i64::from(a0.max(b0))).max(0)
}

/// The smallest rectangle containing all of `rects`, if there are any.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{bounds, Rect};
///
/// let b = bounds(&[Rect::new(0, 0, 10, 10), Rect::new(20, -5, 10, 10)]).unwrap();
/// assert_eq!(b, Rect::new(0, -5, 30, 15));
/// ```
#[must_use]
pub fn bounds(rects: &[Rect]) -> Option<Rect> {
    let first = rects.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.right(), first.bottom());
    for r in &rects[1..] {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.right());
        y1 = y1.max(r.bottom());
    }
    Some(Rect::new(
        x0,
        y0,
        x1.saturating_sub(x0),
        y1.saturating_sub(y0),
    ))
}

/// The pixel size after the transform: width and height swap for 90° and
/// 270°.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::transformed_size;
/// use hyprtilt_core::model::Transform;
///
/// let t = Transform::new(1).unwrap();
/// assert_eq!(transformed_size((2560, 1440), t), (1440, 2560));
/// ```
#[must_use]
pub fn transformed_size(pixels: (u32, u32), transform: Transform) -> (u32, u32) {
    if transform.swaps_axes() {
        (pixels.1, pixels.0)
    } else {
        pixels
    }
}

/// The logical size of a monitor: transformed pixel size divided by the
/// effective scale, each axis rounded half away from zero. This is
/// `m_size` in Hyprland (API §4.3).
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::logical_size;
/// use hyprtilt_core::model::Transform;
///
/// assert_eq!(logical_size((2560, 1440), Transform::NORMAL, 1.25), (2048, 1152));
/// assert_eq!(logical_size((2560, 1440), Transform::new(1).unwrap(), 1.0), (1440, 2560));
/// ```
#[must_use]
pub fn logical_size(pixels: (u32, u32), transform: Transform, scale: f32) -> (i32, i32) {
    let (w, h) = transformed_size(pixels, transform);
    let s = f64::from(scale);
    (
        (f64::from(w) / s).round() as i32,
        (f64::from(h) / s).round() as i32,
    )
}

/// How Hyprland arrived at the effective scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleFit {
    /// The requested scale gives an integral logical size and is used as is.
    Exact,
    /// The nearest multiple of 1/120 is valid; the change is silent.
    Rounded,
    /// A nearby multiple of 1/120 was chosen instead.
    Adjusted,
    /// No valid scale was found nearby; Hyprland used the fallback.
    Fallback,
}

/// The scale Hyprland really uses for a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EffectiveScale {
    /// The effective scale (`m_scale`), as Hyprland stores it.
    pub scale: f32,
    /// How it was derived from the requested scale.
    pub fit: ScaleFit,
    /// Whether Hyprland shows a warning or error for it. Only explicit
    /// scales that are adjusted or fall back do.
    pub notification: bool,
}

/// Whether `pixels / scale` is integral on both axes, compared exactly in
/// `f64` as Hyprland does.
// The exact comparison is the point: Hyprland's `Vector2D::operator==`
// compares exactly, and a tolerance would accept scales it rejects.
#[allow(clippy::float_cmp)]
fn divides(pixels: (u32, u32), scale: f64) -> bool {
    let x = f64::from(pixels.0) / scale;
    let y = f64::from(pixels.1) / scale;
    x == x.round() && y == y.round()
}

/// Replicate Hyprland's scale check in `CMonitor::applyMonitorRule`
/// (API §4.2) for an untransformed pixel size.
///
/// `requested` is the scale from the rule, or the automatic scale when
/// `auto` is set (see [`auto_scale`]). `fallback` is what Hyprland uses when
/// an explicit scale has no valid neighbour: its automatic scale.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{effective_scale, ScaleFit};
///
/// let s = effective_scale((2560, 1440), 1.5, false, 1.0);
/// assert!((s.scale - 1.6).abs() < 1e-6);
/// assert_eq!(s.fit, ScaleFit::Adjusted);
/// assert!(s.notification);
///
/// let s = effective_scale((1920, 1080), 1.25, false, 1.0);
/// assert_eq!((s.scale, s.fit), (1.25, ScaleFit::Exact));
/// ```
#[must_use]
pub fn effective_scale(
    pixels: (u32, u32),
    requested: f32,
    auto: bool,
    fallback: f32,
) -> EffectiveScale {
    let result = |scale: f32, fit: ScaleFit| EffectiveScale {
        scale,
        fit,
        notification: !auto && matches!(fit, ScaleFit::Adjusted | ScaleFit::Fallback),
    };
    if divides(pixels, f64::from(requested)) {
        return result(requested, ScaleFit::Exact);
    }
    // `float searchScale = std::round(m_scale * 120.0)`: computed in double,
    // stored in a float.
    let search = (f64::from(requested) * 120.0).round() as f32;
    let zero = f64::from(search) / 120.0;
    if divides(pixels, zero) {
        return result(zero as f32, ScaleFit::Rounded);
    }
    for i in 1..90u16 {
        // `searchScale + i` is float arithmetic, the division is double.
        let up = f64::from(search + f32::from(i)) / 120.0;
        let down = f64::from(search - f32::from(i)) / 120.0;
        if divides(pixels, up) {
            return result(up as f32, ScaleFit::Adjusted);
        }
        if divides(pixels, down) {
            return result(down as f32, ScaleFit::Adjusted);
        }
    }
    if auto {
        result(zero.round() as f32, ScaleFit::Fallback)
    } else {
        result(fallback, ScaleFit::Fallback)
    }
}

/// Hyprland's automatic scale (`getDefaultScale`, API §4.4): 2 above 200
/// pixels per inch, 1.5 above 140, 1 otherwise. A monitor that reports no
/// physical size counts as high density.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::auto_scale;
///
/// assert_eq!(auto_scale((1920, 1080), (340, 190)), 1.5);
/// assert_eq!(auto_scale((2560, 1440), (700, 400)), 1.0);
/// assert_eq!(auto_scale((3840, 2160), (0, 0)), 2.0);
/// ```
#[must_use]
pub fn auto_scale(pixels: (u32, u32), physical_mm: (u32, u32)) -> f32 {
    const MM_PER_INCH: f64 = 25.4;
    let diagonal_px = f64::from(pixels.0).hypot(f64::from(pixels.1));
    let diagonal_in =
        (f64::from(physical_mm.0) / MM_PER_INCH).hypot(f64::from(physical_mm.1) / MM_PER_INCH);
    let ppi = diagonal_px / diagonal_in;
    if ppi > 200.0 {
        2.0
    } else if ppi > 140.0 {
        1.5
    } else {
        1.0
    }
}

/// Every scale between `min` and `max` (inclusive) that Hyprland accepts
/// without changing it: multiples of 1/120 that give an integral logical
/// size. The TUI offers only these.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::valid_scales;
///
/// let s = valid_scales((2560, 1440), 1.0, 2.0);
/// assert_eq!(s.first(), Some(&1.0));
/// assert!(s.contains(&1.25));
/// assert!(!s.contains(&1.5));
/// assert_eq!(s.last(), Some(&2.0));
/// ```
#[must_use]
pub fn valid_scales(pixels: (u32, u32), min: f64, max: f64) -> Vec<f64> {
    let lo = (min * 120.0).ceil().max(30.0) as u32;
    let hi = (max * 120.0).floor() as u32;
    (lo..=hi)
        .map(|k| f64::from(k) / 120.0)
        .filter(|&s| divides(pixels, s))
        .collect()
}

/// Format a scale for a configuration file: integers without decimals,
/// other values with up to six decimals, which Hyprland rounds back to the
/// intended multiple of 1/120 silently.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::format_scale;
///
/// assert_eq!(format_scale(1.0), "1");
/// assert_eq!(format_scale(1.25), "1.25");
/// assert_eq!(format_scale(5.0 / 3.0), "1.666667");
/// ```
#[must_use]
pub fn format_scale(scale: f64) -> String {
    if (scale - scale.round()).abs() < 1e-9 {
        return format!("{}", scale.round() as i64);
    }
    let s = format!("{scale:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Pairs of indices of rectangles that overlap.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{overlapping_pairs, Rect};
///
/// let r = [Rect::new(0, 0, 10, 10), Rect::new(5, 5, 10, 10), Rect::new(10, 5, 5, 5)];
/// assert_eq!(overlapping_pairs(&r), [(0, 1), (1, 2)]);
/// ```
#[must_use]
pub fn overlapping_pairs(rects: &[Rect]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            if rects[i].overlaps(rects[j]) {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

/// Groups of rectangles connected by shared edges. A layout with more than
/// one group has a gap the pointer cannot cross.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{connected_groups, Rect};
///
/// let r = [Rect::new(0, 0, 10, 10), Rect::new(10, 0, 10, 10), Rect::new(30, 0, 10, 10)];
/// assert_eq!(connected_groups(&r), [vec![0, 1], vec![2]]);
/// ```
#[must_use]
pub fn connected_groups(rects: &[Rect]) -> Vec<Vec<usize>> {
    let mut group_of: Vec<Option<usize>> = vec![None; rects.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for start in 0..rects.len() {
        if group_of[start].is_some() {
            continue;
        }
        let id = groups.len();
        let mut members = vec![start];
        group_of[start] = Some(id);
        let mut next = 0;
        while next < members.len() {
            let current = members[next];
            next += 1;
            for other in 0..rects.len() {
                let linked =
                    rects[current].touches(rects[other]) || rects[current].overlaps(rects[other]);
                if group_of[other].is_none() && linked {
                    group_of[other] = Some(id);
                    members.push(other);
                }
            }
        }
        members.sort_unstable();
        groups.push(members);
    }
    groups
}

/// Move the neighbours of a monitor whose size changed with its top-left
/// corner fixed (after a rotation or a scale change), so that the layout
/// keeps its shape: monitors that touched its right edge move by the width
/// difference, monitors that touched its bottom edge by the height
/// difference, and so on transitively. Nothing else moves.
///
/// `rects` holds the current rectangles with the changed one already at its
/// new size; `old` is its previous rectangle. Returns the new rectangles of
/// the monitors that move.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{reflow, Rect};
///
/// // A portrait monitor turned back to landscape pushes its right
/// // neighbour, and the neighbour's neighbour, to the right.
/// let rects = [
///     Rect::new(0, 0, 2560, 1440),
///     Rect::new(1440, 1335, 1920, 1080),
///     Rect::new(3360, 975, 2560, 1440),
/// ];
/// let moves = reflow(&rects, 0, Rect::new(0, 0, 1440, 2560));
/// assert_eq!(moves, [(1, Rect::new(2560, 1335, 1920, 1080)), (2, Rect::new(4480, 975, 2560, 1440))]);
/// ```
#[must_use]
pub fn reflow(rects: &[Rect], changed: usize, old: Rect) -> Vec<(usize, Rect)> {
    let new = rects[changed];
    let dx = new.right() - old.right();
    let dy = new.bottom() - old.bottom();
    let mut shift = vec![(0i32, 0i32); rects.len()];
    if dx != 0 {
        for i in chain(rects, changed, old, Axis::Horizontal) {
            shift[i].0 = dx;
        }
    }
    if dy != 0 {
        for i in chain(rects, changed, old, Axis::Vertical) {
            shift[i].1 = dy;
        }
    }
    shift
        .iter()
        .enumerate()
        .filter(|&(_, &(sx, sy))| sx != 0 || sy != 0)
        .map(|(i, &(sx, sy))| (i, rects[i].at(rects[i].x + sx, rects[i].y + sy)))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// The monitors reached from `old` by following right (or bottom) edges,
/// using the positions before the change.
fn chain(rects: &[Rect], changed: usize, old: Rect, axis: Axis) -> Vec<usize> {
    let mut reached: Vec<usize> = Vec::new();
    let mut frontier = vec![old];
    while let Some(edge_owner) = frontier.pop() {
        for (i, r) in rects.iter().enumerate() {
            if i == changed || reached.contains(&i) {
                continue;
            }
            let adjacent = match axis {
                Axis::Horizontal => {
                    r.x == edge_owner.right()
                        && overlap_len(r.y, r.bottom(), edge_owner.y, edge_owner.bottom()) > 0
                }
                Axis::Vertical => {
                    r.y == edge_owner.bottom()
                        && overlap_len(r.x, r.right(), edge_owner.x, edge_owner.right()) > 0
                }
            };
            if adjacent {
                reached.push(i);
                frontier.push(*r);
            }
        }
    }
    reached
}

/// A direction of movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards smaller x.
    Left,
    /// Towards larger x.
    Right,
    /// Towards smaller y.
    Up,
    /// Towards larger y.
    Down,
}

impl Direction {
    /// The unit vector of the direction.
    #[must_use]
    pub fn delta(self) -> (i32, i32) {
        match self {
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
        }
    }
}

/// The nearest position in `dir`, at most `max` pixels away, where an edge
/// of `rect` lines up with an edge of one of `others`: left with left,
/// left with right, right with left or right with right (top and bottom
/// for vertical moves). Returns the new top-left corner.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{next_snap, Direction, Rect};
///
/// let me = Rect::new(0, 0, 100, 100);
/// let other = Rect::new(150, 0, 100, 100);
/// // Moving right, the first stop puts my right edge on its left edge.
/// assert_eq!(next_snap(me, &[other], Direction::Right, 500), Some((50, 0)));
/// assert_eq!(next_snap(me, &[other], Direction::Right, 10), None);
/// ```
#[must_use]
pub fn next_snap(rect: Rect, others: &[Rect], dir: Direction, max: i32) -> Option<(i32, i32)> {
    let horizontal = matches!(dir, Direction::Left | Direction::Right);
    let (pos, size) = if horizontal {
        (rect.x, rect.w)
    } else {
        (rect.y, rect.h)
    };
    let candidates = others.iter().flat_map(|o| {
        let (start, end) = if horizontal {
            (o.x, o.right())
        } else {
            (o.y, o.bottom())
        };
        [start, end, start - size, end - size]
    });
    let forward = matches!(dir, Direction::Right | Direction::Down);
    let best = candidates
        .filter(|&c| {
            let distance = if forward { c - pos } else { pos - c };
            distance > 0 && distance <= max
        })
        .min_by_key(|&c| (c - pos).abs())?;
    Some(if horizontal {
        (best, rect.y)
    } else {
        (rect.x, best)
    })
}

/// An alignment shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Top edges on the same line.
    Top,
    /// Bottom edges on the same line.
    Bottom,
    /// Vertical centres on the same line.
    Center,
}

/// The monitor closest to `rects[index]`, measured between their nearest
/// edges. Ties go to the one further left, then further up, so the choice
/// is stable.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{nearest, Rect};
///
/// let r = [Rect::new(0, 0, 10, 10), Rect::new(10, 0, 10, 10), Rect::new(50, 0, 10, 10)];
/// assert_eq!(nearest(&r, 2), Some(1));
/// assert_eq!(nearest(&r[..1], 0), None);
/// ```
#[must_use]
pub fn nearest(rects: &[Rect], index: usize) -> Option<usize> {
    let me = rects[index];
    rects
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != index)
        .min_by_key(|&(_, r)| (me.gap_squared(*r), r.x, r.y))
        .map(|(i, _)| i)
}

/// The y coordinate that aligns `rect` with `to`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::geometry::{aligned_y, Align, Rect};
///
/// let laptop = Rect::new(1440, 1335, 1920, 1080);
/// let portrait = Rect::new(0, 0, 1440, 2560);
/// assert_eq!(aligned_y(laptop, portrait, Align::Bottom), 1480);
/// assert_eq!(aligned_y(laptop, portrait, Align::Top), 0);
/// assert_eq!(aligned_y(laptop, portrait, Align::Center), 740);
/// ```
#[must_use]
pub fn aligned_y(rect: Rect, to: Rect, align: Align) -> i32 {
    match align {
        Align::Top => to.y,
        Align::Bottom => to.bottom() - rect.h,
        Align::Center => to.y + (to.h - rect.h).div_euclid(2),
    }
}

#[cfg(test)]
// Tests compare scales that are exact by construction.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// Pixels, requested scale, effective scale, fit, logical size.
    type ScaleCase = ((u32, u32), f32, f64, ScaleFit, (i32, i32));

    fn t(v: u8) -> Transform {
        Transform::new(v).unwrap()
    }

    /// The maintainer's layout (API §12.2): [HDMI-A-1 portrait] [eDP-1] [DP-1].
    fn maintainer_layout() -> [Rect; 3] {
        let hdmi = logical_size((2560, 1440), t(1), 1.0);
        let edp = logical_size((1920, 1080), t(0), 1.0);
        let dp = logical_size((2560, 1440), t(0), 1.0);
        [
            Rect::new(0, 0, hdmi.0, hdmi.1),
            Rect::new(1440, 1335, edp.0, edp.1),
            Rect::new(3360, 975, dp.0, dp.1),
        ]
    }

    #[test]
    fn maintainer_layout_is_asserted_as_data() {
        let [hdmi, edp, dp] = maintainer_layout();
        assert_eq!(hdmi, Rect::new(0, 0, 1440, 2560));
        assert_eq!(hdmi.bottom(), 2560);
        assert_eq!((edp.bottom(), dp.bottom()), (2415, 2415));
        assert_eq!((edp.x, edp.right()), (1440, 3360));
        assert_eq!((dp.x, dp.right()), (3360, 5920));
        // Neighbours share edges exactly: no overlap, no gap.
        assert!(overlapping_pairs(&maintainer_layout()).is_empty());
        assert_eq!(connected_groups(&maintainer_layout()), [vec![0, 1, 2]]);
        assert!(hdmi.touches(edp) && edp.touches(dp) && !hdmi.touches(dp));
        // Aligning HDMI-A-1's bottom edge with the others needs y = -145.
        assert_eq!(aligned_y(hdmi, edp, Align::Bottom), -145);
    }

    #[test]
    fn transforms_zero_to_seven() {
        for v in 0..8u8 {
            let size = logical_size((2560, 1440), t(v), 1.0);
            let expected = if v % 2 == 1 {
                (1440, 2560)
            } else {
                (2560, 1440)
            };
            assert_eq!(size, expected, "transform {v}");
        }
    }

    /// The worked examples of API §4.5, replicated against the real
    /// hyprutils there.
    #[test]
    fn scale_worked_examples() {
        let cases: &[ScaleCase] = &[
            ((2560, 1440), 1.25, 1.25, ScaleFit::Exact, (2048, 1152)),
            ((2560, 1440), 2.0, 2.0, ScaleFit::Exact, (1280, 720)),
            ((2560, 1440), 1.5, 1.6, ScaleFit::Adjusted, (1600, 900)),
            (
                (2560, 1440),
                1.333,
                4.0 / 3.0,
                ScaleFit::Rounded,
                (1920, 1080),
            ),
            (
                (2560, 1440),
                1.333_333,
                4.0 / 3.0,
                ScaleFit::Rounded,
                (1920, 1080),
            ),
            ((2560, 1440), 1.6, 1.6, ScaleFit::Rounded, (1600, 900)),
            (
                (2560, 1440),
                1.666_667,
                5.0 / 3.0,
                ScaleFit::Rounded,
                (1536, 864),
            ),
            (
                (2560, 1440),
                1.75,
                5.0 / 3.0,
                ScaleFit::Adjusted,
                (1536, 864),
            ),
            (
                (2560, 1440),
                1.8,
                5.0 / 3.0,
                ScaleFit::Adjusted,
                (1536, 864),
            ),
            (
                (2560, 1440),
                1.1,
                1.066_667,
                ScaleFit::Adjusted,
                (2400, 1350),
            ),
            ((2560, 1440), 1.2, 1.25, ScaleFit::Adjusted, (2048, 1152)),
            ((1920, 1080), 1.25, 1.25, ScaleFit::Exact, (1536, 864)),
            ((1920, 1080), 1.5, 1.5, ScaleFit::Exact, (1280, 720)),
            ((1920, 1080), 1.2, 1.2, ScaleFit::Rounded, (1600, 900)),
            (
                (1920, 1080),
                1.75,
                5.0 / 3.0,
                ScaleFit::Adjusted,
                (1152, 648),
            ),
            ((1920, 1080), 1.8, 1.875, ScaleFit::Adjusted, (1024, 576)),
        ];
        for &(pixels, requested, effective, fit, logical) in cases {
            let s = effective_scale(pixels, requested, false, 1.0);
            assert!(
                (f64::from(s.scale) - effective).abs() < 1e-5,
                "{pixels:?} @ {requested}: got {}",
                s.scale
            );
            assert_eq!(s.fit, fit, "{pixels:?} @ {requested}");
            assert_eq!(s.notification, fit == ScaleFit::Adjusted);
            assert_eq!(logical_size(pixels, Transform::NORMAL, s.scale), logical);
        }
        let rotated = effective_scale((2560, 1440), 1.0, false, 1.0);
        assert_eq!(
            logical_size((2560, 1440), t(1), rotated.scale),
            (1440, 2560)
        );
    }

    #[test]
    fn automatic_scales_never_notify() {
        let s = effective_scale((2560, 1440), 1.5, true, 1.0);
        assert_eq!(s.fit, ScaleFit::Adjusted);
        assert!(!s.notification);
    }

    #[test]
    fn scale_without_valid_neighbour_falls_back() {
        // With prime sides only divisors of 120 work, and none is within
        // 89/120 of 3.
        let s = effective_scale((1021, 1019), 3.0, false, 1.5);
        assert_eq!((s.scale, s.fit), (1.5, ScaleFit::Fallback));
        assert!(s.notification);
        let s = effective_scale((1021, 1019), 3.0, true, 1.5);
        assert_eq!(
            (s.scale, s.fit, s.notification),
            (3.0, ScaleFit::Fallback, false)
        );
        // Scale 1 always divides, so scales near it are never a fallback.
        assert_eq!(effective_scale((1021, 1019), 1.1, false, 1.5).scale, 1.0);
    }

    #[test]
    fn automatic_scale_by_density() {
        assert_eq!(auto_scale((3840, 2160), (344, 194)), 2.0);
        assert_eq!(auto_scale((1920, 1080), (340, 190)), 1.5);
        assert_eq!(auto_scale((2560, 1440), (700, 400)), 1.0);
        assert_eq!(auto_scale((0, 0), (0, 0)), 1.0);
    }

    #[test]
    fn valid_scales_are_fixed_points() {
        for pixels in [(2560, 1440), (1920, 1080), (3840, 2160), (2880, 1800)] {
            let scales = valid_scales(pixels, 0.25, 3.0);
            assert!(scales.contains(&1.0));
            for s in scales {
                let written: f32 = format_scale(s).parse().unwrap();
                let e = effective_scale(pixels, written, false, 1.0);
                assert!(!e.notification, "{pixels:?} {s}");
                assert!((f64::from(e.scale) - s).abs() < 1e-6, "{pixels:?} {s}");
            }
        }
        assert!(valid_scales((2560, 1440), 0.0, 0.1).is_empty());
    }

    #[test]
    fn scale_formatting() {
        assert_eq!(format_scale(2.0), "2");
        assert_eq!(format_scale(1.5), "1.5");
        assert_eq!(format_scale(4.0 / 3.0), "1.333333");
        assert_eq!(format_scale(1.066_666_666), "1.066667");
    }

    #[test]
    fn overlaps_and_touching() {
        let a = Rect::new(0, 0, 100, 100);
        assert!(!a.overlaps(Rect::new(100, 0, 10, 10)));
        assert!(!a.overlaps(Rect::new(100, 100, 10, 10)));
        assert!(a.overlaps(Rect::new(99, 99, 10, 10)));
        assert!(a.overlaps(Rect::new(10, 10, 10, 10)));
        assert!(a.touches(Rect::new(0, 100, 10, 10)));
        assert!(a.touches(Rect::new(-10, 0, 10, 10)));
        assert!(a.touches(Rect::new(0, -10, 10, 10)));
        assert!(!a.touches(Rect::new(-10, -10, 10, 10)));
        assert!(!a.touches(Rect::new(101, 0, 10, 10)));
        assert_eq!(a.gap_squared(Rect::new(103, 104, 1, 1)), 9 + 16);
        assert_eq!(a.gap_squared(Rect::new(50, 50, 1, 1)), 0);
    }

    #[test]
    fn bounds_of_nothing() {
        assert_eq!(bounds(&[]), None);
        assert_eq!(
            bounds(&maintainer_layout()),
            Some(Rect::new(0, 0, 5920, 2560))
        );
    }

    #[test]
    fn gaps_split_groups() {
        let mut r = maintainer_layout();
        r[2].x += 1;
        assert_eq!(connected_groups(&r), [vec![0, 1], vec![2]]);
        // Overlapping monitors are connected, too.
        r[2].x -= 2;
        assert_eq!(connected_groups(&r).len(), 1);
        assert_eq!(overlapping_pairs(&r), [(1, 2)]);
    }

    #[test]
    fn reflow_after_rotation_and_scale() {
        // Rotating the portrait monitor back pushes both neighbours.
        let mut r = maintainer_layout();
        let old = r[0];
        r[0] = Rect::new(0, 0, 2560, 1440);
        let moves = reflow(&r, 0, old);
        assert_eq!(
            moves,
            [
                (1, Rect::new(2560, 1335, 1920, 1080)),
                (2, Rect::new(4480, 975, 2560, 1440))
            ]
        );
        // Scaling the laptop up (smaller logical size) pulls DP-1 left only.
        let mut r = maintainer_layout();
        let old = r[1];
        r[1] = Rect::new(1440, 1335, 1280, 720);
        assert_eq!(reflow(&r, 1, old), [(2, Rect::new(2720, 975, 2560, 1440))]);
        // Nothing to move when the size is unchanged.
        let r = maintainer_layout();
        assert!(reflow(&r, 1, r[1]).is_empty());
    }

    #[test]
    fn reflow_moves_monitors_below() {
        let r = [Rect::new(0, 0, 1920, 1080), Rect::new(0, 1080, 1920, 1080)];
        let mut changed = r;
        changed[0] = Rect::new(0, 0, 1080, 1920);
        assert_eq!(
            reflow(&changed, 0, r[0]),
            [(1, Rect::new(0, 1920, 1920, 1080))]
        );
    }

    #[test]
    fn snapping_steps_through_edges() {
        let me = Rect::new(0, 0, 100, 100);
        let others = [Rect::new(150, 20, 100, 50)];
        assert_eq!(next_snap(me, &others, Direction::Right, 500), Some((50, 0)));
        assert_eq!(
            next_snap(me.at(50, 0), &others, Direction::Right, 500),
            Some((150, 0))
        );
        assert_eq!(
            next_snap(me.at(150, 0), &others, Direction::Right, 500),
            Some((250, 0))
        );
        assert_eq!(
            next_snap(me.at(250, 0), &others, Direction::Right, 500),
            None
        );
        assert_eq!(
            next_snap(me.at(250, 0), &others, Direction::Left, 500),
            Some((150, 0))
        );
        // Moving down, the top edges line up first.
        assert_eq!(next_snap(me, &others, Direction::Down, 500), Some((0, 20)));
        assert_eq!(next_snap(me, &others, Direction::Up, 500), Some((0, -30)));
        assert_eq!(
            next_snap(me.at(0, 50), &others, Direction::Up, 500),
            Some((0, 20))
        );
        assert_eq!(Direction::Up.delta(), (0, -1));
        assert_eq!(Direction::Left.delta(), (-1, 0));
    }

    #[test]
    fn alignment_uses_the_nearest_neighbour() {
        let r = maintainer_layout();
        // eDP-1 touches both; the one further left wins the tie.
        assert_eq!(nearest(&r, 1), Some(0));
        assert_eq!(nearest(&r, 2), Some(1));
        assert_eq!(aligned_y(r[2], r[1], Align::Bottom), 975);
        assert_eq!(aligned_y(r[2], r[1], Align::Top), 1335);
        assert_eq!(aligned_y(r[2], r[1], Align::Center), 1155);
        // Odd differences round down.
        assert_eq!(
            aligned_y(Rect::new(0, 0, 1, 3), Rect::new(0, 0, 1, 6), Align::Center),
            1
        );
    }
}
