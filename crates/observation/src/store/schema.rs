//! One current archive format. Older observations are reset by startup, never converted here.
use crate::writer::ObservationStoreError;
use rusqlite::Connection;

pub(super) const STORE_SCHEMA_VERSION: &str = "6";

pub(super) fn migrate(connection: &mut Connection) -> Result<(), ObservationStoreError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='observation_meta' AND type='table')",
        [], |r|r.get(0)).map_err(|_|ObservationStoreError::Corrupt)?;
    if exists {
        let version: String = connection
            .query_row(
                "SELECT value FROM observation_meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| ObservationStoreError::Corrupt)?;
        if version != STORE_SCHEMA_VERSION {
            return Err(ObservationStoreError::Corrupt);
        }
        let generation: String = connection
            .query_row(
                "SELECT value FROM observation_meta WHERE key='format_generation'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| ObservationStoreError::Corrupt)?;
        if generation.len() != 32 || !generation.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ObservationStoreError::Corrupt);
        }
        let count:u64=connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN('logical_requests','observation_checkpoints','observation_current_commits_v6','conversation_content_streams_v2','content_message_instances_v2','content_instances_v2','content_blobs_v2','transcript_roots_v2')",[],|r|r.get(0)).map_err(|_|ObservationStoreError::Corrupt)?;
        if count != 8 {
            return Err(ObservationStoreError::Corrupt);
        }
    } else {
        let populated:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%')",[],|r|r.get(0)).map_err(|_|ObservationStoreError::Corrupt)?;
        if populated {
            return Err(ObservationStoreError::Corrupt);
        }
    }
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    let tx = connection
        .transaction()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS observation_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        INSERT OR IGNORE INTO observation_meta(key,value) VALUES('schema_version','6'),('store_revision','0');
        INSERT OR IGNORE INTO observation_meta VALUES('format_generation',lower(hex(randomblob(16))));")
        .map_err(|_|ObservationStoreError::ActivityUnavailable)?;
    for ddl in [
        BASE_SCHEMA_V2,
        super::agent_attribution::SCHEMA,
        VALUE_SCHEMA_V2,
        AUXILIARY_SCHEMA_V2,
        EXECUTION_FACT_SCHEMA_V3,
        CONVERSATION_CONTENT_SCHEMA_V4,
        super::current_commit::SCHEMA,
        super::fact_identity::SCHEMA,
    ] {
        tx.execute_batch(ddl)
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS observation_capture_scopes_v6(
        workspace_id TEXT NOT NULL,session_id TEXT NOT NULL,seed_request_id TEXT NOT NULL,
        seed_state TEXT NOT NULL,PRIMARY KEY(workspace_id,session_id));
        CREATE TABLE IF NOT EXISTS observation_input_anchors_v6(
        workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,source_message_id TEXT,
        PRIMARY KEY(workspace_id,request_id));",
    )
    .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    tx.commit()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

const BASE_SCHEMA_V2: &str = r#"
    CREATE TABLE IF NOT EXISTS sessions (
        workspace_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        agent_id TEXT NOT NULL DEFAULT '',
        correlation TEXT NOT NULL DEFAULT 'unproven',
        started_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        facts_completeness TEXT NOT NULL DEFAULT 'unknown',
        content_completeness TEXT NOT NULL DEFAULT 'unknown',
        tombstone_reason TEXT,
        PRIMARY KEY(workspace_id, session_id)
    );
    CREATE INDEX IF NOT EXISTS sessions_updated
        ON sessions(workspace_id, updated_at_ms DESC);

    CREATE TABLE IF NOT EXISTS turns (
        workspace_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        started_at_ms INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, turn_id)
    );
    CREATE INDEX IF NOT EXISTS turns_session ON turns(workspace_id, session_id, started_at_ms);

    CREATE TABLE IF NOT EXISTS logical_requests (
        workspace_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        session_scope TEXT,
        correlation_provenance TEXT,
        traffic_kind TEXT NOT NULL DEFAULT 'normal',
        route_json TEXT,
        outcome TEXT,
        receipt_id TEXT,
        started_at_ms INTEGER NOT NULL,
        finished_at_ms INTEGER,
        PRIMARY KEY(workspace_id, request_id)
    );
    CREATE INDEX IF NOT EXISTS requests_session
        ON logical_requests(workspace_id, session_id, started_at_ms);
    CREATE INDEX IF NOT EXISTS requests_observation_fields
        ON logical_requests(workspace_id, request_id, session_id, started_at_ms);

    CREATE TABLE IF NOT EXISTS attempts (
        workspace_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        attempt_id TEXT NOT NULL,
        ordinal INTEGER NOT NULL,
        body_json TEXT NOT NULL,
        PRIMARY KEY(workspace_id, request_id, attempt_id),
        UNIQUE(workspace_id, request_id, ordinal)
    );

    CREATE TABLE IF NOT EXISTS routing_receipts (
        workspace_id TEXT NOT NULL,
        receipt_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        body_json TEXT NOT NULL,
        body_digest TEXT NOT NULL,
        frozen_at_ms INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, receipt_id),
        UNIQUE(workspace_id, request_id)
    );
    "#;

