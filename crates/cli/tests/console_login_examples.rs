//! Keep copyable console sign-in examples aligned with the real CLI parser.
use std::{path::Path, process::Command};

use serde_json::Value;

fn invoke(root: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .args(arguments)
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("LOCALAPPDATA", root)
        .env("APPDATA", root)
        .env("HIROUTE_RUNTIME_DIR", root)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    let envelope = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.code().unwrap(), envelope)
}

#[test]
fn displayed_and_documented_token_examples_pass_argument_validation() {
    let root = tempfile::tempdir().unwrap();
    let page = include_str!("../../../apps/desktop/src/host/ConsoleRoot.tsx");
    let page_commands: Vec<_> = page
        .split("<code>")
        .skip(1)
        .map(|part| part.split_once("</code>").unwrap().0)
        .filter(|command| command.starts_with("hiroute console token create"))
        .collect();
    let guide = include_str!("../../../docs/web-console.md");
    let guide_commands: Vec<_> = guide
        .lines()
        .filter(|line| line.starts_with("hiroute console token create"))
        .collect();
    assert!(!page_commands.is_empty(), "missing copyable token command");
    assert!(
        !guide_commands.is_empty(),
        "missing documented token command"
    );
    for example in page_commands.into_iter().chain(guide_commands) {
        let arguments: Vec<_> = example.split_ascii_whitespace().skip(1).collect();
        let (exit, envelope) = invoke(root.path(), &arguments);
        // No daemon runs in this isolated directory. Reaching transport proves the
        // displayed arguments were accepted without creating any credentials.
        assert_eq!(exit, 6, "{example}: {envelope}");
        assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");
    }
}

#[test]
fn token_name_remains_required_and_expiry_remains_optional() {
    let root = tempfile::tempdir().unwrap();
    let (exit, envelope) = invoke(
        root.path(),
        &["console", "token", "create", "--name", "local-browser"],
    );
    assert_eq!(exit, 6, "{envelope}");
    assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");

    for options in [vec![], vec!["--expires", "7"], vec!["--name"]] {
        let mut arguments = vec!["console", "token", "create"];
        arguments.extend(options);
        let (exit, envelope) = invoke(root.path(), &arguments);
        assert_eq!(exit, 2, "{arguments:?}: {envelope}");
        assert_eq!(envelope["status"], "usage_error");
        assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
    }
}
