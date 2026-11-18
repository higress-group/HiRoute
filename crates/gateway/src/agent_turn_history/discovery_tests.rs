use super::*;
use crate::agent_turn_history::tool_continuation::ToolResultSuffix;
use crate::context_hold::fingerprint::FingerprintBudget;
use crate::server::core_runtime::adapters::decode_ingress_request;
use serde_json::Value;

#[test]
fn discovery_handoff_binds_completed_call_kind_id_and_arguments() {
    let (_root, replay) = replay();
    let call = json!({"type":"tool_search_call","execution":"client","call_id":"c","id":"ts","status":"completed","arguments":{"query":"shipping","limit":3}});
    let output = json!({"type":"tool_search_output","execution":"client","call_id":"c","status":"completed","tools":[]});
    let completed = |items: &[Value]| DeliveredToolBatch::from_completed_output(items, |_| true);
    let batch = completed(std::slice::from_ref(&call)).unwrap();
    assert!(completed(&[call.clone(), call.clone()]).is_none());
    assert!(
        DeliveredToolBatch::from_completed_output(std::slice::from_ref(&call), |_| false).is_none()
    );
    let matches = |mut items: Vec<Value>| {
        items.insert(0, json!({"role":"user","content":"lookup"}));
        let mut request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route","input":items}),
        )
        .unwrap();
        crate::content_ref::externalize_model_request(&mut request, &replay, 0).unwrap();
        ToolResultSuffix::measure(
            &request,
            request.native_history_layout().unwrap(),
            1,
            &replay,
            &mut FingerprintBudget::default(),
        )
        .is_some_and(|suffix| batch.matches(&suffix))
    };
    assert!(matches(vec![call.clone(), output.clone()]));
    assert!(matches(vec![output.clone()]));
    let mut changed = call.clone();
    changed["arguments"]["query"] = json!("different");
    assert!(!matches(vec![changed, output.clone()]));
    assert!(!matches(vec![
        call.clone(),
        json!({"type":"function_call_output","call_id":"c","output":"wrong carrier"})
    ]));
    assert!(!matches(vec![call.clone(), output.clone(), output.clone()]));
    for (field, value) in [
        ("call_id", json!("other")),
        ("status", json!("in_progress")),
        ("future_semantics", json!(true)),
    ] {
        let mut changed = output.clone();
        changed[field] = value;
        assert!(!matches(vec![call.clone(), changed]));
    }
}

#[test]
fn retry_setup_distinguishes_discovery_schemas_and_positions() {
    let plain = json!({"type":"function","name":"eager","parameters":{"type":"object"}});
    let discovery =
        json!({"type":"tool_search","execution":"client","parameters":{"type":"object"}});
    let setup = |tools: Value| {
        let request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route","input":"lookup","tools":tools}),
        )
        .unwrap();
        crate::context_hold::observed_history_with_replay(&request, &[0; 32], None)
            .unwrap()
            .instruction_digest
    };
    let initial = setup(json!([discovery, plain]));
    assert_ne!(initial, setup(json!([plain, discovery])));
    let mut eager = plain.clone();
    eager["defer_loading"] = json!(false);
    let mut deferred = eager.clone();
    deferred["defer_loading"] = json!(true);
    assert_ne!(
        setup(json!([discovery, eager])),
        setup(json!([discovery, deferred]))
    );
    let namespace = |child: Value| json!({"type":"namespace","name":"shipping","tools":[child]});
    assert_ne!(
        setup(json!([discovery, namespace(eager)])),
        setup(json!([discovery, namespace(deferred)]))
    );
    let mut changed = discovery;
    changed["parameters"]["required"] = json!(["query"]);
    assert_ne!(initial, setup(json!([changed, plain])));
}
