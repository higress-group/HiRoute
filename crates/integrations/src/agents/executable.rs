//! Read-only, bounded probes of explicitly selected or PATH-resolved installations.
//! No configuration helper or caller-supplied arguments are executed. Raw output stays private.
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

const OUTPUT_LIMIT: usize = 64 * 1024;

/// Probe the selected native engine for subscription model discovery, not admission.
/// Reuses the bounded `--version` probe; never guesses a release or reads model caches.
pub fn codex_subscription_client_version(path: &Path) -> Option<String> {
    match executable_probe(path) {
        ExecutableProbe::Installed(observation) if !observation.version.is_empty() => {
            // Codex models-manager sends only MAJOR.MINOR.PATCH, including alpha builds.
            observation
                .version
                .split(['-', '+'])
                .next()
                .map(str::to_owned)
        }
        _ => None,
    }
}

pub(super) struct ExecutableObservationV1 {
    pub version: String,
    pub canonical_path: String,
}

pub(super) enum ExecutableProbe {
    Installed(ExecutableObservationV1),
    NotFound,
    Unknown(ProbeFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProbeFailure {
    NotExecutable,
    Unavailable,
    LaunchFailed(Option<i32>),
    #[cfg(unix)]
    PipeSetupFailed(i32),
    #[cfg(unix)]
    PipeReadFailed(Option<i32>),
    #[cfg(unix)]
    WaitFailed(Option<i32>),
    CleanupFailed,
    Failed,
    TimedOut,
    OutputLimit,
}

pub(super) fn executable_probe(path: &Path) -> ExecutableProbe {
    probe(path, Duration::from_secs(2))
}

fn probe(path: &Path, timeout: Duration) -> ExecutableProbe {
    let canonical = match resolve(path) {
        Ok(Some(path)) => path,
        Ok(None) => return ExecutableProbe::NotFound,
        Err(reason) => return ExecutableProbe::Unknown(reason),
    };
    let Some(canonical_path) = canonical.to_str().map(str::to_owned) else {
        return ExecutableProbe::Unknown(ProbeFailure::Unavailable);
    };
    let version = match bounded_version(&canonical, timeout) {
        Ok(bytes) => parse_version(&bytes).unwrap_or_default(),
        Err(ProbeFailure::LaunchFailed(code)) => {
            return ExecutableProbe::Unknown(ProbeFailure::LaunchFailed(code));
        }
        Err(ProbeFailure::NotExecutable) => {
            return ExecutableProbe::Unknown(ProbeFailure::NotExecutable);
        }
        // The process started: version diagnostics cannot establish capability or deny admission.
        Err(_) => String::new(),
    };
    ExecutableProbe::Installed(ExecutableObservationV1 {
        version,
        canonical_path,
    })
}

pub(super) fn resolve(path: &Path) -> Result<Option<PathBuf>, ProbeFailure> {
    #[cfg(windows)]
    if path.is_absolute() || path.components().count() > 1 {
        return admit(selected_candidates(path));
    }
    #[cfg(windows)]
    return resolve_in(&search_directories(), &[path]);

    #[cfg(not(windows))]
    {
        let candidates = if path.is_absolute() || path.components().count() > 1 {
            vec![path.to_owned()]
        } else {
            let mut directories = std::env::var_os("PATH")
                .map(|search| std::env::split_paths(&search).collect::<Vec<_>>())
                .unwrap_or_default();
            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
            {
                directories.push(PathBuf::from(home).join(".local/bin"));
            }
            directories.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);
            #[cfg(target_os = "macos")]
            directories.push(PathBuf::from("/opt/homebrew/bin"));
            directories
                .into_iter()
                .map(|dir| dir.join(path))
                .collect::<Vec<_>>()
        };
        for candidate in candidates {
            match std::fs::symlink_metadata(&candidate) {
                Ok(_) => {
                    let canonical =
                        std::fs::canonicalize(&candidate).map_err(|_| ProbeFailure::Unavailable)?;
                    check_runnable(&canonical)?;
                    return Ok(Some(canonical));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ProbeFailure::Unavailable),
            }
        }
        Ok(None)
    }
}

/// Admit the first candidate that is an ordinary file on a native drive.
///
/// A candidate that simply is not there continues the walk, and so does one whose
/// directory is not a directory: that is a fact about one PATH entry, not a veto on
/// the rest of the search. A candidate that exists but cannot be opened (an alias
/// leaf, a denied read) is remembered, so an installation that stays unobservable
/// still reports an unavailable probe instead of falsely claiming it is absent.
#[cfg(windows)]
fn admit(candidates: impl IntoIterator<Item = PathBuf>) -> Result<Option<PathBuf>, ProbeFailure> {
    let mut first_failure = None;
    for candidate in candidates {
        match super::managed_launch::windows_executable_path(&candidate) {
            Ok(canonical) => return Ok(Some(canonical)),
            Err(error) if is_absent(&error) => continue,
            Err(_) => {
                first_failure.get_or_insert(ProbeFailure::Unavailable);
            }
        }
    }
    match first_failure {
        Some(failure) => Err(failure),
        None => Ok(None),
    }
}

/// Resolve the first name that is present, one search entry at a time.
///
/// Callers that probe several spellings of the same installation (a selected entry plus a
/// native alias) pass them here instead of walking the entries themselves: the search order
/// stays entry-major, so PATH position outranks a spelling, while each entry is still resolved
/// as a directory before any candidate is opened.
#[cfg(windows)]
pub(super) fn resolve_in(
    directories: &[PathBuf],
    names: &[&Path],
) -> Result<Option<PathBuf>, ProbeFailure> {
    admit(
        directories
            .iter()
            .filter_map(|directory| search_directory(directory))
            .flat_map(|directory| {
                names
                    .iter()
                    .flat_map(move |name| directory_candidates(&directory, name))
            }),
    )
}

/// Windows resolves an extensionless selection through the native shims before the
/// bare spelling, which is what a caller-supplied or saved absolute path means here.
#[cfg(windows)]
fn selected_candidates(path: &Path) -> Vec<PathBuf> {
    if path.extension().is_none() {
        return vec![
            path.with_extension("exe"),
            path.with_extension("cmd"),
            path.to_owned(),
        ];
    }
    vec![path.to_owned()]
}

#[cfg(windows)]
fn directory_candidates(directory: &Path, name: &Path) -> Vec<PathBuf> {
    let candidate = directory.join(name);
    if name.extension().is_none() {
        return vec![
            candidate.with_extension("exe"),
            candidate.with_extension("cmd"),
            candidate,
        ];
    }
    vec![candidate]
}

/// Every directory the bare name may live in, as spelled by the environment.
/// Windows PATHs routinely carry junctions and symbolic links (the Oracle
/// `javapath` entries, nvm's `nodejs` link) as well as stale or non-directory
/// entries, so each one is resolved separately below.
///
/// The Unix-only system directories stay out of this list: upstream gates them with
/// `#[cfg(unix)]` because `PathBuf::from("/usr/bin")` on Windows resolves to
/// `C:\usr\bin`, which a Git or Cygwin install can create and turn into stray candidates.
#[cfg(windows)]
fn search_directories() -> Vec<PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|search| std::env::split_paths(&search).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        directories.push(PathBuf::from(home).join(".local/bin"));
    }
    directories
}

