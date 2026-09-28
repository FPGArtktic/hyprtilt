// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Hyprland's event socket (`.socket2.sock`).
//!
//! Each event is a line `<name>>><data>`. Hyprland drops a client that
//! lets events pile up, so the socket is read continuously on a thread of
//! its own. There are no events for mode, position, scale or transform
//! changes; after its own changes hyprtilt polls the monitor list instead
//! (`docs/hyprland-lua-api.md`, section 3.6).

use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// An event hyprtilt reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A monitor was connected or enabled (`monitoradded>>NAME`).
    MonitorAdded(String),
    /// A monitor was disconnected or disabled (`monitorremoved>>NAME`).
    MonitorRemoved(String),
    /// The configuration was reloaded (also after a failed reload).
    ConfigReloaded,
}

/// Parse one event line; `None` for events hyprtilt ignores. The `v2`
/// variants of the monitor events are ignored because the plain ones are
/// always sent as well.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::ipc::events::{parse_event, Event};
///
/// assert_eq!(parse_event("monitoradded>>DP-1"), Some(Event::MonitorAdded("DP-1".into())));
/// assert_eq!(parse_event("configreloaded>>"), Some(Event::ConfigReloaded));
/// assert_eq!(parse_event("workspace>>2"), None);
/// ```
#[must_use]
pub fn parse_event(line: &str) -> Option<Event> {
    let (name, data) = line.trim_end_matches(['\n', '\r']).split_once(">>")?;
    match name {
        "monitoradded" => Some(Event::MonitorAdded(data.to_owned())),
        "monitorremoved" => Some(Event::MonitorRemoved(data.to_owned())),
        "configreloaded" => Some(Event::ConfigReloaded),
        _ => None,
    }
}

/// Connect to the event socket and forward the events hyprtilt reacts to.
/// The reading thread ends when the socket closes or the receiver is
/// dropped.
///
/// # Errors
///
/// Returns an I/O error if the socket cannot be opened.
pub fn listen(socket: &Path) -> io::Result<Receiver<Event>> {
    let stream = UnixStream::connect(socket)?;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("hyprland-events".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            // Event data is cut at 1024 bytes and may split a UTF-8
            // sequence, so lines are read as bytes.
            while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                let text = String::from_utf8_lossy(&line);
                if let Some(event) = parse_event(&text)
                    && tx.send(event).is_err()
                {
                    return;
                }
                line.clear();
            }
        })?;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    #[test]
    fn events_are_forwarded_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".socket2.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.write_all(
                b"workspace>>1\nmonitoraddedv2>>3,DP-2,Acme\nmonitoradded>>DP-2\nmonitorremoved>>DP-2\nconfigreloaded>>\n",
            )
            .unwrap();
        });
        let rx = listen(&path).unwrap();
        let timeout = Duration::from_secs(5);
        assert_eq!(
            rx.recv_timeout(timeout).unwrap(),
            Event::MonitorAdded("DP-2".into())
        );
        assert_eq!(
            rx.recv_timeout(timeout).unwrap(),
            Event::MonitorRemoved("DP-2".into())
        );
        assert_eq!(rx.recv_timeout(timeout).unwrap(), Event::ConfigReloaded);
        server.join().unwrap();
        // The socket closed: the channel ends.
        assert!(rx.recv_timeout(timeout).is_err());
    }

    #[test]
    fn missing_socket() {
        assert!(listen(Path::new("/nonexistent/.socket2.sock")).is_err());
        assert_eq!(parse_event("garbage"), None);
        assert_eq!(
            parse_event("monitorremoved>>eDP-1\r\n"),
            Some(Event::MonitorRemoved("eDP-1".into()))
        );
    }
}
