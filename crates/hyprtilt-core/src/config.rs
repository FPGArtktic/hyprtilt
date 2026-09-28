// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Finding the file hyprtilt edits.
//!
//! Hyprland picks its main configuration in a fixed order (see
//! `docs/hyprland-lua-api.md`, "Config discovery"): an explicit `--config`,
//! then `$HYPRLAND_CONFIG`, then the first `hypr/hyprland.lua` in the XDG
//! configuration directories, then the first `hypr/hyprland.conf`. hyprtilt
//! repeats that search, reading the command line and environment of the
//! running compositor when it can, because they may differ from its own.
//!
//! On top of that comes the Caelestia preset: Caelestia owns
//! `~/.config/hypr/` and overwrites it on update, and loads the user's own
//! `hypr-user.lua` with `require("hypr-user")` through a `package.path` it
//! extends. The target is then the file that `require` really resolves to.
//!
//! Everything here is pure: file system and environment access go through
//! the [`Probe`] trait, so the search is tested without touching the real
//! system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::document::Backend;
use crate::lua::lexer::{self, TokenKind};

/// Read-only access to the environment and the file system.
pub trait Probe {
    /// An environment variable of the Hyprland process (or of hyprtilt
    /// when the compositor's environment is unknown).
    fn var(&self, name: &str) -> Option<String>;
    /// Whether a regular file exists at `path`.
    fn is_file(&self, path: &Path) -> bool;
    /// The content of a file, if it can be read.
    fn read(&self, path: &Path) -> Option<String>;
}

/// Where the main configuration path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainConfigSource {
    /// `--config`/`-c` on the compositor's command line.
    CommandLine,
    /// `$HYPRLAND_CONFIG`.
    Environment,
    /// Found in an XDG configuration directory.
    Xdg,
    /// Nothing exists; Hyprland would generate this file.
    Default,
}

/// Hyprland's main configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainConfig {
    /// The path.
    pub path: PathBuf,
    /// How it was found.
    pub source: MainConfigSource,
    /// The backend Hyprland uses for it (by extension).
    pub backend: Backend,
}

/// The XDG configuration directories in Hyprland's search order:
/// `$XDG_CONFIG_HOME`, `~/.config`, each entry of `$XDG_CONFIG_DIRS`, then
/// `/etc/xdg`, without duplicates. Relative entries are ignored, as the XDG
/// specification requires.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::config::xdg_config_dirs;
/// use std::path::PathBuf;
///
/// let dirs = xdg_config_dirs(Some("/home/u"), None, None);
/// assert_eq!(dirs, [PathBuf::from("/home/u/.config"), PathBuf::from("/etc/xdg")]);
/// ```
#[must_use]
pub fn xdg_config_dirs(
    home: Option<&str>,
    xdg_config_home: Option<&str>,
    xdg_config_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_absolute() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    if let Some(x) = xdg_config_home.filter(|s| !s.is_empty()) {
        push(PathBuf::from(x));
    }
    if let Some(h) = home.filter(|s| !s.is_empty()) {
        push(Path::new(h).join(".config"));
    }
    for d in xdg_config_dirs
        .unwrap_or("")
        .split(':')
        .filter(|s| !s.is_empty())
    {
        push(PathBuf::from(d));
    }
    push(PathBuf::from("/etc/xdg"));
    dirs
}

/// The path given with `-c`/`--config` in a command line, if any.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::config::config_from_cmdline;
/// use std::path::PathBuf;
///
/// let args = ["Hyprland", "--config", "/tmp/h.lua"].map(String::from);
/// assert_eq!(config_from_cmdline(&args), Some(PathBuf::from("/tmp/h.lua")));
/// assert_eq!(config_from_cmdline(&["Hyprland".to_owned()]), None);
/// ```
#[must_use]
pub fn config_from_cmdline(args: &[String]) -> Option<PathBuf> {
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "-c" || arg == "--config" {
            return it.next().map(PathBuf::from);
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(value));
        }
    }
    None
}

