//! Real Resident/daemon exchanges; synthetic private inputs, no user state.
use super::*;
use hiroute_application_api::ClientServiceStatusV1;
use std::os::unix::net::UnixStream;

fn daemon() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted")
}

#[test]
fn slow_local_input_cannot_starve_protected_registration() {
    let root = tempfile::Builder::new()
        .prefix("hr187-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let mut resident = Resident::open(root.path(), &daemon()).unwrap();
    let client = resident.client.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let status = || {
        let envelope = runtime
            .block_on(client.query::<_, ClientServiceStatusV1>(
                "GetClientServiceStatus",
                "authority-regression-status",
                &serde_json::json!({}),
            ))
            .unwrap();
        assert!(envelope.error.is_none());
        envelope.data.unwrap()
    };
    let before = status();
    let input_path = hiroute_host_runtime::protected_input_socket(&root.path().join("run"));
    let mut slow_input = UnixStream::connect(input_path).unwrap();
    // Keep a non-terminated public input active beyond the unchanged launcher deadline.
    // The old per-read socket timeout restarts on each fragment and blocks the ACK loop.
    slow_input.write_all(b"{").unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            for _ in 0..12 {
                std::thread::sleep(Duration::from_millis(250));
                if slow_input.write_all(b" ").is_err() {
                    break;
                }
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        let registered = resident.register_operation(
            &CanonicalDigest::of_bytes(b"synthetic observation"),
            &before.revisions,
            "ListSessionsV2",
        );
        let after = status();
        assert_eq!(after.revisions, before.revisions);
        assert!(after.recovery_ready);
        assert!(registered.is_ok(), "launcher registration: {:?}", registered.err());
        assert!(resident.has_authority());
    });
}
