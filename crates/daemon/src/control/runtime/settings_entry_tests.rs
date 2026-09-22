//! Daemon wire dispatch -> Application -> real coordinator/stores/files/Gateway. Codex catalog
//! planning is structural and intentionally does not authenticate a binary version or digest;
//! native request verification remains a separate capability and surface result.
use super::*;
use crate::control::LocalControlDaemon;
use hiroute_application::{
    ApplicationService,
    control::{AgentConnectionControlPort, ControlReadError},
};
use hiroute_application_api::{self as api, LocalControlWireRequestV2};
use hiroute_gateway::server::{
    dispatch::{DispatchError, GatewayRequestAuthority},
    publication::GatewayPublicationInstaller,
    request_plan::IngressProtocol,
};
use hiroute_integrations::{
    AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
};
use std::sync::Arc;

#[test]
fn v2_settings_first_save_without_native_probe_and_reenable_identical_catalog() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::v2_settings_first_save_without_native_probe_and_reenable_identical_catalog",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let executable = root.path().join("codex-fixture");
    fs::write(&executable, b"#!/bin/sh\nprintf 'codex-cli 99.99.99\\n'\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = executable;
    layout.claude_executable = root.path().join("missing-claude");
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let runtime = open_with_scanner(root.path(), scanner);
    let path = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let before = b"# keep me\nuser_option = true\n";
    fs::write(&path, before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let ordinary = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = ordinary.dispatch_wire(request("ScanAgents", json!({}), None));
    let scan = scan.data.unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_codex_default")
        .unwrap();
    assert_eq!(
        codex["supported"], true,
        "the V2 settings entry treats an executable version as diagnostic, not an allowlist"
    );
    let context = codex["context_id"].as_str().unwrap().to_owned();
    let plans = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap()
        .published_agent_plans()
        .unwrap();
    let plan = plans
        .iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],
        "allowed_plan_ids":[plan.agent_plan_id],
        "default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    for operation in [
        "PreviewAgentConnectionRestore",
        "ApplyAgentConnectionRestore",
    ] {
        let rejected = ordinary.dispatch_wire(request(operation, if operation.starts_with("Preview") {
            json!({"spec":spec})
        } else {
            let digest = CanonicalDigest::of_bytes(b"not-a-restore");
            json!({"spec":spec,"accept_digest":digest,"dependency_digest":digest,
                "expected_revisions":{"target":0,"dependencies":{}},"idempotency_key":"not-a-restore"})
        }, None));
        assert_eq!(
            rejected.error.unwrap().code,
            api::ErrorCode::InvalidArguments
        );
    }

    let unproven = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(unproven.error.is_none(), "{unproven:?}");
    assert_eq!(
        unproven.data.unwrap()["applicable"],
        true,
        "Codex configuration safety is independent of the optional native HTTP diagnostic"
    );
    let preview = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(
        preview["applicable"], true,
        "an unknown diagnostic version must not block structural catalog planning: {preview}"
    );
    assert!(preview["blockers"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(fs::read(&path).unwrap(), before);

    // HiRoute-only routing does not claim Codex's original subscription models, even when
    // Codex has a native auth file. Its selected Plan alone is the published catalog.
    let auth = path.parent().unwrap().join("auth.json");
    fs::write(&path, b"").unwrap();
    fs::write(&auth, b"{}").unwrap();
    fs::set_permissions(&auth, fs::Permissions::from_mode(0o600)).unwrap();
    let empty_with_auth = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(empty_with_auth.error.is_none(), "{empty_with_auth:?}");
    let empty_with_auth = empty_with_auth.data.unwrap();
    assert_eq!(empty_with_auth["applicable"], true);
    assert!(
        empty_with_auth["blockers"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert_eq!(fs::read(&path).unwrap(), b"");
    fs::remove_file(&auth).unwrap();
    fs::write(&path, before).unwrap();
    assert!(
        runtime
            .adapter
            .resolve_active_agent_grant(&format!("agent-connection/{context}"))
            .is_err()
    );

    // The real settings capture reads this exact current default from Codex's user file.
    // Its alias is already authorized by the selected Plan, even with no fixed native model.
    let alias = plan.model_alias.as_str();
    let native_alias = format!("model = {alias:?}\nuser_option = true\n");
    fs::write(&path, &native_alias).unwrap();
    let mut preserve = spec.clone();
    preserve["model"]["settings"]["native_model_mode"] = json!("preserve_available");
    preserve["model"]["settings"]["default_selection"] = json!({"kind":"preserve_native"});
    let preserved = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":preserve.clone()}),
        None,
    ));
    assert!(preserved.error.is_none(), "{preserved:?}");
    let preserved = preserved.data.unwrap();
    assert_eq!(preserved["applicable"], false);
    assert!(
        preserved["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| {
                block["reason"] == "native_default_invalid" && block["model_ids"] == json!([alias])
            })
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), native_alias);

    fs::write(&path, "model = 'hiroute-unpublished'\nuser_option = true\n").unwrap();
    let rejected = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":preserve}),
        None,
    ));
    assert!(rejected.error.is_none(), "{rejected:?}");
    let rejected = rejected.data.unwrap();
    assert_eq!(rejected["applicable"], false);
    assert_eq!(rejected["blockers"][0]["reason"], "model_plan_unavailable");

    // Use the real daemon, planner, external guard, artifact store and native config renderer.
    // Repeated saves can carry different producer observations; a restored scope with the
    // unchanged baseline must reuse the original content-addressed artifact.
    fs::write(&path, before).unwrap();
    let first = apply(&ordinary, &runtime, spec.clone(), "catalog-first");
    let first_operation = OperationId::parse(first["operation_id"].as_str().unwrap()).unwrap();
    let catalog_for = |operation_id: &OperationId| {
        let stores = runtime.adapter.stores_lock().unwrap();
        let operation = stores
            .control()
            .load_operation(operation_id)
            .unwrap()
            .unwrap();
        operation
            .plan
            .external()
            .iter()
            .find(|intent| is_settings_codex_catalog(intent))
            .unwrap()
            .target()
            .to_owned()
    };
    let catalog_target = catalog_for(&first_operation);
    let artifact_path = runtime
        .adapter
        .artifacts
        .native_target_path(&catalog_target)
        .unwrap();
    let catalog_bytes = fs::read(&artifact_path).unwrap();
    assert_eq!(
        fs::read_to_string(&path)
            .unwrap()
            .matches("# keep me")
            .count(),
        1
    );
    let same = apply(&ordinary, &runtime, spec.clone(), "catalog-same");
    let same_operation = OperationId::parse(same["operation_id"].as_str().unwrap()).unwrap();
    let repeated_path = runtime
        .adapter
        .artifacts
        .native_target_path(&catalog_for(&same_operation))
        .unwrap();
    assert_eq!(fs::read(&repeated_path).unwrap(), catalog_bytes);
    assert_eq!(fs::read(&artifact_path).unwrap(), catalog_bytes);
    let restore = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"restore","restore_point_ref":codex_model_restore_point_ref(&same_operation)}});
    apply(&ordinary, &runtime, restore, "catalog-disable");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&artifact_path).unwrap(), catalog_bytes);
    let protected_config = fs::read(&path).unwrap();
    let pending = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(pending.error.is_none(), "{pending:?}");
    let pending = pending.data.unwrap();
    assert_eq!(pending["applicable"], true);
    assert_eq!(pending["resident_service"]["login_item_required"], false);
    fs::write(&artifact_path, b"outside edit").unwrap();
    let stale = ordinary.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        json!({
            "spec":spec, "accept_digest":pending["accept_digest"],
            "dependency_digest":pending["dependency_digest"],
            "expected_revisions":pending["expected_revisions"],
            "idempotency_key":"catalog-drift",
        }),
        None,
    ));
    assert_eq!(
        stale.error.unwrap().code,
        api::ErrorCode::ChangePreviewStale
    );
    assert_eq!(fs::read(&artifact_path).unwrap(), b"outside edit");
    assert_eq!(fs::read(&path).unwrap(), protected_config);
    fs::write(&artifact_path, &catalog_bytes).unwrap();
    let again = apply(&ordinary, &runtime, spec.clone(), "catalog-enable-again");
    let again_operation = OperationId::parse(again["operation_id"].as_str().unwrap()).unwrap();
    assert_eq!(catalog_for(&again_operation), catalog_target);
    assert_eq!(fs::read(&artifact_path).unwrap(), catalog_bytes);
    assert_eq!(
        fs::read_to_string(&path)
            .unwrap()
            .matches("# keep me")
            .count(),
        1
    );
    let configured = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{configured}unrelated_native = true\n")).unwrap();
    apply(
        &ordinary,
        &runtime,
        json!({"schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"configure","settings":{"mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],
        "allowed_plan_ids":[plan.agent_plan_id],"default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}}),
        "catalog-preserve-unrelated",
    );
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("unrelated_native = true")
    );

    // This same wire entry replaces the token without changing the Codex model binding.
    // The old credential must disappear from both the secret store and Gateway publication.
    let connection_id = format!("agent-connection/{context}");
    let previous_ref = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .inspect_agent_access_grant(WorkspaceId::DEFAULT, &connection_id)
        .unwrap()
        .unwrap();
    let previous_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .unwrap();
    let gateway_auth = |token: &[u8]| {
        let gateway = Arc::new(
            GatewayPublicationInstaller::open(root.path().join("gateway-lkg.json")).unwrap(),
        );
        let bearer = format!("Bearer {}", std::str::from_utf8(token).unwrap());
        GatewayRequestAuthority::new(gateway)
            .begin(IngressProtocol::Responses, Some(&bearer))
            .map(|_| ())
    };
    let candidate = api::ComputeCandidateRefV2 {
        candidate_ref: "candidate/native/agent-token-settings-entry".into(),
        candidate_revision: 1,
    };
    let custom_token = "0123456789abcdef-Custom.Token_~";
    runtime
        .register_manual_protected_input(
            candidate.clone(),
            hiroute_domain::ProtectedSecret::new(custom_token.as_bytes().to_vec()).unwrap(),
        )
        .unwrap();
    assert!(
        runtime
            .adapter
            .manual_protected_inputs
            .lock()
            .unwrap()
            .is_empty()
    );
    let mut custom_spec = spec.clone();
    custom_spec["access_token"] = json!({"intent":"set","input_slot":candidate.candidate_ref});
    let custom = apply(&ordinary, &runtime, custom_spec, "catalog-custom-token");
    let custom_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .unwrap();
    assert_eq!(custom_material.expose(), custom_token.as_bytes());
    assert_ne!(custom_material.sha256(), previous_material.sha256());
    assert!(gateway_auth(custom_material.expose()).is_ok());
    assert!(matches!(
        gateway_auth(previous_material.expose()),
        Err(DispatchError::Unauthorized)
    ));
    let custom_ref = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .secrets()
        .inspect_agent_access_grant(WorkspaceId::DEFAULT, &connection_id)
        .unwrap()
        .unwrap();
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .resolve_agent_access_grant(&previous_ref)
            .is_err()
    );
    let published = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    assert!(published.grants.iter().any(|grant| {
        grant.grant_id == custom_ref.grant_id()
            && grant.bearer_token_sha256 == custom_material.sha256()
    }));
    assert!(
        !published
            .grants
            .iter()
            .any(|grant| grant.bearer_token_sha256 == previous_material.sha256())
    );
    let custom_operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&OperationId::parse(custom["operation_id"].as_str().unwrap()).unwrap())
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&custom_operation)
            .unwrap()
            .contains(custom_token)
    );
    runtime
        .release_manual_protected_input(&candidate.candidate_ref)
        .unwrap();

    let mut regenerated_spec = spec;
    regenerated_spec["access_token"] = json!({"intent":"regenerate"});
    apply(
        &ordinary,
        &runtime,
        regenerated_spec,
        "catalog-regenerate-token",
    );
    let regenerated = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .unwrap();
    assert_ne!(regenerated.sha256(), custom_material.sha256());
    assert!(gateway_auth(regenerated.expose()).is_ok());
    assert!(matches!(
        gateway_auth(custom_material.expose()),
        Err(DispatchError::Unauthorized)
    ));
    let republished = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap();
    assert!(
        republished
            .grants
            .iter()
            .any(|grant| grant.bearer_token_sha256 == regenerated.sha256())
    );
    assert!(
        !republished
            .grants
            .iter()
            .any(|grant| grant.bearer_token_sha256 == custom_material.sha256())
    );
    assert!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .resolve_agent_access_grant(&custom_ref)
            .is_err()
    );
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains(std::str::from_utf8(regenerated.expose()).unwrap())
    );
}