/// Find Hyprland's main configuration the way Hyprland 0.56 does. Safe mode
/// (`recoverycfg.lua`) is not considered: hyprtilt never edits it.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::config::{find_main_config, MainConfigSource, Probe};
/// use std::path::Path;
///
/// struct Home;
/// impl Probe for Home {
///     fn var(&self, name: &str) -> Option<String> {
///         (name == "HOME").then(|| "/home/u".to_owned())
///     }
///     fn is_file(&self, p: &Path) -> bool {
///         p == Path::new("/home/u/.config/hypr/hyprland.lua")
///     }
///     fn read(&self, _: &Path) -> Option<String> { None }
/// }
///
/// let main = find_main_config(&Home, None);
/// assert_eq!(main.path, Path::new("/home/u/.config/hypr/hyprland.lua"));
/// assert_eq!(main.source, MainConfigSource::Xdg);
/// ```
#[must_use]
pub fn find_main_config(probe: &dyn Probe, cmdline_config: Option<&Path>) -> MainConfig {
    let make = |path: PathBuf, source| MainConfig {
        backend: Backend::for_path(&path),
        path,
        source,
    };
    if let Some(p) = cmdline_config {
        return make(p.to_path_buf(), MainConfigSource::CommandLine);
    }
    if let Some(p) = probe.var("HYPRLAND_CONFIG").filter(|s| !s.is_empty()) {
        return make(PathBuf::from(p), MainConfigSource::Environment);
    }
    let dirs = xdg_config_dirs(
        probe.var("HOME").as_deref(),
        probe.var("XDG_CONFIG_HOME").as_deref(),
        probe.var("XDG_CONFIG_DIRS").as_deref(),
    );
    for ext in ["lua", "conf"] {
        for dir in &dirs {
            let candidate = dir.join("hypr").join(format!("hyprland.{ext}"));
            if probe.is_file(&candidate) {
                return make(candidate, MainConfigSource::Xdg);
            }
        }
    }
    let base = dirs
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("/etc/xdg"));
    make(base.join("hypr/hyprland.lua"), MainConfigSource::Default)
}

/// A `require("name")` call with a literal module name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequireCall {
    /// The module name.
    pub module: String,
    /// 1-based line of the call.
    pub line: usize,
}

/// A literal path template found in a `package.path` assignment, such as
/// Caelestia's `home .. "/.config/caelestia/?.lua"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathAddition {
    /// The template with `?` for the module name. A template that starts
    /// with `/.config/` after a concatenation is taken as relative to
    /// `$HOME`, which covers the common `os.getenv("HOME") .. "/..."`.
    pub template: String,
    /// Whether it is appended (`package.path .. ";" .. x`, searched last)
    /// rather than prepended.
    pub appended: bool,
}

/// Scan Lua source for `require` calls and `package.path` additions with
/// literal strings. Code is never executed; anything dynamic is ignored.
#[must_use]
pub fn scan_lua_loading(src: &str, home: Option<&str>) -> (Vec<RequireCall>, Vec<PathAddition>) {
    let Ok(tokens) = lexer::tokenize(src) else {
        return (Vec::new(), Vec::new());
    };
    let tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.kind != TokenKind::Comment)
        .collect();
    let mut requires = Vec::new();
    let mut additions = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.is_name(src, "require") {
            let arg = match tokens.get(i + 1) {
                Some(open) if open.is(src, "(") => tokens.get(i + 2),
                other => other,
            };
            if let Some(TokenKind::Str(bytes)) = arg.map(|a| &a.kind) {
                requires.push(RequireCall {
                    module: String::from_utf8_lossy(bytes).into_owned(),
                    line: crate::block::line_of(src, t.span.start),
                });
            }
        }
        let assigns_path = t.is_name(src, "package")
            && tokens.get(i + 1).is_some_and(|d| d.is(src, "."))
            && tokens.get(i + 2).is_some_and(|p| p.is_name(src, "path"))
            && tokens.get(i + 3).is_some_and(|e| e.is(src, "="));
        if assigns_path {
            additions.extend(path_additions(src, &tokens[i + 4..], home));
        }
    }
    (requires, additions)
}

