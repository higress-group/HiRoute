use super::ShellState;
use crate::ConsoleOrigin;
use hiroute_application_api::{ConsoleLaunchRequestV1, ConsoleLaunchV1, SystemStatusV1};
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_host_runtime::{
    StandaloneLayout, read_standalone_install_record, standalone_cli_binary,
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn cli() -> Result<std::path::PathBuf, &'static str> {
    let layout = StandaloneLayout::from_environment().map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    let marker = read_standalone_install_record(&layout.marker_path)
        .map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    if marker.target != "x86_64-pc-windows-msvc"
        || !marker
            .install_root
            .starts_with(layout.home.join(".local/lib/hiroute"))
    {
        return Err("INSTALLATION_UNAVAILABLE");
    }
    let path = standalone_cli_binary(&marker.install_root);
    hiroute_platform::windows::security::open_identity(&path, false)
        .map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    Ok(path)
}
pub(super) fn execute_reply(
    cli: &std::path::Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<(bool, Value), &'static str> {
    use std::os::windows::process::CommandExt;
    let mut child = Command::new(cli)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "SERVICE_COMMAND_FAILED")?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .ok_or("SERVICE_COMMAND_FAILED")?
            .write_all(bytes)
            .map_err(|_| "SERVICE_COMMAND_FAILED")?;
    }
    let output = child.stdout.take().ok_or("SERVICE_COMMAND_FAILED")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        output
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|_| "SERVICE_COMMAND_FAILED")? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("SERVICE_COMMAND_TIMED_OUT");
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "SERVICE_COMMAND_FAILED")?
        .map_err(|_| "SERVICE_COMMAND_FAILED")?;
    if bytes.len() > 16 * 1024 {
        return Err("SERVICE_RESPONSE_INVALID");
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "SERVICE_RESPONSE_INVALID")?;
    Ok((status.success(), value))
}
pub(super) fn execute(
    cli: &std::path::Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Value, &'static str> {
    let (success, value) = execute_reply(cli, args, stdin, timeout)?;
    if !success || value.get("status").is_some_and(|s| s != "succeeded") {
        return Err("SERVICE_COMMAND_FAILED");
    }
    Ok(value.get("data").cloned().unwrap_or(value))
}
pub(super) fn manager(args: &[&str]) -> Result<Value, &'static str> {
    execute(&cli()?, args, None, Duration::from_secs(45))
}
fn client() -> Result<Client, &'static str> {
    let layout = StandaloneLayout::from_environment().map_err(|_| "INSTALLATION_UNAVAILABLE")?;
    Ok(Client::new(
        "hiroute-shell",
        LocalEndpoint::from_runtime_root(layout.runtime_root),
    )
    .with_timeout(Duration::from_secs(2)))
}
pub(super) async fn launch() -> Result<(ConsoleOrigin, url::Url), &'static str> {
    let response = client()?
        .query::<_, ConsoleLaunchV1>(
            "CreateConsoleLaunch",
            "shell-launch",
            &ConsoleLaunchRequestV1 {},
        )
        .await
        .map_err(|_| "CONSOLE_LAUNCH_UNAVAILABLE")?;
    let value = response
        .data
        .filter(|_| response.error.is_none())
        .ok_or("CONSOLE_LAUNCH_UNAVAILABLE")?;
    ConsoleOrigin::launch(&value.url)
}
pub(super) async fn ready_launch(
    state: &ShellState,
) -> Result<(ConsoleOrigin, url::Url), &'static str> {
    let status = tauri::async_runtime::spawn_blocking(|| {
        manager(&["service", "status", "--output", "json"])
    })
    .await
    .map_err(|_| "SERVICE_STATUS_UNAVAILABLE")??;
    if status["manager_active"] != true {
        tauri::async_runtime::spawn_blocking(|| manager(&["service", "start", "--output", "json"]))
            .await
            .map_err(|_| "SERVICE_START_FAILED")??;
    }
    state.phase("waiting", None);
    wait_ready().await?;
    launch().await
}
pub(super) async fn wait_ready() -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if is_ready().await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("SERVICE_NOT_READY");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
pub(super) async fn is_ready() -> bool {
    let Ok(client) = client() else {
        return false;
    };
    client
        .query::<_, SystemStatusV1>("GetSystemStatus", "shell-ready", &serde_json::json!({}))
        .await
        .is_ok_and(|response| {
            response.error.is_none()
                && response
                    .data
                    .is_some_and(|s| s.gateway == "ready" && s.daemon == "role_all")
        })
}
pub(super) fn available_blocking() -> bool {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .is_ok_and(|runtime| runtime.block_on(is_ready()))
}
