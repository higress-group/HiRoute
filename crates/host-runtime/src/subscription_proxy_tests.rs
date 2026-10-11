use super::*;

fn private_root() -> tempfile::TempDir {
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
        builder
    };
    builder.tempdir().unwrap()
}

fn manual(url: &str) -> SubscriptionProxyPolicy {
    SubscriptionProxyPolicy::Manual {
        url: url.into(),
        no_proxy: "alibaba-inc.com".into(),
    }
}
#[test]
fn manual_and_direct_replace_ambient_proxies_including_case_conflicts() {
    let inherited = || {
        [
            ("http_proxy".into(), "http://wrong:9".into()),
            ("HTTPS_PROXY".into(), "http://other:9".into()),
            ("ANTHROPIC_AUTH_TOKEN".into(), "private".into()),
        ]
    };
    let value = manual("http://127.0.0.1:1187")
        .resolve(inherited())
        .unwrap();
    let vars: std::collections::BTreeMap<_, _> = value.variables().collect();
    assert_eq!(vars.len(), 3);
    assert_eq!(
        vars[std::ffi::OsStr::new("HTTPS_PROXY")],
        "http://127.0.0.1:1187"
    );
    assert_eq!(
        vars[std::ffi::OsStr::new("NO_PROXY")],
        "localhost,127.0.0.1,::1,alibaba-inc.com"
    );
    assert_eq!(
        SubscriptionProxyPolicy::Direct
            .resolve(inherited())
            .unwrap()
            .variables()
            .count(),
        0
    );
    assert_eq!(
        SubscriptionProxyPolicy::Inherit
            .resolve(inherited())
            .unwrap()
            .variables()
            .count(),
        2
    );
}
#[test]
fn rejects_unsupported_and_credential_urls_without_echoing_them() {
    for url in [
        "socks5://localhost:1186",
        "http://user:password@localhost:1187",
        "http://localhost:0",
        "http://localhost:bad",
        "http://localhost/path",
        "http://localhost?secret",
        "http://localhost#secret",
        " http://localhost",
        "http://localhost\n",
    ] {
        assert!(manual(url).validate().is_err(), "{url}");
        assert_eq!(
            manual(url).validate().unwrap_err().to_string(),
            "SUBSCRIPTION_PROXY_INVALID_OR_UNAVAILABLE"
        );
    }
    assert!(
        serde_json::from_str::<SubscriptionProxyPolicy>(
            r#"{"mode":"direct","url":"http://localhost"}"#
        )
        .is_err()
    );
}
#[test]
fn persists_desired_and_binds_applied_to_exact_launch_snapshot() {
    let root = private_root();
    let store = SubscriptionProxyStore::new(root.path());
    assert_eq!(
        store.load().unwrap().policy,
        SubscriptionProxyPolicy::Inherit
    );
    assert!(!store.view().unwrap().applied);
    let old = store.configure(manual("http://127.0.0.1:1187")).unwrap();
    store.mark_applied(&old).unwrap();
    assert!(store.view().unwrap().applied);
    let new = store.configure(SubscriptionProxyPolicy::Direct).unwrap();
    store.mark_applied(&old).unwrap();
    assert!(!store.view().unwrap().applied);
    store.mark_applied(&new).unwrap();
    assert!(store.view().unwrap().applied);
    store.clear_applied().unwrap();
    assert!(!store.view().unwrap().applied);
    assert_eq!(store.load().unwrap(), new);
}
#[test]
#[cfg(unix)]
fn accepts_accessible_modes_but_refuses_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = private_root();
    let store = SubscriptionProxyStore::new(root.path());
    store.configure(SubscriptionProxyPolicy::Direct).unwrap();
    let path = root.path().join("subscription-proxy.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        store.load().unwrap().policy,
        SubscriptionProxyPolicy::Direct
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    store.configure(SubscriptionProxyPolicy::Inherit).unwrap();
    fs::remove_file(&path).unwrap();
    symlink("missing", &path).unwrap();
    assert!(store.load().is_err());
    assert!(store.configure(SubscriptionProxyPolicy::Inherit).is_err());
}

#[test]
fn safe_corrupt_record_can_be_replaced_through_product_settings() {
    let root = private_root();
    let store = SubscriptionProxyStore::new(root.path());
    store.configure(SubscriptionProxyPolicy::Direct).unwrap();
    fs::write(root.path().join("subscription-proxy.json"), b"{broken").unwrap();
    assert!(store.load().is_err());
    store.configure(manual("http://127.0.0.1:1187")).unwrap();
    assert_eq!(
        store.load().unwrap().policy,
        manual("http://127.0.0.1:1187")
    );
}

#[test]
fn every_mode_rejects_unowned_fields() {
    for input in [
        r#"{"mode":"inherit","no_proxy":"example.com"}"#,
        r#"{"mode":"direct","url":"http://localhost"}"#,
        r#"{"mode":"manual","url":"http://localhost","extra":true}"#,
        r#"{"mode":"manual"}"#,
    ] {
        assert!(
            serde_json::from_str::<SubscriptionProxyPolicy>(input).is_err(),
            "{input}"
        );
    }
    for value in [
        SubscriptionProxyPolicy::Inherit,
        SubscriptionProxyPolicy::Direct,
        manual("http://127.0.0.1:1187"),
    ] {
        assert_eq!(
            serde_json::from_str::<SubscriptionProxyPolicy>(
                &serde_json::to_string(&value).unwrap()
            )
            .unwrap(),
            value
        );
    }
}

#[test]
fn proxy_authority_is_identical_for_rust_and_cpa_consumers() {
    for value in [
        "http:127.0.0.1:1187",
        "http:/127.0.0.1:1187",
        "http:///127.0.0.1:1187",
        "http:////127.0.0.1:1187",
        r"http://127.0.0.1:1187\",
        r"http://localhost\@other:1187",
    ] {
        assert!(manual(value).validate().is_err(), "{value}");
        assert!(manual(value).resolve([]).is_err(), "{value}");
    }
    let root = private_root();
    let store = SubscriptionProxyStore::new(root.path());
    for (input, expected) in [
        ("http://LOCALHOST:1187/", "http://localhost:1187"),
        ("http://127.1:1187", "http://127.0.0.1:1187"),
    ] {
        let saved = store.configure(manual(input)).unwrap();
        assert_eq!(saved.policy, manual(expected));
        // Also normalize a previously stored, structurally valid policy at launch.
        for policy in [saved.policy, manual(input)] {
            let vars: std::collections::BTreeMap<_, _> =
                policy.resolve([]).unwrap().variables().collect();
            assert_eq!(vars[std::ffi::OsStr::new("HTTP_PROXY")], expected);
            assert_eq!(vars[std::ffi::OsStr::new("HTTPS_PROXY")], expected);
        }
    }
}
