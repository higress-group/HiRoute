//! Public projections must describe actual route execution and its preceding
//! scored stage. No observation records are inserted by this acceptance gate.
use super::super::InstalledProduct;
use super::super::windows_headless_business::{cli, command};
use super::MODEL;
use hiroute_domain::{ExecutionGroupV1, PlanQualitySamplesPage};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) fn assert_committed(product: &InstalledProduct, plan: &str, count: usize, smart: bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    // The scored first stage can appear before the last request is ingested.
    // Keep the same receipt/value assertions and shared deadline, but await
    // every actual request before checking the complete quality projection.
    assert_usage_committed(product, plan, count, smart, deadline);
    let quality = loop {
        let page = quality_page(
            product,
            &["observation", "plan-quality", "samples", "--plan-id", plan],
            deadline,
        );
        if page.summary.scored_stage_count > 0
            && page.samples.iter().any(|sample| sample.upgrade.is_some())
        {
            break page;
        }
        assert!(
            Instant::now() < deadline,
            "actual native execution and assessment did not become publicly queryable"
        );
        thread::sleep(Duration::from_millis(100));
    };
    eprintln!(
        "Windows route quality checkpoint: smart={smart}, requests={count}, samples={}, scored={}, unrated={}",
        quality.samples.len(),
        quality.summary.scored_stage_count,
        quality.summary.unrated_stage_count
    );
    assert!(
        quality.summary.unrated_stage_count > 0,
        "missing later assessments must remain unrated"
    );
    let scope = if smart { "smart_saving" } else { "code" };
    let scored = quality
        .samples
        .iter()
        .find(|sample| {
            sample.executed_branch_id.as_deref() == Some(scope)
                && sample.assessment.as_ref().is_some_and(|assessment| {
                    assessment.reason.as_deref() == Some("windows-native-actual-stage-low")
                })
        })
        .expect("the actual preceding execution stage must own the returned score");
    let assessment = scored.assessment.as_ref().unwrap();
    assert!((assessment.score - 0.1).abs() < f64::EPSILON);
    assert!(!assessment.partial && assessment.evidence_available);
    let execution = scored.branch_execution.as_ref().unwrap();
    assert_eq!(execution.group, ExecutionGroupV1::Regular);
    assert_eq!(execution.candidate_index, 0);
    assert_eq!(execution.policy.floor_millis, Some(500));
    assert!(execution.policy.criteria_digest.is_some());
    let upgrade_source = quality
        .samples
        .iter()
        .find(|sample| sample.upgrade.is_some())
        .unwrap();
    let upgrade = upgrade_source.upgrade.as_ref().unwrap();
    assert_eq!(upgrade_source.segment_id, scored.segment_id);
    // The upgrade belongs to the assessed old stage. Its trigger request
    // identifies the new stage whose actual execution must be Primary.
    let protected = quality
        .samples
        .iter()
        .find(|sample| sample.first_request_id.as_deref() == Some(&upgrade.trigger_request_id))
        .expect("the actual upgrade-triggering request must open the protected execution stage");
    assert_eq!(protected.executed_branch_id.as_deref(), Some(scope));
    assert_eq!(
        protected.branch_execution.as_ref().unwrap().group,
        ExecutionGroupV1::Primary
    );
    assert_eq!(upgrade.decision.segment_id, scored.segment_id);
    assert_eq!(upgrade.decision.from_group, ExecutionGroupV1::Regular);
    assert_eq!(upgrade.decision.floor_millis, 500);
    assert!((upgrade.decision.score.as_f64().unwrap() - 0.1).abs() < f64::EPSILON);
    for sample in &quality.samples {
        assert_eq!(sample.plan_id, plan);
        assert!(sample.plan_revision > 0);
        assert_eq!(sample.native_model.as_deref(), Some(MODEL));
        assert!(sample.execution_evidence_available);
        assert!(
            sample
                .model_configuration_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(
            sample
                .profile_digest
                .as_deref()
                .is_some_and(|digest| digest.starts_with("sha256:"))
        );
        assert_eq!(sample.branch_execution.as_ref().unwrap().candidate_index, 0);
        assert_eq!(
            sample.selected_branch_id,
            sample.executed_branch_id.as_deref().unwrap()
        );
    }
    if !smart {
        let docs = quality
            .samples
            .iter()
            .find(|sample| sample.executed_branch_id.as_deref() == Some("docs"))
            .expect("custom category selection must produce its own actual stage");
        assert_eq!(
            docs.branch_execution.as_ref().unwrap().group,
            ExecutionGroupV1::Regular
        );
        assert!(
            docs.upgrade.is_none(),
            "a single-group docs category cannot inherit code primary protection"
        );
    }
    let below = quality_page(
        product,
        &[
            "observation",
            "plan-quality",
            "samples",
            "--plan-id",
            plan,
            "--competence",
            "below-floor",
        ],
        deadline,
    );
    assert!(
        below
            .samples
            .iter()
            .any(|sample| sample.segment_id == scored.segment_id)
    );
    assert!(below.samples.iter().all(|sample| {
        sample
            .assessment
            .as_ref()
            .is_some_and(|assessment| assessment.score < 0.5 && !assessment.partial)
    }));
}

fn assert_usage_committed(
    product: &InstalledProduct,
    plan: &str,
    count: usize,
    smart: bool,
    deadline: Instant,
) {
    let mut observed = BTreeSet::new();
    let mut observed_usage = (0, 0);
    while observed.len() < count {
        let listed = cli(
            product,
            &["sessions", "list"],
            Some(&request("sessions", query(plan, None))),
        );
        for session in listed["data"]["sessions"].as_array().unwrap() {
            let session_id = session["session_id"].as_str().unwrap();
            let shown = cli(
                product,
                &["sessions", "show"],
                Some(&request("timeline", query(plan, Some(session_id)))),
            );
            for row in shown["data"]["requests"].as_array().unwrap() {
                if row["final_native_model"] != MODEL || row["routing_context"]["plan_id"] != plan {
                    continue;
                }
                assert_eq!(row["routing_context"]["state"], "recorded");
                let id = row["request_id"].as_str().unwrap();
                if observed.contains(id) {
                    continue;
                }
                let facts = cli(
                    product,
                    &["sessions", "receipt"],
                    Some(&request(
                        "facts",
                        json!({"request_id":id,"limit":100,"cursor":null}),
                    )),
                );
                if let Some(usage) =
                    facts["data"]["facts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|fact| {
                            fact["input_tokens"]
                                .as_u64()
                                .zip(fact["output_tokens"].as_u64())
                                .is_some_and(|pair| [(31, 12), (37, 13)].contains(&pair))
                        })
                {
                    let ordinal = usage["attempt_ordinal"].as_u64().unwrap();
                    let started = facts["data"]["facts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|fact| {
                            fact["event_kind"] == "attempt_started"
                                && fact["attempt_ordinal"].as_u64() == Some(ordinal)
                        })
                        .expect(
                            "same public request must retain its exact actual attempt identity",
                        );
                    assert_eq!(started["native_model"], MODEL);
                    observed_usage.0 += usage["input_tokens"].as_u64().unwrap();
                    observed_usage.1 += usage["output_tokens"].as_u64().unwrap();
                    observed.insert(id.to_owned());
                }
            }
        }
        if observed.len() < count {
            assert!(
                Instant::now() < deadline,
                "actual mode requests did not yield their public usage receipts"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }
    assert_eq!(
        observed.len(),
        count,
        "plan-scoped receipts must describe only these actual independent requests"
    );
    let expected = if count == 4 {
        (136, 50)
    } else if smart {
        (167, 62)
    } else {
        (173, 63)
    };
    assert_eq!(observed_usage, expected);
    loop {
        let value = cli(product, &["value", "show", "--routing", plan], None);
        let summary = &value["data"]["summary"];
        if summary["pending_requests"] == 0 && sums(summary) == expected {
            assert!(
                summary["amounts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|amount| amount["known_sum_micros"].is_null()),
                "user-declared Native models must retain unknown prices"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "actual mode usage did not settle in the public value view"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

/// The public query rejects a page when ingestion changes its generation.
/// Repeat only a cursor-free first page within the existing settlement bound;
/// every other failure and all returned business assertions remain mandatory.
fn quality_page(
    product: &InstalledProduct,
    args: &[&str],
    deadline: Instant,
) -> PlanQualitySamplesPage {
    use hiroute_application_api::{ErrorCode, MachineStatus};
    loop {
        let (exit, envelope) = command(product, args, None);
        let status = serde_json::from_value::<MachineStatus>(envelope["status"].clone()).ok();
        let code = serde_json::from_value::<ErrorCode>(envelope["error"]["code"].clone()).ok();
        if exit == 3
            && status == Some(MachineStatus::Conflict)
            && code == Some(ErrorCode::ChangePreviewStale)
        {
            assert!(
                Instant::now() < deadline,
                "quality snapshot did not settle within its original bound"
            );
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        assert_eq!(
            exit, 0,
            "public quality query failed: status={status:?}, code={code:?}"
        );
        assert_eq!(status, Some(MachineStatus::Succeeded));
        return serde_json::from_value(envelope["data"].clone())
            .expect("actual public quality response must conform to its production DTO");
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}
fn query(plan: &str, session: Option<&str>) -> Value {
    json!({"from_ms":now()-600_000,"to_ms":now(),"session_id":session,"request_id":null,
        "plan_id":plan,"limit":50,"cursor":null,"only_model_switch":false})
}
fn request(view: &str, query: Value) -> Value {
    json!({"schema":"hiroute.observation.query/v2","intent":{"view":view,"query":query}})
}
fn sums(summary: &Value) -> (u64, u64) {
    let usage = summary["usage"].as_array().unwrap();
    let count = |metric| {
        usage.iter().find(|item| item["metric"] == metric).unwrap()["known_sum"]
            .as_u64()
            .unwrap_or(0)
    };
    (count("input"), count("output"))
}
