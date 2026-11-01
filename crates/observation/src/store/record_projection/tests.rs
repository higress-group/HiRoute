use super::*;

#[test]
fn frozen_projected_records_keep_their_original_field_mapping() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/projected-v1.json")).unwrap();
    let source = &fixture["content"]["columns"];
    let text = |key: &str| source[key].as_str().unwrap().to_owned();
    let columns = ContentColumns {
        stream: serde_json::from_value(source["stream"].clone()).unwrap(),
        sequence: source["sequence"].as_u64().unwrap(),
        event_id: text("event_id"),
        workspace_id: text("workspace_id"),
        request_id: text("request_id"),
        direction: text("direction"),
        fork_id: text("fork_id"),
        phase: text("phase"),
    };
    let identity = &fixture["ack"]["identity"];
    let feedback = feedback_fields(identity["channel"].as_str().unwrap(), &columns.stream);
    for (kind, shared) in [("content", columns.shared()), ("ack", feedback)] {
        let frame: Vec<u8> = fixture[kind]["frame_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(decode(&frame, &shared).unwrap(), fixture[kind]["original"]);
    }
}

fn values() -> (Value, Value) {
    (
        json!({"schema_version":"original", "identity":{"stream":"fixed", "extension":null},
            "sequence":42, "text":"工具结果 / café 🦀", "unknown":{"array":[null,1,true,"x"]}}),
        json!({"schema_version":"original", "identity":{"stream":"fixed"}, "sequence":42}),
    )
}

#[test]
fn complete_values_survive_projection_including_unknown_and_null_fields() {
    let (value, shared) = values();
    let frame = encode(value.clone(), &shared).unwrap();
    assert!(matches!(frame[0], 2 | 3));
    assert_eq!(decode(&frame, &shared).unwrap(), value);
    assert!(decode(&frame, &json!({})).is_err());
    let mut changed = shared.clone();
    changed["sequence"] = json!(43);
    assert!(
        decode(&frame, &changed).is_err(),
        "source columns are checksum-bound"
    );
    let mut changed = shared;
    changed["identity"]["stream"] = json!("changed");
    assert!(
        decode(&frame, &changed).is_err(),
        "stream identity is checksum-bound"
    );
    changed["identity"]["stream"] = json!("x".repeat(records::MAX_RECORD + 1));
    assert!(
        decode(&frame, &changed).is_err(),
        "oversized source fields are rejected before restoration"
    );
}

#[test]
fn projection_rejects_missing_or_disagreeing_source_fields() {
    let (value, mut shared) = values();
    shared["sequence"] = json!(41);
    assert!(encode(value.clone(), &shared).is_err());
    assert!(encode(value, &json!({"absent":"field"})).is_err());
}

#[test]
fn legacy_identity_and_zstd_records_remain_readable() {
    for value in [json!({}), json!({"text":"abcdef".repeat(2000)})] {
        let bytes = serde_json::to_vec(&value).unwrap();
        let legacy = records::encode(&bytes).unwrap();
        assert!(matches!(legacy[0], 0 | 1));
        assert_eq!(
            decode(&legacy, &json!({"unused":"columns"})).unwrap(),
            value
        );
    }
}

#[test]
fn corruption_truncation_wrong_codec_and_excessive_expansion_fail() {
    let (value, shared) = values();
    let frame = encode(value, &shared).unwrap();
    for length in [0, 1, 10, 36, frame.len() - 1] {
        assert!(decode(&frame[..length], &shared).is_err());
    }
    let mut bad = frame.clone();
    bad[0] = 4;
    assert!(decode(&bad, &shared).is_err());
    let mut bad = frame.clone();
    bad[5] ^= 1;
    assert!(decode(&bad, &shared).is_err());
    let mut bad = frame;
    bad[1..5].copy_from_slice(&((records::MAX_RECORD + 1) as u32).to_le_bytes());
    assert!(decode(&bad, &shared).is_err());
    let mut bomb = records::encode(&vec![b'x'; records::MAX_RECORD]).unwrap();
    bomb[0] = 3;
    bomb[1..5].copy_from_slice(&1024u32.to_le_bytes());
    assert!(
        bomb.len() <= 1024 + 37,
        "exercise expansion, not the encoded-size guard"
    );
    assert!(decode(&bomb, &shared).is_err());
}

#[test]
fn projected_residual_cannot_override_shared_fields() {
    let (value, shared) = values();
    let mut frame = records::encode(&serde_json::to_vec(&value).unwrap()).unwrap();
    frame[0] += 2;
    assert!(decode(&frame, &shared).is_err());
}

#[test]
fn large_records_keep_existing_identity_fallback() {
    let value = json!({"text":"x".repeat(records::MAX_RECORD+1),"shared":"value"});
    let frame = encode(value.clone(), &json!({"shared":"value"})).unwrap();
    assert_eq!(frame[0], 0);
    assert_eq!(decode(&frame, &json!({})).unwrap(), value);
}
