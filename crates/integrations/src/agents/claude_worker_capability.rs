//! An isolated native request must retain the host route despite conflicting settings.
use super::{MARKER, MODEL, binary_identity, http};
use crate::agents::{NativeProbeProcess, probe_files};
use hiroute_domain::delegation::{
    NativeDependencyCheckV1, NativeDependencyFailureReasonV1 as Reason, NativeDependencyFailureV1,
};
use std::{
    net::TcpListener,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub(super) fn check(binary: &Path, search_path: &str) -> Result<(), NativeDependencyFailureV1> {
    let failure = |reason| NativeDependencyFailureV1 {
        check: NativeDependencyCheckV1::ClaudeHostManagedProvider,
        reason,
    };
    let before = binary_identity(binary).map_err(|_| failure(Reason::Unavailable))?;
    let root = probe_files::temporary("hiroute-claude-provider-")
        .map_err(|_| failure(Reason::Unavailable))?;
    let root_path = std::fs::canonicalize(root.path()).map_err(|_| failure(Reason::Unavailable))?;
    let home = root_path.join("home");
    let config = home.join(".claude");
    let workspace = root_path.join("workspace");
    for path in [&home, &config, &workspace] {
        probe_files::directory(path).map_err(|_| failure(Reason::Unavailable))?;
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| failure(Reason::Unavailable))?;
    let conflicting = TcpListener::bind("127.0.0.1:0").map_err(|_| failure(Reason::Unavailable))?;
    for socket in [&listener, &conflicting] {
        socket
            .set_nonblocking(true)
            .map_err(|_| failure(Reason::Unavailable))?;
    }
    let endpoint = format!(
        "http://{}",
        listener
            .local_addr()
            .map_err(|_| failure(Reason::Unavailable))?
    );
    let other = format!(
        "http://{}",
        conflicting
            .local_addr()
            .map_err(|_| failure(Reason::Unavailable))?
    );
    let settings = root_path.join("settings.json");
    let value = serde_json::json!({"env":{
        "ANTHROPIC_BASE_URL":other,
        "ANTHROPIC_AUTH_TOKEN":"hiroute-conflicting-local-noncredential",
        "ANTHROPIC_API_KEY":"hiroute-conflicting-local-noncredential",
        "ANTHROPIC_MODEL":"hiroute-conflicting-model"
    }});
    serde_json::to_writer(
        probe_files::file(&settings).map_err(|_| failure(Reason::Unavailable))?,
        &value,
    )
    .map_err(|_| failure(Reason::Unavailable))?;
    let output = probe_files::file(&root_path.join("output.private"))
        .map_err(|_| failure(Reason::Unavailable))?;
    let observer = output
        .try_clone()
        .map_err(|_| failure(Reason::Unavailable))?;
    let mut command = Command::new(binary);
    command
        .args([
            "--bare",
            "--print",
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--settings",
        ])
        .arg(&settings)
        .args([
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--no-chrome",
            "--permission-mode",
            "dontAsk",
            "--output-format",
            "json",
            "--model",
            MODEL,
            "Reply with OK only. Do not use tools.",
        ])
        .env_clear()
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("PATH", search_path)
        .env("TMPDIR", &workspace)
        .env("TMP", &workspace)
        .env("TEMP", &workspace)
        .env("CLAUDE_CODE_TMPDIR", &workspace)
        .env("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("ANTHROPIC_BASE_URL", endpoint)
        .env("ANTHROPIC_AUTH_TOKEN", MARKER)
        .env("ANTHROPIC_MODEL", MODEL)
        .env("ANTHROPIC_CUSTOM_MODEL_OPTION", MODEL)
        .current_dir(&workspace);
    #[cfg(windows)]
    for (name, value) in crate::agents::windows_native_system_environment()
        .map_err(|_| failure(Reason::Unavailable))?
    {
        command.env(name, value);
    }
    #[cfg(unix)]
    let child = {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                output
                    .try_clone()
                    .map_err(|_| failure(Reason::Unavailable))?,
            ))
            .stderr(Stdio::from(output))
            .process_group(0)
            .spawn()
            .map_err(|_| failure(Reason::Unavailable))?
    };
    #[cfg(windows)]
    let child = crate::agents::windows_runtime::spawn(
        &command,
        false,
        hiroute_platform::windows::job::Stream::Null,
        hiroute_platform::windows::job::Stream::File(&output),
        hiroute_platform::windows::job::Stream::File(&output),
    )
    .map_err(|_| failure(Reason::Unavailable))?;
    let mut child = NativeProbeProcess::new(child);
    let started = Instant::now();
    let result = (|| {
        let mut observed = false;
        let mut requests = 0;
        loop {
            if conflicting.accept().is_ok() {
                return Err(Reason::Unsupported);
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    requests += 1;
                    if requests > 4 {
                        return Err(Reason::InvalidOutput);
                    }
                    observed |= http::serve(&mut stream, MARKER, MODEL, None)
                        .map_err(|_| Reason::Unsupported)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(Reason::Unavailable),
            }
            if let Some(success) = child.observe().map_err(|_| Reason::Unavailable)? {
                return if success && observed {
                    Ok(())
                } else {
                    Err(Reason::Unsupported)
                };
            }
            if started.elapsed() >= Duration::from_secs(10) {
                return Err(Reason::Timeout);
            }
            if observer.metadata().map_err(|_| Reason::Unavailable)?.len() > 128 * 1024 {
                return Err(Reason::InvalidOutput);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if child.stop().is_err() {
        let _ = root.keep();
        return Err(failure(Reason::CleanupFailed));
    }
    result.map_err(failure)?;
    if binary_identity(binary).map_err(|_| failure(Reason::Unavailable))? != before {
        return Err(failure(Reason::Unavailable));
    }
    Ok(())
}
