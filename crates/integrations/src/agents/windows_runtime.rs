//! Windows native execution uses argv and held Jobs, never evaluates npm .cmd text.
use hiroute_platform::windows::job::{Child, Launch, Stream};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub fn windows_agent_command(
    program: &Path,
    args: &[PathBuf],
    search: &str,
) -> io::Result<(PathBuf, Vec<PathBuf>)> {
    if program
        .extension()
        .is_some_and(|ext| ext.as_encoded_bytes().eq_ignore_ascii_case(b"exe"))
    {
        return Ok((program.to_owned(), args.to_vec()));
    }
    let entry = if program
        .extension()
        .is_some_and(|ext| ext.as_encoded_bytes().eq_ignore_ascii_case(b"cmd"))
    {
        windows_npm_entry(program)?
    } else {
        program.to_owned()
    };
    let node = node_path(Some(std::ffi::OsStr::new(search)), &entry)?;
    let mut args = args.to_vec();
    args.insert(0, entry);
    Ok((node, args))
}

/// OS selectors come from the kernel's system directory, never a caller shell override.
pub fn windows_native_system_environment() -> io::Result<[(&'static str, OsString); 4]> {
    let cmd = hiroute_platform::windows::launcher::system_executable(Path::new("cmd.exe"))?;
    let root = cmd
        .parent()
        .and_then(Path::parent)
        .filter(|root| root.is_absolute())
        .ok_or_else(|| io::Error::other("native system root"))?;
    Ok([
        ("SystemRoot", root.as_os_str().to_owned()),
        ("WINDIR", root.as_os_str().to_owned()),
        ("COMSPEC", cmd.into_os_string()),
        ("PATHEXT", ".COM;.EXE;.BAT;.CMD".into()),
    ])
}

pub fn windows_claude_version(
    binary: &Path,
    search_path: &str,
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    let home = super::probe_files::temporary("hiroute-claude-version-")?;
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("PATH", search_path)
        .env("CLAUDE_CONFIG_DIR", home.path().join(".claude"))
        .current_dir(home.path());
    std::fs::create_dir(home.path().join(".claude"))?;
    let result = bounded(&command, false, timeout, 4096);
    if result.as_ref().is_err_and(cleanup_unknown) {
        let _ = home.keep();
    }
    result.map(|bytes| bytes.to_vec())
}

/// Resolve only a declared bin from a known npm producer next to its generated shim.
pub fn windows_npm_entry(shim: &Path) -> io::Result<PathBuf> {
    windows_npm_entry_with_inspection(shim, |_| Ok(()))
}

/// The caller may retain reparse-free manifest and declared-entry identities
/// until its validation or launch completes. Inspection happens before either
/// pathname is read or canonicalized; the existing npm producer parser is shared.
pub fn windows_npm_entry_with_inspection(
    shim: &Path,
    mut inspect: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let name = shim
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("npm entry"))?;
    let package = match name.to_ascii_lowercase().as_str() {
        "codex" => "@openai/codex",
        "claude" => "@anthropic-ai/claude-code",
        "codex-acp" => "@agentclientprotocol/codex-acp",
        "claude-agent-acp" => "@agentclientprotocol/claude-agent-acp",
        "pi" => "@earendil-works/pi-coding-agent",
        "dsh" => "@deepseek-ai/dsh",
        "qoder" | "qodercli" => "@qoder-ai/qodercli",
        "opencode" => "opencode-ai",
        _ => return Err(io::Error::other("unknown npm producer")),
    };
    let bin_directory = shim.parent().ok_or_else(|| io::Error::other("npm root"))?;
    // npm's two declared Windows layouts: a global prefix, or a local
    // node_modules/.bin. Never interpret the generated command script.
    let modules = if bin_directory
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().eq_ignore_ascii_case(b".bin"))
        && bin_directory.parent().is_some_and(|parent| {
            parent.file_name().is_some_and(|name| {
                name.as_encoded_bytes()
                    .eq_ignore_ascii_case(b"node_modules")
            })
        }) {
        bin_directory
            .parent()
            .expect("verified npm local parent")
            .to_owned()
    } else {
        bin_directory.join("node_modules")
    };
    let root = modules.join(package);
    let manifest = root.join("package.json");
    inspect(&manifest)?;
    if std::fs::metadata(&manifest)?.len() > 64 * 1024 {
        return Err(io::Error::other("npm manifest bound"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest)?).map_err(io::Error::other)?;
    if value["name"] != package {
        return Err(io::Error::other("npm producer mismatch"));
    }
    let bin = value["bin"]
        .as_str()
        .or_else(|| {
            value["bin"].as_object()?.iter().find_map(|(key, value)| {
                key.eq_ignore_ascii_case(name)
                    .then(|| value.as_str())
                    .flatten()
            })
        })
        .ok_or_else(|| io::Error::other("npm bin"))?;
    let entry = root.join(bin);
    inspect(&entry)?;
    let root = std::fs::canonicalize(root)?;
    let entry = std::fs::canonicalize(root.join(bin))?;
    if !entry.starts_with(&root) || !entry.is_file() {
        return Err(io::Error::other("npm bin identity"));
    }
    Ok(entry)
}

