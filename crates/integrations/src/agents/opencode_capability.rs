//! Ask the selected native CLI for its database contract in disposable roots.
//! No release label authorizes a borrowed database migration.
use hiroute_domain::delegation::{
    NativeDependencyCheckV1, NativeDependencyFailureReasonV1 as Reason, NativeDependencyFailureV1,
};
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path, process::Command, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    id: String,
}

pub fn opencode_native_format(
    selected: &Path,
    search_path: &str,
) -> Result<Vec<String>, NativeDependencyFailureV1> {
    let failure = |reason| NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::OpenCodeDatabase,
        reason,
    };
    let binary = super::opencode_installation::opencode_native_executable(selected)
        .map_err(|_| failure(Reason::Unsupported))?
        .ok_or_else(|| failure(Reason::Unavailable))?;
    let root = super::probe_files::temporary("hiroute-opencode-format-")
        .map_err(|_| failure(Reason::Unavailable))?;
    let root_path = std::fs::canonicalize(root.path()).map_err(|_| failure(Reason::Unavailable))?;
    let mut command = Command::new(&binary);
    command
        .args([
            "db",
            "--format",
            "json",
            "SELECT id FROM migration ORDER BY id",
        ])
        .env_clear()
        .env("PATH", search_path)
        .env("HOME", &root_path)
        .env("TMPDIR", &root_path)
        .env("TMP", &root_path)
        .env("TEMP", &root_path)
        .env("XDG_CONFIG_HOME", root_path.join("config"))
        .env("XDG_DATA_HOME", root_path.join("data"))
        .env("XDG_CACHE_HOME", root_path.join("cache"))
        .env("XDG_STATE_HOME", root_path.join("state"))
        .env("OPENCODE_DB", root_path.join("native.db"))
        .env("OPENCODE_PURE", "true")
        .env("OPENCODE_DISABLE_DEFAULT_PLUGINS", "true")
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .env("OPENCODE_DISABLE_MODELS_FETCH", "true")
        .current_dir(&root_path);
    #[cfg(windows)]
    for (name, value) in
        super::windows_native_system_environment().map_err(|_| failure(Reason::Unavailable))?
    {
        command.env(name, value);
    }
    let result =
        super::native_probe_process::capture(command, false, Duration::from_secs(10), 64 * 1024);
    if result
        .as_ref()
        .is_err_and(|reason| *reason == Reason::CleanupFailed)
    {
        // Do not remove roots while a native process might still own them.
        let _ = root.keep();
    }
    let bytes = result.map_err(failure)?;
    let rows: Vec<Migration> =
        serde_json::from_slice(&bytes).map_err(|_| failure(Reason::InvalidOutput))?;
    if rows.is_empty()
        || rows.len() > 128
        || rows.iter().any(|row| {
            row.id.is_empty() || row.id.len() > 256 || row.id.chars().any(char::is_control)
        })
    {
        return Err(failure(Reason::Unsupported));
    }
    let ids = rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(failure(Reason::Unsupported));
    }
    Ok(ids)
}
