// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The monitors drawn to scale: where each one lands in terminal cells,
//! and the widget that draws them.

use std::collections::HashSet;

use hyprtilt_core::geometry::format_scale;
use hyprtilt_core::layout::{Layout, Problem};
use hyprtilt_core::model::format_refresh;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Position, Rect as Area};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::commands::list::transform_name;

/// A terminal cell is about twice as tall as it is wide.
const ROW: f64 = 2.0;

/// Where the monitors are on the screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placement {
    /// Each active output's index and cells; the selected one last.
    pub(crate) boxes: Vec<(usize, Area)>,
    /// Logical pixels per column; a row is [`ROW`] columns tall.
    pub(crate) per_column: f64,
}

impl Placement {
    /// The output drawn at a cell (the topmost one where boxes overlap).
    pub(crate) fn hit(&self, column: u16, row: u16) -> Option<usize> {
        self.boxes
            .iter()
            .rev()
            .find(|(_, cells)| cells.contains(Position::new(column, row)))
            .map(|(i, _)| *i)
    }

    /// The logical distance between two cells, rounded to `grid` pixels.
    pub(crate) fn delta(&self, from: (u16, u16), to: (u16, u16), grid: i32) -> (i32, i32) {
        let dx = (f64::from(to.0) - f64::from(from.0)) * self.per_column;
        let dy = (f64::from(to.1) - f64::from(from.1)) * self.per_column * ROW;
        let snap = |v: f64| (v / f64::from(grid)).round() as i32 * grid;
        (snap(dx), snap(dy))
    }
}

/// Fit the active outputs of `layout` into `area`, keeping proportions
/// and leaving a margin of one cell.
pub(crate) fn place(layout: &Layout, area: Area, selected: usize) -> Placement {
    let mut rects = layout.rects();
    let empty = Placement {
        boxes: Vec::new(),
        per_column: 1.0,
    };
    if rects.is_empty() || area.width < 6 || area.height < 4 {
        return empty;
    }
    let left = rects.iter().map(|(_, r)| r.x).min().unwrap_or(0);
    let top = rects.iter().map(|(_, r)| r.y).min().unwrap_or(0);
    let right = rects.iter().map(|(_, r)| r.right()).max().unwrap_or(0);
    let bottom = rects.iter().map(|(_, r)| r.bottom()).max().unwrap_or(0);
    let columns = f64::from(area.width - 2);
    let rows = f64::from(area.height - 2);
    let (width, height) = (f64::from(right - left), f64::from(bottom - top));
    let per_column = (width / columns).max(height / (rows * ROW)).max(1.0);
    let used_columns = ((width / per_column).round() as u16).min(area.width - 2);
    let used_rows = ((height / (per_column * ROW)).round() as u16).min(area.height - 2);
    let x0 = area.x + 1 + (area.width - 2 - used_columns) / 2;
    let y0 = area.y + 1 + (area.height - 2 - used_rows) / 2;
    let column = |x: i32| x0 + (f64::from(x - left) / per_column).round() as u16;
    let row = |y: i32| y0 + (f64::from(y - top) / (per_column * ROW)).round() as u16;
    // The selected output is drawn last, on top of any overlap.
    rects.sort_by_key(|(i, _)| *i == selected);
    let boxes = rects
        .into_iter()
        .filter_map(|(i, r)| {
            let (x, y) = (column(r.x), row(r.y));
            let w = (column(r.right()) - x).max(3);
            let h = (row(r.bottom()) - y).max(2);
            let cells = Area::new(x, y, w, h).intersection(area);
            (!cells.is_empty()).then_some((i, cells))
        })
        .collect();
    Placement { boxes, per_column }
}

/// The monitors as boxes with their name, mode, refresh rate, rotation
/// and scale.
pub(crate) struct Canvas<'a> {
    pub(crate) layout: &'a Layout,
    pub(crate) placement: &'a Placement,
    pub(crate) selected: usize,
}

impl Canvas<'_> {
    fn overlapping(&self) -> HashSet<String> {
        self.layout
            .problems()
            .into_iter()
            .filter_map(|p| match p {
                Problem::Overlap { a, b } => Some([a, b]),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn lines(&self, i: usize) -> Vec<Line<'static>> {
        let o = &self.layout.outputs[i];
        let (w, h) = self.layout.pixels(i);
        let mut lines = vec![
            Line::styled(
                o.info.name.clone(),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!("{w}x{h}")),
            Line::raw(format!("{} Hz", format_refresh(self.layout.refresh(i)))),
        ];
        let transform = o.rule.transform_or_default().value();
        if transform != 0 {
            lines.push(Line::raw(transform_name(transform)));
        }
        let scale = f64::from(self.layout.scale(i).scale);
        if (scale - 1.0).abs() > 1e-6 {
            lines.push(Line::raw(format!("x{}", format_scale(scale))));
        }
        lines
    }
}

impl Widget for Canvas<'_> {
    fn render(self, area: Area, buf: &mut Buffer) {
        if self.placement.boxes.is_empty() {
            Paragraph::new("no active monitor")
                .alignment(Alignment::Center)
                .render(area, buf);
            return;
        }
        let overlapping = self.overlapping();
        for (i, cells) in &self.placement.boxes {
            let name = &self.layout.outputs[*i].info.name;
            let selected = *i == self.selected;
            let color = if overlapping.contains(name) {
                Color::Red
            } else if selected {
                Color::Cyan
            } else {
                Color::Gray
            };
            let block = Block::bordered()
                .border_type(if selected {
                    BorderType::Thick
                } else {
                    BorderType::Plain
                })
                .border_style(Style::new().fg(color));
            let inner = block.inner(*cells);
            Clear.render(*cells, buf);
            block.render(*cells, buf);
            let mut text = self.lines(*i);
            text.truncate(usize::from(inner.height));
            Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(if selected {
                    Style::new().fg(Color::Cyan)
                } else {
                    Style::new()
                })
                .render(inner, buf);
        }
    }
}
