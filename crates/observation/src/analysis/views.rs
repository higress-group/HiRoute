//! Thin ordered request metadata first; hydrate only the selected page through
//! the existing receipt/content and PlanQuality owners.
use super::{scope, scope::Scope};
use crate::{LocalObservationStore, query_v2::ObservationV2Error as Error};
use hiroute_domain::*;
use rusqlite::{Connection, params};

pub(crate) struct Row {
    pub request: AnalysisRequestV1,
    pub anchor: String,
    pub predecessor: Option<String>,
    pub gap: Option<String>,
    pub turn: Option<String>,
    pub ordinal: Option<u64>,
    pub epoch: Option<String>,
    pub turn_partial: bool,
    pub turn_first_request: Option<String>,
    pub turn_last_request: Option<String>,
    pub terminal: bool,
    pub credential_relay: bool,
}

pub(super) fn rows(
    db: &Connection,
    s: &Scope,
    q: &ObservationAnalysisQueryV1,
) -> Result<Vec<Row>, Error> {
    let routing = matches!(q.view, AnalysisViewV1::Routing | AnalysisViewV1::Switches);
    let from = if routing {
        s.expires_ms
            .saturating_sub(crate::managed_text::RETENTION_MS)
            .saturating_add(1)
            .max(0)
    } else {
        s.range.effective_from_ms
    };
    let request_filter = if routing {
        None
    } else {
        s.initial_request.as_deref()
    };
    let sql="SELECT r.session_id,r.request_id,r.started_at_ms,r.finished_at_ms,r.outcome,r.receipt_id,
      m.anchor_id,m.predecessor,m.gap,s.agent_id,
      COALESCE(a.native_model,(SELECT model_id FROM observation_attempt_models_v2 am WHERE am.workspace_id=r.workspace_id AND am.request_id=r.request_id AND am.ordinal=v.accepted_ordinal)),
      a.model_id,a.reasoning_profile_id,COALESCE(v.terminal,0),
      (SELECT COUNT(DISTINCT model_id) FROM observation_attempt_models_v2 am WHERE am.workspace_id=r.workspace_id AND am.request_id=r.request_id),
      t.agent_turn_id,t.ordinal,t.producer_epoch,COALESCE(t.partial,1),
      EXISTS(SELECT 1 FROM observation_analysis_models_v1 ma JOIN observation_analysis_models_v1 mb ON mb.workspace_id=ma.workspace_id AND mb.request_id=ma.request_id AND mb.ordinal=ma.ordinal+1 WHERE ma.workspace_id=r.workspace_id AND ma.request_id=r.request_id AND ma.native_model=mb.native_model AND ma.credential_digest!=mb.credential_digest),t.first_request_id,t.last_request_id
      FROM logical_requests r JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
      JOIN observation_analysis_members_v1 m ON m.scope_id=?1 AND m.session_id=r.session_id
      LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
      LEFT JOIN observation_analysis_requests_v1 a ON a.workspace_id=r.workspace_id AND a.request_id=r.request_id AND a.revision=(SELECT MAX(new.revision) FROM observation_analysis_requests_v1 new WHERE new.workspace_id=r.workspace_id AND new.request_id=r.request_id)
      LEFT JOIN observation_analysis_turns_v1 t ON t.workspace_id=r.workspace_id AND t.request_id=r.request_id
      WHERE r.workspace_id=?2 AND r.started_at_ms>=?3 AND r.started_at_ms<?4 AND r.rowid<=?5
       AND (?6 IS NULL OR v.plan_id=?6) AND (?7 IS NULL OR s.agent_id=?7) AND (?8 IS NULL OR r.request_id=?8)
       AND (?9 IS NULL OR m.anchor_id=?9 OR m.anchor_id=(SELECT anchor_id FROM observation_analysis_members_v1 WHERE scope_id=?1 AND session_id=?9))
      ORDER BY r.started_at_ms,r.request_id";
    let mut stmt = db.prepare(sql)?;
    Ok(stmt
        .query_map(
            params![
                s.id,
                s.workspace,
                from,
                s.range.effective_to_ms,
                s.source_watermark,
                s.plan,
                s.agent,
                request_filter,
                q.session_id
            ],
            |r| {
                let count: u32 = r.get(14)?;
                let model = AnalysisModelV1 {
                    native_model: r.get(10)?,
                    model_configuration_id: r.get(11)?,
                    reasoning_profile_id: r.get(12)?,
                };
                let safe: Option<String> = None;
                let context = safe
                    .and_then(|s| serde_json::from_str::<ObservationSafeFactV2>(&s).ok())
                    .and_then(|f| f.routing_context)
                    .filter(|c| c.valid())
                    .unwrap_or_else(ObservationRoutingContextV1::unavailable);
                Ok(Row {
                    request: AnalysisRequestV1 {
                        request: ObservedRequestSummary {
                            session_id: r.get(0)?,
                            request_id: r.get(1)?,
                            started_at_ms: r.get(2)?,
                            outcome: r.get(4)?,
                            receipt_id: r.get(5)?,
                            run_id: None,
                            parent_context_ref: None,
                            native_turn_id: r.get(15)?,
                            relation_conflicted: false,
                            routing_context: context,
                            native_auxiliary: None,
                            attempted_model_count: count,
                            within_request_fallback: (count > 0).then_some(count > 1),
                            final_native_model: model.native_model.clone(),
                            between_turn_model_change: None,
                            previous_native_turn_id: None,
                            previous_turn_model: None,
                            turn_final_native_model: None,
                        },
                        model,
                        finished_at_ms: r.get(3)?,
                        content: vec![],
                        content_partial: false,
                        content_state: "not_observed".into(),
                    },
                    anchor: r.get(6)?,
                    predecessor: r.get(7)?,
                    gap: r.get(8)?,
                    turn: r.get(15)?,
                    ordinal: r.get(16)?,
                    epoch: r.get(17)?,
                    turn_partial: r.get(18)?,
                    turn_first_request: r.get(20)?,
                    turn_last_request: r.get(21)?,
                    terminal: r.get(13)?,
                    credential_relay: r.get(19)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(super) fn conversation_groups(
    db: &Connection,
    s: &Scope,
    q: &ObservationAnalysisQueryV1,
) -> Result<Vec<AnalysisConversationV1>, Error> {
    let mut stmt=db.prepare("SELECT m.anchor_id,MIN(r.started_at_ms),MAX(r.started_at_ms),COUNT(*),COUNT(DISTINCT m.session_id),MAX(m.gap IS NOT NULL)
      FROM logical_requests r JOIN sessions se ON se.workspace_id=r.workspace_id AND se.session_id=r.session_id
      JOIN observation_analysis_members_v1 m ON m.scope_id=?1 AND m.session_id=r.session_id
      LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
      WHERE r.workspace_id=?2 AND r.started_at_ms>=?3 AND r.started_at_ms<?4 AND r.rowid<=?5
       AND (?6 IS NULL OR v.plan_id=?6) AND (?7 IS NULL OR se.agent_id=?7) AND (?8 IS NULL OR r.request_id=?8)
       AND (?9 IS NULL OR m.anchor_id=?9 OR m.anchor_id=(SELECT anchor_id FROM observation_analysis_members_v1 WHERE scope_id=?1 AND session_id=?9))
       AND (?10 IS NULL OR r.request_id=?10)
      GROUP BY m.anchor_id ORDER BY MAX(r.started_at_ms) DESC,m.anchor_id")?;
    Ok(stmt
        .query_map(
            params![
                s.id,
                s.workspace,
                s.range.effective_from_ms,
                s.range.effective_to_ms,
                s.source_watermark,
                s.plan,
                s.agent,
                s.initial_request,
                q.session_id,
                q.request_id
            ],
            |r| {
                Ok(AnalysisConversationV1 {
                    analysis_session_id: r.get(0)?,
                    first_at_ms: r.get(1)?,
                    last_at_ms: r.get(2)?,
                    request_count: r.get(3)?,
                    member_count: r.get(4)?,
                    lineage_partial: r.get(5)?,
                    members: vec![],
                    members_partial: false,
                })
            },
        )?
        .collect::<Result<_, _>>()?)
}

pub(super) fn group_members(
    db: &Connection,
    s: &Scope,
    q: &ObservationAnalysisQueryV1,
    groups: &mut [AnalysisConversationV1],
) -> Result<(), Error> {
    // A list is a bounded member preview. Its exact count and explicit partial
    // marker lead to the existing paged conversation/request view for detail.
    let mut remaining = 512usize;
    for g in groups {
        let limit = remaining.min(g.member_count as usize);
        let mut stmt=db.prepare("SELECT m.session_id,m.predecessor,m.gap FROM observation_analysis_members_v1 m WHERE m.scope_id=?1 AND m.anchor_id=?2
           AND EXISTS(SELECT 1 FROM logical_requests r LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
             JOIN sessions se ON se.workspace_id=r.workspace_id AND se.session_id=r.session_id
             WHERE r.workspace_id=?4 AND r.session_id=m.session_id AND r.started_at_ms>=?5 AND r.started_at_ms<?6 AND r.rowid<=?7
               AND (?8 IS NULL OR v.plan_id=?8) AND (?9 IS NULL OR se.agent_id=?9) AND (?10 IS NULL OR r.request_id=?10) AND (?11 IS NULL OR r.request_id=?11))
           ORDER BY m.session_id LIMIT ?3")?;
        g.members = stmt
            .query_map(
                params![
                    s.id,
                    g.analysis_session_id,
                    limit,
                    s.workspace,
                    s.range.effective_from_ms,
                    s.range.effective_to_ms,
                    s.source_watermark,
                    s.plan,
                    s.agent,
                    s.initial_request,
                    q.request_id
                ],
                |r| {
                    Ok(AnalysisMemberV1 {
                        session_id: r.get(0)?,
                        predecessor_session_id: r.get(1)?,
                        gap_before: r.get(2)?,
                    })
                },
            )?
            .collect::<Result<_, _>>()?;
        remaining -= g.members.len();
        g.members_partial = (g.members.len() as u64) < g.member_count;
    }
    Ok(())
}

pub(super) fn hydrate_context(
    db: &Connection,
    reader: &ObservationReaderContext,
    rows: &mut [AnalysisRequestV1],
) -> Result<(), Error> {
    let mut stmt=db.prepare("SELECT body_json FROM observation_safe_facts_v2 WHERE workspace_id=?1 AND request_id=?2 ORDER BY rowid LIMIT 1")?;
    use rusqlite::OptionalExtension;
    for r in rows {
        let body: Option<String> = stmt
            .query_row(
                params![reader.workspace().as_str(), r.request.request_id],
                |r| r.get(0),
            )
            .optional()?;
        r.request.routing_context = body
            .and_then(|s| serde_json::from_str::<ObservationSafeFactV2>(&s).ok())
            .and_then(|f| f.routing_context)
            .filter(|c| c.valid())
            .unwrap_or_else(ObservationRoutingContextV1::unavailable);
    }
    Ok(())
}

pub(super) fn content(
    store: &LocalObservationStore,
    db: &Connection,
    reader: &ObservationReaderContext,
    rows: &mut [AnalysisRequestV1],
    now: i64,
) -> Result<(), Error> {
    let mut remaining: usize = 128 * 1024;
    for r in rows {
        let mut stmt=db.prepare("SELECT content_id,message_instance_id,direction,content_kind,canonical_media_type,state FROM content_instances_v2 WHERE workspace_id=?1 AND request_id=?2 AND content_kind NOT IN('provider_state','reasoning_delta','reasoning_finished') ORDER BY rowid LIMIT 17")?;
        let parts = stmt
            .query_map(
                params![reader.workspace().as_str(), r.request.request_id],
                |p| {
                    Ok((
                        p.get::<_, String>(0)?,
                        p.get::<_, String>(1)?,
                        p.get::<_, String>(2)?,
                        p.get::<_, String>(3)?,
                        p.get::<_, String>(4)?,
                        p.get::<_, String>(5)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        r.content_partial = parts.len() > 16;
        let deleted:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 WHERE workspace_id=?1 AND request_id=?2)",params![reader.workspace().as_str(),r.request.request_id],|r|r.get(0))?;
        r.content_state = if deleted {
            "deleted"
        } else if parts.is_empty() {
            "not_observed"
        } else if r.content_partial {
            "partial"
        } else {
            "complete"
        }
        .into();
        if parts.is_empty() {
            r.content_partial = true;
        }
        for (id, message, direction, kind, media, state) in parts.into_iter().take(16) {
            let mut c = AnalysisContentV1 {
                content_id: id,
                message_occurrence_id: message,
                direction,
                kind,
                media_type: media,
                text: None,
                next_cursor: None,
                completeness: state,
            };
            if remaining >= 128 * 1024 && c.completeness == "complete" {
                let query = ObservationContentQueryV2 {
                    request_id: LogicalRequestId::parse(&r.request.request_id)
                        .map_err(|_| Error::Invalid)?,
                    content_id: c.content_id.clone(),
                    cursor: None,
                    anchor_offset: None,
                };
                match store.observed_content_page_admitted(reader, &query, now, true) {
                    Ok(page) => {
                        let text = page.chunks.into_iter().map(|p| p.text).collect::<String>();
                        remaining = remaining.saturating_sub(text.len());
                        let fits = serde_json::to_vec(&text)
                            .map_err(|_| Error::Unavailable)?
                            .len()
                            <= 256 * 1024;
                        if fits {
                            c.text = Some(text);
                        } else {
                            c.completeness = "reference_only".into();
                            r.content_partial = true;
                            r.content_state = "partial".into();
                        }
                        c.next_cursor = page.next_cursor;
                        if c.next_cursor.is_some() {
                            r.content_partial = true;
                            c.completeness = "partial".into();
                            r.content_state = "partial".into();
                        }
                    }
                    Err(Error::Unavailable) => {
                        c.completeness = "unavailable".into();
                        r.content_partial = true;
                    }
                    Err(e) => return Err(e),
                }
            } else {
                r.content_partial = true;
                if c.completeness == "complete" {
                    c.completeness = "reference_only".into();
                    r.content_state = "partial".into();
                }
            }
            r.content.push(c);
        }
    }
    Ok(())
}

pub(super) fn paginate<T: Clone>(
    store: &LocalObservationStore,
    s: &Scope,
    q: &ObservationAnalysisQueryV1,
    values: &[T],
    offset: u64,
) -> Result<(Vec<T>, Option<String>), Error> {
    let start = usize::try_from(offset)
        .map_err(|_| Error::Invalid)?
        .min(values.len());
    let end = (start + usize::from(q.limit)).min(values.len());
    Ok((
        values[start..end].to_vec(),
        if end < values.len() {
            Some(scope::next(store, s, q, end as u64)?)
        } else {
            None
        },
    ))
}
