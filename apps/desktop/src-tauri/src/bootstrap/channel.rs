use super::*;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::*};

impl OwnedChild {
    pub(super) fn exchange_ack(
        &mut self,
        bytes: &[u8],
        id: &str,
        kind: ProtectedExchangeKind,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut phase = ProtectedExchangePhase::RequestWrite;
        let result = (|| {
            write_frame(&mut self.capability, bytes)?;
            phase = ProtectedExchangePhase::AcknowledgementRead;
            receive_ack(&mut self.ack, id, Duration::from_secs(2))
        })();
        if result.is_err() {
            self.authority_healthy = false;
        }
        emit_exchange(
            self.diagnostics.as_ref(),
            kind,
            phase,
            id,
            started,
            result.as_ref().err(),
            result.is_err(),
        );
        result.map_err(|_| "TRUSTED_AUTHORITY_UNAVAILABLE".into())
    }
}

pub(super) fn emit_exchange(
    diagnostics: Option<&DiagnosticsPort>,
    kind: ProtectedExchangeKind,
    phase: ProtectedExchangePhase,
    id: &str,
    started: Instant,
    error: Option<&String>,
    authority_lost: bool,
) {
    let Some(diagnostics) = diagnostics else {
        return;
    };
    let error = error.map(|error| match error.as_str() {
        "PROTECTED_CHANNEL_TIMEOUT" => ProtectedExchangeError::Timeout,
        "PROTECTED_CHANNEL_CLOSED" => ProtectedExchangeError::Closed,
        "PROTECTED_CHANNEL_OVERSIZED" => ProtectedExchangeError::Oversized,
        "REGISTRATION_ACK_INVALID" | "UPGRADE_ACK_INVALID" => ProtectedExchangeError::Invalid,
        "REGISTRATION_ACK_MISMATCH" => ProtectedExchangeError::Mismatch,
        _ => ProtectedExchangeError::Unavailable,
    });
    diagnostics
        .handle()
        .try_emit(DiagnosticEvent::ProtectedExchange(ProtectedExchange {
            kind,
            phase,
            registration_token: diagnostics
                .handle()
                .token(CorrelationDomain::ProtectedRegistration, id),
            elapsed_ms: started.elapsed().as_millis() as u64,
            error,
            authority_lost,
        }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_conflicting_ack_invalidates_authority_and_late_success_is_never_consumed() {
        let root = tempfile::tempdir().unwrap();
        let mut resident = super::super::tests::recovery_resident(root.path(), false);
        let (mut requests, request_writer) = pipe().unwrap();
        let (reply_reader, mut replies) = pipe().unwrap();
        nonblocking(&requests).unwrap();
        nonblocking(&reply_reader).unwrap();
        let owned = resident.owned.as_mut().unwrap();
        owned.capability = request_writer;
        owned.ack = reply_reader;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let bytes = read_frame(&mut requests, 4096, Duration::from_secs(2), None).unwrap();
                let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let conflicting = format!("{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{}\",\"registered\":true}}\n", "f".repeat(64));
                write_frame(&mut replies, conflicting.as_bytes()).unwrap();
                let late = format!("{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":{},\"registered\":true}}\n", request["registration_id"]);
                write_frame(&mut replies, late.as_bytes()).unwrap();
                assert_eq!(read_frame(&mut requests, 4096, Duration::from_millis(100), None).unwrap_err(), "PROTECTED_CHANNEL_TIMEOUT");
            });
            let revisions = RevisionSetV1 {
                target: 0,
                dependencies: Default::default(),
            };
            for _ in 0..2 {
                assert_eq!(
                    resident
                        .register_operation(
                            &CanonicalDigest::of_bytes(b"exact request"),
                            &revisions,
                            "ListSessionsV2"
                        )
                        .unwrap_err(),
                    "TRUSTED_AUTHORITY_UNAVAILABLE"
                );
            }
            assert!(!resident.has_authority());
        });
    }
}
