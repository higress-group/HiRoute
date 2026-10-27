//! Gateway admission owns stage identity. Affinity and routing decisions do not.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(super) struct Admission<'a> {
    pub epoch: &'a str,
    pub value: &'a AgentTurnAdmissionV1,
}

pub(super) fn collect(facts: &[Fact]) -> Result<BTreeMap<&str, Admission<'_>>, ObservationV2Error> {
    let mut requests: BTreeMap<&str, Admission<'_>> = BTreeMap::new();
    let mut stages = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    for fact in facts {
        let ExecutionFactV1::AgentTurnAdmitted(value) = &fact.value else {
            continue;
        };
        let admission = Admission {
            epoch: &fact.producer_epoch,
            value,
        };
        if let Some(previous) = requests.insert(&fact.request_id, admission)
            && (previous.epoch != admission.epoch || previous.value != value)
        {
            return Err(ObservationV2Error::Unavailable);
        }
        let identity = (admission.epoch, value.agent_turn_id.as_str());
        let evidence = (&value.branch_id, value.ordinal, value.opening_cause);
        if let Some(previous) = stages.insert(identity, evidence)
            && previous != evidence
        {
            return Err(ObservationV2Error::Unavailable);
        }
        if let Some(previous) = revisions.insert((identity, value.revision), &fact.request_id)
            && previous != &fact.request_id
        {
            return Err(ObservationV2Error::Unavailable);
        }
    }
    Ok(requests)
}

pub(super) fn pending<'a>(
    ordered: &[&'a Request],
    covered: &BTreeSet<&str>,
    admissions: &BTreeMap<&str, Admission<'_>>,
) -> Vec<Vec<&'a Request>> {
    let mut groups = Vec::new();
    let mut bound = BTreeMap::new();
    let mut legacy = Vec::new();
    for request in ordered {
        let admission = admissions.get(request.view.request_id.as_str());
        if (covered.contains(request.view.request_id.as_str()) || admission.is_some())
            && !legacy.is_empty()
        {
            groups.push(std::mem::take(&mut legacy));
        }
        if covered.contains(request.view.request_id.as_str()) {
            continue;
        }
        if let Some(admission) = admission {
            let identity = (admission.epoch, admission.value.agent_turn_id.as_str());
            let index = *bound.entry(identity).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[index].push(*request);
        } else {
            // Historical bytes did not record admission identity. Preserve their
            // bounded legacy rule, never extending it across a recorded stage.
            let held = request.decision.as_ref().is_some_and(|d| {
                d.reason_ledger
                    .iter()
                    .any(|r| r.code == LedgerReasonCodeV1::ContextHoldApplied)
            });
            if !legacy.is_empty() && !held {
                groups.push(std::mem::take(&mut legacy));
            }
            legacy.push(*request);
        }
    }
    if !legacy.is_empty() {
        groups.push(legacy);
    }
    groups
}