const AUXILIARY_SCHEMA_V2: &str = r#"

    CREATE TABLE IF NOT EXISTS observation_checkpoints (
        channel TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        highest_contiguous_sequence INTEGER NOT NULL,
        highest_accounted_sequence INTEGER NOT NULL,
        PRIMARY KEY(channel, producer_id, producer_epoch, stream_id)
    );

    CREATE TABLE IF NOT EXISTS observation_gaps (
        gap_id INTEGER PRIMARY KEY AUTOINCREMENT,
        channel TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        first_sequence INTEGER NOT NULL,
        last_sequence INTEGER NOT NULL,
        known_loss INTEGER NOT NULL,
        workspace_id TEXT NOT NULL,
        session_id TEXT,
        reason TEXT
    );

    CREATE TABLE IF NOT EXISTS observation_tombstones (
        workspace_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        reason TEXT NOT NULL,
        delete_scope TEXT NOT NULL,
        deleted_at_ms INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, session_id, reason, delete_scope)
    );
    CREATE INDEX IF NOT EXISTS observation_gaps_scope
        ON observation_gaps(workspace_id, session_id, channel);
    CREATE INDEX IF NOT EXISTS observation_gaps_stream_end
        ON observation_gaps(channel, producer_id, producer_epoch, stream_id, last_sequence);
    "#;

const EXECUTION_FACT_SCHEMA_V3: &str = r#"
    CREATE TABLE IF NOT EXISTS execution_fact_events (
        workspace_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        envelope_json TEXT NOT NULL,
        envelope_digest TEXT NOT NULL,
        PRIMARY KEY(producer_id, producer_epoch, stream_id, sequence),
        UNIQUE(producer_id, producer_epoch, stream_id, event_id)
    );
    CREATE INDEX IF NOT EXISTS execution_facts_request
        ON execution_fact_events(workspace_id, request_id, sequence);
    CREATE INDEX IF NOT EXISTS execution_facts_request_payload
        ON execution_fact_events(workspace_id, request_id, envelope_digest, sequence);
    CREATE INDEX IF NOT EXISTS execution_facts_request_identity
        ON execution_fact_events(workspace_id, request_id, envelope_digest, sequence,
          producer_id, producer_epoch, stream_id, event_id);
    CREATE INDEX IF NOT EXISTS execution_facts_session
        ON execution_fact_events(workspace_id, session_id, sequence);
"#;

const CONVERSATION_CONTENT_SCHEMA_V4: &str = r#"
    CREATE TABLE IF NOT EXISTS conversation_content_streams_v2 (
        workspace_id TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        conversation_id TEXT NOT NULL,
        session_scope TEXT NOT NULL,
        correlation_provenance TEXT NOT NULL,
        turn_id TEXT NOT NULL,
        attempt_id TEXT,
        parent_transcript_root TEXT,
        result_transcript_root TEXT,
        state TEXT NOT NULL,
        next_chunk_ordinal INTEGER NOT NULL,
        active_content_id TEXT,
        begin_sequence INTEGER NOT NULL,
        terminal_sequence INTEGER,
        completeness TEXT NOT NULL,
        updated_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(producer_id, producer_epoch, stream_id, request_id, direction, fork_id)
    );
    CREATE INDEX IF NOT EXISTS content_stream_v2_request
        ON conversation_content_streams_v2(workspace_id, request_id, direction, fork_id);


    CREATE TABLE IF NOT EXISTS content_message_instances_v2 (
        workspace_id TEXT NOT NULL,
        message_instance_id TEXT NOT NULL,
        conversation_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        message_ordinal INTEGER NOT NULL,
        message_role TEXT NOT NULL,
        occurred_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, message_instance_id),
        UNIQUE(workspace_id, request_id, direction, fork_id, message_ordinal)
    );

    CREATE TABLE IF NOT EXISTS content_instances_v2 (
        workspace_id TEXT NOT NULL,
        content_id TEXT NOT NULL,
        message_instance_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        part_ordinal INTEGER NOT NULL,
        content_kind TEXT NOT NULL,
        canonical_media_type TEXT NOT NULL,
        content_blob_digest TEXT NOT NULL,
        attempt_id TEXT,
        semantic_metadata_json TEXT,
        expected_byte_count INTEGER,
        has_content_ref INTEGER NOT NULL,
        transport_frame_id TEXT,
        downstream_delivery TEXT,
        accumulated_byte_count INTEGER NOT NULL,
        state TEXT NOT NULL,
        created_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, content_id)
    );
    CREATE UNIQUE INDEX IF NOT EXISTS content_instances_v2_message_part ON content_instances_v2(workspace_id,message_instance_id,part_ordinal);
    CREATE INDEX IF NOT EXISTS content_instances_v2_request
        ON content_instances_v2(workspace_id, request_id, direction, fork_id, part_ordinal);

    CREATE INDEX IF NOT EXISTS content_instances_v2_live_blob
        ON content_instances_v2(workspace_id, content_blob_digest) WHERE state='complete';

    -- Cover live-search metadata without loading every content occurrence.
    -- The implicit rowid preserves the first eligible occurrence per request.
    CREATE INDEX IF NOT EXISTS content_instances_v2_search_blob
        ON content_instances_v2(workspace_id, content_blob_digest,
            content_kind, canonical_media_type, request_id) WHERE state='complete';

    CREATE TABLE IF NOT EXISTS content_chunks_v2 (
        workspace_id TEXT NOT NULL,
        content_id TEXT NOT NULL,
        chunk_ordinal INTEGER NOT NULL,
        object_path TEXT NOT NULL,
        byte_offset INTEGER NOT NULL,
        byte_count INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        PRIMARY KEY(workspace_id, content_id, chunk_ordinal)
    );

    CREATE TABLE IF NOT EXISTS content_blobs_v2 (
        workspace_id TEXT NOT NULL,
        blob_digest TEXT NOT NULL,
        object_path TEXT NOT NULL,
        byte_count INTEGER NOT NULL,
        media_type TEXT NOT NULL,
        state TEXT NOT NULL,
        created_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, blob_digest)
    );
    CREATE INDEX IF NOT EXISTS content_blobs_v2_workspace_bytes
        ON content_blobs_v2(workspace_id, byte_count);


    CREATE TABLE IF NOT EXISTS transcript_roots_v2 (
        workspace_id TEXT NOT NULL,
        transcript_root TEXT NOT NULL,
        conversation_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        parent_transcript_root TEXT,
        state TEXT NOT NULL,
        acknowledged_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, transcript_root)
    );

    CREATE INDEX IF NOT EXISTS transcript_roots_v2_request
        ON transcript_roots_v2(workspace_id, request_id, direction, state);

