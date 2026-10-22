use super::*;

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
