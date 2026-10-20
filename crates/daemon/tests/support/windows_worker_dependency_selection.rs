//! Discovery owns canonical tuples; fixture pins identify files, never replace discovery.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf, Prefix};

use hiroute_application_api::{
    ErrorCode, WorkerDependenciesSelectRequestV1, WorkerDependenciesViewV1,
    WorkerDependencyCandidateStateV1, WorkerDependencyComponentV1, WorkerHarnessV1,
};
use hiroute_platform::windows::security;
use serde_json::Value;

const PROJECT: &[u8] = include_bytes!("../../../../tools/windows-agent-fixtures/package.json");
const LOCK: &[u8] = include_bytes!("../../../../tools/windows-agent-fixtures/package-lock.json");
const PI_ENTRY: &str = "node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js";

pub(crate) struct DiscoveryEnvironment {
    pub(crate) values: Vec<(OsString, OsString)>,
    _held: Vec<File>,
}

pub(crate) fn discovery_environment() -> DiscoveryEnvironment {
    let Some(cli) = std::env::var_os("HIROUTE_WORKER_PI_BINARY") else {
        return DiscoveryEnvironment {
            values: Vec::new(),
            _held: Vec::new(),
        };
    };
    let root = std::env::var_os("HIROUTE_WINDOWS_SOURCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("compiled fixture project must have its source ancestors")
                .to_owned()
        });
    let project = root.join("tools/windows-agent-fixtures");
    let expected = project.join(PI_ENTRY);
    let pinned = PathBuf::from(cli);
    assert!(pinned.is_absolute(), "fixed Pi entry must be absolute");
    let mut held = Vec::new();
    for path in expected.parent().unwrap().ancestors() {
        held.push(retain_directory(path));
    }
    for (path, bytes) in [
        (project.join("package.json"), PROJECT),
        (project.join("package-lock.json"), LOCK),
    ] {
        let identity = security::open_identity(&path, false)
            .expect("fixed discovery source identity must be held");
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .expect("fixed discovery source must be readable");
        let source_identity =
            security::file_identity(&identity).expect("fixed discovery source FileID query failed");
        assert!(
            security::file_identity(&file).is_ok_and(|current| current == source_identity)
                && file
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() == bytes.len() as u64),
            "fixed discovery source identity or size differs"
        );
        let mut actual = Vec::new();
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut actual)
            .expect("fixed discovery source read failed");
        assert!(actual == bytes, "fixed discovery source bytes differ");
        held.extend([identity, file]);
    }
    let pinned = security::open_identity(&pinned, false)
        .expect("fixed selected Pi entry must be reparse-free");
    let expected = security::open_identity(&expected, false)
        .expect("locked fixture Pi entry must be reparse-free");
    let identity = security::file_identity(&pinned).expect("fixed selected Pi FileID query failed");
    assert!(
        security::file_identity(&expected).is_ok_and(|current| current == identity),
        "selected Pi entry differs from the locked fixture project"
    );
    held.extend([pinned, expected]);
    DiscoveryEnvironment {
        values: vec![("NPM_CONFIG_PREFIX".into(), project.into_os_string())],
        _held: held,
    }
}

fn retain_directory(path: &Path) -> File {
    let identity = security::open_identity(path, true)
        .expect("fixed dependency ancestor must be reparse-free");
    let held = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS, no share-delete.
        .open(path)
        .expect("fixed dependency ancestor lease failed");
    let identity =
        security::file_identity(&identity).expect("fixed dependency ancestor FileID query failed");
    assert!(
        security::file_identity(&held).is_ok_and(|current| current == identity),
        "fixed dependency ancestor changed before retention"
    );
    held
}

pub(super) struct Selection {
    pub request: WorkerDependenciesSelectRequestV1,
    pub ordinary: WorkerDependenciesSelectRequestV1,
    _held: Vec<File>,
}

pub(super) fn select(discovered: &Value, cli: &Path, node: &Path) -> Selection {
    let view: WorkerDependenciesViewV1 = serde_json::from_value(discovered["data"].clone())
        .unwrap_or_else(|_| panic!("actual Worker discovery shape differs"));
    assert!(view.valid(), "actual Worker discovery is invalid");
    let revision = view
        .selection_revisions
        .iter()
        .find(|entry| entry.harness == WorkerHarnessV1::Pi)
        .expect("actual discovery must report the Pi selection revision")
        .revision;
    let mut held = Vec::new();
    let cli_path = candidate(
        &view,
        WorkerHarnessV1::Pi,
        WorkerDependencyComponentV1::Cli,
        cli,
        &mut held,
    );
    let node_path = candidate(
        &view,
        WorkerHarnessV1::Pi,
        WorkerDependencyComponentV1::Node,
        node,
        &mut held,
    );
    let request = WorkerDependenciesSelectRequestV1 {
        harness: WorkerHarnessV1::Pi,
        adapter_path: None,
        cli_path,
        node_path: Some(node_path),
        expected_selection_revision: revision,
    };
    let ordinary = |path: &Path| {
        assert!(
            matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))),
            "old regression input must use an ordinary drive spelling"
        );
        path.to_str()
            .expect("fixed dependency path must be Unicode")
            .to_owned()
    };
    let ordinary = WorkerDependenciesSelectRequestV1 {
        cli_path: ordinary(cli),
        node_path: Some(ordinary(node)),
        ..request.clone()
    };
    assert!(
        request.valid() && ordinary.valid() && request != ordinary,
        "same-file canonical tuple must distinguish the old ordinary fixture"
    );
    Selection {
        request,
        ordinary,
        _held: held,
    }
}

