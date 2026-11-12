use super::*;
use hiroute_domain::{
    CanonicalDigest, ObservationContentPageV2, ObservationContentQueryV2, ObservationTextChunkV2,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentCursor {
    schema: u8,
    binding: String,
    visibility: u64,
    ordinal: u64,
}

type ContentMetadata = (
    String,
    String,
    String,
    u64,
    String,
    Option<String>,
    Option<String>,
);

impl crate::LocalObservationStore {
    /// Read verified text in 64 KiB source blocks, capped at two per response to
    /// keep even escaped JSON below one MiB. Binary content remains an explicit
    /// reference; no OS path or implicit blob authority is returned.
    pub fn observed_content_page(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationContentQueryV2,
        now_ms: i64,
    ) -> Result<ObservationContentPageV2, ObservationV2Error> {
        self.observed_content_page_admitted(reader, query, now_ms, false)
    }

    pub(crate) fn observed_content_page_admitted(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationContentQueryV2,
        now_ms: i64,
        admitted: bool,
    ) -> Result<ObservationContentPageV2, ObservationV2Error> {
        reader.check(now_ms, true, false)?;
        let _permit = if admitted {
            None
        } else {
            Some(self.query_permit()?)
        };
        if !identifier(&query.content_id)
            || query
                .anchor_offset
                .is_some_and(|offset| offset > i64::MAX as u64)
        {
            return Err(ObservationV2Error::Invalid);
        }
        let binding = CanonicalDigest::of(&(
            reader.binding()?,
            &query.request_id,
            &query.content_id,
            query.anchor_offset,
        ))
        .map_err(|_| ObservationV2Error::Invalid)?
        .to_string();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = super::QueryDeadline::start_for(&connection, reader)?;
        let transaction = connection.transaction()?;
        if reader.allowed_runs().is_some() {
            relation::authorized_link(&transaction, reader, &query.request_id, now_ms)?;
        }
        let visibility:u64=transaction.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0))?;
        let cursor = match &query.cursor {
            Some(encoded) => {
                let cursor: ContentCursor = self.decode_observation_cursor(encoded)?;
                if cursor.schema != 2
                    || cursor.binding != binding
                    || cursor.visibility != visibility
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => ContentCursor {
                schema: 2,
                binding,
                visibility,
                ordinal:transaction.query_row("SELECT COALESCE(MAX(b.ordinal),0) FROM observation_text_blocks_v2 b JOIN content_instances_v2 c ON c.workspace_id=b.workspace AND c.content_blob_digest=b.digest WHERE c.workspace_id=?1 AND c.request_id=?2 AND c.content_id=?3 AND b.primary_start<=?4",params![reader.workspace().as_str(),query.request_id.as_str(),query.content_id,query.anchor_offset.unwrap_or(0)],|r|r.get(0))?,
            },
        };
        let metadata:Option<ContentMetadata>=transaction.query_row(
            "SELECT c.message_instance_id,c.canonical_media_type,c.content_blob_digest,c.accumulated_byte_count,c.direction,c.downstream_delivery,i.state
             FROM content_instances_v2 c JOIN logical_requests r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id
             LEFT JOIN observation_text_index_v2 i ON i.workspace=c.workspace_id AND i.digest=c.content_blob_digest
             WHERE c.workspace_id=?1 AND c.request_id=?2 AND c.content_id=?3 AND c.state='complete' AND r.started_at_ms>?4",
            params![reader.workspace().as_str(),query.request_id.as_str(),query.content_id,now_ms.saturating_sub(crate::managed_text::RETENTION_MS)],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
        ).optional()?;
        let Some((occurrence, media, digest, byte_count, direction, delivery, index_state)) =
            metadata
        else {
            // Pending content can still complete. A missing or expired instance
            // has no current body to retry, even when a caller has an old ID.
            let retained: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM content_instances_v2 c JOIN logical_requests r ON r.workspace_id=c.workspace_id AND r.request_id=c.request_id
                 WHERE c.workspace_id=?1 AND c.request_id=?2 AND c.content_id=?3
                   AND c.state NOT IN ('deleted','expired') AND r.started_at_ms>?4)",
                params![reader.workspace().as_str(),query.request_id.as_str(),query.content_id,now_ms.saturating_sub(crate::managed_text::RETENTION_MS)],
                |row| row.get(0),
            )?;
            return Err(if retained {
                ObservationV2Error::Unavailable
            } else {
                ObservationV2Error::NotFound
            });
        };
        if query
            .anchor_offset
            .is_some_and(|offset| offset > byte_count)
        {
            return Err(ObservationV2Error::Invalid);
        }
        let mut chunks = Vec::new();
        let mut next_cursor = None;
        let state = if index_state.as_deref() == Some("ready") {
            let mut statement=transaction.prepare("SELECT ordinal,original_start,primary_start,original FROM observation_text_blocks_v2 WHERE workspace=?1 AND digest=?2 AND ordinal>=?3 ORDER BY ordinal LIMIT 3")?;
            let rows = statement
                .query_map(
                    params![reader.workspace().as_str(), digest, cursor.ordinal],
                    |row| {
                        Ok((
                            row.get::<_, u64>(0)?,
                            row.get::<_, u64>(1)?,
                            row.get::<_, u64>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            if rows.len() > 2 {
                next_cursor = Some(self.encode_observation_cursor(&ContentCursor {
                    ordinal: rows[2].0,
                    ..cursor
                })?);
            }
            for (_, start, primary, text) in rows.into_iter().take(2) {
                let offset = usize::try_from(
                    primary
                        .checked_sub(start)
                        .ok_or(ObservationV2Error::Unavailable)?,
                )
                .map_err(|_| ObservationV2Error::Unavailable)?;
                let text = text.get(offset..).ok_or(ObservationV2Error::Unavailable)?;
                if text.len() > 64 * 1024 + 3 {
                    return Err(ObservationV2Error::Unavailable);
                }
                chunks.push(ObservationTextChunkV2 {
                    original_byte_offset: primary,
                    text: text.into(),
                });
            }
            "available"
        } else if media.starts_with("text/")
            || media == "application/json"
            || media.starts_with("application/vnd.hiroute.")
        {
            "index_pending_or_unavailable"
        } else {
            "binary_reference"
        };
        transaction.commit()?;
        let current:u64=connection.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0))?;
        if current != visibility {
            return Err(ObservationV2Error::Stale);
        }
        reader.check(now_ms, true, false)?;
        Ok(ObservationContentPageV2 {
            request_id: query.request_id.clone(),
            content_id: query.content_id.clone(),
            message_occurrence_id: occurrence,
            media_type: media,
            original_digest: digest,
            original_byte_count: byte_count,
            direction,
            downstream_delivery: delivery,
            state: state.into(),
            chunks,
            next_cursor,
        })
    }
}
