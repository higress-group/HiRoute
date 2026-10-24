use super::*;
use crate::content_ref::{ContentRef, MARKER_PREFIX};
use crate::server::core_runtime::adapters::{
    ReplacementEncoding, RequestedReplacement, prepare_replay_json_template,
};
use serde_json::{Value, json};

#[test]
fn template_keys_keep_exact_json_meaning_across_nested_and_escaped_fragments() {
    let key = format!("pre-{MARKER_PREFIX}99_0_4_4__\"\\\n中文-{MARKER_PREFIX}not-a-ref");
    let escaped = "quote \" slash \\ newline\n";
    let body =
        json!({key.clone(): [{key.clone(): "plain"}], "after": {key: true}, "escaped": escaped});
    let bytes = serde_json::to_vec(&body).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), body);
    let mut values = value_strings(&bytes)
        .map(|range| {
            serde_json::from_slice::<String>(&bytes[range.start - 1..range.end + 1]).unwrap()
        })
        .collect::<Vec<_>>();
    values.sort();
    let mut expected = vec![escaped.to_owned(), "plain".to_owned()];
    expected.sort();
    assert_eq!(values, expected);
    let template = prepare_replay_json_template(&body, []).unwrap();
    assert!(template.replacements.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&template.bytes).unwrap(),
        body
    );
}

#[test]
fn template_key_equal_to_registered_reference_does_not_consume_a_value_replacement() {
    let content = ContentRef::new(99, 0, 4, 4);
    let marker = content.wire_marker();
    let body = json!({marker.clone(): {"value": marker.clone()}, "after": "plain"});
    let requested = || RequestedReplacement {
        content: content.clone(),
        encoding: ReplacementEncoding::JsonString,
    };
    let template = prepare_replay_json_template(&body, [requested()]).unwrap();
    assert_eq!(template.replacements.len(), 1);
    let replacement = &template.replacements[0];
    assert_eq!(
        &template.bytes[replacement.start..replacement.end],
        marker.as_bytes()
    );
    assert_eq!(replacement.content, content);
    assert_eq!(
        serde_json::from_slice::<Value>(&template.bytes).unwrap(),
        body
    );

    // Keys never satisfy a missing value, and unknown or excess value references
    // are still rejected rather than being treated as literal data.
    assert!(prepare_replay_json_template(&json!({marker.clone(): true}), [requested()]).is_err());
    assert!(prepare_replay_json_template(&body, []).is_err());
    assert!(
        prepare_replay_json_template(&json!({"a":marker.clone(),"b":marker}), [requested()])
            .is_err()
    );
}

#[test]
fn template_value_boundaries_preserve_raw_and_nested_json_replacement_spans() {
    let content = ContentRef::new(99, 0, 4, 4);
    let marker = content.wire_marker();
    for (encoding, value, placeholder) in [
        (
            ReplacementEncoding::RawJson,
            marker.clone(),
            format!("\"{marker}\""),
        ),
        (
            ReplacementEncoding::RawJsonInJsonString,
            format!("{{\"args\":\"{marker}\"}}"),
            format!("\\\"{marker}\\\""),
        ),
    ] {
        let body = json!({marker.clone():true, "value":value});
        let template = prepare_replay_json_template(
            &body,
            [RequestedReplacement {
                content: content.clone(),
                encoding,
            }],
        )
        .unwrap();
        assert_eq!(template.replacements.len(), 1);
        let replacement = &template.replacements[0];
        assert_eq!(replacement.encoding, encoding);
        assert_eq!(
            &template.bytes[replacement.start..replacement.end],
            placeholder.as_bytes()
        );
    }
}
