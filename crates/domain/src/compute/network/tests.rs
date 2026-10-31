use super::*;

fn environment(values: &[(&str, &str)]) -> NativeProxyEnvironmentV1 {
    NativeProxyEnvironmentV1::from_variables(
        "service-epoch-1".into(),
        values.iter().map(|(k, v)| (k.to_string(), v.to_string())),
    )
    .unwrap()
}
fn destination(value: &str) -> NativeNetworkDestinationV1 {
    NativeNetworkDestinationV1::from_url(value).unwrap()
}
fn manual(bypass: &str) -> NativeNetworkPolicyV1 {
    NativeNetworkPolicyV1::Manual {
        proxy_url: "http://proxy.example:8080".into(),
        no_proxy: bypass.into(),
    }
}

#[test]
fn direct_and_manual_ignore_environment_and_do_not_fallback_between_schemes() {
    let env = environment(&[("HTTPS_PROXY", "not-a-proxy"), ("NO_PROXY", "bad/rule")]);
    let dst = destination("https://api.example/v1");
    assert!(
        resolve_native_network_route(&NativeNetworkPolicyV1::Direct, &env, &dst)
            .unwrap()
            .proxy
            .is_none()
    );
    assert!(
        resolve_native_network_route(&manual(""), &env, &dst)
            .unwrap()
            .proxy
            .is_some()
    );
    assert!(resolve_native_network_route(&NativeNetworkPolicyV1::Inherit, &env, &dst).is_err());
    let env = environment(&[("HTTP_PROXY", "http://proxy.example:8080")]);
    let route = resolve_native_network_route(&NativeNetworkPolicyV1::Inherit, &env, &dst).unwrap();
    assert_eq!(route.reason, NativeNetworkRouteReasonV1::NoProxyConfigured);
    assert!(route.proxy.is_none());
}

#[test]
fn captured_environment_has_stable_precedence_and_never_prints_credentials() {
    let env = environment(&[
        (
            "HTTPS_PROXY",
            "http://user:sensitive-value@upper.example:80",
        ),
        ("https_proxy", "http://lower.example:8080"),
    ]);
    let route = resolve_native_network_route(
        &NativeNetworkPolicyV1::Inherit,
        &env,
        &destination("https://api.example/v1"),
    )
    .unwrap();
    assert_eq!(route.proxy.as_ref().unwrap().host, "upper.example");
    assert_eq!(route.environment_epoch.as_deref(), Some("service-epoch-1"));
    for debug in [format!("{env:?}"), format!("{route:?}")] {
        for secret in ["sensitive-value", "user:", "upper.example"] {
            assert!(!debug.contains(secret));
        }
    }
    let env = environment(&[("HTTPS_PROXY", ""), ("https_proxy", "http://lower.example")]);
    assert_eq!(
        resolve_native_network_route(
            &NativeNetworkPolicyV1::Inherit,
            &env,
            &destination("https://api.example")
        )
        .unwrap()
        .proxy
        .unwrap()
        .host,
        "lower.example"
    );
}

#[test]
fn bypass_uses_the_original_host_and_port_without_dns() {
    let env = environment(&[]);
    let cases = [
        ("example.test", "https://example.test", true),
        (".Example.Test.", "https://deep.api.example.test", true),
        ("example.test", "https://badexample.test", false),
        ("example.test:8443", "https://api.example.test:8443", true),
        ("example.test:8443", "https://api.example.test", false),
        ("192.0.2.0/24", "https://192.0.2.9", true),
        ("192.0.2.0/24", "https://proxy-only-dns.example", false),
        ("[2001:db8::1]:443", "https://[2001:db8::1]", true),
        ("2001:db8::/32", "https://[2001:db8::2]", true),
        (" , example.test, ", "https://example.test", true),
        ("*", "https://anything.example", true),
        ("", "http://localhost:8123", true),
        ("", "https://nested.localhost", true),
        ("", "http://127.9.2.1:8000", true),
        ("", "http://[::1]:8000", true),
        ("", "http://192.0.2.1", false),
        ("xn--bcher-kva.example", "https://bücher.example", true),
    ];
    for (bypass, target, direct) in cases {
        let route =
            resolve_native_network_route(&manual(bypass), &env, &destination(target)).unwrap();
        assert_eq!(route.proxy.is_none(), direct, "{bypass} / {target}");
    }
}

#[test]
fn malformed_proxy_and_bypass_are_not_legacy_direct() {
    for url in [
        "socks5://proxy:1080",
        "http://u:p@proxy",
        "http://@proxy",
        "http://proxy/a",
        "http://proxy?x=1",
        "http://proxy#x",
        "http://proxy:0",
        "http:\\proxy",
        "http://proxy\n",
    ] {
        let policy = NativeNetworkPolicyV1::Manual {
            proxy_url: url.into(),
            no_proxy: "".into(),
        };
        assert!(policy.normalized().is_err(), "{url}");
    }
    for bypass in [
        "*.example.test",
        "http://example.test",
        "example.test/path",
        "example.test:0",
        "2001:db8::1:443:invalid",
        "example..test",
        "192.0.2.0/99",
    ] {
        assert!(manual(bypass).normalized().is_err(), "{bypass}");
    }
    assert!(serde_json::from_str::<NativeNetworkPolicyV1>(r#"{"mode":"unknown"}"#).is_err());
    assert!(
        serde_json::from_str::<NativeNetworkPolicyV1>(
            r#"{"mode":"direct","proxy_url":"http://proxy"}"#
        )
        .is_err()
    );
    // An invalid selected environment proxy remains explicit even for a loopback destination.
    let env = environment(&[("HTTPS_PROXY", "broken")]);
    assert!(
        resolve_native_network_route(
            &NativeNetworkPolicyV1::Inherit,
            &env,
            &destination("https://localhost")
        )
        .is_err()
    );
}

#[test]
fn route_selection_uses_each_endpoint_instead_of_the_source_generation_host() {
    let env = environment(&[
        ("HTTPS_PROXY", "https://proxy.example:8443"),
        ("NO_PROXY", "decisions.example:8443"),
    ]);
    let generation = resolve_native_network_route(
        &NativeNetworkPolicyV1::Inherit,
        &env,
        &destination("https://generation.example"),
    )
    .unwrap();
    let decision = resolve_native_network_route(
        &NativeNetworkPolicyV1::Inherit,
        &env,
        &destination("https://decisions.example:8443/decisions"),
    )
    .unwrap();
    assert!(generation.proxy.is_some());
    assert!(decision.proxy.is_none());
    assert_eq!(decision.reason, NativeNetworkRouteReasonV1::BypassMatched);
    assert_eq!(generation.environment_epoch, decision.environment_epoch);
}
