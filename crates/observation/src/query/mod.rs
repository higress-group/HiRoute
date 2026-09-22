//! Read-only implementation of the frozen ObservationQuery port.

mod content;
mod facts;
mod filters;
mod search;
mod status;
use status::{aggregate_content_completeness, aggregate_facts_completeness, read_gaps};
mod value;

use hiroute_domain::{
    AttemptId, ContentAccess, ContentBlobDigest, ContentCompleteness, ContentId, ContentMode,
    ContentRefV2, ConversationContentDirectionV2, CorrelationProvenance, FactsCompleteness,
    MessageInstanceId, ObservationChannel, ObservationCompletenessScope, ObservationGapV1,
    ObservationQueryError, ObservationQueryPort, ObservationStatusV1, ObservationStreamV1,
    ReceiptId, RoutingReceiptV1, SessionDetailV1, SessionId, SessionListQueryV1, SessionListV1,
    SessionSummaryV1, StoredMessageV1, TombstoneReason, TranscriptRoot, TurnDetailV1, TurnId,
    ValueLedgerEntryV1, ValueQueryV1, ValueViewV1, WorkspaceId,
};
use rusqlite::{OptionalExtension, params};

use crate::store::{LocalObservationStore, store_revision};
use crate::value::aggregate_entries;

use filters::{matches_switch, validate_list_query};

impl ObservationQueryPort for LocalObservationStore {
    fn observed_maintenance_status(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationMaintenanceStatusV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        if reader.allowed_runs().is_some() {
            return Err(ObservationQueryError::Unauthorized);
        }
        let (running, error_count) = self.maintenance_status();
        Ok(hiroute_domain::ObservationMaintenanceStatusV2 {
            running,
            error_count,
            index_running: self
                .index_running
                .load(std::sync::atomic::Ordering::Acquire),
        })
    }

    fn preview_session_deletion_v2(
        &self,
        principal: &hiroute_domain::ObservationPrincipalV1,
        spec: &hiroute_domain::SessionDeletionSpecV1,
        through_ms: i64,
    ) -> Result<hiroute_domain::SessionDeletionPreviewV2, ObservationQueryError> {
        self.preview_session_deletion_v2(principal, spec, through_ms)
    }
    fn apply_session_deletion_v2(
        &self,
        principal: &hiroute_domain::ObservationPrincipalV1,
        preview: &hiroute_domain::SessionDeletionPreviewV2,
        accepted: &hiroute_domain::CanonicalDigest,
        now_ms: i64,
    ) -> Result<hiroute_domain::SessionDeletionOutcomeV2, ObservationQueryError> {
        self.apply_session_deletion_v2(principal, preview, accepted, now_ms)
    }

