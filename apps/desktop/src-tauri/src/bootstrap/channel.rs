use super::*;
use hiroute_diagnostics::{correlation::CorrelationDomain, event::*};

pub(super) const REGISTRATION_BUSY: &str = "TRUSTED_AUTHORITY_BUSY";

#[derive(Debug, Eq, PartialEq)]
enum AckOutcome {
    Registered,
    Busy,
}

/// One fully written request and its exact reply. Partial reply bytes survive a
/// timeout; malformed replies and incomplete request writes are not recoverable.
pub(super) struct PendingAck {
    id: String,
    bytes: Vec<u8>,
    allow_busy: bool,
}

impl PendingAck {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            bytes: Vec::new(),
            allow_busy: false,
        }
    }

    fn receive(&mut self, reader: &mut impl Read, timeout: Duration) -> Result<AckOutcome, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                return Err("PROTECTED_CHANNEL_TIMEOUT".into());
            }
            let mut byte = [0];
            match reader.read(&mut byte) {
                Ok(0) => return Err("PROTECTED_CHANNEL_CLOSED".into()),
                Ok(_) if byte[0] == b'\n' => {
                    let ack: Ack = serde_json::from_slice(&self.bytes)
                        .map_err(|_| "REGISTRATION_ACK_INVALID")?;
                    if ack.schema != "hiroute.protected-apply-ack/v2"
                        || ack.registration_id != self.id
                    {
                        return Err("REGISTRATION_ACK_MISMATCH".into());
                    }
                    return if ack.registered {
                        Ok(AckOutcome::Registered)
                    } else if self.allow_busy {
                        Ok(AckOutcome::Busy)
                    } else {
                        Err("REGISTRATION_ACK_MISMATCH".into())
                    };
                }
                Ok(_) => {
                    if self.bytes.len() >= 1024 {
                        return Err("PROTECTED_CHANNEL_OVERSIZED".into());
                    }
                    self.bytes.push(byte[0]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return Err("PROTECTED_CHANNEL_UNAVAILABLE".into()),
            }
        }
    }
}

