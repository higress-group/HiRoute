//! Owner-verified lifecycle for the existing standalone role-all process.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use hiroute_platform::local_record::{self, Channel, Direction};
use hiroute_platform::windows::lifecycle::{MAXIMUM, Request, Response, SCHEMA, UpgradeResponse};
use hiroute_platform::windows::pipe::{Listener, Stream};

pub(super) struct LifecycleServer {
    listener: Listener,
}

impl LifecycleServer {
    pub(super) fn bind(layout: &hiroute_host_runtime::StandaloneLayout) -> Result<Self, String> {
        Self::bind_path(&layout.lifecycle_endpoint())
    }

    fn bind_path(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or("standalone lifecycle root is unavailable")?;
        hiroute_platform::windows::security::open_owner_only(parent, true)
            .map_err(|_| "standalone lifecycle root is unsafe")?;
        if std::fs::symlink_metadata(path).is_ok() {
            return Err("standalone lifecycle locator conflicts with another entry".into());
        }
        let listener = Listener::bind(path).map_err(|_| "standalone lifecycle bind failed")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "standalone lifecycle setup failed")?;
        Ok(Self { listener })
    }

    pub(super) fn poll_stop(&self, role: &hiroute_daemon::RoleAllHandle) -> Result<bool, String> {
        match self.listener.accept() {
            Ok((stream, _)) => Ok(handle_connection(stream, role).unwrap_or(false)),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(_) => Err("standalone lifecycle accept failed".into()),
        }
    }
}

fn handle_connection(
    mut stream: Stream,
    role: &hiroute_daemon::RoleAllHandle,
) -> Result<bool, String> {
    // Listener verifies the client's actual process SID before returning this stream.
    stream
        .set_deadline(Some(Instant::now() + super::IO_TIMEOUT))
        .map_err(|_| "standalone lifecycle deadline setup failed")?;
    let body = local_record::read(&mut stream, Channel::Lifecycle, Direction::Request, MAXIMUM)
        .map_err(|_| "standalone lifecycle request read failed")?;
    let request: Request =
        serde_json::from_slice(&body).map_err(|_| "standalone lifecycle request is invalid")?;
    if request.schema != SCHEMA
        || !matches!(
            request.action.as_str(),
            "status" | "stop" | "upgrade-prepare" | "upgrade-status" | "upgrade-cancel"
        )
    {
        return Err("standalone lifecycle request is invalid".into());
    }
    let stop = request.action == "stop";
    let response = if request.action.starts_with("upgrade-") {
        use hiroute_host_runtime::UpgradeAction;
        let action = match request.action.as_str() {
            "upgrade-prepare" => UpgradeAction::Prepare,
            "upgrade-status" => UpgradeAction::Status,
            _ => UpgradeAction::Cancel,
        };
        let status = role.upgrade_status(action, "windows-install".into())?;
        serde_json::to_vec(&UpgradeResponse {
            schema: SCHEMA.into(),
            action: request.action,
            pid: std::process::id(),
            accepted: true,
            paused: status.paused,
            active_calls: status.active_calls,
            active_tasks: status.active_tasks,
        })
    } else {
        serde_json::to_vec(&Response {
            schema: SCHEMA.into(),
            action: request.action,
            pid: std::process::id(),
            accepted: true,
        })
    }
    .map_err(|_| "standalone lifecycle response is invalid")?;
    local_record::write(
        &mut stream,
        Channel::Lifecycle,
        Direction::Response,
        &response,
        MAXIMUM,
    )
    .map_err(|_| "standalone lifecycle response write failed")?;
    // Closing a pipe before the client consumed the response can discard it. A lost ACK
    // does not undo an already accepted stop request; the client holds the real process.
    let _ = local_record::await_acknowledgement(&mut stream);
    Ok(stop)
}

pub(super) fn shutdown_signal() -> Result<Receiver<()>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|_| "standalone signal runtime unavailable")?;
    let (mut interrupt, mut break_signal) = {
        let _entered = runtime.enter();
        (
            tokio::signal::windows::ctrl_c()
                .map_err(|_| "standalone Ctrl-C handler unavailable")?,
            tokio::signal::windows::ctrl_break()
                .map_err(|_| "standalone Ctrl-Break handler unavailable")?,
        )
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("hiroute-standalone-signals".into())
        .spawn(move || {
            runtime.block_on(async move {
                tokio::select! {
                    _ = interrupt.recv() => {}
                    _ = break_signal.recv() => {}
                }
                let _ = sender.try_send(());
            })
        })
        .map_err(|_| "standalone signal handler unavailable")?;
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_cannot_register_or_apply_business_authority() {
        assert!(
            serde_json::from_slice::<Request>(
                br#"{"schema":"hiroute.windows-lifecycle/v1","action":"stop","secret":"x"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_slice::<Request>(
                br#"{"schema":"hiroute.windows-lifecycle/v1","action":"stop","pid":1}"#
            )
            .is_err()
        );
    }
}
