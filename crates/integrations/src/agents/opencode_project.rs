//! Prove the pinned native Git stop using the Worker's filtered HOME/XDG/PATH.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{OpenCodeNativeContext, QoderNativeError, qoder_error};

pub(super) fn confirm_root(
    native: &OpenCodeNativeContext,
    root: &Path,
    search_path: &str,
) -> Result<(), QoderNativeError> {
    let unavailable = || qoder_error("OpenCode project boundary unavailable");
    // Resolve only the actual child search path. Discovery's ambient fallbacks
    // cannot prove which Git the native process will execute.
    let mut selected = None;
    for directory in std::env::split_paths(search_path) {
        if !directory.is_absolute() {
            return Err(unavailable());
        }
        #[cfg(windows)]
        let names = ["git", "git.com", "git.exe", "git.bat", "git.cmd"];
        #[cfg(not(windows))]
        let names = ["git"];
        for name in names {
            let candidate = directory.join(name);
            if !candidate.try_exists().map_err(|_| unavailable())? {
                continue;
            }
            #[cfg(windows)]
            if name != "git.exe" {
                return Err(unavailable());
            }
            selected = super::executable::resolve(&candidate).map_err(|_| unavailable())?;
            break;
        }
        if selected.is_some() {
            break;
        }
    }
    let git = selected.ok_or_else(unavailable)?;
    // Match Git.repo.discover at OpenCode 1.19.3. Any failure is a scoped
    // capability refusal; it never licenses stopping at an unverified .git.
    for option in ["--show-toplevel", "--git-dir", "--git-common-dir"] {
        let mut command = Command::new(&git);
        command
            .args(["rev-parse", option])
            .current_dir(root)
            .env_clear()
            .env("PATH", search_path)
            .env("HOME", &native.home)
            .env(
                "XDG_CONFIG_HOME",
                native.config_root.parent().ok_or_else(unavailable)?,
            );
        #[cfg(windows)]
        for (key, value) in super::windows_runtime::windows_native_system_environment()
            .map_err(|_| unavailable())?
        {
            command.env(key, value);
        }
        let output =
            super::native_probe_process::capture(command, false, Duration::from_secs(2), 16 * 1024)
                .map_err(|_| unavailable())?;
        let value = std::str::from_utf8(&output)
            .map_err(|_| unavailable())?
            .trim();
        if value.is_empty() || value.contains(['\0', '\r', '\n']) {
            return Err(unavailable());
        }
        if option == "--show-toplevel" {
            let top = PathBuf::from(value);
            if !top.is_absolute() || std::fs::canonicalize(top).map_err(|_| unavailable())? != root
            {
                return Err(unavailable());
            }
        }
    }
    Ok(())
}
