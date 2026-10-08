use hiroute_domain::{AgentTurnAttributionV1, ExecutionFactEnvelopeV1, ExecutionFactV1};
use rusqlite::{OptionalExtension, Transaction, params};

use super::FactProjectionError;

pub(super) fn apply(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), FactProjectionError> {
    let changed = match &envelope.fact {
        ExecutionFactV1::AgentTurnFinished {
            branch_execution,
            agent_turn_id,
            segment_id,
            ordinal,
            plan_id,
            plan_revision,
            selected_branch_id,
            executed_branch_id,
            model_configuration_id,
            profile_digest,
            attribution,
            started_at_ms,
            finished_at_ms,
            history_partial,
            first_request_id,
            last_request_id,
            ..
        } => {
            type ExistingExecution = (
                String,
                String,
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i64>,
                Option<String>,
                Option<i64>,
            );
            let existing: Option<ExistingExecution> = transaction
                .query_row(
                    "SELECT session_id,plan_id,plan_revision,selected_branch_id,
                            executed_branch_id,model_configuration_id,profile_digest,attribution,
                            first_turn_id,first_turn_ordinal,last_observed_turn_id,
                            last_observed_turn_ordinal FROM plan_quality_segments
                     WHERE workspace_id=?1 AND segment_id=?2",
                    params![envelope.correlation.workspace_id.as_str(), segment_id,],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                            row.get(9)?,
                            row.get(10)?,
                            row.get(11)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| FactProjectionError::Storage)?;
            let revision = sql_u64(*plan_revision)?;
            let ordinal = sql_u64(*ordinal)?;
            let started_at_ms = sql_u64(*started_at_ms)?;
            let finished_at_ms = sql_u64(*finished_at_ms)?;
            let sequence = sql_u64(envelope.sequence)?;
            let attribution = attribution_name(*attribution);
            let profile_digest = profile_digest.as_ref().map(|value| value.as_str());
            if existing.as_ref().is_some_and(
                |(
                    session,
                    plan,
                    stored_revision,
                    selected,
                    executed,
                    model,
                    profile,
                    stored_attribution,
                    first_turn,
                    first_ordinal,
                    last_turn,
                    last_ordinal,
                )| {
                    session != envelope.correlation.conversation_id.as_str()
                        || plan != plan_id.as_str()
                        || *stored_revision != revision
                        || selected
                            .as_ref()
                            .is_some_and(|value| value != selected_branch_id)
                        || model
                            .as_deref()
                            .is_some_and(|value| Some(value) != model_configuration_id.as_deref())
                        || profile
                            .as_deref()
                            .is_some_and(|value| Some(value) != profile_digest)
                        || (selected.is_some()
                            && (executed.as_deref() != executed_branch_id.as_deref()
                                || stored_attribution.as_deref() != Some(attribution)))
                        || first_ordinal
                            .zip(first_turn.as_ref())
                            .is_some_and(|(stored, turn)| {
                                stored == ordinal && turn != agent_turn_id
                            })
                        || last_ordinal
                            .zip(last_turn.as_ref())
                            .is_some_and(|(stored, turn)| {
                                stored == ordinal && turn != agent_turn_id
                            })
                },
            ) {
                return Err(FactProjectionError::ImmutableConflict);
            }
            transaction
                .execute(
                    "INSERT INTO plan_quality_segments(
                        workspace_id,segment_id,session_id,plan_id,plan_revision,
                        selected_branch_id,executed_branch_id,model_configuration_id,
                        profile_digest,attribution,first_turn_id,first_turn_ordinal,
                        last_observed_turn_id,last_observed_turn_ordinal,first_at_ms,last_at_ms,
                        history_partial,first_request_id,last_request_id,execution_event_id,
                        execution_sequence)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?11,?12,?13,?14,
                            ?15,?16,?17,?18,?19)
                     ON CONFLICT(workspace_id,segment_id) DO UPDATE SET
                        selected_branch_id=COALESCE(selected_branch_id,excluded.selected_branch_id),
                        executed_branch_id=CASE WHEN selected_branch_id IS NULL
                          THEN excluded.executed_branch_id ELSE executed_branch_id END,
                        model_configuration_id=CASE WHEN selected_branch_id IS NULL
                          THEN excluded.model_configuration_id ELSE model_configuration_id END,
                        profile_digest=CASE WHEN selected_branch_id IS NULL
                          THEN excluded.profile_digest ELSE profile_digest END,
                        attribution=CASE WHEN selected_branch_id IS NULL
                          THEN excluded.attribution ELSE attribution END,
                        first_turn_id=CASE
                          WHEN first_turn_ordinal IS NULL OR excluded.first_turn_ordinal<first_turn_ordinal
                          THEN excluded.first_turn_id ELSE first_turn_id END,
                        first_turn_ordinal=CASE
                          WHEN first_turn_ordinal IS NULL THEN excluded.first_turn_ordinal
                          ELSE MIN(first_turn_ordinal,excluded.first_turn_ordinal) END,
                        first_at_ms=CASE WHEN first_at_ms IS NULL THEN excluded.first_at_ms
                          ELSE MIN(first_at_ms,excluded.first_at_ms) END,
                        last_observed_turn_id=CASE
                          WHEN last_observed_turn_ordinal IS NULL
                            OR excluded.last_observed_turn_ordinal>last_observed_turn_ordinal
                            OR (excluded.last_observed_turn_ordinal=last_observed_turn_ordinal
                                AND excluded.execution_sequence>COALESCE(execution_sequence,0))
                          THEN excluded.last_observed_turn_id ELSE last_observed_turn_id END,
                        last_observed_turn_ordinal=CASE
                          WHEN last_observed_turn_ordinal IS NULL
                          THEN excluded.last_observed_turn_ordinal
                          ELSE MAX(last_observed_turn_ordinal,excluded.last_observed_turn_ordinal) END,
                        last_at_ms=CASE WHEN last_at_ms IS NULL THEN excluded.last_at_ms
                          ELSE MAX(last_at_ms,excluded.last_at_ms) END,
                        history_partial=MAX(history_partial,excluded.history_partial),
                        first_request_id=CASE
                          WHEN first_turn_ordinal IS NULL
                            OR excluded.first_turn_ordinal<first_turn_ordinal
                          THEN excluded.first_request_id ELSE first_request_id END,
                        last_request_id=CASE
                          WHEN last_observed_turn_ordinal IS NULL
                            OR excluded.last_observed_turn_ordinal>last_observed_turn_ordinal
                            OR (excluded.last_observed_turn_ordinal=last_observed_turn_ordinal
                                AND excluded.execution_sequence>COALESCE(execution_sequence,0))
                          THEN excluded.last_request_id ELSE last_request_id END,
                        execution_event_id=CASE
                          WHEN last_observed_turn_ordinal IS NULL
                            OR excluded.last_observed_turn_ordinal>last_observed_turn_ordinal
                            OR (excluded.last_observed_turn_ordinal=last_observed_turn_ordinal
                                AND excluded.execution_sequence>execution_sequence)
                          THEN excluded.execution_event_id ELSE execution_event_id END,
                        execution_sequence=CASE
                          WHEN last_observed_turn_ordinal IS NULL
                            OR excluded.last_observed_turn_ordinal>last_observed_turn_ordinal
                            OR (excluded.last_observed_turn_ordinal=last_observed_turn_ordinal
                                AND excluded.execution_sequence>execution_sequence)
                          THEN excluded.execution_sequence ELSE execution_sequence END",
                    params![
                        envelope.correlation.workspace_id.as_str(),
                        segment_id,
                        envelope.correlation.conversation_id.as_str(),
                        plan_id.as_str(),
                        revision,
                        selected_branch_id,
                        executed_branch_id,
                        model_configuration_id,
                        profile_digest,
                        attribution,
                        agent_turn_id,
                        ordinal,
                        started_at_ms,
                        finished_at_ms,
                        *history_partial,
                        first_request_id.as_ref().map(|value| value.as_str()),
                        last_request_id.as_ref().map(|value| value.as_str()),
                        envelope.event_id.as_str(),
                        sequence,
                    ],
                )
                .map_err(|_| FactProjectionError::Storage)?;
            if let Some(metadata) = branch_execution {
                let encoded =
                    serde_json::to_string(metadata).map_err(|_| FactProjectionError::Invalid)?;
                let previous: Option<String> = transaction.query_row("SELECT branch_execution_json FROM plan_quality_segments WHERE workspace_id=?1 AND segment_id=?2", params![envelope.correlation.workspace_id.as_str(), segment_id], |row| row.get(0)).map_err(|_| FactProjectionError::Storage)?;
                if previous.as_ref().is_some_and(|old| old != &encoded) {
                    return Err(FactProjectionError::ImmutableConflict);
                }
                transaction.execute("UPDATE plan_quality_segments SET branch_execution_json=?3 WHERE workspace_id=?1 AND segment_id=?2", params![envelope.correlation.workspace_id.as_str(), segment_id, encoded]).map_err(|_| FactProjectionError::Storage)?;
            }
            true
        }
        ExecutionFactV1::BranchAssessmentRecorded {
            segment_id,
            plan_id,
            plan_revision,
            model_configuration_id,
            profile_digest,
            trigger_request_id,
            target_from_turn_id,
            target_through_turn_id,
            target_from_ordinal,
            target_through_ordinal,
            assessed_at_ms,
            score,
            partial,
            reason,
        } => {
            let revision = sql_u64(*plan_revision)?;
            type ExistingSegmentIdentity = (String, String, i64, Option<String>, Option<String>);
            let existing: Option<ExistingSegmentIdentity> = transaction
                .query_row(
                    "SELECT session_id,plan_id,plan_revision,model_configuration_id,profile_digest
                     FROM plan_quality_segments WHERE workspace_id=?1 AND segment_id=?2",
                    params![envelope.correlation.workspace_id.as_str(), segment_id,],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| FactProjectionError::Storage)?;
            if existing
                .as_ref()
                .is_some_and(|(session, plan, stored_revision, model, profile)| {
                    session != envelope.correlation.conversation_id.as_str()
                        || plan != plan_id.as_str()
                        || *stored_revision != revision
                        || model.as_deref() != Some(model_configuration_id)
                        || profile.as_deref() != Some(profile_digest.as_str())
                })
            {
                return Err(FactProjectionError::ImmutableConflict);
            }
            if existing.is_none() {
                transaction
                    .execute(
                        "INSERT INTO plan_quality_segments(
                            workspace_id,segment_id,session_id,plan_id,plan_revision,
                            model_configuration_id,profile_digest,history_partial)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,0)",
                        params![
                            envelope.correlation.workspace_id.as_str(),
                            segment_id,
                            envelope.correlation.conversation_id.as_str(),
                            plan_id.as_str(),
                            revision,
                            model_configuration_id,
                            profile_digest.as_str(),
                        ],
                    )
                    .map_err(|_| FactProjectionError::Storage)?;
            }
            let changed = transaction
                .execute(
                    "UPDATE plan_quality_segments SET
                        assessment_event_id=?3,assessment_sequence=?4,
                        assessment_trigger_request_id=?5,assessed_at_ms=?6,
                        target_from_turn_id=?7,target_through_turn_id=?8,
                        target_from_ordinal=?9,target_through_ordinal=?10,
                        score=?11,assessment_partial=?12,reason_present=?13
                     WHERE workspace_id=?1 AND segment_id=?2 AND (
                        target_through_ordinal IS NULL OR target_through_ordinal<?10 OR
                        (target_through_ordinal=?10 AND (
                          assessed_at_ms<?6 OR
                          (assessed_at_ms=?6 AND COALESCE(assessment_sequence,0)<?4))))",
                    params![
                        envelope.correlation.workspace_id.as_str(),
                        segment_id,
                        envelope.event_id.as_str(),
                        sql_u64(envelope.sequence)?,
                        trigger_request_id.as_str(),
                        sql_u64(*assessed_at_ms)?,
                        target_from_turn_id.as_str(),
                        target_through_turn_id.as_str(),
                        sql_u64(*target_from_ordinal)?,
                        sql_u64(*target_through_ordinal)?,
                        score,
                        *partial,
                        reason.is_some(),
                    ],
                )
                .map_err(|_| FactProjectionError::Storage)?;
            changed > 0
        }
        ExecutionFactV1::RouteDecision(fact) => {
            if let (
                Some(plan_id),
                hiroute_domain::ModelRequestRouteV2::Plan { revision, .. },
                Some(trigger),
            ) = (
                &fact.plan_id,
                &fact.route,
                fact.complexity
                    .as_ref()
                    .and_then(|c| c.competence_trigger.as_ref()),
            ) {
                let upgrade = hiroute_domain::PlanQualityUpgrade {
                    decision: trigger.clone(),
                    trigger_request_id: envelope.correlation.request_id.to_string(),
                };
                let encoded =
                    serde_json::to_string(&upgrade).map_err(|_| FactProjectionError::Invalid)?;
                transaction.execute("INSERT INTO plan_quality_segments(workspace_id,segment_id,session_id,plan_id,plan_revision,upgrade_json)
                    VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,segment_id) DO UPDATE SET upgrade_json=excluded.upgrade_json
                    WHERE plan_quality_segments.upgrade_json IS NULL AND plan_quality_segments.session_id=excluded.session_id AND plan_quality_segments.plan_id=excluded.plan_id AND plan_quality_segments.plan_revision=excluded.plan_revision",
                    params![envelope.correlation.workspace_id.as_str(), trigger.segment_id, envelope.correlation.conversation_id.as_str(), plan_id.as_str(), revision, encoded])
                    .map_err(|_| FactProjectionError::Storage)? > 0
            } else {
                false
            }
        }
        _ => false,
    };
    if changed {
        transaction
            .execute(
                "UPDATE observation_meta SET value=CAST(value AS INTEGER)+1
                 WHERE key='plan_quality_generation'",
                [],
            )
            .map_err(|_| FactProjectionError::Storage)?;
    }
    Ok(())
}

fn sql_u64(value: u64) -> Result<i64, FactProjectionError> {
    value.try_into().map_err(|_| FactProjectionError::Invalid)
}

fn attribution_name(value: AgentTurnAttributionV1) -> &'static str {
    match value {
        AgentTurnAttributionV1::Single => "single",
        AgentTurnAttributionV1::Mixed => "mixed",
        AgentTurnAttributionV1::Unknown => "unknown",
    }
}
