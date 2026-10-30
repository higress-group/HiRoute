use super::*;
use std::io::BufRead;

fn exchange_peer(resident: &mut Resident, wrong_id: bool) -> std::thread::JoinHandle<Vec<String>> {
    let (requests, writer) = pipe().unwrap();
    let (reader, mut replies) = pipe().unwrap();
    nonblocking(&reader).unwrap();
    let owned = resident.owned.as_mut().unwrap();
    owned.capability = writer;
    owned.ack = reader;
    std::thread::spawn(move || {
        let mut requests = std::io::BufReader::new(requests);
        let mut ids = Vec::new();
        loop {
            let mut line = String::new();
            if requests.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = value["registration_id"].as_str().unwrap().to_owned();
            ids.push(id.clone());
            let registered = ids.len() > 1;
            let reply_id = if wrong_id { "0".repeat(64) } else { id };
            let reply = format!(
                "{{\"schema\":\"hiroute.protected-apply-ack/v2\",\"registration_id\":\"{reply_id}\",\"registered\":{registered}}}\n"
            );
            if replies.write_all(reply.as_bytes()).is_err() {
                break;
            }
        }
        ids
    })
}

#[test]
fn busy_refusal_keeps_owned_channel_and_next_exact_registration_usable() {
    let root = tempfile::tempdir().unwrap();
    let mut resident = super::tests::recovery_resident(root.path(), false);
    let pid = resident.owned.as_ref().unwrap().child.id();
    let peer = exchange_peer(&mut resident, false);
    let digest = CanonicalDigest::of_bytes(b"reviewed operation");
    let revisions = RevisionSetV1 {
        target: 0,
        dependencies: Default::default(),
    };
    let first = resident.register_operation(&digest, &revisions, "ListSessionsV2");
    let still_healthy = resident.has_authority();
    let second = resident.register_operation(&digest, &revisions, "ListSessionsV2");
    let same_pid = resident.owned.as_ref().unwrap().child.id() == pid;
    let pending = resident.owned.as_ref().unwrap().pending_ack.is_some();
    drop(resident);
    let ids = peer.join().unwrap();
    assert_eq!(first.err().as_deref(), Some("TRUSTED_AUTHORITY_BUSY"));
    assert!(still_healthy && same_pid && !pending);
    assert!(second.is_ok());
    assert_eq!(ids.len(), 2);
    assert_ne!(
        ids[0], ids[1],
        "a later user request needs its own registration"
    );
}

#[test]
fn wrong_identity_in_a_refusal_cannot_preserve_native_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut resident = super::tests::recovery_resident(root.path(), false);
    let peer = exchange_peer(&mut resident, true);
    let result = resident.register_operation(
        &CanonicalDigest::of_bytes(b"reviewed operation"),
        &RevisionSetV1 {
            target: 0,
            dependencies: Default::default(),
        },
        "ListSessionsV2",
    );
    let healthy = resident.has_authority();
    drop(resident);
    assert_eq!(peer.join().unwrap().len(), 1);
    assert_eq!(
        result.err().as_deref(),
        Some("TRUSTED_AUTHORITY_UNAVAILABLE")
    );
    assert!(!healthy);
}
