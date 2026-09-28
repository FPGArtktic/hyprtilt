// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The terminal interface: `hyprtilt` without a subcommand.
//!
//! [`app`] holds the state and the keys, [`controller`] carries out what
//! they ask for, [`view`] and [`canvas`] draw it. This module owns the
//! terminal and the event loop: keys and the mouse, a clock tick every
//! 50 ms, hotplug events from Hyprland's event socket, and signals.

mod app;
mod canvas;
mod controller;
#[cfg(test)]
mod testing;
mod view;

use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use hyprtilt_core::ipc::events::{self, Event as HyprEvent};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use self::app::App;
use self::controller::{Controller, Env, Flow};
use crate::context::Context;
use crate::error::AppError;

/// How often the clock ticks.
const TICK: Duration = Duration::from_millis(50);

/// Run the terminal interface.
pub(crate) fn run(ctx: &Context) -> Result<(), AppError> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(AppError::Other(
            "the terminal interface needs a terminal; `hyprtilt --help` lists the commands"
                .to_owned(),
        ));
    }
    let mut controller = Controller::new(Env::from_context(ctx)?);
    let mut app = controller.load()?;
    let hotplug = ctx
        .event_socket()
        .and_then(|socket| events::listen(&socket).ok());
    let signal = Arc::new(AtomicBool::new(false));
    for sig in [SIGINT, SIGTERM, SIGHUP] {
        // Without the handler a signal ends the process without rolling
        // back; failing to install it leaves that default.
        let _ = signal_hook::flag::register(sig, Arc::clone(&signal));
    }
    let mut terminal = ratatui::try_init()?;
    // Without mouse capture the interface still works from the keyboard.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = event_loop(
        &mut terminal,
        &mut controller,
        &mut app,
        hotplug.as_ref(),
        &signal,
    );
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    let _ = std::io::stdout().flush();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    controller: &mut Controller<'_>,
    app: &mut App,
    hotplug: Option<&Receiver<HyprEvent>>,
    signal: &AtomicBool,
) -> Result<(), AppError> {
    let start = Instant::now();
    loop {
        let mut placement = None;
        terminal.draw(|frame| placement = Some(view::draw(frame, app)))?;
        app.placement = placement;
        if signal.load(Ordering::Relaxed) {
            controller.signal(app, start.elapsed());
            return Ok(());
        }
        if event::poll(TICK)? {
            let flow = match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let effect = app.key(key);
                    controller.perform(app, effect, start.elapsed())
                }
                Event::Mouse(mouse) => {
                    app.mouse(mouse);
                    Flow::Continue
                }
                _ => Flow::Continue,
            };
            if flow == Flow::Quit {
                return Ok(());
            }
        }
        if let Some(rx) = hotplug {
            while let Ok(event) = rx.try_recv() {
                controller.event(app, &event);
            }
        }
        if controller.tick(app, start.elapsed()) == Flow::Quit {
            return Ok(());
        }
    }
}
