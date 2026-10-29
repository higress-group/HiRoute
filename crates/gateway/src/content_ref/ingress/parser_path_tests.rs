use super::*;
use crate::replay::{ReplayConfig, ReplayManager};
use hiroute_gateway_core::runtime::body::{BudgetTree, StreamBudget};

struct Fixture {
    store: ReplayStore,
    budget: StreamBudget,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new(threshold: usize) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hiroute-parser-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: threshold,
            record_bytes: 31,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
            .unwrap()
            .stream(8 * 1024 * 1024)
            .unwrap();
        Self {
            store: manager.begin_request(budget.clone()).unwrap(),
            budget,
            root,
        }
    }

    fn raw(&self, bytes: &[u8]) -> crate::content_ref::ContentRef {
        let mut writer = self.store.begin_raw().unwrap();
        for chunk in bytes.chunks(113) {
            writer.append(chunk).unwrap();
        }
        writer.seal().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn optional_slice_scratch_is_charged_released_and_refused_under_pressure() {
    let fixture = Fixture::new(1024 * 1024);
    let body = br#"{"model":"alias","input":"complete input"}"#;
    let raw = fixture.raw(body);
    let stats = scan_ingress_document(fixture.store.reader(&raw).unwrap()).unwrap();
    let before = fixture.budget.snapshot().unwrap().live;
    let backing = fixture.store.snapshot();
    let scratch = memory_parse_input(&fixture.store, &raw, stats)
        .unwrap()
        .unwrap();
    assert_eq!(scratch.bytes().as_ref(), body);
    assert_eq!(fixture.budget.snapshot().unwrap().live, before + body.len());
    assert_eq!(fixture.store.snapshot(), backing);
    drop(scratch);
    assert_eq!(fixture.budget.snapshot().unwrap().live, before);

    let free = fixture.budget.available_bytes().unwrap();
    let _pressure = fixture
        .budget
        .reserve(MemoryRole::AttemptWire, free - 32 * 1024)
        .unwrap();
    assert!(
        memory_parse_input(&fixture.store, &raw, stats)
            .unwrap()
            .is_none()
    );
    let (document, _workspace) =
        parse_ingress_document(IngressProtocol::Responses, &fixture.store, &raw, stats).unwrap();
    assert_eq!(document, serde_json::from_slice::<Value>(body).unwrap());
}

#[test]
fn malformed_slice_and_reader_inputs_release_scratch_and_keep_the_same_error() {
    for threshold in [31, 1024 * 1024] {
        let fixture = Fixture::new(threshold);
        let valid = fixture.raw(br#"{"model":"alias","input":"ok"}"#);
        let stats = scan_ingress_document(fixture.store.reader(&valid).unwrap()).unwrap();
        for body in [
            br#"{"model":"alias","input":"bad\q"}"#.as_slice(),
            b"{\"model\":\"alias\",\"input\":\"\xff\"}",
            br#"{"model":"alias","input":"ok"} []"#,
            br#"{"model":"alias","input":"ok""#,
        ] {
            let raw = fixture.raw(body);
            let before = fixture.budget.snapshot().unwrap().live;
            assert!(matches!(
                parse_ingress_document(IngressProtocol::Responses, &fixture.store, &raw, stats),
                Err(ReplayError::InvalidJson)
            ));
            assert_eq!(fixture.budget.snapshot().unwrap().live, before);
        }
    }
}

#[test]
fn disk_backing_never_gathers_a_whole_parser_input() {
    let fixture = Fixture::new(31);
    let body =
        serde_json::to_vec(&serde_json::json!({"model":"alias","input":"x".repeat(2048)})).unwrap();
    let raw = fixture.raw(&body);
    let stats = scan_ingress_document(fixture.store.reader(&raw).unwrap()).unwrap();
    assert!(fixture.store.snapshot().disk_backed);
    assert!(
        memory_parse_input(&fixture.store, &raw, stats)
            .unwrap()
            .is_none()
    );
    let (document, _workspace) =
        parse_ingress_document(IngressProtocol::Responses, &fixture.store, &raw, stats).unwrap();
    assert_eq!(document, serde_json::from_slice::<Value>(&body).unwrap());
}

#[test]
fn nested_arguments_keep_value_semantics_on_slice_disk_and_pressure_paths() {
    for threshold in [31, 1024 * 1024] {
        for pressure in [false, true] {
            let fixture = Fixture::new(threshold);
            let _pressure = pressure.then(|| {
                fixture
                    .budget
                    .reserve(
                        MemoryRole::AttemptWire,
                        fixture.budget.available_bytes().unwrap() - 32 * 1024,
                    )
                    .unwrap()
            });
            for body in [
                br#"{"array":[null,true,1.25],"text":"\uD83D\uDE80\n\\\""}"#.as_slice(),
                br#"{"number":1e999}"#,
                br#"{"x":1} []"#,
                br#"{"text":"\uD800"}"#,
                b"{\"text\":\"\xff\"}",
                br#"{"text":"unfinished"#,
            ] {
                let raw = fixture.raw(body);
                // The original serde Value is the independent syntax/number
                // authority. Sizing does not assert JSON validity.
                let stats = IngressDocumentStats {
                    structure_units: 32,
                    max_depth: 4,
                    max_string_bytes: body.len(),
                };
                let before = fixture.budget.snapshot().unwrap().live;
                let expected = serde_json::from_slice::<Value>(body);
                let actual = decode_json_argument::<Value>(&fixture.store, &raw, stats);
                let valid = decode_json_argument::<ValidModelJson>(&fixture.store, &raw, stats);
                assert_eq!(valid.is_ok(), expected.is_ok());
                match expected {
                    Ok(expected) => assert_eq!(actual.unwrap(), expected),
                    Err(_) => assert!(matches!(actual, Err(ReplayError::InvalidJson))),
                }
                assert_eq!(fixture.budget.snapshot().unwrap().live, before);
            }
        }
    }
}
