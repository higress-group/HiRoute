//! Decision spending follows the real exchange, independently of answer parsing
//! and optional observation delivery.
use super::*;
use crate::ports::KeySpendGuard;
use hiroute_domain::{KeySpendEstimateV1, KeySpendSettlementV1, KeySpendUsageV1};
use std::collections::BTreeMap;

pub(super) fn input_bound(prepared: &PreparedDecisionRequest) -> u64 {
    let questions = prepared
        .bindings
        .as_ref()
        .map_or(1, |b| {
            usize::from(b.choice.is_some())
                + b.degrees.len()
                + usize::from(b.assessment.is_some())
                + usize::from(b.applicability.is_some())
        })
        .max(1);
    // Escaped wire bytes conservatively cover textual tokens. Reserve shared
    // input once per question as an upper bound; settle the actual provider total.
    (prepared.template.wire_len as u64).saturating_mul(questions as u64)
}

pub(super) fn failure(kind: FailureKind) -> CallFailure {
    CallFailure::Http(DecisionFailureV1 {
        kind,
        http_status: None,
    })
}

pub(super) fn reserve<'a>(
    lease: &CredentialLease,
    estimates: impl FnOnce() -> &'a BTreeMap<String, KeySpendEstimateV1>,
) -> Result<Option<KeySpendGuard>, CallFailure> {
    let currency = lease
        .spending_currency()
        .map_err(|_| failure(FailureKind::BudgetUnavailable))?;
    let estimate = currency
        .as_ref()
        .and_then(|currency| estimates().get(currency));
    lease
        .reserve_spending(estimate)
        .or_else(|error| match error {
            hiroute_domain::KeyBudgetGateErrorV1::Exhausted => {
                Err(failure(FailureKind::BudgetExhausted))
            }
            hiroute_domain::KeyBudgetGateErrorV1::Authority(_) => {
                Err(failure(FailureKind::BudgetUnavailable))
            }
            // Authorities predating unpriced ledger support may still report these
            // optional-price failures. Price uncertainty must not reject a request.
            hiroute_domain::KeyBudgetGateErrorV1::PriceUnavailable
            | hiroute_domain::KeyBudgetGateErrorV1::CurrencyMismatch => Ok(None),
        })
}

pub(super) fn unsent<T: hiroute_gateway_core::runtime::attempt::AttemptTransport>(
    guard: Option<&KeySpendGuard>,
    exchange: &AttemptExchange<T>,
) {
    if exchange.snapshot().upstream_request_fence
        == hiroute_gateway_core::runtime::attempt::CommitFence::Clear
        && let Some(guard) = guard
    {
        guard.settle(KeySpendSettlementV1::Unsent);
    }
}

pub(super) fn response(
    guard: Option<&KeySpendGuard>,
    body: &[u8],
    protocol: hiroute_domain::DecisionWireProtocolV2,
) {
    let Some(guard) = guard else { return };
    let settlement = usage(body, protocol).map_or(KeySpendSettlementV1::Unknown, |usage| {
        KeySpendSettlementV1::Usage { usage }
    });
    guard.settle(settlement);
}

fn usage(body: &[u8], protocol: hiroute_domain::DecisionWireProtocolV2) -> Option<KeySpendUsageV1> {
    if protocol != hiroute_domain::DecisionWireProtocolV2::OpenaiDecisions {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct Envelope {
        usage: Usage,
    }
    #[derive(serde::Deserialize)]
    struct Usage {
        input_tokens: u64,
        output_tokens: Option<u64>,
        input_tokens_details: Option<InputDetails>,
        output_tokens_details: Option<OutputDetails>,
    }
    #[derive(serde::Deserialize)]
    struct InputDetails {
        cached_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
    }
    #[derive(serde::Deserialize)]
    struct OutputDetails {
        reasoning_tokens: Option<u64>,
    }
    let Envelope { usage } = serde_json::from_slice::<Envelope>(body).ok()?;
    let (read, write) = usage.input_tokens_details.map_or((None, None), |details| {
        (details.cached_tokens, details.cache_write_tokens)
    });
    if read
        .unwrap_or(0)
        .checked_add(write.unwrap_or(0))
        .is_none_or(|cached| cached > usage.input_tokens)
    {
        return None;
    }
    Some(KeySpendUsageV1 {
        input_tokens: Some(usage.input_tokens),
        output_tokens: usage.output_tokens,
        cache_read_tokens: read,
        cache_write_tokens: write,
        reasoning_tokens: usage
            .output_tokens_details
            .and_then(|details| details.reasoning_tokens),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::DecisionWireProtocolV2::OpenaiDecisions;
    #[test]
    fn usage_is_independent_of_answers_and_keeps_missing_or_invalid_counts_unknown() {
        let body = br#"{"answers":"malformed","usage":{"input_tokens":100,"output_tokens":0,"input_tokens_details":{"cached_tokens":30,"cache_write_tokens":20}}}"#;
        let parsed = usage(body, OpenaiDecisions).unwrap();
        assert_eq!(
            (
                parsed.input_tokens,
                parsed.cache_read_tokens,
                parsed.cache_write_tokens
            ),
            (Some(100), Some(30), Some(20))
        );
        let partial = usage(br#"{"usage":{"input_tokens":100}}"#, OpenaiDecisions).unwrap();
        assert_eq!(
            (partial.output_tokens, partial.cache_read_tokens),
            (None, None)
        );
        for body in [br#"{"usage":{"input_tokens":-1}}"#.as_slice(),
            br#"{"usage":{"input_tokens":5,"input_tokens_details":{"cached_tokens":4,"cache_write_tokens":2}}}"#,
            br#"{"answers":[]}"#] {
            assert!(usage(body, OpenaiDecisions).is_none());
        }
    }
}
