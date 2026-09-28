// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Drawing the interface: title, canvas, details of the selected monitor,
//! status line, key hints and the dialogs.

use hyprtilt_core::apply::Phase;
use hyprtilt_core::geometry::{ScaleFit, format_scale, logical_size};
use hyprtilt_core::layout::Problem;
use hyprtilt_core::model::format_refresh;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect as Area};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{
    App, ChangeKind, Countdown, KEYS, Level, Overlay, Picker, ProfileDialog, vrr_name,
};
use super::canvas::{self, Canvas, Placement};
use crate::commands::list::transform_name;

/// The width of the details panel beside the canvas.
const DETAILS_WIDTH: u16 = 34;
/// Below this width the details go under the canvas.
const WIDE: u16 = 80;

const LABEL: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// Draw everything; returns where the monitors are, for the mouse.
pub(crate) fn draw(frame: &mut Frame, app: &App) -> Placement {
    let [title, main, status, hints] = Split::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (canvas_area, details_area) = if main.width >= WIDE {
        let [c, d] =
            Split::horizontal([Constraint::Fill(1), Constraint::Length(DETAILS_WIDTH)]).areas(main);
        (c, d)
    } else {
        let [c, d] = Split::vertical([
            Constraint::Fill(1),
            Constraint::Length((main.height / 2).min(10)),
        ])
        .areas(main);
        (c, d)
    };
    frame.render_widget(Paragraph::new(title_line(app)), title);
    let block = Block::bordered().title(" Layout ");
    let inner = block.inner(canvas_area);
    frame.render_widget(block, canvas_area);
    let placement = canvas::place(&app.layout, inner, app.selected);
    frame.render_widget(
        Canvas {
            layout: &app.layout,
            placement: &placement,
            selected: app.selected,
        },
        inner,
    );
    frame.render_widget(
        Paragraph::new(details(app))
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Monitor ")),
        details_area,
    );
    frame.render_widget(Paragraph::new(status_line(app)), status);
    frame.render_widget(Paragraph::new(hint_line()).style(LABEL), hints);
    overlay(frame, app);
    placement
}

fn title_line(app: &App) -> Line<'static> {
    let mut spans = vec![
        Span::styled(" hyprtilt ", BOLD.fg(Color::Cyan)),
        Span::raw(format!("{} ({})", app.file.path, app.file.backend)),
    ];
    let mut flag = |text: &str, color: Color| {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("[{text}]"), Style::new().fg(color)));
    };
    if app.modified() {
        flag("modified", Color::Yellow);
    }
    if app.live_unsaved {
        flag("live change not in the file", Color::Yellow);
    }
    if app.offline {
        flag("offline", Color::Red);
    }
    if !app.snap {
        flag("snapping off", Color::DarkGray);
    }
    Line::from(spans)
}

fn status_line(app: &App) -> Line<'static> {
    match &app.message {
        Some(m) => {
            let color = match m.level {
                Level::Info => Color::Reset,
                Level::Success => Color::Green,
                Level::Error => Color::Red,
            };
            Line::styled(format!(" {}", m.text), Style::new().fg(color))
        }
        None => Line::styled(" ? shows every key", LABEL),
    }
}

fn hint_line() -> Line<'static> {
    Line::raw(
        " Tab select  hjkl move  r rotate  m mode  [ ] Hz  s scale  a apply  w write  u undo  ? help  q quit",
    )
}

fn row(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9} "), LABEL),
        Span::raw(value.into()),
    ])
}

