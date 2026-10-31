use super::*;
fn auth() -> (tempfile::TempDir, ConsoleAuth) {
    let root = tempfile::tempdir().unwrap();
    let auth = ConsoleAuth::open(root.path()).unwrap();
    (root, auth)
}
fn local_target() -> LaunchTarget {
    LaunchTarget::new("127.0.0.1:5838".parse().unwrap(), true)
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
fn regression_all_installed_product_commands_can_be_audited_without_expiring_a_session() {
    let (_root, mut auth) = auth();
    let code = auth.create_launch(local_target()).unwrap();
    let session = auth
        .exchange_launch(&code, peer(), &local_target().origin)
        .unwrap();
    for command in hiroute_product_session::catalog::commands() {
        auth.audit_command(&session.session, "1234567890abcdef", &command.name, None)
            .unwrap_or_else(|error| {
                panic!("admitted command {} was rejected: {error:?}", command.name)
            });
    }
    for invalid in [
        "preview-save",
        "preview/save",
        "PREVIEW",
        "preview save",
        "preview\nsave",
    ] {
        assert_eq!(
            auth.audit_command(&session.session, "1234567890abcdef", invalid, None),
            Err(AuthError::Invalid)
        );
    }
    assert!(auth.authenticate(&headers(&session.bearer), peer()).is_ok());
}

#[test]
fn launch_is_loopback_only_short_lived_and_single_use() {
    let (_root, mut a) = auth();
    let code = a.create_launch(local_target()).unwrap();
    assert!(matches!(
        a.exchange_launch(&code, "192.0.2.1".parse().unwrap(), &local_target().origin),
        Err(AuthError::Invalid)
    ));
    let session = a
        .exchange_launch(&code, peer(), &local_target().origin)
        .unwrap();
    assert!(a.authenticate(&headers(&session.bearer), peer()).is_ok());
    assert!(
        a.exchange_launch(&code, peer(), &local_target().origin)
            .is_err()
    );
    let code = a.create_launch(local_target()).unwrap();
    a.launches.get_mut(&hash(&code)).unwrap().expires = now().unwrap() - 1;
    assert!(
        a.exchange_launch(&code, peer(), &local_target().origin)
            .is_err()
    );
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
        let code = a.create_launch(local_target()).unwrap();
        let session = a
            .exchange_launch(&code, peer(), &local_target().origin)
            .unwrap();
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
    let code = a.create_launch(local_target()).unwrap();
    let local = "127.0.0.2".parse().unwrap();
    let session = a
        .exchange_launch(&code, local, &local_target().origin)
        .unwrap();
    assert!(a.authenticate(&headers(&session.bearer), local).is_ok());
    // A separate local peer limit still applies; isolation is not an exemption.
    assert_eq!(
        a.exchange_launch(&code, peer(), &local_target().origin)
            .err(),
        Some(AuthError::RateLimited)
    );
}
#[test]
fn bearer_is_explicit_and_capacity_evicts_the_least_recently_used_session() {
    let (root, mut a) = auth();
    assert!(a.authenticate(&HeaderMap::new(), peer()).is_err());
    let mut sessions = vec![];
    for _ in 0..16 {
        let code = a.create_launch(local_target()).unwrap();
        sessions.push(
            a.exchange_launch(&code, peer(), &local_target().origin)
                .unwrap(),
        );
    }
    // Refresh the oldest session: the second session must now be evicted.
    assert!(
        a.authenticate(&headers(&sessions[0].bearer), peer())
            .is_ok()
    );
    let code = a.create_launch(local_target()).unwrap();
    let newest = a
        .exchange_launch(&code, peer(), &local_target().origin)
        .unwrap();
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

#[test]
fn network_launch_is_origin_port_bound_and_mismatches_do_not_consume_it() {
    let (root, mut auth) = auth();
    let remote: IpAddr = "198.51.100.20".parse().unwrap();
    for address in ["192.0.2.1:5838", "[2001:db8::1]:80"] {
        let target = LaunchTarget::new(address.parse().unwrap(), false);
        let code = auth.create_launch(target.clone()).unwrap();
        for wrong in [
            "http://192.0.2.1:5839",
            "http://127.0.0.1:5838",
            "http://evil.example:5838",
        ] {
            assert_eq!(
                auth.exchange_launch(&code, remote, wrong).err(),
                Some(AuthError::Invalid)
            );
        }
        let session = auth.exchange_launch(&code, remote, &target.origin).unwrap();
        assert!(auth.authenticate(&headers(&session.bearer), remote).is_ok());
        assert_eq!(
            auth.exchange_launch(&code, remote, &target.origin).err(),
            Some(AuthError::Invalid)
        );
        let audit = std::fs::read_to_string(root.path().join("console/audit.jsonl")).unwrap();
        assert!(!audit.contains(code.as_str()));
        assert!(!audit.contains(&session.bearer));
    }
}
