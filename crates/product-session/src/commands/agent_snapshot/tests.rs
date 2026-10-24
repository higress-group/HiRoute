//! Real Client Core transport plus a controlled slow-read barrier, no time-based
//! success assertion. The old locked path is a negative control for the same test.
use super::*;
use hiroute_client_core::{Client, LocalEndpoint};
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::{Mutex, Notify},
};
use zeroize::Zeroizing;

struct Authority(Arc<AtomicBool>);
impl HostAuthority for Authority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn native_connection(&self) -> Option<crate::NativeConnectionStatus> {
        Some(if self.0.load(Ordering::SeqCst) {
            crate::NativeConnectionStatus::RecoveryRequired
        } else {
            crate::NativeConnectionStatus::Ready
        })
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        if operation != "ListSessionsV2" {
            return Err("UNEXPECTED_MUTATION".into());
        }
        Ok(Zeroizing::new("test-observation-grant".into()))
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    None,
    OldLock,
    Replace,
    Epoch,
    Revision,
    Publication,
    Recovery,
    Authority,
    Cancel,
}

#[tokio::test]
async fn slow_agent_scan_does_not_block_observation_or_cursor_updates() {
    exercise(Change::None).await;
}
#[tokio::test]
async fn old_locked_agent_scan_is_a_negative_control_for_observation() {
    exercise(Change::OldLock).await;
}
#[tokio::test]
async fn replaced_session_or_epoch_cannot_publish_old_agent_facts() {
    exercise(Change::Replace).await;
    exercise(Change::Epoch).await;
}
#[tokio::test]
async fn same_session_configuration_recovery_and_authority_changes_reject_old_agent_facts() {
    for change in [
        Change::Revision,
        Change::Publication,
        Change::Recovery,
        Change::Authority,
    ] {
        exercise(change).await;
    }
}

#[tokio::test]
async fn cancelled_agent_scan_releases_its_read_without_mutating_session() {
    exercise(Change::Cancel).await;
}

