use hiroute_diagnostics::event::{
    CpaDiscoveryAttempt, CpaDiscoveryObservation, CpaDiscoveryOutcome, CpaDiscoveryPhase,
    CpaDiscoveryUnavailable, CpaModelDiscovery,
};
use serde::Deserialize;
use serde_json::Value;

const MAX_OBSERVATIONS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: String,
    process_id: u32,
    evicted_accounts: u64,
    first_failure: Option<CpaDiscoveryAttempt>,
    latest: Option<CpaDiscoveryAttempt>,
    registration_pending: Option<bool>,
}

/// Dedupe transitions, ignoring the growing elapsed time of an entered phase.
/// Both retention and emitted records are bounded for this control operation.
#[derive(Default)]
pub(super) struct Observations {
    seen: Vec<CpaModelDiscovery>,
    truncated: bool,
}

impl Observations {
    pub fn read(&mut self, value: &Value, ordinal: Option<u32>) -> Vec<CpaModelDiscovery> {
        let observations = parse(value)
            .map(|(_, observations)| observations)
            .unwrap_or_else(|reason| vec![CpaDiscoveryObservation::Unavailable { reason }]);
        let mut result = Vec::new();
        for observation in observations {
            let event = CpaModelDiscovery {
                account_ordinal: ordinal,
                observation,
            };
            let mut key = event.clone();
            if let CpaDiscoveryObservation::Observed { attempt, .. } = &mut key.observation {
                attempt.elapsed_ms = 0;
                attempt.phase_elapsed_ms = 0;
                for timing in &mut attempt.timings {
                    timing.elapsed_ms = 0;
                }
            }
            if self.seen.contains(&key) {
                continue;
            }
            if self.seen.len() == MAX_OBSERVATIONS {
                if !self.truncated {
                    self.truncated = true;
                    result.push(CpaModelDiscovery {
                        account_ordinal: ordinal,
                        observation: CpaDiscoveryObservation::Unavailable {
                            reason: CpaDiscoveryUnavailable::ObservationLimit,
                        },
                    });
                }
                break;
            }
            self.seen.push(key);
            result.push(event);
        }
        result
    }
}

fn parse(
    value: &Value,
) -> Result<(Option<bool>, Vec<CpaDiscoveryObservation>), CpaDiscoveryUnavailable> {
    let raw = value
        .get("hiroute_model_discovery")
        .ok_or(CpaDiscoveryUnavailable::MetadataAbsent)?;
    let snapshot: Snapshot = serde_json::from_value(raw.clone())
        .map_err(|_| CpaDiscoveryUnavailable::InvalidMetadata)?;
    if snapshot.schema != "hiroute.cpa-model-discovery/v1" || snapshot.process_id == 0 {
        return Err(CpaDiscoveryUnavailable::InvalidMetadata);
    }
    let mut result = Vec::new();
    for (retained_first_failure, attempt) in
        [(true, snapshot.first_failure), (false, snapshot.latest)]
    {
        let Some(attempt) = attempt else { continue };
        if !valid(&attempt)
            || (retained_first_failure
                && !matches!(
                    attempt.outcome,
                    CpaDiscoveryOutcome::Failed | CpaDiscoveryOutcome::Discarded
                ))
        {
            return Err(CpaDiscoveryUnavailable::InvalidMetadata);
        }
        result.push(CpaDiscoveryObservation::Observed {
            process_id: snapshot.process_id,
            evicted_accounts: snapshot.evicted_accounts,
            retained_first_failure,
            attempt,
        });
    }
    if result.is_empty() {
        return Err(CpaDiscoveryUnavailable::NoAttempt);
    }
    Ok((snapshot.registration_pending, result))
}

fn valid(attempt: &CpaDiscoveryAttempt) -> bool {
    attempt.attempt_id != 0
        && attempt.timings.len() <= 6
        && [attempt.http_status, attempt.proxy_status]
            .into_iter()
            .flatten()
            .all(|status| (100..=599).contains(&status))
        && (!matches!(
            attempt.outcome,
            CpaDiscoveryOutcome::Failed | CpaDiscoveryOutcome::Discarded
        ) || attempt.reason.is_some())
}

/// Only a stable catalog read with no registration in flight can end an empty wait.
/// Missing/malformed metadata and retained older failures cannot establish this.
pub(super) fn latest_is_terminal(value: &Value) -> bool {
    parse(value).is_ok_and(|(pending, observations)| {
        pending == Some(false) && observations.into_iter().any(|observation| {
            matches!(observation, CpaDiscoveryObservation::Observed {
                retained_first_failure: false, attempt, ..
            } if matches!(attempt.outcome, CpaDiscoveryOutcome::Failed | CpaDiscoveryOutcome::Discarded)
                || (attempt.phase == CpaDiscoveryPhase::Registration
                    && attempt.outcome == CpaDiscoveryOutcome::Completed
                    && attempt.model_count == Some(0)))
        })
    })
}

/// Only a quiescent, current transport failure can request saved-source recovery.
/// Authentication, malformed catalogs, revocation and incomplete evidence cannot.
pub(super) fn retryable_failure(value: &Value) -> Option<u32> {
    use hiroute_diagnostics::event::CpaDiscoveryReason;
    let (pending, observations) = parse(value).ok()?;
    if pending != Some(false) {
        return None;
    }
    observations.into_iter().find_map(|observation| {
        let CpaDiscoveryObservation::Observed {
            process_id,
            retained_first_failure: false,
            attempt,
            ..
        } = observation
        else {
            return None;
        };
        let transient = match attempt.reason {
            Some(
                CpaDiscoveryReason::Timeout
                | CpaDiscoveryReason::DnsFailure
                | CpaDiscoveryReason::ConnectFailure
                | CpaDiscoveryReason::BodyReadFailure,
            ) => true,
            Some(CpaDiscoveryReason::HttpRejected) => {
                matches!(attempt.http_status, Some(408 | 500 | 502 | 503 | 504))
            }
            Some(CpaDiscoveryReason::ProxyRejected) => {
                matches!(attempt.proxy_status, Some(408 | 500 | 502 | 503 | 504))
            }
            _ => false,
        };
        (attempt.outcome == CpaDiscoveryOutcome::Failed
            && matches!(
                attempt.phase,
                CpaDiscoveryPhase::Request | CpaDiscoveryPhase::Response
            )
            && transient)
            .then_some(process_id)
    })
}

#[cfg(test)]
mod tests;
