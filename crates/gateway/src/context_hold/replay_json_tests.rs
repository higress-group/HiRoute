// Frozen semantic oracle from main 2e6ba55c1; never used by production.
use super::*;
use crate::replay::write_canonical_json;
pub(crate) fn legacy_canonical_json(
    writer: &mut impl Write,
    value: &Value,
    replay: &ReplayStore,
) -> Option<()> {
    if let Some(reference) = value.content_ref() {
        if !replay.is_canonical_json(&reference) {
            return None;
        }
        let mut reader = replay.reader(&reference).ok()?;
        std::io::copy(&mut reader, writer).ok()?;
        reader.verify_terminal().ok()?;
        return Some(());
    }
    match value {
        Value::String(value) if value.content_ref().is_some() => {
            let reference = value.content_ref()?;
            let mut reader = replay.reader(&reference).ok()?;
            writer.write_all(b"\"").ok()?;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let count = reader.read(&mut buffer).ok()?;
                if count == 0 {
                    break;
                }
                let mut start = 0;
                for (index, &byte) in buffer[..count].iter().enumerate() {
                    let escape = match byte {
                        b'"' => Some(b"\\\"".as_slice()),
                        b'\\' => Some(b"\\\\".as_slice()),
                        b'\x08' => Some(b"\\b".as_slice()),
                        b'\x0c' => Some(b"\\f".as_slice()),
                        b'\n' => Some(b"\\n".as_slice()),
                        b'\r' => Some(b"\\r".as_slice()),
                        b'\t' => Some(b"\\t".as_slice()),
                        0x00..=0x1f => None,
                        _ => continue,
                    };
                    writer.write_all(&buffer[start..index]).ok()?;
                    if let Some(escape) = escape {
                        writer.write_all(escape).ok()?;
                    } else {
                        const HEX: &[u8; 16] = b"0123456789abcdef";
                        writer
                            .write_all(&[
                                b'\\',
                                b'u',
                                b'0',
                                b'0',
                                HEX[(byte >> 4) as usize],
                                HEX[(byte & 0xf) as usize],
                            ])
                            .ok()?;
                    }
                    start = index + 1;
                }
                writer.write_all(&buffer[start..count]).ok()?;
            }
            reader.verify_terminal().ok()?;
            writer.write_all(b"\"").ok()?;
        }
        Value::Array(values) => {
            writer.write_all(b"[").ok()?;
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    writer.write_all(b",").ok()?;
                }
                legacy_canonical_json(writer, value, replay)?;
            }
            writer.write_all(b"]").ok()?;
        }
        Value::Object(values) => {
            writer.write_all(b"{").ok()?;
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    writer.write_all(b",").ok()?;
                }
                serde_json::to_writer(&mut *writer, key).ok()?;
                writer.write_all(b":").ok()?;
                legacy_canonical_json(writer, &values[key], replay)?;
            }
            writer.write_all(b"}").ok()?;
        }
        _ => write_canonical_json(writer, value).ok()?,
    }
    Some(())
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn store(threshold: usize) -> (Self, ReplayStore) {
        use crate::replay::{ReplayConfig, ReplayManager};
        use hiroute_gateway_core::runtime::body::BudgetTree;
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "hiroute-json-cursor-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: threshold,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let tree = BudgetTree::new(256 * 1024 * 1024, 256 * 1024 * 1024).unwrap();
        let store = manager
            .begin_request(tree.stream(256 * 1024 * 1024).unwrap())
            .unwrap();
        (Self(root), store)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn collect(value: &Value, replay: &ReplayStore, width: usize) -> (Vec<u8>, usize) {
    let mut cursor = CanonicalJsonReplayCursor::new(value, replay);
    let mut output = Vec::new();
    let mut buffer = vec![0; width];
    let mut yields = 0;
    loop {
        let step = cursor.pump(&mut buffer, &mut JsonFuel::default()).unwrap();
        output.extend_from_slice(&buffer[..step.written]);
        if step.status == JsonPumpStatus::Yield {
            yields += 1;
        }
        if step.status == JsonPumpStatus::Done {
            break;
        }
    }
    (output, yields)
}

#[test]
fn canonical_cursor_matches_frozen_semantics_at_all_escape_and_chunk_boundaries() {
    let (_root, replay) = Fixture::store(1024 * 1024);
    let controls: String = (0..=31).map(char::from).collect();
    let value = serde_json::json!({
        "z": [null, true, false, -0.125, 1e30, "", controls],
        "a": {"z": "路由🦀\\\"".repeat(5000), "a": [[], {}, [1, 2, 3]]},
        "__hiroute_content_ref_v2_1_0_3_3__": "a marker in a key stays literal"
    });
    let mut expected = Vec::new();
    legacy_canonical_json(&mut expected, &value, &replay).unwrap();
    for width in [1, 7, 16384, 65536] {
        let (actual, _) = collect(&value, &replay, width);
        assert_eq!(actual, expected, "pump width {width}");
    }
}

#[test]
fn memory_and_disk_references_expand_identically_without_exposing_locators() {
    for threshold in [64, 1024 * 1024] {
        let (_root, replay) = Fixture::store(threshold);
        let text = "路由🦀\n\t\\\"".repeat(4096);
        let object = serde_json::json!({"z": [1, false], "a": text});
        let mut pool = replay.begin_content_pool().unwrap();
        let string = pool.append(text.as_bytes()).unwrap();
        let canonical = pool.append_canonical_json(&object).unwrap();
        let raw_json = pool.append(br#"{"not":"proved canonical"}"#).unwrap();
        pool.seal().unwrap();
        replay
            .prevalidate(&[string.clone(), canonical.clone(), raw_json.clone()])
            .unwrap();
        let value = serde_json::json!([string.wire_marker(), canonical.json_marker()]);
        let mut expected = Vec::new();
        legacy_canonical_json(&mut expected, &value, &replay).unwrap();
        let (actual, _) = collect(&value, &replay, 17);
        assert_eq!(actual, expected);
        let expected_value = serde_json::json!([text, object]);
        assert_eq!(
            serde_json::from_slice::<Value>(&actual).unwrap(),
            expected_value
        );
        let raw_marker = raw_json.json_marker();
        let mut cursor = CanonicalJsonReplayCursor::new(&raw_marker, &replay);
        // A JSON-shaped raw range is not proof of canonicalization.
        assert!(cursor.pump(&mut [0; 16], &mut JsonFuel::default()).is_err());
    }
}

#[test]
fn wide_objects_yield_before_emitting_and_keep_lexicographic_order() {
    let (_root, replay) = Fixture::store(1024 * 1024);
    let fields = (0..4096)
        .rev()
        .map(|index| (format!("field-{index:04}"), Value::from(index)))
        .collect();
    let value = Value::Object(fields);
    let mut cursor = CanonicalJsonReplayCursor::new(&value, &replay);
    let first = cursor.pump(&mut [0; 7], &mut JsonFuel::default()).unwrap();
    assert_eq!(first.written, 0);
    assert_eq!(first.status, JsonPumpStatus::Yield);
    let mut expected = Vec::new();
    legacy_canonical_json(&mut expected, &value, &replay).unwrap();
    let (actual, yields) = collect(&value, &replay, 31);
    assert_eq!(actual, expected);
    assert!(yields > 0);
}

#[test]
fn forged_reference_fails_closed_and_the_cursor_cannot_resume_as_success() {
    let (_root, replay) = Fixture::store(64);
    let value = Value::String(crate::content_ref::ContentRef::new(999, 0, 12, 12).wire_marker());
    let mut cursor = CanonicalJsonReplayCursor::new(&value, &replay);
    assert!(cursor.pump(&mut [0; 31], &mut JsonFuel::default()).is_err());
    assert!(cursor.pump(&mut [0; 31], &mut JsonFuel::default()).is_err());
}
