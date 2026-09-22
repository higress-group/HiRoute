//! Real native renderer → protected store → disk → restart → field restore.
//! Component integration; not a substitute for the production CLI/daemon entry path.
use super::*;
use hiroute_application::agent_connection::*;
use hiroute_domain::NativeAgentArtifactPort;
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1,
    AgentConnectionTransactionKindV1, AgentConnectionTransactionSubjectV1,
};
use hiroute_integrations::{
    CodexFileConfiguration, CodexSelectionTarget, stage_codex_configuration,
    stage_codex_restoration,
};
use std::os::unix::fs::PermissionsExt;

#[path = "native_claude_file_tests.rs"]
mod native_claude_file_tests;
#[path = "native_codex_catalog_tests.rs"]
mod native_codex_catalog_tests;
#[path = "native_codex_test.rs"]
mod native_codex_test;
#[path = "settings_skill_journal_tests.rs"]
mod settings_skill_journal_tests;
#[path = "skill_parent_tests.rs"]
mod skill_parent_tests;

fn intent(
    kind: AgentConnectionTransactionKindV1,
    role: AgentConnectionEffectRoleV1,
    before: Option<CanonicalDigest>,
) -> ExternalEffectIntentV1 {
    intent_for_agent(kind, role, before, "agent.codex", "codex.profile.v1")
}
fn intent_for_agent(
    kind: AgentConnectionTransactionKindV1,
    role: AgentConnectionEffectRoleV1,
    before: Option<CanonicalDigest>,
    agent: &str,
    profile: &str,
) -> ExternalEffectIntentV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: kind.command_id().into(),
        resource_id: Some("agent-connection/native".into()),
        desired_state: json!({"agent_id":agent, "profile_id":"default"}),
    };
    let subject =
        AgentConnectionTransactionSubjectV1::from_registered_profile(agent, "default", profile)
            .unwrap();
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        kind,
        subject,
        &spec,
        &json!({"configuration":"native"}),
    )
    .unwrap();
    ExternalEffectIntentV1::from_agent_connection_planner(
        &control,
        role,
        before,
        &json!({"native_effect":"fixture-v1"}),
        if role == AgentConnectionEffectRoleV1::ManagedConfiguration {
            0o600
        } else {
            0o644
        },
    )
    .unwrap()
}
fn store(root: &Path, target: &str, path: &Path) -> ManagedArtifactStore {
    ManagedArtifactStore::open_with_external_target(
        &crate::test_storage_authority(),
        root.join("artifacts"),
        root.join("restore"),
        target,
        path,
    )
    .unwrap()
}
fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn apply(
    store: &ManagedArtifactStore,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    before: &[u8],
) -> OwnedEffectV1 {
    stage_codex_configuration(
        store,
        operation,
        intent,
        CodexFileConfiguration {
            expected_content: &CanonicalDigest::of_bytes(before),
            selection: CodexSelectionTarget::Root,
            provider_id: "hiroute",
            endpoint: "http://127.0.0.1:5837/v1",
            model: Some("hiroute/0011223344556677"),
            local_grant: &AgentAccessGrantMaterial::from_csprng_entropy([7; 32]),
            model_catalog: None,
            managed_aliases: &[],
        },
    )
    .unwrap()
}

#[test]
fn codex_real_file_restore_reopens_and_preserves_user_edits() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let original = b"# private configuration\nmodel = 'original'\nunrelated = 1\n";
    write(&path, original);
    let prototype = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        None,
    );
    let first = store(temp.path(), prototype.target(), &path);
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        first
            .current_external_fingerprint(prototype.target())
            .unwrap(),
    );
    let operation = OperationId::parse("op_11111111111111111111111111111111").unwrap();
    let staged = apply(&first, &operation, &install, original);
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(first);
    let reopened = store(temp.path(), install.target(), &path);
    reopened.activate_artifact(&staged).unwrap();
    assert_eq!(apply(&reopened, &operation, &install, original), staged);
    let changed = fs::read_to_string(&path)
        .unwrap()
        .replace("unrelated = 1", "unrelated = 2");
    write(&path, changed.as_bytes());
    let restore = intent(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        reopened
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    let restore_operation = OperationId::parse("op_22222222222222222222222222222222").unwrap();
    let staged_restore = stage_codex_restoration(
        &reopened,
        &restore_operation,
        &restore,
        &operation,
        &install,
        None,
    )
    .unwrap();
    drop(reopened);
    let reopened = store(temp.path(), install.target(), &path);
    reopened.activate_artifact(&staged_restore).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        std::str::from_utf8(original)
            .unwrap()
            .replace("unrelated = 1", "unrelated = 2")
    );
    assert!(
        !fs::read_to_string(&path)
            .unwrap()
            .contains("experimental_bearer_token")
    );
}

