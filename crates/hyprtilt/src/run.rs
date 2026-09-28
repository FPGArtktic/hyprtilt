// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Running an apply session on the command line: the clock ticks every
//! 50 ms, confirmation is read from standard input with a countdown on
//! standard error, and `SIGINT` or `SIGTERM` rolls the change back.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use hyprtilt_core::apply::{Effects, Input, Outcome, Phase, Session, step};
use signal_hook::consts::{SIGINT, SIGTERM};

/// Run `session` to its end.
pub(crate) fn run(session: &mut Session, effects: &mut dyn Effects) -> Outcome {
    let signal = Arc::new(AtomicBool::new(false));
    for sig in [SIGINT, SIGTERM] {
        // Without the handler a signal ends the process without rolling
        // back; failing to install it leaves that default.
        let _ = signal_hook::flag::register(sig, Arc::clone(&signal));
    }
    let start = Instant::now();
    if let Some(outcome) = step(session, None, start.elapsed(), effects) {
        return outcome;
    }
    let mut answers: Option<mpsc::Receiver<String>> = None;
    let mut shown: Option<u64> = None;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = start.elapsed();
        let input = if signal.load(Ordering::Relaxed) {
            Input::Signal
        } else if session.phase() == Phase::Confirming {
            let rx = answers.get_or_insert_with(read_lines);
            prompt(session.remaining(now), &mut shown);
            match rx.try_recv() {
                Ok(line) if is_yes(&line) => Input::Confirm,
                Ok(_) => Input::Cancel,
                Err(_) => Input::Tick,
            }
        } else {
            Input::Tick
        };
        if let Some(outcome) = step(session, Some(input), now, effects) {
            if shown.is_some() {
                eprintln!();
            }
            return outcome;
        }
    }
}

fn is_yes(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Lines from standard input, read on a thread of their own. At the end
/// of the input nothing more is sent, so the countdown runs out.
fn read_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}

fn prompt(remaining: Option<Duration>, shown: &mut Option<u64>) {
    let seconds = remaining.map_or(0, |r| r.as_millis().div_ceil(1000) as u64);
    if *shown == Some(seconds) {
        return;
    }
    *shown = Some(seconds);
    let mut err = std::io::stderr().lock();
    let _ = write!(
        err,
        "\rKeep this layout? Type y and Enter to keep it, anything else to revert ({seconds:>2} s) "
    );
    let _ = err.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        assert!(is_yes("y\n") && is_yes(" YES ") && !is_yes("n") && !is_yes(""));
    }
}