#[test]
#[ignore = "requires explicitly selected trusted native Codex binary"]
fn native_check_unblocks_real_settings_apply_and_restore() {
    if crate::test_support::isolated_ignored_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::native_check_unblocks_real_settings_apply_and_restore",
    ) {
        return;
    }
    exercise_settings_entry();
}

fn exercise_settings_entry() {
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
    layout.codex_executable = std::path::PathBuf::from(
        std::env::var_os("HIROUTE_NATIVE_CODEX").expect("explicit native Codex"),
    );
    layout.claude_executable = root.path().join("missing-claude");
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data).unwrap(),
    );
    let runtime = open_with_scanner(root.path(), scanner);
    let path = runtime.adapter.scanner.codex_user_config_target();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&path, b"# keep me\nuser_option = true\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    ensure_target_cache(&runtime.adapter);
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let ordinary = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
    let scan = ordinary.dispatch_wire(request("ScanAgents", json!({}), None));
    let scan = scan.data.unwrap();
    let codex = scan["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_codex_default")
        .unwrap();
    assert_eq!(
        codex["supported"], true,
        "the V2 settings entry treats an executable version as diagnostic, not an allowlist"
    );
    let context = codex["context_id"].as_str().unwrap().to_owned();
    let plans = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap()
        .published_agent_plans()
        .unwrap();
    let plan = plans
        .iter()
        .find(|plan| {
            plan.active
                && plan
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let extra_plan = plans
        .iter()
        .find(|candidate| {
            candidate.agent_plan_id != plan.agent_plan_id
                && candidate.active
                && candidate
                    .supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .unwrap();
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],
        "allowed_plan_ids":[plan.agent_plan_id],
        "default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    for operation in [
        "PreviewAgentConnectionRestore",
        "ApplyAgentConnectionRestore",
    ] {
        let rejected = ordinary.dispatch_wire(request(operation, if operation.starts_with("Preview") {
            json!({"spec":spec})
        } else {
            let digest = CanonicalDigest::of_bytes(b"not-a-restore");
            json!({"spec":spec,"accept_digest":digest,"dependency_digest":digest,
                "expected_revisions":{"target":0,"dependencies":{}},"idempotency_key":"not-a-restore"})
        }, operation.starts_with("Apply").then(|| "not-a-restore".into())));
        assert_eq!(
            rejected.error.unwrap().code,
            api::ErrorCode::InvalidArguments
        );
    }

    let unproven = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(unproven.error.is_none(), "{unproven:?}");
    assert_eq!(
        unproven.data.unwrap()["applicable"],
        false,
        "printing a version is not native authentication proof"
    );
    let payload = json!({"agent_id":"agent_codex_default","scope":"native_authentication","suite":"quick","allow_model_call":false});
    let checked = ordinary.dispatch_wire(request("CheckAgentConnection", payload, None));
    assert!(checked.error.is_none(), "{checked:?}");
    assert_eq!(checked.data.unwrap()["model_verified"], false);
    assert_eq!(fs::read(&path).unwrap(), b"# keep me\nuser_option = true\n");
    let service = ordinary;
    // The first model connection does not need a host login-item declaration.
    let guard_preview = service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(guard_preview.error.is_none(), "{guard_preview:?}");
    let guard_preview = guard_preview.data.unwrap();
    assert_eq!(
        guard_preview["resident_service"]["login_item_required"],
        false
    );
    let first = apply(&service, &runtime, spec.clone(), "settings-first");
    let operation = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(
            &OperationId::parse(first["operation_id"].as_str().unwrap().to_owned()).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(
        operation
            .steps
            .iter()
            .flat_map(|step| step.effects.iter())
            .all(|effect| effect.kind != hiroute_domain::OwnedEffectKind::LoginItem),
        "new settings saves never create a login-item effect"
    );
    let configured = fs::read_to_string(&path).unwrap();
    assert!(configured.contains("experimental_bearer_token"));
    assert!(configured.contains("# keep me"));
    let connection_id = format!("agent-connection/{context}");
    let first_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("configured V2 settings must expose their exact grant through the owner socket");
    let reused = apply(&service, &runtime, spec.clone(), "settings-reuse");
    assert_ne!(reused["operation_id"], first["operation_id"]);
    let reused_operation: OperationId =
        serde_json::from_value(reused["operation_id"].clone()).unwrap();
    let reused_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("an identical settings apply must keep exposing the reused grant");
    assert_eq!(reused_material.sha256(), first_material.sha256());
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}")
            )
            .unwrap(),
        1,
        "an identical settings apply reuses the current grant generation"
    );
    let update_spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure","settings":{
        "mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],
        "allowed_plan_ids":[plan.agent_plan_id,extra_plan.agent_plan_id],
        "default_selection":{"kind":"plan","plan_id":plan.agent_plan_id}}}});
    let update = apply(&service, &runtime, update_spec, "settings-update");
    let update_operation: OperationId =
        serde_json::from_value(update["operation_id"].clone()).unwrap();
    let update_journal = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&update_operation)
        .unwrap()
        .unwrap();
    let update_intent = update_journal
        .plan
        .external()
        .iter()
        .find(|intent| is_settings_codex_model(intent))
        .unwrap();
    let update_payload =
        settings_codex_model_file_for_operation(&update_journal, update_intent).unwrap();
    let CodexModelFileAction::Configure {
        previous_operation, ..
    } = update_payload.change
    else {
        panic!("the update must be a Codex model configuration");
    };
    assert_eq!(previous_operation, Some(reused_operation));
    let updated_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("updated V2 settings must preserve their exact grant");
    assert_eq!(first_material.sha256(), updated_material.sha256());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains(std::str::from_utf8(updated_material.expose()).unwrap())
    );
    let restore_spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{
        "intent":"restore","restore_point_ref":codex_model_restore_point_ref(&update_operation)}});
    apply(&service, &runtime, restore_spec, "settings-restore");
    let restored = fs::read_to_string(&path).unwrap();
    assert!(!restored.contains("experimental_bearer_token"));
    assert!(restored.contains("user_option = true"));
    assert!(
        runtime
            .adapter
            .resolve_active_agent_grant(&connection_id)
            .is_err()
    );
    // After final restore the native file belongs to the user again. Codex may update an
    // unrelated preference before the next HiRoute connection, without making the old
    // restore look like an active connection in drift.
    fs::write(&path, format!("{restored}later_native_option = true\n")).unwrap();
    let disconnected = service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({"schema_version":{"major":2,"minor":0},"context_id":context}),
        None,
    ));
    assert!(disconnected.error.is_none(), "{disconnected:?}");
    assert_eq!(disconnected.data.unwrap()["state"], "not_configured");
    apply(&service, &runtime, spec, "settings-again");
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("later_native_option = true")
    );
    let next_material = runtime
        .adapter
        .resolve_active_agent_grant(&connection_id)
        .expect("reconfigured V2 settings must expose the new exact grant");
    assert_ne!(updated_material.sha256(), next_material.sha256());
    assert_eq!(
        runtime
            .adapter
            .stores_lock()
            .unwrap()
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}")
            )
            .unwrap(),
        4
    );
}