#[test]
fn codex_created_file_is_removed_only_when_no_user_content_remains() {
    for add_user_content in [false, true] {
        let temp = crate::test_tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let install = intent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            None,
        );
        let store = store(temp.path(), install.target(), &path);
        let operation = OperationId::parse("op_33333333333333333333333333333333").unwrap();
        let effect = apply(&store, &operation, &install, b"");
        store.activate_artifact(&effect).unwrap();
        if add_user_content {
            let current = fs::read_to_string(&path).unwrap();
            write(&path, format!("user_setting = true\n{current}").as_bytes());
        }
        let restore = intent(
            AgentConnectionTransactionKindV1::Restore,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            store
                .current_external_fingerprint(install.target())
                .unwrap(),
        );
        let restore_operation = OperationId::parse("op_44444444444444444444444444444444").unwrap();
        let effect = stage_codex_restoration(
            &store,
            &restore_operation,
            &restore,
            &operation,
            &install,
            None,
        )
        .unwrap();
        store.activate_artifact(&effect).unwrap();
        assert_eq!(path.exists(), add_user_content);
        if add_user_content {
            assert_eq!(
                fs::read_to_string(&path).unwrap().trim(),
                "user_setting = true"
            );
        }
    }
}

#[test]
fn collaboration_skill_real_file_survives_shared_reference_until_final_remove() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("SKILL.md");
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::RoutingSkill,
        None,
    );
    let store = store(temp.path(), install.target(), &path);
    let template = CollaborationSkillTemplate::bundled(
        "fixture-v1",
        "# Collaboration\nUse the approved CLI.\n",
    )
    .unwrap();
    let mut planned = plan_skill_install("skill/root", "context/a", &template, None, None).unwrap();
    let operation = OperationId::parse("op_55555555555555555555555555555555").unwrap();
    let effect = stage_collaboration_skill(
        &store,
        &operation,
        &install,
        None,
        &mut planned,
        Some(&template),
    )
    .unwrap()
    .unwrap();
    store.activate_artifact(&effect).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), template.content);
    let shared = plan_skill_install(
        "skill/root",
        "context/b",
        &template,
        Some(&planned.next),
        Some(&template.digest),
    )
    .unwrap();
    let first_remove =
        plan_skill_remove("context/a", &shared.next, Some(&template.digest)).unwrap();
    assert_eq!(first_remove.file_action, SkillFileAction::Keep);
    let mut last_remove =
        plan_skill_remove("context/b", &first_remove.next, Some(&template.digest)).unwrap();
    let remove = intent(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionEffectRoleV1::RoutingSkill,
        store
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    let operation = OperationId::parse("op_66666666666666666666666666666666").unwrap();
    let effect = stage_collaboration_skill(
        &store,
        &operation,
        &remove,
        Some(&first_remove.next),
        &mut last_remove,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(path.exists());
    store.activate_artifact(&effect).unwrap();
    assert!(!path.exists());
}

#[test]
fn native_owned_field_conflict_and_skill_user_edit_leave_files_unchanged() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("config.toml");
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        None,
    );
    let artifacts = store(temp.path(), install.target(), &path);
    let operation = OperationId::parse("op_77777777777777777777777777777777").unwrap();
    let effect = apply(&artifacts, &operation, &install, b"");
    artifacts.activate_artifact(&effect).unwrap();
    let current = fs::read_to_string(&path)
        .unwrap()
        .replace("hiroute/0011223344556677", "user-chosen-model");
    write(&path, current.as_bytes());
    let restore = intent(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    let restore_operation = OperationId::parse("op_88888888888888888888888888888888").unwrap();
    assert_eq!(
        stage_codex_restoration(
            &artifacts,
            &restore_operation,
            &restore,
            &operation,
            &install,
            None
        )
        .unwrap_err()
        .code,
        PortErrorCode::Conflict
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), current);

    let skill_root = crate::test_tempdir().unwrap();
    let skill_path = skill_root.path().join("SKILL.md");
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::RoutingSkill,
        None,
    );
    let artifacts = store(skill_root.path(), install.target(), &skill_path);
    let template = CollaborationSkillTemplate::bundled("fixture-v1", "# approved skill\n").unwrap();
    let mut planned = plan_skill_install("skill/root", "context/a", &template, None, None).unwrap();
    let effect = stage_collaboration_skill(
        &artifacts,
        &operation,
        &install,
        None,
        &mut planned,
        Some(&template),
    )
    .unwrap()
    .unwrap();
    artifacts.activate_artifact(&effect).unwrap();
    let mut removal =
        plan_skill_remove("context/a", &planned.next, Some(&template.digest)).unwrap();
    let remove = intent(
        AgentConnectionTransactionKindV1::Restore,
        AgentConnectionEffectRoleV1::RoutingSkill,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    write(&skill_path, b"user edited skill after preview");
    assert_eq!(
        stage_collaboration_skill(
            &artifacts,
            &restore_operation,
            &remove,
            Some(&planned.next),
            &mut removal,
            None
        )
        .unwrap_err()
        .code,
        PortErrorCode::Conflict
    );
    assert_eq!(
        fs::read(&skill_path).unwrap(),
        b"user edited skill after preview"
    );
}

