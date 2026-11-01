use serde_json::Value;
use std::process::Command;

#[test]
fn daemon_version_reports_its_own_build_before_any_service_initialization() {
    let root = tempfile::tempdir().unwrap();
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_hirouted"))
            .arg(flag)
            .current_dir(root.path())
            .env("HOME", root.path().join("missing-home"))
            .env("HIROUTE_RUNTIME_DIR", root.path().join("missing-runtime"))
            .env(
                "HIROUTE_BUILD_SOURCE_SHA",
                "runtime-value-must-not-be-reported",
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let identity: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(identity["schema"], "hiroute.build-identity/v1");
        assert_eq!(identity["component"], "daemon");
        assert_eq!(identity["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            identity["source_revision"],
            serde_json::to_value(option_env!("HIROUTE_BUILD_SOURCE_SHA")).unwrap()
        );
        assert!(
            !String::from_utf8(output.stdout)
                .unwrap()
                .contains("runtime-value")
        );
    }
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        0,
        "version initialized service state"
    );
}