pub(super) fn select_native(discovered: &Value, harness: WorkerHarnessV1, cli: &Path) -> Selection {
    let view: WorkerDependenciesViewV1 =
        serde_json::from_value(discovered["data"].clone()).unwrap();
    assert!(view.valid());
    let revision = view
        .selection_revisions
        .iter()
        .find(|row| row.harness == harness)
        .unwrap()
        .revision;
    let mut held = Vec::new();
    let request = WorkerDependenciesSelectRequestV1 {
        harness,
        cli_path: candidate(
            &view,
            harness,
            WorkerDependencyComponentV1::Cli,
            cli,
            &mut held,
        ),
        adapter_path: None,
        node_path: None,
        expected_selection_revision: revision,
    };
    assert!(request.valid());
    let ordinary = WorkerDependenciesSelectRequestV1 {
        cli_path: cli.to_str().unwrap().to_owned(),
        ..request.clone()
    };
    Selection {
        request,
        ordinary,
        _held: held,
    }
}

fn candidate(
    view: &WorkerDependenciesViewV1,
    harness: WorkerHarnessV1,
    component: WorkerDependencyComponentV1,
    pinned: &Path,
    held: &mut Vec<File>,
) -> String {
    let pin = security::open_identity(pinned, false)
        .expect("fixed dependency identity must be held without reparses");
    let identity = security::file_identity(&pin).expect("fixed dependency FileID query failed");
    held.push(pin);
    let mut matches = Vec::new();
    for row in &view.candidates {
        if row.harness != harness
            || row.component != component
            || row.state != WorkerDependencyCandidateStateV1::Found
        {
            continue;
        }
        let path = Path::new(&row.path);
        let Ok(file) = security::open_identity(path, false) else {
            continue;
        };
        if security::file_identity(&file).ok() != Some(identity) {
            continue;
        }
        for ancestor in path.parent().unwrap().ancestors() {
            held.push(retain_directory(ancestor));
        }
        let current = security::open_identity(path, false)
            .expect("canonical candidate must reopen under its retained ancestors");
        assert!(
            security::file_identity(&current).is_ok_and(|current| current == identity),
            "canonical candidate identity changed before submission"
        );
        assert!(
            std::fs::canonicalize(path)
                .is_ok_and(|canonical| canonical.as_os_str() == path.as_os_str()),
            "discovery must supply the exact canonical dependency spelling"
        );
        matches.push(row.path.clone());
        held.extend([file, current]);
    }
    assert!(
        matches.len() == 1,
        "discovery must identify one exact fixed dependency candidate"
    );
    matches.pop().unwrap()
}

pub(super) fn command_label(args: &[&str]) -> &'static str {
    [
        (
            "worker.dependencies.select",
            &["worker", "dependencies", "select"][..],
        ),
        ("worker.exec", &["worker", "exec"][..]),
        ("worker.wait", &["worker", "wait"][..]),
        ("worker.result", &["worker", "result"][..]),
        ("worker.continue", &["worker", "continue"][..]),
        (
            "agents.collaboration.configure",
            &["agents", "collaboration", "configure"][..],
        ),
        ("agents.check", &["agents", "check"][..]),
    ]
    .into_iter()
    .find_map(|(label, prefix)| args.starts_with(prefix).then_some(label))
    .unwrap_or("other-public-worker-command")
}

pub(super) fn message_key(value: &Value) -> String {
    use hiroute_domain::delegation::{
        NativeDependencyCheckV1 as Check, NativeDependencyFailureReasonV1 as Reason,
        NativeDependencyFailureV1,
    };
    let key = value["error"]["message_key"].as_str();
    if let Ok(code) = serde_json::from_value::<ErrorCode>(value["error"]["code"].clone())
        && key == Some(code.message_key())
    {
        return code.message_key().to_owned();
    }
    for fixed in [
        "worker.dependencies.missing",
        "worker.dependencies.invalid",
        "worker.dependencies.unavailable",
    ] {
        if key == Some(fixed) {
            return fixed.to_owned();
        }
    }
    for check in [
        Check::ClaudeVersion,
        Check::PiPackage,
        Check::PiNodeVersion,
        Check::PiSdk,
    ] {
        for reason in [
            Reason::Unavailable,
            Reason::Timeout,
            Reason::ProcessFailed,
            Reason::CleanupFailed,
            Reason::InvalidOutput,
            Reason::Unsupported,
        ] {
            let fixed = NativeDependencyFailureV1 { check, reason }.message_key();
            if key == Some(fixed.as_str()) {
                return fixed;
            }
        }
    }
    "unclassified-key".to_owned()
}

