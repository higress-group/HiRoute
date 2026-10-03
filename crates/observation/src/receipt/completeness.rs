//! Incremental projection over the append-only fact log. Gap repair and retention
//! can remove evidence, so those exceptional paths still rebuild from surviving facts.
use super::*;

pub(super) fn append(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), FactProjectionError> {
    let workspace = envelope.correlation.workspace_id.as_str();
    let session = envelope.correlation.conversation_id.as_str();
    let (previous, has_prior, gap): (String, bool, bool) = transaction.query_row(
        "SELECT facts_completeness,
          EXISTS(SELECT 1 FROM execution_fact_events WHERE workspace_id=?1 AND session_id=?2
            AND NOT (producer_id=?3 AND producer_epoch=?4 AND stream_id=?5 AND sequence=?6)),
          EXISTS(SELECT 1 FROM observation_gaps WHERE channel='fact' AND workspace_id=?1 AND session_id=?2)
         FROM sessions WHERE workspace_id=?1 AND session_id=?2",
        params![workspace, session, envelope.producer.stream.producer_id.as_str(),
            envelope.producer.stream.producer_epoch.as_str(), envelope.producer.stream.stream_id.as_str(),
            i64::try_from(envelope.sequence).map_err(|_| FactProjectionError::Invalid)?],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| FactProjectionError::Storage)?;
    let current = completeness_from_facts(std::slice::from_ref(envelope));
    // A content/gap-created placeholder or a fully retired session has no prior
    // fact contribution. Its initial "unknown" must not taint the first fact.
    let result = if gap {
        FactsCompleteness::Partial
    } else if !has_prior {
        current
    } else {
        let previous = match previous.as_str() {
            "complete" => FactsCompleteness::Complete,
            "partial" => FactsCompleteness::Partial,
            "unknown" => FactsCompleteness::Unknown,
            _ => return Err(FactProjectionError::Corrupt),
        };
        least_complete(previous, current)
    };
    transaction
        .execute(
            "UPDATE sessions SET facts_completeness=?3 WHERE workspace_id=?1 AND session_id=?2",
            params![workspace, session, facts_completeness_str(result)],
        )
        .map_err(|_| FactProjectionError::Storage)?;
    Ok(())
}
