// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Writing configuration files safely: through symbolic links, atomically,
//! with backups.
//!
//! - Symbolic links are followed and the **target** is replaced, so a
//!   dotfile manager's link (GNU Stow, chezmoi symlink mode) stays a link.
//! - The new content goes to a temporary file in the same directory, is
//!   flushed to disk and renamed over the target; the directory is flushed
//!   too. A crash leaves either the old or the new file, never half of one.
//! - The file's permissions are kept.
//! - Before the write, the old content is copied to
//!   `<name>.bak.<YYYYMMDDTHHMMSSZ>` next to the file or in a backup
//!   directory, and old backups beyond a count are removed.
//! - A file that changed on disk since it was read is not overwritten.
//!
//! Time is an input (`now`), so backup names are testable.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where backups go and how many are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPolicy {
    /// Directory for backups; next to the file when `None`.
    pub dir: Option<PathBuf>,
    /// How many backups of one file to keep; 0 disables backups.
    pub keep: usize,
}

impl Default for BackupPolicy {
    fn default() -> Self {
        BackupPolicy {
            dir: None,
            keep: 10,
        }
    }
}

/// A file as it was read, to be replaced later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The path as given.
    pub path: PathBuf,
    /// The file the path names after following symbolic links.
    pub real: PathBuf,
    /// The content, or `None` if the file does not exist.
    pub content: Option<String>,
}

impl Snapshot {
    /// The content, or an empty string for a missing file.
    #[must_use]
    pub fn text(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }
}

/// What a write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReport {
    /// The file that was replaced (symbolic links followed).
    pub real: PathBuf,
    /// The backup of the previous content, if one was made.
    pub backup: Option<PathBuf>,
}

/// Why a file could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The file changed after it was read; writing would lose that change.
    #[error("{0} changed on disk since hyprtilt read it; nothing was written")]
    Changed(PathBuf),
    /// The file is read-only, for example a link into the Nix store.
    #[error("{path} is read-only{hint}")]
    ReadOnly {
        /// The file.
        path: PathBuf,
        /// A hint on what to do instead.
        hint: String,
    },
    /// An I/O operation failed.
    #[error("{what} {path}: {source}")]
    Io {
        /// What hyprtilt was doing.
        what: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
}

fn io_error<'p>(what: &'static str, path: &'p Path) -> impl FnOnce(io::Error) -> WriteError + 'p {
    move |source| WriteError::Io {
        what,
        path: path.to_path_buf(),
        source,
    }
}

/// The file `path` names after following every symbolic link, also when
/// the final target does not exist yet.
///
/// # Errors
///
/// Returns an I/O error for a link that cannot be read, or after 40 links
/// (a loop).
pub fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = if target.is_absolute() {
                    target
                } else {
                    current
                        .parent()
                        .unwrap_or_else(|| Path::new("/"))
                        .join(target)
                };
            }
            Ok(_) => return Ok(current),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "too many levels of symbolic links at {}",
        path.display()
    )))
}

/// Read a file for a later [`replace`].
///
/// # Errors
///
/// Returns an I/O error other than "not found", including invalid UTF-8.
pub fn snapshot(path: &Path) -> io::Result<Snapshot> {
    let real = resolve(path)?;
    let content = match fs::read_to_string(&real) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    Ok(Snapshot {
        path: path.to_path_buf(),
        real,
        content,
    })
}

/// Replace the file of `snapshot` with `content`, atomically, after a
/// backup (unless `backups` is `None`).
///
/// # Errors
///
/// Returns [`WriteError::Changed`] if the file no longer has the content of
/// the snapshot, [`WriteError::ReadOnly`] for a read-only file or directory,
/// and [`WriteError::Io`] for any other failure; the file is then unchanged.
pub fn replace(
    snapshot: &Snapshot,
    content: &str,
    backups: Option<&BackupPolicy>,
    now: SystemTime,
) -> Result<WriteReport, WriteError> {
    let real = &snapshot.real;
    let current = match fs::read_to_string(real) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_error("cannot read", real)(e)),
    };
    if current != snapshot.content {
        return Err(WriteError::Changed(snapshot.path.clone()));
    }
    check_writable(snapshot)?;
    let backup = match (backups, &current) {
        (Some(policy), Some(old)) if policy.keep > 0 => Some(back_up(real, old, policy, now)?),
        _ => None,
    };
    write_atomic(real, content)?;
    Ok(WriteReport {
        real: real.clone(),
        backup,
    })
}