#[test]
fn native_created_directories_cleanup_after_reopen_preserves_existing_parent_and_user_content() {
    for add_user_content in [false, true] {
        let temp = crate::test_tempdir().unwrap();
        let path = temp.path().join("new-skills/managed/SKILL.md");
        let install = intent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::RoutingSkill,
            None,
        );
        let artifacts = store(temp.path(), install.target(), &path);
        assert!(
            !path.parent().unwrap().exists(),
            "binding and preview remain read-only"
        );
        let operation = OperationId::parse("op_99999999999999999999999999999999").unwrap();
        let effect = artifacts
            .stage_native_target(&operation, &install, Some(b"owned skill"), false)
            .unwrap();
        artifacts.activate_artifact(&effect).unwrap();
        assert!(
            !artifacts
                .cleanup_native_parents(&operation, &install)
                .unwrap(),
            "live file keeps its directories"
        );
        let removal = intent(
            AgentConnectionTransactionKindV1::Restore,
            AgentConnectionEffectRoleV1::RoutingSkill,
            artifacts
                .current_external_fingerprint(install.target())
                .unwrap(),
        );
        let remove_operation = OperationId::parse("op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let effect = artifacts
            .stage_native_target(&remove_operation, &removal, None, false)
            .unwrap();
        artifacts.activate_artifact(&effect).unwrap();
        if add_user_content {
            write(&path.parent().unwrap().join("user.txt"), b"keep");
        }
        drop(artifacts);
        let reopened = store(temp.path(), install.target(), &path);
        assert_eq!(
            reopened
                .cleanup_native_parents(&operation, &install)
                .unwrap(),
            !add_user_content
        );
        assert_eq!(path.parent().unwrap().exists(), add_user_content);
        assert!(temp.path().is_dir());
        assert_eq!(
            reopened
                .cleanup_native_parents(&operation, &install)
                .unwrap(),
            !add_user_content
        );
    }
}

#[test]
fn native_directory_replacement_is_not_removed_as_owned() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("managed/SKILL.md");
    let install = intent(
        AgentConnectionTransactionKindV1::Apply,
        AgentConnectionEffectRoleV1::RoutingSkill,
        None,
    );
    let artifacts = store(temp.path(), install.target(), &path);
    let operation = OperationId::parse("op_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
    let effect = artifacts
        .stage_native_target(&operation, &install, Some(b"owned skill"), false)
        .unwrap();
    artifacts.activate_artifact(&effect).unwrap();
    // Retain the old directory under a new name so its inode cannot be reused by this fixture.
    fs::rename(
        path.parent().unwrap(),
        temp.path().join("user-retained-copy"),
    )
    .unwrap();
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !artifacts
            .cleanup_native_parents(&operation, &install)
            .unwrap()
    );
    assert!(path.parent().unwrap().exists());
}

#[test]
fn all_native_target_bindings_are_restored_together_before_marker_validation() {
    let root = crate::test_tempdir().unwrap();
    let targets = [
        intent_for_agent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            None,
            "agent.one",
            "profile.one",
        ),
        intent_for_agent(
            AgentConnectionTransactionKindV1::Apply,
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            None,
            "agent.two",
            "profile.two",
        ),
    ];
    let paths = [root.path().join("one.toml"), root.path().join("two.toml")];
    let bindings = || {
        targets
            .iter()
            .zip(&paths)
            .map(|(intent, path)| (intent.target().to_owned(), path.clone()))
            .collect::<Vec<_>>()
    };
    let open = |bindings| {
        ManagedArtifactStore::open_with_external_targets(
            &crate::test_storage_authority(),
            root.path().join("artifacts"),
            root.path().join("restore"),
            bindings,
        )
    };
    let store = open(bindings()).unwrap();
    let operations =
        [0xa14090, 0xa14091].map(|id| OperationId::parse(format!("op_{id:032x}")).unwrap());
    for (intent, operation) in targets.iter().zip(&operations) {
        let staged = apply(&store, operation, intent, b"");
        store.activate_artifact(&staged).unwrap();
    }
    drop(store);
    let reopened = open(bindings()).unwrap();
    for (intent, operation) in targets.iter().zip(&operations) {
        assert!(matches!(
            reopened.observe_artifact(operation, intent).unwrap(),
            EffectReconciliation::Applied(_)
        ));
    }
    assert!(
        open(vec![(targets[0].target().into(), paths[0].clone())]).is_err(),
        "missing binding must not redirect recovery"
    );
    let mut duplicate = bindings();
    duplicate.push(duplicate[0].clone());
    assert!(open(duplicate).is_err());
    let mut redirected = bindings();
    redirected[1].1 = root.path().join("replacement.toml");
    assert!(open(redirected).is_err());
}
