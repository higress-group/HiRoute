use std::fs;
use std::path::{Path, PathBuf};

use hiroute_e2e::p0::coverage;
use hiroute_e2e::p0::schema::validate_document;
use serde_json::Value;

#[test]
fn p0_coverage_manifest_is_an_exact_projection_of_the_frozen_real_receipt_registry() {
    let root = repository_root();
    let schema = read_json(&root.join("e2e/schema/p0-gateway-coverage.schema.json"));
    let manifest = read_json(&root.join("e2e/matrix/p0-gateway-coverage.json"));
    validate_document(&schema, &manifest).unwrap();
    coverage::validate_manifest(&root, &manifest).unwrap();
    assert!(
        manifest["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|receipt| receipt.get("source_sha256").is_none())
    );

    let mut legacy = manifest;
    legacy["receipts"][0]["source_sha256"] = Value::String(format!("sha256:{}", "0".repeat(64)));
    assert!(validate_document(&schema, &legacy).is_err());
}

#[test]
fn p0_coverage_registry_rejects_missing_or_self_reported_rows() {
    let root = repository_root();
    let mut missing = coverage::manifest_value();
    missing["rows"].as_array_mut().unwrap().pop();
    assert!(coverage::validate_manifest(&root, &missing).is_err());

    let mut forged = coverage::manifest_value();
    forged["receipts"][0]["execution"] = Value::String("frozen_protocol_golden".into());
    assert!(coverage::validate_manifest(&root, &forged).is_err());

    let mut forged = coverage::manifest_value();
    forged["receipts"][0]["symbol"] = Value::String("unexecuted_symbol".into());
    assert!(coverage::validate_manifest(&root, &forged).is_err());

    let mut forged = coverage::manifest_value();
    forged["receipts"][0]["assertion_ids"][0] = Value::String("unbound_assertion".into());
    assert!(coverage::validate_manifest(&root, &forged).is_err());

    let mut forged = coverage::manifest_value();
    forged["receipts"][3]["assertion_binding"] = Value::String("declaration".into());
    assert!(coverage::validate_manifest(&root, &forged).is_err());
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
