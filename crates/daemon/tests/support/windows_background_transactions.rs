//! Native owner transaction faults and proofs around the shipped installer.
//! Exercises the Rust maintenance CLI; no fixture edits the installation journal.
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use hiroute_host_runtime::StandaloneLayout;
use hiroute_platform::windows::{lifecycle, security};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

use super::{Request, accepted, process};

fn document(path: &Path) -> Value {
    let file = security::open_owner_only(path, false).unwrap();
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 65536);
    serde_json::from_slice(&bytes).unwrap()
}

fn digest(path: &Path) -> String {
    let mut file = security::open_identity(path, false).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}

fn identity(path: &Path) -> (u32, u64) {
    security::file_identity(&security::open_identity(path, false).unwrap()).unwrap()
}

fn snapshot(layout: &StandaloneLayout) -> Value {
    let marker = document(&layout.marker_path);
    let definition = document(&layout.data_root.join("service/windows-background.json"));
    // This public package resource uses inherited native read permissions. The
    // private marker/definition/journal above always use owner-only handles.
    let manifest_path = layout.data_root.join("package-manifest.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let journal_path = layout.data_root.join("service/install-journal.json");
    let journal = if journal_path.exists() {
        document(&journal_path)
    } else {
        Value::Null
    };
    json!({
        "marker":marker,"definition":definition,"manifest":manifest,"journal":journal,
        "cli_identity":identity(&layout.home.join(".local/bin/hiroute.exe")),
        "daemon_identity":identity(&layout.home.join(".local/bin/hirouted.exe")),
        "agent_skill_identity":identity(&layout.home.join(".agents/skills/hiroute-management/SKILL.md")),
        "manifest_sha256":digest(&manifest_path)
    })
}

fn verify_published(layout: &StandaloneLayout) -> Value {
    let result = snapshot(layout);
    assert!(
        result["journal"].is_null(),
        "completed install left a recovery journal"
    );
    let root = std::path::PathBuf::from(result["marker"]["install_root"].as_str().unwrap());
    assert_eq!(
        identity(&root.join("hiroute.exe")),
        identity(&layout.home.join(".local/bin/hiroute.exe"))
    );
    assert_eq!(
        identity(&root.join("hirouted.exe")),
        identity(&layout.home.join(".local/bin/hirouted.exe"))
    );
    for (name, facts) in result["manifest"]["files"].as_object().unwrap() {
        let destination = match name.as_str() {
            "bin/hiroute.exe" => layout.home.join(".local/bin/hiroute.exe"),
            "bin/hirouted.exe" => layout.home.join(".local/bin/hirouted.exe"),
            "skills/hiroute-management/SKILL.md" => {
                let body = layout
                    .home
                    .join(".agents/skills/hiroute-management/SKILL.md");
                assert_eq!(
                    digest(
                        &layout
                            .home
                            .join(".claude/skills/hiroute-management/SKILL.md")
                    ),
                    facts["sha256"].as_str().unwrap()
                );
                body
            }
            name if name.starts_with("docs/") || name.starts_with("licenses/") => {
                layout.data_root.join(name)
            }
            _ => continue,
        };
        assert_eq!(
            digest(&destination),
            facts["sha256"].as_str().unwrap(),
            "published package entry differs from its actual manifest"
        );
    }
    result
}

pub(super) fn run(request: &Request) -> Value {
    let layout = StandaloneLayout::from_environment().unwrap();
    match request.action.as_str() {
        "transaction-snapshot" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            snapshot(&layout)
        }
        "transaction-verify" => {
            assert!(request.program.is_none() && request.arguments.is_empty());
            verify_published(&layout)
        }
        "transaction-fail-install" | "transaction-upgrade-install" => {
            let before = snapshot(&layout);
            assert_eq!(before["definition"]["manager"], "task");
            assert!(before["journal"].is_null());
            let old_root =
                std::path::PathBuf::from(before["marker"]["install_root"].as_str().unwrap());
            let old_daemon = lifecycle::retain_for_stop(
                &layout.lifecycle_endpoint(),
                &hiroute_host_runtime::standalone_daemon_binary(&old_root),
                Duration::from_secs(3),
            )
            .ok();
            // plan_files can copy this existing read-only resource for its
            // backup, but its final os.replace is denied. This destination is
            // last, after actual stable entries and packaged resources.
            let blocker: Option<File> = if request.action == "transaction-fail-install" {
                Some(
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ)
                        .open(layout.data_root.join("package-manifest.json"))
                        .unwrap(),
                )
            } else {
                None
            };
            let command = maintenance(request);
            if let Some(old) = old_daemon {
                assert!(
                    old.wait(Duration::from_secs(10)).unwrap(),
                    "installation did not stop the actual retained old business daemon"
                );
            }
            let after = snapshot(&layout);
            if blocker.is_some() {
                assert_ne!(
                    command["code"], 0,
                    "deny-delete fault did not fail the actual installer"
                );
                assert_eq!(
                    after["journal"]["schema"],
                    "hiroute.windows-install-journal/v1"
                );
                assert_eq!(after["journal"]["phase"], "prepared");
                assert_eq!(after["journal"]["operation"], "install");
                assert_eq!(after["journal"]["old_marker"], before["marker"]);
                assert_eq!(after["manifest_sha256"], before["manifest_sha256"]);
                assert_ne!(
                    after["agent_skill_identity"], before["agent_skill_identity"],
                    "failure happened before any real file publication; it cannot prove rollback coverage"
                );
            } else {
                accepted(&command);
                verify_published(&layout);
            }
            drop(blocker);
            json!({"command":command,"before":before,"after":after})
        }
        _ => panic!("unknown transaction fixture action"),
    }
}

/// Both real logged-on and alternate-token owners invoke the same private product entry.
pub(super) fn maintenance(request: &Request) -> Value {
    let uninstall = request.action == "uninstall";
    let body = if uninstall {
        assert!(request.arguments.is_empty());
        json!({"schema":"hiroute.windows-uninstall-request/v1"})
    } else {
        assert_eq!(request.arguments.len(), 1);
        json!({"schema":"hiroute.windows-install-request/v1",
            "package_directory":request.arguments[0],"autostart":false})
    };
    process(
        None,
        request.program.as_ref().unwrap(),
        &[
            OsString::from(if uninstall {
                "__windows-uninstall"
            } else {
                "__windows-install"
            }),
            OsString::from("--request-stdin"),
        ],
        Some(&serde_json::to_vec(&body).unwrap()),
        false,
        Duration::from_secs(75),
        false,
    )
}
