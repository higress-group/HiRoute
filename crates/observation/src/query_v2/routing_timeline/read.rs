use super::*;
use crate::valuation::{
    EQUAL_RATE_VALUATION_ALGORITHM, MetricCoverage, RawUsage, VALUATION_ALGORITHM,
    ValuationRecordV2, value_recorded_attempt,
};
use rusqlite::{Transaction, named_params};

struct RetainedValuation {
    record: ValuationRecordV2,
    inputs: Vec<(ExecutionPricingEvidenceV1, RawUsage)>,
    valid: bool,
}

pub(super) fn position(tx: &Transaction<'_>) -> Result<Position, ObservationV2Error> {
    Ok(Position {
        facts: tx.query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM execution_fact_events",
            [],
            |r| r.get(0),
        )?,
        values: tx.query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM valuation_records_v2",
            [],
            |r| r.get(0),
        )?,
        links: tx.query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM observation_link_events",
            [],
            |r| r.get(0),
        )?,
    })
}

pub(super) fn read(
    tx: &Transaction<'_>,
    reader: &ObservationReaderContext,
    session: &str,
    target: Position,
    now: i64,
    selected: Option<&std::collections::BTreeSet<String>>,
) -> Result<(BTreeMap<String, Request>, Vec<Fact>), ObservationV2Error> {
    let selected = selected
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| ObservationV2Error::Invalid)?;
    let runs = reader
        .allowed_runs()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| ObservationV2Error::Invalid)?;
    let retained = now
        .saturating_sub(crate::managed_text::RETENTION_MS)
        .saturating_add(1)
        .max(0);
    let mut requests = BTreeMap::new();
    let mut retained_valuations = BTreeMap::new();
    let mut statement = tx.prepare("SELECT r.request_id,r.started_at_ms,r.finished_at_ms,r.outcome,CASE WHEN length(CAST(l.body_json AS BLOB))<=1048576 THEN l.body_json END,l.conflicted,
        COALESCE((SELECT MAX(e.rowid) FROM observation_link_events e WHERE e.workspace_id=r.workspace_id AND e.request_id=r.request_id AND e.rowid<=:links),0),
        (SELECT CASE WHEN length(CAST(v.body_json AS BLOB))<=1048576 THEN v.body_json END FROM valuation_records_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.rowid<=:values ORDER BY v.revision DESC LIMIT 1),
        COALESCE((SELECT MAX(v.rowid) FROM valuation_records_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.rowid<=:values),0),
        r.traffic_kind, l.run_id,
        COALESCE(length(CAST(l.body_json AS BLOB)),0)>1048576 OR COALESCE((SELECT length(CAST(v.body_json AS BLOB)) FROM valuation_records_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.rowid<=:values ORDER BY v.revision DESC LIMIT 1),0)>1048576
        FROM logical_requests r JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
        LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
        WHERE r.workspace_id=:workspace AND r.session_id=:session AND r.started_at_ms>=:retained
          AND (:selected IS NULL OR r.request_id IN(SELECT value FROM json_each(:selected)))
          AND (s.tombstone_reason IS NULL OR EXISTS(SELECT 1 FROM observation_tombstones t WHERE t.workspace_id=s.workspace_id AND t.session_id=s.session_id AND t.delete_scope='content_only'))
          AND (:runs IS NULL OR (l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(:runs))))
          AND EXISTS(SELECT 1 FROM execution_fact_events e WHERE e.workspace_id=r.workspace_id AND e.request_id=r.request_id AND e.rowid<=:facts)
        ORDER BY r.started_at_ms,r.request_id LIMIT 10001")?;
    let mut bytes = 0usize;
    let rows = statement.query_map(named_params! { ":selected":selected, ":workspace":reader.workspace().as_str(), ":session":session, ":retained":retained, ":runs":runs, ":facts":target.facts, ":values":target.values, ":links":target.links }, |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<bool>>(5)?,r.get::<_,i64>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,i64>(8)?,r.get::<_,String>(9)?,r.get::<_,Option<String>>(10)?,r.get::<_,bool>(11)?)))?;
    for row in rows {
        let (
            id,
            start,
            finish,
            outcome,
            link,
            conflicted,
            link_position,
            valuation,
            value_position,
            traffic,
            recorded_run,
            oversized,
        ) = row?;
        if oversized {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        bytes += link.as_ref().map_or(0, String::len) + valuation.as_ref().map_or(0, String::len);
        if bytes > 16 * 1024 * 1024 || requests.len() >= 10_000 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        let request_bytes =
            link.as_ref().map_or(0, String::len) + valuation.as_ref().map_or(0, String::len);
        let link = link
            .and_then(|s| serde_json::from_str::<RunObservationLink>(&s).ok())
            .filter(|l| {
                conflicted == Some(false)
                    && l.workspace_id == *reader.workspace()
                    && l.request_id.as_str() == id
                    && recorded_run.as_deref() == Some(l.run_id.as_str())
                    && ObservationRoutingContextV1::recorded(
                        l.plan_id.clone(),
                        l.plan_revision.clone(),
                        None,
                    )
                    .valid()
            });
        let record = if traffic == "normal" {
            valuation
                .as_deref()
                .map(serde_json::from_str::<ValuationRecordV2>)
                .transpose()
                .map_err(|_| ObservationV2Error::Unavailable)?
        } else {
            None
        };
        let costs = if traffic == "normal" {
            costs(record.as_ref())?
        } else {
            vec![]
        };
        if traffic == "normal"
            && let Some(record) = record
            && record.terminal_observed
            && !record.facts_partial
            && record.unknown_attempt_count == 0
            && record.attempts.iter().any(|attempt| {
                attempt.coverage != MetricCoverage::Complete
                    && attempt.algorithm == VALUATION_ALGORITHM
            })
        {
            retained_valuations.insert(
                id.clone(),
                RetainedValuation {
                    record,
                    inputs: Vec::new(),
                    valid: true,
                },
            );
        }
        requests.insert(
            id.clone(),
            Request {
                view: RoutingTimelineRequestV1 {
                    request_id: id,
                    started_at_ms: start,
                    duration_ms: finish.and_then(|end| u64::try_from(end - start).ok()),
                    native_model: None,
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    cache_read_share: None,
                    costs,
                    outcome,
                },
                position: Position {
                    facts: 0,
                    values: value_position,
                    links: link_position,
                },
                first_fact: i64::MAX,
                native_turn: link.as_ref().and_then(|l| l.native_turn_id.clone()),
                run_id: link.map(|l| l.run_id.to_string()),
                decision: None,
                candidate: None,
                identities: Default::default(),
                identity_gap: false,
                request_finished: false,
                stage_finished: false,
                models: BTreeMap::new(),
                credentials: BTreeMap::new(),
                partial: conflicted == Some(true),
                body_bytes: request_bytes,
                request_bytes,
                fact_count: 0,
                usage_count: 0,
            },
        );
    }
    let mut statement = tx.prepare("SELECT e.rowid,e.request_id,e.event_id,CASE WHEN length(CAST(p.body_json AS BLOB))<=1048576 THEN json_extract(p.body_json,'$.fact') END,COALESCE(length(CAST(p.body_json AS BLOB)),0)>1048576,e.producer_epoch
        FROM execution_fact_events e LEFT JOIN observation_sensitive_payloads_v2 p ON p.id='fact:'||e.envelope_digest
        WHERE e.workspace_id=?1 AND e.session_id=?2 AND e.rowid<=?3 AND (?4 IS NULL OR e.request_id IN(SELECT value FROM json_each(?4))) ORDER BY e.rowid LIMIT 50001")?;
    let rows = statement.query_map(
        rusqlite::params![reader.workspace().as_str(), session, target.facts, selected],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
            ))
        },
    )?;
    let mut facts = Vec::new();
    for (index, row) in rows.enumerate() {
        let (position, request_id, event_id, value, oversized, producer_epoch) = row?;
        if index >= 50_000 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        let Some(request) = requests.get_mut(&request_id) else {
            continue;
        };
        request.fact_count += 1;
        if oversized {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        request.position.facts = position;
        request.first_fact = request.first_fact.min(position);
        let Some(value) = value else {
            request.partial = true;
            continue;
        };
        bytes += value.len();
        request.body_bytes += value.len();
        if bytes > 32 * 1024 * 1024 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        facts.push(Fact {
            position,
            event_id,
            producer_epoch,
            request_id,
            value: serde_json::from_str(&value).map_err(|_| ObservationV2Error::Unavailable)?,
        });
    }
    // Reuse the valuation owner's cumulative/delta normalization. Unknown usage
    // remains unknown rather than silently turning a partial sum into a total.
    let revaluation_ids = serde_json::to_string(&retained_valuations.keys().collect::<Vec<_>>())
        .map_err(|_| ObservationV2Error::Invalid)?;
    let mut statement = tx.prepare("SELECT i.request_id,CASE WHEN length(CAST(i.usage_json AS BLOB))<=65536 THEN i.usage_json END,COALESCE(length(CAST(i.usage_json AS BLOB)),0)>65536,CASE WHEN i.request_id IN(SELECT value FROM json_each(?4)) AND length(CAST(i.pricing_json AS BLOB))<=65536 THEN i.pricing_json END,v.input_digest,v.input_revision FROM valuation_attempt_inputs_v2 i JOIN logical_requests r ON r.workspace_id=i.workspace_id AND r.request_id=i.request_id LEFT JOIN valuation_requests_v2 v ON v.workspace_id=i.workspace_id AND v.request_id=i.request_id WHERE r.workspace_id=?1 AND r.session_id=?2 AND (?3 IS NULL OR r.request_id IN(SELECT value FROM json_each(?3))) ORDER BY i.request_id,i.ordinal LIMIT 50001")?;
    let rows = statement.query_map(
        rusqlite::params![
            reader.workspace().as_str(),
            session,
            selected,
            revaluation_ids
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<u64>>(5)?,
            ))
        },
    )?;
    let mut usage: BTreeMap<String, [Option<u64>; 3]> = BTreeMap::new();
    for (index, row) in rows.enumerate() {
        if index >= 50_000 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        let (id, body, oversized, pricing, input_digest, input_revision) = row?;
        if !requests.contains_key(&id) {
            continue;
        }
        if oversized {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        bytes += body.as_ref().map_or(0, String::len);
        if let Some(request) = requests.get_mut(&id) {
            request.body_bytes += body.as_ref().map_or(0, String::len);
            request.usage_count += 1;
        }
        if bytes > 32 * 1024 * 1024 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        let value = body.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        if let Some(retained) = retained_valuations.get_mut(&id) {
            let extra_bytes = pricing.as_ref().map_or(0, String::len);
            let within_budget = bytes
                .checked_add(extra_bytes)
                .filter(|total| *total <= 32 * 1024 * 1024);
            let inputs = within_budget
                .and(pricing.as_deref())
                .and_then(|json| serde_json::from_str::<ExecutionPricingEvidenceV1>(json).ok())
                .zip(
                    value
                        .as_ref()
                        .and_then(|json| serde_json::from_value::<RawUsage>(json.clone()).ok()),
                );
            if input_digest.as_deref() != Some(retained.record.input_digest.as_str())
                || input_revision != Some(retained.record.input_revision)
                || inputs.is_none()
            {
                retained.valid = false;
            } else if let Some(inputs) = inputs {
                bytes = within_budget.expect("inputs require bounded pricing");
                retained.inputs.push(inputs);
            }
        }
        let sums = usage.entry(id).or_insert([Some(0); 3]);
        for (sum, key) in sums.iter_mut().zip(["input", "output", "read"]) {
            *sum = sum
                .zip(value.as_ref().and_then(|v| v.get(key)?.as_u64()))
                .and_then(|(a, b)| a.checked_add(b));
        }
    }
    for (id, [input, output, read]) in usage {
        let request = requests
            .get_mut(&id)
            .ok_or(ObservationV2Error::Unavailable)?;
        request.view.input_tokens = input;
        request.view.output_tokens = output;
        request.view.cache_read_tokens = read;
        request.view.cache_read_share = input
            .zip(read)
            .filter(|(i, r)| *i > 0 && r <= i)
            .map(|(i, r)| r as f64 / i as f64);
    }
    // Older settlement records remain immutable. Recompute an amount only for
    // an exact, fully retained input revision and the same frozen attempt quote.
    // This is a bounded read projection; it never fills in missing token facts.
    for (id, mut retained) in retained_valuations {
        if !retained.valid || retained.inputs.len() != retained.record.attempts.len() {
            continue;
        }
        let mut changed = false;
        for (attempt, (pricing, raw)) in retained.record.attempts.iter_mut().zip(&retained.inputs) {
            if pricing.quote.as_ref() != Some(&attempt.quote) {
                retained.valid = false;
                break;
            }
            let Ok(derived) = value_recorded_attempt(pricing, raw) else {
                retained.valid = false;
                break;
            };
            if derived.algorithm == EQUAL_RATE_VALUATION_ALGORITHM
                && attempt.algorithm == VALUATION_ALGORITHM
            {
                *attempt = derived;
                changed = true;
            }
        }
        if retained.valid
            && changed
            && let Some(request) = requests.get_mut(&id)
        {
            request.view.costs = costs(Some(&retained.record))?;
        }
    }
    projection::hydrate(&mut requests, &facts);
    Ok((requests, facts))
}

