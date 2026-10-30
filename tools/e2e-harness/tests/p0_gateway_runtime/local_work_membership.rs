//! The serial reference sends independent two-request sessions. A matching
//! membership count alone cannot detect attribution to another session.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

pub(super) fn verify_pairs(records: &[Value], count: usize) -> Result<(), &'static str> {
    if count == 0 || !count.is_multiple_of(2) {
        return Err("reference must contain complete request pairs");
    }
    let mut requests = Vec::new();
    let mut assessments = BTreeMap::new();
    for record in records {
        let kind = record["fact"]["kind"].as_str();
        if !matches!(kind, Some("route_decision" | "branch_assessment_recorded")) {
            continue;
        }
        let request = record["correlation"]["request_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("missing request identity")?;
        if kind == Some("route_decision") {
            requests.push(request);
        } else if assessments.insert(request, &record["fact"]).is_some() {
            return Err("duplicate assessment for one request");
        }
    }
    if requests.len() != count
        || requests.iter().copied().collect::<BTreeSet<_>>().len() != count
        || assessments.len() != count / 2
    {
        return Err("missing, extra or duplicated request evidence");
    }
    for pair in requests.chunks_exact(2) {
        let assessment = assessments
            .remove(pair[1])
            .ok_or("follow-up has no assessment")?;
        if assessment["history_membership"]["request_ids"] != json!([pair[0]]) {
            return Err("assessment does not name its actual predecessor");
        }
        if assessment["partial"] != false
            || assessment["history_membership"]["capture_partial"] != false
        {
            return Err("assessment is incomplete");
        }
    }
    if !assessments.is_empty() {
        return Err("assessment belongs to an unexpected request");
    }
    Ok(())
}

/// Concurrent waves preserve order within each session, not across sessions.
/// Keep exact predecessor checks after requiring two requests per actual scope.
pub(super) fn verify_scoped_pairs(records: &[Value], count: usize) -> Result<(), &'static str> {
    let mut sessions = BTreeMap::<&str, Vec<Value>>::new();
    for record in records
        .iter()
        .filter(|record| record["fact"]["kind"] == "route_decision")
    {
        let scope = record["correlation"]["conversation_id"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("missing conversation identity")?;
        sessions.entry(scope).or_default().push(record.clone());
    }
    if sessions.len() != count / 2 || sessions.values().any(|requests| requests.len() != 2) {
        return Err("concurrent reference requires independent complete pairs");
    }
    let mut ordered: Vec<_> = sessions.into_values().flatten().collect();
    ordered.extend(
        records
            .iter()
            .filter(|record| record["fact"]["kind"] == "branch_assessment_recorded")
            .cloned(),
    );
    verify_pairs(&ordered, count)
}

#[test]
fn concurrent_oracle_rejects_scope_and_member_substitution() {
    let mut records = [("a", "a0"), ("b", "b0"), ("b", "b1"), ("a", "a1")].into_iter()
        .map(|(scope,id)| json!({"correlation":{"request_id":id,"conversation_id":scope},"fact":{"kind":"route_decision"}})).collect::<Vec<_>>();
    for (id, predecessor) in [("a1", "a0"), ("b1", "b0")] {
        records.push(json!({"correlation":{"request_id":id},"fact":{"kind":"branch_assessment_recorded","partial":false,
            "history_membership":{"request_ids":[predecessor],"capture_partial":false}}}));
    }
    assert!(
        verify_pairs(&records, 4).is_err(),
        "arrival order is not pair order"
    );
    assert_eq!(verify_scoped_pairs(&records, 4), Ok(()));
    for mutation in 0..4 {
        let mut changed = records.clone();
        match mutation {
            0 => changed[3]["correlation"]["conversation_id"] = json!("b"),
            1 => changed[4]["fact"]["history_membership"]["request_ids"] = json!(["b0"]),
            2 => changed[0]["correlation"]["conversation_id"] = Value::Null,
            3 => changed[4]["fact"]["partial"] = json!(true),
            _ => unreachable!(),
        }
        assert!(verify_scoped_pairs(&changed, 4).is_err());
    }
}

#[test]
fn paired_membership_oracle_rejects_wrong_identity_and_incomplete_evidence() {
    let mut valid = (0..4)
        .map(|index| {
            json!({"correlation":{"request_id":format!("r{index}")},
                "fact":{"kind":"route_decision"}})
        })
        .collect::<Vec<_>>();
    // Assessment records need not arrive in request order.
    for (current, previous) in [("r3", "r2"), ("r1", "r0")] {
        valid.push(json!({"correlation":{"request_id":current},"fact":{
            "kind":"branch_assessment_recorded","partial":false,
            "history_membership":{"request_ids":[previous],"capture_partial":false}
        }}));
    }
    assert_eq!(verify_pairs(&valid, 4), Ok(()));
    for mutation in 0..12 {
        let mut records = valid.clone();
        match mutation {
            0 => records[4]["fact"]["history_membership"]["request_ids"] = json!(["r0"]),
            1 => records[4]["fact"]["history_membership"]["request_ids"] = json!(["r3"]),
            2 => records[4]["fact"]["history_membership"]["request_ids"] = json!([]),
            3 => records[4]["fact"]["history_membership"]["request_ids"] = json!(["r2", "r0"]),
            4 => records[4]["fact"]["partial"] = json!(true),
            5 => records[4]["fact"]["history_membership"]["capture_partial"] = json!(true),
            6 => records[4]["correlation"]["request_id"] = json!("unknown"),
            7 => records[4] = records[5].clone(),
            8 => records[0] = records[1].clone(),
            9 => records[0]["correlation"]["request_id"] = Value::Null,
            10 => {
                records.pop();
            }
            11 => {
                records[4]["fact"]
                    .as_object_mut()
                    .unwrap()
                    .remove("partial");
            }
            _ => unreachable!(),
        }
        assert!(verify_pairs(&records, 4).is_err(), "mutation={mutation}");
    }
    assert!(verify_pairs(&[], 0).is_err());
    assert!(verify_pairs(&valid, 3).is_err());
}
