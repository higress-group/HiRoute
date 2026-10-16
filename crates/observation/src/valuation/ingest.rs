use super::ValuationError;
use hiroute_domain::{
    CanonicalDigest, ExecutionFactEnvelopeV1, ExecutionFactV1, UsageFrameKindV1, UsageProvenanceV1,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct RawUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub read: Option<u64>,
    pub write: Option<u64>,
    pub reasoning: Option<u64>,
}

pub(crate) fn ingest(
    transaction: &Transaction<'_>,
    envelope: &ExecutionFactEnvelopeV1,
    digest: &CanonicalDigest,
) -> Result<(), ValuationError> {
    if !matches!(
        envelope.fact,
        ExecutionFactV1::RouteDecision(_)
            | ExecutionFactV1::AttemptStarted { .. }
            | ExecutionFactV1::UsageAndCache { .. }
            | ExecutionFactV1::RequestFinished { .. }
    ) {
        return Ok(());
    }
    let workspace = envelope.correlation.workspace_id.as_str();
    let request = envelope.correlation.request_id.as_str();
    let previous:Option<String>=transaction.query_row("SELECT input_digest FROM valuation_requests_v2 WHERE workspace_id=?1 AND request_id=?2",params![workspace,request],|row|row.get(0)).optional()?;
    let next_digest =
        CanonicalDigest::of(&(previous, digest)).map_err(|_| ValuationError::InvalidEvidence)?;
    transaction.execute(
        "INSERT INTO valuation_requests_v2(workspace_id,request_id,session_id,plan_id,started_ms,input_revision,input_digest,partial)
         VALUES(?1,?2,?3,?4,?5,1,?6,?7) ON CONFLICT(workspace_id,request_id) DO UPDATE SET
            input_revision=input_revision+1,input_digest=excluded.input_digest,partial=MAX(partial,excluded.partial)",
        params![workspace,request,envelope.correlation.conversation_id.as_str(),envelope.trust.agent_plan_id.as_ref().map(|id| id.as_str()),
            envelope.occurred_at_ms().map_err(|_|ValuationError::InvalidEvidence)?,next_digest.as_str(),envelope.completeness_delta.is_some()],
    )?;
    match &envelope.fact {
        ExecutionFactV1::AttemptStarted { ordinal, .. } => {
            let pricing = envelope
                .pricing
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| ValuationError::InvalidEvidence)?;
            transaction.execute("INSERT OR IGNORE INTO valuation_attempt_inputs_v2(workspace_id,request_id,ordinal,pricing_json) VALUES(?1,?2,?3,?4)",
                params![workspace,request,ordinal,pricing])?;
        }
        ExecutionFactV1::UsageAndCache {
            ordinal,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            input_provenance,
            output_provenance,
            cache_read_provenance,
            cache_write_provenance,
            reasoning_provenance,
            ..
        } => {
            let reported = |value: Option<u64>, provenance: &UsageProvenanceV1| {
                if *provenance == UsageProvenanceV1::Reported {
                    value
                } else {
                    None
                }
            };
            let mut usage = RawUsage {
                input: reported(*input_tokens, input_provenance),
                output: reported(*output_tokens, output_provenance),
                read: reported(*cache_read_tokens, cache_read_provenance),
                write: reported(*cache_write_tokens, cache_write_provenance),
                reasoning: reported(*reasoning_tokens, reasoning_provenance),
            };
            let previous:Option<(Option<String>,Option<String>)>=transaction.query_row(
                "SELECT pricing_json,usage_json FROM valuation_attempt_inputs_v2 WHERE workspace_id=?1 AND request_id=?2 AND ordinal=?3",
                params![workspace,request,ordinal],|row|Ok((row.get(0)?,row.get(1)?)),
            ).optional()?;
            if let Some((pricing, old)) = previous {
                let pricing = pricing
                    .map(|json| {
                        serde_json::from_str::<hiroute_domain::ExecutionPricingEvidenceV1>(&json)
                    })
                    .transpose()
                    .map_err(|_| ValuationError::InvalidEvidence)?;
                if pricing.as_ref().is_some_and(|value| {
                    value.usage_semantics.frame_kind == UsageFrameKindV1::Delta
                }) && let Some(old) = old
                {
                    let old: RawUsage =
                        serde_json::from_str(&old).map_err(|_| ValuationError::InvalidEvidence)?;
                    let add = |a: Option<u64>, b: Option<u64>| {
                        a.zip(b).and_then(|(a, b)| a.checked_add(b))
                    };
                    usage = RawUsage {
                        input: add(old.input, usage.input),
                        output: add(old.output, usage.output),
                        read: add(old.read, usage.read),
                        write: add(old.write, usage.write),
                        reasoning: add(old.reasoning, usage.reasoning),
                    };
                }
            }
            let json =
                serde_json::to_string(&usage).map_err(|_| ValuationError::InvalidEvidence)?;
            transaction.execute("INSERT INTO valuation_attempt_inputs_v2(workspace_id,request_id,ordinal,usage_json) VALUES(?1,?2,?3,?4)
                ON CONFLICT(workspace_id,request_id,ordinal) DO UPDATE SET usage_json=excluded.usage_json",params![workspace,request,ordinal,json])?;
        }
        ExecutionFactV1::RequestFinished {
            accepted_attempt_ordinal,
            facts_completeness,
            ..
        } => {
            transaction.execute("UPDATE valuation_requests_v2 SET terminal=1,accepted_ordinal=?3,partial=MAX(partial,?4) WHERE workspace_id=?1 AND request_id=?2",
                params![workspace,request,accepted_attempt_ordinal,*facts_completeness!=hiroute_domain::FactsCompleteness::Complete])?;
        }
        _ => {}
    }
    transaction.execute(
        "INSERT OR IGNORE INTO valuation_pending_v2(workspace_id,request_id) VALUES(?1,?2)",
        params![workspace, request],
    )?;
    Ok(())
}