fn apply(
    service: &LocalControlDaemon,
    runtime: &ProductionControlRuntime,
    spec: serde_json::Value,
    key: &str,
) -> serde_json::Value {
    let model_changed = spec["model"].get("intent").is_some();
    let model_restore = spec["model"]["intent"] == "restore";
    let restore = model_restore || spec["collaboration"]["intent"] == "restore";
    let preview_operation = if restore {
        "PreviewAgentConnectionRestore"
    } else {
        "PreviewAgentConnectionChange"
    };
    let apply_operation = if restore {
        "ApplyAgentConnectionRestore"
    } else {
        "ApplyAgentConnectionChange"
    };
    let preview = service.dispatch_wire(request(preview_operation, json!({"spec":spec}), None));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true, "{preview}");
    let mut payload = json!({"spec":spec,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":key});
    // Only restoration of a journal-owned item from an older connection needs a host action.
    if preview["resident_service"]["login_item_removal_required"] == json!(true) {
        payload["login_item"] =
            json!({"before":"enabled","after":"not_registered","created":false});
    }
    if restore {
        let wrong_authority = service.dispatch_wire(request(
            apply_operation,
            payload.clone(),
            Some("unexpected-grant".into()),
        ));
        assert_eq!(
            wrong_authority.error.unwrap().code,
            api::ErrorCode::CapabilityDenied
        );
    }
    let response = service.dispatch_wire(request(apply_operation, payload.clone(), None));
    assert!(response.error.is_none(), "{response:?}");
    let reference = response
        .operation
        .as_ref()
        .expect("accepted settings Operation");
    assert_eq!(
        reference.operation_id,
        response.data.as_ref().unwrap()["operation_id"]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        reference.state,
        response.data.as_ref().unwrap()["state"].as_str().unwrap()
    );
    let data = response.data.unwrap();
    if model_changed {
        if data["state"] != "succeeded" {
            let id = OperationId::parse(data["operation_id"].as_str().unwrap().to_owned()).unwrap();
            let stored = runtime
                .adapter
                .stores_lock()
                .unwrap()
                .control()
                .load_operation(&id)
                .unwrap()
                .unwrap();
            panic!(
                "settings operation failed: code={:?}, steps={:?}",
                stored.safe_error_code,
                stored
                    .steps
                    .iter()
                    .map(|step| (step.kind, step.status, step.terminal_result.as_deref()))
                    .collect::<Vec<_>>()
            );
        }
        let status = service.dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":spec["context_id"]}),
            None,
        ));
        assert!(status.error.is_none(), "{status:?}");
        let status = status.data.unwrap();
        assert_eq!(
            status["state"],
            if model_restore {
                // A formal restore withdraws the grant and removes the owned artifact, so the
                // context returns to its unconfigured baseline.
                "not_configured"
            } else {
                "configured"
            },
            "apply={data} status={status}"
        );
        assert_eq!(status["model_verified"], false);
        if model_restore {
            assert!(status.get("current_selection").is_none());
        } else {
            assert_eq!(status["current_selection"], spec["model"]["settings"]);
        }
    }

    if data["state"] != "succeeded" {
        let operation_id: OperationId =
            serde_json::from_value(data["operation_id"].clone()).unwrap();
        let stores = runtime.adapter.stores_lock().unwrap();
        let operation = stores
            .control()
            .load_operation(&operation_id)
            .unwrap()
            .unwrap();
        let steps = operation
            .steps
            .iter()
            .map(|step| {
                (
                    step.kind,
                    step.status,
                    step.attempts,
                    step.terminal_result.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        panic!(
            "settings operation did not succeed: {data}; safe_error={:?}; steps={steps:?}",
            operation.safe_error_code
        );
    }
    let replay = service.dispatch_wire(request(apply_operation, payload, None));
    assert!(replay.error.is_none(), "{replay:?}");
    assert_eq!(
        replay.operation.unwrap().operation_id,
        data["operation_id"].as_str().unwrap()
    );
    assert_eq!(replay.data.unwrap()["operation_id"], data["operation_id"]);
    data
}
fn request(
    operation: &str,
    payload: serde_json::Value,
    capability: Option<String>,
) -> LocalControlWireRequestV2 {
    LocalControlWireRequestV2 {
        schema_version: api::SchemaVersion::new(2, 0),
        request_id: "settings-entry-test".into(),
        operation_id: operation.into(),
        payload,
        protected_grant: capability.map(|capability| api::ProtectedClientGrantV2 {
            principal_kind: api::PrincipalKind::InteractiveUser,
            capability,
        }),
    }
}

/// Test-only capability and Worker-directory projection. It exercises the public settings
/// transaction and native Skill artifact without asserting a real Worker runtime exists.
struct FixtureFacts {
    adapter: Arc<LocalControlAdapter>,
    fixture_worker_metadata: bool,
}

impl FixtureFacts {
    fn model(adapter: Arc<LocalControlAdapter>) -> Self {
        Self {
            adapter,
            fixture_worker_metadata: false,
        }
    }

    fn collaboration(adapter: Arc<LocalControlAdapter>) -> Self {
        Self {
            adapter,
            fixture_worker_metadata: true,
        }
    }
}

impl AgentConnectionControlPort for FixtureFacts {
    fn settings_status(
        &self,
        request: &api::AgentSettingsStatusRequestV2,
    ) -> Result<api::AgentModelSettingsStatusV2, ControlReadError> {
        self.adapter.settings_status(request)
    }

    fn settings_facts(
        &self,
        spec: &api::AgentSettingsSpecV2,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let mut input = self.adapter.capture_settings_facts(spec)?;
        // The fixture proves the public settings transaction and catalog capability boundary;
        // native fixed-source associations are not supplied by this isolated environment.
        if self.fixture_worker_metadata {
            input.facts.dependency_digest = CanonicalDigest::of(&(
                "settings-entry-fixture-skill-capabilities/v1",
                &input.facts.dependency_digest,
            ))
            .map_err(|_| ControlReadError::Corrupt)?;
        }
        let mut capabilities = vec![
            AgentCapability::EffectiveConfiguration,
            AgentCapability::AtomicManagedReplace,
            AgentCapability::IngressAuthentication,
        ];
        if self.fixture_worker_metadata {
            capabilities.extend([
                AgentCapability::SkillLoading,
                AgentCapability::TrustedCliExecution,
            ]);
        }
        input.facts.capabilities =
            AgentCapabilitySet::new(capabilities.into_iter().map(|capability| {
                CapabilityEvidence {
                    capability,
                    state: CapabilityState::Proven,
                    adapter_contract: "explicit-test-fixture/1".into(),
                    observed_at_unix_ms: 1,
                    dependency_digest: input.facts.dependency_digest.clone(),
                    reason: None,
                }
            }))
            .unwrap();
        Ok(input)
    }
    fn planning_facts(
        &self,
        spec: &api::AgentConnectSpecV1,
    ) -> Result<AgentConnectionPlanningInputV1, ControlReadError> {
        self.adapter.planning_facts(spec)
    }
    fn connection_status(
        &self,
        request: &api::AgentConnectionStatusRequestV1,
    ) -> Result<api::AgentConnectionStatusV1, ControlReadError> {
        self.adapter.connection_status(request)
    }
    fn managed_launch_descriptor(
        &self,
        request: &api::AgentLaunchDescriptorRequestV1,
    ) -> Result<api::ManagedClaudeLaunchDescriptorV2, ControlReadError> {
        self.adapter.managed_launch_descriptor(request)
    }
}

#[path = "settings_entry_facets_tests.rs"]
mod facets;
