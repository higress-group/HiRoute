use super::*;
use hiroute_domain::{
    ClassifierAuthHeaderV1, DecisionAuthenticationV2, DecisionConnectionV2, DecisionServiceRecord,
    DecisionServiceSchemaV2, DecisionServiceV2, DecisionWireProtocolV2,
};

fn service(endpoint: &str) -> DecisionServiceV2 {
    DecisionServiceV2 {
        schema: DecisionServiceSchemaV2::V2,
        id: "decision/example".into(),
        revision: 1,
        name: "Example".into(),
        connection: DecisionConnectionV2::Native {
            provider: "openai".into(),
            wire_protocol: DecisionWireProtocolV2::OpenaiDecisions,
            model: "example".into(),
            endpoint: endpoint.into(),
            timeout_ms: 1000,
            authentication: DecisionAuthenticationV2::Independent {
                auth_header: ClassifierAuthHeaderV1 {
                    name: "Authorization".into(),
                    value_secret_ref: "decision/example/key".into(),
                },
            },
        },
    }
}

#[test]
fn independent_header_reuse_matches_effective_origin_and_exact_authentication() {
    let old = DecisionServiceRecord::V2(service("https://example.test/v1/decisions"));
    for endpoint in [
        "https://example.test:443/other",
        "https://EXAMPLE.test/v1/decisions",
    ] {
        let new = service(endpoint);
        assert!(
            decision_header_is_reusable(&old, &new, new.connection.transport().2.unwrap()),
            "{endpoint}"
        );
    }
    for endpoint in [
        "https://example.test:444/v1/decisions",
        "https://other.test/v1/decisions",
        "http://example.test/v1/decisions",
    ] {
        let new = service(endpoint);
        assert!(
            !decision_header_is_reusable(&old, &new, new.connection.transport().2.unwrap()),
            "{endpoint}"
        );
    }
    let mut replacement = service("https://example.test:443/v1/decisions");
    if let DecisionConnectionV2::Native {
        authentication: DecisionAuthenticationV2::Independent { auth_header },
        ..
    } = &mut replacement.connection
    {
        auth_header.value_secret_ref = "decision/example/other".into();
    }
    assert!(!decision_header_is_reusable(
        &old,
        &replacement,
        replacement.connection.transport().2.unwrap()
    ));
    let old_http = DecisionServiceRecord::V2(service("http://127.0.0.1/v1/decisions"));
    let new_http = service("http://127.0.0.1:80/other");
    assert!(decision_header_is_reusable(
        &old_http,
        &new_http,
        new_http.connection.transport().2.unwrap()
    ));
}