impl OwnedChild {
    pub(super) fn release_input(&mut self, candidate_ref: &str) -> Result<(), String> {
        let id = crate::random_id()?;
        #[derive(serde::Serialize)]
        struct Release<'a> {
            schema: &'static str,
            registration_id: &'a str,
            candidate_ref: &'a str,
        }
        let mut bytes = Zeroizing::new(
            serde_json::to_vec(&Release {
                schema: "hiroute.protected-input/v1",
                registration_id: &id,
                candidate_ref,
            })
            .map_err(|_| "PROTECTED_INPUT_INVALID")?,
        );
        bytes.push(b'\n');
        let result = self.exchange_ack(
            &bytes,
            &id,
            hiroute_diagnostics::event::ProtectedExchangeKind::InputRelease,
        );
        if result.is_ok() {
            self.manual_input_candidates.remove(candidate_ref);
        } else {
            self.authority_healthy = false;
        }
        result
    }

    fn finish_pending_ack(&mut self) -> Result<AckOutcome, String> {
        let pending = self
            .pending_ack
            .as_mut()
            .ok_or("AUTHORITY_RECOVERY_UNAVAILABLE")?;
        let result = pending.receive(&mut self.ack, Duration::from_secs(2));
        // Only a timeout can preserve a known request/reply boundary.
        if !result
            .as_ref()
            .is_err_and(|error| error == "PROTECTED_CHANNEL_TIMEOUT")
        {
            self.pending_ack = None;
        }
        result
    }

    pub(super) fn resume_ack(&mut self) -> Result<(), String> {
        let id = self
            .pending_ack
            .as_ref()
            .ok_or("AUTHORITY_RECOVERY_UNAVAILABLE")?
            .id
            .clone();
        let started = Instant::now();
        // Either complete result restores the original frame boundary. A
        // refusal restores no grant and never replays the original operation.
        let result = self.finish_pending_ack().map(|_| ());
        emit_exchange(
            self.diagnostics.as_ref(),
            ProtectedExchangeKind::Recovery,
            ProtectedExchangePhase::AcknowledgementRead,
            &id,
            started,
            result.as_ref().err(),
            result.is_err(),
        );
        result
    }

    pub(super) fn exchange_ack(
        &mut self,
        bytes: &[u8],
        id: &str,
        kind: ProtectedExchangeKind,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut phase = ProtectedExchangePhase::RequestWrite;
        let result = (|| {
            if self.pending_ack.is_some() {
                return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
            }
            write_frame(&mut self.capability, bytes)?;
            let mut pending = PendingAck::new(id);
            pending.allow_busy = kind == ProtectedExchangeKind::Grant;
            self.pending_ack = Some(pending);
            phase = ProtectedExchangePhase::AcknowledgementRead;
            self.finish_pending_ack()
        })()
        .and_then(|outcome| match outcome {
            AckOutcome::Registered => Ok(()),
            AckOutcome::Busy => Err(REGISTRATION_BUSY.into()),
        });
        let authority_lost = result
            .as_ref()
            .is_err_and(|error| error != REGISTRATION_BUSY);
        if authority_lost {
            self.authority_healthy = false;
        }
        emit_exchange(
            self.diagnostics.as_ref(),
            kind,
            phase,
            id,
            started,
            result.as_ref().err(),
            authority_lost,
        );
        result.map_err(|error| {
            if error == REGISTRATION_BUSY {
                error
            } else {
                "TRUSTED_AUTHORITY_UNAVAILABLE".into()
            }
        })
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
        REGISTRATION_BUSY => ProtectedExchangeError::Busy,
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
    fn partial_ack_bytes_survive_timeout_and_only_the_exact_reply_resumes() {
        let (mut reader, mut writer) = pipe().unwrap();
        nonblocking(&reader).unwrap();
        let id = "a".repeat(64);
        let frame = format!(
            "{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{id}\",\"registered\":true}}\n"
        );
        writer.write_all(&frame.as_bytes()[..31]).unwrap();
        let mut pending = PendingAck::new(&id);
        assert_eq!(
            pending
                .receive(&mut reader, Duration::from_millis(20))
                .unwrap_err(),
            "PROTECTED_CHANNEL_TIMEOUT"
        );
        assert_eq!(pending.bytes.len(), 31);
        writer.write_all(&frame.as_bytes()[31..]).unwrap();
        pending
            .receive(&mut reader, Duration::from_millis(20))
            .unwrap();
    }

    #[test]
    fn resynchronization_transfers_live_child_without_writing_another_request() {
        let root = tempfile::tempdir().unwrap();
        let mut resident = super::super::tests::recovery_resident(root.path(), false);
        let (mut requests, writer) = pipe().unwrap();
        let (reader, mut replies) = pipe().unwrap();
        nonblocking(&requests).unwrap();
        nonblocking(&reader).unwrap();
        let owned = resident.owned.as_mut().unwrap();
        owned.capability = writer;
        owned.ack = reader;
        let pid = owned.child.id();
        let id = "b".repeat(64);
        assert!(
            owned
                .exchange_ack(b"one complete request\n", &id, ProtectedExchangeKind::Grant)
                .is_err()
        );
        assert_eq!(
            read_frame(&mut requests, 1024, Duration::from_millis(20), None).unwrap(),
            b"one complete request"
        );
        assert_eq!(
            resident.recover_authority().err().unwrap(),
            "AUTHORITY_RECOVERY_PENDING"
        );
        assert!(resident.owned.as_ref().unwrap().pending_ack.is_some());
        replies.write_all(format!("{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{id}\",\"registered\":true}}\n").as_bytes()).unwrap();
        let mut replacement = resident.recover_authority().unwrap();
        assert!(replacement.has_authority());
        assert!(!resident.has_authority());
        assert_eq!(replacement.owned.as_ref().unwrap().child.id(), pid);
        assert!(replacement.owned.as_ref().unwrap().shutdown.is_some());
        assert_eq!(
            read_frame(&mut requests, 1024, Duration::from_millis(20), None).unwrap_err(),
            "PROTECTED_CHANNEL_TIMEOUT"
        );
        drop(resident);
        assert!(
            replacement
                .owned
                .as_mut()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert!(acquire_host_lock(root.path()).is_err());
    }
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
            assert!(resident.owned.as_ref().unwrap().pending_ack.is_none());
            assert!(resident.recover_authority().is_err());
        });
    }
}