"#;

const VALUE_SCHEMA_V2: &str = r#"
    CREATE TABLE IF NOT EXISTS value_ledger_entries (
        workspace_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        receipt_id TEXT NOT NULL,
        body_json TEXT NOT NULL,
        body_digest TEXT NOT NULL,
        agent_plan_id TEXT NOT NULL,
        currency TEXT NOT NULL,
        billing_unit TEXT NOT NULL,
        price_version TEXT NOT NULL,
        price_override_revision TEXT,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        reasoning_tokens INTEGER NOT NULL,
        baseline_api_equivalent_cost_micros INTEGER,
        chosen_api_equivalent_cost_micros INTEGER,
        actual_incremental_cost_micros INTEGER,
        routing_savings_micros INTEGER,
        entitlement_savings_micros INTEGER,
        estimated_total_savings_micros INTEGER,
        facts_completeness TEXT NOT NULL,
        frozen_at_ms INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, request_id)
    );
    CREATE INDEX IF NOT EXISTS value_time
        ON value_ledger_entries(workspace_id, agent_plan_id, currency, frozen_at_ms);

    CREATE TABLE IF NOT EXISTS session_value_rollup_contributions (
        workspace_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        agent_plan_id TEXT NOT NULL,
        day_number INTEGER NOT NULL,
        currency TEXT NOT NULL,
        billing_unit TEXT NOT NULL,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        reasoning_tokens INTEGER NOT NULL,
        baseline_api_equivalent_cost_micros INTEGER,
        chosen_api_equivalent_cost_micros INTEGER,
        actual_incremental_cost_micros INTEGER,
        routing_savings_micros INTEGER,
        entitlement_savings_micros INTEGER,
        estimated_total_savings_micros INTEGER,
        price_version_refs_json TEXT NOT NULL,
        price_override_revision_refs_json TEXT NOT NULL,
        facts_completeness TEXT NOT NULL,
        PRIMARY KEY(workspace_id, session_id, agent_plan_id, day_number, currency, billing_unit)
    );
    CREATE INDEX IF NOT EXISTS rollup_contributions_bucket
        ON session_value_rollup_contributions(
            workspace_id, agent_plan_id, day_number, currency, billing_unit
        );

    CREATE TABLE IF NOT EXISTS daily_value_rollups (
        workspace_id TEXT NOT NULL,
        agent_plan_id TEXT NOT NULL,
        day_number INTEGER NOT NULL,
        currency TEXT NOT NULL,
        billing_unit TEXT NOT NULL,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        reasoning_tokens INTEGER NOT NULL,
        baseline_api_equivalent_cost_micros INTEGER,
        chosen_api_equivalent_cost_micros INTEGER,
        actual_incremental_cost_micros INTEGER,
        routing_savings_micros INTEGER,
        entitlement_savings_micros INTEGER,
        estimated_total_savings_micros INTEGER,
        price_version_refs_json TEXT NOT NULL,
        price_override_revision_refs_json TEXT NOT NULL,
        facts_completeness TEXT NOT NULL,
        PRIMARY KEY(workspace_id, agent_plan_id, day_number, currency, billing_unit)
    );
"#;
