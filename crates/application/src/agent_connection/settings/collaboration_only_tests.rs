//! The collaboration capability has its own confirmation and effects, with no model shell.
use super::*;
use crate::agent_connection::{
    AgentSettingsPlanningInput, CollaborationSkillTemplate, SettingsSkillFileFacts,
};
use hiroute_application_api::AgentSettingsApplyV2;
use hiroute_domain::{AgentConnectionTransactionSubjectV1, RevisionSetV1};

fn collaboration_spec() -> AgentSettingsSpecV2 {
    serde_json::from_value(json!({
        "schema_version":{"major":2,"minor":0},"context_id":"agent-context/test",
        "collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}}
    }))
    .unwrap()
}

fn input(facts: AgentSettingsFacts) -> AgentSettingsPlanningInput {
    AgentSettingsPlanningInput {
        additional_skill_file: None,
        collaboration_state: json!({"trigger_mode":"explicit"}),
        facts,
        expected_revisions: RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        subject: AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_qoder_default",
            "default",
            "builtin/qoder-collaboration/v1",
        )
        .unwrap(),
        model_file: None,
        skill_file: SettingsSkillFileFacts {
            root_ref: "skill-root/qoder".into(),
            target: "/selected/qoder/skills/hiroute-collaboration/SKILL.md".into(),
            before: None,
            observed_file: None,
            before_fingerprint: None,
            template: CollaborationSkillTemplate::bundled("fixture/v1", "# Collaboration\n")
                .unwrap(),
        },
    }
}

fn confirm(spec: AgentSettingsSpecV2, facts: &AgentSettingsFacts) -> ConfirmedAgentSettings {
    let preview = preview_agent_settings(spec.clone(), facts).unwrap();
    confirm_agent_settings(
        AgentSettingsApplyV2 {
            expected_revisions: RevisionSetV1 {
                target: 0,
                dependencies: BTreeMap::new(),
            },
            spec,
            accept_digest: preview.accept_digest,
            dependency_digest: preview.dependency_digest,
            idempotency_key: "collaboration-only-confirmation".into(),
            login_item: None,
        },
        facts,
    )
    .unwrap()
}

#[test]
fn collaboration_only_confirmation_seals_only_the_skill_effect_without_model_facts() {
    let mut fresh = facts();
    fresh.model = None;
    let confirmed = confirm(collaboration_spec(), &fresh);
    assert!(confirmed.preview().model_grant.is_none());
    assert!(confirmed.preview().context_windows.is_empty());
    let plan = input(fresh).seal_settings(&confirmed).unwrap();
    assert_eq!(plan.external().len(), 1);
    assert!(plan.agent_access_grants().is_empty());
    assert!(plan.secrets().is_empty());
    assert!(plan.runtime().is_empty());
    assert_eq!(plan.spec().desired_state["model"]["intent"], "keep");
    assert!(plan.control()["payload"]["state"]["model_grant"].is_null());
}

#[test]
fn collaboration_only_preview_rejects_every_model_mutation() {
    let mut fresh = facts();
    fresh.model = None;
    let baseline = collaboration_spec();
    for model in [
        json!({"intent":"restore","restore_point_ref":"restore/model"}),
        json!({"intent":"configure","settings":{
            "mode":"codex_default","native_model_mode":"hiroute_only", "fixed_models":[],
            "allowed_plan_ids":[], "default_selection":{"kind":"preserve_native"}
        }}),
    ] {
        let mut wire = serde_json::to_value(&baseline).unwrap();
        wire["model"] = model;
        let spec = serde_json::from_value(wire).unwrap();
        assert_eq!(
            preview_agent_settings(spec, &fresh).unwrap_err(),
            SettingsPlanningError::InvalidSelection
        );
    }
    for access_token in [
        hiroute_domain::AgentAccessTokenIntentV1::Regenerate,
        hiroute_domain::AgentAccessTokenIntentV1::Set {
            input_slot: "token/input".into(),
        },
    ] {
        let mut spec = baseline.clone();
        spec.access_token = access_token;
        assert_eq!(
            preview_agent_settings(spec, &fresh).unwrap_err(),
            SettingsPlanningError::InvalidSelection
        );
    }
}

