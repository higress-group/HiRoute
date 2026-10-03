//! Immutable Receipt and Value projections derived from the durable execution-fact log.

mod completeness;
mod plan_quality;
mod validation;

use hiroute_domain::{
    CanonicalDigest, ExecutionFactEnvelopeV1, ExecutionFactV1, ExecutionRequestOutcomeV1,
    FactsCompleteness, LogicalRequestId, ROUTING_RECEIPT_SCHEMA_V1, ReceiptExecutionEventV1,
    ReceiptId, RequestOutcome, RoutingReceiptV1, SessionId, TrafficKind, VALUE_LEDGER_SCHEMA_V1,
    ValueLedgerEntryV1, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::store::fact_log;
use crate::writer::ObservationStoreError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FactProjectionError {
    MissingPrerequisite,
    ImmutableConflict,
    Invalid,
    Storage,
    Corrupt,
}

pub(crate) fn apply_fact(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), FactProjectionError> {
    let facts = fact_log::load_request(
        transaction,
        &envelope.correlation.workspace_id,
        &envelope.correlation.request_id,
    )
    .map_err(map_store_error)?;
    validation::ordered_facts(&facts, false)?;
    ensure_scope(transaction, &facts, envelope)?;
    plan_quality::apply(transaction, envelope)?;
    if matches!(envelope.fact, ExecutionFactV1::RequestFinished { .. }) {
        let receipt = build_receipt(transaction, &facts, None)?;
        immutable_receipt_insert(transaction, &receipt)?;
        project_value(transaction, &receipt)?;
        transaction
            .execute(
                "UPDATE logical_requests SET outcome=?3, receipt_id=?4, finished_at_ms=?5
                 WHERE workspace_id=?1 AND request_id=?2 AND outcome IS NULL",
                params![
                    receipt.workspace_id.as_str(),
                    receipt.request_id.as_str(),
                    enum_json(&receipt.outcome)?,
                    receipt.receipt_id.as_str(),
                    receipt.frozen_at_ms,
                ],
            )
            .map_err(|_| FactProjectionError::Storage)?;
    }
    touch_session(transaction, envelope)
}

pub(crate) fn build_receipt_for_request(
    connection: &Connection,
    workspace_id: &hiroute_domain::WorkspaceId,
    request_id: &LogicalRequestId,
    frozen_completeness: FactsCompleteness,
) -> Result<RoutingReceiptV1, FactProjectionError> {
    let facts =
        fact_log::load_request(connection, workspace_id, request_id).map_err(map_store_error)?;
    validation::ordered_facts(&facts, false)?;
    build_receipt(connection, &facts, Some(frozen_completeness))
}

fn ensure_scope(
    transaction: &Transaction<'_>,
    facts: &[ExecutionFactEnvelopeV1],
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), FactProjectionError> {
    let first = facts
        .first()
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let correlation = &first.correlation;
    let started_at_ms = first
        .occurred_at_ms()
        .map_err(|_| FactProjectionError::Invalid)?;
    let correlation_name = enum_json(&correlation.correlation_provenance)?;
    let session_scope = enum_json(&correlation.session_scope)?;
    let explicit_traffic = facts.iter().find_map(|envelope| match envelope.fact {
        ExecutionFactV1::ValueSnapshot { traffic_kind, .. } => Some(traffic_kind),
        _ => None,
    });
    let has_usage = facts
        .iter()
        .any(|envelope| matches!(envelope.fact, ExecutionFactV1::UsageAndCache { .. }));
    // Run relations and execution facts use independent bounded delivery channels. If the
    // verified relation wins the race, recover its normal-traffic classification while
    // projecting the first fact. A conflicted relation quarantines the request as unknown.
    let (linked_normal, relation_conflicted): (bool, bool) = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM observation_run_links
                     WHERE workspace_id=?1 AND request_id=?2 AND conflicted=0),
                    EXISTS(SELECT 1 FROM observation_run_links
                     WHERE workspace_id=?1 AND request_id=?2 AND conflicted!=0)",
            params![
                correlation.workspace_id.as_str(),
                correlation.request_id.as_str()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| FactProjectionError::Storage)?;
    let initial_traffic = classified_traffic(
        "unknown",
        explicit_traffic,
        linked_normal,
        has_usage,
        relation_conflicted,
    )?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO sessions
             (workspace_id, session_id, agent_id, correlation, started_at_ms, updated_at_ms,
              facts_completeness, content_completeness)
             VALUES (?1, ?2, '', ?3, ?4, ?4, 'complete', 'unknown')",
            params![
                correlation.workspace_id.as_str(),
                correlation.conversation_id.as_str(),
                correlation_name,
                started_at_ms,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    let existing_correlation: String = transaction
        .query_row(
            "SELECT correlation FROM sessions WHERE workspace_id=?1 AND session_id=?2",
            params![
                correlation.workspace_id.as_str(),
                correlation.conversation_id.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if existing_correlation == "unproven" && correlation_name != "unproven" {
        transaction
            .execute(
                "UPDATE sessions SET correlation=?3
                 WHERE workspace_id=?1 AND session_id=?2",
                params![
                    correlation.workspace_id.as_str(),
                    correlation.conversation_id.as_str(),
                    correlation_name,
                ],
            )
            .map_err(|_| FactProjectionError::Storage)?;
    } else if existing_correlation != correlation_name {
        return Err(FactProjectionError::ImmutableConflict);
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO turns
             (workspace_id, turn_id, session_id, started_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![
                correlation.workspace_id.as_str(),
                correlation.turn_id.as_str(),
                correlation.conversation_id.as_str(),
                started_at_ms,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    let turn_session: String = transaction
        .query_row(
            "SELECT session_id FROM turns WHERE workspace_id=?1 AND turn_id=?2",
            params![
                correlation.workspace_id.as_str(),
                correlation.turn_id.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if turn_session != correlation.conversation_id.as_str() {
        return Err(FactProjectionError::ImmutableConflict);
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO logical_requests
             (workspace_id, request_id, session_id, turn_id, session_scope,
              correlation_provenance, traffic_kind, started_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                correlation.workspace_id.as_str(),
                correlation.request_id.as_str(),
                correlation.conversation_id.as_str(),
                correlation.turn_id.as_str(),
                session_scope,
                correlation_name,
                initial_traffic,
                started_at_ms,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    transaction
        .execute(
            "UPDATE logical_requests SET session_scope=?3, correlation_provenance=?4
             WHERE workspace_id=?1 AND request_id=?2
               AND session_scope IS NULL AND correlation_provenance IS NULL",
            params![
                correlation.workspace_id.as_str(),
                correlation.request_id.as_str(),
                session_scope,
                correlation_name,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    let request_scope: (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = transaction
        .query_row(
            "SELECT session_id, turn_id, traffic_kind, outcome, session_scope,
                    correlation_provenance FROM logical_requests
             WHERE workspace_id=?1 AND request_id=?2",
            params![
                correlation.workspace_id.as_str(),
                correlation.request_id.as_str()
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if request_scope.0 != correlation.conversation_id.as_str()
        || request_scope.1 != correlation.turn_id.as_str()
        || request_scope.4.as_deref() != Some(session_scope.as_str())
        || request_scope.5.as_deref() != Some(correlation_name.as_str())
        || request_scope.3.is_some()
            && !matches!(
                envelope.fact,
                ExecutionFactV1::RequestFinished { .. } | ExecutionFactV1::UsageAndCache { .. }
            )
    {
        return Err(FactProjectionError::ImmutableConflict);
    }
    let next_traffic = classified_traffic(
        &request_scope.2,
        explicit_traffic,
        linked_normal,
        has_usage,
        relation_conflicted,
    )?;
    if request_scope.2 != next_traffic {
        transaction
            .execute(
                "UPDATE logical_requests SET traffic_kind=?3, started_at_ms=MIN(started_at_ms, ?4)
                 WHERE workspace_id=?1 AND request_id=?2",
                params![
                    correlation.workspace_id.as_str(),
                    correlation.request_id.as_str(),
                    next_traffic,
                    started_at_ms,
                ],
            )
            .map_err(|_| FactProjectionError::Storage)?;
    }
    completeness::append(transaction, envelope)
}

fn classified_traffic(
    current: &str,
    explicit: Option<TrafficKind>,
    linked_normal: bool,
    has_usage: bool,
    relation_conflicted: bool,
) -> Result<&'static str, FactProjectionError> {
    if !matches!(current, "unknown" | "normal" | "connectivity_probe") {
        return Err(FactProjectionError::Corrupt);
    }
    if relation_conflicted {
        return Ok("unknown");
    }
    if current == "connectivity_probe" || explicit == Some(TrafficKind::ConnectivityProbe) {
        return Ok("connectivity_probe");
    }
    if current == "normal" || explicit == Some(TrafficKind::Normal) || linked_normal || has_usage {
        return Ok("normal");
    }
    Ok("unknown")
}

fn build_receipt(
    connection: &Connection,
    facts: &[ExecutionFactEnvelopeV1],
    frozen_completeness: Option<FactsCompleteness>,
) -> Result<RoutingReceiptV1, FactProjectionError> {
    let finish_index = facts
        .iter()
        .position(|envelope| matches!(envelope.fact, ExecutionFactV1::RequestFinished { .. }))
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let receipt_facts = &facts[..=finish_index];
    let first = receipt_facts
        .first()
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let finish = receipt_facts
        .last()
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let ExecutionFactV1::RequestFinished {
        outcome,
        accepted_attempt_ordinal,
        ..
    } = &finish.fact
    else {
        return Err(FactProjectionError::MissingPrerequisite);
    };
    let final_attempt_id = accepted_attempt_ordinal.and_then(|accepted| {
        receipt_facts
            .iter()
            .find_map(|envelope| match &envelope.fact {
                ExecutionFactV1::AttemptStarted { ordinal, .. } if *ordinal == accepted => {
                    envelope.attempt_id.clone()
                }
                _ => None,
            })
    });
    let facts_completeness =
        frozen_completeness.map_or_else(|| receipt_completeness(connection, receipt_facts), Ok)?;
    validation::ordered_facts(
        receipt_facts,
        facts_completeness == FactsCompleteness::Complete,
    )?;
    let receipt = RoutingReceiptV1 {
        schema: ROUTING_RECEIPT_SCHEMA_V1.to_owned(),
        receipt_id: ReceiptId::parse(first.correlation.request_id.as_str().to_owned())
            .map_err(|_| FactProjectionError::Invalid)?,
        workspace_id: first.correlation.workspace_id.clone(),
        session_id: first.correlation.conversation_id.clone(),
        turn_id: first.correlation.turn_id.clone(),
        request_id: first.correlation.request_id.clone(),
        producer: first.producer.clone(),
        correlation: first.correlation.clone(),
        trust: first.trust.clone(),
        ordered_facts: receipt_facts
            .iter()
            .map(|envelope| ReceiptExecutionEventV1 {
                sequence: envelope.sequence,
                event_id: envelope.event_id.clone(),
                attempt_id: envelope.attempt_id.clone(),
                occurred_at_unix_nanos: envelope.occurred_at_unix_nanos,
                fact: envelope.fact.clone(),
            })
            .collect(),
        outcome: request_outcome(*outcome),
        final_attempt_id,
        facts_completeness,
        frozen_at_ms: finish
            .occurred_at_ms()
            .map_err(|_| FactProjectionError::Invalid)?,
    };
    receipt
        .validate()
        .map_err(|_| FactProjectionError::Invalid)?;
    Ok(receipt)
}

fn receipt_completeness(
    connection: &Connection,
    facts: &[ExecutionFactEnvelopeV1],
) -> Result<FactsCompleteness, FactProjectionError> {
    let first = facts
        .first()
        .ok_or(FactProjectionError::MissingPrerequisite)?;
    let has_gap: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM observation_gaps
             WHERE channel='fact' AND workspace_id=?1 AND session_id=?2)",
            params![
                first.correlation.workspace_id.as_str(),
                first.correlation.conversation_id.as_str()
            ],
            |row| row.get(0),
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if has_gap {
        return Ok(FactsCompleteness::Partial);
    }
    Ok(completeness_from_facts(facts))
}

pub(crate) fn session_facts_completeness(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    session_id: &SessionId,
) -> Result<FactsCompleteness, FactProjectionError> {
    let has_gap: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM observation_gaps
             WHERE channel='fact' AND workspace_id=?1 AND session_id=?2)",
            params![workspace_id.as_str(), session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if has_gap {
        return Ok(FactsCompleteness::Partial);
    }
    let facts =
        fact_log::load_session(connection, workspace_id, session_id).map_err(map_store_error)?;
    if facts.is_empty() {
        return Ok(FactsCompleteness::Unknown);
    }
    Ok(completeness_from_facts(&facts))
}

fn completeness_from_facts(facts: &[ExecutionFactEnvelopeV1]) -> FactsCompleteness {
    let mut result = FactsCompleteness::Complete;
    for envelope in facts {
        result = least_complete(result, envelope.facts_completeness());
        if let ExecutionFactV1::RequestFinished {
            facts_completeness, ..
        } = envelope.fact
        {
            result = least_complete(result, facts_completeness);
        }
    }
    result
}

fn immutable_receipt_insert(
    transaction: &Transaction<'_>,
    receipt: &RoutingReceiptV1,
) -> Result<(), FactProjectionError> {
    let body = serde_json::to_string(receipt).map_err(|_| FactProjectionError::Storage)?;
    let digest = receipt.digest().map_err(|_| FactProjectionError::Storage)?;
    let body = crate::store::sensitive::put(
        transaction,
        "receipt",
        digest.as_str(),
        receipt.workspace_id.as_str(),
        receipt.session_id.as_str(),
        receipt.request_id.as_str(),
        receipt.frozen_at_ms,
        &body,
    )
    .map_err(|_| FactProjectionError::Storage)?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO routing_receipts
             (workspace_id, receipt_id, session_id, turn_id, request_id, body_json, body_digest,
              frozen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                receipt.workspace_id.as_str(),
                receipt.receipt_id.as_str(),
                receipt.session_id.as_str(),
                receipt.turn_id.as_str(),
                receipt.request_id.as_str(),
                body,
                digest.as_str(),
                receipt.frozen_at_ms,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if inserted == 0 {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT body_digest FROM routing_receipts WHERE workspace_id=?1 AND receipt_id=?2",
                params![receipt.workspace_id.as_str(), receipt.receipt_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| FactProjectionError::Storage)?;
        if existing.as_deref() != Some(digest.as_str()) {
            return Err(FactProjectionError::ImmutableConflict);
        }
    }
    Ok(())
}

fn project_value(
    transaction: &Transaction<'_>,
    receipt: &RoutingReceiptV1,
) -> Result<(), FactProjectionError> {
    let snapshots = receipt
        .ordered_facts
        .iter()
        .filter_map(|event| match &event.fact {
            ExecutionFactV1::ValueSnapshot {
                traffic_kind,
                usage,
                value,
            } => Some((traffic_kind, usage, value)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some((traffic_kind, usage, frozen)) = snapshots.first() else {
        return Ok(());
    };
    if snapshots.len() != 1 {
        return Err(FactProjectionError::ImmutableConflict);
    }
    if **traffic_kind != TrafficKind::Normal {
        return Ok(());
    }
    let value = ValueLedgerEntryV1 {
        schema: VALUE_LEDGER_SCHEMA_V1.to_owned(),
        workspace_id: receipt.workspace_id.clone(),
        session_id: receipt.session_id.clone(),
        turn_id: receipt.turn_id.clone(),
        request_id: receipt.request_id.clone(),
        receipt_id: receipt.receipt_id.clone(),
        usage: (*usage).clone(),
        frozen: (*frozen).clone(),
        facts_completeness: receipt.facts_completeness,
        frozen_at_ms: receipt.frozen_at_ms,
    };
    value.validate().map_err(|_| FactProjectionError::Invalid)?;
    immutable_value_insert(transaction, &value)
}

fn immutable_value_insert(
    transaction: &Transaction<'_>,
    value: &ValueLedgerEntryV1,
) -> Result<(), FactProjectionError> {
    let body = serde_json::to_string(value).map_err(|_| FactProjectionError::Storage)?;
    let body_digest = CanonicalDigest::of(value).map_err(|_| FactProjectionError::Storage)?;
    let usage_numbers = [
        value.usage.input_tokens,
        value.usage.output_tokens,
        value.usage.cache_read_tokens,
        value.usage.cache_write_tokens,
        value.usage.reasoning_tokens,
    ];
    let cost_numbers = [
        value.frozen.baseline_api_equivalent_cost_micros,
        value.frozen.chosen_api_equivalent_cost_micros,
        value.frozen.actual_incremental_cost_micros,
    ];
    if usage_numbers.iter().any(|number| *number > i64::MAX as u64)
        || cost_numbers
            .iter()
            .flatten()
            .any(|number| *number > i64::MAX as u64)
    {
        return Err(FactProjectionError::Invalid);
    }
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO value_ledger_entries
             (workspace_id, request_id, session_id, receipt_id, body_json, body_digest,
              agent_plan_id, currency, billing_unit, price_version, price_override_revision,
              input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
              baseline_api_equivalent_cost_micros, chosen_api_equivalent_cost_micros,
              actual_incremental_cost_micros, routing_savings_micros,
              entitlement_savings_micros, estimated_total_savings_micros,
              facts_completeness, frozen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                value.workspace_id.as_str(),
                value.request_id.as_str(),
                value.session_id.as_str(),
                value.receipt_id.as_str(),
                body,
                body_digest.as_str(),
                value.frozen.agent_plan_id.as_str(),
                value.frozen.currency,
                value.frozen.billing_unit,
                value.frozen.price_version,
                value.frozen.price_override_revision,
                usage_numbers[0] as i64,
                usage_numbers[1] as i64,
                usage_numbers[2] as i64,
                usage_numbers[3] as i64,
                usage_numbers[4] as i64,
                cost_numbers[0].map(|number| number as i64),
                cost_numbers[1].map(|number| number as i64),
                cost_numbers[2].map(|number| number as i64),
                value.frozen.routing_savings_micros,
                value.frozen.entitlement_savings_micros,
                value.frozen.estimated_total_savings_micros,
                facts_completeness_str(value.facts_completeness),
                value.frozen_at_ms,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    if inserted == 0 {
        let existing: String = transaction
            .query_row(
                "SELECT body_digest FROM value_ledger_entries
                 WHERE workspace_id=?1 AND request_id=?2",
                params![value.workspace_id.as_str(), value.request_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| FactProjectionError::Storage)?;
        if existing != body_digest.as_str() {
            return Err(FactProjectionError::ImmutableConflict);
        }
    }
    Ok(())
}

fn touch_session(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), FactProjectionError> {
    transaction
        .execute(
            "UPDATE sessions SET updated_at_ms=MAX(updated_at_ms, ?3)
             WHERE workspace_id=?1 AND session_id=?2",
            params![
                envelope.correlation.workspace_id.as_str(),
                envelope.correlation.conversation_id.as_str(),
                envelope
                    .occurred_at_ms()
                    .map_err(|_| FactProjectionError::Invalid)?,
            ],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    Ok(())
}

const fn request_outcome(value: ExecutionRequestOutcomeV1) -> RequestOutcome {
    match value {
        ExecutionRequestOutcomeV1::Accepted => RequestOutcome::Accepted,
        ExecutionRequestOutcomeV1::Failed => RequestOutcome::Failed,
        ExecutionRequestOutcomeV1::Cancelled => RequestOutcome::Cancelled,
        ExecutionRequestOutcomeV1::PostcommitPartial => RequestOutcome::PostcommitPartial,
        ExecutionRequestOutcomeV1::PostcommitTransportFailed => {
            RequestOutcome::PostcommitTransportFailed
        }
    }
}

fn least_complete(left: FactsCompleteness, right: FactsCompleteness) -> FactsCompleteness {
    match (left, right) {
        (FactsCompleteness::Partial, _) | (_, FactsCompleteness::Partial) => {
            FactsCompleteness::Partial
        }
        (FactsCompleteness::Unknown, _) | (_, FactsCompleteness::Unknown) => {
            FactsCompleteness::Unknown
        }
        _ => FactsCompleteness::Complete,
    }
}

const fn facts_completeness_str(value: FactsCompleteness) -> &'static str {
    match value {
        FactsCompleteness::Complete => "complete",
        FactsCompleteness::Partial => "partial",
        FactsCompleteness::Unknown => "unknown",
    }
}

fn enum_json<T: serde::Serialize>(value: &T) -> Result<String, FactProjectionError> {
    serde_json::to_value(value)
        .map_err(|_| FactProjectionError::Storage)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or(FactProjectionError::Storage)
}

fn map_store_error(error: ObservationStoreError) -> FactProjectionError {
    match error {
        ObservationStoreError::ActivityUnavailable => FactProjectionError::Storage,
        ObservationStoreError::Corrupt => FactProjectionError::Corrupt,
        ObservationStoreError::ContentUnavailable | ObservationStoreError::Io => {
            FactProjectionError::Storage
        }
    }
}
