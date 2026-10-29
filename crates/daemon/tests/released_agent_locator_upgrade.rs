//! Exact released producers exercise the ordinary three-store upgrade without model calls.
#![cfg(target_os = "linux")]

#[test]
#[ignore = "downloads checksum-pinned public releases and starts isolated fixture daemons"]
fn supported_released_agent_stores_upgrade_and_reopen() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let daemon = std::path::Path::new(env!("CARGO_BIN_EXE_hirouted"));
    let result = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("crates/daemon/tests/support/released_agent_locator_upgrade.py"))
        .arg(&root)
        .env(
            "HIROUTE_VALIDATION_PRODUCT_BIN_DIR",
            daemon.parent().unwrap(),
        )
        .status()
        .unwrap();
    assert!(result.success(), "released Agent locator upgrade failed");
}