/// The details of the selected monitor.
fn details(app: &App) -> Vec<Line<'static>> {
    let layout = &app.layout;
    let Some(o) = layout.outputs.get(app.selected) else {
        return vec![Line::raw("no monitors")];
    };
    let i = app.selected;
    let mut lines = vec![Line::from(vec![
        Span::styled(o.info.name.clone(), BOLD),
        Span::styled(format!("  {} of {}", i + 1, layout.outputs.len()), LABEL),
    ])];
    if !o.info.description.is_empty() {
        lines.push(Line::styled(o.info.description.clone(), LABEL));
    }
    let (w, h) = layout.pixels(i);
    lines.push(row("Mode", format!("{w}x{h}")));
    let hz = layout.refresh(i);
    lines.push(Line::from(vec![
        Span::styled(format!("{:<9} ", "Refresh"), LABEL),
        Span::styled(format!("{} Hz", format_refresh(hz)), BOLD.fg(Color::Cyan)),
        Span::styled("  [ ]", LABEL),
    ]));
    let rates = layout.refresh_rates(i);
    if rates.len() > 1 {
        let mut spans = vec![Span::styled(format!("{:<9} ", "Rates"), LABEL)];
        for rate in rates {
            let style = if (rate - hz).abs() < 1.0 {
                BOLD.fg(Color::Cyan)
            } else {
                Style::new()
            };
            spans.push(Span::styled(format_refresh(rate), style));
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }
    if let Some(r) = layout.rect(i) {
        lines.push(row("Position", format!("{}, {}", r.x, r.y)));
    }
    let scale = layout.scale(i);
    let transform = o.rule.transform_or_default();
    let (lw, lh) = logical_size((w, h), transform, scale.scale);
    lines.push(row(
        "Scale",
        format!(
            "{}{} ({lw}x{lh})",
            format_scale(f64::from(scale.scale)),
            if o.rule
                .scale
                .is_none_or(|s| s == hyprtilt_core::model::Scale::Auto)
            {
                " auto"
            } else {
                ""
            }
        ),
    ));
    if matches!(scale.fit, ScaleFit::Adjusted | ScaleFit::Fallback) {
        lines.push(Line::styled(
            "          adjusted by Hyprland",
            Style::new().fg(Color::Yellow),
        ));
    }
    lines.push(row("Rotation", transform_name(transform.value())));
    lines.push(row("VRR", vrr_name(o.rule.vrr)));
    lines.push(row(
        "State",
        if o.rule.is_disabled() {
            "disabled"
        } else {
            "enabled"
        },
    ));
    lines.push(row(
        "Rule",
        match app.file.line_of(&o.rule) {
            Some((line, false)) => format!("line {line}"),
            Some((line, true)) => format!("line {line}, edited"),
            None => "new, not in the file".to_owned(),
        },
    ));
    if !app.offline {
        let live = &o.info;
        lines.push(row(
            "Running",
            if live.disabled {
                "disabled".to_owned()
            } else {
                format!(
                    "{}x{}@{} at {},{} x{}",
                    live.width,
                    live.height,
                    format_refresh(live.refresh_rate),
                    live.x,
                    live.y,
                    format_scale(live.scale)
                )
            },
        ));
    }
    lines.extend(layout_notes(&app.layout));
    lines
}

/// The problems of the whole layout and the rules for absent monitors.
fn layout_notes(layout: &hyprtilt_core::layout::Layout) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let problems = layout.problems();
    if !problems.is_empty() {
        lines.push(Line::raw(""));
    }
    for p in problems {
        let color = if p.is_blocking() {
            Color::Red
        } else {
            Color::Yellow
        };
        let text = match &p {
            Problem::Gap { .. } => format!("! {p}; the pointer cannot cross"),
            _ => format!("! {p}"),
        };
        lines.push(Line::styled(text, Style::new().fg(color)));
    }
    if !layout.detached.is_empty() {
        lines.push(Line::styled(
            format!(
                "{} rule(s) for monitors not connected",
                layout.detached.len()
            ),
            LABEL,
        ));
    }
    lines
}

/// A rectangle of at most `width` x `height` in the middle of `area`.
fn centered(area: Area, width: u16, height: u16) -> Area {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Area::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn popup(frame: &mut Frame, title: &str, lines: Vec<Line<'static>>, width: u16) {
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let area = centered(frame.area(), width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(format!(" {title} "))),
        area,
    );
}

