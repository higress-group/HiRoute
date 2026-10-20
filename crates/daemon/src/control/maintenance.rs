//! Optional independent launcher maintenance. It cannot register business authority.
use super::*;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::*};

pub(super) struct Channel {
    requests: ProtectedFrameReader,
    replies: std::fs::File,
}

pub(super) fn open(arguments: &Arguments) -> Result<Option<Channel>, StartupError> {
    let (input, output) = match (arguments.maintenance_fd, arguments.maintenance_ack_fd) {
        (None, None) => return Ok(None),
        (Some(input), Some(output)) => (input, output),
        _ => return Err("maintenance requires two distinct inherited descriptors".into()),
    };
    if input == output
        || [
            arguments.shutdown_fd,
            arguments.capability_fd,
            arguments.capability_ack_fd,
        ]
        .into_iter()
        .flatten()
        .any(|fd| fd == input || fd == output)
    {
        return Err("maintenance descriptors must be distinct from all other channels".into());
    }
    Ok(Some(Channel {
        requests: ProtectedFrameReader::new(protected_inherited_reader(input)?)?,
        replies: protected_inherited_writer(output)?,
    }))
}

fn upgrade_only(
    frame: ProtectedChannelFrame,
) -> Result<hiroute_host_runtime::LauncherUpgradeRequest, String> {
    match frame {
        ProtectedChannelFrame::Upgrade(request) => Ok(request),
        _ => Err("maintenance cannot register capabilities or protected inputs".into()),
    }
}

pub(super) fn poll(
    channel: &mut Option<Channel>,
    role: &hiroute_daemon::RoleAllHandle,
    diagnostics: &DiagnosticRuntime,
) {
    let Some(active) = channel.as_mut() else {
        return;
    };
    let result = (|| {
        for frame in active.requests.read_available()? {
            let request = upgrade_only(frame)?;
            let id = request.registration_id.clone();
            let started = std::time::Instant::now();
            let status = role.upgrade_status(request.action, request.registration_id)?;
            let result = write_status(&mut active.replies, &status);
            diagnostics.emit(DiagnosticEvent::ProtectedExchange(ProtectedExchange {
                kind: ProtectedExchangeKind::Recovery,
                phase: ProtectedExchangePhase::AcknowledgementWrite,
                registration_token: diagnostics
                    .handle()
                    .token(CorrelationDomain::ProtectedRegistration, &id),
                elapsed_ms: started.elapsed().as_millis() as u64,
                error: result
                    .as_ref()
                    .err()
                    .map(|_| ProtectedExchangeError::Unavailable),
                authority_lost: result.is_err(),
            }));
            result?;
        }
        Ok::<_, String>(())
    })();
    if result.is_err() {
        // Keep actual model/task work alive. This maintenance channel is never reused.
        *channel = None;
    }
}

pub(super) fn write_status(
    writer: &mut std::fs::File,
    status: &hiroute_host_runtime::LauncherUpgradeStatus,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(status).map_err(|_| "maintenance response invalid")?;
    bytes.push(b'\n');
    let mut bytes = bytes.as_slice();
    let deadline = std::time::Instant::now() + PROTECTED_FRAME_DEADLINE;
    while !bytes.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err("maintenance acknowledgement timed out".into());
        }
        match writer.write(bytes) {
            Ok(0) => return Err("maintenance channel closed".into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("maintenance channel unavailable".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_maintenance_cannot_register_input_or_apply_authority() {
        let request = serde_json::json!({"schema":"hiroute.protected-input/v1", "registration_id":"a".repeat(64), "candidate_ref":"candidate/manual", "candidate_revision":1, "secret":"synthetic"});
        assert!(
            upgrade_only(parse_capability_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
                .is_err()
        );
        let request = serde_json::json!({"schema":"hiroute.launcher-upgrade/v1", "registration_id":"b".repeat(64), "action":"prepare"});
        assert!(
            upgrade_only(parse_capability_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
                .is_ok()
        );
    }
}
