use super::*;
fn auth() -> (tempfile::TempDir, ConsoleAuth) {
    let root = tempfile::tempdir().unwrap();
    let auth = ConsoleAuth::open(root.path()).unwrap();
    (root, auth)
}
fn peer() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}
fn headers(bearer: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        http::header::AUTHORIZATION,
        format!("Bearer {bearer}").parse().unwrap(),
    );
    h
}
#[test]
fn launch_is_loopback_only_short_lived_and_single_use() {
    let (_root, mut a) = auth();
    let code = a.create_launch().unwrap();
    assert!(matches!(
        a.exchange_launch(&code, "192.0.2.1".parse().unwrap()),
        Err(AuthError::Invalid)
    ));
    let session = a.exchange_launch(&code, peer()).unwrap();
    assert!(a.authenticate(&headers(&session.bearer), peer()).is_ok());
    assert!(a.exchange_launch(&code, peer()).is_err());
    let code = a.create_launch().unwrap();
    a.launches.insert(hash(&code), now().unwrap() - 1);
    assert!(a.exchange_launch(&code, peer()).is_err());
}
#[test]
fn token_revocation_ends_sessions_and_disk_never_contains_credentials() {
    let (root, mut a) = auth();
    let token = a.create_token("Test".into(), None).unwrap();
    let session = a.exchange_token(&token.token, peer()).unwrap();
    a.audit_command(
        &session.session,
        "1234567890abcdef",
        "observation_read",
        Some("operation-one"),
    )
    .unwrap();
    a.revoke_token(&token.metadata.id).unwrap();
    assert!(a.authenticate(&headers(&session.bearer), peer()).is_err());
    for entry in std::fs::read_dir(root.path().join("console")).unwrap() {
        let bytes = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!bytes.contains(&token.token));
        assert!(!bytes.contains(&session.bearer));
    }
    let mut reopened = ConsoleAuth::open(root.path()).unwrap();
    assert!(reopened.exchange_token(&token.token, peer()).is_err());
}
#[test]
fn idle_absolute_and_credential_expiry_end_authority() {
    let (_root, mut a) = auth();
    for absolute in [false, true] {
        let code = a.create_launch().unwrap();
        let session = a.exchange_launch(&code, peer()).unwrap();
        let entry = a.sessions.get_mut(&hash(&session.bearer)).unwrap();
        if absolute {
            entry.created = now().unwrap() - ABSOLUTE_TTL;
        } else {
            entry.last_used = now().unwrap() - IDLE_TTL;
        }
        assert!(a.authenticate(&headers(&session.bearer), peer()).is_err());
    }
    let token = a.create_token("Expired".into(), Some(1)).unwrap();
    a.tokens[0].metadata.expires = Some(now().unwrap() - 1);
    assert!(a.exchange_token(&token.token, peer()).is_err());
}
#[test]
fn failed_exchanges_have_per_peer_and_global_bounded_limits() {
    let (_root, mut a) = auth();
    for _ in 0..8 {
        assert_eq!(
            a.exchange_token("invalid", peer()).err(),
            Some(AuthError::Invalid)
        );
    }
    assert_eq!(
        a.exchange_token("invalid", peer()).err(),
        Some(AuthError::RateLimited)
    );
    for i in 0..64 {
        let ip = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, i + 1));
        assert_eq!(
            a.exchange_token("invalid", ip).err(),
            Some(AuthError::Invalid)
        );
    }
    assert_eq!(
        a.exchange_token("invalid", "198.51.100.1".parse().unwrap())
            .err(),
        Some(AuthError::RateLimited)
    );
    assert_eq!(a.failures.len(), 72);
    let code = a.create_launch().unwrap();
    let local = "127.0.0.2".parse().unwrap();
    let session = a.exchange_launch(&code, local).unwrap();
    assert!(a.authenticate(&headers(&session.bearer), local).is_ok());
    // A separate local peer limit still applies; isolation is not an exemption.
    assert_eq!(
        a.exchange_launch(&code, peer()).err(),
        Some(AuthError::RateLimited)
    );
}
#[test]
fn bearer_is_explicit_and_capacity_evicts_the_least_recently_used_session() {
    let (root, mut a) = auth();
    assert!(a.authenticate(&HeaderMap::new(), peer()).is_err());
    let mut sessions = vec![];
    for _ in 0..16 {
        let code = a.create_launch().unwrap();
        sessions.push(a.exchange_launch(&code, peer()).unwrap());
    }
    // Refresh the oldest session: the second session must now be evicted.
    assert!(
        a.authenticate(&headers(&sessions[0].bearer), peer())
            .is_ok()
    );
    let code = a.create_launch().unwrap();
    let newest = a.exchange_launch(&code, peer()).unwrap();
    assert_eq!(a.sessions.len(), 16);
    assert!(
        a.authenticate(&headers(&sessions[0].bearer), peer())
            .is_ok()
    );
    assert!(
        a.authenticate(&headers(&sessions[1].bearer), peer())
            .is_err()
    );
    assert!(a.authenticate(&headers(&newest.bearer), peer()).is_ok());
    assert!(!a.session_active(&sessions[1].session.id));
    assert!(
        std::fs::read_to_string(root.path().join("console/audit.jsonl"))
            .unwrap()
            .contains("session_evicted")
    );
    let b = ConsoleAuth::open(root.path()).unwrap();
    assert!(b.sessions.is_empty());
}
#[test]
fn invalid_credentials_cannot_evict_sessions_and_local_failures_do_not_block_remote_login() {
    let (_root, mut a) = auth();
    let token = a.create_token("Remote".into(), None).unwrap();
    for _ in 0..16 {
        a.exchange_token(&token.token, peer()).unwrap();
    }
    let before: Vec<_> = a.sessions.keys().cloned().collect();
    assert_eq!(
        a.exchange_token("invalid", peer()).err(),
        Some(AuthError::Invalid)
    );
    assert_eq!(a.sessions.keys().cloned().collect::<Vec<_>>(), before);
    for i in 1..=63 {
        let ip = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, i + 1));
        assert_eq!(
            a.exchange_token("invalid", ip).err(),
            Some(AuthError::Invalid)
        );
    }
    assert_eq!(
        a.exchange_token("invalid", "127.0.1.1".parse().unwrap())
            .err(),
        Some(AuthError::RateLimited)
    );
    assert!(
        a.exchange_token(&token.token, "192.0.2.1".parse().unwrap())
            .is_ok()
    );
    // The failure window expires without restarting the service.
    let old = now().unwrap() - 61;
    for (at, _) in &mut a.failures {
        *at = old;
    }
    assert!(a.exchange_token(&token.token, peer()).is_ok());
}
#[cfg(unix)]
#[test]
fn foreign_modes_links_and_corrupt_store_are_preserved_and_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (root, mut a) = auth();
    a.create_token("One".into(), None).unwrap();
    let path = root.path().join("console/tokens.json");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ConsoleAuth::open(root.path()).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(&path).unwrap();
    let foreign = root.path().join("foreign");
    std::fs::write(&foreign, &bytes).unwrap();
    symlink(&foreign, &path).unwrap();
    assert!(ConsoleAuth::open(root.path()).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), bytes);
}