#[test]
fn sealing_rechecks_model_support_even_for_a_previously_confirmed_restore() {
    let mut fresh = facts();
    let proof = CapabilityEvidence {
        capability: AgentCapability::EffectiveConfiguration,
        state: CapabilityState::Proven,
        adapter_contract: "fixture/restore".into(),
        observed_at_unix_ms: 1,
        dependency_digest: fresh.dependency_digest.clone(),
        reason: None,
    };
    fresh.capabilities = AgentCapabilitySet::new([
        proof.clone(),
        CapabilityEvidence {
            capability: AgentCapability::AtomicManagedReplace,
            ..proof
        },
    ])
    .unwrap();
    fresh
        .restore_points
        .insert("restore/model".into(), AgentSettingsFacet::Model);
    let mut spec = collaboration_spec();
    spec.collaboration = AgentFacetIntent::Keep;
    spec.model = AgentFacetIntent::Restore {
        restore_point_ref: "restore/model".into(),
    };
    let confirmed = confirm(spec, &fresh);
    // Even if a buggy facts provider retained a digest, the absent model facet cannot be
    // converted into model file/grant effects or a default configuration target.
    fresh.model = None;
    assert!(input(fresh).seal_settings(&confirmed).is_err());
}

#[test]
fn both_skills_seal_distinct_owned_effects_without_model_authority() {
    let mut facts = facts();
    facts.model = None;
    let mut spec = collaboration_spec();
    spec.management_skill = Some(AgentFacetIntent::Configure {
        settings: hiroute_domain::AgentManagementSkillSelectionV1 {},
    });
    let confirmed = confirm(spec, &facts);
    assert_eq!(
        confirmed.preview().changed_facets,
        BTreeSet::from([
            AgentSettingsFacet::ManagementSkill,
            AgentSettingsFacet::Collaboration,
        ])
    );
    let mut input = input(facts);
    input.additional_skill_file = Some(SettingsSkillFileFacts {
        root_ref: "skill-root/qoder/additional".into(),
        target: "agent-connections/test/management-skill".into(),
        before: None,
        observed_file: None,
        before_fingerprint: None,
        template: CollaborationSkillTemplate::bundled("management/v1", "# Management\n").unwrap(),
    });
    let plan = input.seal_settings(&confirmed).unwrap();
    assert!(plan.agent_access_grants().is_empty());
    assert_eq!(plan.external().len(), 2);
    assert_eq!(
        plan.external()
            .iter()
            .map(|i| i.effect_id())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "agent-connection-routing-skill",
            "agent-connection-management-skill",
        ])
    );
    for missing in 0..2 {
        let effects = plan
            .external()
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != missing)
            .map(|(_, e)| e.clone())
            .collect();
        assert!(
            hiroute_domain::TransactionPlanV1::from_registered_typed_planner(
                plan.spec().clone(),
                plan.control().clone(),
                None,
                vec![],
                vec![],
                effects,
            )
            .is_err(),
            "recovery must retain both exact effects"
        );
    }
}

#[test]
fn management_conflict_blocks_only_the_requested_management_facet() {
    let mut facts = facts();
    facts.model = None;
    facts.management_file_conflict = true;
    let mut spec = collaboration_spec();
    assert!(
        preview_agent_settings(spec.clone(), &facts)
            .unwrap()
            .blockers
            .is_empty()
    );
    spec.collaboration = AgentFacetIntent::Keep;
    spec.management_skill = Some(AgentFacetIntent::Configure {
        settings: hiroute_domain::AgentManagementSkillSelectionV1 {},
    });
    let preview = preview_agent_settings(spec, &facts).unwrap();
    assert_eq!(preview.blockers.len(), 1);
    assert_eq!(
        preview.blockers[0].facet,
        AgentSettingsFacet::ManagementSkill
    );
}
