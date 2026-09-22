use super::*;
use hiroute_domain::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogCursor {
    schema: String,
    binding: String,
    visibility: u64,
    watermark: i64,
    after: Option<(String, String, u64, u64)>,
}

impl crate::LocalObservationStore {
    pub fn observed_catalog(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationCatalogQueryV2,
        now_ms: i64,
    ) -> Result<ObservationCatalogPageV2, ObservationV2Error> {
        reader.check(now_ms, true, false)?;
        if query.limit == 0 || query.limit > 200 {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        let tx = connection.transaction()?;
        if reader.allowed_runs().is_some() {
            relation::authorized_link(&tx, reader, &query.request_id, now_ms)?;
        }
        let binding = CanonicalDigest::of(&(reader.binding()?, &query.request_id, query.limit))
            .map_err(|_| ObservationV2Error::Invalid)?
            .to_string();
        let visibility=tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|r|r.get(0))?;
        let cursor = match &query.cursor {
            Some(encoded) => {
                let cursor: CatalogCursor = self.decode_observation_cursor(encoded)?;
                if cursor.schema != "catalog/v2"
                    || cursor.binding != binding
                    || cursor.visibility != visibility
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => CatalogCursor {
                schema: "catalog/v2".into(),
                binding,
                visibility,
                watermark: tx.query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM content_instances_v2",
                    [],
                    |r| r.get(0),
                )?,
                after: None,
            },
        };
        let after = cursor.after.clone().unwrap_or_default();
        let mut contents = {
            let mut stmt=tx.prepare("SELECT c.content_id,c.message_instance_id,m.message_ordinal,c.part_ordinal,m.message_role,c.direction,c.fork_id,c.canonical_media_type,c.accumulated_byte_count,c.state,c.downstream_delivery,c.content_kind
             FROM content_instances_v2 c JOIN content_message_instances_v2 m ON m.workspace_id=c.workspace_id AND m.message_instance_id=c.message_instance_id
             JOIN logical_requests r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id
             WHERE c.workspace_id=?1 AND c.request_id=?2 AND r.started_at_ms>?3 AND c.rowid<=?4
               AND (?5=0 OR (c.direction,c.fork_id,m.message_ordinal,c.part_ordinal)>(?6,?7,?8,?9))
             ORDER BY c.direction,c.fork_id,m.message_ordinal,c.part_ordinal LIMIT ?10")?;
            stmt.query_map(
                params![
                    reader.workspace().as_str(),
                    query.request_id.as_str(),
                    now_ms.saturating_sub(crate::managed_text::RETENTION_MS),
                    cursor.watermark,
                    cursor.after.is_some(),
                    after.0,
                    after.1,
                    after.2,
                    after.3,
                    u64::from(query.limit) + 1
                ],
                |r| {
                    Ok(ObservationCatalogEntryV2 {
                        request_id: query.request_id.clone(),
                        content_id: r.get(0)?,
                        message_occurrence_id: r.get(1)?,
                        message_ordinal: r.get(2)?,
                        part_ordinal: r.get(3)?,
                        role: r.get(4)?,
                        kind: r.get(11)?,
                        direction: r.get(5)?,
                        fork_id: r.get(6)?,
                        media_type: r.get(7)?,
                        byte_count: r.get(8)?,
                        state: r.get(9)?,
                        downstream_delivery: r.get(10)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        let more = contents.len() > usize::from(query.limit);
        contents.truncate(usize::from(query.limit));
        let next_cursor = if more {
            let last = contents.last().ok_or(ObservationV2Error::Unavailable)?;
            Some(self.encode_observation_cursor(&CatalogCursor {
                after: Some((
                    last.direction.clone(),
                    last.fork_id.clone(),
                    last.message_ordinal,
                    last.part_ordinal,
                )),
                ..cursor
            })?)
        } else {
            None
        };
        let mut transcript_roots = {
            let mut stmt=tx.prepare("SELECT t.transcript_root FROM transcript_roots_v2 t JOIN logical_requests r ON r.workspace_id=t.workspace_id AND r.request_id=t.request_id WHERE t.workspace_id=?1 AND t.request_id=?2 AND r.started_at_ms>?3 ORDER BY t.transcript_root LIMIT 33")?;
            stmt.query_map(
                params![
                    reader.workspace().as_str(),
                    query.request_id.as_str(),
                    now_ms.saturating_sub(crate::managed_text::RETENTION_MS)
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        let roots_partial = transcript_roots.len() > 32;
        transcript_roots.truncate(32);
        tx.commit()?;
        check_visibility(&connection, visibility)?;
        Ok(ObservationCatalogPageV2 {
            contents,
            next_cursor,
            transcript_roots,
            roots_partial,
        })
    }

    /// No unbounded recursive SQL or client-driven path access. A missing,
    /// unauthorized, cyclic or over-depth parent is an explicit history gap.
    pub fn observed_ancestry(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationAncestryQueryV2,
        now_ms: i64,
    ) -> Result<ObservationAncestryV2, ObservationV2Error> {
        reader.check(now_ms, true, false)?;
        if !identifier(&query.transcript_root) {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        let tx = connection.transaction()?;
        if reader.allowed_runs().is_some() {
            relation::authorized_link(&tx, reader, &query.request_id, now_ms)?;
        }
        let visibility=tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|r|r.get(0))?;
        let session:Option<String>=tx.query_row("SELECT session_id FROM logical_requests WHERE workspace_id=?1 AND request_id=?2 AND started_at_ms>?3",params![reader.workspace().as_str(),query.request_id.as_str(),now_ms.saturating_sub(crate::managed_text::RETENTION_MS)],|r|r.get(0)).optional()?;
        let session = session.ok_or(ObservationV2Error::Unavailable)?;
        let mut next = Some(query.transcript_root.clone());
        let mut visited = std::collections::BTreeSet::new();
        let mut roots = Vec::new();
        let mut gap = None;
        while let Some(root) = next.take() {
            if !visited.insert(root.clone()) {
                gap = Some("cycle".into());
                break;
            }
            if roots.len() == 32 {
                gap = Some("depth_budget".into());
                break;
            }
            let row:Option<(String,String,String,String,Option<String>)>=tx.query_row("SELECT t.request_id,t.fork_id,t.direction,t.state,t.parent_transcript_root FROM transcript_roots_v2 t JOIN logical_requests r ON r.workspace_id=t.workspace_id AND r.request_id=t.request_id WHERE t.workspace_id=?1 AND t.transcript_root=?2 AND t.conversation_id=?3 AND r.started_at_ms>?4",params![reader.workspace().as_str(),root,session,now_ms.saturating_sub(crate::managed_text::RETENTION_MS)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
            let Some((request, fork, direction, state, parent)) = row else {
                gap = Some("history_unavailable".into());
                break;
            };
            if roots.is_empty() && request != query.request_id.as_str() {
                return Err(ObservationV2Error::Invalid);
            }
            let request =
                LogicalRequestId::parse(request).map_err(|_| ObservationV2Error::Unavailable)?;
            if reader.allowed_runs().is_some()
                && relation::authorized_link(&tx, reader, &request, now_ms).is_err()
            {
                gap = Some("history_unavailable".into());
                break;
            }
            roots.push(ObservationTranscriptRootV2 {
                transcript_root: root,
                request_id: request,
                fork_id: fork,
                direction,
                state,
            });
            next = parent;
        }
        tx.commit()?;
        check_visibility(&connection, visibility)?;
        Ok(ObservationAncestryV2 { roots, gap })
    }
}