fn overlay(frame: &mut Frame, app: &App) {
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => {
            let mut lines: Vec<Line<'static>> = KEYS
                .iter()
                .map(|(keys, what)| {
                    Line::from(vec![
                        Span::styled(format!("{keys:<22} "), BOLD),
                        Span::raw(*what),
                    ])
                })
                .collect();
            lines.push(Line::raw(""));
            lines.push(Line::styled("any key closes this help", LABEL));
            popup(frame, "Keys", lines, 84);
        }
        Overlay::Modes(picker) => picker_popup(frame, picker),
        Overlay::Scales(picker) => picker_popup(frame, picker),
        Overlay::Adopt(lines) => {
            let list: Vec<String> = lines.iter().map(ToString::to_string).collect();
            popup(
                frame,
                "Rules outside the block",
                vec![
                    Line::raw(format!(
                        "{} monitor rule(s) outside the managed block (line {}).",
                        lines.len(),
                        list.join(", ")
                    )),
                    Line::raw("Adopting moves them into the block, where hyprtilt edits them."),
                    Line::raw(""),
                    Line::styled("y adopt   n not now", BOLD),
                ],
                70,
            );
        }
        Overlay::Profiles(dialog) => profiles_popup(frame, dialog),
        Overlay::Quit { live } => {
            let mut lines = vec![
                Line::raw(if *live {
                    "A change applied live is not in the file."
                } else {
                    "The layout differs from the file."
                }),
                Line::raw(""),
                Line::styled("w write and quit   q quit without writing", BOLD),
            ];
            if *live {
                lines.push(Line::styled(
                    "r reload the file (drop the live change) and quit",
                    BOLD,
                ));
            }
            lines.push(Line::styled("Esc stay", BOLD));
            popup(frame, "Quit", lines, 60);
        }
        Overlay::Countdown(countdown) => countdown_popup(frame, countdown),
    }
}

fn picker_popup<T>(frame: &mut Frame, picker: &Picker<T>) {
    let height = u16::try_from(picker.items.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let area = centered(frame.area(), 48, height);
    let items: Vec<ListItem<'static>> = picker
        .items
        .iter()
        .map(|(label, _)| ListItem::new(label.clone()))
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(format!(" {} ", picker.title))
                .title_bottom(" Enter choose  Esc close "),
        )
        .highlight_style(BOLD.fg(Color::Black).bg(Color::Cyan))
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(picker.cursor));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(list, area, &mut state);
}

fn profiles_popup(frame: &mut Frame, dialog: &ProfileDialog) {
    let mut lines: Vec<Line<'static>> = if dialog.names.is_empty() {
        vec![Line::styled("no profiles yet", LABEL)]
    } else {
        dialog
            .names
            .iter()
            .enumerate()
            .map(|(k, name)| {
                if k == dialog.cursor && dialog.input.is_none() {
                    Line::styled(format!("> {name}"), BOLD.fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::raw(format!("  {name}"))
                }
            })
            .collect()
    };
    lines.push(Line::raw(""));
    match &dialog.input {
        Some(input) => {
            lines.push(Line::from(vec![
                Span::styled("Save as: ", BOLD),
                Span::raw(format!("{input}_")),
            ]));
            lines.push(Line::styled("Enter save   Esc back", LABEL));
        }
        None => lines.push(Line::styled(
            "Enter load   s save the layout as a profile   Esc close",
            LABEL,
        )),
    }
    popup(frame, "Profiles", lines, 60);
}

fn countdown_popup(frame: &mut Frame, countdown: &Countdown) {
    let what = match countdown.kind {
        ChangeKind::Live => "Applying the layout live",
        ChangeKind::Write => "Writing the file and reloading",
    };
    let lines = match countdown.phase {
        Phase::Confirming => {
            let seconds = countdown
                .remaining
                .map_or(0, |r| r.as_millis().div_ceil(1000));
            vec![
                Line::styled("Keep this layout?", BOLD),
                Line::raw(""),
                Line::styled(format!("Reverting in {seconds} s"), BOLD.fg(Color::Yellow)),
                Line::raw(""),
                Line::styled("y or Enter keep   n or Esc revert", BOLD),
            ]
        }
        Phase::RollingBack => vec![Line::raw("Rolling back...")],
        _ => vec![
            Line::raw(format!("{what}...")),
            Line::raw("Waiting for Hyprland to show it."),
            Line::styled("Esc revert", LABEL),
        ],
    };
    popup(frame, "Apply", lines, 46);
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
