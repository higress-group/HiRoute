use std::sync::Arc;

use hiroute_domain::{
    AttemptDispositionV1, AttemptDownstreamOutcomeV1, AttemptId, AttemptOutcomeV1,
    AttemptStreamOutcomeV1, AttemptTerminationReasonV1, CommitFenceV1, ContentMode, EventId,
    ExecutionFactV1, ExecutionRequestOutcomeV1, FactsCompleteness, ObservationFactsQueryV2,
    ObservationNackDetailV1, ObservationQueryPort, ObservationReaderContext,
    ObservationValueQueryV2, ReceiptId, RequestOutcome,
};
use tempfile::tempdir;

use crate::{
    FactChannel, LocalObservationStore, LocalObservationWriter, OfferOutcome, WriterCycleOutcome,
};

use super::support::*;

#[test]
fn postcommit_cancelled_receipt_retains_the_accepted_attempt_without_claiming_success() {
    assert_postcommit_receipt(
        AttemptOutcomeV1::PostcommitCancelled,
        AttemptDownstreamOutcomeV1::Cancelled,
        ExecutionRequestOutcomeV1::Cancelled,
        RequestOutcome::Cancelled,
    );
}

#[test]
fn postcommit_transport_failure_receipt_retains_the_accepted_attempt_without_claiming_success() {
    assert_postcommit_receipt(
        AttemptOutcomeV1::PostcommitTransportFailed,
        AttemptDownstreamOutcomeV1::Failed,
        ExecutionRequestOutcomeV1::PostcommitTransportFailed,
        RequestOutcome::PostcommitTransportFailed,
    );
}