/// One search entry as the directory it actually names. An entry that is absent,
/// is a file rather than a directory, or cannot be read contributes no candidate
/// instead of ending the search: an ancestor's spelling is a fact about one entry,
/// not an admission decision. Only the leaf candidate has to be reparse-free, and
/// the admitted path is the native spelling of the directory reached, so a caller
/// can never be redirected by a retargeted ancestor between these two steps.
#[cfg(windows)]
fn search_directory(directory: &Path) -> Option<PathBuf> {
    match std::fs::canonicalize(directory) {
        Ok(canonical) if canonical.is_dir() => Some(canonical),
        Ok(_) => None,
        Err(_) => None,
    }
}

/// Kinds that only prove this candidate is not the installation: the file is not
/// there, or the search entry that was supposed to contain it is not a directory.
/// Everything else (a denied read, an alias leaf, an identity that does not round
/// trip) stays a failure, so an unobservable installation is reported as unavailable
/// instead of being claimed absent.
#[cfg(windows)]
fn is_absent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

#[cfg(unix)]
fn check_runnable(path: &Path) -> Result<(), ProbeFailure> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).map_err(|_| ProbeFailure::Unavailable)?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(ProbeFailure::NotExecutable);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn check_runnable(path: &Path) -> Result<(), ProbeFailure> {
    let metadata = std::fs::metadata(path).map_err(|_| ProbeFailure::Unavailable)?;
    if !metadata.is_file() {
        return Err(ProbeFailure::NotExecutable);
    }
    Ok(())
}

