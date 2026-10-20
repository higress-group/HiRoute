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
    let result = (|| {
        match frame {
            ProtectedChannelFrame::Upgrade(request) => {
                let status = role.upgrade_status(request.action, request.registration_id)?;
                phase = ProtectedExchangePhase::AcknowledgementWrite;
                return maintenance::write_status(writer, &status);
            }
            ProtectedChannelFrame::Apply { registration, .. } => {
                role.register_apply_capability(registration)?
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
            .map(|_| ProtectedExchangeError::Unavailable),
        authority_lost: result.is_err(),
    }));
    result
}
