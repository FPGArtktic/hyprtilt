// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Hyprland's request socket, used directly (`hyprctl` is not needed).
//!
//! The socket is `$XDG_RUNTIME_DIR/hypr/<signature>/.socket.sock`. Hyprland
//! 0.56 reads a request in 1023-byte chunks without a timeout and writes
//! the reply with a blocking write, so a client must send the request in
//! one write, shut down its sending side, and read the reply to the end
//! at once (`docs/hyprland-lua-api.md`, section 3.3).

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{HyprlandIpc, IpcError};

/// A running Hyprland instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// `HYPRLAND_INSTANCE_SIGNATURE`.
    pub signature: String,
    /// `$XDG_RUNTIME_DIR/hypr/<signature>`.
    pub dir: PathBuf,
    /// The compositor's process ID, from its lock file.
    pub pid: Option<u32>,
}

impl Instance {
    /// The request socket.
    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.dir.join(".socket.sock")
    }

    /// The event socket.
    #[must_use]
    pub fn event_socket(&self) -> PathBuf {
        self.dir.join(".socket2.sock")
    }
}

/// `$XDG_RUNTIME_DIR/hypr`, or `/run/user/<uid>/hypr` when the variable is
/// unset (as `hyprctl` does).
#[must_use]
pub fn runtime_root(xdg_runtime_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = xdg_runtime_dir.filter(|d| !d.is_empty()) {
        return Path::new(dir).join("hypr");
    }
    let uid = std::fs::metadata("/proc/self").map_or(0, |m| m.uid());
    PathBuf::from(format!("/run/user/{uid}/hypr"))
}

/// Parse a `hyprland.lock` file: the PID on the first line and the Wayland
/// socket on the second.
#[must_use]
pub fn parse_lock(text: &str) -> Option<u32> {
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    lines.next()?;
    Some(pid)
}

/// The running instances under `root`, newest first, as `hyprctl
/// instances` lists them: every directory with a lock file whose process
/// is alive. The start time is the second `_`-separated part of the
/// signature.
#[must_use]
pub fn instances(root: &Path) -> Vec<Instance> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(u64, Instance)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let dir = e.path();
            let lock = std::fs::read_to_string(dir.join("hyprland.lock")).ok()?;
            let pid = parse_lock(&lock)?;
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return None;
            }
            let signature = e.file_name().to_string_lossy().into_owned();
            let time = signature
                .split('_')
                .nth(1)
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
            Some((
                time,
                Instance {
                    signature,
                    dir,
                    pid: Some(pid),
                },
            ))
        })
        .collect();
    found.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    found.into_iter().map(|(_, i)| i).collect()
}

/// The instance to talk to: the one named by `signature`
/// (`$HYPRLAND_INSTANCE_SIGNATURE`), else the newest running one.
///
/// # Errors
///
/// Returns [`IpcError::NotRunning`] when there is none.
pub fn find_instance(root: &Path, signature: Option<&str>) -> Result<Instance, IpcError> {
    if let Some(sig) = signature.filter(|s| !s.is_empty()) {
        let dir = root.join(sig);
        let pid = std::fs::read_to_string(dir.join("hyprland.lock"))
            .ok()
            .and_then(|l| parse_lock(&l));
        return Ok(Instance {
            signature: sig.to_owned(),
            dir,
            pid,
        });
    }
    instances(root).into_iter().next().ok_or_else(|| {
        IpcError::NotRunning(format!(
            "HYPRLAND_INSTANCE_SIGNATURE is not set and no instance was found in {}",
            root.display()
        ))
    })
}

/// A connection to the request socket of one instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketIpc {
    socket: PathBuf,
    timeout: Duration,
}

impl SocketIpc {
    /// Talk to the socket at `socket`, waiting at most `timeout` for a
    /// reply.
    #[must_use]
    pub fn new(socket: PathBuf, timeout: Duration) -> Self {
        SocketIpc { socket, timeout }
    }

    /// The socket's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.socket
    }
}

impl HyprlandIpc for SocketIpc {
    fn request(&self, request: &str) -> Result<String, IpcError> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| {
            IpcError::NotRunning(format!("cannot connect to {}: {e}", self.socket.display()))
        })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(request.as_bytes())?;
        stream.shutdown(Shutdown::Write)?;
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply)?;
        Ok(String::from_utf8_lossy(&reply).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// A server that answers one request with `reply` and returns the
    /// request it got.
    fn serve_once(path: &Path, reply: &'static str) -> thread::JoinHandle<String> {
        let listener = UnixListener::bind(path).unwrap();
        thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = String::new();
            conn.read_to_string(&mut request).unwrap();
            conn.write_all(reply.as_bytes()).unwrap();
            request
        })
    }

    #[test]
    fn request_and_reply() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join(".socket.sock");
        let server = serve_once(&sock, "ok");
        let ipc = SocketIpc::new(sock.clone(), Duration::from_secs(5));
        assert_eq!(ipc.path(), sock);
        ipc.reload().unwrap();
        assert_eq!(server.join().unwrap(), "/reload");
    }

    #[test]
    fn long_requests_end_with_the_shutdown() {
        // 2046 bytes: a multiple of Hyprland's 1023-byte chunk.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join(".socket.sock");
        let server = serve_once(&sock, "ok");
        let code = "x".repeat(2046 - "/eval ".len());
        SocketIpc::new(sock, Duration::from_secs(5))
            .eval(&code)
            .unwrap();
        assert_eq!(server.join().unwrap().len(), 2046);
    }

    #[test]
    fn missing_socket_means_not_running() {
        let ipc = SocketIpc::new(
            PathBuf::from("/nonexistent/.socket.sock"),
            Duration::from_secs(1),
        );
        assert!(matches!(
            ipc.request("j/version"),
            Err(IpcError::NotRunning(_))
        ));
    }

    #[test]
    fn lock_files_and_instances() {
        assert_eq!(parse_lock("3436\nwayland-1\n"), Some(3436));
        assert_eq!(parse_lock("3436\n"), None);
        assert_eq!(parse_lock("x\ny\n"), None);
        let root = tempfile::tempdir().unwrap();
        let me = std::process::id();
        for (sig, pid) in [
            ("abc_1790000000_1", me),
            ("abc_1790000500_2", me),
            ("abc_1790000900_3", u32::MAX),
        ] {
            let dir = root.path().join(sig);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("hyprland.lock"), format!("{pid}\nwayland-1\n")).unwrap();
        }
        std::fs::create_dir(root.path().join("no-lock")).unwrap();
        let found = instances(root.path());
        let sigs: Vec<&str> = found.iter().map(|i| i.signature.as_str()).collect();
        assert_eq!(sigs, ["abc_1790000500_2", "abc_1790000000_1"]);
        assert_eq!(
            found[0].socket(),
            root.path().join("abc_1790000500_2/.socket.sock")
        );
        assert_eq!(
            found[0].event_socket(),
            root.path().join("abc_1790000500_2/.socket2.sock")
        );
        let chosen = find_instance(root.path(), None).unwrap();
        assert_eq!(chosen.signature, "abc_1790000500_2");
        let named = find_instance(root.path(), Some("abc_1790000000_1")).unwrap();
        assert_eq!(named.pid, Some(me));
        assert!(find_instance(Path::new("/nonexistent"), None).is_err());
        assert!(instances(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn runtime_roots() {
        assert_eq!(
            runtime_root(Some("/run/user/1000")),
            PathBuf::from("/run/user/1000/hypr")
        );
        assert!(runtime_root(None).starts_with("/run/user"));
    }
}
