//! Production launcher registration and ACK diagnostics share an irreversible correlation.
use super::*;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::*};

pub(super) fn process(
    role: &hiroute_daemon::RoleAllHandle,
    frame: ProtectedChannelFrame,
    writer: &mut std::fs::File,
    diagnostics: &DiagnosticRuntime,
) -> Result<(), String> {
    let id = frame.registration_id();
    let kind = match &frame {
        ProtectedChannelFrame::Upgrade(_) => ProtectedExchangeKind::Upgrade,
        ProtectedChannelFrame::Apply { .. } => ProtectedExchangeKind::Grant,
        ProtectedChannelFrame::InputRegister { .. } => ProtectedExchangeKind::InputRegister,
        ProtectedChannelFrame::InputRelease { .. } => ProtectedExchangeKind::InputRelease,
    };
    let started = std::time::Instant::now();
    let mut phase = ProtectedExchangePhase::Register;
    let mut registration_busy = false;
    let result = (|| {
        match frame {
            ProtectedChannelFrame::Upgrade(request) => {
                let status = role.upgrade_status(request.action, request.registration_id)?;
                phase = ProtectedExchangePhase::AcknowledgementWrite;
                return write_status(writer, &status);
            }
            ProtectedChannelFrame::Apply { registration, .. } => {
                if role.try_register_apply_capability(registration)?
                    == hiroute_local_storage::ApplyCapabilityRegistrationOutcome::Busy
                {
                    // No writer was acquired and no grant was accepted. Finish
                    // the exact reply so the host can retain synchronization.
                    registration_busy = true;
                    phase = ProtectedExchangePhase::AcknowledgementWrite;
                    return write_ack_result(writer, &id, false);
                }
            }
            ProtectedChannelFrame::InputRegister {
                candidate, secret, ..
            } => role.register_manual_protected_input(candidate, secret)?,
            ProtectedChannelFrame::InputRelease { candidate_ref, .. } => {
                role.release_manual_protected_input(&candidate_ref)?
            }
        }
        phase = ProtectedExchangePhase::AcknowledgementWrite;
        write_ack(writer, &id)
    })();
    diagnostics.emit(DiagnosticEvent::ProtectedExchange(ProtectedExchange {
        kind,
        phase,
        registration_token: diagnostics
            .handle()
            .token(CorrelationDomain::ProtectedRegistration, &id),
        elapsed_ms: started.elapsed().as_millis() as u64,
        error: result
            .as_ref()
            .err()
            .map(|_| ProtectedExchangeError::Unavailable)
            .or(registration_busy.then_some(ProtectedExchangeError::Busy)),
        authority_lost: false,
    }));
    result
}

pub(super) fn write_status(
    writer: &mut std::fs::File,
    status: &hiroute_host_runtime::LauncherUpgradeStatus,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(status).map_err(|_| "upgrade response invalid")?;
    bytes.push(b'\n');
    let mut bytes = bytes.as_slice();
    let deadline = std::time::Instant::now() + PROTECTED_FRAME_DEADLINE;
    while !bytes.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err("upgrade acknowledgement timed out".into());
        }
        match writer.write(bytes) {
            Ok(0) => return Err("upgrade channel closed".into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("upgrade channel unavailable".into()),
        }
    }
    Ok(())
}