fn check_writable(snapshot: &Snapshot) -> Result<(), WriteError> {
    let real = &snapshot.real;
    let in_nix_store = real.starts_with("/nix/store");
    let read_only = in_nix_store || fs::metadata(real).is_ok_and(|m| m.permissions().readonly());
    if read_only {
        let hint = if in_nix_store {
            " (it is managed by Nix; change your Nix configuration instead)".to_owned()
        } else {
            String::new()
        };
        return Err(WriteError::ReadOnly {
            path: snapshot.path.clone(),
            hint,
        });
    }
    Ok(())
}

/// Write `content` to a temporary file next to `path` and rename it over
/// `path`, keeping the permissions of the old file.
fn write_atomic(path: &Path, content: &str) -> Result<(), WriteError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    if !dir.as_os_str().is_empty() && !dir.exists() {
        fs::create_dir_all(dir).map_err(io_error("cannot create directory", dir))?;
    }
    let name = path
        .file_name()
        .map_or_else(|| "config".into(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.hyprtilt-{}.tmp", std::process::id()));
    let mode = fs::metadata(path).map_or(0o644, |m| m.permissions().mode() & 0o7777);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| read_only_or(e, dir, "cannot create"))?;
        file.write_all(content.as_bytes())
            .map_err(io_error("cannot write", &tmp))?;
        file.set_permissions(fs::Permissions::from_mode(mode))
            .map_err(io_error("cannot set permissions of", &tmp))?;
        file.sync_all().map_err(io_error("cannot flush", &tmp))?;
        fs::rename(&tmp, path).map_err(io_error("cannot replace", path))?;
        // Flush the rename itself; failing here does not undo the write.
        let _ = File::open(dir).and_then(|d| d.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn read_only_or(e: io::Error, dir: &Path, what: &'static str) -> WriteError {
    if e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(30) {
        WriteError::ReadOnly {
            path: dir.to_path_buf(),
            hint: " (hyprtilt needs to create a temporary file in the directory)".to_owned(),
        }
    } else {
        io_error(what, dir)(e)
    }
}

/// Copy `old` to a new backup file and remove the oldest backups beyond
/// the policy's count. Returns the backup's path.
fn back_up(
    real: &Path,
    old: &str,
    policy: &BackupPolicy,
    now: SystemTime,
) -> Result<PathBuf, WriteError> {
    let dir = match &policy.dir {
        Some(d) => d.clone(),
        None => real
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };
    fs::create_dir_all(&dir).map_err(io_error("cannot create backup directory", &dir))?;
    let name = real
        .file_name()
        .map_or_else(|| "config".into(), |n| n.to_string_lossy().into_owned());
    let base = format!("{name}.bak.{}", utc_stamp(now));
    let mut path = dir.join(&base);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{base}.{n}"));
        n += 1;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(io_error("cannot create backup", &path))?;
    file.write_all(old.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io_error("cannot write backup", &path))?;
    prune(&dir, &name, policy.keep);
    Ok(path)
}

/// The backups of `name` in `dir`, oldest first.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be listed.
pub fn backups(dir: &Path, name: &str) -> io::Result<Vec<PathBuf>> {
    let prefix = format!("{name}.bak.");
    let mut found: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    // The timestamp sorts chronologically; a ".N" suffix sorts after the
    // name without it, which is the order they were made in.
    found.sort();
    Ok(found)
}

fn prune(dir: &Path, name: &str, keep: usize) {
    let Ok(found) = backups(dir, name) else {
        return;
    };
    let excess = found.len().saturating_sub(keep);
    for old in &found[..excess] {
        // A backup that cannot be removed is harmless.
        let _ = fs::remove_file(old);
    }
}

/// `YYYYMMDDTHHMMSSZ` in UTC.
///
/// # Examples
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
/// use hyprtilt_core::fsio::utc_stamp;
///
/// let t = UNIX_EPOCH + Duration::from_secs(1_790_602_009);
/// assert_eq!(utc_stamp(t), "20260928T132649Z");
/// ```
#[must_use]
pub fn utc_stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A unified diff between the old and new content of a file, for
/// `--dry-run`; empty when they are equal.
///
/// # Examples
///
/// ```
/// let d = hyprtilt_core::fsio::diff("a\nb\n", "a\nc\n", "hypr-user.lua");
/// assert!(d.contains("-b\n+c\n"));
/// assert!(hyprtilt_core::fsio::diff("x", "x", "f").is_empty());
/// ```
#[must_use]
pub fn diff(old: &str, new: &str, name: &str) -> String {
    if old == new {
        return String::new();
    }
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn dates() {
        assert_eq!(utc_stamp(at(0)), "19700101T000000Z");
        assert_eq!(utc_stamp(at(951_782_400)), "20000229T000000Z");
        assert_eq!(utc_stamp(at(4_102_444_799)), "20991231T235959Z");
        assert_eq!(
            utc_stamp(UNIX_EPOCH - Duration::from_secs(5)),
            "19700101T000000Z"
        );
    }

    #[test]
    fn replace_writes_backs_up_and_keeps_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hypr-user.lua");
        fs::write(&file, "old\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let snap = snapshot(&file).unwrap();
        assert_eq!(snap.text(), "old\n");
        let report = replace(&snap, "new\n", Some(&BackupPolicy::default()), at(0)).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
        let backup = report.backup.unwrap();
        assert_eq!(
            backup,
            dir.path().join("hypr-user.lua.bak.19700101T000000Z")
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old\n");
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // No temporary files are left behind.
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn a_link_stays_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("dotfiles");
        fs::create_dir(&target_dir).unwrap();
        let target = target_dir.join("hypr-user.lua");
        fs::write(&target, "old").unwrap();
        let link = dir.path().join("link.lua");
        symlink("dotfiles/hypr-user.lua", &link).unwrap();
        let snap = snapshot(&link).unwrap();
        assert_eq!(snap.real, target);
        replace(&snap, "new", None, at(0)).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn missing_files_are_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub/dir/hypr-user.lua");
        let snap = snapshot(&file).unwrap();
        assert_eq!(snap.content, None);
        assert_eq!(snap.text(), "");
        let report = replace(&snap, "x", Some(&BackupPolicy::default()), at(0)).unwrap();
        assert_eq!(report.backup, None);
        assert_eq!(fs::read_to_string(&file).unwrap(), "x");
    }

    #[test]
    fn a_changed_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.conf");
        fs::write(&file, "one").unwrap();
        let snap = snapshot(&file).unwrap();
        fs::write(&file, "two").unwrap();
        let err = replace(&snap, "three", None, at(0)).unwrap_err();
        assert!(matches!(err, WriteError::Changed(_)), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
    }

    #[test]
    fn read_only_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.conf");
        fs::write(&file, "one").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
        let snap = snapshot(&file).unwrap();
        let err = replace(&snap, "two", None, at(0)).unwrap_err();
        assert!(matches!(err, WriteError::ReadOnly { .. }), "{err}");
        let nix = Snapshot {
            path: PathBuf::from("/nix/store/x-hm/hyprland.lua"),
            real: PathBuf::from("/nix/store/x-hm/hyprland.lua"),
            content: None,
        };
        let err = replace(&nix, "two", None, at(0)).unwrap_err();
        assert!(err.to_string().contains("managed by Nix"), "{err}");
    }

    #[test]
    fn backups_are_rotated_and_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let backup_dir = dir.path().join("backups");
        let file = dir.path().join("hyprland.conf");
        fs::write(&file, "0").unwrap();
        let policy = BackupPolicy {
            dir: Some(backup_dir.clone()),
            keep: 3,
        };
        for i in 1..=5u64 {
            let snap = snapshot(&file).unwrap();
            // Two writes in the same second get distinct names.
            replace(&snap, &i.to_string(), Some(&policy), at(i / 2)).unwrap();
        }
        let kept: Vec<String> = backups(&backup_dir, "hyprland.conf")
            .unwrap()
            .iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect();
        assert_eq!(kept, ["2", "3", "4"]);
        // keep = 0 disables backups.
        let none = BackupPolicy { dir: None, keep: 0 };
        let snap = snapshot(&file).unwrap();
        assert_eq!(
            replace(&snap, "6", Some(&none), at(9)).unwrap().backup,
            None
        );
    }

    #[test]
    fn link_loops_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        symlink(&b, &a).unwrap();
        symlink(&a, &b).unwrap();
        assert!(resolve(&a).is_err());
        assert!(snapshot(&a).is_err());
    }

    #[test]
    fn diffs() {
        let d = diff("a\n", "b\n", "x.lua");
        assert!(d.starts_with("--- a/x.lua\n+++ b/x.lua\n"), "{d}");
    }
}
