use super::*;

fn decision() -> profiles::BranchDecisionV1 {
    let strategy = profiles::ComplexityV1::compile(Vec::<(String, String)>::new()).unwrap();
    profiles::ComplexityV1::decide(Some("simple"), None, &strategy)
        .unwrap()
        .0
}

#[test]
fn admitted_capture_survives_busy_decision_index_and_remains_evaluable() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut admitted = fixture.begin(&first, fixture.prepare(&first));
    let guard = admitted.guard.as_mut().unwrap();
    let selected = decision();
    fixture
        .runtime
        .agent_turn_history
        .with_lock_for_runtime_test(|| {
            guard.commit_decision_or_defer(&selected).unwrap();
        });
    assert!(guard.decision_deferred);
    let turn_id = guard.ticket().agent_turn_id.clone();
    // Actual delivered output is captured before the deferred decision is indexed.
    fixture
        .runtime
        .agent_turn_history
        .accept_output(guard.ticket(), &[text_event("actual accepted answer")]);
    guard.finish_deferred_decision(Some(&selected)).unwrap();
    assert!(!guard.decision_deferred);
    let completed = fixture
        .runtime
        .agent_turn_history
        .finish_request(
            guard.ticket(),
            "actual-request".into(),
            AgentTurnStatus::Completed,
            vec![crate::agent_turn_history::AcceptedExecution {
                branch_execution: None,
                model_configuration_id: "actual-model".into(),
                profile_digest: "actual-profile".into(),
                executed_branch_id: selected.branch_id.clone(),
                request_id: "actual-request".into(),
            }],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.agent_turn_id, turn_id);
    guard.disarm();
    drop(admitted);

    let second = Fixture::request(json!([
        {"role":"user","content":"first"},
        {"role":"assistant","content":"actual accepted answer"},
        {"role":"user","content":"second"}
    ]));
    let next = fixture.begin(&second, fixture.prepare(&second));
    let target = next.history.assessment_target.as_ref().unwrap();
    assert_eq!(target.through_turn_id, turn_id);
    assert!(!target.target_partial);
    assert!(matches!(&target.attribution,
        crate::agent_turn_history::ExecutionAttribution::Single {
            model_configuration_id, profile_digest, ..
        } if model_configuration_id == "actual-model" && profile_digest == "actual-profile"
    ));
    assert_eq!(
        next.history.visible_conversation[0].steps,
        vec![vec![VisibleContentPart::Text {
            text: "actual accepted answer".into()
        }]]
    );
}

#[test]
fn deferred_decision_cannot_reuse_a_revoked_ticket() {
    let fixture = Fixture::new();
    let request = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut admitted = fixture.begin(&request, fixture.prepare(&request));
    let guard = admitted.guard.as_mut().unwrap();
    let selected = decision();
    fixture
        .runtime
        .agent_turn_history
        .with_lock_for_runtime_test(|| {
            guard.commit_decision_or_defer(&selected).unwrap();
        });
    fixture.runtime.agent_turn_history.abort(guard.ticket());
    assert_eq!(
        guard.finish_deferred_decision(Some(&selected)),
        Err(AgentTurnHistoryError::TurnContextUnavailable)
    );
    let fresh = fixture.begin(&request, fixture.prepare(&request));
    assert!(fresh.history.assessment_target.is_none());
}

#[test]
fn deferred_decision_keeps_capacity_failure_explicit_and_recovers_on_fresh_input() {
    let mut fixture = Fixture::new();
    fixture.runtime.agent_turn_history = Arc::new(
        crate::agent_turn_history::AgentTurnHistoryStore::new(128 * 1024, Duration::from_secs(60)),
    );
    let request = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut admitted = fixture.begin(&request, fixture.prepare(&request));
    let guard = admitted.guard.as_mut().unwrap();
    let mut oversized = decision();
    oversized.strategy_id = "x".repeat(256 * 1024);
    guard.commit_decision_or_defer(&oversized).unwrap();
    fixture
        .runtime
        .agent_turn_history
        .accept_output(guard.ticket(), &[text_event("delivered")]);
    assert_eq!(
        guard.finish_deferred_decision(Some(&oversized)),
        Err(AgentTurnHistoryError::Resource)
    );
    drop(admitted); // A failed optional publication retains the gap fence.
    let a = Fixture::request(json!([{"role":"user","content":"new A"}]));
    let mut captured = fixture.begin(&a, fixture.prepare(&a));
    assert!(captured.history.assessment_target.is_none());
    fixture.finish(&mut captured, &[text_event("new actual A answer")], true);
    drop(captured);
    let b = Fixture::request(json!([
        {"role":"user","content":"new A"},
        {"role":"assistant","content":"new actual A answer"},
        {"role":"user","content":"new B"}
    ]));
    let next = fixture.begin(&b, fixture.prepare(&b));
    assert!(
        !next
            .history
            .assessment_target
            .as_ref()
            .unwrap()
            .target_partial
    );
    let visible = serde_json::to_string(&next.history.visible_conversation).unwrap();
    assert!(visible.contains("new actual A answer"));
    assert!(!visible.contains("delivered"));
}
