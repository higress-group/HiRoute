use super::*;

fn login(kind: CpaAccountKind) -> Value {
    let host = match kind {
        CpaAccountKind::Codex => "auth.openai.com",
        CpaAccountKind::Claude => "claude.ai",
    };
    serde_json::json!({
        "status": "ok",
        "state": "fixture_state",
        "url": format!("https://{host}/oauth/authorize?state=fixture_state&code_challenge_method=S256"),
        // Even if upstream adds sensitive metadata, the public DTO is an explicit allowlist.
        "refresh_token": "must-not-be-returned",
    })
}

#[test]
fn oauth_login_projection_returns_only_bound_provider_url_and_state() {
    for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
        let parsed = parse_login(kind, &login(kind)).unwrap();
        assert_eq!(parsed.state, "fixture_state");
        assert!(!format!("{parsed:?}").contains("fixture_state"));
        assert!(!parsed.authorization_url.contains("must-not-be-returned"));
        let other = match kind {
            CpaAccountKind::Codex => CpaAccountKind::Claude,
            CpaAccountKind::Claude => CpaAccountKind::Codex,
        };
        assert!(parse_login(kind, &login(other)).is_err());
    }
}

#[test]
fn oauth_login_rejects_foreign_hosts_duplicate_state_and_missing_pkce() {
    for url in [
        "http://auth.openai.com/oauth/authorize?state=fixture_state&code_challenge_method=S256",
        "https://auth.openai.com.foreign.test/oauth/authorize?state=fixture_state&code_challenge_method=S256",
        "https://auth.openai.com/oauth/authorize?state=fixture_state&state=other&code_challenge_method=S256",
        "https://auth.openai.com/oauth/authorize?state=other&code_challenge_method=S256",
        "https://auth.openai.com/oauth/authorize?state=fixture_state",
    ] {
        let mut value = login(CpaAccountKind::Codex);
        value["url"] = url.into();
        assert!(parse_login(CpaAccountKind::Codex, &value).is_err());
    }
}

#[test]
fn callback_input_is_bound_to_provider_state_and_single_code() {
    assert_eq!(
        callback_code(
            CpaAccountKind::Codex,
            "expected",
            "http://localhost:1455/auth/callback?state=expected&code=secret-code"
        )
        .unwrap()
        .as_str(),
        "secret-code"
    );
    assert_eq!(
        callback_code(CpaAccountKind::Claude, "expected", "secret-code#expected")
            .unwrap()
            .as_str(),
        "secret-code"
    );
    for input in [
        "http://localhost:1455/auth/callback?state=wrong&code=secret-code",
        "http://localhost:1455/auth/callback?state=expected&code=first&code=second",
        "http://foreign.test:1455/auth/callback?state=expected&code=secret-code",
        "http://localhost:54545/callback?state=expected&code=secret-code",
        "secret-code#expected",
    ] {
        assert!(callback_code(CpaAccountKind::Codex, "expected", input).is_err());
    }
    assert!(callback_code(CpaAccountKind::Claude, "expected", "secret-code#wrong").is_err());
}

#[test]
fn stale_and_cancelled_oauth_state_never_reaches_management() {
    let mut inner = RuntimeInner::default();
    assert!(validate_session(&inner, "expected").is_err());
    inner.oauth_state = Some("expected".into());
    assert!(validate_session(&inner, "expected").is_ok());
    assert!(validate_session(&inner, "wrong").is_err());
    assert!(validate_session(&inner, "expected&provider=claude").is_err());
    inner.oauth_state = None;
    assert!(validate_session(&inner, "expected").is_err());
}

#[test]
fn callback_acknowledgement_allows_only_missing_version_header() {
    let expected = crate::MANAGED_CPA_ARTIFACT_VERSION;
    for policy in [
        OAuthResponsePolicy::Management,
        OAuthResponsePolicy::CallbackAcknowledgement,
    ] {
        assert!(validate_response_version(policy, Some(expected), expected).is_ok());
        assert!(validate_response_version(policy, Some(&format!("v{expected}")), expected).is_ok());
        for mismatch in ["", "unexpected-version"] {
            assert!(matches!(
                validate_response_version(policy, Some(mismatch), expected),
                Err(CpaLifecycleError::UnsafeControlResponse)
            ));
        }
    }
    assert!(
        validate_response_version(OAuthResponsePolicy::CallbackAcknowledgement, None, expected)
            .is_ok()
    );
    assert!(matches!(
        validate_response_version(OAuthResponsePolicy::Management, None, expected),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
}

#[test]
fn live_credential_status_separates_relogin_from_temporary_failure() {
    let mut value = serde_json::json!({"files": [{
        "name": "credential.json", "provider": "codex", "source": "file",
        "runtime_only": false, "account_type": "oauth", "status": "active",
        "disabled": false, "unavailable": false, "status_message": ""
    }]});
    assert!(validate_credential_status(&value, "credential.json", CpaAccountKind::Codex).is_ok());
    for terminal in [
        "unauthorized",
        "invalid grant (retrying)",
        "disabled (invalid grant)",
    ] {
        value["files"][0]["status"] = "error".into();
        value["files"][0]["unavailable"] = true.into();
        value["files"][0]["status_message"] = terminal.into();
        let result = validate_credential_status(&value, "credential.json", CpaAccountKind::Codex);
        assert!(matches!(
            result,
            Err(CpaLifecycleError::ManagedOAuthAuthenticationRequired)
        ));
    }
    for transient in [
        "token expired",
        "network error with private upstream details",
        "unknown",
    ] {
        value["files"][0]["status_message"] = transient.into();
        let result = validate_credential_status(&value, "credential.json", CpaAccountKind::Codex);
        assert!(matches!(result, Err(CpaLifecycleError::ControlUnavailable)));
    }
    assert!(matches!(
        validate_credential_status(&value, "another.json", CpaAccountKind::Codex),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
    assert!(matches!(
        validate_credential_status(&value, "credential.json", CpaAccountKind::Claude),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
    assert!(matches!(
        validate_credential_status(
            &serde_json::json!({"files": []}),
            "credential.json",
            CpaAccountKind::Codex
        ),
        Err(CpaLifecycleError::ManagedOAuthCredentialsMissing)
    ));
}
