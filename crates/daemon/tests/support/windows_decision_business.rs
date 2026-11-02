//! Actual decision service CLI, protected credentials, immutable versions and protocol test.
use super::windows_business_http as http;
use super::windows_headless_business::{self as business, cli};
use super::{InstalledProduct, SECRET};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn roundtrip(product: &InstalledProduct) {
    let seen = Arc::new(Mutex::new(Vec::<(String, usize)>::new()));
    let requests = seen.clone();
    let first = format!("Bearer {SECRET}");
    let second = "Bearer windows-native-decision-rotated-secret-0123456789";
    super::track_secret(first.as_bytes());
    super::track_secret(second.as_bytes());
    let original = first.clone();
    let c_vectors = Arc::new(AtomicBool::new(false));
    let response_vectors = c_vectors.clone();
    let upstream = http::Server::start(move |request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/systemone");
        let version = if request.authenticated(&original) {
            "r1"
        } else if request.authenticated(second) {
            "r2"
        } else {
            panic!("decision service lost its version-owned credential")
        };
        assert_eq!(request.body["model"], "windows-decision-model");
        let state = request.body["state"].as_object().unwrap();
        assert_eq!(state.len(), 5);
        for field in [
            "latest_user",
            "prior_context",
            "assessment_stage",
            "history_partial",
            "request_context",
        ] {
            assert!(state.contains_key(field), "current request lacks {field}");
        }
        assert_eq!(state["request_context"]["source"], "request_replay");
        assert!(state["request_context"]["messages"].is_array());
        assert!(state["request_context"]["truncated"].is_boolean());
        assert!(state["request_context"]["boundary_unavailable"].is_boolean());
        let questions = request.body["questions"].as_object().unwrap();
        let historical = questions
            .values()
            .any(|q| q["criteria"].get("applicable").is_some());
        let target = state["assessment_stage"].as_array().unwrap();
        if historical {
            assert!(!target.is_empty());
        } else {
            assert!(
                target.is_empty(),
                "connection test fabricated historical work"
            );
        }
        assert!(questions.len() == 3 || historical && questions.len() == 19);
        let answers=questions.iter().map(|(id,question)| {
            let vectors = response_vectors.load(Ordering::SeqCst);
            let answer=if vectors && question["criteria"].get("applicable").is_some() {
                json!({"type":"choice","probabilities":{"applicable":0.7,"not_established":0.3}})
            } else if vectors && question["type"]=="score" && question["criteria"].as_array().is_some_and(|a| a.len()==3) {
                json!({"type":"score","score":1.0})
            } else if question["type"]=="choice" {
                json!({"type":"choice","choice":question["criteria"].as_object().unwrap().keys().next().unwrap()})
            } else {
                assert_eq!(question["type"],"score");
                json!({"type":"score","probabilities":{"0":0.9,"1":0.1}})
            };
            (id.clone(),answer)
        }).collect::<serde_json::Map<_,_>>();
        requests
            .lock()
            .unwrap()
            .push((version.to_owned(), questions.len()));
        http::Reply::json(200, json!({"answers":answers}))
    });
    let initial = cli(product, &["decision", "services", "list"], None);
    assert!(initial["data"]["services"].as_array().unwrap().is_empty());
    let mut service = json!({"id":"windows-public-decision","revision":1,"name":"Windows decision service",
        "connection":{"kind":"system_one","provider":"compatible","model":"windows-decision-model",
            "endpoint":format!("http://{}/systemone",upstream.address),"timeout_ms":3000,
            "auth_header":{"name":"Authorization","value_secret_ref":"decision/windows/r1"}}});
    let slot = "candidate/native/windows-decision-r1";
    assert_eq!(
        product.register_inherited_secret_for(slot, &first)["data"]["registered"],
        true
    );
    mutate(
        product,
        &json!({"id":service["id"],"expected_revision":0,"service":service,"input_slot":slot}),
        "windows-decision-create",
    );
    assert_eq!(
        product.cli(&["protected-input", "release", "--candidate", slot])["data"]["registered"],
        true
    );
    assert_eq!(
        cli(product, &["decision", "services", "list"], None)["data"]["services"],
        json!([service])
    );
    test(product, &service);
    assert_eq!(*seen.lock().unwrap(), vec![("r1".to_owned(), 3)]);
    // Without accept_digest this public command previews a change. Retain a
    // valid r1 deletion preview, then attempt its actual submission after r2.
    let stale_preview = cli(
        product,
        &["decision", "services", "apply"],
        Some(&json!({"schema_version":{"major":1,"minor":0},
            "spec":{"schema_version":{"major":1,"minor":0},"command_id":"decision.services.apply",
                "resource_id":service["id"],"desired_state":{"id":service["id"],"expected_revision":1,"service":null}}})),
    )["data"]
        .clone();
    let stale = json!({"schema_version":{"major":1,"minor":0},
        "spec":stale_preview["normalized_spec"],"accept_digest":stale_preview["change_digest"],
        "expected_revisions":stale_preview["expected_revisions"],"idempotency_key":"windows-decision-stale-delete"});
    let historical = service.clone();
    service["revision"] = json!(2);
    service["connection"]["auth_header"]["value_secret_ref"] = json!("decision/windows/r2");
    let slot = "candidate/native/windows-decision-r2";
    assert_eq!(
        product.register_inherited_secret_for(slot, second)["data"]["registered"],
        true
    );
    mutate(
        product,
        &json!({"id":service["id"],"expected_revision":1,"service":service,"input_slot":slot}),
        "windows-decision-rotate",
    );
    assert_eq!(
        product.cli(&["protected-input", "release", "--candidate", slot])["data"]["registered"],
        true
    );
    test(product, &service);
    test(product, &historical);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ("r1".to_owned(), 3),
            ("r2".to_owned(), 3),
            ("r1".to_owned(), 3)
        ],
        "rotation must not mutate retained r1 credentials"
    );
    let (exit, rejected) =
        business::command(product, &["decision", "services", "apply"], Some(&stale));
    assert_ne!(exit, 0);
    assert!(matches!(
        rejected["error"]["code"].as_str(),
        Some("REVISION_CONFLICT" | "CHANGE_PREVIEW_STALE")
    ));
    assert_eq!(
        cli(product, &["decision", "services", "list"], None)["data"]["services"],
        json!([service])
    );
    test_c(product, &historical, false, "unsupported");
    c_vectors.store(true, Ordering::SeqCst);
    test_c(product, &historical, false, "verified");
    test_c(product, &service, true, "verified");
    mutate(
        product,
        &json!({"id":service["id"],"expected_revision":2,"service":null}),
        "windows-decision-delete",
    );
    assert!(
        cli(product, &["decision", "services", "list"], None)["data"]["services"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(seen.lock().unwrap().len(), 6);
}

fn test_c(product: &InstalledProduct, service: &Value, maximum: bool, expected: &str) {
    use hiroute_domain::*;
    let p = NativeProtocolV1::default();
    let mut j = JudgmentSettingsV1::default();
    p.stock_assessment(&mut j);
    let scopes: Vec<_> = if maximum {
        (0..16).map(|i| (format!("probe-{i}"), j.clone())).collect()
    } else {
        vec![(SMART_SAVING_SCOPE_ID.into(), j.clone())]
    };
    let decision = if maximum {
        DecisionDefinitionV1::Categorical {
            instructions: "Choose a synthetic category.".into(),
            options: scopes
                .iter()
                .map(|(id, _)| CategoryOptionV1 {
                    id: id.clone(),
                    criterion: id.clone(),
                    refinement: Some(j.degree.definition()),
                })
                .collect(),
        }
    } else {
        j.degree.definition().into()
    };
    let frozen = FrozenNativeProtocolV1::freeze(&p, decision, scopes).unwrap();
    let request = json!({"schema":"hiroute.historical-applicability-test/v1","service_id":service["id"],"service_revision":service["revision"],"protocol":frozen,"shape":if maximum {"max_branches"} else {"standard"}});
    let tested = cli(product, &["decision", "services", "test"], Some(&request));
    assert_eq!(tested["data"]["state"], expected);
    assert_eq!(tested["data"]["service_revision"], service["revision"]);
    assert_eq!(
        tested["data"]["protocol_digest"],
        request["protocol"]["digest"]
    );
    assert_eq!(
        tested["data"]["question_count"],
        if maximum { 19 } else { 3 }
    );
}
pub(crate) fn mutate(product: &InstalledProduct, change: &Value, key: &str) {
    let version = json!({"major":1,"minor":0});
    let spec = json!({"schema_version":version,"command_id":"decision.services.apply","resource_id":change["id"],"desired_state":change});
    let preview = cli(
        product,
        &["decision", "services", "apply"],
        Some(&json!({"schema_version":version,"spec":spec})),
    )["data"]
        .clone();
    let request = json!({"schema_version":version,"spec":preview["normalized_spec"],"accept_digest":preview["change_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":key});
    let applied = cli(product, &["decision", "services", "apply"], Some(&request));
    assert_eq!(applied["data"]["state"], "succeeded");
    let replay = cli(product, &["decision", "services", "apply"], Some(&request));
    assert_eq!(
        replay["data"], applied["data"],
        "actual CLI retry must reuse its journaled operation"
    );
}
fn test(product: &InstalledProduct, service: &Value) {
    let tested = cli(
        product,
        &["decision", "services", "test"],
        Some(
            &json!({"schema":"hiroute.classifier-decision-test/v1","classifier":{"kind":"decision_service","service":service}}),
        ),
    );
    assert_eq!(
        tested["data"]["schema"],
        "hiroute.classifier-decision-test-result/v1"
    );
    assert_eq!(tested["data"]["outcome"], "passed");
    assert!(tested["data"]["failure_code"].is_null());
}
