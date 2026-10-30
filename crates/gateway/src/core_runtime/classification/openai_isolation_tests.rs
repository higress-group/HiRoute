use super::*;
use crate::server::core_runtime::classification::openai_decisions::parse_response as parse_openai;

const DEGREE: &str = r#"{"name":"q0","type":"score","score":0.2,"probabilities":[{"value":0,"label":"0","probability":0.8},{"value":1,"label":"1","probability":0.2}]}"#;
const ASSESSMENT: &str = r#"{"name":"q1","type":"score","score":0.95,"probabilities":[{"value":0,"label":"0","probability":0.25},{"value":1,"label":"1","probability":0.55},{"value":2,"label":"2","probability":0.2}]}"#;
const APPLICABILITY: &str = r#"{"name":"q2","type":"choice","choice":"applicable","probabilities":[{"value":"applicable","probability":0.9},{"value":"not_established","probability":0.1}]}"#;

#[test]
fn openai_optional_answer_number_overflow_stays_local_to_its_question() {
    let mut authority = c_authority();
    authority.wire_protocol = hiroute_domain::DecisionWireProtocolV2::OpenaiDecisions;
    let history = c_history(&authority);
    let prepared = request_template(&authority, &diagnostic_request(), &history, false).unwrap();
    let parse = |answers: &[&str]| {
        let body = format!(r#"{{"answers":[{}]}}"#, answers.join(","));
        parse_openai(body.as_bytes(), prepared.bindings.as_ref().unwrap()).unwrap()
    };
    for invalid in [
        r#"{"name":"q1","type":"score","score":1e999,"probabilities":[]}"#,
        r#"{"name":"q1","type":"score","score":0.95,"probabilities":[{"value":0,"label":"0","probability":1e999}]}"#,
    ] {
        let result = parse(&[DEGREE, invalid, APPLICABILITY]);
        assert_eq!(result.probabilities.unwrap()["simple"], 0.8);
        assert!(!result.degree_failed);
        assert!(result.invalid_assessment && result.assessment.is_none());
        assert_eq!(result.applicability, Some(0.9));
    }
    for invalid in [
        r#"{"name":"q2","type":"choice","probabilities":[{"value":"applicable","probability":1e999}]}"#,
        r#"{"name":"q2","type":1e999}"#,
        r#"{"name":"q2","type":"choice","type":"refusal"}"#,
    ] {
        let result = parse(&[DEGREE, ASSESSMENT, invalid]);
        assert_eq!(result.probabilities.unwrap()["simple"], 0.8);
        assert!(!result.degree_failed);
        assert_eq!(result.assessment.unwrap().score, 0.475);
        assert!(result.applicability.is_none());
    }
    let invalid_degree = r#"{"name":"q0","type":"score","score":1e999}"#;
    let result = parse(&[invalid_degree, ASSESSMENT, APPLICABILITY]);
    assert!(result.degree_failed);
    assert_eq!(result.assessment.unwrap().score, 0.475);
    assert_eq!(result.applicability, Some(0.9));
}