fn costs(
    record: Option<&ValuationRecordV2>,
) -> Result<Vec<RoutingTimelineCostV1>, ObservationV2Error> {
    let Some(record) = record else {
        return Ok(vec![unknown(1)]);
    };
    let mut result = Vec::new();
    for attempt in &record.attempts {
        result.push(RoutingTimelineCostV1 {
            model_configuration_id: Some(match &attempt.quote.exact_target.model_identity {
                PriceModelIdentityV1::CatalogModel(id) | PriceModelIdentityV1::LocalModel(id) => {
                    id.clone()
                }
            }),
            currency: Some(attempt.currency.clone()),
            valuation_kind: Some(
                serde_json::to_value(attempt.valuation_kind)
                    .map_err(|_| ObservationV2Error::Unavailable)?
                    .as_str()
                    .ok_or(ObservationV2Error::Unavailable)?
                    .into(),
            ),
            known_micros: attempt.known_sum_micros,
            unknown_contributions: u64::from(
                attempt.coverage != crate::valuation::MetricCoverage::Complete,
            ),
        });
    }
    if record.unknown_attempt_count > 0 {
        result.push(unknown(record.unknown_attempt_count));
    }
    if result.is_empty() || record.facts_partial || !record.terminal_observed {
        result.push(unknown(1));
    }
    Ok(result)
}
fn unknown(count: u64) -> RoutingTimelineCostV1 {
    RoutingTimelineCostV1 {
        model_configuration_id: None,
        currency: None,
        valuation_kind: None,
        known_micros: None,
        unknown_contributions: count,
    }
}

