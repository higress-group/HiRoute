use super::*;
use crate::context_hold::identity::identity_facts;
use crate::server::core_runtime::adapters;
use crate::server::request_plan::IngressProtocol;
use serde_json::json;

fn headers(purpose: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-hiroute-agent", "hermes".parse().unwrap());
    headers.insert("x-hermes-session-id", "child".parse().unwrap());
    headers.insert(
        "x-hiroute-routing-context",
        json!({"version":2,"agent":"hermes","purpose":purpose,
            "session_id":"child","turn_id":"turn","request_id":"turn:api:1",
            "input":{"kind":"observed_batch","event_id":"turn"}})
        .to_string()
        .parse()
        .unwrap(),
    );
    headers
}

fn lineage() -> Value {
    let stamp = json!({"observer_id":"observer","sequence":1,"observed_at_ms":1000,"age_ms":0});
    json!({"version":1,"ids":["root","child"],"complete":true,
        "proof_observation":stamp,"current_state":{"value":"open","observation":stamp}})
}

fn facts(headers: &HeaderMap) -> ContextIdentityFacts {
    let authorized = crate::server::publication::authorized_native_consumer_fixture(Some(
        hiroute_domain::AgentModelConsumerV1::Hermes,
    ));
    let body = json!({"model":"route","input":[{"role":"user","content":"actual input"}]});
    let request = adapters::decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    identity_facts(
        &authorized,
        IngressProtocol::Responses,
        headers,
        &body,
        &request,
        &[1; 32],
        &[2; 32],
    )
}

#[test]
fn v2_hermes_reuses_only_complete_open_compression_normalization() {
    for variant in 0..7 {
        let mut headers = headers("main");
        let mut proof = lineage();
        match variant {
            0 => {}
            1 => proof["complete"] = json!(false),
            2 => {
                proof["invalid_edges"] = json!({"edges":[["root","child"]],
                "observation":proof["proof_observation"].clone()})
            }
            3 => proof["ids"] = json!(["root", "child", "root", "child"]),
            4 => proof["ids"] = json!(["root", "ordinary-fork"]),
            5 => proof["current_state"]["value"] = json!("other_closed"),
            6 => {}
            _ => unreachable!(),
        }
        if variant != 6 {
            headers.insert(
                "x-hermes-compression-lineage",
                proof.to_string().parse().unwrap(),
            );
        }
        let facts = facts(&headers);
        assert_eq!(facts.session_scope(), "conversation");
        assert_eq!(facts.observation_identity().unwrap().1, &["child"]);
        assert_eq!(
            facts.native_checkpoint().user_origin,
            NativeUserOrigin::Observed
        );
        assert_eq!(facts.native_checkpoint().turn, Some(digest("turn")));
        if variant == 0 {
            assert_eq!(facts.hold_identity().unwrap().1, &["root"]);
            assert_eq!(facts.native_checkpoint().checkpoint, Some(digest("child")));
        } else {
            assert_eq!(facts.hold_identity().unwrap().1, &["child"]);
            assert!(facts.native_checkpoint().checkpoint.is_none());
        }
    }
}

#[test]
fn v2_hermes_auxiliary_lineage_never_reenables_main_history() {
    for purpose in ["compaction", "summary", "title", "session-title", "prewarm"] {
        let mut headers = headers(purpose);
        headers.insert(
            "x-hermes-compression-lineage",
            lineage().to_string().parse().unwrap(),
        );
        let facts = facts(&headers);
        assert!(facts.hold_identity().is_none(), "{purpose}");
        assert!(facts.native_checkpoint().checkpoint.is_none(), "{purpose}");
    }
}

#[test]
fn hermes_lineage_scope_is_independent_of_optional_context_failure() {
    for context in [
        None,
        Some("not-json"),
        Some(r#"{"version":99,"agent":"hermes","session_id":"child","purpose":"main"}"#),
        Some(r#"{"version":2,"version":2,"agent":"hermes","session_id":"child","purpose":"main"}"#),
        Some(
            r#"{"version":2,"agent":"hermes","session_id":"child","purpose":"main","input":{"kind":"unknown"}}"#,
        ),
    ] {
        for valid_lineage in [false, true] {
            let mut h = headers("main");
            h.remove("x-hiroute-routing-context");
            if let Some(context) = context {
                h.insert("x-hiroute-routing-context", context.parse().unwrap());
            }
            let mut proof = lineage();
            if !valid_lineage {
                proof["current_state"]["observation"]["age_ms"] = json!(1001);
            }
            h.insert(
                "x-hermes-compression-lineage",
                proof.to_string().parse().unwrap(),
            );
            let actual = facts(&h);
            assert_eq!(actual.observation_identity().unwrap().1, &["child"]);
            assert_eq!(
                actual.hold_identity().unwrap().1,
                &[if valid_lineage { "root" } else { "child" }]
            );
            assert_eq!(
                actual.native_checkpoint().user_origin,
                NativeUserOrigin::Observed
            );
            assert!(actual.native_checkpoint().turn.is_none());
            assert_eq!(
                actual.native_checkpoint().checkpoint,
                valid_lineage.then(|| digest("child"))
            );
            // The independent proof cannot rescue conflicting native identities.
            h.insert("session-id", "competing-family".parse().unwrap());
            let conflicting = facts(&h);
            assert!(conflicting.hold_identity().is_none());
            assert!(conflicting.native_checkpoint().checkpoint.is_none());
        }
    }
}

#[test]
fn legacy_hermes_context_and_hint_require_objects_and_keep_duplicate_rejection() {
    let mut headers = headers("main");
    let context =
        r#"{"version":1,"agent":"hermes","session_id":"child","purpose":"main","turn_id":"turn"}"#;
    let hint = json!({"version":1,"kind":"observed_turn_input","reverse_user_index":1,"text_sha256":"a".repeat(64)}).to_string();
    headers.insert("x-hiroute-routing-context", context.parse().unwrap());
    headers.insert("x-hermes-observed-input", hint.parse().unwrap());
    assert!(hermes_observed_input(&headers).is_some());
    assert_eq!(
        facts(&headers).native_checkpoint().turn,
        Some(digest("turn"))
    );
    headers.insert(
        "x-hermes-observed-input",
        format!(r#"[1,"observed_turn_input",1,"{}"]"#, "a".repeat(64))
            .parse()
            .unwrap(),
    );
    assert!(hermes_observed_input(&headers).is_none());
    headers.insert("x-hermes-observed-input", hint.parse().unwrap());
    for invalid in [
        r#"[1,"hermes","child","main","turn",null,null,true]"#.to_owned(),
        context.replace(r#""version":1"#, r#""version":1,"version":1"#),
    ] {
        headers.insert("x-hiroute-routing-context", invalid.parse().unwrap());
        assert!(hermes_observed_input(&headers).is_none());
        let facts = facts(&headers);
        assert!(facts.native_checkpoint().turn.is_none());
        assert_eq!(
            facts.native_checkpoint().user_origin,
            NativeUserOrigin::Observed
        );
        assert_eq!(facts.session_scope(), "conversation");
    }
}
