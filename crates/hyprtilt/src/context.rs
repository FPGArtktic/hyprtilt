// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! What every command needs: settings, the connection to Hyprland, the
//! compositor's process, and the file to edit.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprtilt_core::config::{self, MainConfig, SystemProbe, Target, TargetRequest};
use hyprtilt_core::document::{Backend, SaveOptions};
use hyprtilt_core::ipc::fake::FakeHyprland;
use hyprtilt_core::ipc::socket::{self, Instance, SocketIpc};
use hyprtilt_core::ipc::{HyprlandIpc, StatusInfo};
use hyprtilt_core::lua::syntax::Luac;
use hyprtilt_core::proc::{self, ProcessInfo};
use hyprtilt_core::profile::ProfileStore;
use hyprtilt_core::settings::{self, Settings, expand_tilde};

use crate::cli::{BackendArg, GlobalArgs};
use crate::error::AppError;

/// How long to wait for one IPC reply.
const IPC_TIMEOUT: Duration = Duration::from_secs(5);

/// The shared state of a command.
pub(crate) struct Context {
    /// Global options.
    pub(crate) global: GlobalArgs,
    /// Settings from config.toml.
    pub(crate) settings: Settings,
    /// `$HOME`.
    pub(crate) home: Option<String>,
    ipc: Result<Box<dyn HyprlandIpc>, String>,
    instance: Option<Instance>,
    fake_config: Option<PathBuf>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

impl Context {
    /// Read the settings and connect to Hyprland (or the in-memory one).
    pub(crate) fn new(global: GlobalArgs) -> Result<Context, AppError> {
        let home = env("HOME");
        let settings_file =
            settings::config_dir(env("XDG_CONFIG_HOME").as_deref(), home.as_deref())
                .join("config.toml");
        let settings = Settings::load(&settings_file, home.as_deref())?;
        if let Some(path) = &global.fake_hyprland {
            let fake = FakeHyprland::from_file(path).map_err(AppError::Other)?;
            // A running compositor has loaded its configuration.
            fake.reload()?;
            return Ok(Context {
                fake_config: fake.config_path(),
                ipc: Ok(Box::new(fake)),
                instance: None,
                global,
                settings,
                home,
            });
        }
        let root = socket::runtime_root(env("XDG_RUNTIME_DIR").as_deref());
        let (ipc, instance): (Result<Box<dyn HyprlandIpc>, String>, _) =
            match socket::find_instance(&root, env("HYPRLAND_INSTANCE_SIGNATURE").as_deref()) {
                Ok(instance) => (
                    Ok(Box::new(SocketIpc::new(instance.socket(), IPC_TIMEOUT))),
                    Some(instance),
                ),
                Err(e) => (Err(e.to_string()), None),
            };
        Ok(Context {
            global,
            settings,
            home,
            ipc,
            instance,
            fake_config: None,
        })
    }

    /// The connection to Hyprland.
    pub(crate) fn ipc(&self) -> Result<&dyn HyprlandIpc, AppError> {
        self.ipc
            .as_ref()
            .map(AsRef::as_ref)
            .map_err(|e| AppError::Ipc(e.clone()))
    }

    /// What `/proc` shows about the compositor.
    pub(crate) fn process(&self) -> Option<ProcessInfo> {
        let pid = self.instance.as_ref()?.pid?;
        Some(proc::inspect(Path::new("/proc"), pid))
    }

    /// The compositor's configuration provider (`None` when it cannot be
    /// asked).
    pub(crate) fn status(&self) -> Option<StatusInfo> {
        let ipc = self.ipc().ok()?;
        match ipc.status() {
            Ok(Some(status)) => Some(status),
            // Hyprland before 0.55 has no status request and only hyprlang.
            Ok(None) => Some(StatusInfo {
                config_provider: "hyprlang".to_owned(),
                backend: String::new(),
            }),
            Err(_) => None,
        }
    }

    fn probe(&self) -> SystemProbe {
        match self.process() {
            Some(p) if !p.environ.is_empty() => SystemProbe {
                environ: Some(p.environ),
            },
            _ => SystemProbe::default(),
        }
    }

    fn request(&self) -> TargetRequest {
        let process = self.process();
        let explicit = self
            .global
            .file
            .as_ref()
            .map(|f| expand_tilde(f, self.home.as_deref()))
            .or_else(|| self.fake_config.clone());
        TargetRequest {
            explicit,
            backend: self.global.backend.map(|b| match b {
                BackendArg::Lua => Backend::Lua,
                BackendArg::Hyprlang => Backend::Hyprlang,
            }),
            settings_target: self.settings.target.clone(),
            cmdline_config: process
                .as_ref()
                .and_then(|p| config::config_from_cmdline(&p.cmdline)),
            cwd: process.and_then(|p| p.cwd),
        }
    }

    /// Hyprland's main configuration file.
    pub(crate) fn main_config(&self) -> MainConfig {
        let request = self.request();
        config::find_main_config(&self.probe(), request.cmdline_config.as_deref())
    }

    /// The file to edit, with the backend from `--backend`, the settings or
    /// the file name.
    pub(crate) fn target(&self) -> Target {
        let mut request = self.request();
        if request.backend.is_none() {
            request.backend = self.settings.backend;
        }
        config::resolve_target(&self.probe(), &request)
    }

    /// Refuse to write a file Hyprland cannot read with its current
    /// configuration language.
    pub(crate) fn check_backend(&self, target: &Target) -> Result<(), AppError> {
        let Some(status) = self.status() else {
            return Ok(());
        };
        let runs = match status.config_provider.as_str() {
            "lua" => Backend::Lua,
            "hyprlang" => Backend::Hyprlang,
            _ => return Ok(()),
        };
        if runs == target.backend {
            return Ok(());
        }
        Err(AppError::Config(format!(
            "Hyprland runs a {runs} configuration and would never read {} ({}); choose another file with --file",
            target.path.display(),
            target.backend
        )))
    }

    /// What the writers need to know about the running Hyprland.
    pub(crate) fn save_options(&self) -> SaveOptions {
        let hyprland = self
            .ipc()
            .ok()
            .and_then(|ipc| ipc.version().ok())
            .and_then(|v| v.parsed());
        SaveOptions { hyprland }
    }

    /// The Lua compiler for syntax checks.
    pub(crate) fn luac() -> Option<Luac> {
        Luac::find(&env("PATH").unwrap_or_default())
    }

    /// The profile directory.
    pub(crate) fn profiles(&self) -> ProfileStore {
        let dir = settings::config_dir(env("XDG_CONFIG_HOME").as_deref(), self.home.as_deref());
        ProfileStore::new(dir.join("profiles"))
    }
}