    fn observed_value_totals(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationValueQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationValueSummaryV2, ObservationQueryError> {
        self.observed_value_totals(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_ancestry(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationAncestryQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationAncestryV2, ObservationQueryError> {
        self.observed_ancestry(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_catalog(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationCatalogQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationCatalogPageV2, ObservationQueryError> {
        self.observed_catalog(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_timeline(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationRequestPage, ObservationQueryError> {
        self.observed_timeline(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_plan_quality_samples(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::PlanQualitySamplesQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::PlanQualitySamplesPage, ObservationQueryError> {
        self.observed_plan_quality_samples(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_sessions(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationSessionPageV2, ObservationQueryError> {
        self.observed_sessions(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_facts(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationFactsQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationFactsPageV2, ObservationQueryError> {
        self.observed_facts(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_content_page(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationContentQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationContentPageV2, ObservationQueryError> {
        self.observed_content_page(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn search_observed_text(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationSearchQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationSearchPageV2, ObservationQueryError> {
        self.search_observed_text(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn observed_valuation(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        request: &hiroute_domain::LogicalRequestId,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationValuationSummaryV2, ObservationQueryError> {
        self.observed_valuation_summary(reader, request, now_ms)
    }

    fn list_observed_requests(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationRequestPage, ObservationQueryError> {
        self.observed_requests(reader, query, now_ms)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)
    }

    fn list_sessions(
        &self,
        workspace_id: &WorkspaceId,
        query: &SessionListQueryV1,
    ) -> Result<SessionListV1, ObservationQueryError> {
        validate_list_query(query)?;
        let _permit = self
            .query_permit()
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let connection = rusqlite::Connection::open_with_flags(
            &self.activity_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
        let _deadline = crate::query_v2::QueryDeadline::start(&connection)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let visibility = query_visibility(&connection)?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, agent_id, correlation, started_at_ms, updated_at_ms,
                        facts_completeness, content_completeness, tombstone_reason
                 FROM sessions WHERE workspace_id=?1
                   AND (?2 IS NULL OR updated_at_ms>=?2) AND (?3 IS NULL OR updated_at_ms<=?3)
                   AND (?4 IS NULL OR agent_id=?4)
                 ORDER BY updated_at_ms DESC, session_id LIMIT 201",
            )
            .map_err(|_| ObservationQueryError::Unavailable)?;
        let rows = statement
            .query_map(
                params![
                    workspace_id.as_str(),
                    query.from_ms,
                    query.to_ms,
                    query.agent_id
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .map_err(|_| ObservationQueryError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationQueryError::Corrupt)?;
        // Reject the bounded 201st raw candidate before expensive per-session joins.
        // Otherwise a query deadline can interrupt the first 200 summaries and mask
        // the deterministic InvalidQuery result under concurrent read load.
        if rows.len() > 200 {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let mut sessions = Vec::new();
        let mut search_budget = (0usize, 0usize);
        for (id, agent, correlation, started, updated, facts, content, tombstone) in rows {
            if query.from_ms.is_some_and(|from| updated < from)
                || query.to_ms.is_some_and(|to| updated > to)
                || query.agent_id.as_ref().is_some_and(|value| value != &agent)
                || (!query.include_unlinked && correlation == "unproven")
            {
                continue;
            }
            let session_id = SessionId::parse(id).map_err(|_| ObservationQueryError::Corrupt)?;
            let model_switch = facts::session_model_switch(&connection, workspace_id, &session_id)?;
            if !matches_switch(query.model_switch, model_switch) {
                continue;
            }
            if let Some(text) = query.query.as_deref()
                && !search::session_contains(
                    &connection,
                    &self.authority,
                    workspace_id,
                    &session_id,
                    text,
                    &mut search_budget,
                )?
            {
                continue;
            }
            sessions.push(session_summary(
                &connection,
                workspace_id,
                session_id,
                agent,
                &correlation,
                started,
                updated,
                &facts,
                &content,
                tombstone.as_deref(),
                model_switch,
                self.retention.detail_retention_ms,
            )?);
        }
        if query_visibility(&connection)? != visibility {
            return Err(ObservationQueryError::StalePreview);
        }
        Ok(SessionListV1 { sessions })
    }

    fn get_session(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        content_mode: ContentMode,
    ) -> Result<SessionDetailV1, ObservationQueryError> {
        let _permit = self
            .query_permit()
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let connection = rusqlite::Connection::open_with_flags(
            &self.activity_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
        let _deadline = crate::query_v2::QueryDeadline::start(&connection)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let visibility = query_visibility(&connection)?;
        let row = connection
            .query_row(
                "SELECT agent_id, correlation, started_at_ms, updated_at_ms, facts_completeness,
                        content_completeness, tombstone_reason
                 FROM sessions WHERE workspace_id=?1 AND session_id=?2",
                params![workspace_id.as_str(), session_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| ObservationQueryError::Unavailable)?
            .ok_or(ObservationQueryError::NotFound)?;
        let model_switch = facts::session_model_switch(&connection, workspace_id, session_id)?;
        let summary = session_summary(
            &connection,
            workspace_id,
            session_id.clone(),
            row.0,
            &row.1,
            row.2,
            row.3,
            &row.4,
            &row.5,
            row.6.as_deref(),
            model_switch,
            self.retention.detail_retention_ms,
        )?;
        let turns = read_turns(
            &connection,
            &self.authority,
            workspace_id,
            session_id,
            content_mode,
        )?;
        if query_visibility(&connection)? != visibility {
            return Err(ObservationQueryError::StalePreview);
        }
        Ok(SessionDetailV1 {
            summary,
            turns,
            content_access: if content_mode == ContentMode::None {
                ContentAccess::NotRequested
            } else {
                ContentAccess::Authorized
            },
        })
    }

    fn get_receipt(
        &self,
        workspace_id: &WorkspaceId,
        receipt_id: &ReceiptId,
    ) -> Result<RoutingReceiptV1, ObservationQueryError> {
        let _permit = self
            .query_permit()
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let connection = rusqlite::Connection::open_with_flags(
            &self.activity_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
        let _deadline = crate::query_v2::QueryDeadline::start(&connection)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let (request, body, stored_digest): (String, String, String) = connection
            .query_row(
                "SELECT r.request_id, CASE WHEN length(r.body_json)<=1048576 THEN r.body_json ELSE '' END, r.body_digest FROM routing_receipts r
                 JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
                 WHERE r.workspace_id=?1 AND r.receipt_id=?2
                   AND s.content_completeness NOT IN ('deleted','expired')
                   AND NOT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 t WHERE t.workspace_id=r.workspace_id AND t.request_id=r.request_id)",
                params![workspace_id.as_str(), receipt_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| ObservationQueryError::Unavailable)?
            .ok_or(ObservationQueryError::NotFound)?;
        if body.is_empty() {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let body = crate::store::sensitive::hydrate_bounded(
            &connection,
            "receipt",
            &stored_digest,
            &body,
            1024 * 1024,
        )
        .map_err(|_| ObservationQueryError::Corrupt)?;
        let receipt: RoutingReceiptV1 =
            serde_json::from_str(&body).map_err(|_| ObservationQueryError::Corrupt)?;
        receipt
            .validate()
            .map_err(|_| ObservationQueryError::Corrupt)?;
        let request_id = hiroute_domain::LogicalRequestId::parse(request)
            .map_err(|_| ObservationQueryError::Corrupt)?;
        let (fact_count, fact_bytes):(u64,u64)=connection.query_row(
            "SELECT count(*),COALESCE(sum(size),0) FROM (SELECT COALESCE(length(p.body_json),length(e.envelope_json)) AS size FROM execution_fact_events e LEFT JOIN observation_sensitive_payloads_v2 p ON p.id='fact:'||e.envelope_digest WHERE e.workspace_id=?1 AND e.request_id=?2 LIMIT 201)",
            params![workspace_id.as_str(),request_id.as_str()],|row|Ok((row.get(0)?,row.get(1)?)),
        ).map_err(|_|ObservationQueryError::Unavailable)?;
        if fact_count > 200 || fact_bytes > 1024 * 1024 {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let rebuilt = crate::receipt::build_receipt_for_request(
            &connection,
            workspace_id,
            &request_id,
            receipt.facts_completeness,
        )
        .map_err(|_| ObservationQueryError::Corrupt)?;
        if rebuilt != receipt
            || receipt.receipt_id != *receipt_id
            || receipt
                .digest()
                .map_err(|_| ObservationQueryError::Corrupt)?
                .as_str()
                != stored_digest
        {
            return Err(ObservationQueryError::Corrupt);
        }
        Ok(receipt)
    }

    fn get_value(
        &self,
        workspace_id: &WorkspaceId,
        query: &ValueQueryV1,
    ) -> Result<ValueViewV1, ObservationQueryError> {
        value::get_value(self, workspace_id, query)
    }

    fn get_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<ObservationStatusV1, ObservationQueryError> {
        let _permit = self
            .query_permit()
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let connection = rusqlite::Connection::open_with_flags(
            &self.activity_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
        let _deadline = crate::query_v2::QueryDeadline::start(&connection)
            .map_err(crate::query_v2::ObservationV2Error::into_domain)?;
        let gaps = read_gaps(&connection, workspace_id)?;
        let facts_completeness = aggregate_facts_completeness(&connection, workspace_id, &gaps)?;
        let content_completeness =
            aggregate_content_completeness(&connection, workspace_id, &gaps)?;
        let content_bytes: i64 = connection
            .query_row(
                "SELECT COALESCE(SUM(byte_count), 0) FROM content_blobs_v2 WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| ObservationQueryError::Unavailable)?;
        let activity_bytes = self
            .activity_path
            .metadata()
            .map(|value| value.len())
            .unwrap_or(0);
        Ok(ObservationStatusV1 {
            retention: self.retention.clone(),
            store_revision: store_revision(&connection)
                .map_err(|_| ObservationQueryError::Corrupt)?,
            facts_completeness,
            content_completeness,
            completeness_scope: ObservationCompletenessScope::GatewayVisible,
            gaps,
            activity_bytes,
            content_bytes: content_bytes
                .try_into()
                .map_err(|_| ObservationQueryError::Corrupt)?,
            production_exporter: "not_installed".to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn session_summary(
    connection: &rusqlite::Connection,
    workspace_id: &WorkspaceId,
    session_id: SessionId,
    agent_id: String,
    correlation: &str,
    started_at_ms: i64,
    updated_at_ms: i64,
    facts: &str,
    content: &str,
    tombstone: Option<&str>,
    model_switch: Option<bool>,
    retention_ms: i64,
) -> Result<SessionSummaryV1, ObservationQueryError> {
    let turn_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE workspace_id=?1 AND session_id=?2",
            params![workspace_id.as_str(), session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    let request_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM logical_requests WHERE workspace_id=?1 AND session_id=?2",
            params![workspace_id.as_str(), session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    Ok(SessionSummaryV1 {
        session_id,
        agent_id,
        correlation_provenance: parse_enum::<CorrelationProvenance>(correlation)?,
        started_at_ms,
        updated_at_ms,
        retention_deadline_ms: updated_at_ms
            .checked_add(retention_ms)
            .ok_or(ObservationQueryError::Corrupt)?,
        turn_count: turn_count
            .try_into()
            .map_err(|_| ObservationQueryError::Corrupt)?,
        request_count: request_count
            .try_into()
            .map_err(|_| ObservationQueryError::Corrupt)?,
        model_switch,
        facts_completeness: parse_facts(facts)?,
        content_completeness: parse_content(content)?,
        completeness_scope: ObservationCompletenessScope::GatewayVisible,
        tombstone_reason: tombstone.map(parse_tombstone).transpose()?,
    })
}

fn read_turns(
    connection: &rusqlite::Connection,
    authority: &crate::DigestAuthority,
    workspace_id: &WorkspaceId,
    session_id: &SessionId,
    content_mode: ContentMode,
) -> Result<Vec<TurnDetailV1>, ObservationQueryError> {
    let mut statement = connection
        .prepare(
            "SELECT turn_id, started_at_ms FROM turns WHERE workspace_id=?1 AND session_id=?2
             ORDER BY started_at_ms, turn_id LIMIT 201",
        )
        .map_err(|_| ObservationQueryError::Unavailable)?;
    let rows = statement
        .query_map(params![workspace_id.as_str(), session_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|_| ObservationQueryError::Unavailable)?;
    let mut turns = Vec::new();
    let mut body_budget = 128 * 1024;
    let mut identity_budget = 200usize;
    for row in rows {
        if turns.len() >= 200 {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let (turn, started) = row.map_err(|_| ObservationQueryError::Corrupt)?;
        let turn_id = TurnId::parse(turn).map_err(|_| ObservationQueryError::Corrupt)?;
        let request_ids = request_ids(connection, workspace_id, &turn_id)?;
        let receipt_ids = receipt_ids(connection, workspace_id, &turn_id)?;
        identity_budget = identity_budget
            .checked_sub(request_ids.len() + receipt_ids.len())
            .ok_or(ObservationQueryError::InvalidQuery)?;
        turns.push(TurnDetailV1 {
            request_ids,
            receipt_ids,
            messages: if content_mode == ContentMode::None {
                Vec::new()
            } else {
                content::messages(
                    connection,
                    authority,
                    workspace_id,
                    &turn_id,
                    content_mode,
                    &mut body_budget,
                )?
            },
            turn_id,
            started_at_ms: started,
        });
    }
    Ok(turns)
}

fn request_ids(
    connection: &rusqlite::Connection,
    workspace_id: &WorkspaceId,
    turn_id: &TurnId,
) -> Result<Vec<hiroute_domain::LogicalRequestId>, ObservationQueryError> {
    query_id_list(
        connection,
        "SELECT request_id FROM logical_requests WHERE workspace_id=?1 AND turn_id=?2
         ORDER BY started_at_ms, request_id",
        workspace_id.as_str(),
        turn_id.as_str(),
        hiroute_domain::LogicalRequestId::parse,
    )
}

fn receipt_ids(
    connection: &rusqlite::Connection,
    workspace_id: &WorkspaceId,
    turn_id: &TurnId,
) -> Result<Vec<ReceiptId>, ObservationQueryError> {
    query_id_list(
        connection,
        "SELECT receipt_id FROM routing_receipts WHERE workspace_id=?1 AND turn_id=?2
         ORDER BY frozen_at_ms, receipt_id",
        workspace_id.as_str(),
        turn_id.as_str(),
        ReceiptId::parse,
    )
}

fn query_id_list<T>(
    connection: &rusqlite::Connection,
    sql: &str,
    workspace: &str,
    parent: &str,
    parse: impl Fn(String) -> Result<T, hiroute_domain::ObservationIdentityError>,
) -> Result<Vec<T>, ObservationQueryError> {
    let mut statement = connection
        .prepare(&format!("{sql} LIMIT 201"))
        .map_err(|_| ObservationQueryError::Unavailable)?;
    let ids: Vec<T> = statement
        .query_map(params![workspace, parent], |row| row.get::<_, String>(0))
        .map_err(|_| ObservationQueryError::Unavailable)?
        .map(|row| {
            row.map_err(|_| ObservationQueryError::Corrupt)
                .and_then(|value| parse(value).map_err(|_| ObservationQueryError::Corrupt))
        })
        .collect::<Result<_, _>>()?;
    if ids.len() > 200 {
        return Err(ObservationQueryError::InvalidQuery);
    }
    Ok(ids)
}

fn parse_facts(value: &str) -> Result<FactsCompleteness, ObservationQueryError> {
    parse_enum(value)
}

fn parse_content(value: &str) -> Result<ContentCompleteness, ObservationQueryError> {
    parse_enum(value)
}

fn parse_tombstone(value: &str) -> Result<TombstoneReason, ObservationQueryError> {
    parse_enum(value)
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, ObservationQueryError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| ObservationQueryError::Corrupt)
}

fn query_visibility(connection: &rusqlite::Connection) -> Result<u64, ObservationQueryError> {
    connection.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0)).map_err(|_|ObservationQueryError::Unavailable)
}
