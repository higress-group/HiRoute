//! One owner oracle for both upgrade gates; only process creation differs.
//! Current never enables privileges, stages a token or changes profile routing.
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use serde_json::Value;

use super::super::{Cleanup as TokenCleanup, Inputs, fixture, windows_background_standard};

#[derive(Clone, Copy)]
pub(in super::super) enum Owner<'a> {
    Token(&'a File),
    Current,
}

impl Owner<'_> {
    pub(in super::super) fn request(self, request: fixture::Request, timeout: Duration) -> Value {
        match self {
            Self::Token(token) => fixture::owner(token, request, timeout),
            Self::Current => windows_background_standard::current_owner(request, timeout),
        }
    }

    pub(in super::super) fn action(self, action: &str, timeout: Duration) -> Value {
        self.request(
            fixture::Request {
                action: action.into(),
                program: None,
                arguments: vec![],
            },
            timeout,
        )
    }

    pub(in super::super) fn command(self, program: &Path, arguments: Vec<String>) -> Value {
        self.request(
            fixture::Request {
                action: "command".into(),
                program: Some(program.to_owned()),
                arguments,
            },
            Duration::from_secs(90),
        )
    }

    pub(in super::super) fn cli(self, program: &Path, arguments: &[&str]) -> Value {
        if let Self::Token(token) = self {
            return fixture::cli(token, program, arguments);
        }
        let result = self.command(
            program,
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .chain([
                    "--output".into(),
                    "json".into(),
                    "--timeout".into(),
                    "45s".into(),
                ])
                .collect(),
        );
        let envelope = accepted(&result, "cli");
        assert_eq!(envelope["status"], "succeeded");
        envelope["data"].clone()
    }

    pub(in super::super) fn install(self, input: &Inputs) -> Value {
        if let Self::Token(token) = self {
            return input.install(token, false);
        }
        let result = self.request(input.install_request("install"), Duration::from_secs(90));
        accepted(&result, "install").clone()
    }

    pub(in super::super) fn transaction_install(self, input: &Inputs, action: &str) -> Value {
        self.request(input.install_request(action), Duration::from_secs(90))
    }

    pub(in super::super) fn uninstall(self, input: &Inputs, _cli: &Path) -> Value {
        let result = self.request(
            fixture::Request {
                action: "uninstall".into(),
                program: Some(input.admin_cli.clone()),
                arguments: vec![],
            },
            Duration::from_secs(90),
        );
        accepted(&result, "uninstall").clone()
    }
}

fn accepted<'a>(result: &'a Value, stage: &'static str) -> &'a Value {
    assert!(
        result["code"] == 0 && result["result"].is_object(),
        "upgrade production command failed: stage={stage}, exit={:?}, result_present={}",
        result["code"].as_i64(),
        result["result"].is_object()
    );
    &result["result"]
}

/// The existing alternate-token gate retains its original cleanup. The current
/// owner gate may uninstall only the exact record this journey actually read.
pub(in super::super) enum Cleanup<'a> {
    Token(TokenCleanup<'a>),
    Current(Box<CurrentCleanup<'a>>),
}

impl<'a> Cleanup<'a> {
    pub(in super::super) fn new(owner: Owner<'a>, input: &'a Inputs, cli: PathBuf) -> Self {
        match owner {
            Owner::Token(token) => Self::Token(TokenCleanup {
                inputs: input,
                token,
                cli,
                armed: true,
            }),
            Owner::Current => {
                let layout = StandaloneLayout::from_environment().unwrap();
                let record = verified_record(input, &layout, &cli);
                Self::Current(Box::new(CurrentCleanup {
                    input,
                    layout,
                    record,
                    cli,
                    armed: true,
                }))
            }
        }
    }

    pub(in super::super) fn upgraded(&mut self, input: &'a Inputs, cli: PathBuf) {
        match self {
            Self::Token(cleanup) => {
                cleanup.inputs = input;
                cleanup.cli = cli;
            }
            Self::Current(cleanup) => {
                cleanup.record = verified_record(input, &cleanup.layout, &cli);
                cleanup.input = input;
                cleanup.cli = cli;
            }
        }
    }

    pub(in super::super) fn disarm(&mut self) {
        match self {
            Self::Token(cleanup) => cleanup.armed = false,
            Self::Current(cleanup) => cleanup.armed = false,
        }
    }
}

fn verified_record(
    input: &Inputs,
    layout: &StandaloneLayout,
    cli: &Path,
) -> StandaloneInstallRecordV1 {
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
    assert!(
        record.install_root == cli.parent().unwrap()
            && record.install_root.file_name().unwrap() == input.version_directory.as_str()
            && record.service_definition
                == layout.data_root.join("service/windows-background.json")
            && record.target == hiroute_platform::windows::install::native_target().unwrap(),
        "standard upgrade cleanup requires its exact selected package root and definition"
    );
    record
}

pub(in super::super) struct CurrentCleanup<'a> {
    input: &'a Inputs,
    layout: StandaloneLayout,
    record: StandaloneInstallRecordV1,
    cli: PathBuf,
    armed: bool,
}
impl Drop for CurrentCleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if !owns_record(&self.layout.marker_path, &self.record) {
            eprintln!("standard upgrade cleanup preserved state: exact marker no longer matches");
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Owner::Current.uninstall(self.input, &self.cli)
        }))
        .is_err()
        {
            eprintln!("standard upgrade exact-candidate uninstall failed; preserve user state");
        }
    }
}

