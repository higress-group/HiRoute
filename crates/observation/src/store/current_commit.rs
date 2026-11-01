//! The last committed identity resolves a lost reply without an event/reply archive.
use hiroute_domain::ObservationStreamV1;
use rusqlite::{Connection, OptionalExtension, params};

use super::cached_sql;
use crate::writer::ObservationStoreError as Error;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS observation_current_commits_v6(
 channel TEXT NOT NULL,producer_id TEXT NOT NULL,producer_epoch TEXT NOT NULL,stream_id TEXT NOT NULL,
 sequence INTEGER NOT NULL,event_id TEXT NOT NULL,payload_digest TEXT NOT NULL,
 PRIMARY KEY(channel,producer_id,producer_epoch,stream_id));";

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Commit {
    pub(super) sequence: u64,
    pub(super) event_id: String,
    pub(super) digest: String,
}

pub(super) fn channel(value: &str) -> Result<&'static str, Error> {
    match value {
        "fact" | "execution_fact" => Ok("fact"),
        "content" | "conversation_content" => Ok("content"),
        _ => Err(Error::Corrupt),
    }
}

pub(super) fn load(
    db: &Connection,
    kind: &str,
    stream: &ObservationStreamV1,
) -> Result<Option<Commit>, Error> {
    let row: Option<(i64, String, String)> = cached_sql::query_row(
        db,
        "SELECT sequence,event_id,payload_digest FROM observation_current_commits_v6
         WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4",
        params![
            channel(kind)?,
            stream.producer_id.as_str(),
            stream.producer_epoch.as_str(),
            stream.stream_id.as_str()
        ],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(|_| Error::ActivityUnavailable)?;
    row.map(|(sequence, event_id, digest)| {
        Ok(Commit {
            sequence: sequence.try_into().map_err(|_| Error::Corrupt)?,
            event_id,
            digest,
        })
    })
    .transpose()
}

pub(super) fn save(
    db: &Connection,
    kind: &str,
    stream: &ObservationStreamV1,
    sequence: u64,
    event_id: &str,
    digest: &str,
) -> Result<(), Error> {
    let sequence: i64 = sequence.try_into().map_err(|_| Error::Corrupt)?;
    let changed = cached_sql::execute(
        db,
        "INSERT INTO observation_current_commits_v6
         (channel,producer_id,producer_epoch,stream_id,sequence,event_id,payload_digest)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(channel,producer_id,producer_epoch,stream_id) DO UPDATE SET
           sequence=excluded.sequence,event_id=excluded.event_id,payload_digest=excluded.payload_digest
         WHERE excluded.sequence>observation_current_commits_v6.sequence
           OR (excluded.sequence=observation_current_commits_v6.sequence
               AND excluded.event_id=observation_current_commits_v6.event_id
               AND excluded.payload_digest=observation_current_commits_v6.payload_digest)",
        params![channel(kind)?,stream.producer_id.as_str(),stream.producer_epoch.as_str(),
            stream.stream_id.as_str(),sequence,event_id,digest],
    ).map_err(|_| Error::ActivityUnavailable)?;
    if changed != 1 {
        return Err(Error::Corrupt);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream() -> ObservationStreamV1 {
        ObservationStreamV1 {
            producer_id: hiroute_domain::ProducerId::parse("producer").unwrap(),
            producer_epoch: hiroute_domain::ProducerEpoch::parse("epoch").unwrap(),
            stream_id: hiroute_domain::StreamId::parse("stream").unwrap(),
        }
    }

    #[test]
    fn current_commit_is_bounded_and_cannot_be_replaced_by_a_conflicting_retry() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let stream = stream();
        for sequence in 1..=10_001 {
            save(
                &db,
                "conversation_content",
                &stream,
                sequence,
                &format!("event-{sequence}"),
                "digest",
            )
            .unwrap();
        }
        let head = load(&db, "content", &stream).unwrap().unwrap();
        assert_eq!(head.sequence, 10_001);
        assert_eq!(head.event_id, "event-10001");
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM observation_current_commits_v6",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        save(&db, "content", &stream, 10_001, "event-10001", "digest").unwrap();
        assert!(save(&db, "content", &stream, 10_001, "event-10001", "changed").is_err());
        assert!(save(&db, "content", &stream, 10_000, "event-10000", "digest").is_err());
        assert_eq!(load(&db, "content", &stream).unwrap(), Some(head));
    }

    #[test]
    fn rolled_back_commit_is_not_a_proof_and_channels_remain_independent() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let stream = stream();
        {
            let tx = db.transaction().unwrap();
            save(&tx, "execution_fact", &stream, 1, "fact", "digest").unwrap();
        }
        assert!(load(&db, "fact", &stream).unwrap().is_none());
        save(&db, "execution_fact", &stream, 1, "fact", "fact-digest").unwrap();
        save(&db, "content", &stream, 2, "content", "content-digest").unwrap();
        assert_eq!(
            load(&db, "fact", &stream).unwrap().unwrap().event_id,
            "fact"
        );
        assert_eq!(
            load(&db, "content", &stream).unwrap().unwrap().event_id,
            "content"
        );
    }
}