pub(super) fn node_path(search: Option<&std::ffi::OsStr>, entry: &Path) -> io::Result<PathBuf> {
    let mut directories = vec![entry.parent().unwrap_or(entry).to_owned()];
    if let Some(search) = search {
        directories.extend(std::env::split_paths(search).filter(|path| path.is_absolute()));
    }
    for directory in directories {
        let node = directory.join("node.exe");
        if node.is_file() {
            return std::fs::canonicalize(node);
        }
    }
    Err(io::Error::other("selected command Node unavailable"))
}

pub(super) fn spawn(
    command: &Command,
    inherit: bool,
    input: Stream<'_>,
    output: Stream<'_>,
    error: Stream<'_>,
) -> io::Result<Child> {
    let selected = Path::new(command.get_program());
    let mut args: Vec<_> = command.get_args().map(ToOwned::to_owned).collect();
    let mut env: std::collections::BTreeMap<OsString, OsString> = if inherit {
        std::env::vars_os().collect()
    } else {
        Default::default()
    };
    for (key, value) in command.get_envs() {
        // Windows logical keys are case-insensitive, including absent protected keys.
        env.retain(|name, _| {
            !name
                .as_encoded_bytes()
                .eq_ignore_ascii_case(key.as_encoded_bytes())
        });
        if let Some(value) = value {
            env.insert(key.to_owned(), value.to_owned());
        }
    }
    for (key, value) in windows_native_system_environment()? {
        env.retain(|name, _| !name.as_encoded_bytes().eq_ignore_ascii_case(key.as_bytes()));
        env.insert(key.into(), value);
    }
    let value = |name: &str, env: &std::collections::BTreeMap<OsString, OsString>| {
        env.iter()
            .find(|(key, _)| key.as_encoded_bytes().eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, value)| value.clone())
    };
    if let Some(home) = value("HOME", &env) {
        env.retain(|key, _| !key.as_encoded_bytes().eq_ignore_ascii_case(b"USERPROFILE"));
        env.insert("USERPROFILE".into(), home.clone());
        for (key, suffix) in [
            ("APPDATA", "AppData/Roaming"),
            ("LOCALAPPDATA", "AppData/Local"),
        ] {
            if value(key, &env).is_none() {
                env.insert(
                    key.into(),
                    PathBuf::from(&home).join(suffix).into_os_string(),
                );
            }
        }
    }
    let program = if selected
        .extension()
        .is_some_and(|ext| ext.as_encoded_bytes().eq_ignore_ascii_case(b"exe"))
    {
        selected.to_owned()
    } else {
        let entry = if selected
            .extension()
            .is_some_and(|ext| ext.as_encoded_bytes().eq_ignore_ascii_case(b"cmd"))
        {
            windows_npm_entry(selected)?
        } else {
            selected.to_owned()
        };
        let search = value("PATH", &env);
        let node = node_path(search.as_deref(), &entry)?;
        args.insert(0, entry.into_os_string());
        node
    };
    let env = env.into_iter().collect::<Vec<_>>();
    Child::spawn(Launch {
        program: &program,
        args: &args,
        env: &env,
        cwd: command.get_current_dir(),
        stdin: input,
        stdout: output,
        stderr: error,
    })
}

