use super::*;

#[test]
fn admission_is_emitted_before_any_planner_decision_with_request_envelope_identity() {
    use std::sync::mpsc;
    use std::time::Duration;

    struct Capture(mpsc::Sender<Value>);
    impl ObservationRecordSink for Capture {
        fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
            self.0
                .send(serde_json::from_slice(record.payload()).unwrap())
                .unwrap();
            Ok(accounted_acknowledgement(record))
        }
    }
    let (sender, receiver) = mpsc::channel();
    let request = request_with_sinks(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            execution_fact: Arc::new(Capture(sender)),
            ..GatewayObservationSinks::discard()
        },
    );
    let admission = hiroute_domain::AgentTurnAdmissionV1 {
        version: 1,
        agent_turn_id: "turn/admitted".into(),
        branch_id: "history/branch".into(),
        revision: 2,
        ordinal: 3,
        opening_cause: hiroute_domain::AgentTurnOpeningCauseV1::NoComparableHistory,
    };
    request.agent_turn_admitted(&admission);
    let wire = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(wire["fact"]["kind"], "agent_turn_admitted");
    assert_eq!(
        wire["correlation"]["request_id"],
        request.logical_request_id()
    );
    assert!(
        wire["producer"]["producer_epoch"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    );
    assert_eq!(wire["agent_plan_id"], "plan/test");
    assert_eq!(wire["route"]["revision"], 5);
    assert!(wire["attempt_id"].is_null());
    assert!(wire.get("pricing").is_none());
    let decoded: ExecutionFactEnvelopeV1 = serde_json::from_value(wire).unwrap();
    assert!(
        matches!(decoded.fact, ExecutionFactV1::AgentTurnAdmitted(value) if value == admission)
    );
    assert!(
        receiver.try_recv().is_err(),
        "admission must not manufacture a decision or completion"
    );
}

#[test]
fn native_timeline_early_turn_facts_keep_authorized_plan_identity() {
    use std::sync::mpsc;
    use std::time::Duration;

    struct Capture(mpsc::Sender<Value>);
    impl ObservationRecordSink for Capture {
        fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
            self.0
                .send(serde_json::from_slice(record.payload()).unwrap())
                .unwrap();
            Ok(accounted_acknowledgement(record))
        }
    }
    let (sender, receiver) = mpsc::channel();
    let discard = GatewayObservationSinks::discard();
    let request = request_with_sinks(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        GatewayObservationSinks {
            execution_fact: Arc::new(Capture(sender)),
            ..discard
        },
    );
    // A preceding tool stage may close at ingress, before this request is planned.
    request.agent_turn_finished(&crate::agent_turn_history::CompletedAgentTurn {
        evaluation_scope: None,
        history_membership: None,
        opening_cause: hiroute_domain::AgentTurnOpeningCauseV1::NoComparableHistory,
        branch_execution: None,
        agent_turn_id: "turn/previous".into(),
        segment_id: "segment/previous".into(),
        ordinal: 1,
        plan: crate::agent_turn_history::PlanSnapshot {
            plan_id: "plan/test".into(),
            plan_revision: 5,
        },
        selected_branch_id: "smart_saving".into(),
        attribution: crate::agent_turn_history::ExecutionAttribution::Unknown,
        started_at_ms: 1000,
        finished_at_ms: 2000,
        status: crate::agent_turn_history::AgentTurnStatus::Unknown,
        history_partial: true,
        first_request_id: Some("request/previous".into()),
        last_request_id: Some("request/previous".into()),
    });
    let fact = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(fact["fact"]["kind"], "agent_turn_finished");
    assert_eq!(fact["agent_plan_id"], "plan/test");
    assert_eq!(fact["route"]["revision"], 5);
}