async fn exercise(change: Change) {
    let root = tempfile::Builder::new()
        .prefix("hragents-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let runtime = root.path().join("run");
    std::fs::create_dir_all(runtime.join("hiroute")).unwrap();
    std::fs::set_permissions(
        runtime.join("hiroute"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let endpoint = LocalEndpoint::for_child(&runtime, std::process::id());
    let listener = UnixListener::bind(endpoint.path()).unwrap();
    std::fs::set_permissions(endpoint.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let changed = Arc::new(AtomicBool::new(false));
    let lost_authority = Arc::new(AtomicBool::new(false));
    let sessions = Arc::new(Mutex::new(Some(Session::new(
        Client::new("agent-concurrency-test", endpoint),
        Arc::new(Authority(lost_authority.clone())),
    ))));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (started_server, release_server, changed_server) =
        (started.clone(), release.clone(), changed.clone());
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (started, release, changed) = (
                started_server.clone(),
                release_server.clone(),
                changed_server.clone(),
            );
            tokio::spawn(async move {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
                stream
                    .get_mut()
                    .write_all(&serde_json::to_vec(&negotiate_hello(&hello).unwrap()).unwrap())
                    .await
                    .unwrap();
                stream.get_mut().write_all(b"\n").await.unwrap();
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                let request: LocalControlWireRequestV2 = serde_json::from_str(&line).unwrap();
                if request.operation_id == "ListAgents" {
                    started.notify_one();
                    release.notified().await;
                }
                let altered = changed.load(Ordering::SeqCst);
                let data = match request.operation_id.as_str() {
                    "GetClientServiceStatus" => {
                        json!({"schema":"hiroute.client-service-status/v1","daemon_role":"resident","recovery_ready":!(altered && change==Change::Recovery),"mutation_available":true,"gateway":"ready","active_publication": if altered && change==Change::Publication {json!({"revision":2,"digest":CanonicalDigest::of_bytes(b"new-publication")})} else {Value::Null},"revisions":{"target":if altered && change==Change::Revision {2} else {1},"dependencies":{}}})
                    }
                    "ListAgents" | "ScanAgents" => json!({"agents":[]}),
                    "ListAgentPlanCatalog" => {
                        json!({"schema":"hiroute.agent-plan-catalog/v2","plans":[],"drafts":[],"next_cursor":null})
                    }
                    "ListSessions" => {
                        assert!(request.protected_grant.is_some());
                        json!({"sessions":[],"next_cursor":null})
                    }
                    other => panic!("unexpected read: {other}"),
                };
                let response = json!({"schema_version":LOCAL_CONTROL_SCHEMA_V2,"request_id":request.request_id,"status":"succeeded","data":data,"operation":null,"warnings":[],"next_actions":[],"error":null});
                let _ = stream
                    .get_mut()
                    .write_all(&serde_json::to_vec(&response).unwrap())
                    .await;
                let _ = stream.get_mut().write_all(b"\n").await;
            });
        }
    });
    let error = OnceLock::new();
    let epoch = AtomicU64::new(1);
    let diagnostics = hiroute_diagnostics::context::DiagnosticHandle::noop();
    let state = CommandState(&sessions, &error, &epoch, &diagnostics);
    let broker = crate::web_confirmation::ConfirmationBroker::default();
    let context = ClientContext {
        id: "agent-test",
        confirmations: &broker,
        active: &|| true,
        notify: None,
    };
    let mut snapshot = Box::pin(async {
        if change == Change::OldLock {
            sessions
                .lock()
                .await
                .as_mut()
                .unwrap()
                .agent_snapshot()
                .await
        } else {
            agent_snapshot(&context, &state, None).await
        }
    });
    if change == Change::Cancel {
        tokio::select! {
            _ = started.notified() => {}
            _ = &mut snapshot => panic!("scan did not wait on its barrier"),
        }
        drop(snapshot);
        let read = super::super::observation_read(
            &context,
            &state,
            ObservationReadRequestV2::new(ObservationReadIntentV2::Status(
                ObservationStatusQueryV2 {},
            )),
        );
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), read).await;
        release.notify_one();
        assert!(outcome.unwrap().unwrap()["sessions"].is_array());
        server.abort();
        return;
    }
    let concurrent = async {
        started.notified().await;
        let read = super::super::observation_read(
            &context,
            &state,
            ObservationReadRequestV2::new(ObservationReadIntentV2::Status(
                ObservationStatusQueryV2 {},
            )),
        );
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), read).await;
        if change == Change::OldLock {
            release.notify_one();
            assert!(outcome.is_err(), "negative control did not block");
            return;
        }
        if outcome.is_err() {
            release.notify_one();
            panic!("slow Agent snapshot blocked observation");
        }
        assert!(outcome.unwrap().unwrap()["sessions"].is_array());
        {
            let mut guard = sessions.lock().await;
            let session = guard.as_mut().unwrap();
            // Observation cursors are unrelated to Agent configuration.
            session.stop_observing();
            session.resume_observing();
            match change {
                Change::Replace => {
                    *guard = Some(Session::new(
                        session.client.clone(),
                        Arc::new(Authority(lost_authority.clone())),
                    ));
                }
                Change::Epoch => {
                    epoch.fetch_add(1, Ordering::SeqCst);
                }
                Change::Authority => {
                    lost_authority.store(true, Ordering::SeqCst);
                }
                _ => {
                    changed.store(true, Ordering::SeqCst);
                }
            }
        }
        release.notify_one();
    };
    let (result, _) = tokio::join!(snapshot, concurrent);
    if matches!(change, Change::None | Change::OldLock) {
        assert!(result.is_ok());
    } else {
        assert!(result.is_err(), "old Agent facts escaped after {change:?}");
    }
    server.abort();
}