/// The string literals containing `?` in the right-hand side of a
/// `package.path = ...` statement (up to the end of its line).
fn path_additions(src: &str, rhs: &[lexer::Token], home: Option<&str>) -> Vec<PathAddition> {
    let Some(first) = rhs.first() else {
        return Vec::new();
    };
    let line = crate::block::line_of(src, first.span.start);
    let rhs: Vec<_> = rhs
        .iter()
        .take_while(|t| crate::block::line_of(src, t.span.start) == line)
        .collect();
    // "package.path .. x" appends, "x .. package.path" prepends.
    let appended = rhs.first().is_some_and(|t| t.is_name(src, "package"));
    rhs.iter()
        .filter_map(|t| match &t.kind {
            TokenKind::Str(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .flat_map(|s| {
            s.split(';')
                .filter(|p| p.contains('?'))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .map(|template| {
            let template = match home {
                Some(h) if template.starts_with("/.config/") => format!("{h}{template}"),
                _ => template,
            };
            PathAddition { template, appended }
        })
        .collect()
}

/// Lua 5.5's default `package.path` as built by Arch Linux and most
/// distributions (see `luaconf.h`).
pub const LUA_DEFAULT_PATH: &str = "/usr/local/share/lua/5.5/?.lua;/usr/local/share/lua/5.5/?/init.lua;\
/usr/share/lua/5.5/?.lua;/usr/share/lua/5.5/?/init.lua;/usr/local/lib/lua/5.5/?.lua;\
/usr/local/lib/lua/5.5/?/init.lua;/usr/lib/lua/5.5/?.lua;/usr/lib/lua/5.5/?/init.lua;./?.lua;./?/init.lua";

/// The `package.path` templates Hyprland searches for `require`, in order:
/// the main configuration's directory, then the default path (from
/// `LUA_PATH_5_5` or `LUA_PATH` when set, where `;;` stands for the
/// default), with the additions the configuration makes.
#[must_use]
pub fn require_search_path(
    config_dir: &Path,
    lua_path_env: Option<&str>,
    additions: &[PathAddition],
) -> Vec<String> {
    let default = match lua_path_env {
        Some(env) => env.replace(";;", &format!(";{LUA_DEFAULT_PATH};")),
        None => LUA_DEFAULT_PATH.to_owned(),
    };
    let dir = config_dir.display();
    let mut path: Vec<String> = vec![format!("{dir}/?.lua"), format!("{dir}/?/init.lua")];
    path.extend(
        default
            .split(';')
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    );
    for a in additions {
        if a.appended {
            path.push(a.template.clone());
        } else {
            path.insert(0, a.template.clone());
        }
    }
    path
}

/// Resolve a module the way `package.searchpath` does: replace `.` in the
/// name with `/`, substitute it for `?` in each template, and return every
/// existing candidate in order. The first one is what `require` loads; the
/// others are shadowed. Relative templates are resolved against `cwd`
/// (Hyprland's working directory).
#[must_use]
pub fn resolve_module(
    probe: &dyn Probe,
    module: &str,
    search_path: &[String],
    cwd: &Path,
) -> Vec<PathBuf> {
    let name = module.replace('.', "/");
    let mut found = Vec::new();
    for template in search_path {
        let candidate = PathBuf::from(template.replace('?', &name));
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        };
        if probe.is_file(&candidate) && !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    found
}

/// Why a file was chosen as the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetReason {
    /// Given with `--file`.
    Explicit,
    /// `target` in the settings file.
    Settings,
    /// The Caelestia preset: the file `require("hypr-user")` resolves to.
    Caelestia,
    /// Hyprland's main configuration.
    MainConfig(MainConfigSource),
}

/// The file hyprtilt edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The path.
    pub path: PathBuf,
    /// Its backend.
    pub backend: Backend,
    /// Why this file.
    pub reason: TargetReason,
    /// Problems worth telling the user about (shadowed modules, ...).
    pub warnings: Vec<String>,
}

/// What [`resolve_target`] needs to know.
#[derive(Debug, Clone, Default)]
pub struct TargetRequest {
    /// `--file`.
    pub explicit: Option<PathBuf>,
    /// `--backend`, overriding the extension.
    pub backend: Option<Backend>,
    /// `target` from the settings.
    pub settings_target: Option<PathBuf>,
    /// `--config` of the running compositor.
    pub cmdline_config: Option<PathBuf>,
    /// The compositor's working directory (for relative `package.path`
    /// entries); `$HOME` when unknown.
    pub cwd: Option<PathBuf>,
}

/// The module name Caelestia uses for the user's file.
pub const CAELESTIA_MODULE: &str = "hypr-user";

/// Decide which file to edit (ADR 0001, D6).
///
/// # Examples
///
/// ```
/// use hyprtilt_core::config::{resolve_target, Probe, TargetReason, TargetRequest};
/// use std::path::{Path, PathBuf};
///
/// struct Fs;
/// impl Probe for Fs {
///     fn var(&self, name: &str) -> Option<String> {
///         (name == "HOME").then(|| "/home/u".to_owned())
///     }
///     fn is_file(&self, p: &Path) -> bool {
///         p == Path::new("/home/u/.config/hypr/hyprland.conf")
///     }
///     fn read(&self, _: &Path) -> Option<String> { Some(String::new()) }
/// }
///
/// let target = resolve_target(&Fs, &TargetRequest::default());
/// assert_eq!(target.path, PathBuf::from("/home/u/.config/hypr/hyprland.conf"));
/// assert!(matches!(target.reason, TargetReason::MainConfig(_)));
/// ```
#[must_use]
pub fn resolve_target(probe: &dyn Probe, request: &TargetRequest) -> Target {
    let with_backend = |path: PathBuf, reason| Target {
        backend: request.backend.unwrap_or_else(|| Backend::for_path(&path)),
        path,
        reason,
        warnings: Vec::new(),
    };
    if let Some(p) = &request.explicit {
        return with_backend(p.clone(), TargetReason::Explicit);
    }
    if let Some(p) = &request.settings_target {
        return with_backend(p.clone(), TargetReason::Settings);
    }
    let main = find_main_config(probe, request.cmdline_config.as_deref());
    if main.backend == Backend::Lua
        && let Some(target) = caelestia_target(probe, &main, request)
    {
        return target;
    }
    with_backend(main.path.clone(), TargetReason::MainConfig(main.source))
}

/// The Caelestia preset applies when the main Lua configuration requires
/// `hypr-user` and extends `package.path` with a `caelestia` directory.
fn caelestia_target(
    probe: &dyn Probe,
    main: &MainConfig,
    request: &TargetRequest,
) -> Option<Target> {
    let src = probe.read(&main.path)?;
    let home = probe.var("HOME");
    let (requires, additions) = scan_lua_loading(&src, home.as_deref());
    let uses_module = requires.iter().any(|r| r.module == CAELESTIA_MODULE);
    let caelestia_path = additions.iter().any(|a| a.template.contains("caelestia"));
    if !(uses_module && caelestia_path) {
        return None;
    }
    let config_dir = main.path.parent().unwrap_or(Path::new("/"));
    let lua_path = probe.var("LUA_PATH_5_5").or_else(|| probe.var("LUA_PATH"));
    let search = require_search_path(config_dir, lua_path.as_deref(), &additions);
    let cwd = request
        .cwd
        .clone()
        .or_else(|| home.as_ref().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    let found = resolve_module(probe, CAELESTIA_MODULE, &search, &cwd);
    let mut warnings = Vec::new();
    let path = if let Some(first) = found.first() {
        for shadowed in &found[1..] {
            warnings.push(format!(
                "{} is shadowed by {}: require(\"{CAELESTIA_MODULE}\") loads the latter",
                shadowed.display(),
                first.display()
            ));
        }
        first.clone()
    } else {
        // Caelestia creates the file on the next reload; target the
        // location it creates.
        let template = additions
            .iter()
            .find(|a| a.template.contains("caelestia"))?
            .template
            .clone();
        PathBuf::from(template.replace('?', CAELESTIA_MODULE))
    };
    Some(Target {
        backend: request.backend.unwrap_or(Backend::Lua),
        path,
        reason: TargetReason::Caelestia,
        warnings,
    })
}

/// A [`Probe`] backed by the real system: the environment of the process
/// given by `environ` (for example the compositor's, read from
/// `/proc/<pid>/environ`), and the real file system.
#[derive(Debug, Clone, Default)]
pub struct SystemProbe {
    /// Environment variables to use instead of hyprtilt's own.
    pub environ: Option<HashMap<String, String>>,
}

impl SystemProbe {
    /// Parse a NUL-separated `/proc/<pid>/environ` or `cmdline` blob.
    #[must_use]
    pub fn split_nul(blob: &[u8]) -> Vec<String> {
        blob.split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect()
    }

    /// A probe using the environment in a `/proc/<pid>/environ` blob.
    #[must_use]
    pub fn from_environ_blob(blob: &[u8]) -> SystemProbe {
        let environ = Self::split_nul(blob)
            .into_iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect();
        SystemProbe {
            environ: Some(environ),
        }
    }
}

impl Probe for SystemProbe {
    fn var(&self, name: &str) -> Option<String> {
        match &self.environ {
            Some(env) => env.get(name).cloned(),
            None => std::env::var(name).ok(),
        }
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A fake system: environment variables and files with contents.
    #[derive(Default)]
    struct Fake {
        env: HashMap<&'static str, &'static str>,
        files: HashMap<PathBuf, String>,
    }

    impl Fake {
        fn home() -> Fake {
            let mut f = Fake::default();
            f.env.insert("HOME", "/home/u");
            f
        }
        fn file(mut self, p: &str, content: &str) -> Fake {
            self.files.insert(PathBuf::from(p), content.to_owned());
            self
        }
    }

    impl Probe for Fake {
        fn var(&self, name: &str) -> Option<String> {
            self.env.get(name).map(|s| (*s).to_owned())
        }
        fn is_file(&self, p: &Path) -> bool {
            self.files.contains_key(p)
        }
        fn read(&self, p: &Path) -> Option<String> {
            self.files.get(p).cloned()
        }
    }

    const CAELESTIA_MAIN: &str = r#"local home   = os.getenv("HOME")
local hypr   = home .. "/.config/hypr"
package.path = package.path .. ";" .. home .. "/.config/caelestia/?.lua"
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
require("hyprland.env")
-- require("commented.out")
require("hypr-user")
"#;

    #[test]
    fn xdg_dirs_order_and_dedup() {
        let dirs = xdg_config_dirs(
            Some("/home/u"),
            Some("/home/u/.config"),
            Some("/opt/x:rel:/etc/xdg"),
        );
        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/u/.config"),
                PathBuf::from("/opt/x"),
                PathBuf::from("/etc/xdg")
            ]
        );
        assert_eq!(
            xdg_config_dirs(None, None, None),
            [PathBuf::from("/etc/xdg")]
        );
    }

    #[test]
    fn cmdline_forms() {
        let a = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            config_from_cmdline(&a(&["H", "-c", "/x.lua"])),
            Some("/x.lua".into())
        );
        assert_eq!(
            config_from_cmdline(&a(&["H", "--config=/y.conf"])),
            Some("/y.conf".into())
        );
        assert_eq!(config_from_cmdline(&a(&["H", "--watchdog-fd", "4"])), None);
        assert_eq!(config_from_cmdline(&a(&["H", "-c"])), None);
    }

    #[test]
    fn main_config_precedence() {
        let fs = Fake::home()
            .file("/home/u/.config/hypr/hyprland.conf", "")
            .file("/etc/xdg/hypr/hyprland.lua", "");
        // A .lua anywhere wins over a .conf, even a system-wide one.
        let m = find_main_config(&fs, None);
        assert_eq!(m.path, PathBuf::from("/etc/xdg/hypr/hyprland.lua"));
        assert_eq!(m.backend, Backend::Lua);

        let m = find_main_config(&fs, Some(Path::new("/c/h.conf")));
        assert_eq!(
            (m.source, m.backend),
            (MainConfigSource::CommandLine, Backend::Hyprlang)
        );

        let mut fs = Fake::home();
        fs.env.insert("HYPRLAND_CONFIG", "/e/h.lua");
        let m = find_main_config(&fs, None);
        assert_eq!(
            (m.path, m.source),
            (PathBuf::from("/e/h.lua"), MainConfigSource::Environment)
        );

        let mut fs = Fake::home();
        fs.env.insert("XDG_CONFIG_HOME", "/xdg");
        let m = find_main_config(&fs, None);
        assert_eq!(m.path, PathBuf::from("/xdg/hypr/hyprland.lua"));
        assert_eq!(m.source, MainConfigSource::Default);
    }

    #[test]
    fn scans_requires_and_package_path() {
        let (req, add) = scan_lua_loading(CAELESTIA_MAIN, Some("/home/u"));
        let names: Vec<_> = req.iter().map(|r| r.module.as_str()).collect();
        assert_eq!(names, ["hyprland.env", "hypr-user"]);
        assert_eq!(req[1].line, 7);
        assert_eq!(
            add,
            [PathAddition {
                template: "/home/u/.config/caelestia/?.lua".to_owned(),
                appended: true
            }]
        );
        let (_, add) = scan_lua_loading("package.path = '/opt/?.lua;' .. package.path", None);
        assert!(!add[0].appended);
        let (req, _) = scan_lua_loading("require 'x' require(name)", None);
        assert_eq!(req.len(), 1);
        assert_eq!(scan_lua_loading("x = \"", None), (Vec::new(), Vec::new()));
    }

    #[test]
    fn search_path_order() {
        let add = [PathAddition {
            template: "/c/?.lua".to_owned(),
            appended: true,
        }];
        let p = require_search_path(Path::new("/h/.config/hypr"), None, &add);
        assert_eq!(p[0], "/h/.config/hypr/?.lua");
        assert_eq!(p[1], "/h/.config/hypr/?/init.lua");
        assert_eq!(p.last().unwrap(), "/c/?.lua");
        assert!(p.contains(&"./?.lua".to_owned()));
        let p = require_search_path(Path::new("/d"), Some("/x/?.lua;;"), &[]);
        assert_eq!(p[2], "/x/?.lua");
        assert!(p.contains(&"/usr/share/lua/5.5/?.lua".to_owned()));
    }

    #[test]
    fn caelestia_preset_resolves_hypr_user() {
        let fs = Fake::home()
            .file("/home/u/.config/hypr/hyprland.lua", CAELESTIA_MAIN)
            .file("/home/u/.config/caelestia/hypr-user.lua", "return {}\n");
        let t = resolve_target(&fs, &TargetRequest::default());
        assert_eq!(
            t.path,
            PathBuf::from("/home/u/.config/caelestia/hypr-user.lua")
        );
        assert_eq!(t.reason, TargetReason::Caelestia);
        assert_eq!(t.backend, Backend::Lua);
        assert!(t.warnings.is_empty());
    }

    #[test]
    fn caelestia_preset_reports_shadowing() {
        let fs = Fake::home()
            .file("/home/u/.config/hypr/hyprland.lua", CAELESTIA_MAIN)
            .file("/home/u/hypr-user.lua", "")
            .file("/home/u/.config/caelestia/hypr-user.lua", "");
        let t = resolve_target(&fs, &TargetRequest::default());
        // ./?.lua relative to Hyprland's cwd ($HOME) comes before caelestia.
        assert_eq!(t.path, PathBuf::from("/home/u/hypr-user.lua"));
        assert_eq!(t.warnings.len(), 1);
        assert!(t.warnings[0].contains("caelestia/hypr-user.lua is shadowed"));
    }

    #[test]
    fn caelestia_preset_before_the_file_exists() {
        let fs = Fake::home().file("/home/u/.config/hypr/hyprland.lua", CAELESTIA_MAIN);
        let t = resolve_target(&fs, &TargetRequest::default());
        assert_eq!(
            t.path,
            PathBuf::from("/home/u/.config/caelestia/hypr-user.lua")
        );
    }

    #[test]
    fn plain_lua_config_is_the_target() {
        let fs = Fake::home().file(
            "/home/u/.config/hypr/hyprland.lua",
            "hl.monitor({ output = \"DP-1\" })\nrequire(\"hypr-user\")\n",
        );
        let t = resolve_target(&fs, &TargetRequest::default());
        assert_eq!(t.path, PathBuf::from("/home/u/.config/hypr/hyprland.lua"));
        assert_eq!(t.reason, TargetReason::MainConfig(MainConfigSource::Xdg));
    }

    #[test]
    fn explicit_and_settings_targets_win() {
        let fs = Fake::home().file("/home/u/.config/hypr/hyprland.lua", CAELESTIA_MAIN);
        let req = TargetRequest {
            explicit: Some("/x/monitors.conf".into()),
            settings_target: Some("/y.lua".into()),
            ..TargetRequest::default()
        };
        let t = resolve_target(&fs, &req);
        assert_eq!(
            (t.path.as_path(), t.backend),
            (Path::new("/x/monitors.conf"), Backend::Hyprlang)
        );
        let req = TargetRequest {
            settings_target: Some("/y.txt".into()),
            backend: Some(Backend::Lua),
            ..TargetRequest::default()
        };
        let t = resolve_target(&fs, &req);
        assert_eq!(
            (t.reason, t.backend),
            (TargetReason::Settings, Backend::Lua)
        );
    }

    #[test]
    fn module_resolution_uses_dots_and_cwd() {
        let fs = Fake::default()
            .file("/w/a/b.lua", "")
            .file("/w/a/b/init.lua", "");
        let found = resolve_module(
            &fs,
            "a.b",
            &["./?.lua".to_owned(), "./?/init.lua".to_owned()],
            Path::new("/w"),
        );
        assert_eq!(
            found,
            [
                PathBuf::from("/w/a/b.lua"),
                PathBuf::from("/w/a/b/init.lua")
            ]
        );
    }

    #[test]
    fn system_probe_parses_proc_blobs() {
        let p = SystemProbe::from_environ_blob(b"HOME=/home/u\0HYPRLAND_CONFIG=/x.lua\0BROKEN\0");
        assert_eq!(p.var("HOME").as_deref(), Some("/home/u"));
        assert_eq!(p.var("HYPRLAND_CONFIG").as_deref(), Some("/x.lua"));
        assert_eq!(p.var("BROKEN"), None);
        assert_eq!(SystemProbe::split_nul(b"a\0b\0"), ["a", "b"]);
        let own = SystemProbe::default();
        assert!(own.var("PATH").is_some());
        assert!(!own.is_file(Path::new("/nonexistent/hyprtilt")));
        assert!(own.read(Path::new("/nonexistent/hyprtilt")).is_none());
    }
}
