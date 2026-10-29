use super::{cost, prices, routing, schema, scope, views};
use crate::{
    LocalObservationStore,
    query_v2::{ObservationV2Error as Error, QueryDeadline},
};
use hiroute_domain::*;
use rusqlite::{Connection, params};

impl LocalObservationStore {
    pub fn observed_analysis(
        &self,
        reader: &ObservationReaderContext,
        q: &ObservationAnalysisQueryV1,
        range: Option<&AnalysisTimeRangeV1>,
        capture: Option<&AnalysisPricesV1>,
        now: i64,
    ) -> Result<ObservationAnalysisPageV1, Error> {
        let deadline = reader.query_deadline().unwrap_or_else(|| {
            std::time::Instant::now() + hiroute_domain::ObservationReaderContext::QUERY_BUDGET
        });
        reader.check(now, q.view.content(), q.query.is_some())?;
        if reader.allowed_runs().is_some() {
            return Err(Error::Unauthorized);
        }
        if q.limit == 0
            || q.limit > 200
            || [
                &q.session_id,
                &q.request_id,
                &q.plan_id,
                &q.agent_id,
                &q.currency,
                &q.reference_binding_id,
            ]
            .into_iter()
            .flatten()
            .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
            || q.query.as_ref().is_some_and(|k| {
                k.trim().is_empty()
                    || k.chars().count() > 256
                    || q.view != AnalysisViewV1::Conversations
            })
        {
            return Err(Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut db = Connection::open(&self.activity_path)?;
        let _deadline = QueryDeadline::start_for(&db, reader)?;
        // Four existing read permits bound page-cache memory to 64 MiB. Keeping
        // narrow numeric/source indexes resident avoids repeated large-row reads
        // without extending the shared deadline or retaining query results.
        db.pragma_update(None, "cache_size", -16_384)?;
        if !schema::available(&db) {
            return Err(Error::Unavailable);
        }
        // Initial membership and price captures are writes. Reserve that writer
        // before reading their source snapshot so concurrent catch-up cannot
        // turn every pending-progress response into SQLITE_BUSY_SNAPSHOT.
        // Existing complete scopes keep concurrent read transactions.
        let behavior = if (q.analysis_scope.is_none() && q.cursor.is_none()) || capture.is_some() {
            rusqlite::TransactionBehavior::Immediate
        } else {
            rusqlite::TransactionBehavior::Deferred
        };
        // Captured traffic and maintenance already serialize through this
        // writer. Join that admission only when persisting a new scope/capture,
        // rather than repeatedly losing SQLite's shorter busy slice to them.
        // Waiting consumes the caller's original deadline. Existing scopes do
        // not take the writer, and external writers still use SQLite's bound.
        let writer = if matches!(behavior, rusqlite::TransactionBehavior::Immediate) {
            Some(
                self.connection
                    .try_lock_until(deadline)
                    .ok_or(Error::Timeout)?,
            )
        } else {
            None
        };
        let tx = db.transaction_with_behavior(behavior)?;
        let (mut s, offset) = if q.analysis_scope.is_some() || q.cursor.is_some() {
            scope::load(self, &tx, reader, q, now)?
        } else {
            (
                scope::create(&tx, reader, q, range.ok_or(Error::Invalid)?.clone(), now)?,
                0,
            )
        };
        let initial_scope = scope::digest(&s)?;
        let mut page = ObservationAnalysisPageV1 {
            analysis_scope: Some(scope::token(self, &s)?),
            time: Some(s.range.clone()),
            expires_at_ms: Some(s.expires_ms),
            relation_watermark: Some(s.r),
            numeric_watermark: s.e,
            price_capture_ref: s.p.clone(),
            basis: s.basis,
            ..Default::default()
        };
        if q.view.monetary() {
            if s.basis.is_none() {
                s.basis = Some(
                    q.basis
                        .unwrap_or(if q.view == AnalysisViewV1::SavingsSummary {
                            AnalysisCostBasisV1::ApiEquivalent
                        } else {
                            AnalysisCostBasisV1::Recorded
                        }),
                );
            }
            if s.currency.is_none() {
                s.currency = q.currency.clone();
            }
            if s.valuation_kind.is_none() {
                s.valuation_kind = q.valuation_kind;
            }
            if s.reference.is_none() {
                s.reference = q.reference_binding_id.clone();
            }
            page.basis = s.basis;
            if s.e.is_none() {
                if let Some(progress) = pending(&tx, &s, q, now)? {
                    page.status = AnalysisStatusV1::Pending;
                    page.progress = Some(progress);
                    scope::save(&tx, &s)?;
                    tx.commit()?;
                    return Ok(page);
                }
                s.e = Some(schema::get(&tx, "numeric_revision")?);
            }
            page.numeric_watermark = s.e;
            if s.basis == Some(AnalysisCostBasisV1::ApiEquivalent) && s.p.is_none() {
                let Some(capture) = capture else {
                    page.status = AnalysisStatusV1::ChoicesRequired;
                    page.price_capture_request = Some(AnalysisPriceCaptureRequestV1 {
                        currency: s.currency.clone(),
                        plan_id: s.plan.clone(),
                        reference_binding_id: s.reference.clone(),
                    });
                    scope::save(&tx, &s)?;
                    tx.commit()?;
                    return Ok(page);
                };
                prices::capture(&tx, &mut s, capture)?;
                if s.reference.is_none() {
                    s.reference = capture.reference_binding_id.clone();
                }
            }
            let p = prices::load(&tx, &s)?;
            let report = cost::evaluate(&tx, &s, q, p.as_ref(), reader, now)?;
            let (costs, next) = views::paginate(self, &s, q, &report.rows, offset)?;
            page.costs = costs;
            page.savings = report.savings;
            page.next_cursor = next;
            page.choices = report.choices;
            if !page.choices.is_empty() {
                page.status = AnalysisStatusV1::ChoicesRequired;
            }
            page.numeric_watermark = s.e;
            page.price_capture_ref = s.p.clone();
        } else {
            match q.view {
                AnalysisViewV1::Conversations => {
                    let values = views::conversation_groups(&tx, &s, q)?;
                    let (values, next) = views::paginate(self, &s, q, &values, offset)?;
                    page.conversations = values;
                    views::group_members(&tx, &s, q, &mut page.conversations)?;
                    page.next_cursor = next;
                }
                AnalysisViewV1::Conversation => {
                    let rows = views::rows(&tx, &s, q)?;
                    let values: Vec<_> = rows
                        .into_iter()
                        .filter(|r| {
                            q.request_id
                                .as_ref()
                                .is_none_or(|id| &r.request.request.request_id == id)
                        })
                        .map(|r| r.request)
                        .collect();
                    let (values, next) = views::paginate(self, &s, q, &values, offset)?;
                    page.requests = values;
                    views::hydrate_context(&tx, reader, &mut page.requests)?;
                    page.next_cursor = next;
                }
                AnalysisViewV1::Routing | AnalysisViewV1::Switches => {
                    let rows = views::rows(&tx, &s, q)?;
                    let transitions: Vec<_> = routing::transitions(&rows)
                        .into_iter()
                        .filter(|t| {
                            t.at_ms >= s.range.effective_from_ms
                                && t.at_ms < s.range.effective_to_ms
                        })
                        .filter(|t| {
                            s.initial_request
                                .as_ref()
                                .is_none_or(|id| &t.request_id == id)
                        })
                        .collect();
                    for t in &transitions {
                        let key = serde_json::to_value(t.kind)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .ok_or(Error::Unavailable)?;
                        *page.transition_counts.entry(key).or_default() += 1;
                    }
                    let values: Vec<_> = transitions
                        .into_iter()
                        .filter(|t| q.request_id.as_ref().is_none_or(|id| &t.request_id == id))
                        .filter(|t| {
                            q.view != AnalysisViewV1::Switches
                                || matches!(
                                    t.kind,
                                    AnalysisTransitionKindV1::BetweenTurnChange
                                        | AnalysisTransitionKindV1::WithinRequestFallback
                                        | AnalysisTransitionKindV1::CredentialRelay
                                        | AnalysisTransitionKindV1::ReasoningChange
                                        | AnalysisTransitionKindV1::ContinuationBoundary
                                )
                        })
                        .collect();
                    let (values, next) = views::paginate(self, &s, q, &values, offset)?;
                    page.transitions = values;
                    routing::reasons(&tx, reader, &mut page.transitions)?;
                    if page.transitions.iter().any(|t| t.reason.is_none()) {
                        page.gaps.push("recorded_reason_unavailable".into());
                        page.status = AnalysisStatusV1::Partial;
                    }
                    if page
                        .transitions
                        .iter()
                        .any(|t| t.completeness != "complete")
                    {
                        page.gaps.push("routing_evidence_partial".into());
                        page.status = AnalysisStatusV1::Partial;
                    }
                    page.next_cursor = next;
                }
                _ => return Err(Error::Invalid),
            }
        }
        if s.range.effective_from_ms == s.range.effective_to_ms {
            page.gaps.push("no_retained_stable_interval".into());
            page.status = AnalysisStatusV1::Partial;
        }
        if s.range.clipped_for_retention {
            page.gaps.push("retention_prefix_clipped".into());
            if page.status == AnalysisStatusV1::Ready {
                page.status = AnalysisStatusV1::Partial
            }
        }
        if s.index_partial {
            page.gaps.push("index_unavailable".into());
            if page.status == AnalysisStatusV1::Ready {
                page.status = AnalysisStatusV1::Partial
            }
        }
        if scope::digest(&s)? != initial_scope {
            scope::save(&tx, &s)?;
        }
        tx.commit()?;
        drop(writer);
        if q.view.content() {
            views::content(self, &db, reader, &mut page.requests, now)?;
        }
        // Leave envelope/cursor headroom in the existing 1 MiB Local Control
        // frame. Limit is an upper bound; continue at the actual returned item.
        let returned = match q.view {
            AnalysisViewV1::Conversation => Some(frame_prefix(&page.requests)?),
            AnalysisViewV1::Routing | AnalysisViewV1::Switches => {
                Some(frame_prefix(&page.transitions)?)
            }
            _ => None,
        };
        if let Some(returned) = returned {
            let previous = if q.view == AnalysisViewV1::Conversation {
                page.requests.len()
            } else {
                page.transitions.len()
            };
            if returned < previous {
                page.requests.truncate(returned);
                page.transitions.truncate(returned);
                page.next_cursor = Some(scope::next(self, &s, q, offset + returned as u64)?);
            }
        }
        if page.conversations.iter().any(|c| c.lineage_partial) {
            page.gaps.push("native_grouping_partial".into());
            if page.status == AnalysisStatusV1::Ready {
                page.status = AnalysisStatusV1::Partial;
            }
        }
        if (page.requests.iter().any(|r| r.content_partial)
            || page
                .costs
                .iter()
                .any(|r| r.coverage != ObservationMetricCoverageV2::Complete))
            && page.status == AnalysisStatusV1::Ready
        {
            page.status = AnalysisStatusV1::Partial;
        }
        // The final visibility check observes one current snapshot. Unchanged
        // core population revisions need no second full population hash, while
        // relation withdrawal is checked even without a core revision change.
        let visibility = db.unchecked_transaction()?;
        scope::check_live(&visibility, &s)?;
        visibility.commit()?;
        reader.check(now, q.view.content(), q.query.is_some())?;
        Ok(page)
    }
}

fn frame_prefix<T: serde::Serialize>(values: &[T]) -> Result<usize, Error> {
    let mut bytes = 32 * 1024;
    for (i, value) in values.iter().enumerate() {
        bytes += serde_json::to_vec(value)
            .map_err(|_| Error::Unavailable)?
            .len()
            + 1;
        if bytes > 768 * 1024 {
            return if i == 0 {
                Err(Error::Unavailable)
            } else {
                Ok(i)
            };
        }
    }
    Ok(values.len())
}

fn pending(
    db: &Connection,
    s: &scope::Scope,
    q: &ObservationAnalysisQueryV1,
    now: i64,
) -> Result<Option<AnalysisProgressV1>, Error> {
    // Missing rows, input changes and settlement-only writes all have dirty
    // notifications. Unrelated cleanup or a different member cannot block this
    // scope's already-covered inputs.
    let total:u64=db.query_row("SELECT COUNT(*) FROM logical_requests r JOIN observation_analysis_members_v1 m ON m.scope_id=?1 AND m.session_id=r.session_id
      JOIN sessions se ON se.workspace_id=r.workspace_id AND se.session_id=r.session_id
      LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
      JOIN observation_analysis_dirty_v1 d ON d.workspace_id=r.workspace_id AND d.request_id=r.request_id
      WHERE r.workspace_id=?2 AND r.started_at_ms>=?3 AND r.started_at_ms<?4 AND r.rowid<=?5 AND (?6 IS NULL OR v.plan_id=?6) AND (?7 IS NULL OR se.agent_id=?7)
       AND (?8 IS NULL OR r.request_id=?8) AND (?9 IS NULL OR r.request_id=?9)",params![s.id,s.workspace,s.range.effective_from_ms,s.range.effective_to_ms,s.source_watermark,s.plan,s.agent,s.initial_request,q.request_id],|r|r.get(0))?;
    if total == 0 {
        return Ok(None);
    }
    let last = schema::get(db, "job_last_progress_ms")? as i64;
    let stalled = last > 0 && now.saturating_sub(last) >= 10_000;
    Ok(Some(AnalysisProgressV1 {
        status: if stalled { "stalled" } else { "pending" }.into(),
        phase: "numeric_reconciliation".into(),
        target_ref: format!("analysis-target/{}", schema::get(db, "job_target")?),
        processed_requests: s.source_count.saturating_sub(total),
        total_requests: s.source_count,
        started_at_ms: schema::get(db, "job_started_ms")? as i64,
        last_progress_at_ms: last,
        reason: stalled.then(|| "reconciliation_stalled".into()),
        retry_after_ms: 500,
    }))
}