/// Scan only the newly appended interval. Advancing the authenticated cursor
/// across an empty interval prevents repeated work on other sessions' traffic.
pub(super) fn changed_requests(
    tx: &Transaction<'_>,
    reader: &ObservationReaderContext,
    session: &str,
    from: Position,
    through: Position,
) -> Result<std::collections::BTreeSet<String>, ObservationV2Error> {
    let mut result = std::collections::BTreeSet::new();
    for (table, low, high) in [
        ("execution_fact_events", from.facts, through.facts),
        ("valuation_records_v2", from.values, through.values),
        ("observation_link_events", from.links, through.links),
    ] {
        if low == high {
            continue;
        }
        let sql = format!(
            "SELECT DISTINCT e.request_id FROM {table} e JOIN logical_requests r ON r.workspace_id=e.workspace_id AND r.request_id=e.request_id WHERE e.rowid>?1 AND e.rowid<=?2 AND r.workspace_id=?3 AND r.session_id=?4 LIMIT 10001"
        );
        let mut statement = tx.prepare(&sql)?;
        for row in statement.query_map(
            rusqlite::params![low, high, reader.workspace().as_str(), session],
            |r| r.get::<_, String>(0),
        )? {
            result.insert(row?);
            if result.len() > 10_000 {
                return Err(ObservationV2Error::ScopeTooLarge);
            }
        }
    }
    Ok(result)
}

/// Match the cold reader's row bounds even when some rows are outside a
/// run-scoped reader. These indexed counts never deserialize session payloads.
pub(super) fn check_row_bounds(
    tx: &Transaction<'_>,
    reader: &ObservationReaderContext,
    session: &str,
    target: Position,
) -> Result<(), ObservationV2Error> {
    let facts: usize = tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM execution_fact_events WHERE workspace_id=?1 AND session_id=?2 AND rowid<=?3 LIMIT 50001)", rusqlite::params![reader.workspace().as_str(),session,target.facts], |r| r.get(0))?;
    let usage: usize = tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM valuation_attempt_inputs_v2 i JOIN logical_requests r ON r.workspace_id=i.workspace_id AND r.request_id=i.request_id WHERE r.workspace_id=?1 AND r.session_id=?2 LIMIT 50001)",rusqlite::params![reader.workspace().as_str(),session],|r|r.get(0))?;
    if facts > 50_000 || usage > 50_000 {
        return Err(ObservationV2Error::ScopeTooLarge);
    }
    Ok(())
}
