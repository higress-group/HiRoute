use super::*;
use hiroute_domain::*;
use rusqlite::{Connection, OpenFlags, Transaction, params};
impl crate::LocalObservationStore {
    pub fn observed_value_totals(
        &self,
        reader: &ObservationReaderContext,
        q: &ObservationValueQueryV2,
        now_ms: i64,
    ) -> Result<ObservationValueSummaryV2, ObservationV2Error> {
        Ok(self
            .observed_value_report(reader, q, ValueGroupByV1::None, now_ms)?
            .summary)
    }

    /// All totals and optional UTC day groups share one transaction, deadline,
    /// authorization scope and visibility generation. Never walk request pages.
    pub fn observed_value_report(
        &self,
        reader: &ObservationReaderContext,
        q: &ObservationValueQueryV2,
        group_by: ValueGroupByV1,
        now_ms: i64,
    ) -> Result<ObservationValueReportV2, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        if q.from_ms < 0
            || q.from_ms > q.to_ms
            || [&q.session_id, &q.plan_id, &q.currency]
                .into_iter()
                .flatten()
                .any(|s| !identifier(s))
        {
            return Err(ObservationV2Error::Invalid);
        }
        const DAY: i64 = 86_400_000;
        let grouped = group_by == ValueGroupByV1::Day;
        if grouped && q.to_ms.saturating_sub(1) / DAY - q.from_ms / DAY >= 200 {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut db =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&db, reader)?;
        let tx = db.transaction()?;
        let visibility = tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |r| r.get(0))?;
        let summary = read_totals(&tx, reader, q, now_ms)?;
        let mut days = Vec::new();
        if grouped {
            let mut from_ms = q.from_ms;
            while from_ms < q.to_ms {
                let to_ms = (from_ms / DAY + 1).saturating_mul(DAY).min(q.to_ms);
                days.push(read_totals(
                    &tx,
                    reader,
                    &ObservationValueQueryV2 {
                        from_ms,
                        to_ms,
                        ..q.clone()
                    },
                    now_ms,
                )?);
                from_ms = to_ms;
            }
        }
        tx.commit()?;
        check_visibility(&db, visibility)?;
        reader.check(now_ms, false, false)?;
        Ok(ObservationValueReportV2 {
            summary,
            group_by,
            day_timezone: grouped.then(|| "UTC".into()),
            days,
        })
    }
}