fn assert_postcommit_receipt(
    attempt_outcome: AttemptOutcomeV1,
    downstream_outcome: AttemptDownstreamOutcomeV1,
    request_outcome: ExecutionRequestOutcomeV1,
    expected_outcome: RequestOutcome,
) {
    for completeness in [FactsCompleteness::Complete, FactsCompleteness::Partial] {
        let temporary = tempdir().unwrap();
        let fixture = Fixture::new("postcommit");
        let store = open_store(temporary.path());
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        let mut facts = request_facts(
            &fixture,
            "attempt-postcommit",
            finished("attempt-postcommit"),
            100,
        );
        facts.retain(|event| {
            !matches!(
                event.fact,
                ExecutionFactV1::UsageAndCache { .. } | ExecutionFactV1::ValueSnapshot { .. }
            )
        });
        for (index, event) in facts.iter_mut().enumerate() {
            event.sequence = index as u64 + 1;
            match &mut event.fact {
                ExecutionFactV1::AttemptFinished(attempt) => {
                    attempt.outcome = attempt_outcome;
                    attempt.downstream_outcome = downstream_outcome;
                    attempt.stream_outcome = AttemptStreamOutcomeV1::StreamStartedNoRetry;
                    attempt.termination_reason = AttemptTerminationReasonV1::StreamStartedNoRetry;
                    attempt.error_class = Some("stream_started_no_retry".into());
                    attempt.retryable = Some(false);
                }
                ExecutionFactV1::RequestFinished {
                    outcome,
                    facts_completeness,
                    ..
                } => {
                    *outcome = request_outcome;
                    *facts_completeness = completeness;
                }
                _ => {}
            }
            assert!(
                matches!(
                    offer_fact(&writer, &channel, event.clone()),
                    WriterCycleOutcome::Ack(_)
                ),
                "transport-accepted {attempt_outcome:?} must remain recordable at sequence {}",
                event.sequence
            );
        }
        let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
        let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
        assert_eq!(receipt.outcome, expected_outcome);
        assert_eq!(
            receipt.final_attempt_id,
            Some(AttemptId::parse("attempt-postcommit").unwrap())
        );
        assert_eq!(receipt.facts_completeness, completeness);
        assert!(receipt.ordered_facts.iter().any(|event| matches!(
            &event.fact, ExecutionFactV1::AttemptFinished(attempt) if attempt.outcome == attempt_outcome
        )));
        assert!(
            !receipt
                .ordered_facts
                .iter()
                .any(|event| matches!(event.fact, ExecutionFactV1::UsageAndCache { .. }))
        );
        let recorded: (bool, bool) = store.connection.lock().query_row(
            "SELECT outcome IS NOT NULL, finished_at_ms IS NOT NULL FROM logical_requests WHERE request_id=?1",
            [fixture.request.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(recorded, (true, true));
        assert!(matches!(
            offer_fact(&writer, &channel, facts.last().unwrap().clone()),
            WriterCycleOutcome::Ack(_)
        ));
        assert_eq!(
            store.get_receipt(&fixture.workspace, &receipt_id).unwrap(),
            receipt
        );
        let gaps: u64 = store
            .connection
            .lock()
            .query_row("SELECT COUNT(*) FROM observation_gaps", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(gaps, 0);
        drop(channel);
        drop(writer);
        drop(store);
        assert_eq!(
            open_store(temporary.path())
                .get_receipt(&fixture.workspace, &receipt_id)
                .unwrap(),
            receipt
        );
    }
}

#[test]
fn delivered_native_terminal_failure_freezes_a_failed_receipt_with_actual_attempt() {
    for model_event in ["response_incomplete", "response_failed"] {
        let temporary = tempdir().unwrap();
        let fixture = Fixture::new("delivered-native-failure");
        let store = open_store(temporary.path());
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        let mut facts = request_facts(&fixture, "attempt-native", finished("attempt-native"), 100);
        facts.retain(|event| !matches!(event.fact, ExecutionFactV1::ValueSnapshot { .. }));
        for (index, event) in facts.iter_mut().enumerate() {
            event.sequence = index as u64 + 1;
            match &mut event.fact {
                ExecutionFactV1::AttemptFinished(attempt) => {
                    assert_eq!(attempt.outcome, AttemptOutcomeV1::Accepted);
                    attempt.error_class = Some("provider_rejected".into());
                    attempt.retryable = Some(false);
                    attempt.provider_model_event = Some(model_event.into());
                }
                ExecutionFactV1::RequestFinished { outcome, .. } => {
                    *outcome = ExecutionRequestOutcomeV1::Failed;
                }
                _ => {}
            }
            assert!(matches!(
                offer_fact(&writer, &channel, event.clone()),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
        let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
        assert_eq!(receipt.outcome, RequestOutcome::Failed);
        assert_eq!(
            receipt.final_attempt_id,
            Some(AttemptId::parse("attempt-native").unwrap())
        );
        assert!(receipt.ordered_facts.iter().any(|event| matches!(
            &event.fact, ExecutionFactV1::AttemptFinished(attempt)
                if attempt.provider_model_event.as_deref() == Some(model_event)
                    && attempt.error_class.as_deref() == Some("provider_rejected")
        )));
        drop(channel);
        drop(writer);
        drop(store);
        assert_eq!(
            open_store(temporary.path())
                .get_receipt(&fixture.workspace, &receipt_id)
                .unwrap(),
            receipt
        );
    }
}

#[test]
fn postcommit_ordinal_still_rejects_attempts_without_transport_acceptance() {
    for outcome in [
        AttemptOutcomeV1::Rejected,
        AttemptOutcomeV1::FailedBeforeTransportAcceptance,
    ] {
        let temporary = tempdir().unwrap();
        let fixture = Fixture::new("uncommitted-ordinal");
        let store = open_store(temporary.path());
        let writer = writer(&store);
        let channel = fact_channel(&fixture, 512 * 1024);
        let mut facts = request_facts(
            &fixture,
            "attempt-uncommitted",
            finished("attempt-uncommitted"),
            100,
        );
        facts.retain(|event| {
            !matches!(
                event.fact,
                ExecutionFactV1::SemanticCommit { .. }
                    | ExecutionFactV1::UsageAndCache { .. }
                    | ExecutionFactV1::ValueSnapshot { .. }
            )
        });
        for (index, event) in facts.iter_mut().enumerate() {
            event.sequence = index as u64 + 1;
            match &mut event.fact {
                ExecutionFactV1::AttemptFinished(attempt) => {
                    attempt.outcome = outcome;
                    attempt.disposition = if outcome == AttemptOutcomeV1::Rejected {
                        AttemptDispositionV1::Continue
                    } else {
                        AttemptDispositionV1::Accept
                    };
                    attempt.commits.downstream_semantic = CommitFenceV1::Clear;
                    attempt.stream_outcome = AttemptStreamOutcomeV1::NotStarted;
                    attempt.downstream_outcome = AttemptDownstreamOutcomeV1::NotStarted;
                    attempt.termination_reason = AttemptTerminationReasonV1::AttemptFailure;
                }
                ExecutionFactV1::RequestFinished { outcome, .. } => {
                    *outcome = ExecutionRequestOutcomeV1::Failed
                }
                _ => {}
            }
        }
        let terminal = facts.pop().unwrap();
        for event in facts {
            assert!(matches!(
                offer_fact(&writer, &channel, event),
                WriterCycleOutcome::Ack(_)
            ));
        }
        let WriterCycleOutcome::Nack(nack) = offer_fact(&writer, &channel, terminal) else {
            panic!("uncommitted {outcome:?} cannot be an accepted ordinal");
        };
        assert!(matches!(
            nack.detail,
            ObservationNackDetailV1::ImmutableProjectionConflict { .. }
        ));
        assert!(!nack.retryable);
        assert!(
            store
                .get_receipt(
                    &fixture.workspace,
                    &ReceiptId::parse(fixture.request.as_str()).unwrap()
                )
                .is_err()
        );
    }
}

#[test]
fn late_usage_after_request_finish_is_queryable_and_does_not_poison_the_fact_stream() {
    let temporary = tempdir().unwrap();
    let fixture = Fixture::new("late-usage");
    let store = open_store(temporary.path());
    let writer = writer(&store);
    let channel = fact_channel(&fixture, 512 * 1024);
    let mut terminal_facts = request_facts(&fixture, "attempt-late", finished("attempt-late"), 100);
    let mut late_usage = terminal_facts.remove(6);
    for (index, fact) in terminal_facts.iter_mut().enumerate() {
        fact.sequence = index as u64 + 1;
    }
    late_usage.sequence = 10;

    for fact in terminal_facts {
        assert!(matches!(
            offer_fact(&writer, &channel, fact),
            WriterCycleOutcome::Ack(_)
        ));
    }
    let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
    let frozen_receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
    assert_eq!(frozen_receipt.ordered_facts.len(), 9);
    assert!(matches!(
        frozen_receipt.ordered_facts.last().map(|event| &event.fact),
        Some(ExecutionFactV1::RequestFinished { .. })
    ));

    let WriterCycleOutcome::Ack(late_usage_ack) = offer_fact(&writer, &channel, late_usage) else {
        panic!("usage reported after request finish must remain independently durable")
    };
    assert_eq!(late_usage_ack.highest_contiguous_sequence, 10);
    assert_eq!(
        store.get_receipt(&fixture.workspace, &receipt_id).unwrap(),
        frozen_receipt,
        "late usage must not mutate the immutable routing receipt"
    );

    let reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local".into(),
        1,
        10_000,
        false,
        false,
    )
    .unwrap();
    let facts = store
        .observed_facts(
            &reader,
            &ObservationFactsQueryV2 {
                include_routing_observations: false,
                request_id: fixture.request.clone(),
                limit: 20,
                cursor: None,
            },
            1_000,
        )
        .unwrap();
    assert!(!facts.projection_partial);
    assert_eq!(facts.facts.len(), 10);
    assert_eq!(facts.facts[8].event_kind, "request_finished");
    assert_eq!(facts.facts[9].event_kind, "usage_and_cache");
    assert_eq!(facts.facts[9].input_tokens, Some(100));
    assert_eq!(facts.facts[9].output_tokens, Some(25));

    assert_eq!(store.settle_pending_valuations(16).unwrap(), 1);
    let totals = store
        .observed_value_totals(
            &reader,
            &ObservationValueQueryV2 {
                from_ms: 0,
                to_ms: 1_000,
                session_id: Some(fixture.session.to_string()),
                plan_id: None,
                currency: None,
            },
            1_000,
        )
        .unwrap();
    assert_eq!(
        totals
            .usage
            .iter()
            .find(|metric| metric.metric == "input")
            .and_then(|metric| metric.known_sum),
        Some(100)
    );

    let mut next = Fixture::new("after-late-usage");
    next.session = fixture.session.clone();
    next.fact_stream = fixture.fact_stream.clone();
    let mut next_facts = request_facts(&next, "attempt-next", finished("attempt-next"), 200);
    for (index, fact) in next_facts.iter_mut().enumerate() {
        fact.sequence = index as u64 + 11;
        fact.event_id = EventId::parse(format!("next-fact-event-{}", fact.sequence)).unwrap();
    }
    for (index, fact) in next_facts.into_iter().enumerate() {
        let WriterCycleOutcome::Ack(ack) = offer_fact(&writer, &channel, fact) else {
            panic!("the request after late usage must not inherit a sink NACK gap")
        };
        assert_eq!(ack.highest_contiguous_sequence, index as u64 + 11);
    }
    store
        .get_receipt(
            &next.workspace,
            &ReceiptId::parse(next.request.as_str()).unwrap(),
        )
        .unwrap();
    let gap_count: u64 = store
        .connection
        .lock()
        .query_row("SELECT COUNT(*) FROM observation_gaps", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(gap_count, 0);
}

#[test]
fn routing_attempt_receipt_and_value_survive_content_v2_migration_and_restart() {
    let temporary = tempdir().unwrap();
    let fixture = Fixture::new("v2-regression");
    let store = open_store(temporary.path());
    let writer = writer(&store);
    let channel = fact_channel(&fixture, 512 * 1024);
    let facts = request_facts(&fixture, "attempt-v2", finished("attempt-v2"), 100);

    for (index, fact) in facts.iter().cloned().enumerate() {
        let WriterCycleOutcome::Ack(ack) = offer_fact(&writer, &channel, fact) else {
            panic!("execution fact must remain accepted");
        };
        assert_eq!(ack.identity.channel, "execution_fact");
        assert_eq!(ack.highest_contiguous_sequence, index as u64 + 1);
        assert!(ack.content_acknowledgement.is_none());
    }

    let receipt_id = ReceiptId::parse(fixture.request.as_str()).unwrap();
    let receipt = store.get_receipt(&fixture.workspace, &receipt_id).unwrap();
    assert_eq!(receipt.ordered_facts.len(), 10);
    assert_eq!(receipt.trust.gateway_publication_revision, "41");
    assert!(receipt.ordered_facts.iter().any(|fact| matches!(
        fact.fact,
        ExecutionFactV1::AttemptStarted { ref model_configuration_id, .. }
            if model_configuration_id == "model-a"
    )));
    let value = store
        .get_value(
            &fixture.workspace,
            &value_query("plan/codex-daily", "USD", Some(fixture.session.clone())),
        )
        .unwrap();
    assert_eq!(value.estimated_total_savings_micros, Some(900));
    let session = store
        .get_session(&fixture.workspace, &fixture.session, ContentMode::None)
        .unwrap();
    assert_eq!(session.turns[0].request_ids, vec![fixture.request.clone()]);

    drop(channel);
    drop(writer);
    drop(store);
    let reopened = Arc::new(LocalObservationStore::open(temporary.path(), authority()).unwrap());
    let writer = LocalObservationWriter::new(reopened.clone());
    let replay = FactChannel::new(fixture.fact_stream.clone(), 64 * 1024);
    assert_eq!(replay.offer(facts[9].clone()), OfferOutcome::Accepted);
    let WriterCycleOutcome::Ack(ack) = writer.consume_fact(&replay) else {
        panic!("durable execution-fact ACK must replay after restart");
    };
    assert_eq!(ack.highest_contiguous_sequence, 10);
    assert_eq!(
        reopened
            .get_receipt(&fixture.workspace, &receipt_id)
            .unwrap(),
        receipt
    );

    {
        let connection = reopened.connection.lock();
        let body: String = connection
            .query_row(
                "SELECT body_json FROM observation_sensitive_payloads_v2 WHERE id=(SELECT 'fact:'||envelope_digest FROM execution_fact_events WHERE sequence=1)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tampered = body.replace("authority-local", "authority-tampered");
        assert_ne!(tampered, body);
        connection
            .execute(
                "UPDATE observation_sensitive_payloads_v2 SET body_json=?1 WHERE id=(SELECT 'fact:'||envelope_digest FROM execution_fact_events WHERE sequence=1)",
                [tampered],
            )
            .unwrap();
    }
    let tampered_replay = FactChannel::new(fixture.fact_stream.clone(), 64 * 1024);
    assert_eq!(
        tampered_replay.offer(facts[9].clone()),
        OfferOutcome::Accepted
    );
    assert_eq!(
        writer.consume_fact(&tampered_replay),
        WriterCycleOutcome::StoreFailed(crate::writer::ObservationStoreError::Corrupt),
    );
}
