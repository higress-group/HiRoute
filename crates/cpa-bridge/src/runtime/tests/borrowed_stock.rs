use std::fs::{self, File};
use std::io::Read;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::*;

#[test]
fn borrowed_codex_reenabled_source_leases_without_auth_rotation() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "codex-model")]);
    let runtime = fixture_runtime(&root, backend, control, 2);
    runtime.start().unwrap();
    let source = root.path().join("codex-auth.json");
    let source_before = fs::read(&source).unwrap();
    let first = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    runtime
        .apply_account_management(&first.account_subject, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    runtime
        .apply_account_management(
            &first.account_subject,
            2,
            CpaSourceManagementState::Disabled,
        )
        .unwrap();
    runtime
        .apply_account_management(&first.account_subject, 3, CpaSourceManagementState::Enabled)
        .unwrap();
    let resumed = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    assert_eq!(resumed.credential_ref.generation(), 2);
    assert_eq!(fs::read(&source).unwrap(), source_before);
    let target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &resumed.credential_ref,
            upstream_model_id: "codex-model",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    let request = || ExactCpaCredentialRequest {
        credential_id: target.credential_ref().credential_id(),
        connector_id: target.connector_id(),
        upstream_model_id: target.upstream_model_id(),
        protocol: target.protocol(),
        address: target.address(),
        request_path: target.request_path(),
        native_transport_model: target.native_transport_model(),
        runtime_epoch: target.runtime_epoch(),
        target_epoch: target.target_epoch(),
        excluded_key_ids: &[],
    };
    let first_capability = runtime
        .lease_downstream_capability(request())
        .unwrap()
        .unwrap();
    assert_eq!(first_capability.generation(), 2);

    write_fixture_codex_source(
        &source,
        "fixture-account-one",
        "fixture-codex-access-lease-rotated",
        "fixture.codex.id-token-rotated",
        "fixture-refresh-time-two",
    );
    let rotated_capability = runtime
        .lease_downstream_capability(request())
        .unwrap()
        .unwrap();
    assert!(rotated_capability.generation() > first_capability.generation());
    assert_eq!(
        first_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    runtime.shutdown().unwrap();
}

#[test]
fn borrowed_codex_rotation_and_deletion_revoke_attempts_inside_cpa() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "codex-model")]);
    let runtime = fixture_runtime(&root, backend, control, 2);
    runtime.start().unwrap();
    let source = root.path().join("codex-auth.json");
    let first = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    runtime
        .apply_account_management(&first.account_subject, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    let old_target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &first.credential_ref,
            upstream_model_id: "codex-model",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    let old_capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: old_target.credential_ref().credential_id(),
            connector_id: old_target.connector_id(),
            upstream_model_id: old_target.upstream_model_id(),
            protocol: old_target.protocol(),
            address: old_target.address(),
            request_path: old_target.request_path(),
            native_transport_model: old_target.native_transport_model(),
            runtime_epoch: old_target.runtime_epoch(),
            target_epoch: old_target.target_epoch(),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();

    write_fixture_codex_source(
        &source,
        "fixture-account-one",
        "fixture-codex-access-lease-rotated",
        "fixture.codex.id-token-rotated",
        "fixture-refresh-time-two",
    );
    // The request capability boundary performs its own CPA-private refresh before any later
    // maintenance cycle observes the same-account token rotation.
    let current_capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: old_target.credential_ref().credential_id(),
            connector_id: old_target.connector_id(),
            upstream_model_id: old_target.upstream_model_id(),
            protocol: old_target.protocol(),
            address: old_target.address(),
            request_path: old_target.request_path(),
            native_transport_model: old_target.native_transport_model(),
            runtime_epoch: old_target.runtime_epoch(),
            target_epoch: old_target.target_epoch(),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();
    assert_eq!(current_capability.generation(), 2);
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    let rotated_target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &first.credential_ref,
            upstream_model_id: "codex-model",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    assert_eq!(rotated_target.credential_ref().generation(), 2);
    assert_eq!(
        rotated_target.credential_ref().credential_id(),
        first.credential_ref.credential_id()
    );
    assert_eq!(
        current_capability.credential_ref(),
        rotated_target.credential_ref()
    );
    let wrong_owner = hiroute_domain::CredentialRefV1::new(
        rotated_target.credential_ref().credential_id(),
        "source/cpa/not-the-current-account",
        rotated_target.credential_ref().subject(),
        rotated_target.credential_ref().purpose(),
        rotated_target
            .credential_ref()
            .allowed_destinations()
            .iter()
            .cloned(),
        1,
    )
    .unwrap();
    assert!(matches!(
        prepare_target(
            &runtime,
            ExactCpaAttemptRequest {
                credential_ref: &wrong_owner,
                upstream_model_id: "codex-model",
                protocol: UpstreamProtocol::Responses,
            }
        ),
        Err(CpaAttemptError::RevokedCredential)
    ));

    let rotated = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    assert_eq!(rotated.source_id, first.source_id);
    assert_eq!(rotated.account_subject, first.account_subject);
    assert_eq!(
        rotated.credential_ref.credential_id(),
        first.credential_ref.credential_id()
    );
    assert_eq!(rotated.credential_ref.generation(), 2);
    assert_ne!(rotated.credential_ref, first.credential_ref);

    // Native auth deletion is also detected by Attempt itself, independently of maintenance.
    fs::remove_file(&source).unwrap();
    assert!(matches!(
        runtime.lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: rotated_target.credential_ref().credential_id(),
            connector_id: rotated_target.connector_id(),
            upstream_model_id: rotated_target.upstream_model_id(),
            protocol: rotated_target.protocol(),
            address: rotated_target.address(),
            request_path: rotated_target.request_path(),
            native_transport_model: rotated_target.native_transport_model(),
            runtime_epoch: rotated_target.runtime_epoch(),
            target_epoch: rotated_target.target_epoch(),
            excluded_key_ids: &[],
        }),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(
        current_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    assert!(matches!(
        prepare_target(
            &runtime,
            ExactCpaAttemptRequest {
                credential_ref: rotated_target.credential_ref(),
                upstream_model_id: "codex-model",
                protocol: UpstreamProtocol::Responses,
            }
        ),
        Err(CpaAttemptError::Unavailable)
    ));
    runtime.shutdown().unwrap();
}

#[test]
fn borrowed_codex_account_replacement_is_rejected_before_managed_auth_changes() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "codex-model")]);
    let runtime = fixture_runtime(&root, backend, control, 2);
    runtime.start().unwrap();
    let source = root.path().join("codex-auth.json");
    let first = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    runtime
        .apply_account_management(&first.account_subject, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    let target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &first.credential_ref,
            upstream_model_id: "codex-model",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    let capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: target.credential_ref().credential_id(),
            connector_id: target.connector_id(),
            upstream_model_id: target.upstream_model_id(),
            protocol: target.protocol(),
            address: target.address(),
            request_path: target.request_path(),
            native_transport_model: target.native_transport_model(),
            runtime_epoch: target.runtime_epoch(),
            target_epoch: target.target_epoch(),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();
    let managed_path = root.path().join("auth/hiroute-managed-codex.json");
    let state_path = root.path().join("auth/.hiroute-borrowed-codex.state");
    let managed_before = fs::read(&managed_path).unwrap();
    let state_before = fs::read(&state_path).unwrap();

    write_fixture_codex_source(
        &source,
        "fixture-account-two",
        "fixture-codex-access-lease-account-two",
        "fixture.codex.id-token-account-two",
        "fixture-refresh-time-account-two",
    );

    assert!(matches!(
        runtime.lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: target.credential_ref().credential_id(),
            connector_id: target.connector_id(),
            upstream_model_id: target.upstream_model_id(),
            protocol: target.protocol(),
            address: target.address(),
            request_path: target.request_path(),
            native_transport_model: target.native_transport_model(),
            runtime_epoch: target.runtime_epoch(),
            target_epoch: target.target_epoch(),
            excluded_key_ids: &[],
        }),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(fs::read(&managed_path).unwrap(), managed_before);
    assert_eq!(fs::read(&state_path).unwrap(), state_before);
    assert_eq!(
        capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    assert!(matches!(
        prepare_target(
            &runtime,
            ExactCpaAttemptRequest {
                credential_ref: &first.credential_ref,
                upstream_model_id: "codex-model",
                protocol: UpstreamProtocol::Responses,
            }
        ),
        Err(CpaAttemptError::Unavailable)
    ));
    runtime.shutdown().unwrap();
}

#[test]
#[ignore = "requires HIROUTE_STOCK_CPA_BIN and HIROUTE_STOCK_CPA_SHA256"]
fn stock_cpa_v7_2_140_accepts_managed_auth_ready_and_shutdown_contract() {
    let binary = PathBuf::from(std::env::var("HIROUTE_STOCK_CPA_BIN").unwrap());
    let digest = std::env::var("HIROUTE_STOCK_CPA_SHA256").unwrap();
    let root = tempfile::tempdir().unwrap();
    let auth_dir = ensure_private_dir(&root.path().join("auth")).unwrap();
    let mut account = serde_json::Map::from_iter([("type".into(), json!("claude"))]);
    account.insert(
        ["access", "token"].join("_"),
        json!("fixture-claude-noncredential"),
    );
    private_atomic_write(
        &auth_dir.join("claude-fixture.json"),
        &serde_json::to_vec(&account).unwrap(),
    )
    .unwrap();
    let codex_auth_source = fixture_codex_auth_source(&root);
    let spec = CpaRuntimeSpec {
        instance_id: "stock-contract".into(),
        state_root: root.path().join("state"),
        auth_dir,
        borrowed_claude_auth: None,
        managed_oauth: None,
        borrowed_codex_auth: Some(BorrowedCodexAuthSpec::new(codex_auth_source)),
        bindings: bindings(),
        startup_timeout: Duration::from_secs(10),
        control_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        restart_policy: RestartPolicy::default(),
    };
    let locator = PinnedCpaBinaryLocator::new(PinnedCpaArtifact::new(
        binary.parent().unwrap(),
        binary.file_name().unwrap(),
        Version::parse(VERSION).unwrap(),
        digest,
    ));
    let runtime = ManagedCpaRuntime::new(spec, fixture_catalog(), Arc::new(locator)).unwrap();
    let CpaHealth::Ready { address, pid, .. } = runtime.start().unwrap() else {
        panic!("stock CPA was not ready")
    };
    validate_private_file(&root.path().join("state/stock-contract/config.yaml")).unwrap();
    let unauthenticated = request(LoopbackRequest {
        address,
        method: "GET",
        path: "/v1/models",
        authorization: None,
        body: &[],
        timeout: Duration::from_secs(2),
    })
    .unwrap();
    assert_eq!(unauthenticated.status, 401);
    let deadline = Instant::now() + Duration::from_secs(3);
    let registered = loop {
        let registered = runtime.discover_registered_sources().unwrap();
        if !registered.is_empty() {
            break registered;
        }
        assert!(
            Instant::now() < deadline,
            "stock CPA did not load fixture OAuth metadata"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(registered.len(), 2);
    let connectors = registered
        .iter()
        .map(|source| source.source.connector_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        connectors,
        BTreeSet::from(["connector.cpa.claude", "connector.cpa.codex"])
    );
    let debug = format!("{registered:?}");
    assert!(!debug.contains("fixture-codex-noncredential"));
    assert!(!debug.contains("fixture-claude-noncredential"));
    let repeated = runtime.discover_registered_sources().unwrap();
    assert_eq!(repeated.len(), registered.len());
    // Machine authentication must not opt into the upstream interactive client's
    // ten-second keep-alive watchdog. No requests or model calls during this idle gap.
    std::thread::sleep(Duration::from_secs(11));
    assert!(
        matches!(runtime.health().unwrap(), CpaHealth::Ready { pid: current, restart_count: 0, .. } if current == pid),
        "managed CPA must remain the same ready process beyond the interactive watchdog interval"
    );
    assert_eq!(runtime.discover_registered_sources().unwrap(), repeated);
    let process_exit = runtime.shutdown().unwrap();
    assert_eq!(runtime.last_exit(), Some(process_exit));
}

#[test]
#[ignore = "uses the current Codex subscription and performs one real Responses request"]
fn stock_cpa_v7_2_140_real_borrowed_codex_responses_smoke() {
    let binary = PathBuf::from(std::env::var("HIROUTE_STOCK_CPA_BIN").unwrap());
    let digest = std::env::var("HIROUTE_STOCK_CPA_SHA256").unwrap();
    let source = PathBuf::from(std::env::var("HIROUTE_CODEX_AUTH_SOURCE").unwrap());
    let model = std::env::var("HIROUTE_STOCK_CPA_SMOKE_MODEL")
        .unwrap_or_else(|_| "gpt-5.6-terra".to_owned());
    let source_before = private_test_file_digest(&source);
    let source_metadata_before = fs::symlink_metadata(&source).unwrap();
    let root = tempfile::tempdir().unwrap();
    let auth_dir = ensure_private_dir(&root.path().join("auth")).unwrap();
    let spec = CpaRuntimeSpec {
        instance_id: "real-stock-contract".into(),
        state_root: root.path().join("state"),
        auth_dir,
        borrowed_claude_auth: None,
        managed_oauth: None,
        borrowed_codex_auth: Some(BorrowedCodexAuthSpec::new(source.clone())),
        bindings: vec![bindings().remove(0)],
        startup_timeout: Duration::from_secs(20),
        control_timeout: Duration::from_secs(10),
        shutdown_timeout: Duration::from_secs(5),
        restart_policy: RestartPolicy::default(),
    };
    let locator = PinnedCpaBinaryLocator::new(PinnedCpaArtifact::new(
        binary.parent().unwrap(),
        binary.file_name().unwrap(),
        Version::parse(VERSION).unwrap(),
        digest,
    ));
    let runtime = ManagedCpaRuntime::new(spec, fixture_catalog(), Arc::new(locator)).unwrap();
    let CpaHealth::Ready { address, .. } = runtime.start().unwrap() else {
        panic!("stock CPA was not ready")
    };
    let materialization = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    assert!(materialization.observed_model_ids.contains(&model));
    let repeated_materialization = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    assert_eq!(repeated_materialization, materialization);
    let prepared = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &materialization.credential_ref,
            upstream_model_id: &model,
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    assert_eq!(prepared.address(), address);
    let request_body = serde_json::to_vec(&json!({
        "model": prepared.native_transport_model(),
        "input": "Return exactly OK.",
        "stream": false,
        "max_output_tokens": 64
    }))
    .unwrap();
    let response = {
        let inner = runtime.inner.lock();
        let live = inner.live.as_ref().unwrap();
        request(LoopbackRequest {
            address,
            method: "POST",
            path: prepared.request_path(),
            authorization: Some(&live.secrets.downstream),
            body: &request_body,
            timeout: Duration::from_secs(60),
        })
    };
    let process_exit = runtime.shutdown().unwrap();
    let source_after = private_test_file_digest(&source);
    let source_metadata_after = fs::symlink_metadata(&source).unwrap();
    assert_eq!(source_after, source_before);
    assert_eq!(source_metadata_after.len(), source_metadata_before.len());
    assert_eq!(
        source_metadata_after.modified().unwrap(),
        source_metadata_before.modified().unwrap()
    );
    assert!(matches!(process_exit.code, None | Some(0)));
    let response = response.unwrap();
    assert_eq!(response.status, 200);
    let response_json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    let error = response_json.get("error");
    assert!(
        error.is_none_or(serde_json::Value::is_null),
        "Responses smoke returned error metadata: type={:?}, code={:?}",
        error
            .and_then(|value| value.get("type"))
            .and_then(serde_json::Value::as_str),
        error
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str)
    );
}

fn private_test_file_digest(path: &Path) -> String {
    validate_private_file(path).unwrap();
    let mut file = File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = Zeroizing::new([0_u8; 64 * 1024]);
    loop {
        let read = file.read(buffer.as_mut()).unwrap();
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    format!("{:x}", hasher.finalize())
}
