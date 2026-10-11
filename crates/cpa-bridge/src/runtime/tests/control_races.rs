//! Status reads use real loopback responses while lifecycle ownership stays private.
use super::request_io::{SourceKind, active_fixture};
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

fn accept_status(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_millis(600);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                assert!(peer.ip().is_loopback());
                break stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "managed inspection never entered the control request"
                );
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("controlled loopback accept failed: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        head.push(byte[0]);
        assert!(head.len() < 16 * 1024);
    }
    assert!(head.starts_with(b"GET /v8/management/credentials?name=credential.json HTTP/1.1\r\n"));
    stream
}

fn reply_status(mut stream: TcpStream, auth_required: bool) -> std::io::Result<()> {
    let body = serde_json::to_vec(&json!({"files": [{
        "name": "credential.json", "provider": "codex", "source": "file", "runtime_only": false,
        "account_type": "oauth", "status": if auth_required { "error" } else { "active" },
        "disabled": false, "unavailable": auth_required,
        "status_message": if auth_required { "unauthorized" } else { "" }
    }]}))
    .unwrap();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Cpa-Version: {}\r\nConnection: close\r\n\r\n",
        body.len(),
        VERSION
    )?;
    stream.write_all(&body)
}

fn begin_inspection(
    runtime: &Arc<ManagedCpaRuntime>,
) -> thread::JoinHandle<Result<crate::BorrowedSubscriptionEvidence, CpaLifecycleError>> {
    let runtime = Arc::clone(runtime);
    thread::spawn(move || runtime.inspect_subscription())
}