/// Native one-shot probes have no stdin and retain their exact Job through stop.
/// File capture keeps synchronous output reads outside the execution deadline.
pub fn spawn_windows_native_probe(
    command: &Command,
    inherit: bool,
    output: &std::fs::File,
    error: Option<&std::fs::File>,
) -> io::Result<super::NativeProbeProcess> {
    spawn(
        command,
        inherit,
        Stream::Null,
        Stream::File(output),
        error.map_or(Stream::Null, Stream::File),
    )
    .map(super::NativeProbeProcess::new)
}

/// Capture to a private regular file so no synchronous pipe read can defeat the deadline.
pub(super) fn bounded(
    command: &Command,
    inherit: bool,
    timeout: Duration,
    limit: usize,
) -> io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let capture =
        tempfile::tempdir().map_err(|error| native_probe_stage("capture-create", error))?;
    // The tempfile container may inherit an administrator TokenOwner. The file
    // carrying native output is created atomically with the current user's
    // descriptor and retains its exact handle through capture and final read.
    let writer =
        hiroute_platform::windows::security::create_owner_only_file(&capture.path().join("output"))
            .map_err(|error| native_probe_stage("capture-private-create", error))?;
    let child = spawn(
        command,
        inherit,
        Stream::Null,
        Stream::File(&writer),
        Stream::Null,
    )
    .map_err(|error| {
        let error = native_probe_stage("job-spawn", error);
        io::Error::other(LaunchFailure(error.raw_os_error()))
    })?;
    let start = Instant::now();
    let result = (|| {
        loop {
            if writer
                .metadata()
                .map_err(|error| native_probe_stage("capture-metadata", error))?
                .len()
                > limit as u64
            {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native output bound",
                ));
            }
            if let Some(code) = child
                .exit_code()
                .map_err(|error| native_probe_stage("job-exit-query", error))?
            {
                break if code == 0 {
                    Ok(())
                } else {
                    #[cfg(test)]
                    eprintln!("native acceptance process stage=job-exit status={code}");
                    Err(io::Error::other(ProcessExit(code)))
                };
            }
            if start.elapsed() >= timeout {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native check timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    let cleanup = (|| {
        child
            .terminate()
            .map_err(|error| native_probe_stage("job-terminate", error))?;
        let stop = Instant::now();
        while !child
            .empty()
            .map_err(|error| native_probe_stage("job-empty-query", error))?
        {
            if stop.elapsed() >= Duration::from_secs(1) {
                return Err(io::Error::other("native cleanup failed"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    if cleanup.is_err() {
        let _ = capture.keep();
        return Err(io::Error::other(CleanupUnknown));
    }
    // Even after the root exits, a descendant can still write stdout. Stop and
    // observe the whole held Job before reading the final bounded capture.
    result?;
    read_held_capture(&writer, limit)
        .map_err(|error| native_probe_stage("capture-final-read", error))
}

fn read_held_capture(
    file: &std::fs::File,
    limit: usize,
) -> io::Result<zeroize::Zeroizing<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native output bound",
        ));
    }
    Ok(bytes)
}

fn native_probe_stage(stage: &'static str, error: io::Error) -> io::Error {
    // Preserve classification and launch_failure_code. Test diagnostics contain
    // only a fixed stage, kind and OS status; native output may contain secrets.
    #[cfg(test)]
    eprintln!(
        "native acceptance process stage={stage} kind={:?} os={:?}",
        error.kind(),
        error.raw_os_error()
    );
    #[cfg(not(test))]
    let _ = stage;
    error
}

#[derive(Debug, thiserror::Error)]
#[error("native process failed")]
struct ProcessExit(i32);

pub(super) fn process_exit_code(error: &io::Error) -> Option<i32> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ProcessExit>())
        .map(|failure| failure.0)
}

#[derive(Debug, thiserror::Error)]
#[error("native process did not start")]
struct LaunchFailure(Option<i32>);

pub(super) fn launch_failure_code(error: &io::Error) -> Option<Option<i32>> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<LaunchFailure>())
        .map(|failure| failure.0)
}

