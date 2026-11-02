use super::*;
use hiroute_domain::delegation::{
    NativeDependencyCheckV1 as Check, NativeDependencyFailureReasonV1 as Reason,
};
use std::os::unix::fs::PermissionsExt;

fn executable(path: &std::path::Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn claude_provider_admission_observes_behavior_independently_of_release_labels() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("claude");
    let script = include_str!("test_fixtures/claude_provider.py");
    for version in ["0.0.1", "99.0.0", "3.0.0-beta+local"] {
        executable(
            &binary,
            &script.replace("VERSION = \"0.0.1\"", &format!("VERSION = {version:?}")),
        );
        ClaudeNativeIngressProbe::check_host_managed_provider(&binary, "/usr/bin:/bin").unwrap();
    }
    executable(
        &binary,
        &script
            .replace("CAPABLE = True", "CAPABLE = False")
            .replace("0.0.1", "2.1.231"),
    );
    let failure = ClaudeNativeIngressProbe::check_host_managed_provider(&binary, "/usr/bin:/bin")
        .unwrap_err();
    assert_eq!(failure.check, Check::ClaudeHostManagedProvider);
    assert_eq!(failure.reason, Reason::Unsupported);
}

#[test]
fn opencode_database_capability_isolated_from_daily_roots_and_package_version() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("opencode");
    executable(
        &binary,
        r#"#!/usr/bin/env python3
import os,sys,json
assert sys.argv[1:] == ['db','--format','json','SELECT id FROM migration ORDER BY id']
assert os.getcwd() == os.environ['HOME']
assert os.environ['OPENCODE_DB'].startswith(os.environ['HOME'] + os.sep)
assert os.environ['OPENCODE_PURE'] == 'true'
assert 'HIROUTE_RUN_TOKEN' not in os.environ
print(json.dumps([{'id':'native-format-a'}, {'id':'native-format-b'}]))
"#,
    );
    assert_eq!(
        opencode_native_format(&binary, "/usr/bin:/bin").unwrap(),
        vec!["native-format-a", "native-format-b"]
    );
    for output in [
        "[]",
        "not json",
        "[{\"id\":\"same\"},{\"id\":\"same\"}]",
        "[{\"id\":\"ok\",\"unknown\":true}]",
    ] {
        executable(&binary, &format!("#!/bin/sh\nprintf '%s' '{output}'\n"));
        let failure = opencode_native_format(&binary, "/usr/bin:/bin").unwrap_err();
        assert_eq!(failure.check, Check::OpenCodeDatabase);
    }
}
