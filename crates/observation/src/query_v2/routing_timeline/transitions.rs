//! Adapt recorded timeline endpoints to the retained analysis classifier. No second taxonomy.
use super::*;
use crate::analysis::{RoutingRow, routing_transitions};

type Identity = (u64, String);
pub(super) fn classify(
    session: &str,
    turn: &RoutingTimelineTurnV1,
    identity: Option<&Identity>,
    prior: Option<(&RoutingTimelineTurnV1, Option<&Identity>)>,
    requests: &BTreeMap<String, Request>,
) -> Vec<AnalysisTransitionV1> {
    let mut rows = Vec::new();
    if let Some((previous, identity)) = prior {
        append(session, previous, identity, requests, &mut rows);
    }
    append(session, turn, identity, requests, &mut rows);
    rows.sort_by(|a, b| {
        (
            a.request.request.started_at_ms,
            &a.request.request.request_id,
        )
            .cmp(&(
                b.request.request.started_at_ms,
                &b.request.request.request_id,
            ))
    });
    let ids: std::collections::BTreeSet<_> = turn
        .requests
        .iter()
        .map(|r| r.request_id.as_str())
        .collect();
    let mut values: Vec<_> = routing_transitions(&rows)
        .into_iter()
        .filter(|t| ids.contains(t.request_id.as_str()))
        .collect();
    coalesce_completed_comparison(turn, identity, &mut values);
    values
}

fn coalesce_completed_comparison(
    turn: &RoutingTimelineTurnV1,
    identity: Option<&Identity>,
    values: &mut Vec<AnalysisTransitionV1>,
) {
    if turn.finished_at_ms.is_none() || turn.evidence_partial || identity.is_none() {
        return;
    }
    let Some(last) = turn.requests.last() else {
        return;
    };
    let Some(final_comparison) = values
        .iter()
        .find(|t| {
            t.request_id == last.request_id
                && t.completeness == "complete"
                && t.previous_session_id.is_some()
                && t.previous_request_id.is_some()
                && matches!(
                    t.kind,
                    AnalysisTransitionKindV1::BetweenTurnChange
                        | AnalysisTransitionKindV1::ReasoningChange
                        | AnalysisTransitionKindV1::Unchanged
                )
        })
        .cloned()
    else {
        return;
    };
    // The shared classifier keeps per-request diagnostics. A nonfinal request
    // cannot yet prove the turn's final endpoint, so it may report context reset.
    // Only the same proven predecessor and identical model/reasoning endpoints
    // make that diagnostic redundant with this completed turn comparison.
    // Other intermediate models, gaps, pending requests and relays remain visible.
    values.retain(|t| {
        !(t.request_id != final_comparison.request_id
            && t.kind == AnalysisTransitionKindV1::Unknown
            && t.completeness == "turn_context_reset"
            && t.previous_session_id == final_comparison.previous_session_id
            && t.previous_request_id == final_comparison.previous_request_id
            && t.previous_model == final_comparison.previous_model
            && t.model == final_comparison.model
            && t.model_changed == final_comparison.model_changed
            && t.reasoning_changed == final_comparison.reasoning_changed)
    });
}
fn append(
    session: &str,
    turn: &RoutingTimelineTurnV1,
    identity: Option<&Identity>,
    requests: &BTreeMap<String, Request>,
    rows: &mut Vec<RoutingRow>,
) {
    for view in &turn.requests {
        let raw = &requests[&view.request_id];
        let models: Vec<_> = raw.models.values().collect();
        let credentials: Vec<_> = raw.credentials.values().collect();
        rows.push(RoutingRow {
            request: AnalysisRequestV1 {
                request: ObservedRequestSummary {
                    session_id: session.into(),
                    request_id: view.request_id.clone(),
                    started_at_ms: view.started_at_ms,
                    outcome: view.outcome.clone(),
                    receipt_id: None,
                    run_id: raw.run_id.clone(),
                    parent_context_ref: None,
                    native_turn_id: raw.native_turn.clone(),
                    relation_conflicted: raw.partial,
                    routing_context: ObservationRoutingContextV1::unavailable(),
                    native_auxiliary: None,
                    attempted_model_count: models
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len() as u32,
                    within_request_fallback: (!models.is_empty())
                        .then(|| models.windows(2).any(|p| p[0] != p[1])),
                    final_native_model: view.native_model.clone(),
                    between_turn_model_change: None,
                    previous_native_turn_id: None,
                    previous_turn_model: None,
                    turn_final_native_model: None,
                },
                model: AnalysisModelV1 {
                    native_model: view.native_model.clone(),
                    model_configuration_id: raw.candidate.as_ref().map(|c| c.0.clone()),
                    reasoning_profile_id: raw.candidate.as_ref().and_then(|c| c.1.clone()),
                },
                finished_at_ms: view
                    .duration_ms
                    .and_then(|n| view.started_at_ms.checked_add(i64::try_from(n).ok()?)),
                content: vec![],
                content_partial: false,
                content_state: "not_observed".into(),
            },
            anchor: session.into(),
            predecessor: None,
            gap: None,
            turn: turn.agent_turn_id.clone(),
            ordinal: identity.map(|i| i.0),
            epoch: identity.map(|i| i.1.clone()),
            turn_partial: turn.evidence_partial || identity.is_none(),
            turn_first_request: turn.requests.first().map(|r| r.request_id.clone()),
            turn_last_request: turn.requests.last().map(|r| r.request_id.clone()),
            terminal: view.duration_ms.is_some(),
            credential_relay: models
                .windows(2)
                .zip(credentials.windows(2))
                .any(|(m, c)| m[0] == m[1] && c[0] != c[1]),
        });
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