fn read_totals(
    tx: &Transaction<'_>,
    reader: &ObservationReaderContext,
    q: &ObservationValueQueryV2,
    now_ms: i64,
) -> Result<ObservationValueSummaryV2, ObservationV2Error> {
    let runs = reader
        .allowed_runs()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| ObservationV2Error::Invalid)?;
    // All statements use the same scope and deadline. Archives are never
    // exposed for a run/session-scoped request because they have no reverse IDs.
    let scoped = reader.allowed_runs().is_some() || q.session_id.is_some();
    let retained_from = if scoped {
        q.from_ms.max(
            now_ms
                .saturating_sub(crate::managed_text::RETENTION_MS)
                .saturating_add(1),
        )
    } else {
        q.from_ms
    };
    let retention_boundary_partial = scoped && retained_from > q.from_ms;
    let scope = "WITH selected AS (SELECT r.* FROM logical_requests r WHERE r.workspace_id=?1 AND r.started_at_ms>=?2 AND r.started_at_ms<?3 AND (?4 IS NULL OR r.session_id=?4) AND (?5 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?5)) AND (?7 IS NULL OR EXISTS(SELECT 1 FROM observation_run_links l WHERE l.workspace_id=r.workspace_id AND l.request_id=r.request_id AND l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(?7)))))";
    let args = params![
        reader.workspace().as_str(),
        retained_from,
        q.to_ms,
        q.session_id,
        q.plan_id,
        q.currency,
        runs
    ];
    let (pending,provisional,unknown,excluded,normal_pending,normal_provisional):(u64,u64,u64,u64,u64,u64)=tx.query_row(&format!("{scope} SELECT COALESCE(SUM(EXISTS(SELECT 1 FROM valuation_pending_v2 p WHERE p.workspace_id=r.workspace_id AND p.request_id=r.request_id)),0),COALESCE(SUM(r.outcome IS NULL),0),COALESCE(SUM(r.traffic_kind='unknown'),0),COALESCE(SUM(r.traffic_kind='connectivity_probe'),0),COALESCE(SUM(r.traffic_kind='normal' AND EXISTS(SELECT 1 FROM valuation_pending_v2 p WHERE p.workspace_id=r.workspace_id AND p.request_id=r.request_id)),0),COALESCE(SUM(r.traffic_kind='normal' AND r.outcome IS NULL),0) FROM selected r WHERE (?6 IS NULL OR 1=1)"),args,|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let unvalued: u64 = tx.query_row(&format!("{scope} SELECT COUNT(*) FROM selected r WHERE r.traffic_kind='normal' AND (?6 IS NULL OR 1=1) AND NOT EXISTS(SELECT 1 FROM valuation_contributions_v2 c WHERE c.workspace_id=r.workspace_id AND c.request_id=r.request_id) AND (EXISTS(SELECT 1 FROM valuation_attempt_inputs_v2 i WHERE i.workspace_id=r.workspace_id AND i.request_id=r.request_id AND i.pricing_json IS NOT NULL) OR NOT EXISTS(SELECT 1 FROM value_ledger_entries e WHERE e.workspace_id=r.workspace_id AND e.request_id=r.request_id AND e.chosen_api_equivalent_cost_micros IS NOT NULL))"), args, |r| r.get(0))?;
    let sql=format!("{scope}, contributions AS (
          SELECT c.currency,CASE WHEN json_valid(c.valuation_kind) THEN json_extract(c.valuation_kind,'$') ELSE c.valuation_kind END AS valuation_kind,c.known_micros AS amount,c.coverage!='\"complete\"' AS missing FROM valuation_contributions_v2 c JOIN selected r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id WHERE r.traffic_kind='normal' AND EXISTS(SELECT 1 FROM valuation_attempt_inputs_v2 i WHERE i.workspace_id=r.workspace_id AND i.request_id=r.request_id AND i.pricing_json IS NOT NULL)
          UNION ALL SELECT e.currency,'legacy_estimate',e.chosen_api_equivalent_cost_micros,e.chosen_api_equivalent_cost_micros IS NULL FROM value_ledger_entries e JOIN selected r ON r.workspace_id=e.workspace_id AND r.request_id=e.request_id WHERE r.traffic_kind='normal' AND NOT EXISTS(SELECT 1 FROM valuation_attempt_inputs_v2 i WHERE i.workspace_id=r.workspace_id AND i.request_id=r.request_id AND i.pricing_json IS NOT NULL)
          UNION ALL SELECT a.currency,CASE WHEN json_valid(a.valuation_kind) THEN json_extract(a.valuation_kind,'$') ELSE a.valuation_kind END,a.known_micros,a.partial_count FROM valuation_archives_v2 a WHERE a.valuation_kind NOT LIKE 'unclassified:%' AND a.workspace_id=?1 AND ?4 IS NULL AND ?7 IS NULL AND (?5 IS NULL OR a.plan_id=?5) AND a.day*86400000>=?2 AND (a.day+1)*86400000<=?3
          UNION ALL SELECT a.currency,'legacy_estimate',a.chosen_api_equivalent_cost_micros,a.chosen_api_equivalent_cost_micros IS NULL FROM daily_value_rollups a WHERE a.workspace_id=?1 AND ?4 IS NULL AND ?7 IS NULL AND (?5 IS NULL OR a.agent_plan_id=?5) AND a.day_number*86400000>=?2 AND (a.day_number+1)*86400000<=?3
        ) SELECT currency,valuation_kind,SUM(amount),SUM(missing),SUM(amount IS NULL) FROM contributions WHERE (?6 IS NULL OR currency=?6) GROUP BY currency,valuation_kind ORDER BY currency,valuation_kind LIMIT 201");
    let mut amounts = Vec::new();
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<u64>>(2)?,
            r.get::<_, u64>(3)?,
            r.get::<_, u64>(4)?,
        ))
    })?;
    for row in rows {
        let (currency, kind, sum, missing, _nulls) = row?;
        if amounts.len() == 200 {
            return Err(ObservationV2Error::Unavailable);
        }
        amounts.push(ObservationValueTotalV2 {
            currency,
            valuation_kind: kind,
            known_sum_micros: sum,
            coverage: coverage(
                sum,
                missing + pending + provisional + unknown + unvalued,
                retention_boundary_partial,
            ),
            missing_contribution_count: missing + unvalued,
        });
    }
    drop(stmt);
    const METRICS: [(&str, &str); 5] = [
        ("input", "input"),
        ("output", "output"),
        ("cache_read", "read"),
        ("cache_write", "write"),
        ("reasoning", "reasoning"),
    ];
    let mut usage_totals: [UsageAccumulator; 5] = Default::default();
    let sql = format!(
        "{scope} SELECT i.usage_json FROM valuation_attempt_inputs_v2 i JOIN selected r ON r.workspace_id=i.workspace_id AND r.request_id=i.request_id WHERE r.traffic_kind='normal'"
    );
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map(args, |row| row.get::<_, Option<String>>(0))?;
    for row in rows {
        let raw = row?;
        let parsed = raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        for (total, (_, field)) in usage_totals.iter_mut().zip(METRICS) {
            total.add(
                parsed
                    .as_ref()
                    .and_then(|value| value.get(field))
                    .and_then(|value| value.as_u64()),
            );
        }
    }
    drop(stmt);
    if !scoped {
        let mut stmt = tx.prepare("SELECT metric,known_sum,missing_count FROM observation_usage_archives_v2 WHERE workspace_id=?1 AND day*86400000>=?2 AND (day+1)*86400000<=?3 AND (?4 IS NULL OR plan_id=?4) AND metric IN ('input','output','cache_read','cache_write','reasoning')")?;
        let rows = stmt.query_map(
            params![reader.workspace().as_str(), q.from_ms, q.to_ms, q.plan_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?.map(|value| value as u64),
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        for row in rows {
            let (name, known, stored_missing) = row?;
            if let Some(index) = METRICS.iter().position(|(metric, _)| *metric == name) {
                if stored_missing < 0 {
                    // A negative count is the archived overflow marker;
                    // keep that metric unknown even if later days are known.
                    usage_totals[index].overflow = true;
                    usage_totals[index].add_missing((-i128::from(stored_missing) - 1) as u64);
                } else {
                    usage_totals[index].add_archive(known, stored_missing as u64);
                }
            }
        }
    }
    let usage = METRICS
        .into_iter()
        .zip(usage_totals)
        .map(|((name, _), total)| {
            let sum = if total.overflow { None } else { total.sum };
            ObservationUsageTotalV2 {
                metric: name.into(),
                known_sum: sum,
                coverage: coverage(
                    sum,
                    total.missing.saturating_add(unknown),
                    retention_boundary_partial,
                ),
                missing_attempt_count: total.missing,
            }
        })
        .collect();
    let input_cache_hit = cache_hit::aggregate(
        tx,
        reader,
        q,
        retained_from,
        normal_pending,
        normal_provisional,
        unknown,
    )?;
    let archive_boundary_partial: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM valuation_archives_v2 a WHERE a.workspace_id=?1 AND ?4 IS NULL AND ?7 IS NULL AND (?5 IS NULL OR a.plan_id=?5) AND (?6 IS NULL OR a.currency=?6) AND a.valuation_kind NOT LIKE 'unclassified:%' AND a.day*86400000<?3 AND (a.day+1)*86400000>?2 AND (a.day*86400000<?2 OR (a.day+1)*86400000>?3)) OR EXISTS(SELECT 1 FROM daily_value_rollups a WHERE a.workspace_id=?1 AND ?4 IS NULL AND ?7 IS NULL AND (?5 IS NULL OR a.agent_plan_id=?5) AND (?6 IS NULL OR a.currency=?6) AND a.day_number*86400000<?3 AND (a.day_number+1)*86400000>?2 AND (a.day_number*86400000<?2 OR (a.day_number+1)*86400000>?3)) OR EXISTS(SELECT 1 FROM observation_usage_archives_v2 a WHERE a.workspace_id=?1 AND ?4 IS NULL AND ?7 IS NULL AND (?5 IS NULL OR a.plan_id=?5) AND a.day*86400000<?3 AND (a.day+1)*86400000>?2 AND (a.day*86400000<?2 OR (a.day+1)*86400000>?3))",
            args, |r| r.get(0),
        )?;
    Ok(ObservationValueSummaryV2 {
        from_ms: q.from_ms,
        to_ms: q.to_ms,
        pending_requests: pending,
        provisional_requests: provisional,
        unknown_traffic_requests: unknown,
        excluded_requests: excluded,
        amounts,
        usage,
        input_cache_hit,
        archive_boundary_partial,
        retention_boundary_partial,
    })
}

#[derive(Default)]
struct UsageAccumulator {
    sum: Option<u64>,
    missing: u64,
    overflow: bool,
}

impl UsageAccumulator {
    fn add(&mut self, value: Option<u64>) {
        match value {
            Some(value) if !self.overflow => {
                self.sum = self.sum.unwrap_or(0).checked_add(value);
                self.overflow = self.sum.is_none();
            }
            Some(_) => {}
            None => self.add_missing(1),
        }
    }

    fn add_archive(&mut self, known: Option<u64>, missing: u64) {
        if known.is_none() {
            self.add_missing(missing.max(1));
        } else {
            self.add_missing(missing);
            self.add(known);
        }
    }

    fn add_missing(&mut self, missing: u64) {
        match self.missing.checked_add(missing) {
            Some(total) => self.missing = total,
            None => self.overflow = true,
        }
    }
}

fn coverage(sum: Option<u64>, missing: u64, boundary_partial: bool) -> ObservationMetricCoverageV2 {
    match (sum, missing, boundary_partial) {
        (None, _, _) => ObservationMetricCoverageV2::Unknown,
        (Some(_), 0, false) => ObservationMetricCoverageV2::Complete,
        _ => ObservationMetricCoverageV2::Partial,
    }
}
