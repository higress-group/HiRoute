//! Real role-all / Local Control / HTTP composition; installed acceptance is a separate gate.
use super::*;
use hiroute_application_api::*;
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_host_runtime::StandaloneLayout;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::Duration,
};
pub(super) fn request(operation: &str, payload: Value) -> LocalControlWireRequestV2 {
    LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: format!("console-{operation}"),
        operation_id: operation.into(),
        payload,
        protected_grant: None,
    }
}
pub(super) fn http(
    address: SocketAddr,
    path: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    extra: &str,
) -> (u16, Value, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    let authorization = bearer
        .map(|b| format!("Authorization: Bearer {b}\r\n"))
        .unwrap_or_default();
    write!(stream,"{} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nX-HiRoute-Context: 1234567890abcdef\r\n{authorization}{extra}Content-Length: {}\r\n\r\n{body}",if body.is_empty(){"GET"}else{"POST"},body.len()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (status, content) = raw.split_once("\r\n\r\n").unwrap();
    let code = status.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        code,
        serde_json::from_str(content).unwrap_or(Value::Null),
        status.into(),
    )
}
// KnownFolders cannot be redirected on Windows; its installed composition uses
// the native standard-user acceptance route instead of this isolated Unix home.
#[cfg(unix)]
#[test]
fn real_role_all_keeps_external_gate_and_console_auth_authority_separate() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "console::tests::real_role_all_keeps_external_gate_and_console_auth_authority_separate",
    ) {
        return;
    }
    if installed_test_layout(
        "console::tests::real_role_all_keeps_external_gate_and_console_auth_authority_separate",
    ) {
        return;
    }
    let directory = crate::test_support::private_tempdir();
    let layout = StandaloneLayout::from_values(
        false,
        Some(directory.path().to_path_buf()),
        None,
        None,
        None,
    )
    .unwrap();
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway = held.local_addr().unwrap();
    drop(held);
    let mut role = crate::start_role_all(
        crate::RoleAllConfig::new(
            layout.storage_root(),
            layout.runtime_root.clone(),
            gateway,
            layout.gateway_lkg(),
        )
        .with_released_commands_only(),
    )
    .unwrap();
    let assets = directory.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(
        assets.join("index.html"),
        b"<!doctype html><title>HiRoute</title>",
    )
    .unwrap();
    hiroute_host_runtime::ConsoleListenerStore::new(layout.gateway_config_root())
        .configure(hiroute_host_runtime::ConsoleListenerDesiredV1::loopback())
        .unwrap();
    let mut console = ConsoleService::start(&role, layout.clone(), assets, None).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let external = Client::new(
        "cli",
        LocalEndpoint::from_runtime_root(&layout.runtime_root),
    );
    let denied = runtime
        .block_on(external.call_wire(request("GetClientServiceStatus", json!({}))))
        .unwrap();
    assert_eq!(denied.error.unwrap().code, ErrorCode::UnknownCommand);
    let internal = runtime
        .block_on(
            role.console_client()
                .call_wire(request("GetClientServiceStatus", json!({}))),
        )
        .unwrap();
    assert!(internal.error.is_none(), "{:?}", internal.error);
    let mut agent = request("CreateConsoleLaunch", json!({}));
    agent.protected_grant = Some(ProtectedClientGrantV2 {
        principal_kind: PrincipalKind::SealedCollaboration,
        capability: "not-an-owner".into(),
    });
    assert_eq!(
        runtime
            .block_on(external.call_wire(agent))
            .unwrap()
            .error
            .unwrap()
            .code,
        ErrorCode::CapabilityDenied
    );
    let launch = runtime
        .block_on(external.call_wire(request("CreateConsoleLaunch", json!({}))))
        .unwrap()
        .data
        .unwrap();
    let url = launch["url"].as_str().unwrap();
    let (address, code) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once("/launch?code=")
        .unwrap();
    let address: SocketAddr = address.parse().unwrap();
    assert_eq!(
        http(
            address,
            "/api/console/v1/session/launch",
            Some(json!({"code":code})),
            None,
            "Origin: http://evil.example\r\n"
        )
        .0,
        403
    );
    let (status, exchange, headers) = http(
        address,
        "/api/console/v1/session/launch",
        Some(json!({"code":code})),
        None,
        "",
    );
    assert_eq!(status, 200);
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-security-policy:")
    );
    assert!(!headers.to_ascii_lowercase().contains("set-cookie"));
    let bearer = exchange["bearer"].as_str().unwrap();
    assert_eq!(
        http(
            address,
            "/api/console/v1/session/launch",
            Some(json!({"code":code})),
            None,
            ""
        )
        .0,
        401
    );
    assert_eq!(
        http(address, "/api/console/v1/session", None, None, "").0,
        401
    );
    assert_eq!(
        http(
            address,
            "/api/console/v1/command/quit_desktop",
            Some(json!({})),
            Some(bearer),
            ""
        )
        .0,
        404
    );
    let (status, snapshot, _) = http(
        address,
        "/api/console/v1/command/desktop_snapshot",
        Some(json!({})),
        Some(bearer),
        "",
    );
    assert_eq!(status, 200);
    assert_eq!(snapshot["trusted_authority"], true);
    let (status, invalid_preview, _) = http(
        address,
        "/api/console/v1/command/preview_compute_save_v3",
        Some(json!({})),
        Some(bearer),
        "",
    );
    assert_eq!(
        status, 400,
        "invalid preview arguments must not invalidate authentication: {invalid_preview}"
    );
    assert_eq!(invalid_preview["code"], "REQUEST_INVALID");
    assert_eq!(
        http(address, "/api/console/v1/session", None, Some(bearer), "").0,
        200
    );
    assert_eq!(
        http(
            address,
            "/api/console/v1/command/preview_compute_save_v3",
            Some(json!({})),
            None,
            ""
        )
        .0,
        401,
        "versioned commands still require authentication"
    );
    assert_eq!(
        http(
            address,
            "/api/console/v1/command/preview_compute_save_v99",
            Some(json!({})),
            Some(bearer),
            ""
        )
        .0,
        404,
        "audit syntax does not admit commands outside the installed catalog"
    );
    // The same optional native-target arguments used by Desktop must reach the
    // shared session over HTTP, rather than being dropped by the host adapter.
    let missing_profile = directory.path().join("missing-hermes");
    let target = serde_json::to_value(hiroute_domain::AgentNativeTargetV1::Hermes {
        config_root: missing_profile.to_str().unwrap().into(),
    })
    .unwrap();
    let (status, agents, _) = http(
        address,
        "/api/console/v1/command/agent_snapshot",
        Some(json!({"nativeTargets":[target]})),
        Some(bearer),
        "",
    );
    assert_eq!(status, 200, "{agents}");
    let unavailable = agents["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["native_target"] == target)
        .expect("the selected unavailable profile must not fall back");
    assert_eq!(unavailable["configuration_state"], "profile_unavailable");
    assert!(!missing_profile.exists());
    let (status, scan, _) = http(
        address,
        "/api/console/v1/command/compute_scan",
        Some(json!({"nativeTarget":null})),
        Some(bearer),
        "",
    );
    assert_eq!(status, 200, "{scan}");
    assert!(scan["connections"].is_array(), "{scan}");
    assert!(scan.get("sources").is_none());
    // Versioned mutations must reach product input validation, rather than
    // being rejected as invalid console credentials by the audit-name guard.
    for command in ["save_decision_service_v2", "test_decision_service_v2"] {
        let (status, response, _) = http(
            address,
            &format!("/api/console/v1/command/{command}"),
            Some(json!({})),
            Some(bearer),
            "",
        );
        assert_eq!(status, 400, "{command}: {response}");
        assert_ne!(response["code"], "CONSOLE_CREDENTIAL_INVALID");
    }
    let secret = "console-fixture-secret-must-not-be-logged";
    let (status, input, _) = http(
        address,
        "/api/console/v1/command/register_protected_model_input",
        Some(json!({"secret":secret,"sourceId":null})),
        Some(bearer),
        "",
    );
    assert_eq!(status, 200, "{input}");
    assert!(!input.to_string().contains(secret));
    // Exercise capacity through OS-trusted minting and the real HTTP exchange,
    // rather than creating sessions directly in the authentication fixture.
    let login = || {
        let launch = runtime
            .block_on(external.call_wire(request("CreateConsoleLaunch", json!({}))))
            .unwrap()
            .data
            .unwrap();
        let code = launch["url"]
            .as_str()
            .unwrap()
            .split_once("?code=")
            .unwrap()
            .1;
        let (status, session, _) = http(
            address,
            "/api/console/v1/session/launch",
            Some(json!({"code":code})),
            None,
            "",
        );
        assert_eq!(status, 200, "new login must not require a daemon restart");
        session["bearer"].as_str().unwrap().to_owned()
    };
    let sessions: Vec<_> = (0..16).map(|_| login()).collect();
    assert_eq!(
        http(address, "/api/console/v1/session", None, Some(bearer), "").0,
        401
    );
    assert_eq!(
        http(
            address,
            "/api/console/v1/session",
            None,
            Some(&sessions[0]),
            ""
        )
        .0,
        200
    );
    let newest = login();
    assert_eq!(
        http(
            address,
            "/api/console/v1/session",
            None,
            Some(&sessions[1]),
            ""
        )
        .0,
        401
    );
    assert_eq!(
        http(
            address,
            "/api/console/v1/session",
            None,
            Some(&sessions[0]),
            ""
        )
        .0,
        200
    );
    assert_eq!(
        http(address, "/api/console/v1/session", None, Some(&newest), "").0,
        200
    );
    role.upgrade_status(hiroute_host_runtime::UpgradeAction::Prepare, "test".into())
        .unwrap();
    assert!(
        runtime
            .block_on(
                role.console_client()
                    .call_wire(request("GetClientServiceStatus", json!({})))
            )
            .unwrap()
            .error
            .is_some()
    );
    role.upgrade_status(hiroute_host_runtime::UpgradeAction::Cancel, "test".into())
        .unwrap();
    let audit = std::fs::read_to_string(layout.data_root.join("console/audit.jsonl")).unwrap();
    assert!(audit.contains("register_protected_model_input"));
    assert!(!audit.contains(secret));
    assert!(!audit.contains(bearer));
    assert!(!audit.contains(code));
    assert!(!audit.contains(&newest));
    console.shutdown();
    role.shutdown();
    role.join(Duration::from_secs(30)).unwrap();
}