fn listener(runtime: &ManagedCpaRuntime) -> TcpListener {
    let address = runtime.observation.lock().live.as_ref().unwrap().address;
    let listener = TcpListener::bind(address).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

#[test]
fn old_auth_required_response_reuses_newer_success_for_the_same_login_and_epoch() {
    let fixture = active_fixture(SourceKind::ManagedCodex);
    let listener = listener(&fixture.runtime);
    let epochs = fixture.runtime.epochs.current();
    let old = begin_inspection(&fixture.runtime);
    let old_response = accept_status(&listener);
    let new = begin_inspection(&fixture.runtime);
    let new_response = accept_status(&listener);
    reply_status(new_response, false).unwrap();
    let new_proof = new.join().unwrap().unwrap();
    reply_status(old_response, true).unwrap();
    let old_proof = old
        .join()
        .unwrap()
        .expect("an obsolete status failure escaped to the maintenance caller");
    assert_eq!(old_proof.account_ref(), new_proof.account_ref());
    assert_eq!(old_proof.evidence_digest(), new_proof.evidence_digest());
    assert!(
        !fixture
            .runtime
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    assert_eq!(fixture.runtime.epochs.current(), epochs);
    fixture
        .capability
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn older_success_cannot_clear_a_new_auth_required_result() {
    let fixture = active_fixture(SourceKind::ManagedCodex);
    let listener = listener(&fixture.runtime);
    let old = begin_inspection(&fixture.runtime);
    let old_response = accept_status(&listener);
    let new = begin_inspection(&fixture.runtime);
    let new_response = accept_status(&listener);
    reply_status(new_response, true).unwrap();
    assert!(matches!(
        new.join().unwrap(),
        Err(CpaLifecycleError::ManagedOAuthAuthenticationRequired)
    ));
    let epochs = fixture.runtime.epochs.current();
    if let Err(error) = reply_status(old_response, false) {
        // A newer rejection revokes the old HTTP scope immediately. Its caller
        // must fail even if the obsolete response has already lost its socket.
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
        ));
    }
    assert!(old.join().unwrap().is_err());
    assert!(
        fixture
            .runtime
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    assert_eq!(fixture.runtime.epochs.current(), epochs);
    assert_eq!(
        fixture
            .capability
            .apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn newer_success_for_another_login_cannot_mask_the_old_logins_auth_required_result() {
    let fixture = active_fixture(SourceKind::ManagedCodex);
    let listener = listener(&fixture.runtime);
    let old = begin_inspection(&fixture.runtime);
    let old_response = accept_status(&listener);
    private_atomic_write(&fixture.source_path, &serde_json::to_vec(&json!({
        "type": "codex", "access_token": "another-fixture-access", "refresh_token": "fixture-managed-refresh",
        "account_id": "another-managed-account"
    })).unwrap()).unwrap();
    let new = begin_inspection(&fixture.runtime);
    let new_response = accept_status(&listener);
    reply_status(new_response, false).unwrap();
    assert!(new.join().unwrap().is_ok());
    reply_status(old_response, true).unwrap();
    assert!(matches!(
        old.join().unwrap(),
        Err(CpaLifecycleError::ManagedOAuthAuthenticationRequired)
    ));
    assert!(
        fixture
            .runtime
            .admission
            .state
            .lock()
            .subscription_execution_suspended
    );
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn an_expired_ready_snapshot_is_unhealthy_while_the_lifecycle_owner_is_busy() {
    let fixture = active_fixture(SourceKind::NativeCodex);
    assert!(matches!(
        fixture.runtime.health().unwrap(),
        CpaHealth::Ready { .. }
    ));
    fixture.runtime.observation.lock().health_sample =
        Some(Instant::now() - Duration::from_secs(6));
    let owner = fixture.runtime.inner.lock();
    let runtime = Arc::clone(&fixture.runtime);
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        let _ = send.send(runtime.health());
    });
    let health = receive
        .recv_timeout(Duration::from_millis(600))
        .expect("health waited for the lifecycle owner")
        .unwrap();
    assert!(
        matches!(health, CpaHealth::Unhealthy { .. }),
        "an expired observation remained ready"
    );
    drop(owner);
    reader.join().unwrap();
    fixture.runtime.shutdown().unwrap();
}

#[test]
fn disabled_live_batches_finish_without_io_or_waiting_for_the_lifecycle_owner() {
    for kind in [
        SourceKind::NativeClaude,
        SourceKind::NativeCodex,
        SourceKind::ManagedCodex,
    ] {
        let fixture = active_fixture(kind);
        let old_batch = fixture.runtime.begin_routing_batch().unwrap();
        let credential_ref = old_batch.sources()[0].credential_ref.clone();
        let (model, protocol) = match kind {
            SourceKind::NativeClaude => ("claude-sonnet-5", UpstreamProtocol::Messages),
            SourceKind::NativeCodex | SourceKind::ManagedCodex => {
                ("gpt-5.5", UpstreamProtocol::Responses)
            }
        };
        old_batch
            .prepare_target(ExactCpaAttemptRequest {
                credential_ref: &credential_ref,
                upstream_model_id: model,
                protocol,
            })
            .unwrap();
        fixture
            .runtime
            .apply_account_management(
                fixture.account_subject(),
                2,
                CpaSourceManagementState::Disabled,
            )
            .unwrap();
        assert_eq!(
            fixture
                .capability
                .apply_authorization(&mut http::HeaderMap::new()),
            Err(CpaAttemptError::RevokedCredential)
        );
        let calls = (
            fixture.profile_calls(),
            fixture.control_calls(),
            fixture.spawn_count(),
        );
        let source_before = std::fs::read(&fixture.source_path).unwrap();
        let runtime = Arc::clone(&fixture.runtime);
        let outcome = thread::scope(|scope| {
            let owner = fixture.runtime.inner.lock();
            let (send, receive) = mpsc::channel();
            let reader = scope.spawn(move || {
                let old_accepted = old_batch.finish().unwrap_or(false);
                let disabled = runtime.begin_routing_batch().unwrap();
                let empty = disabled.sources().is_empty();
                let denied = matches!(
                    disabled.prepare_target(ExactCpaAttemptRequest {
                        credential_ref: &credential_ref,
                        upstream_model_id: model,
                        protocol,
                    }),
                    Err(CpaAttemptError::RevokedCredential)
                );
                let finished = disabled.finish().unwrap();
                let _ = send.send((old_accepted, empty, denied, finished));
            });
            let outcome = receive.recv_timeout(Duration::from_millis(600));
            // Release ownership before any failure or scoped-thread join, so a
            // regression reports a bounded failure without hanging the suite.
            drop(owner);
            reader.join().unwrap();
            outcome.expect("disabled batch waited for the lifecycle owner")
        });
        assert_eq!(outcome, (false, true, true, true));
        assert_eq!(
            (
                fixture.profile_calls(),
                fixture.control_calls(),
                fixture.spawn_count(),
            ),
            calls,
            "disabled publication performed profile, control, or process I/O"
        );
        assert_eq!(std::fs::read(&fixture.source_path).unwrap(), source_before);
        fixture.runtime.shutdown().unwrap();
    }
}

#[test]
fn the_same_runtime_restarts_after_shutdown_and_readmits_all_three_source_modes() {
    for kind in [
        SourceKind::NativeClaude,
        SourceKind::NativeCodex,
        SourceKind::ManagedCodex,
    ] {
        let fixture = active_fixture(kind);
        let source_before = std::fs::read(&fixture.source_path).unwrap();
        let epochs = fixture.runtime.epochs.current();
        assert_eq!(fixture.spawn_count(), 1);
        fixture.runtime.shutdown().unwrap();
        assert!(matches!(
            fixture.runtime.health(),
            Ok(CpaHealth::Stopped { .. })
        ));
        assert_eq!(
            fixture
                .capability
                .apply_authorization(&mut http::HeaderMap::new()),
            Err(CpaAttemptError::RevokedCredential)
        );

        // Keep the same object, source and saved revision. Process startup alone
        // never replaces the saved source's fresh account/catalog validation.
        fixture.runtime.start().unwrap();
        assert_eq!(fixture.spawn_count(), 2);
        fixture
            .runtime
            .apply_account_management(
                fixture.account_subject(),
                1,
                CpaSourceManagementState::Enabled,
            )
            .unwrap();
        fixture
            .runtime
            .ensure_saved_runtime_ready(fixture.account_subject(), 1)
            .unwrap();
        let (connector, endpoint, model, protocol) = match kind {
            SourceKind::NativeClaude => (
                "connector.cpa.claude",
                "endpoint.cpa.claude",
                "claude-sonnet-5",
                UpstreamProtocol::Messages,
            ),
            SourceKind::NativeCodex | SourceKind::ManagedCodex => (
                "connector.cpa.codex",
                "endpoint.cpa.codex",
                "gpt-5.5",
                UpstreamProtocol::Responses,
            ),
        };
        let source = fixture
            .runtime
            .materialize_account(connector, endpoint)
            .unwrap();
        assert_eq!(source.account_subject, fixture.account_subject());
        let target = prepare_target(
            &fixture.runtime,
            ExactCpaAttemptRequest {
                credential_ref: &source.credential_ref,
                upstream_model_id: model,
                protocol,
            },
        )
        .unwrap();
        let current = fixture
            .runtime
            .lease_downstream_capability(ExactCpaCredentialRequest {
                credential_id: target.credential_ref().credential_id(),
                connector_id: target.connector_id(),
                upstream_model_id: model,
                protocol,
                address: target.address(),
                request_path: target.request_path(),
                native_transport_model: target.native_transport_model(),
                runtime_epoch: target.runtime_epoch(),
                target_epoch: target.target_epoch(),
                excluded_key_ids: &[],
            })
            .unwrap()
            .unwrap();
        current
            .apply_authorization(&mut http::HeaderMap::new())
            .unwrap();
        assert_ne!(fixture.runtime.epochs.current(), epochs);
        assert_eq!(fixture.spawn_count(), 2);
        assert_eq!(
            fixture
                .capability
                .apply_authorization(&mut http::HeaderMap::new()),
            Err(CpaAttemptError::RevokedCredential)
        );
        assert_eq!(std::fs::read(&fixture.source_path).unwrap(), source_before);
        fixture.runtime.shutdown().unwrap();
    }
}
