//! Resolve the desktop package and its own agent. A CLI on PATH is never a substitute.
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub(super) struct Installation {
    executable: PathBuf,
    pub engine: PathBuf,
}

fn paired(executable: PathBuf) -> Option<Installation> {
    if !executable.is_absolute() || !executable.is_file() {
        return None;
    }
    let directory = executable.parent()?;
    let leaves = if cfg!(windows) {
        ["resources/codex-cli/bin/codex.exe", "resources/codex.exe"]
    } else {
        ["resources/codex-cli/bin/codex", "resources/codex"]
    };
    let engine = leaves
        .into_iter()
        .map(|leaf| directory.join(leaf))
        .find(|path| path.is_file());
    Some(Installation {
        executable,
        engine: engine?,
    })
}

#[cfg(windows)]
fn cached_engine() -> Option<PathBuf> {
    let root = hiroute_platform::windows::background::local_app_data()
        .ok()?
        .join("OpenAI/Codex/bin");
    cached_engine_in(&root)
}

#[cfg(any(windows, test))]
fn cached_engine_in(root: &Path) -> Option<PathBuf> {
    let current = root.join("codex.exe");
    if current.is_file() {
        // The app owns this stable cache entry, which can point at a versioned runtime.
        // Resolve it here; ordinary borrowed executable admission still rejects reparses.
        return current.canonicalize().ok();
    }
    let mut entries = std::fs::read_dir(root)
        .ok()?
        .take(128)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let parts = name
                .split('.')
                .map(str::parse::<u64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let path = entry.path().join("codex.exe");
            (parts.len() >= 3 && path.is_file()).then_some((parts, path))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries
        .into_iter()
        .next()
        .and_then(|(_, path)| path.canonicalize().ok())
}

#[cfg(windows)]
pub(super) fn installed() -> Option<Installation> {
    use std::{io::Read, os::windows::process::CommandExt};
    let powershell = hiroute_platform::windows::launcher::system_executable(Path::new(
        "WindowsPowerShell/v1.0/powershell.exe",
    ))
    .ok()?;
    // Fixed current-user package query. No shell text, app path or environment comes from IPC.
    let script = r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $p=Get-AppxPackage -Name OpenAI.Codex | Sort-Object Version -Descending | Select-Object -First 1; if ($p) { $m=$p | Get-AppxPackageManifest; $a=$m.Package.Applications.Application | Where-Object { $_.Executable -match '(?i)(^|[\\/])(ChatGPT|Codex)\.exe$' } | Select-Object -First 1; if ($a) { [PSCustomObject]@{executable=[IO.Path]::Combine($p.InstallLocation,[string]$a.Executable)} | ConvertTo-Json -Compress } }"#;
    let mut child = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let output = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.take(16385).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader.join().ok()?.ok()?;
    if !success || bytes.len() > 16384 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let executable = PathBuf::from(value["executable"].as_str()?);
    paired(executable.clone()).or_else(|| {
        (executable.is_absolute() && executable.is_file()).then_some(Installation {
            executable,
            engine: cached_engine()?,
        })
    })
}

#[cfg(target_os = "linux")]
pub(super) fn installed() -> Option<Installation> {
    // Distribution packages expose `chatgpt`; canonicalization locates its resources.
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|root| root.is_absolute())
        .map(|root| root.join("chatgpt"))
        .chain([
            PathBuf::from("/opt/ChatGPT/chatgpt"),
            PathBuf::from("/opt/chatgpt/chatgpt"),
        ])
        .filter_map(|path| path.canonicalize().ok())
        .find_map(paired)
}

fn command(app: &Path, codex_home: &Path, user_data: &Path) -> Command {
    let mut command = Command::new(app);
    let mut argument = std::ffi::OsString::from("--user-data-dir=");
    argument.push(user_data);
    command
        .arg(argument)
        .env("CODEX_HOME", codex_home)
        .env("CODEX_SQLITE_HOME", codex_home)
        .env("CODEX_ELECTRON_USER_DATA_PATH", user_data)
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008);
    }
    command
}

