//! Capture, common evaluation scope and single-model attribution are independent.
use super::native_retry::begin;
use super::*;
use hiroute_domain::{BranchExecutionV1, ExecutionGroupV1};

fn policy() -> hiroute_domain::BranchExecutionPolicyV1 {
    let mut judgment = hiroute_domain::JudgmentSettingsV1::default();
    let protocol = hiroute_domain::NativeProtocolV1::default();
    protocol.stock_assessment(&mut judgment);
    let frozen = hiroute_domain::FrozenNativeProtocolV1::freeze(
        &protocol,
        judgment.degree.definition().into(),
        [("regular".into(), judgment.clone())],
    )
    .unwrap();
    frozen.execution_policy("regular", &judgment, "Task")
}

fn executed(id: &str, model: &str, group: ExecutionGroupV1) -> AcceptedExecution {
    let mut value = execution(model, model, "regular", id);
    value.branch_execution = Some(BranchExecutionV1 {
        policy: policy(),
        group,
        candidate_index: 0,
    });
    value
}

#[test]
fn composite_retry_is_fully_captured_evaluated_once_and_next_single_stage_recovers() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store.scope_key("workspace", "composite").unwrap();
    let mut input = vec![text(MessageRole::User, "A")];
    let (a, _) = new_turn(begin(
        &store,
        &scope,
        &request(input.clone()),
        &replay,
        Some(1),
    ));
    let mut selected = decision("regular");
    selected.policy = Some(policy());
    store.commit_decision(&a, selected).unwrap();
    stage_closure::accepted_tool(&store, &a, "lookup");
    assert!(
        store
            .finish_request(
                &a,
                "flash-tool".into(),
                AgentTurnStatus::Completed,
                vec![executed("flash-tool", "flash", ExecutionGroupV1::Regular)],
                false,
                Instant::now()
            )
            .unwrap()
            .is_none()
    );
    input.extend([tool_call("lookup", "lookup"), tool_result("lookup", false)]);
    let AgentTurnBegin::Continuation {
        ticket: continued, ..
    } = begin(&store, &scope, &request(input.clone()), &replay, Some(1))
    else {
        panic!("tool continuation")
    };
    let failed = store
        .finish_request(
            &continued,
            "astra-500".into(),
            AgentTurnStatus::Failed,
            vec![],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    let frozen_failed = failed.history_membership.clone();
    let AgentTurnBegin::Continuation { ticket: retry, .. } =
        begin(&store, &scope, &request(input.clone()), &replay, Some(1))
    else {
        panic!("exact retry")
    };
    accepted_output::text_output(&store, &retry, "A complete");
    let complete = store
        .finish_request(
            &retry,
            "astra-retry".into(),
            AgentTurnStatus::Completed,
            vec![executed("astra-retry", "astra", ExecutionGroupV1::Primary)],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert!(matches!(complete.attribution, ExecutionAttribution::Mixed));
    assert!(!complete.history_partial);
    assert!(
        !complete
            .history_membership
            .as_ref()
            .unwrap()
            .capture_partial
    );
    assert_eq!(failed.history_membership, frozen_failed);
    assert_eq!(failed.status, AgentTurnStatus::Failed);
    input.extend([
        text(MessageRole::Assistant, "A complete"),
        text(MessageRole::User, "B"),
    ]);
    let (b, history) = new_turn(begin(
        &store,
        &scope,
        &request(input.clone()),
        &replay,
        Some(2),
    ));
    let target = history
        .assessment_target
        .as_ref()
        .expect("joint actual execution must remain evaluable");
    assert_eq!(
        target.history_membership.as_ref().unwrap().request_ids,
        ["flash-tool", "astra-500", "astra-retry"]
    );
    assert!(!target.target_partial);
    assert!(
        target
            .history_membership
            .as_ref()
            .unwrap()
            .source_unverified
    );
    assert!(target.evaluation_scope.is_some());
    assert!(target.branch_execution.is_none());
    drop(history);
    let mut selected = decision("regular");
    selected.policy = Some(policy());
    store.commit_decision(&b, selected).unwrap();
    accepted_output::text_output(&store, &b, "B complete");
    store
        .finish_request(
            &b,
            "b-flash".into(),
            AgentTurnStatus::Completed,
            vec![executed("b-flash", "flash", ExecutionGroupV1::Regular)],
            true,
            Instant::now(),
        )
        .unwrap();
    input.extend([
        text(MessageRole::Assistant, "B complete"),
        text(MessageRole::User, "C"),
    ]);
    let (c, history) = new_turn(begin(&store, &scope, &request(input), &replay, Some(3)));
    let recovered = history.assessment_target.as_ref().unwrap();
    assert!(matches!(
        recovered.attribution,
        ExecutionAttribution::Single { .. }
    ));
    assert!(recovered.evaluation_scope.is_none());
    assert_eq!(
        recovered.history_membership.as_ref().unwrap().request_ids,
        ["b-flash"]
    );
    store.abort(&c);
}

#[test]
fn different_rubrics_withdraw_composite_grading_without_claiming_capture_loss() {
    let (_directory, replay) = replay();
    let store = AgentTurnHistoryStore::default();
    let scope = store
        .scope_key("workspace", "incompatible-rubrics")
        .unwrap();
    let input = request(vec![text(MessageRole::User, "A")]);
    let (a, _) = new_turn(begin(&store, &scope, &input, &replay, Some(1)));
    store.commit_decision(&a, decision("regular")).unwrap();
    let first = executed("first", "flash", ExecutionGroupV1::Regular);
    let mut second = executed("second", "astra", ExecutionGroupV1::Primary);
    second
        .branch_execution
        .as_mut()
        .unwrap()
        .policy
        .criteria_digest = Some(hiroute_domain::CanonicalDigest::of_bytes(b"another rubric"));
    let complete = store
        .finish_request(
            &a,
            "actual".into(),
            AgentTurnStatus::Completed,
            vec![first, second],
            true,
            Instant::now(),
        )
        .unwrap()
        .unwrap();
    assert!(matches!(complete.attribution, ExecutionAttribution::Mixed));
    assert!(complete.evaluation_scope.is_none());
    assert!(!complete.history_partial);
    assert!(!complete.history_membership.unwrap().capture_partial);
}
