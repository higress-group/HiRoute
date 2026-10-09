//! Provider-local source errors must not become daemon startup failures.
use super::*;
use std::net::TcpListener;
use std::process::Command;

#[test]
fn invalid_subscription_source_is_local_to_its_provider() {
    const TEST: &str = "role_all::subscription_startup_tests::invalid_subscription_source_is_local_to_its_provider";
    const CHILD: &str = "HIROUTE_SUBSCRIPTION_STARTUP_CASE";
    let Ok(case) = std::env::var(CHILD) else {
        for case in [
            "claude-secure-relative",
            "claude-secure-parent",
            "codex-relative",
            "codex-parent",
            "both",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", TEST, "--nocapture"])
                .env_clear()
                .env(CHILD, case)
                .env("HOME", root.path())
                .env("PATH", "/usr/bin:/bin")
                .env("CODEX_HOME", root.path().join(".codex"))
                .env("CLAUDE_CONFIG_DIR", root.path().join(".claude"));
            match case {
                "claude-secure-relative" | "both" => {
                    child.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", "relative");
                }
                "claude-secure-parent" => {
                    child.env(
                        "CLAUDE_SECURESTORAGE_CONFIG_DIR",
                        root.path().join("other/../.claude"),
                    );
                }
                _ => {}
            }
            if matches!(case, "codex-relative" | "both") {
                child.env("HIROUTE_CODEX_AUTH_SOURCE", "relative/auth.json");
            } else if case == "codex-parent" {
                child.env(
                    "HIROUTE_CODEX_AUTH_SOURCE",
                    root.path().join("other/../auth.json"),
                );
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        }
        return;
    };
    let root = PathBuf::from(std::env::var_os("HOME").unwrap());
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let listen = socket.local_addr().unwrap();
    drop(socket);
    let config = RoleAllConfig::new(
        root.join("storage"),
        root.join("runtime"),
        listen,
        root.join("gateway-lkg.json"),
    )
    .with_cpa(RoleAllCpaConfig {
        // No account is authorized: construction must not read credentials or start CPA.
        binary: root.join("unused-cpa"),
        expected_sha256_hex: "0".repeat(64),
    });
    let mut role = start_role_all(config).unwrap();
    assert_eq!(
        role.phases(),
        (ManagedControlPhase::Ready, ManagedGatewayPhase::Ready)
    );
    let runtimes = role.cpa.as_ref().unwrap();
    assert_eq!(
        runtimes.for_kind(CpaAccountKind::Claude).is_some(),
        case.starts_with("codex-")
    );
    assert_eq!(
        runtimes.for_kind(CpaAccountKind::Codex).is_some(),
        case.starts_with("claude-")
    );
    assert!(!root.join(".codex/auth.json").exists());
    assert!(!root.join(".claude/.credentials.json").exists());
    role.shutdown();
    role.join(Duration::from_secs(15)).unwrap();
}
