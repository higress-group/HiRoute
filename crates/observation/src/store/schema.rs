use hiroute_domain::{CanonicalDigest, ValueLedgerEntryV1};
use rusqlite::{Connection, Transaction, params};

use crate::writer::ObservationStoreError;

pub(super) const STORE_SCHEMA_VERSION: &str = "4";
const LEGACY_AGENT_PLAN_ID: &str = "legacy/unknown";

pub(super) fn migrate(connection: &mut Connection) -> Result<(), ObservationStoreError> {
    connection
        .execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS observation_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO observation_meta(key, value) VALUES
                ('schema_version', '4'),
                ('store_revision', '0');
            "#,
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;

    let version: String = connection
        .query_row(
            "SELECT value FROM observation_meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ObservationStoreError::Corrupt)?;
    match version.as_str() {
        "1" => {
            migrate_v1_to_v2(connection)?;
            migrate_v2_to_v3(connection)?;
            migrate_v3_to_v4(connection)
        }
        "2" => {
            migrate_v2_to_v3(connection)?;
            migrate_v3_to_v4(connection)
        }
        "3" => migrate_v3_to_v4(connection),
        STORE_SCHEMA_VERSION => create_schema_v4(connection),
        _ => Err(ObservationStoreError::Corrupt),
    }
}

fn create_schema_v2(connection: &Connection) -> Result<(), ObservationStoreError> {
    connection
        .execute_batch(BASE_SCHEMA_V2)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    connection
        .execute_batch(VALUE_SCHEMA_V2)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    connection
        .execute_batch(AUXILIARY_SCHEMA_V2)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

fn create_schema_v3(connection: &Connection) -> Result<(), ObservationStoreError> {
    create_schema_v2(connection)?;
    connection
        .execute_batch(EXECUTION_FACT_SCHEMA_V3)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

fn create_schema_v4(connection: &Connection) -> Result<(), ObservationStoreError> {
    create_schema_v3(connection)?;
    ensure_gap_reason_column(connection)?;
    connection
        .execute_batch(CONVERSATION_CONTENT_SCHEMA_V4)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

fn migrate_v3_to_v4(connection: &mut Connection) -> Result<(), ObservationStoreError> {
    let transaction = connection
        .transaction()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    retire_empty_legacy_content_contract(&transaction)?;
    ensure_gap_reason_column(&transaction)?;
    transaction
        .execute_batch(CONVERSATION_CONTENT_SCHEMA_V4)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .execute(
            "UPDATE observation_meta SET value='4' WHERE key='schema_version'",
            [],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .commit()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    create_schema_v4(connection)
}

fn retire_empty_legacy_content_contract(
    transaction: &Transaction<'_>,
) -> Result<(), ObservationStoreError> {
    const LEGACY_TABLES: &[&str] = &[
        "content_streams",
        "content_blobs",
        "message_instances",
        "transcript_roots",
    ];
    for table in LEGACY_TABLES {
        if table_exists(transaction, table)?
            && transaction
                .query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {table} LIMIT 1)"),
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| ObservationStoreError::Corrupt)?
        {
            // The fake per-content contract has no direction/fork stream, Gateway digest, or
            // rich-feedback coordinates. Rewriting populated rows would invent frozen v2 facts;
            // keep the v3 database untouched and require an explicit data disposition instead.
            return Err(ObservationStoreError::Corrupt);
        }
    }
    for table in [
        "observation_events",
        "observation_checkpoints",
        "observation_gaps",
    ] {
        if table_exists(transaction, table)?
            && transaction
                .query_row(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM {table} WHERE channel='content' LIMIT 1)"
                    ),
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| ObservationStoreError::Corrupt)?
        {
            return Err(ObservationStoreError::Corrupt);
        }
    }
    transaction
        .execute_batch(
            "DROP TABLE IF EXISTS content_streams;
             DROP TABLE IF EXISTS content_blobs;
             DROP TABLE IF EXISTS message_instances;
             DROP TABLE IF EXISTS transcript_roots;",
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)
}

fn table_exists(transaction: &Transaction<'_>, table: &str) -> Result<bool, ObservationStoreError> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|_| ObservationStoreError::Corrupt)
}

fn ensure_gap_reason_column(connection: &Connection) -> Result<(), ObservationStoreError> {
    let has_reason = {
        let mut statement = connection
            .prepare("PRAGMA table_info(observation_gaps)")
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let columns = columns
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        columns.iter().any(|column| column == "reason")
    };
    if !has_reason {
        connection
            .execute("ALTER TABLE observation_gaps ADD COLUMN reason TEXT", [])
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> Result<(), ObservationStoreError> {
    let transaction = connection
        .transaction()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .execute_batch(EXECUTION_FACT_SCHEMA_V3)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .execute(
            "UPDATE observation_meta SET value='3' WHERE key='schema_version'",
            [],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .commit()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    create_schema_v3(connection)
}

fn migrate_v1_to_v2(connection: &mut Connection) -> Result<(), ObservationStoreError> {
    let transaction = connection
        .transaction()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .execute_batch(
            r#"
            ALTER TABLE value_ledger_entries RENAME TO value_ledger_entries_v1;
            ALTER TABLE daily_value_rollups RENAME TO daily_value_rollups_v1;
            "#,
        )
        .map_err(|_| ObservationStoreError::Corrupt)?;
    transaction
        .execute_batch(VALUE_SCHEMA_V2)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;

    let legacy_entries = {
        let mut statement = transaction
            .prepare(
                "SELECT body_json FROM value_ledger_entries_v1 ORDER BY workspace_id, request_id",
            )
            .map_err(|_| ObservationStoreError::Corrupt)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| ObservationStoreError::Corrupt)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationStoreError::Corrupt)?
    };
    for body in legacy_entries {
        let entry = migrate_legacy_entry(&body)?;
        insert_migrated_entry(&transaction, &entry)?;
    }

    // V1 rollups irreversibly mixed plans and currencies and have no per-session provenance.
    // Keeping or guessing those axes would expose a false authoritative ledger and would make
    // exact session deletion impossible. Detail entries above retain every attributable fact;
    // only the already-conflated derived rows are retired during the upgrade.
    transaction
        .execute_batch(
            r#"
            DROP TABLE value_ledger_entries_v1;
            DROP TABLE daily_value_rollups_v1;
            CREATE INDEX IF NOT EXISTS value_time
                ON value_ledger_entries(workspace_id, agent_plan_id, currency, frozen_at_ms);
            UPDATE observation_meta SET value='2' WHERE key='schema_version';
            "#,
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    transaction
        .commit()
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    create_schema_v2(connection)
}

fn migrate_legacy_entry(body: &str) -> Result<ValueLedgerEntryV1, ObservationStoreError> {
    let mut body: serde_json::Value =
        serde_json::from_str(body).map_err(|_| ObservationStoreError::Corrupt)?;
    let frozen = body
        .get_mut("frozen")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(ObservationStoreError::Corrupt)?;
    let chosen = take_legacy_u64(frozen, "api_equivalent_micros")?;
    let actual = take_legacy_u64(frozen, "incremental_cost_micros")?;
    let legacy_savings = take_legacy_u64(frozen, "savings_micros")?;
    let entitlement = signed_difference(chosen, actual)?;
    if i64::try_from(legacy_savings).map_err(|_| ObservationStoreError::Corrupt)? != entitlement {
        return Err(ObservationStoreError::Corrupt);
    }
    frozen.insert(
        "agent_plan_id".to_owned(),
        serde_json::Value::String(LEGACY_AGENT_PLAN_ID.to_owned()),
    );
    frozen.insert(
        "baseline_api_equivalent_cost_micros".to_owned(),
        serde_json::Value::Null,
    );
    frozen.insert(
        "chosen_api_equivalent_cost_micros".to_owned(),
        serde_json::Value::from(chosen),
    );
    frozen.insert(
        "actual_incremental_cost_micros".to_owned(),
        serde_json::Value::from(actual),
    );
    frozen.insert("routing_savings_micros".to_owned(), serde_json::Value::Null);
    frozen.insert(
        "entitlement_savings_micros".to_owned(),
        serde_json::Value::from(entitlement),
    );
    frozen.insert(
        "estimated_total_savings_micros".to_owned(),
        serde_json::Value::Null,
    );
    let entry: ValueLedgerEntryV1 =
        serde_json::from_value(body).map_err(|_| ObservationStoreError::Corrupt)?;
    entry
        .validate()
        .map_err(|_| ObservationStoreError::Corrupt)?;
    Ok(entry)
}

fn take_legacy_u64(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u64, ObservationStoreError> {
    object
        .remove(key)
        .and_then(|value| value.as_u64())
        .ok_or(ObservationStoreError::Corrupt)
}

fn signed_difference(left: u64, right: u64) -> Result<i64, ObservationStoreError> {
    let left = i64::try_from(left).map_err(|_| ObservationStoreError::Corrupt)?;
    let right = i64::try_from(right).map_err(|_| ObservationStoreError::Corrupt)?;
    left.checked_sub(right)
        .ok_or(ObservationStoreError::Corrupt)
}

fn insert_migrated_entry(
    transaction: &Transaction<'_>,
    entry: &ValueLedgerEntryV1,
) -> Result<(), ObservationStoreError> {
    let body = serde_json::to_string(entry).map_err(|_| ObservationStoreError::Corrupt)?;
    let digest = CanonicalDigest::of(entry).map_err(|_| ObservationStoreError::Corrupt)?;
    let frozen = &entry.frozen;
    transaction
        .execute(
            "INSERT INTO value_ledger_entries
             (workspace_id, request_id, session_id, receipt_id, body_json, body_digest,
              agent_plan_id, currency, billing_unit, price_version, price_override_revision,
              input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
              baseline_api_equivalent_cost_micros, chosen_api_equivalent_cost_micros,
              actual_incremental_cost_micros, routing_savings_micros,
              entitlement_savings_micros, estimated_total_savings_micros,
              facts_completeness, frozen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                entry.workspace_id.as_str(),
                entry.request_id.as_str(),
                entry.session_id.as_str(),
                entry.receipt_id.as_str(),
                body,
                digest.as_str(),
                frozen.agent_plan_id.as_str(),
                frozen.currency,
                frozen.billing_unit,
                frozen.price_version,
                frozen.price_override_revision,
                to_sql_u64(entry.usage.input_tokens)?,
                to_sql_u64(entry.usage.output_tokens)?,
                to_sql_u64(entry.usage.cache_read_tokens)?,
                to_sql_u64(entry.usage.cache_write_tokens)?,
                to_sql_u64(entry.usage.reasoning_tokens)?,
                optional_sql_u64(frozen.baseline_api_equivalent_cost_micros)?,
                optional_sql_u64(frozen.chosen_api_equivalent_cost_micros)?,
                optional_sql_u64(frozen.actual_incremental_cost_micros)?,
                frozen.routing_savings_micros,
                frozen.entitlement_savings_micros,
                frozen.estimated_total_savings_micros,
                enum_name(entry.facts_completeness)?,
                entry.frozen_at_ms,
            ],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

fn to_sql_u64(value: u64) -> Result<i64, ObservationStoreError> {
    value.try_into().map_err(|_| ObservationStoreError::Corrupt)
}

fn optional_sql_u64(value: Option<u64>) -> Result<Option<i64>, ObservationStoreError> {
    value.map(to_sql_u64).transpose()
}

fn enum_name<T: serde::Serialize>(value: T) -> Result<String, ObservationStoreError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(ObservationStoreError::Corrupt)
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
    CREATE TABLE IF NOT EXISTS observation_events (
        channel TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        payload_digest TEXT NOT NULL,
        PRIMARY KEY(channel, producer_id, producer_epoch, stream_id, sequence),
        UNIQUE(channel, producer_id, producer_epoch, stream_id, event_id)
    );

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

    CREATE TABLE IF NOT EXISTS conversation_content_events_v2 (
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        workspace_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        phase TEXT NOT NULL,
        metadata_json TEXT NOT NULL,
        canonical_chunk_object_ref TEXT,
        canonical_chunk_byte_offset INTEGER,
        canonical_chunk_byte_count INTEGER,
        envelope_digest TEXT NOT NULL,
        PRIMARY KEY(producer_id, producer_epoch, stream_id, sequence),
        UNIQUE(producer_id, producer_epoch, stream_id, event_id)
    );
    CREATE INDEX IF NOT EXISTS content_events_v2_request
        ON conversation_content_events_v2(workspace_id, request_id, direction, fork_id, sequence);

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
        expected_byte_count INTEGER,
        has_content_ref INTEGER NOT NULL,
        transport_frame_id TEXT,
        downstream_delivery TEXT,
        accumulated_byte_count INTEGER NOT NULL,
        state TEXT NOT NULL,
        created_at_unix_nanos INTEGER NOT NULL,
        PRIMARY KEY(workspace_id, content_id)
    );
    CREATE INDEX IF NOT EXISTS content_instances_v2_request
        ON content_instances_v2(workspace_id, request_id, direction, fork_id, part_ordinal);

    CREATE INDEX IF NOT EXISTS content_instances_v2_live_blob
        ON content_instances_v2(workspace_id, content_blob_digest) WHERE state='complete';

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

    CREATE TABLE IF NOT EXISTS request_content_refs_v2 (
        workspace_id TEXT NOT NULL,
        request_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        fork_id TEXT NOT NULL,
        message_instance_id TEXT NOT NULL,
        content_id TEXT NOT NULL,
        message_ordinal INTEGER NOT NULL,
        part_ordinal INTEGER NOT NULL,
        blob_digest TEXT NOT NULL,
        PRIMARY KEY(workspace_id, request_id, direction, fork_id, message_ordinal, part_ordinal)
    );

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

    CREATE TABLE IF NOT EXISTS observation_feedback_v2 (
        feedback_id INTEGER PRIMARY KEY AUTOINCREMENT,
        channel TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        rejected_or_acked_sequence INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        envelope_digest TEXT NOT NULL,
        feedback_kind TEXT NOT NULL,
        feedback_json TEXT NOT NULL,
        created_at_unix_nanos INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS observation_feedback_v2_lookup
        ON observation_feedback_v2(channel, producer_id, producer_epoch, stream_id,
                                   rejected_or_acked_sequence, event_id, envelope_digest,
                                   feedback_id DESC);

    CREATE TABLE IF NOT EXISTS observation_gap_heartbeats_v1 (
        channel TEXT NOT NULL,
        producer_id TEXT NOT NULL,
        producer_epoch TEXT NOT NULL,
        stream_id TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        event_id TEXT NOT NULL,
        heartbeat_json TEXT NOT NULL,
        PRIMARY KEY(channel, producer_id, producer_epoch, stream_id, sequence),
        UNIQUE(channel, producer_id, producer_epoch, stream_id, event_id)
    );
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
