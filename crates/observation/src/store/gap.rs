use std::collections::BTreeSet;

use hiroute_domain::{LossNoticeV1, ObservationChannel, ObservationStreamV1};
use rusqlite::{Transaction, params};

use crate::writer::ObservationStoreError;

pub(super) fn record(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    loss: &LossNoticeV1,
    known_loss: bool,
    reason: Option<&str>,
) -> Result<(), ObservationStoreError> {
    transaction
        .execute(
            "INSERT INTO observation_gaps
             (channel, producer_id, producer_epoch, stream_id, first_sequence, last_sequence,
              known_loss, workspace_id, session_id, reason)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
             WHERE NOT EXISTS (
               SELECT 1 FROM observation_gaps WHERE channel=?1 AND producer_id=?2
                 AND producer_epoch=?3 AND stream_id=?4 AND first_sequence=?5
                 AND last_sequence=?6 AND known_loss=?7 AND workspace_id=?8
                 AND COALESCE(session_id, '')=COALESCE(?9, '')
             )",
            params![
                channel.as_str(),
                stream.producer_id.as_str(),
                stream.producer_epoch.as_str(),
                stream.stream_id.as_str(),
                as_i64(loss.range.first)?,
                as_i64(loss.range.last)?,
                known_loss,
                loss.scope.workspace_id.as_str(),
                loss.scope.session_id.as_ref().map(|value| value.as_str()),
                reason,
            ],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    if let Some(reason) = reason {
        transaction
            .execute(
                "UPDATE observation_gaps SET reason=COALESCE(reason, ?10)
                 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4
                   AND first_sequence=?5 AND last_sequence=?6 AND known_loss=?7
                   AND workspace_id=?8 AND COALESCE(session_id, '')=COALESCE(?9, '')",
                params![
                    channel.as_str(),
                    stream.producer_id.as_str(),
                    stream.producer_epoch.as_str(),
                    stream.stream_id.as_str(),
                    as_i64(loss.range.first)?,
                    as_i64(loss.range.last)?,
                    known_loss,
                    loss.scope.workspace_id.as_str(),
                    loss.scope.session_id.as_ref().map(|value| value.as_str()),
                    reason,
                ],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    }
    Ok(())
}

pub(super) fn mark_session_partial(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    loss: &LossNoticeV1,
) -> Result<(), ObservationStoreError> {
    let Some(session_id) = &loss.scope.session_id else {
        return Ok(());
    };
    transaction
        .execute(
            "INSERT OR IGNORE INTO sessions
             (workspace_id, session_id, agent_id, correlation, started_at_ms, updated_at_ms,
              facts_completeness, content_completeness)
             VALUES (?1, ?2, '', 'unproven', 0, 0, 'unknown', 'unknown')",
            params![loss.scope.workspace_id.as_str(), session_id.as_str()],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    let column = match channel {
        ObservationChannel::Fact => "facts_completeness",
        ObservationChannel::Content => "content_completeness",
    };
    let sql =
        format!("UPDATE sessions SET {column}='partial' WHERE workspace_id=?1 AND session_id=?2");
    transaction
        .execute(
            &sql,
            params![loss.scope.workspace_id.as_str(), session_id.as_str()],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

pub(super) fn resolve_sequence(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    sequence: u64,
) -> Result<(), ObservationStoreError> {
    let sequence = as_i64(sequence)?;
    let rows = {
        let mut statement = transaction
            .prepare(
                "SELECT gap_id, first_sequence, last_sequence, known_loss, workspace_id,
                        session_id, reason FROM observation_gaps
                 WHERE channel=?1 AND producer_id=?2 AND producer_epoch=?3 AND stream_id=?4
                   AND first_sequence<=?5 AND last_sequence>=?5",
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        statement
            .query_map(
                params![
                    channel.as_str(),
                    stream.producer_id.as_str(),
                    stream.producer_epoch.as_str(),
                    stream.stream_id.as_str(),
                    sequence,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ObservationStoreError::Corrupt)?
    };
    let mut scopes = BTreeSet::new();
    for (gap_id, first, last, known, workspace, session, reason) in rows {
        transaction
            .execute("DELETE FROM observation_gaps WHERE gap_id=?1", [gap_id])
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        if first < sequence {
            insert_fragment(
                transaction,
                channel,
                stream,
                first,
                sequence - 1,
                known,
                &workspace,
                session.as_deref(),
                reason.as_deref(),
            )?;
        }
        if sequence < last {
            insert_fragment(
                transaction,
                channel,
                stream,
                sequence + 1,
                last,
                known,
                &workspace,
                session.as_deref(),
                reason.as_deref(),
            )?;
        }
        if let Some(session) = session {
            scopes.insert((workspace, session));
        }
    }
    for (workspace, session) in scopes {
        refresh_session_completeness(transaction, channel, &workspace, &session)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_fragment(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    stream: &ObservationStreamV1,
    first: i64,
    last: i64,
    known: bool,
    workspace: &str,
    session: Option<&str>,
    reason: Option<&str>,
) -> Result<(), ObservationStoreError> {
    transaction
        .execute(
            "INSERT INTO observation_gaps
             (channel, producer_id, producer_epoch, stream_id, first_sequence, last_sequence,
              known_loss, workspace_id, session_id, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                channel.as_str(),
                stream.producer_id.as_str(),
                stream.producer_epoch.as_str(),
                stream.stream_id.as_str(),
                first,
                last,
                known,
                workspace,
                session,
                reason,
            ],
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

fn refresh_session_completeness(
    transaction: &Transaction<'_>,
    channel: ObservationChannel,
    workspace: &str,
    session: &str,
) -> Result<(), ObservationStoreError> {
    let still_missing: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM observation_gaps
             WHERE channel=?1 AND workspace_id=?2 AND session_id=?3)",
            params![channel.as_str(), workspace, session],
            |row| row.get(0),
        )
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    if still_missing {
        return Ok(());
    }
    if channel == ObservationChannel::Fact {
        let workspace_id = hiroute_domain::WorkspaceId::parse(workspace)
            .map_err(|_| ObservationStoreError::Corrupt)?;
        let session_id = hiroute_domain::SessionId::parse(session)
            .map_err(|_| ObservationStoreError::Corrupt)?;
        let completeness =
            crate::receipt::session_facts_completeness(transaction, &workspace_id, &session_id)
                .map_err(|error| match error {
                    crate::receipt::FactProjectionError::Storage => {
                        ObservationStoreError::ActivityUnavailable
                    }
                    _ => ObservationStoreError::Corrupt,
                })?;
        let value = match completeness {
            hiroute_domain::FactsCompleteness::Complete => "complete",
            hiroute_domain::FactsCompleteness::Partial => "partial",
            hiroute_domain::FactsCompleteness::Unknown => "unknown",
        };
        transaction
            .execute(
                "UPDATE sessions SET facts_completeness=?3
                 WHERE workspace_id=?1 AND session_id=?2",
                params![workspace, session, value],
            )
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        return Ok(());
    }
    crate::content::completeness::refresh_session(transaction, workspace, session)
        .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, ObservationStoreError> {
    value.try_into().map_err(|_| ObservationStoreError::Corrupt)
}
