//! Exercise the real observation callbacks in both legal orders. The listener
//! fixtures use the same oracle, so canonical-first deterministically covers
//! the scheduling order that failed on CI without sleeps or callback retries.
use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::server::core_runtime::model_ir::ModelUsage;
use crate::server::core_runtime::response_diagnostics_tests::assert_success_usage;
use hiroute_gateway_core::core::execution_plan::{
    CredentialRef, PlanRevision, ResolvedTargetBindingId,
};
use hiroute_gateway_core::runtime::attempt::{AttemptTransportFacts, CommitFence};
use hiroute_gateway_core::runtime::driver::{
    AttemptBudgetGrant, AttemptCleanupOutcome, AttemptCommitFacts, AttemptDownstreamOutcome,
    AttemptStreamOutcome, AttemptTerminationReason, CompletedAttemptObservation,
    ProviderClassificationFacts, RealtimeRoutingFacts, RouteDecisionId, UsageDimension, UsageFact,
};

struct RecordSink(Sender<Value>);
impl ObservationRecordSink for RecordSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        self.0
            .send(serde_json::from_slice(record.payload()).unwrap())
            .unwrap();
        Ok(accounted_acknowledgement(record))
    }
}

fn recorded_usage(canonical_first: bool) -> Vec<Value> {
    let (sender, receiver): (_, Receiver<Value>) = mpsc::channel();
    let mut sinks = GatewayObservationSinks::discard();
    sinks.execution_fact = Arc::new(RecordSink(sender));
    let request = request_with_sinks(
        OtelContentPolicy::Disabled,
        DiagnosticsPort::default(),
        true,
        sinks,
    );
    let credential = "credential/none/usage-order";
    request.no_credential_materialized("binding:test", credential);
    request.disposition_published(&PublishedDisposition {
        request_id: RequestId(9),
        attempt_id: AttemptId(4),
        generation: AttemptGeneration(2),
        disposition: Disposition::Accept,
    });
    assert!(request.accept_current("frame:test", 128).is_some());
    let now = Instant::now();
    let completion = CompletedAttemptObservation {
        route_decision_id: RouteDecisionId(1),
        attempt_id: AttemptId(4),
        generation: AttemptGeneration(2),
        binding: ResolvedTargetBindingId::new(PlanRevision(1), 1),
        credential_ref: CredentialRef::new(credential).unwrap(),
        budget: AttemptBudgetGrant {
            issued_at: now,
            allocated: Duration::from_secs(1),
            deadline: now + Duration::from_secs(1),
        },
        routing_facts: RealtimeRoutingFacts::default(),
        provider: Some(ProviderClassificationFacts {
            usage: Some(UsageFact {
                input: UsageDimension::reported(11),
                output: UsageDimension::reported(7),
                ..UsageFact::default()
            }),
            ..ProviderClassificationFacts::default()
        }),
        failure: None,
        transport: AttemptTransportFacts {
            started_at: now,
            upstream_protocol: None,
            connect_elapsed: None,
            request_write_elapsed: None,
            upstream_ttfb: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            upstream_body_bytes: 128,
            timeout: None,
        },
        disposition: Disposition::Accept,
        commits: AttemptCommitFacts {
            upstream_request: CommitFence::WriteConfirmed,
            downstream_headers: CommitFence::WriteConfirmed,
            downstream_semantic: CommitFence::WriteConfirmed,
        },
        stream: AttemptStreamOutcome::CompletedEos,
        downstream: AttemptDownstreamOutcome::Completed,
        cleanup: AttemptCleanupOutcome::Completed,
        ended_at: now,
        termination_reason: AttemptTerminationReason::AcceptedEos,
    };
    let usage = ModelUsage {
        input_tokens: Some(11),
        output_tokens: Some(7),
        ..ModelUsage::default()
    };
    if canonical_first {
        request.usage_model(&usage);
        request.completed_attempt(&completion, None);
    } else {
        request.completed_attempt(&completion, None);
        request.usage_model(&usage);
    }
    request.finish("accepted");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut records = Vec::new();
    // All callbacks above have returned; this final record is a barrier for
    // their already-enqueued facts, not a scheduling assumption about workers.
    loop {
        let record = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        let finished = record["fact"]["kind"] == "request_finished";
        records.push(record);
        if finished {
            break;
        }
    }
    let terminal = records.last().unwrap();
    assert_eq!(terminal["fact"]["outcome"], "accepted");
    assert_eq!(terminal["fact"]["facts_completeness"], "complete");
    assert_eq!(terminal["fact"]["attempts_started"], 1);
    assert_eq!(terminal["fact"]["attempts_finished"], 1);
    records
}

#[test]
fn canonical_usage_before_completion_satisfies_listener_oracle() {
    let records = recorded_usage(true);
    assert_success_usage(&records, 1);
    assert_eq!(
        records
            .iter()
            .filter(|r| r["fact"]["kind"] == "usage_and_cache")
            .count(),
        1
    );
}

#[test]
fn provider_completion_before_canonical_usage_satisfies_listener_oracle() {
    let records = recorded_usage(false);
    assert_success_usage(&records, 1);
    assert_eq!(
        records
            .iter()
            .filter(|r| r["fact"]["kind"] == "usage_and_cache")
            .count(),
        2
    );
}

#[test]
fn listener_usage_oracle_rejects_missing_duplicate_wrong_or_uncorrelated_usage() {
    let records = recorded_usage(false);
    for mutation in [
        "missing",
        "duplicate",
        "wrong_tokens",
        "wrong_attempt",
        "wrong_ordinal",
        "wrong_request",
        "unknown_source",
    ] {
        let mut invalid = records.clone();
        let position = invalid
            .iter()
            .position(|r| r["fact"]["source"] == "accepted_canonical_model_event")
            .unwrap();
        match mutation {
            "missing" => {
                invalid.remove(position);
            }
            "duplicate" => invalid.push(invalid[position].clone()),
            "wrong_tokens" => invalid[position]["fact"]["input_tokens"] = json!(12),
            "wrong_attempt" => invalid[position]["attempt_id"] = json!("another-attempt"),
            "wrong_ordinal" => invalid[position]["fact"]["ordinal"] = json!(2),
            "wrong_request" => {
                invalid[position]["correlation"]["request_id"] = json!("another-request")
            }
            "unknown_source" => {
                let mut unexpected = invalid[position].clone();
                unexpected["fact"]["source"] = json!("unknown");
                invalid.push(unexpected);
            }
            _ => unreachable!(),
        }
        assert!(
            std::panic::catch_unwind(|| assert_success_usage(&invalid, 1)).is_err(),
            "oracle accepted {mutation}"
        );
    }
}