fn owns_record(marker: &Path, expected: &StandaloneInstallRecordV1) -> bool {
    hiroute_host_runtime::read_standalone_install_record(marker)
        .ok()
        .as_ref()
        == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::{StandaloneInstallRecordV1, owns_record};
    use hiroute_platform::windows::security;
    use std::path::PathBuf;

    #[test]
    fn standard_upgrade_cleanup_refuses_foreign_or_reparse_marker_and_preserves_state() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = std::env::temp_dir().join(format!(
            "hiroute-upgrade-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        drop(security::create_owner_only_directory(&root).unwrap());
        let root = Fixture(root);
        let marker = root.0.join("marker.json");
        let state = root.0.join("retained-user-state");
        hiroute_platform::durable::write_private_new(&state, b"owned-state-preservation").unwrap();
        let held_state = security::open_owner_only(&state, false).unwrap();
        let state_identity = security::file_identity(&held_state).unwrap();
        let state_permissions = hiroute_platform::file_security::capture(&held_state).unwrap();
        let expected = StandaloneInstallRecordV1 {
            schema_version: "hiroute.standalone-install/v1".into(),
            version: "0.3.1".into(),
            target: "x86_64-pc-windows-msvc".into(),
            install_root: root.0.join("baseline"),
            service_definition: root.0.join("windows-background.json"),
            installed_skills: vec![],
            cpa_binary: None,
            cpa_sha256: None,
        };
        expected.validate().unwrap();
        let bytes = serde_json::to_vec(&expected).unwrap();
        hiroute_platform::durable::write_private_new(&marker, &bytes).unwrap();
        assert!(owns_record(&marker, &expected));
        let mut foreign = expected.clone();
        foreign.install_root = root.0.join("foreign");
        let foreign = serde_json::to_vec(&foreign).unwrap();
        hiroute_platform::durable::write_private_atomic(&marker, &foreign).unwrap();
        assert!(!owns_record(&marker, &expected));
        assert_eq!(std::fs::read(&marker).unwrap(), foreign);
        std::fs::remove_file(&marker).unwrap();
        let target = root.0.join("external-directory");
        drop(security::create_owner_only_directory(&target).unwrap());
        let external = target.join("external-nonce");
        hiroute_platform::durable::write_private_new(&external, b"external-preserved").unwrap();
        hiroute_platform_test_support::junction::create(&marker, &target).unwrap();
        assert!(!owns_record(&marker, &expected));
        assert_eq!(std::fs::read(&external).unwrap(), b"external-preserved");
        std::fs::remove_dir(&marker).unwrap();
        assert!(!owns_record(&marker, &expected));
        assert_eq!(std::fs::read(state).unwrap(), b"owned-state-preservation");
        assert_eq!(
            security::file_identity(&held_state).unwrap(),
            state_identity
        );
        assert!(
            hiroute_platform::file_security::capture(&held_state).unwrap() == state_permissions
        );
    }
}
