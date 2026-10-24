#![cfg(unix)]
//! The installed CLI retries actual reader admission failures over Local Control.
use hiroute_domain::*;
use hiroute_observation::{DigestAuthority, LocalObservationStore};
use std::{
    collections::VecDeque,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

struct TracedStore {
    store: Arc<LocalObservationStore>,
    busy: mpsc::Sender<bool>,
    pages: Option<Mutex<VecDeque<Result<RoutingTimelinePageV1, ObservationQueryError>>>>,
}

impl ObservationQueryPort for TracedStore {
    fn observed_routing_timeline(
        &self,
        r: &ObservationReaderContext,
        q: &RoutingTimelineQueryV1,
        now: i64,
    ) -> Result<RoutingTimelinePageV1, ObservationQueryError> {
        // Controlled pages exercise the CLI consumer; storage projections have their own tests.
        let result = if let Some(pages) = &self.pages {
            pages.lock().unwrap().pop_front().unwrap_or_else(|| {
                Ok(RoutingTimelinePageV1 {
                    turns: vec![],
                    watermark: "settled".into(),
                    complete: true,
                })
            })
        } else {
            ObservationQueryPort::observed_routing_timeline(&*self.store, r, q, now)
        };
        if matches!(
            &result,
            Err(ObservationQueryError::ReaderBusy | ObservationQueryError::ReadTimeout)
        ) {
            self.busy.send(true).unwrap();
        } else if result.is_ok() {
            self.busy.send(false).unwrap();
        }
        result
    }
    fn list_sessions(
        &self,
        w: &WorkspaceId,
        q: &SessionListQueryV1,
    ) -> Result<SessionListV1, ObservationQueryError> {
        ObservationQueryPort::list_sessions(&*self.store, w, q)
    }
    fn get_session(
        &self,
        w: &WorkspaceId,
        s: &SessionId,
        c: ContentMode,
    ) -> Result<SessionDetailV1, ObservationQueryError> {
        ObservationQueryPort::get_session(&*self.store, w, s, c)
    }
    fn get_receipt(
        &self,
        w: &WorkspaceId,
        r: &ReceiptId,
    ) -> Result<RoutingReceiptV1, ObservationQueryError> {
        ObservationQueryPort::get_receipt(&*self.store, w, r)
    }
    fn get_value(
        &self,
        w: &WorkspaceId,
        q: &ValueQueryV1,
    ) -> Result<ValueViewV1, ObservationQueryError> {
        ObservationQueryPort::get_value(&*self.store, w, q)
    }
    fn get_status(&self, w: &WorkspaceId) -> Result<ObservationStatusV1, ObservationQueryError> {
        ObservationQueryPort::get_status(&*self.store, w)
    }
}

struct OwnedChild(Option<Child>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn review_cli_follow_retries_real_busy_reads() {
    // Runtime startup may inspect native paths; keep this test and its CLI in
    // one isolated child HOME without changing the test runner's environment.
    if std::env::var_os("HIROUTE_TIMELINE_TEST_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "review_cli_follow_retries_real_busy_reads",
                "--nocapture",
            ])
            .env("HOME", home.path())
            .env("HIROUTE_TIMELINE_TEST_CHILD", "1")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("DSH_CONFIG_DIR")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = root.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = hiroute_daemon::control::ProductionControlRuntime::open(&state).unwrap();
    let store = Arc::new(
        LocalObservationStore::open(
            root.path().join("observation"),
            DigestAuthority::new([27; 32]),
        )
        .unwrap(),
    );
    let (busy, reads) = mpsc::channel();
    let mut ports = runtime.application_ports();
    ports.observation = Arc::new(TracedStore {
        store: store.clone(),
        busy,
        pages: None,
    });
    let endpoint = root.path().join("runtime");
    let mut server = hiroute_daemon::control::start_control(
        hiroute_application::ApplicationService::new(ports),
        &endpoint,
    )
    .unwrap();
    let mut child = store.with_all_query_permits_for_test(|| {
        let child = OwnedChild(Some(
            Command::new(env!("CARGO_BIN_EXE_hiroute"))
                .args([
                    "sessions",
                    "timeline",
                    "missing-session",
                    "--follow",
                    "--output",
                    "json",
                ])
                .env("HIROUTE_RUNTIME_DIR", &endpoint)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        // Two real failures prove the CLI survived the first busy response and
        // retried through the server, rather than exiting or using a fake hint.
        for _ in 0..2 {
            assert!(
                reads
                    .recv_timeout(Duration::from_secs(5))
                    .expect("follow did not retry the real busy read")
            );
        }
        child
    });
    // An empty timeline is a successful page. Observe two successful reads to
    // prove the CLI accepted recovery and continued its normal polling cadence.
    for _ in 0..2 {
        assert!(
            !reads
                .recv_timeout(Duration::from_secs(5))
                .expect("follow did not recover to successful reads")
        );
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.0.as_ref().unwrap().id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "follow did not stop on Ctrl-C after reader recovery"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "an unchanged empty timeline emits no changes"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("temporarily unavailable"));
    server.shutdown();
    server.join(Duration::from_secs(3)).unwrap();
}

#[test]
fn follow_recovers_timeout_and_preserves_historical_protection_in_changed_turns() {
    if std::env::var_os("HIROUTE_PROTECTION_TEST_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "follow_recovers_timeout_and_preserves_historical_protection_in_changed_turns",
                "--nocapture",
            ])
            .env("HOME", home.path())
            .env("HIROUTE_PROTECTION_TEST_CHILD", "1")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("DSH_CONFIG_DIR")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = root.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = hiroute_daemon::control::ProductionControlRuntime::open(&state).unwrap();
    let store = Arc::new(
        LocalObservationStore::open(
            root.path().join("observation"),
            DigestAuthority::new([29; 32]),
        )
        .unwrap(),
    );
    let turns: Vec<RoutingTimelineTurnV1> = ["applied", "suppressed", "retained_unavailable", "not_needed"].into_iter().enumerate().map(|(i, outcome)| {
        let protected = matches!(outcome, "applied" | "retained_unavailable");
        serde_json::from_value(serde_json::json!({
            "turn_id":format!("turn-{i}"),"agent_turn_id":null,"segment_id":null,
            "started_at_ms":1000+i,"finished_at_ms":null,"boundary":"new_user_message",
            "plan_id":"trial","plan_revision":8,"branch_id":"smart_saving",
            "model_configuration_id":"trial-model","reasoning_profile_id":null,
            "group":if protected {"primary"} else {"regular"},
            "selection":{
                "execution_group":if protected {"primary"} else {"regular"},
                "simple_probability":0.94,"simple_threshold_millis":900,
                "selection_reason":if protected {"low_competence"} else {"simple_task"},
                "strategy_id":"routing","schema_version":"routing/v1",
                "payload_digest":CanonicalDigest::of_bytes(b"consumer-fixture"),
                "branch_id":"smart_saving","complexity_score":null,"threshold":null,
                "decision_source":"external_classifier","reason_codes":[],"matched_user_phrase_ids":[],
                "fallback_used":false,"classification_duration_micros":1,"fallback_reason":null,
                "historical_protection":{
                    "outcome":outcome,"previous_score":if outcome=="not_needed" {0.8} else {0.3},
                    "competence_floor_millis":500,
                    "applicability_probability":if outcome=="retained_unavailable" {None} else {Some(if outcome=="suppressed" {0.05} else {0.99})},
                    "applicability_boundary_millis":500
                }
            },
            "tool_continuations":0,"relays":[],"transitions":[],"assessment_state":"pending",
            "assessment":null,"competence_floor_millis":500,"protected_turn_ids":[],"status":"running",
            "requests":[],"cost_by_model":[],"evidence_partial":false,"content_digest":format!("initial-{i}")
        })).unwrap()
    }).collect();
    let mut changed = turns[0].clone();
    changed.status = "completed".into();
    changed.finished_at_ms = Some(2000);
    changed.content_digest = "later-completion".into();
    let pages = VecDeque::from([
        Ok(RoutingTimelinePageV1 {
            turns,
            watermark: "initial".into(),
            complete: true,
        }),
        // Exercise the production Application error envelope and installed CLI
        // retry after a successful page. The store deadline itself is covered
        // by Observation tests; this controlled failure covers its consumer.
        Err(ObservationQueryError::ReadTimeout),
        Ok(RoutingTimelinePageV1 {
            turns: vec![changed.clone()],
            watermark: "changed".into(),
            complete: true,
        }),
        Ok(RoutingTimelinePageV1 {
            turns: vec![changed],
            watermark: "same".into(),
            complete: true,
        }),
    ]);
    let (busy, reads) = mpsc::channel();
    let mut ports = runtime.application_ports();
    ports.observation = Arc::new(TracedStore {
        store,
        busy,
        pages: Some(Mutex::new(pages)),
    });
    let endpoint = root.path().join("runtime");
    let mut server = hiroute_daemon::control::start_control(
        hiroute_application::ApplicationService::new(ports),
        &endpoint,
    )
    .unwrap();
    let mut child = OwnedChild(Some(
        Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .args([
                "sessions",
                "timeline",
                "trial-session",
                "--follow",
                "--output",
                "json",
            ])
            .env("HIROUTE_RUNTIME_DIR", &endpoint)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    for failure in [false, true, false, false, false] {
        assert_eq!(
            reads
                .recv_timeout(Duration::from_secs(8))
                .expect("CLI did not recover from the timeout and consume the next page"),
            failure,
        );
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.0.as_ref().unwrap().id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "follow did not stop on Ctrl-C");
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.len(),
        5,
        "four initial turns and one completion; the equal digest is deduplicated"
    );
    for (event, outcome) in events.iter().zip([
        "applied",
        "suppressed",
        "retained_unavailable",
        "not_needed",
        "applied",
    ]) {
        let history = &event["turn"]["selection"]["historical_protection"];
        assert_eq!(history["outcome"], outcome);
        assert_eq!(history["competence_floor_millis"], 500);
        assert_eq!(
            history["applicability_probability"].is_null(),
            outcome == "retained_unavailable"
        );
    }
    assert_eq!(events[4]["turn"]["status"], "completed");
    server.shutdown();
    server.join(Duration::from_secs(3)).unwrap();
}
