//! Actual serving transitions are separate from recorded decision stages. A user
//! turn may contain several model HTTP requests; compare its final endpoint once.
use super::views::Row;
use crate::query_v2::ObservationV2Error as Error;
use hiroute_domain::*;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn transitions(rows: &[Row]) -> Vec<AnalysisTransitionV1> {
    let mut previous_http = BTreeMap::<&str, &Row>::new();
    let mut previous_turn = BTreeMap::<&str, &Row>::new();
    let mut first_by_turn = BTreeMap::new();
    for r in rows {
        if let Some(turn) = r.turn.as_deref()
            && r.turn_first_request.as_deref() == Some(r.request.request.request_id.as_str())
        {
            first_by_turn.insert(
                (
                    r.request.request.session_id.as_str(),
                    r.epoch.as_deref(),
                    turn,
                ),
                r.request.request.started_at_ms,
            );
        }
    }
    // Timestamp ties cannot choose an endpoint by lexical request ID. Native
    // first/last references disambiguate a whole known turn; otherwise a tied
    // physical-session endpoint remains unknown.
    let mut last_groups = BTreeMap::<&str, Vec<&Row>>::new();
    for r in rows {
        let group = last_groups
            .entry(r.request.request.session_id.as_str())
            .or_default();
        if group
            .first()
            .is_some_and(|p| p.request.request.started_at_ms < r.request.request.started_at_ms)
        {
            group.clear();
        }
        group.push(r);
    }
    let last_by_session: BTreeMap<_, _> = last_groups
        .into_iter()
        .filter_map(|(id, group)| {
            if group.len() == 1 {
                return Some((id, group[0]));
            }
            let first = group[0];
            if first.turn.is_none()
                || first.turn_partial
                || group.iter().any(|r| {
                    r.turn != first.turn
                        || r.epoch != first.epoch
                        || r.turn_partial
                        || r.turn_last_request != first.turn_last_request
                })
            {
                return None;
            }
            group
                .iter()
                .find(|r| {
                    r.turn_last_request.as_deref() == Some(r.request.request.request_id.as_str())
                })
                .copied()
                .map(|r| (id, r))
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for r in rows {
        let own = &r.request.request;
        let first_physical = seen.insert(own.session_id.as_str());
        let boundary = first_physical
            && r.predecessor
                .as_deref()
                .is_some_and(|id| last_by_session.get(id).is_none_or(|p| p.anchor == r.anchor));
        let final_turn = r.turn.as_deref().is_some_and(|turn| {
            r.turn_last_request.as_deref() == Some(own.request_id.as_str())
                && first_by_turn.contains_key(&(own.session_id.as_str(), r.epoch.as_deref(), turn))
        });
        let prior_http = previous_http.get(own.session_id.as_str()).copied();
        let prev = if boundary {
            r.predecessor
                .as_deref()
                .and_then(|id| last_by_session.get(id).copied())
        } else if final_turn {
            previous_turn
                .get(own.session_id.as_str())
                .copied()
                .or(prior_http.filter(|p| p.turn != r.turn))
        } else {
            prior_http
        };
        let known = r.terminal && r.request.model.native_model.is_some();
        let endpoint_start = if final_turn && !boundary {
            r.turn
                .as_deref()
                .and_then(|turn| {
                    first_by_turn.get(&(own.session_id.as_str(), r.epoch.as_deref(), turn))
                })
                .copied()
                .unwrap_or(own.started_at_ms)
        } else {
            own.started_at_ms
        };
        let endpoints = known
            && prev.is_some_and(|p| {
                p.terminal
                    && p.request
                        .finished_at_ms
                        .is_some_and(|at| at < endpoint_start)
                    && p.request.model.native_model.is_some()
            });
        let changed = endpoints
            .then(|| prev.unwrap().request.model.native_model != r.request.model.native_model);
        let reasoning = endpoints
            .then(|| {
                prev.and_then(|p| {
                    p.request
                        .model
                        .reasoning_profile_id
                        .as_ref()
                        .zip(r.request.model.reasoning_profile_id.as_ref())
                        .map(|(a, b)| a != b)
                })
            })
            .flatten();
        let adjacent = final_turn
            && prev.is_some_and(|p| {
                p.epoch.is_some()
                    && p.epoch == r.epoch
                    && p.ordinal
                        .zip(r.ordinal)
                        .is_some_and(|(a, b)| a.checked_add(1) == Some(b))
                    && p.turn != r.turn
                    && !p.turn_partial
                    && !r.turn_partial
                    && endpoints
            });
        let same_turn =
            prev.is_some_and(|p| p.turn.is_some() && p.turn == r.turn && p.epoch == r.epoch);
        let kind = if boundary {
            AnalysisTransitionKindV1::ContinuationBoundary
        } else if !r.terminal {
            AnalysisTransitionKindV1::Unknown
        } else if final_turn
            && prev.is_none()
            && r.ordinal == Some(1)
            && !r.turn_partial
            && r.gap.is_none()
        {
            AnalysisTransitionKindV1::InitialSelection
        } else if adjacent && changed == Some(true) {
            AnalysisTransitionKindV1::BetweenTurnChange
        } else if adjacent && reasoning == Some(true) {
            AnalysisTransitionKindV1::ReasoningChange
        } else if (adjacent || same_turn) && changed == Some(false) {
            AnalysisTransitionKindV1::Unchanged
        } else {
            AnalysisTransitionKindV1::Unknown
        };
        let completeness = if !r.terminal || prev.is_some_and(|p| !p.terminal) {
            "pending"
        } else if boundary || prev.is_some() && !adjacent && !same_turn {
            "turn_context_reset"
        } else if known && (endpoints || kind == AnalysisTransitionKindV1::InitialSelection) {
            "complete"
        } else {
            "unknown"
        };
        let transition = AnalysisTransitionV1 {
            kind,
            session_id: own.session_id.clone(),
            request_id: own.request_id.clone(),
            previous_session_id: prev.map(|p| p.request.request.session_id.clone()),
            previous_request_id: prev.map(|p| p.request.request.request_id.clone()),
            at_ms: own.started_at_ms,
            previous_model: prev.map(|p| p.request.model.clone()),
            model: r.request.model.clone(),
            model_changed: changed,
            reasoning_changed: reasoning,
            completeness: completeness.into(),
            receipt_id: own.receipt_id.clone(),
            reason: None,
        };
        result.push(transition.clone());
        for kind in [
            (own.within_request_fallback == Some(true))
                .then_some(AnalysisTransitionKindV1::WithinRequestFallback),
            r.credential_relay
                .then_some(AnalysisTransitionKindV1::CredentialRelay),
        ]
        .into_iter()
        .flatten()
        {
            let mut within = transition.clone();
            within.kind = kind;
            within.previous_session_id = None;
            within.previous_request_id = None;
            within.previous_model = None;
            within.model_changed = Some(kind == AnalysisTransitionKindV1::WithinRequestFallback);
            within.reasoning_changed = None;
            result.push(within);
        }
        previous_http.insert(own.session_id.as_str(), r);
        if final_turn {
            previous_turn.insert(own.session_id.as_str(), r);
        }
    }
    result
}

pub(super) fn reasons(
    db: &Connection,
    reader: &ObservationReaderContext,
    values: &mut [AnalysisTransitionV1],
) -> Result<(), Error> {
    let mut cache = BTreeMap::new();
    for t in values {
        if !cache.contains_key(&t.request_id) {
            cache.insert(
                t.request_id.clone(),
                crate::query_v2::routing_reason(
                    db,
                    reader.workspace().as_str(),
                    &t.session_id,
                    &t.request_id,
                )?,
            );
        }
        t.reason = cache.get(&t.request_id).cloned().flatten();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, turn: &str, ordinal: u64, model: &str, at: i64) -> Row {
        Row {
            request: AnalysisRequestV1 {
                request: ObservedRequestSummary {
                    session_id: "physical-a".into(),
                    request_id: id.into(),
                    started_at_ms: at,
                    outcome: Some("accepted".into()),
                    receipt_id: Some(format!("receipt-{id}")),
                    run_id: None,
                    parent_context_ref: None,
                    native_turn_id: Some(turn.into()),
                    relation_conflicted: false,
                    routing_context: ObservationRoutingContextV1::unavailable(),
                    native_auxiliary: None,
                    attempted_model_count: 1,
                    within_request_fallback: Some(false),
                    final_native_model: Some(model.into()),
                    between_turn_model_change: None,
                    previous_native_turn_id: None,
                    previous_turn_model: None,
                    turn_final_native_model: None,
                },
                model: AnalysisModelV1 {
                    native_model: Some(model.into()),
                    model_configuration_id: Some(model.into()),
                    reasoning_profile_id: Some("max".into()),
                },
                finished_at_ms: Some(at + 1),
                content: vec![],
                content_partial: false,
                content_state: "not_observed".into(),
            },
            anchor: "physical-a".into(),
            predecessor: None,
            gap: None,
            turn: Some(turn.into()),
            ordinal: Some(ordinal),
            epoch: Some("epoch-1".into()),
            turn_partial: false,
            turn_first_request: Some(id.into()),
            turn_last_request: Some(id.into()),
            terminal: true,
            credential_relay: false,
        }
    }
    #[test]
    fn stage_http_changes_do_not_replace_the_final_model_of_each_user_turn() {
        let mut rows = vec![
            row("a", "turn-a", 1, "cheap", 10),
            row("b", "turn-b", 2, "primary", 20),
            row("c", "turn-b", 2, "cheap", 30),
            row("d", "turn-c", 3, "cheap", 40),
            row("e", "turn-c", 3, "primary", 50),
        ];
        for (start, end) in [(1, 2), (3, 4)] {
            let first = rows[start].request.request.request_id.clone();
            let last = rows[end].request.request.request_id.clone();
            for r in &mut rows[start..=end] {
                r.turn_first_request = Some(first.clone());
                r.turn_last_request = Some(last.clone());
            }
        }
        let values = transitions(&rows);
        let changed: Vec<_> = values
            .iter()
            .filter(|v| v.kind == AnalysisTransitionKindV1::BetweenTurnChange)
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].request_id, "e");
        assert_eq!(changed[0].previous_request_id.as_deref(), Some("c"));
        assert_eq!(
            values.iter().find(|v| v.request_id == "c").unwrap().kind,
            AnalysisTransitionKindV1::Unchanged
        );
    }
    #[test]
    fn tied_request_timestamps_use_the_native_last_request_not_lexical_order() {
        let mut rows = vec![
            row("a", "one", 1, "primary", 10),
            row("b", "two", 2, "primary", 20),
            row("c", "two", 2, "cheap", 20),
            row("d", "three", 3, "cheap", 40),
        ];
        for r in &mut rows[1..=2] {
            r.turn_first_request = Some("c".into());
            r.turn_last_request = Some("b".into());
        }
        let values = transitions(&rows);
        let changed: Vec<_> = values
            .iter()
            .filter(|v| v.kind == AnalysisTransitionKindV1::BetweenTurnChange)
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].request_id, "d");
        assert_eq!(changed[0].previous_request_id.as_deref(), Some("b"));
    }
    #[test]
    fn restarted_turn_order_does_not_prove_adjacent_user_turns() {
        let a = row("a", "before", 1, "cheap", 10);
        let mut b = row("b", "after", 1, "primary", 20);
        b.epoch = Some("new-epoch".into());
        let values = transitions(&[a, b]);
        assert_eq!(values[1].kind, AnalysisTransitionKindV1::Unknown);
        assert_eq!(values[1].completeness, "turn_context_reset");
    }
    #[test]
    fn interleaved_continuation_endpoints_leave_both_changes_unknown() {
        let mut a = row("a", "before", 1, "cheap", 10);
        a.request.finished_at_ms = Some(40);
        let mut b = row("b", "after", 2, "primary", 20);
        b.request.request.session_id = "physical-b".into();
        b.predecessor = Some("physical-a".into());
        let values = transitions(&[a, b]);
        assert_eq!(
            values[1].kind,
            AnalysisTransitionKindV1::ContinuationBoundary
        );
        assert_eq!(values[1].model_changed, None);
        assert_eq!(values[1].reasoning_changed, None);
        assert_eq!(values[1].completeness, "turn_context_reset");
    }
    #[test]
    fn same_model_credential_relay_is_not_a_model_switch() {
        let mut a = row("a", "turn", 1, "cheap", 10);
        a.credential_relay = true;
        let values = transitions(&[a]);
        let relay = values
            .iter()
            .find(|v| v.kind == AnalysisTransitionKindV1::CredentialRelay)
            .unwrap();
        assert_eq!(relay.model_changed, Some(false));
    }
}
