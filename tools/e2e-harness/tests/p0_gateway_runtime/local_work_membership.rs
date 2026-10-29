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