#[cfg(not(target_os = "macos"))]
pub(super) fn launch(codex_home: &Path, user_data: &Path) -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY")
        .filter(|v| !v.is_empty())
        .is_none()
        && std::env::var_os("WAYLAND_DISPLAY")
            .filter(|v| !v.is_empty())
            .is_none()
    {
        return Err("CODEX_DESKTOP_DISPLAY_REQUIRED");
    }
    let app = installed().ok_or("CODEX_DESKTOP_NOT_INSTALLED")?;
    let mut child = command(&app.executable, codex_home, user_data)
        .spawn()
        .map_err(|_| "CODEX_DESKTOP_LAUNCH_FAILED")?;
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("CODEX_DESKTOP_LAUNCH_FAILED")
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(_) => return Err("CODEX_DESKTOP_LAUNCH_FAILED"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Codex Desktop installed for the current Windows user"]
    fn actual_windows_codex_desktop_package_has_its_own_engine() {
        let installation = installed().expect("CODEX_DESKTOP_CURRENT_USER_PACKAGE_REQUIRED");
        assert!(installation.executable.is_absolute() && installation.executable.is_file());
        assert!(installation.engine.is_absolute() && installation.engine.is_file());
        let mut engine = Command::new(installation.engine)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the installed Desktop engine");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match engine.try_wait() {
                Ok(Some(status)) => {
                    assert!(status.success(), "installed Desktop engine version failed");
                    break;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(None) => {
                    let _ = engine.kill();
                    let _ = engine.wait();
                    panic!("installed Desktop engine version timed out");
                }
                Err(_) => {
                    let _ = engine.kill();
                    let _ = engine.wait();
                    panic!("installed Desktop engine status unavailable");
                }
            }
        }
    }

    #[test]
    fn codex_desktop_cache_prefers_current_entry_and_compares_versions_numerically() {
        let root = tempfile::tempdir().unwrap();
        for version in ["1.9.0", "1.10.0", "unrelated"] {
            let directory = root.path().join(version);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("codex.exe"), "fixture").unwrap();
        }
        assert_eq!(
            cached_engine_in(root.path()).unwrap(),
            root.path().join("1.10.0/codex.exe").canonicalize().unwrap()
        );
        let current = root.path().join("codex.exe");
        std::fs::write(&current, "current").unwrap();
        assert_eq!(
            cached_engine_in(root.path()).unwrap(),
            current.canonicalize().unwrap()
        );
    }
    #[test]
    fn codex_desktop_package_requires_its_own_engine_and_isolates_only_child_environment() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("ChatGPT.exe");
        std::fs::write(&executable, "fixture").unwrap();
        assert!(paired(executable.clone()).is_none());
        std::fs::create_dir_all(root.path().join("resources/codex-cli/bin")).unwrap();
        let engine = root.path().join(if cfg!(windows) {
            "resources/codex-cli/bin/codex.exe"
        } else {
            "resources/codex-cli/bin/codex"
        });
        std::fs::write(&engine, "fixture").unwrap();
        let pair = paired(executable.clone()).unwrap();
        assert_eq!(pair.engine, engine);
        assert_eq!(pair.executable, executable);
        let home = root.path().join("private folder/用户'$(touch no)/codex");
        let data = root.path().join("private folder/desktop");
        let launch = command(&executable, &home, &data);
        assert_eq!(launch.get_program(), executable.as_os_str());
        assert_eq!(
            launch.get_args().collect::<Vec<_>>(),
            vec![format!("--user-data-dir={}", data.display()).as_str()]
        );
        let env = launch
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        for key in ["CODEX_HOME", "CODEX_SQLITE_HOME"] {
            assert_eq!(env[std::ffi::OsStr::new(key)], Some(home.as_os_str()));
        }
        assert_eq!(
            env[std::ffi::OsStr::new("CODEX_ELECTRON_USER_DATA_PATH")],
            Some(data.as_os_str())
        );
        assert!(!env.contains_key(std::ffi::OsStr::new("HOME")));
        assert!(!env.contains_key(std::ffi::OsStr::new("USERPROFILE")));
    }
}
