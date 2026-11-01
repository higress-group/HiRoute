use serde_json::Value;
use std::process::Command;

#[test]
fn version_is_offline_and_reports_the_cli_build_in_every_output_mode() {
    let root = tempfile::tempdir().unwrap();
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .args(args)
            .current_dir(root.path())
            .env("HOME", root.path().join("missing-home"))
            .env("HIROUTE_RUNTIME_DIR", root.path().join("missing-runtime"))
            .env(
                "HIROUTE_BUILD_SOURCE_SHA",
                "runtime-value-must-not-be-reported",
            )
            .output()
            .unwrap()
    };
    for args in [
        &["version"][..],
        &["--version", "--output", "json"],
        &["-V", "--json"],
    ] {
        let output = invoke(args);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["data"]["schema"], "hiroute.build-identity/v1");
        assert_eq!(envelope["data"]["component"], "cli");
        assert_eq!(envelope["data"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            envelope["data"]["source_revision"],
            serde_json::to_value(option_env!("HIROUTE_BUILD_SOURCE_SHA")).unwrap()
        );
        assert_eq!(envelope["data"]["os"], std::env::consts::OS);
        assert!(
            !String::from_utf8(output.stdout)
                .unwrap()
                .contains("runtime-value")
        );
    }
    for args in [
        &["--version"][..],
        &["version", "--output", "text"],
        &["-V"],
    ] {
        let output = invoke(args);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("HiRoute cli ") && text.contains("Source revision:"));
    }
    let quiet = invoke(&["version", "--output", "quiet"]);
    assert_eq!(
        String::from_utf8(quiet.stdout).unwrap(),
        format!("{}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(invoke(&["version", "unexpected"]).status.code(), Some(2));
    assert!(invoke(&["version", "--help"]).status.success());
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        0,
        "version created runtime state"
    );
}