#[derive(Debug)]
struct CleanupUnknown;
impl std::fmt::Display for CleanupUnknown {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("native cleanup failed")
    }
}
impl std::error::Error for CleanupUnknown {}
pub(super) fn cleanup_unknown(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|error| error.is::<CleanupUnknown>())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_FIXTURE: &str = "agents::windows_runtime::tests::windows_native_environment_fixture";

    #[test]
    #[ignore = "selected only as the actual native environment child"]
    fn windows_native_environment_fixture() {
        let values = ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"]
            .into_iter()
            .map(|key| (key, std::env::var(key).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        println!(
            "WINDOW_NATIVE_SYSTEM_ENV={}",
            serde_json::to_string(&values).unwrap()
        );
    }

    #[test]
    fn windows_native_system_selectors_override_ambient_shell_paths() {
        let home = super::super::probe_files::temporary("hiroute-system-environment-").unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", ENV_FIXTURE, "--nocapture"])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "")
            .env("systemroot", r"C:\untrusted-root")
            .env("windir", r"C:\untrusted-windows")
            .env("comspec", r"C:\untrusted-shell.cmd")
            .env("pathext", ".UNTRUSTED")
            .current_dir(home.path());
        let output = bounded(&command, false, Duration::from_secs(10), 4096).unwrap();
        let text = std::str::from_utf8(&output).unwrap();
        let values: std::collections::BTreeMap<String, String> = serde_json::from_str(
            text.lines()
                .find_map(|line| {
                    line.split_once("WINDOW_NATIVE_SYSTEM_ENV=")
                        .map(|(_, value)| value)
                })
                .expect("actual child must report the selectors"),
        )
        .unwrap();
        for (key, value) in windows_native_system_environment().unwrap() {
            assert_eq!(values[key], value.to_str().unwrap());
        }
    }

    #[test]
    #[ignore = "required pinned Pi SDK native acceptance gate"]
    fn windows_selected_pi_sdk_capabilities_load_the_actual_package() {
        let selected = |name| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.is_file())
                .expect("the required pinned native Pi and Node entries must exist")
        };
        let cli = selected("HIROUTE_WORKER_PI_BINARY");
        let cli = if cli
            .extension()
            .is_some_and(|value| value.as_encoded_bytes().eq_ignore_ascii_case(b"cmd"))
        {
            windows_npm_entry(&cli).unwrap()
        } else {
            cli
        };
        let node = selected("HIROUTE_WORKER_NODE");
        use super::super::pi_runtime::{PiSdkCapability, check_pi_sdk_capability};
        for (scope, capability) in [
            ("worker", PiSdkCapability::Worker),
            ("continue", PiSdkCapability::Continue),
            ("collaboration", PiSdkCapability::Collaboration),
            ("models", PiSdkCapability::Models),
        ] {
            eprintln!("native acceptance Pi SDK scope={scope}");
            check_pi_sdk_capability(&cli, &node, capability).unwrap_or_else(|failure| {
                panic!("the selected actual SDK must satisfy scope={scope}: {failure:?}")
            });
        }
    }

    #[test]
    #[ignore = "required pinned Node native acceptance gate"]
    fn windows_selected_node_version_uses_the_private_capture_and_held_job() {
        let node = std::env::var_os("HIROUTE_WORKER_NODE")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_file())
            .expect("HIROUTE_WORKER_NODE must select the pinned native Node executable");
        super::super::pi_runtime::validate_pi_node(&node)
            .expect("the real selected Node version must pass the production private Job probe");
    }

    #[test]
    fn windows_npm_shims_bind_global_and_local_declared_bins_without_evaluation() {
        let root = super::super::probe_files::temporary("hiroute-npm-entry-").unwrap();
        for local in [false, true] {
            let prefix = root.path().join(if local { "local" } else { "global" });
            let modules = prefix.join("node_modules");
            let package = modules.join("@deepseek-ai/dsh");
            let bin_dir = if local {
                modules.join(".bin")
            } else {
                prefix.clone()
            };
            std::fs::create_dir_all(package.join("lib")).unwrap();
            std::fs::create_dir_all(&bin_dir).unwrap();
            let shim = bin_dir.join("DSH.cmd");
            // If any shim text is evaluated the test changes an unrelated file.
            let marker = root.path().join("shim-was-evaluated");
            std::fs::write(&shim, format!("@echo dangerous > \"{}\"", marker.display())).unwrap();
            let entry = package.join("lib/bin.js");
            std::fs::write(&entry, "throw new Error('must not execute discovery');").unwrap();
            let manifest = package.join("package.json");
            std::fs::write(
                &manifest,
                br#"{"name":"@deepseek-ai/dsh","bin":{"dsh":"lib/bin.js"}}"#,
            )
            .unwrap();
            assert_eq!(
                windows_npm_entry(&shim).unwrap(),
                std::fs::canonicalize(&entry).unwrap()
            );
            assert!(!marker.exists());
            std::fs::write(
                &manifest,
                br#"{"name":"@deepseek-ai/dsh","bin":{"dsh":"../../../../shim-was-evaluated"}}"#,
            )
            .unwrap();
            std::fs::write(&marker, "outside the selected package").unwrap();
            assert!(windows_npm_entry(&shim).is_err());
            std::fs::remove_file(marker).unwrap();
            std::fs::write(
                &manifest,
                br#"{"name":"another-producer","bin":"lib/bin.js"}"#,
            )
            .unwrap();
            assert!(windows_npm_entry(&shim).is_err());
        }
    }

    #[test]
    fn windows_npm_inspection_runs_before_manifest_read_and_entry_canonicalization() {
        let root = super::super::probe_files::temporary("hiroute-npm-inspection-").unwrap();
        let package = root.path().join("node_modules/@deepseek-ai/dsh");
        std::fs::create_dir_all(package.join("lib")).unwrap();
        let shim = root.path().join("dsh.cmd");
        let manifest = package.join("package.json");
        let entry = package.join("lib/bin.js");
        std::fs::write(&shim, "do not evaluate this shim").unwrap();
        std::fs::write(&manifest, b"malformed JSON must not be read").unwrap();
        let mut inspected = Vec::new();
        let rejected = windows_npm_entry_with_inspection(&shim, |path| {
            inspected.push(path.to_owned());
            Err(io::ErrorKind::PermissionDenied.into())
        })
        .unwrap_err();
        assert_eq!(rejected.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(inspected, [manifest.clone()]);

        std::fs::write(
            &manifest,
            br#"{"name":"@deepseek-ai/dsh","bin":{"dsh":"./lib/bin.js"}}"#,
        )
        .unwrap();
        inspected.clear();
        let rejected = windows_npm_entry_with_inspection(&shim, |path| {
            inspected.push(path.to_owned());
            if inspected.len() == 2 {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(rejected.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(inspected, [manifest, package.join("./lib/bin.js")]);
        assert!(
            !entry.exists(),
            "inspection must run before resolving a missing entry"
        );
        std::fs::write(&entry, b"never execute").unwrap();
        let inspected_entry = windows_npm_entry_with_inspection(&shim, |_| Ok(())).unwrap();
        assert_eq!(inspected_entry, windows_npm_entry(&shim).unwrap());
        assert_eq!(inspected_entry, std::fs::canonicalize(entry).unwrap());
    }
}
