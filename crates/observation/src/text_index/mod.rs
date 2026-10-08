//! Rebuildable content index. Incomplete or digest-invalid builds never publish
//! hits. The original content store and its visibility remain authoritative.
mod block;
mod worker;
pub(crate) use block::{fold, original_offset};
use rusqlite::Connection;
pub use worker::TextIndexBuilder;

pub(crate) fn migrate(
    transaction: &Connection,
) -> Result<(), crate::writer::ObservationStoreError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS observation_text_index_v2(
            workspace TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,published INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(workspace,digest));
         CREATE TABLE IF NOT EXISTS observation_text_blocks_v2(
            workspace TEXT NOT NULL,digest TEXT NOT NULL,ordinal INTEGER NOT NULL,
            original_start INTEGER NOT NULL,primary_start INTEGER NOT NULL,
            folded TEXT NOT NULL,offsets BLOB NOT NULL,original TEXT NOT NULL,
            UNIQUE(workspace,digest,ordinal));
         CREATE INDEX IF NOT EXISTS observation_text_blocks_source ON observation_text_blocks_v2(workspace,digest,ordinal);
         CREATE INDEX IF NOT EXISTS observation_text_index_published ON observation_text_index_v2(published);"
    ).map_err(|_| crate::writer::ObservationStoreError::ActivityUnavailable)?;
    let has_original: bool = {
        let mut statement = transaction
            .prepare("PRAGMA table_info(observation_text_blocks_v2)")
            .map_err(|_| crate::writer::ObservationStoreError::ActivityUnavailable)?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| crate::writer::ObservationStoreError::ActivityUnavailable)?;
        names
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| crate::writer::ObservationStoreError::ActivityUnavailable)?
            .iter()
            .any(|name| name == "original")
    };
    if !has_original {
        transaction.execute_batch("ALTER TABLE observation_text_blocks_v2 ADD COLUMN original TEXT NOT NULL DEFAULT ''; DELETE FROM observation_text_blocks_v2; DELETE FROM observation_text_index_v2; UPDATE observation_meta SET value=CAST(value AS INTEGER)+1 WHERE key='query_visibility_generation';").map_err(|_|crate::writer::ObservationStoreError::ActivityUnavailable)?;
    }
    Ok(())
}