#[cfg(unix)]
fn bounded_version(path: &Path, timeout: Duration) -> Result<Vec<u8>, ProbeFailure> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn nonblocking(fd: impl AsFd) -> Result<(), ProbeFailure> {
        let flags = fcntl(&fd, FcntlArg::F_GETFL)
            .map_err(|error| ProbeFailure::PipeSetupFailed(error as i32))?;
        fcntl(
            &fd,
            FcntlArg::F_SETFL(OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|error| ProbeFailure::PipeSetupFailed(error as i32))?;
        Ok(())
    }
    fn drain(
        reader: &mut impl Read,
        target: &mut Vec<u8>,
        total: &mut usize,
    ) -> Result<(), ProbeFailure> {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    *total += count;
                    if *total > OUTPUT_LIMIT {
                        return Err(ProbeFailure::OutputLimit);
                    }
                    target.extend_from_slice(&chunk[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ProbeFailure::PipeReadFailed(error.raw_os_error())),
            }
        }
    }
    // Removing HOME alone lets native startup fall back to the OS account home.
    // Even --version can create aliases/caches, so give every probe disposable roots.
    let probe_home = tempfile::Builder::new()
        .prefix("hiroute-version-")
        .tempdir()
        .map_err(|_| ProbeFailure::Unavailable)?;
    let home = std::fs::canonicalize(probe_home.path()).map_err(|_| ProbeFailure::Unavailable)?;
    let codex_home = home.join(".codex");
    let claude_home = home.join(".claude");
    let config_home = home.join(".config");
    let cache_home = home.join(".cache");
    let data_home = home.join(".local/share");
    for directory in [
        &codex_home,
        &claude_home,
        &config_home,
        &cache_home,
        &data_home,
    ] {
        std::fs::create_dir_all(directory).map_err(|_| ProbeFailure::Unavailable)?;
    }
    let mut command = Command::new(path);
    command
        .arg("--version")
        .env_clear()
        .env("HOME", &home)
        .env("CODEX_HOME", &codex_home)
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CACHE_HOME", &cache_home)
        .env("XDG_DATA_HOME", &data_home)
        .current_dir(&home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(search) = std::env::var_os("PATH") {
        command.env("PATH", search);
    }
    let started = Instant::now();
    // ETXTBSY means exec did not start a child program. A concurrent installer or a fork
    // briefly retaining a CLOEXEC writer can cause this. Retry only this read-only probe,
    // inside both its original deadline and a small launch budget; never retry an Agent task.
    let launch_budget = timeout.min(Duration::from_millis(100));
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error)
                if error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32)
                    && started.elapsed() < launch_budget =>
            {
                std::thread::sleep(Duration::from_millis(5));
                check_runnable(path)?;
            }
            Err(error) => return Err(ProbeFailure::LaunchFailed(error.raw_os_error())),
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut child = super::NativeProbeProcess::new(child);
    let result = (|| {
        let mut stdout = stdout.ok_or(ProbeFailure::Unavailable)?;
        let mut stderr = stderr.ok_or(ProbeFailure::Unavailable)?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let (mut out, mut err, mut total) = (Vec::new(), Vec::new(), 0);
        loop {
            drain(&mut stdout, &mut out, &mut total)?;
            drain(&mut stderr, &mut err, &mut total)?;
            if let Some(status) = child
                .observe()
                .map_err(|error| ProbeFailure::WaitFailed(error.raw_os_error()))?
            {
                drain(&mut stdout, &mut out, &mut total)?;
                drain(&mut stderr, &mut err, &mut total)?;
                return if status {
                    Ok(if out.is_empty() { err } else { out })
                } else {
                    Err(ProbeFailure::Failed)
                };
            }
            if started.elapsed() >= timeout {
                return Err(ProbeFailure::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    // WNOWAIT reserves the leader until every mutating group signal is finished.
    // Never signal a numeric PID/PGID after reap or discard unproven live material.
    if child.stop().is_err() {
        let _ = probe_home.keep();
        return Err(ProbeFailure::CleanupFailed);
    }
    result
}

#[cfg(windows)]
fn bounded_version(path: &Path, timeout: Duration) -> Result<Vec<u8>, ProbeFailure> {
    let home =
        super::probe_files::temporary("hiroute-version-").map_err(|_| ProbeFailure::Unavailable)?;
    let home_path = std::fs::canonicalize(home.path()).map_err(|_| ProbeFailure::Unavailable)?;
    let mut command = std::process::Command::new(path);
    command
        .arg("--version")
        .env_clear()
        .env("HOME", &home_path)
        .env("USERPROFILE", &home_path)
        .current_dir(&home_path);
    for (key, relative) in [
        ("CODEX_HOME", ".codex"),
        ("CLAUDE_CONFIG_DIR", ".claude"),
        ("QODER_CONFIG_DIR", ".qoder"),
        ("DSH_HOME", ".dsh"),
        ("PI_CODING_AGENT_DIR", ".pi/agent"),
    ] {
        std::fs::create_dir_all(home_path.join(relative)).map_err(|_| ProbeFailure::Unavailable)?;
        command.env(key, home_path.join(relative));
    }
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let result = super::windows_runtime::bounded(&command, false, timeout, OUTPUT_LIMIT);
    if result
        .as_ref()
        .is_err_and(super::windows_runtime::cleanup_unknown)
    {
        let _ = home.keep();
    }
    let bytes = result.map_err(|error| {
        if let Some(code) = super::windows_runtime::launch_failure_code(&error) {
            ProbeFailure::LaunchFailed(code)
        } else if error.kind() == std::io::ErrorKind::TimedOut {
            ProbeFailure::TimedOut
        } else {
            ProbeFailure::Unavailable
        }
    })?;
    Ok(bytes.to_vec())
}

#[cfg(not(any(unix, windows)))]
fn bounded_version(_path: &Path, _timeout: Duration) -> Result<Vec<u8>, ProbeFailure> {
    Err(ProbeFailure::Unavailable)
}

fn parse_version(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()?
        .split_whitespace()
        .find(|word| {
            if word.len() > 64
                || !word
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b))
            {
                return false;
            }
            let numeric = word.split(['-', '+']).next().unwrap_or("");
            let pieces = numeric.split('.').collect::<Vec<_>>();
            pieces.len() == 3
                && pieces
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use hiroute_domain::{
        AgentClaudePresetValuesV2, CanonicalDigest, ManagedClaudeLaunchDescriptorV2,
        ManagedLaunchProfileV1,
    };

    /// Junctions are the ordinary spelling of a Windows PATH entry (the Oracle
    /// `javapath` mount point, nvm's `nodejs` link), and real PATHs also carry stale
    /// entries and files. None of them may end the search for a name that is present
    /// further along; only the leaf candidate is an admission decision.
    #[test]
    fn windows_path_search_traverses_junction_and_junk_entries() {
        let fixture = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let nvm = root.join("nvm v24");
        let javapath = root.join("java javapath");
        for directory in [&nvm, &javapath] {
            std::fs::create_dir(directory).unwrap();
        }
        hiroute_platform_test_support::junction::create(&root.join("nodejs"), &nvm).unwrap();
        hiroute_platform_test_support::junction::create(&root.join("javapath"), &javapath).unwrap();
        let agent = nvm.join("agent.cmd");
        std::fs::write(&agent, b"@echo off\r\n").unwrap();
        // A file in the search path names no directory, so it contributes nothing.
        let file_entry = root.join("powershell.exe");
        std::fs::write(&file_entry, b"not a directory").unwrap();
        // Each non-directory entry is a fact about itself, not a veto on the search.
        for entry in [
            root.join("missing"),
            file_entry.clone(),
            root.join("javapath"),
        ] {
            assert_eq!(resolve_in(&[entry], &[Path::new("agent")]), Ok(None));
        }

        let resolved = resolve_in(
            &[
                root.join("missing"),
                file_entry,
                root.join("javapath"),
                root.join("nodejs"),
            ],
            &[Path::new("agent")],
        )
        .unwrap()
        .expect("installation behind a junction entry must be found");
        assert_eq!(
            resolved,
            super::super::managed_launch::windows_executable_path(&agent).unwrap()
        );
        assert!(!resolved.to_str().unwrap().starts_with(r"\\?\"));
        assert_eq!(
            resolved.file_name().unwrap(),
            std::ffi::OsStr::new("agent.cmd")
        );
    }

    /// A caller that probes several spellings of one installation must not inherit an
    /// entry's spelling as a verdict either: a junction entry that holds neither name is
    /// simply an entry with nothing in it, and the alias further along still gets found.
    #[test]
    fn windows_alias_search_traverses_junction_entries() {
        let fixture = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let nvm = root.join("nvm v24");
        let javapath = root.join("java javapath");
        for directory in [&nvm, &javapath] {
            std::fs::create_dir(directory).unwrap();
        }
        hiroute_platform_test_support::junction::create(&root.join("nodejs"), &nvm).unwrap();
        hiroute_platform_test_support::junction::create(&root.join("javapath"), &javapath).unwrap();
        let alias = nvm.join("agent-alias.cmd");
        std::fs::write(&alias, b"@echo off\r\n").unwrap();

        // Nothing under either spelling in the junction entry: honest absence, not a veto.
        assert_eq!(
            resolve_in(
                &[root.join("javapath"), root.join("missing")],
                &[Path::new("agent"), Path::new("agent-alias")],
            ),
            Ok(None)
        );

        let resolved = resolve_in(
            &[
                root.join("missing"),
                root.join("javapath"),
                root.join("nodejs"),
            ],
            &[Path::new("agent"), Path::new("agent-alias")],
        )
        .unwrap()
        .expect("an alias behind a junction entry must still be found");
        assert_eq!(
            resolved,
            super::super::managed_launch::windows_executable_path(&alias).unwrap()
        );
    }

    /// Traversing an ancestor is not the same as accepting an alias for the leaf: an
    /// installation that stays unobservable reports an unavailable probe instead of
    /// being admitted or falsely claimed absent.
    #[test]
    fn windows_path_search_still_rejects_a_reparse_leaf() {
        let fixture = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        let payload = root.join("payload");
        std::fs::create_dir(&payload).unwrap();
        hiroute_platform_test_support::junction::create(&root.join("agent.cmd"), &payload).unwrap();

        assert_eq!(
            resolve_in(&[root], &[Path::new("agent")]),
            Err(ProbeFailure::Unavailable)
        );
    }

    #[test]
    fn windows_executable_observation_prepares_managed_launch_with_native_drive_identity() {
        let current = std::env::current_exe().unwrap();
        let trusted = std::fs::canonicalize(&current).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("native 路径 %! '&.exe");
        // This is a copy of the actual native test executable, not script bytes
        // named .exe. It is borrowed executable input, not a private auth file.
        std::fs::copy(&current, &selected).unwrap();
        let projected = resolve(&selected).unwrap().unwrap();
        assert!(!projected.to_str().unwrap().starts_with(r"\\?\"));
        assert_eq!(
            std::fs::canonicalize(&projected).unwrap(),
            std::fs::canonicalize(&selected).unwrap()
        );
        let native = hiroute_platform::windows::security::open_identity(&selected, false).unwrap();
        let projected_file =
            hiroute_platform::windows::security::open_identity(&projected, false).unwrap();
        assert_eq!(
            hiroute_platform::windows::security::file_identity(&native).unwrap(),
            hiroute_platform::windows::security::file_identity(&projected_file).unwrap()
        );
        let ExecutableProbe::Installed(observation) = probe(&selected, Duration::from_secs(2))
        else {
            panic!("actual native executable observation unavailable");
        };
        assert_eq!(observation.canonical_path, projected.to_str().unwrap());
        let preflight = super::super::ManagedClaudeLaunchPreflightV1::from_observations(
            observation.canonical_path,
            &ManagedLaunchProfileV1::claude_code(),
            &[],
        );
        let helper = super::super::managed_launch::windows_executable_path(&current).unwrap();
        let descriptor = ManagedClaudeLaunchDescriptorV2::trusted(
            "agent-connection/windows-native-path",
            "claude-messages-v1",
            &preflight.executable,
            CanonicalDigest::of_bytes(b"native-path-snapshot"),
            1,
            CanonicalDigest::of_bytes(b"native-path-publication"),
            "http://127.0.0.1:5837",
            AgentClaudePresetValuesV2 {
                opus: None,
                sonnet: None,
                haiku: None,
            },
            helper.to_str().unwrap(),
        )
        .unwrap();
        let mut prepared = super::super::ManagedClaudeProcessV1::prepare(
            &descriptor,
            &serde_json::json!({"env":{"UNRELATED":"keep"}}),
            &[],
            trusted.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(prepared.command_mut().get_program(), projected.as_os_str());
        drop(prepared);
        drop(projected_file);
        drop(native);
    }
}