pub(super) fn error_code(value: &Value) -> String {
    serde_json::from_value::<ErrorCode>(value["error"]["code"].clone())
        .ok()
        .and_then(|code| serde_json::to_value(code).ok())
        .and_then(|code| code.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unclassified-code".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn closed_worker_diagnostic_never_returns_a_path_payload_or_unknown_key() {
        assert_eq!(
            command_label(&["worker", "dependencies", "select"]),
            "worker.dependencies.select"
        );
        assert_eq!(
            command_label(&["private-account-path"]),
            "other-public-worker-command"
        );
        assert_eq!(command_label(&["worker", "result"]), "worker.result");
        for value in [
            json!({"error":{"code":"INVALID_ARGUMENTS","message_key":"cli.error.invalid_arguments"}}),
            json!({"error":{"code":"CAPABILITY_UNAVAILABLE","message_key":"worker.dependencies.pi_sdk.timeout"}}),
        ] {
            assert_eq!(
                message_key(&value),
                value["error"]["message_key"].as_str().unwrap()
            );
        }
        for value in [
            json!({"error":{"message_key":"private-auth-value"}}),
            json!({"error":{"code":"private-auth-value"}}),
        ] {
            assert_eq!(message_key(&value), "unclassified-key");
            assert_eq!(error_code(&value), "unclassified-code");
        }
    }

    #[test]
    fn discovery_selection_requires_one_exact_held_native_file_identity() {
        // Actual filesystem identities test the chooser, not SDK/account acceptance.
        use hiroute_application_api::{
            WORKER_DEPENDENCIES_VIEW_SCHEMA_V1, WorkerDependencyCandidateSourceV1,
            WorkerDependencyCandidateV1, WorkerDependencySelectionRevisionV1,
        };
        use std::io::Write;
        struct Namespace(PathBuf);
        impl Drop for Namespace {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let nonce: String = nonce.iter().map(|value| format!("{value:02x}")).collect();
        let root = hiroute_platform::windows::background::local_app_data()
            .unwrap()
            .join(format!("hiroute-worker-discovery-{nonce}"));
        drop(security::create_owner_only_directory(&root).unwrap());
        let namespace = Namespace(root);
        let cli = namespace.0.join("cli 中文 % ! ' &.js");
        let node = namespace.0.join("node.exe");
        let foreign = namespace.0.join("same-bytes-foreign.js");
        for path in [&cli, &node, &foreign] {
            let mut file = security::create_owner_only_file(path).unwrap();
            file.write_all(b"native chooser fixture; not executable SDK evidence")
                .unwrap();
            file.sync_all().unwrap();
        }
        let row = |component, path: &Path| WorkerDependencyCandidateV1 {
            harness: WorkerHarnessV1::Pi,
            component,
            path: std::fs::canonicalize(path)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
            source: WorkerDependencyCandidateSourceV1::NpmGlobal,
            state: WorkerDependencyCandidateStateV1::Found,
            reason_code: None,
        };
        let mut view = WorkerDependenciesViewV1 {
            schema: WORKER_DEPENDENCIES_VIEW_SCHEMA_V1.into(),
            selection_revisions: vec![WorkerDependencySelectionRevisionV1 {
                harness: WorkerHarnessV1::Pi,
                revision: 3,
            }],
            candidates: vec![
                row(WorkerDependencyComponentV1::Cli, &cli),
                row(WorkerDependencyComponentV1::Node, &node),
            ],
            selected: Vec::new(),
            install_hints: Vec::new(),
        };
        fn data(view: &WorkerDependenciesViewV1) -> Value {
            json!({"data":view})
        }
        let selection = select(&data(&view), &cli, &node);
        assert_eq!(selection.request.expected_selection_revision, 3);
        assert_ne!(selection.request, selection.ordinary);
        assert!(selection.request.valid() && selection.ordinary.valid());
        drop(selection);
        view.candidates[0] = row(WorkerDependencyComponentV1::Cli, &foreign);
        assert!(std::panic::catch_unwind(|| select(&data(&view), &cli, &node)).is_err());
        view.candidates[0] = row(WorkerDependencyComponentV1::Cli, &cli);
        view.candidates.push(view.candidates[0].clone());
        assert!(std::panic::catch_unwind(|| select(&data(&view), &cli, &node)).is_err());
    }
}
