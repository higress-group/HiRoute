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
    Mutation,
    Authority,
    Cancel,
    FinalEpoch,
    Hidden,
    ClientRevoked,
    WrongTrialSchema,
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

#[tokio::test]
async fn epoch_expiry_during_final_service_read_rejects_the_snapshot() {
    exercise(Change::FinalEpoch).await;
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ReadKind {
    Agents,
    SelectedAgent,
    TrialChoices,
    TrialModel,
    TrialSetup,
    HomeSummary,
    Associations,
    Catalog,
    Observation,
}

#[tokio::test]
async fn hidden_window_reads_succeed_but_revocation_during_transport_is_rejected() {
    for kind in [
        ReadKind::Agents,
        ReadKind::SelectedAgent,
        ReadKind::Catalog,
        ReadKind::Observation,
        ReadKind::TrialChoices,
        ReadKind::TrialModel,
        ReadKind::TrialSetup,
        ReadKind::HomeSummary,
        ReadKind::Associations,
    ] {
        exercise_read(Change::Hidden, kind).await;
        exercise_read(Change::ClientRevoked, kind).await;
    }
}

#[tokio::test]
async fn narrow_trial_reads_keep_all_current_read_fences_without_unrelated_agent_status() {
    for kind in [
        ReadKind::TrialChoices,
        ReadKind::TrialModel,
        ReadKind::TrialSetup,
        ReadKind::SelectedAgent,
    ] {
        for change in [
            Change::None,
            Change::Replace,
            Change::Epoch,
            Change::Revision,
            Change::Publication,
            Change::Recovery,
            Change::Mutation,
            Change::Authority,
            Change::Cancel,
            Change::FinalEpoch,
        ] {
            exercise_read(change, kind).await;
        }
    }
}

#[tokio::test]
async fn trial_snapshot_schema_is_checked_at_the_command_boundary() {
    exercise_read(Change::None, ReadKind::TrialSetup).await;
    exercise_read(Change::WrongTrialSchema, ReadKind::TrialSetup).await;
    exercise_read(Change::Epoch, ReadKind::TrialSetup).await;
    exercise_read(Change::Authority, ReadKind::TrialSetup).await;
}

#[tokio::test]
async fn home_and_routing_reads_release_session_and_reject_stale_results() {
    for kind in [ReadKind::HomeSummary, ReadKind::Associations] {
        for change in [
            Change::None,
            Change::Replace,
            Change::Epoch,
            Change::Revision,
            Change::Publication,
            Change::Recovery,
            Change::Authority,
            Change::Cancel,
            Change::FinalEpoch,
        ] {
            exercise_read(change, kind).await;
        }
    }
}

#[tokio::test]
async fn slow_catalog_and_body_reads_release_session_and_reject_stale_publication() {
    for kind in [ReadKind::Catalog, ReadKind::Observation] {
        for change in [
            Change::None,
            Change::Replace,
            Change::Epoch,
            Change::Revision,
            Change::Publication,
            Change::Recovery,
            Change::Authority,
            Change::Cancel,
            Change::FinalEpoch,
        ] {
            exercise_read(change, kind).await;
        }
    }
}

async fn exercise(change: Change) {
    exercise_read(change, ReadKind::Agents).await;
}
async fn exercise_read(change: Change, read_kind: ReadKind) {
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
    let final_status_started = Arc::new(Notify::new());
    let final_status_release = Arc::new(Notify::new());
    let (final_started_server, final_release_server) =
        (final_status_started.clone(), final_status_release.clone());
    let status_reads = Arc::new(AtomicU64::new(0));
    let blocked_reads = Arc::new(AtomicU64::new(0));
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
            let (final_started, final_release, status_reads) = (
                final_started_server.clone(),
                final_release_server.clone(),
                status_reads.clone(),
            );
            let blocked_reads = blocked_reads.clone();
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
                let blocked = match read_kind {
                    ReadKind::Agents
                    | ReadKind::TrialChoices
                    | ReadKind::HomeSummary
                    | ReadKind::Associations => "ListAgents",
                    ReadKind::SelectedAgent => "ScanAgents",
                    ReadKind::TrialModel => "GetAgentConnectionStatus",
                    ReadKind::TrialSetup => "GetTrialSetupSnapshotV1",
                    ReadKind::Catalog => "ListAgentPlanCatalog",
                    ReadKind::Observation => "ListSessions",
                };
                if request.operation_id == blocked
                    && blocked_reads.fetch_add(1, Ordering::SeqCst) == 0
                {
                    started.notify_one();
                    release.notified().await;
                }
                if request.operation_id == "GetClientServiceStatus"
                    && change == Change::FinalEpoch
                    && status_reads.fetch_add(1, Ordering::SeqCst) == 1
                {
                    final_started.notify_one();
                    final_release.notified().await;
                }
                let altered = changed.load(Ordering::SeqCst);
                let data = match request.operation_id.as_str() {
                    "GetClientServiceStatus" => {
                        json!({"schema":"hiroute.client-service-status/v1","daemon_role":"resident","recovery_ready":!(altered && change==Change::Recovery),"mutation_available":!(altered && change==Change::Mutation),"gateway":"ready","active_publication": if altered && change==Change::Publication {json!({"revision":2,"digest":CanonicalDigest::of_bytes(b"new-publication")})} else {Value::Null},"revisions":{"target":if altered && change==Change::Revision {2} else {1},"dependencies":{}}})
                    }
                    "ListAgents" | "ScanAgents" => {
                        if read_kind == ReadKind::SelectedAgent {
                            assert_eq!(
                                request.operation_id, "ScanAgents",
                                "selected reads cannot discover all Agents"
                            );
                            assert_eq!(request.payload["context_id"], "agent-context/dsh");
                            assert!(request.payload.get("native_target").is_none());
                        }
                        let agents = if matches!(
                            read_kind,
                            ReadKind::TrialChoices
                                | ReadKind::SelectedAgent
                                | ReadKind::HomeSummary
                                | ReadKind::Associations
                        ) {
                            ["claude", "codex", "dsh", "pi", "qoder", "opencode"].into_iter()
                                .filter(|name| read_kind != ReadKind::SelectedAgent || *name == "dsh")
                                .map(|name|
                                json!({"agent_id":format!("agent_{name}_default"), "context_id":format!("agent-context/{name}"),"version":"test","configuration_state":"detected"})
                            ).collect::<Vec<_>>()
                        } else {
                            vec![]
                        };
                        json!({"agents":agents})
                    }
                    "GetAgentConnectionStatus" => {
                        if read_kind == ReadKind::Associations {
                            assert_eq!(request.payload["projection"], "model");
                            assert!(request.payload.get("include_skills").is_none());
                            let context = request.payload["context_id"].as_str().unwrap();
                            json!({"schema":"hiroute.agent-model-settings-status/v2", "context_id":context, "state":"not_configured", "operation_id":null, "operation_state":null, "restore_point_ref":null, "applied_revision":null, "surface_results":[], "model_verified":false})
                        } else {
                            let selected = read_kind == ReadKind::SelectedAgent;
                            assert!(selected || read_kind == ReadKind::TrialModel);
                            let context = if selected {
                                "agent-context/dsh"
                            } else {
                                "agent-context/selected"
                            };
                            assert_eq!(
                                request.payload["context_id"], context,
                                "unrelated Agent status must not be read"
                            );
                            if selected {
                                assert_eq!(request.payload["include_skills"], true);
                                assert_ne!(request.payload["projection"], "model");
                            } else {
                                assert_eq!(request.payload["projection"], "model");
                                assert!(request.payload.get("include_skills").is_none());
                            }
                            json!({"schema":"hiroute.agent-model-settings-status/v2", "context_id":context, "state":"not_configured", "operation_id":null, "operation_state":null, "restore_point_ref":null, "applied_revision":null, "surface_results":[], "model_verified":false})
                        }
                    }
                    "GetAgentSettingsSummary" => {
                        assert_eq!(read_kind, ReadKind::HomeSummary);
                        let facet = json!({"state":"absent","coverage":"complete","action":null,"native_checked":false});
                        let entries = request.payload["context_ids"].as_array().unwrap().iter().map(|context| json!({
                            "context_id":context,"model":facet,"collaboration":facet,"management_skill":facet
                        })).collect::<Vec<_>>();
                        json!({"entries":entries})
                    }
                    "GetTrialSetupSnapshotV1" => {
                        assert_eq!(read_kind, ReadKind::TrialSetup);
                        assert_eq!(request.payload, json!({}));
                        assert!(request.protected_grant.is_none());
                        let new_revision = altered && change == Change::Revision;
                        json!({"schema":if change == Change::WrongTrialSchema {"hiroute.trial-setup-snapshot/unsupported"} else {"hiroute.trial-setup-snapshot/v1"},
                        "options":{"authoring":null,"claude_capabilities":null,"context_window":null,"suggested_alias":null,"candidates":[{
                            "binding_id":"binding/same", "model_configuration_id":"fixture-model", "display_name":"fixture model",
                            "reasoning":{"kind":"fixed","profile":if new_revision {"new"} else {"old"}},
                            "billing_class":"paid","routable":true,"ingress_protocols":["responses"],"native_ingress_protocols":["responses"]
                        }],"free_suggestions":null,"ratings":null,"codex_capabilities":null,"revisions":{"target":if new_revision {2} else {1},"dependencies":{}}},
                        "management":{"revisions":{"target":if new_revision {2} else {1},"dependencies":{}},"retained_api_connections":[],"sources":[{
                            "source_id":"source/same", "revision":if new_revision {2} else {1},
                            "display_name":if new_revision {"changed source"} else {"original source"},
                            "provenance":"user_configured","access_kind":"api","state":"ready","actions":[],
                            "models":[{"model_ref":"model/same", "binding_id":"binding/same", "revision":if new_revision {2} else {1},
                                "upstream_model_id":"fixture-model", "display_name":"fixture model", "membership":"user_declared",
                                "catalog_admitted":false, "availability":if new_revision {"disabled"} else {"available"}}]
                        }]}})
                    }
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
        active: &|| change != Change::ClientRevoked || !changed.load(Ordering::SeqCst),
        visible: &|| change != Change::Hidden,
        notify: None,
    };
    let mut snapshot = Box::pin(async {
        if change == Change::OldLock {
            return sessions
                .lock()
                .await
                .as_mut()
                .unwrap()
                .agent_snapshot()
                .await
                .map(|_| ());
        }
        match read_kind {
            ReadKind::Agents => agent_snapshot(&context, &state, None, None)
                .await
                .map(|_| ()),
            ReadKind::SelectedAgent => {
                agent_snapshot(&context, &state, None, Some("agent-context/dsh".into()))
                    .await
                    .map(|snapshot| {
                        assert_eq!(snapshot.agents.len(), 1);
                        assert_eq!(
                            snapshot.agents[0].context_id.as_deref(),
                            Some("agent-context/dsh")
                        );
                        assert!(snapshot.agents[0].settings.is_some());
                        assert!(snapshot.agents[0].status_error.is_none());
                    })
            }
            ReadKind::TrialChoices => super::super::trial_agent_choices(&context, &state)
                .await
                .map(|snapshot| {
                    assert_eq!(snapshot.choices.len(), 6);
                    let value = serde_json::to_value(snapshot).unwrap();
                    assert!(
                        value["choices"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|choice| choice.get("settings").is_none())
                    );
                }),
            ReadKind::TrialModel => {
                super::super::trial_agent_model(&context, &state, "agent-context/selected".into())
                    .await
                    .map(|_| ())
            }
            ReadKind::TrialSetup => super::super::trial_setup_snapshot(&context, &state)
                .await
                .map(|snapshot| {
                    if change != Change::WrongTrialSchema {
                        assert_eq!(snapshot.schema, TRIAL_SETUP_SNAPSHOT_SCHEMA_V1);
                    }
                    let source = &snapshot.management.sources[0];
                    assert_eq!(source.display_name, "original source");
                    assert_eq!(source.models[0].binding_id, "binding/same");
                    assert_eq!(source.models[0].availability, ComputeModelAvailabilityV1::Available);
                    assert_eq!(snapshot.options.candidates[0].binding_id, "binding/same");
                    assert!(matches!(snapshot.options.candidates[0].reasoning,
                        hiroute_domain::NativeReasoningCapabilityV1::Fixed { ref profile } if profile == "old"));
                }),
            ReadKind::HomeSummary => super::super::agent_home_summary(&context, &state)
                .await
                .map(|snapshot| {
                    assert_eq!(snapshot.agents.len(), 6);
                    let value = serde_json::to_value(snapshot).unwrap();
                    assert!(
                        value["agents"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|item| item.get("settings").is_none())
                    );
                }),
            ReadKind::Associations => super::super::agent_model_associations(&context, &state)
                .await
                .map(|snapshot| {
                    assert!(snapshot.complete);
                    assert_eq!(snapshot.agents.len(), 6);
                }),
            ReadKind::Catalog => super::super::desktop_snapshot(&context, &state)
                .await
                .map(|_| ()),
            ReadKind::Observation => super::super::observation_read(
                &context,
                &state,
                ObservationReadRequestV2::new(ObservationReadIntentV2::Status(
                    ObservationStatusQueryV2 {},
                )),
            )
            .await
            .map(|_| ()),
        }
    });
    if change == Change::FinalEpoch {
        let expire = async {
            started.notified().await;
            release.notify_one();
            final_status_started.notified().await;
            assert!(
                sessions.try_lock().is_err(),
                "final status must hold the Session lock"
            );
            // Window expiry advances this atomic without taking Session's lock.
            epoch.fetch_add(1, Ordering::SeqCst);
            final_status_release.notify_one();
        };
        let (result, _) = tokio::join!(snapshot, expire);
        assert!(
            matches!(result, Err(DesktopFailure::Native { ref code }) if code == "CONFIRMATION_STALE"),
            "epoch expiry during the final await must reject the snapshot"
        );
        server.abort();
        return;
    }
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
    if change == Change::WrongTrialSchema {
        assert!(
            matches!(result, Err(DesktopFailure::Native { ref code }) if code == "RESPONSE_DATA_INVALID")
        );
    } else if matches!(change, Change::None | Change::OldLock | Change::Hidden) {
        assert!(result.is_ok());
    } else {
        assert!(result.is_err(), "old Agent facts escaped after {change:?}");
    }
    server.abort();
}