/// Standalone admission authenticates the adjacent CLI and stable entry even
/// when a composition test never executes the CLI. Reexec in a private layout;
/// never create an alias beside Cargo's shared test binaries. This native test
/// image is only a layout fixture, not installed CLI or provider-call evidence.
#[cfg(unix)]
pub(super) fn installed_test_layout(test: &str) -> bool {
    const CHILD: &str = "HIROUTE_CONSOLE_LAYOUT_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return false;
    }
    assert_eq!(
        std::env::var("HIROUTE_ISOLATED_DAEMON_TEST").as_deref(),
        Ok(test)
    );
    let home = crate::control::native_agent_home().unwrap();
    let staged = crate::test_support::private_tempdir();
    let daemon = staged.path().join("hirouted");
    let cli = staged.path().join("hiroute");
    std::fs::copy(std::env::current_exe().unwrap(), &daemon).unwrap();
    std::fs::copy(&daemon, &cli).unwrap();
    #[cfg(target_os = "macos")]
    for executable in [&daemon, &cli] {
        // macOS can reject a copied Rust test executable's CT signature before
        // the installed-layout child reaches any assertion. Sign only these
        // private, self-built fixture copies; production signing is unchanged.
        let status = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(executable)
            .status()
            .unwrap();
        assert!(status.success(), "private installed fixture signing failed");
    }
    let bin = home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&cli, bin.join("hiroute")).unwrap();
    let status = std::process::Command::new(&daemon)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, test)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "installed-layout console composition failed: {status}"
    );
    true
}
