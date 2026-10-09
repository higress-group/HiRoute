//! Hold a stale CPA manager snapshot while the lease updates its private file.
use super::*;
use crate::borrowed_codex::{BorrowedCodexAuthSpec, ManagedAuthLease};
use crate::config::{ensure_private_dir, private_atomic_write};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn explicit_recheck_syncs_fresh_version_before_stale_manager_writeback() {
    for old in [None, Some("0.147.0")] {
        check_stale_manager(old);
    }
}

fn check_stale_manager(old: Option<&str>) {
    let root = tempfile::tempdir().unwrap();
    let auth_dir = ensure_private_dir(&root.path().join("auth")).unwrap();
    let source = root.path().join("source.json");
    let source_bytes = serde_json::to_vec(&serde_json::json!({
        "auth_mode":"chatgpt", "last_refresh":"fixture",
        "tokens":{"access_token":"fixture-access", "id_token":"fixture.id",
            "account_id":"fixture-account", "refresh_token":"fixture-refresh-never-copy"}
    }))
    .unwrap();
    private_atomic_write(&source, &source_bytes).unwrap();
    let engine = root.path().join("selected-codex");
    let initial = old.map_or("exit 7".to_owned(), |v| {
        format!("printf 'codex-cli {v}\\n'")
    });
    fs::write(&engine, format!("#!/bin/sh\n{initial}\n")).unwrap();
    fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
    let spec = BorrowedCodexAuthSpec::new(&source).with_executable(engine.clone());
    let evidence = spec.inspect().unwrap();
    let mut lease = ManagedAuthLease::acquire(&auth_dir, Some(&spec)).unwrap();
    let before = lease.refresh().unwrap();
    let name = before[0].stock_file_name.clone();
    let file = auth_dir.join(&name);
    // No watcher: the manager deliberately retains the earlier account metadata.
    let mut manager: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(manager["hiroute_client_version"], serde_json::json!(old));
    fs::write(&engine, "#!/bin/sh\nprintf 'codex-cli 0.162.0\\n'\n").unwrap();
    let identities = lease.refresh_expected(Some(&evidence), None).unwrap();
    assert_eq!(identities[0].account_digest, before[0].account_digest);
    assert_eq!(identities[0].generation, before[0].generation);
    let current: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(current["hiroute_client_version"], "0.162.0");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let worker = thread::spawn(move || {
        let mut discovered = false;
        let mut patches = 0;
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let response = if header.starts_with("PATCH ") {
                patches += 1;
                let patch: Value = serde_json::from_slice(&body).unwrap();
                assert!(patch.get("access_token").is_none());
                assert!(patch.get("refresh_token").is_none());
                // Pinned CPA PatchAuthFileFields merges the request into its
                // loaded Auth, persists that entire object, then calls its hook.
                for (key, value) in patch.as_object().unwrap() {
                    if key != "name" {
                        manager[key] = value.clone();
                    }
                }
                private_atomic_write(&file, &serde_json::to_vec(&manager).unwrap()).unwrap();
                discovered = manager["hiroute_client_version"] == "0.162.0";
                "{}".to_owned()
            } else if header.starts_with("GET /v0/management/auth-files?name=") {
                String::from_utf8(super::tests::oauth_response(""))
                    .unwrap()
                    .replace("codex-a.json", &name)
            } else if header.starts_with("GET /v0/management/auth-files/models?") {
                if discovered {
                    r#"{"models":[{"id":"hiroute-codex-current/fresh-model"}]}"#
                } else {
                    r#"{"models":[]}"#
                }
                .to_owned()
            } else if header.starts_with("GET /v1/models ") {
                if discovered {
                    r#"{"data":[{"id":"hiroute-codex-current/fresh-model"}]}"#
                } else {
                    r#"{"data":[]}"#
                }
                .to_owned()
            } else {
                panic!("unexpected control request");
            };
            write!(stream, "HTTP/1.1 200 OK\r\nX-CPA-Version: 8.0.4-hiroute.2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
        (manager, patches)
    });
    let result = StockCpaControlPlane.discover_and_pin(
        address,
        &auth_dir,
        &identities,
        &InstanceSecrets::generate().unwrap(),
        "8.0.4-hiroute.2",
        Duration::from_secs(2),
        true,
    );
    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(address);
    let (manager, patches) = worker.join().unwrap();
    assert_eq!(patches, 1);
    assert_eq!(
        manager["hiroute_client_version"], "0.162.0",
        "stale manager reverted the fresh version"
    );
    let after: Value =
        serde_json::from_slice(&fs::read(auth_dir.join(&identities[0].stock_file_name)).unwrap())
            .unwrap();
    assert_eq!(after["hiroute_client_version"], "0.162.0");
    assert!(after.get("refresh_token").is_none());
    let accounts = result.unwrap();
    assert_eq!(
        accounts[0].observed_model_ids,
        BTreeSet::from(["fresh-model".into()])
    );
    assert_eq!(fs::read(source).unwrap(), source_bytes);
    assert!(ManagedAuthLease::acquire(&auth_dir, Some(&spec)).is_err());
}
