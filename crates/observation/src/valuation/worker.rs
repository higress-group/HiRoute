use super::{ingest::RawUsage, *};
use crate::LocalObservationStore;
use hiroute_domain::{
    ExecutionPricingEvidenceV1, InputUsageMeaningV1, OutputUsageMeaningV1, PriceValuationKindV1,
    UsageFrameKindV1,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type ValuationRequestMetadata = (u64, String, bool, bool, i64, Option<String>, Option<u64>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValuationAmountV2 {
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
    pub known_sum_micros: Option<u64>,
    pub coverage: MetricCoverage,
    pub missing_attempt_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValuationRecordV2 {
    pub algorithm: String,
    pub input_revision: u64,
    pub input_digest: String,
    pub supersedes: Option<u64>,
    pub terminal_observed: bool,
    pub facts_partial: bool,
    pub unknown_attempt_count: u64,
    pub attempts: Vec<AttemptValuationV2>,
    pub amounts: Vec<ValuationAmountV2>,
    #[serde(default)]
    pub reference_comparison: Option<ReferenceComparisonV2>,
}

impl LocalObservationStore {
    /// One maintenance worker calls this bounded cycle. Input and result updates
    /// use the same writer transaction; retries cannot double-add contributions.
    pub fn settle_pending_valuations(&self, limit: usize) -> Result<usize, ValuationError> {
        if limit == 0 || limit > 200 {
            return Err(ValuationError::InvalidInput);
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let pending = {
            let mut statement=transaction.prepare("SELECT workspace_id,request_id FROM valuation_pending_v2 ORDER BY workspace_id,request_id LIMIT ?1")?;
            statement
                .query_map([limit], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (workspace, request) in &pending {
            settle_one(&transaction, workspace, request)?;
            transaction.execute(
                "DELETE FROM valuation_pending_v2 WHERE workspace_id=?1 AND request_id=?2",
                params![workspace, request],
            )?;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|n| i64::try_from(n.as_millis()).ok())
            .unwrap_or(0);
        crate::analysis::sync(&transaction, 200, now_ms)?;
        transaction.commit()?;
        Ok(pending.len())
    }
}

pub(super) fn settle_one(
    transaction: &rusqlite::Transaction<'_>,
    workspace: &str,
    request: &str,
) -> Result<(), ValuationError> {
    let metadata:Option<ValuationRequestMetadata>=transaction.query_row(
        "SELECT input_revision,input_digest,terminal,partial,started_ms,plan_id,accepted_ordinal FROM valuation_requests_v2 WHERE workspace_id=?1 AND request_id=?2",
        params![workspace,request],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).optional()?;
    let Some((revision, digest, terminal, partial, started, plan, accepted_ordinal)) = metadata
    else {
        return Ok(());
    };
    let previous:Option<(u64,String)>=transaction.query_row(
        "SELECT revision,input_digest FROM valuation_records_v2 WHERE workspace_id=?1 AND request_id=?2 ORDER BY revision DESC LIMIT 1",
        params![workspace,request],|row|Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    if previous.as_ref().is_some_and(|(_, old)| old == &digest) {
        return Ok(());
    }
    let inputs = {
        let mut statement=transaction.prepare("SELECT pricing_json,usage_json,ordinal FROM valuation_attempt_inputs_v2 WHERE workspace_id=?1 AND request_id=?2 ORDER BY ordinal LIMIT 257")?;
        statement
            .query_map(params![workspace, request], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let truncated = inputs.len() > 256;
    let mut record = ValuationRecordV2 {
        algorithm: VALUATION_ALGORITHM.into(),
        input_revision: revision,
        input_digest: digest.clone(),
        supersedes: previous.map(|(revision, _)| revision),
        terminal_observed: terminal,
        facts_partial: partial || truncated,
        unknown_attempt_count: u64::from(truncated),
        attempts: vec![],
        amounts: vec![],
        reference_comparison: None,
    };
    let mut reference_input = None;
    for (pricing, usage, ordinal) in inputs.into_iter().take(256) {
        let pricing = match pricing
            .map(|json| serde_json::from_str::<ExecutionPricingEvidenceV1>(&json))
            .transpose()
        {
            Ok(value) => value,
            Err(_) => {
                record.unknown_attempt_count += 1;
                record.facts_partial = true;
                continue;
            }
        };
        let Some(pricing) = pricing else {
            record.unknown_attempt_count += 1;
            continue;
        };
        let Some(_) = &pricing.quote else {
            record.unknown_attempt_count += 1;
            continue;
        };
        let raw = match usage
            .map(|json| serde_json::from_str::<RawUsage>(&json))
            .transpose()
        {
            Ok(value) => value.unwrap_or_default(),
            Err(_) => {
                record.unknown_attempt_count += 1;
                record.facts_partial = true;
                continue;
            }
        };
        let usage = normalize(&pricing, &raw);
        if accepted_ordinal == Some(ordinal) {
            reference_input = pricing
                .reference_quote
                .clone()
                .map(|reference| (reference, usage.clone()));
        }
        match value_recorded_attempt(&pricing, &raw) {
            Ok(attempt) => record.attempts.push(attempt),
            Err(_) => {
                record.unknown_attempt_count += 1;
                record.facts_partial = true;
            }
        }
    }
    if record
        .attempts
        .iter()
        .any(|attempt| attempt.algorithm == EQUAL_RATE_VALUATION_ALGORITHM)
    {
        record.algorithm = EQUAL_RATE_VALUATION_ALGORITHM.into();
    }
    let mut groups: BTreeMap<(String, PriceValuationKindV1), ValuationAmountV2> = BTreeMap::new();
    let mut overflowed = std::collections::BTreeSet::new();
    for attempt in &record.attempts {
        let key = (attempt.currency.clone(), attempt.valuation_kind);
        let group = groups
            .entry((attempt.currency.clone(), attempt.valuation_kind))
            .or_insert_with(|| ValuationAmountV2 {
                currency: attempt.currency.clone(),
                valuation_kind: attempt.valuation_kind,
                known_sum_micros: None,
                coverage: MetricCoverage::Complete,
                missing_attempt_count: record.unknown_attempt_count,
            });
        if (attempt.algorithm == EQUAL_RATE_VALUATION_ALGORITHM
            && attempt.known_sum_micros.is_none())
            || attempt.components.iter().any(|component| {
                component.missing == Some(ValuationMissingReason::ArithmeticOverflow)
            })
        {
            overflowed.insert(key.clone());
        }
        if !overflowed.contains(&key)
            && let Some(amount) = attempt.known_sum_micros
        {
            group.known_sum_micros = group
                .known_sum_micros
                .unwrap_or(0)
                .checked_add(amount)
                .filter(|sum| *sum <= i64::MAX as u64);
            if group.known_sum_micros.is_none() {
                overflowed.insert(key.clone());
            }
        }
        if overflowed.contains(&key) {
            group.known_sum_micros = None;
        }
        if attempt.coverage != MetricCoverage::Complete {
            group.missing_attempt_count += 1;
        }
        if group.known_sum_micros.is_none() {
            group.coverage = MetricCoverage::Unknown;
        } else if group.missing_attempt_count > 0 || record.facts_partial || !terminal {
            group.coverage = MetricCoverage::Partial;
        }
    }
    record.amounts = groups.into_values().collect();
    if let Some((reference, usage)) = reference_input {
        record.reference_comparison = Some(compare_reference(
            &reference,
            &usage,
            &record.attempts,
            terminal && !record.facts_partial && record.unknown_attempt_count == 0,
        )?);
    }
    let body = serde_json::to_string(&record).map_err(|_| ValuationError::InvalidEvidence)?;
    transaction.execute("INSERT INTO valuation_records_v2(workspace_id,request_id,revision,input_digest,body_json) VALUES(?1,?2,?3,?4,?5)",
        params![workspace,request,revision,digest,body])?;
    // Replace active contributions in the same commit as the new revision.
    transaction.execute(
        "DELETE FROM valuation_contributions_v2 WHERE workspace_id=?1 AND request_id=?2",
        params![workspace, request],
    )?;
    for amount in &record.amounts {
        transaction.execute("INSERT INTO valuation_contributions_v2(workspace_id,request_id,day,plan_id,currency,valuation_kind,known_micros,coverage)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![workspace,request,started/86_400_000,plan,amount.currency,
                serde_json::to_string(&amount.valuation_kind).map_err(|_|ValuationError::InvalidEvidence)?,amount.known_sum_micros,
                serde_json::to_string(&amount.coverage).map_err(|_|ValuationError::InvalidEvidence)?])?;
    }
    Ok(())
}

pub(crate) fn normalize(pricing: &ExecutionPricingEvidenceV1, raw: &RawUsage) -> UsageBucketsV2 {
    normalize_semantics(pricing.usage_semantics, raw)
}
pub(crate) fn normalize_semantics(
    semantics: hiroute_domain::UsageSemanticsV1,
    raw: &RawUsage,
) -> UsageBucketsV2 {
    if semantics.frame_kind == UsageFrameKindV1::Unknown {
        return UsageBucketsV2::default();
    }
    let output = match semantics.output {
        OutputUsageMeaningV1::IncludesReasoning => raw.output,
        OutputUsageMeaningV1::ReasoningSeparatelyAtOutputRate => raw
            .output
            .zip(raw.reasoning)
            .and_then(|(a, b)| a.checked_add(b)),
        OutputUsageMeaningV1::Unknown => None,
    };
    let (read, write) = if semantics.cache_buckets_exclusive {
        (raw.read, raw.write)
    } else {
        (None, None)
    };
    match semantics.input {
        InputUsageMeaningV1::IncludesExclusiveCache => {
            UsageBucketsV2::from_inclusive_input(raw.input, output, read, write)
        }
        InputUsageMeaningV1::UncachedOnly => UsageBucketsV2 {
            input_uncached: raw.input,
            output,
            cache_read: read,
            cache_write: write,
        },
        InputUsageMeaningV1::Unknown => UsageBucketsV2 {
            output,
            ..UsageBucketsV2::default()
        },
    }
}
