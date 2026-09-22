use super::*;
use hiroute_domain::{
    CanonicalDigest, ObservationSearchPageV2, ObservationSearchQueryV2, ObservationTextAnchorV2,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCursor {
    schema: u8,
    binding: String,
    visibility: u64,
    content_watermark: i64,
    request_watermark: i64,
    publication_watermark: u64,
    content_row: i64,
    ordinal: u64,
    position: usize,
}

impl crate::LocalObservationStore {
    pub fn search_observed_text(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationSearchQueryV2,
        now_ms: i64,
    ) -> Result<ObservationSearchPageV2, ObservationV2Error> {
        reader.check(now_ms, true, true)?;
        let _permit = self.query_permit()?;
        let started = Instant::now();
        let keyword = query.keyword.trim();
        if keyword.is_empty()
            || keyword.len() > 256
            || query.from_ms < 0
            || query.from_ms >= query.to_ms
            || query.limit == 0
            || query.limit > 200
            || [
                &query.session_id,
                &query.agent_id,
                &query.plan_id,
                &query.native_model,
                &query.outcome,
            ]
            .into_iter()
            .flatten()
            .any(|id| !identifier(id))
        {
            return Err(ObservationV2Error::Invalid);
        }
        let needle = keyword.to_lowercase();
        let binding = CanonicalDigest::of(&(
            reader.binding()?,
            query.from_ms,
            query.to_ms,
            &query.session_id,
            &needle,
            query.limit,
            &query.agent_id,
            &query.plan_id,
            &query.native_model,
            &query.outcome,
            query.only_model_switch,
        ))
        .map_err(|_| ObservationV2Error::Invalid)?
        .to_string();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = super::QueryDeadline::start_for(&connection, reader)?;
        let transaction = connection.transaction()?;
        let visibility: u64 = transaction.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0))?;
        let mut cursor: SearchCursor = match &query.cursor {
            Some(encoded) => {
                let cursor: SearchCursor = self.decode_observation_cursor(encoded)?;
                if cursor.schema != 2
                    || cursor.binding != binding
                    || cursor.visibility != visibility
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => SearchCursor {
                schema: 2,
                binding,
                visibility,
                request_watermark: transaction.query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM logical_requests",
                    [],
                    |r| r.get(0),
                )?,
                content_watermark: transaction.query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM content_instances_v2",
                    [],
                    |row| row.get(0),
                )?,
                publication_watermark: transaction.query_row(
                    "SELECT COALESCE(MAX(published),0) FROM observation_text_index_v2",
                    [],
                    |row| row.get(0),
                )?,
                content_row: 0,
                ordinal: 0,
                position: 0,
            },
        };
        let runs = reader
            .allowed_runs()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let from = query.from_ms.max(
            now_ms
                .saturating_sub(crate::managed_text::RETENTION_MS)
                .saturating_add(1),
        );
        let partial_sql = format!("{}{}", super::turns::cte(5, "?15"),
            "SELECT EXISTS(SELECT 1 FROM content_instances_v2 c JOIN logical_requests r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id
             LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
             LEFT JOIN observation_text_index_v2 i ON i.workspace=c.workspace_id AND i.digest=c.content_blob_digest
             WHERE c.workspace_id=?1 AND c.state='complete' AND c.content_kind NOT IN ('provider_state','reasoning_delta','reasoning_finished') AND r.started_at_ms>=?2 AND r.started_at_ms<?3 AND (?4 IS NULL OR r.session_id=?4)
               AND (?5 IS NULL OR (l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(?5))))
 AND (?10 IS NULL OR EXISTS(SELECT 1 FROM sessions s WHERE s.workspace_id=r.workspace_id AND s.session_id=r.session_id AND s.agent_id=?10))
 AND (?11 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?11))
 AND (?12 IS NULL OR EXISTS(SELECT 1 FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id AND a.model_id=?12))
 AND (?13 IS NULL OR r.outcome=?13)
 AND (?14=0 OR (SELECT COUNT(DISTINCT model_id) FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id)>1 OR EXISTS(SELECT 1 FROM turn_changes t WHERE t.request_id=r.request_id AND t.changed=1))
               AND (c.canonical_media_type LIKE 'text/%' OR c.canonical_media_type='application/json' OR c.canonical_media_type LIKE 'application/vnd.hiroute.%')
               AND (i.state IS NULL OR i.state!='ready' OR i.published>?6))");
        let index_partial: bool = transaction.query_row(
            &partial_sql,
            params![
                reader.workspace().as_str(),
                from,
                query.to_ms,
                query.session_id,
                runs,
                cursor.publication_watermark,
                None::<i64>,
                None::<i64>,
                None::<i64>,
                query.agent_id,
                query.plan_id,
                query.native_model,
                query.outcome,
                query.only_model_switch,
                cursor.request_watermark
            ],
            |row| row.get(0),
        )?;
        let mut hits = Vec::new();
        let mut bytes = 0;
        let mut candidates = 0;
        let mut budget_exhausted = false;
        let mut more = false;
        {
            let sql = format!("{}{}", super::turns::cte(5, "?15"),
                "SELECT c.rowid,b.ordinal,b.original_start,b.primary_start,b.folded,b.offsets,r.session_id,r.request_id,c.message_instance_id,c.content_id,CASE WHEN l.conflicted=0 THEN l.body_json ELSE NULL END
                 FROM content_instances_v2 c JOIN observation_text_index_v2 i ON i.workspace=c.workspace_id AND i.digest=c.content_blob_digest
                 JOIN observation_text_blocks_v2 b ON b.workspace=i.workspace AND b.digest=i.digest
                 JOIN logical_requests r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id
                 LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
                 WHERE c.workspace_id=?1 AND c.state='complete' AND c.content_kind NOT IN ('provider_state','reasoning_delta','reasoning_finished') AND i.state='ready' AND r.started_at_ms>=?2 AND r.started_at_ms<?3
                   AND (?4 IS NULL OR r.session_id=?4) AND (?5 IS NULL OR (l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(?5))))
                   AND c.rowid<=?6 AND i.published<=?7 AND (c.rowid>?8 OR (c.rowid=?8 AND b.ordinal>=?9))
 AND (?10 IS NULL OR EXISTS(SELECT 1 FROM sessions s WHERE s.workspace_id=r.workspace_id AND s.session_id=r.session_id AND s.agent_id=?10))
 AND (?11 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?11))
 AND (?12 IS NULL OR EXISTS(SELECT 1 FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id AND a.model_id=?12))
 AND (?13 IS NULL OR r.outcome=?13)
 AND (?14=0 OR (SELECT COUNT(DISTINCT model_id) FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id)>1 OR EXISTS(SELECT 1 FROM turn_changes t WHERE t.request_id=r.request_id AND t.changed=1))
                 ORDER BY c.rowid,b.ordinal LIMIT 201"
            );
            let mut statement = transaction.prepare(&sql)?;
            let mut rows = statement.query(params![
                reader.workspace().as_str(),
                from,
                query.to_ms,
                query.session_id,
                runs,
                cursor.content_watermark,
                cursor.publication_watermark,
                cursor.content_row,
                cursor.ordinal,
                query.agent_id,
                query.plan_id,
                query.native_model,
                query.outcome,
                query.only_model_switch,
                cursor.request_watermark
            ])?;
            'candidates: while let Some(row) = rows.next()? {
                if candidates == 200
                    || bytes >= 8 * 1024 * 1024
                    || started.elapsed() >= Duration::from_millis(500)
                {
                    if candidates == 0 {
                        return Err(ObservationV2Error::Unavailable);
                    }
                    budget_exhausted = true;
                    more = true;
                    break;
                }
                let id: i64 = row.get(0)?;
                let ordinal: u64 = row.get(1)?;
                let original_start: u64 = row.get(2)?;
                let primary_start: u64 = row.get(3)?;
                let needed = row
                    .get_ref(4)?
                    .as_bytes()
                    .map_err(|_| ObservationV2Error::Unavailable)?
                    .len()
                    + row
                        .get_ref(5)?
                        .as_bytes()
                        .map_err(|_| ObservationV2Error::Unavailable)?
                        .len();
                if bytes + needed > 8 * 1024 * 1024 {
                    budget_exhausted = true;
                    more = true;
                    break;
                }
                let text: String = row.get(4)?;
                let offsets: Vec<u8> = row.get(5)?;
                candidates += 1;
                bytes += text.len() + offsets.len();
                let mut position = if cursor.content_row == id && cursor.ordinal == ordinal {
                    cursor.position
                } else {
                    0
                };
                if !text.is_char_boundary(position) {
                    return Err(ObservationV2Error::Invalid);
                }
                while let Some(relative) = text[position..].find(&needle) {
                    let found = position + relative;
                    let next = found
                        + text[found..]
                            .chars()
                            .next()
                            .ok_or(ObservationV2Error::Unavailable)?
                            .len_utf8();
                    let offset = crate::text_index::original_offset(&offsets, found)
                        .ok_or(ObservationV2Error::Unavailable)?
                        + original_start;
                    let end = crate::text_index::original_offset(&offsets, found + needle.len())
                        .ok_or(ObservationV2Error::Unavailable)?
                        + original_start;
                    position = next;
                    cursor.content_row = id;
                    cursor.ordinal = ordinal;
                    cursor.position = position;
                    if ordinal > 0 && offset < primary_start && end <= primary_start {
                        continue;
                    }
                    let native_turn_id = row
                        .get::<_, Option<String>>(10)?
                        .and_then(|body| serde_json::from_str::<RunObservationLink>(&body).ok())
                        .and_then(|link| link.native_turn_id);
                    hits.push(ObservationTextAnchorV2 {
                        session_id: row.get(6)?,
                        request_id: row.get(7)?,
                        native_turn_id,
                        message_occurrence_id: row.get(8)?,
                        content_id: row.get(9)?,
                        original_text_offset: offset,
                        match_kind: "literal_text".into(),
                    });
                    if hits.len() == usize::from(query.limit) {
                        more = true;
                        break 'candidates;
                    }
                }
                cursor.content_row = id;
                cursor.ordinal = ordinal + 1;
                cursor.position = 0;
            }
        }
        transaction.commit()?;
        // A fresh read after the short snapshot is the visibility linearization
        // point. A deletion committed before it makes every candidate stale.
        let current: u64 = connection.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0))?;
        if current != visibility {
            return Err(ObservationV2Error::Stale);
        }
        Ok(ObservationSearchPageV2 {
            hits,
            index_partial,
            budget_exhausted,
            next_cursor: if more {
                Some(self.encode_observation_cursor(&cursor)?)
            } else {
                None
            },
        })
    }
}
