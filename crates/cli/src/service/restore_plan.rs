//! Restore the actual owning contexts, rather than a UI status projection.
use hiroute_application_api::{
    AGENT_SETTINGS_SCHEMA_V2, AgentAccessTokenIntentV1, AgentFacetIntent,
    AgentModelSettingsStateV2, AgentNativeTargetV1, AgentSettingsSpecV2, AgentSettingsStatusV2,
};
use serde_json::Value;
use std::{collections::BTreeMap, io};

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Context {
    pub id: String,
    pub agent: String,
    pub model: bool,
    pub collaboration: bool,
    pub management_skill: bool,
    pub native_target: Option<AgentNativeTargetV1>,
}

pub(super) fn decision(mut agents: Vec<String>) -> io::Error {
    agents.sort_unstable();
    agents.dedup();
    io::Error::other(format!(
        "AGENT_RESTORE_REQUIRED:{}",
        serde_json::to_string(&agents).unwrap_or_default()
    ))
}

pub(super) fn contexts(list: &Value) -> io::Result<Vec<Context>> {
    let unavailable = || io::Error::other("AGENT_RESTORE_UNAVAILABLE");
    let agents = list["agents"].as_array().ok_or_else(unavailable)?;
    let mut contexts = BTreeMap::new();
    for agent in agents {
        let Some(context) = agent["context_id"].as_str().filter(|c| !c.is_empty()) else {
            continue;
        };
        let name = agent["agent_id"].as_str().unwrap_or(context);
        let target: Option<AgentNativeTargetV1> =
            serde_json::from_value(agent["native_target"].clone())
                .map_err(|_| decision(vec![name.into()]))?;
        if target
            .as_ref()
            .is_some_and(|target| target.validate().is_err())
            || (name == "agent_hermes_default" && target.is_none())
        {
            return Err(decision(vec![name.into()]));
        }
        let mut insert = |id: &str, model, collaboration| {
            let value = Context {
                id: id.into(),
                agent: name.into(),
                model,
                collaboration,
                management_skill: collaboration,
                native_target: target.clone(),
            };
            if contexts.insert(id.to_owned(), value).is_some() {
                return Err(unavailable());
            }
            Ok(())
        };
        if name == "agent_codex_default" {
            let access = &agent["codex_access"];
            let root = access["root_context_id"]
                .as_str()
                .filter(|c| !c.is_empty())
                .ok_or_else(unavailable)?;
            let profile = access["profile_context_id"]
                .as_str()
                .filter(|c| !c.is_empty())
                .ok_or_else(unavailable)?;
            if root == profile || ![root, profile].contains(&context) {
                return Err(unavailable());
            }
            // Profile status deliberately displays both root-owned Skill facets.
            // Their restore points belong to the root, never to the profile.
            insert(context, true, context == root)?;
            if context != root {
                insert(root, false, true)?;
            }
        } else if name == "agent_dsh_desktop" {
            // Desktop exposes a model facet only; its Skills belong to the
            // separate DSH context and are not present in Desktop status.
            insert(context, true, false)?;
        } else {
            insert(context, true, true)?;
        }
    }
    Ok(contexts.into_values().collect())
}

pub(super) fn spec(
    context: &Context,
    status: &AgentSettingsStatusV2,
) -> Option<AgentSettingsSpecV2> {
    use AgentModelSettingsStateV2::{Configured, NotConfigured, Restored};
    if context.agent == "agent_dsh_desktop" && status.model().is_none() {
        return None;
    }
    let restore = |state: &AgentModelSettingsStateV2, point: &Option<String>| match state {
        NotConfigured | Restored => Some(None),
        Configured => point.clone().map(Some),
        _ => None,
    };
    let model = match (context.model, status.model()) {
        (true, Some(m)) => restore(&m.state, &m.restore_point_ref)?,
        _ => None,
    };
    let collaboration = match (context.collaboration, status.collaboration()) {
        (true, Some(c)) => restore(&c.state, &c.restore_point_ref)?,
        _ => None,
    };
    let management = if context.management_skill {
        let management = match status {
            AgentSettingsStatusV2::Model(model) => model.management_skill.as_ref(),
            AgentSettingsStatusV2::CollaborationOnly(status) => status.management_skill.as_ref(),
        }?;
        if management.schema != "hiroute.agent-management-skill-status/v1" {
            return None;
        }
        restore(&management.state, &management.restore_point_ref)?
    } else {
        None
    };
    Some(AgentSettingsSpecV2 {
        schema_version: AGENT_SETTINGS_SCHEMA_V2,
        context_id: context.id.clone(),
        native_target: context.native_target.clone(),
        management_skill: management
            .map(|restore_point_ref| AgentFacetIntent::Restore { restore_point_ref }),
        model: model
            .map(|restore_point_ref| AgentFacetIntent::Restore { restore_point_ref })
            .unwrap_or_default(),
        collaboration: collaboration
            .map(|restore_point_ref| AgentFacetIntent::Restore { restore_point_ref })
            .unwrap_or_default(),
        restore_native_model: None,
        protected_native_model_ids: vec![],
        access_token: AgentAccessTokenIntentV1::Keep,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codex(selected: &str) -> Value {
        json!({"agents":[{"agent_id":"agent_codex_default","context_id":selected,
            "codex_access":{"root_context_id":"root","profile_context_id":"profile"}}]})
    }

    fn status(context: &str, model: &str, collaboration: &str) -> AgentSettingsStatusV2 {
        serde_json::from_value(json!({"schema":"hiroute.agent-model-settings-status/v2",
            "context_id":context,"state":model,"operation_id":null,"operation_state":null,
            "restore_point_ref":if model == "configured" { Some("model-point") } else { None },
            "applied_revision":null,"surface_results":[],"model_verified":false,
            "management_skill":{"schema":"hiroute.agent-management-skill-status/v1",
                "state":"not_configured","operation_id":null,"operation_state":null,"restore_point_ref":null},
            "collaboration":{"schema":"hiroute.agent-collaboration-settings-status/v2",
                "state":collaboration,"operation_id":null,"operation_state":null,
                "restore_point_ref":if collaboration == "configured" { Some("root-skill-point") } else { None }}})).unwrap()
    }

    fn management_status(context: &str, state: &str) -> AgentSettingsStatusV2 {
        let mut value =
            serde_json::to_value(status(context, "not_configured", "not_configured")).unwrap();
        value["management_skill"]["state"] = json!(state);
        if state == "configured" {
            value["management_skill"]["restore_point_ref"] = json!("management-point");
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn management_only_restore_keeps_the_explicit_native_target_and_all_other_facets() {
        let root = tempfile::tempdir().unwrap();
        let target = AgentNativeTargetV1::Hermes {
            config_root: root.path().to_str().unwrap().into(),
        };
        let rows = contexts(&json!({"agents":[{"agent_id":"agent_hermes_default","context_id":"hermes","native_target":target}]})).unwrap();
        let restore = spec(&rows[0], &management_status("hermes", "configured")).unwrap();
        assert!(restore.is_restore_only());
        assert_eq!(restore.native_target, Some(target));
        assert_eq!(
            restore.management_skill,
            Some(AgentFacetIntent::Restore {
                restore_point_ref: "management-point".into()
            })
        );
        assert!(matches!(restore.model, AgentFacetIntent::Keep));
        assert!(matches!(restore.collaboration, AgentFacetIntent::Keep));
        for state in ["drift", "pending", "needs_attention"] {
            assert!(spec(&rows[0], &management_status("hermes", state)).is_none());
        }
        let mut missing = serde_json::to_value(management_status("hermes", "configured")).unwrap();
        missing["management_skill"]["restore_point_ref"] = Value::Null;
        assert!(spec(&rows[0], &serde_json::from_value(missing).unwrap()).is_none());
        let missing_target =
            json!({"agents":[{"agent_id":"agent_hermes_default","context_id":"hermes"}]});
        assert!(
            super::contexts(&missing_target)
                .unwrap_err()
                .to_string()
                .starts_with("AGENT_RESTORE_REQUIRED:")
        );
    }

    #[test]
    fn codex_management_projection_is_restored_only_through_the_original_root() {
        let rows = contexts(&codex("profile")).unwrap();
        let profile = rows.iter().find(|row| row.id == "profile").unwrap();
        let root = rows.iter().find(|row| row.id == "root").unwrap();
        assert!(
            spec(profile, &management_status("profile", "configured"))
                .unwrap()
                .management_skill
                .is_none()
        );
        assert!(matches!(
            spec(root, &management_status("root", "configured"))
                .unwrap()
                .management_skill,
            Some(AgentFacetIntent::Restore { .. })
        ));
        let mut missing = serde_json::to_value(management_status("root", "configured")).unwrap();
        missing.as_object_mut().unwrap().remove("management_skill");
        assert!(spec(root, &serde_json::from_value(missing).unwrap()).is_none());
    }

    #[test]
    fn codex_profile_projection_restores_the_skill_only_through_its_root() {
        for selected in ["profile", "root"] {
            let contexts = contexts(&codex(selected)).unwrap();
            assert_eq!(contexts.len(), if selected == "profile" { 2 } else { 1 });
            assert!(contexts.iter().all(|c| c.agent == "agent_codex_default"));
            let root = contexts.iter().find(|c| c.id == "root").unwrap();
            if selected == "profile" {
                let profile = contexts.iter().find(|c| c.id == "profile").unwrap();
                let profile =
                    spec(profile, &status("profile", "configured", "configured")).unwrap();
                assert!(matches!(profile.model, AgentFacetIntent::Restore { .. }));
                assert!(matches!(profile.collaboration, AgentFacetIntent::Keep));
            }
            let root = spec(root, &status("root", "not_configured", "configured")).unwrap();
            assert!(matches!(root.model, AgentFacetIntent::Keep));
            assert_eq!(
                root.collaboration,
                AgentFacetIntent::Restore {
                    restore_point_ref: "root-skill-point".into()
                }
            );
        }
    }

    #[test]
    fn collaboration_only_codex_does_not_submit_a_restore_to_an_empty_profile() {
        let contexts = contexts(&codex("profile")).unwrap();
        let profile = contexts.iter().find(|c| c.id == "profile").unwrap();
        assert!(
            !spec(profile, &status("profile", "not_configured", "configured"))
                .unwrap()
                .is_restore_only()
        );
        let root = contexts.iter().find(|c| c.id == "root").unwrap();
        assert!(
            spec(root, &status("root", "not_configured", "configured"))
                .unwrap()
                .is_restore_only()
        );
    }

    #[test]
    fn dsh_desktop_restores_only_its_supported_model_facet() {
        let list = json!({"agents":[{"agent_id":"agent_dsh_desktop","context_id":"desktop"}]});
        let contexts = contexts(&list).unwrap();
        let desktop = &contexts[0];
        assert!(desktop.model);
        assert!(!desktop.collaboration);
        assert!(!desktop.management_skill);

        for state in ["not_configured", "configured", "drift"] {
            let mut value =
                serde_json::to_value(status("desktop", state, "not_configured")).unwrap();
            value.as_object_mut().unwrap().remove("management_skill");
            value.as_object_mut().unwrap().remove("collaboration");
            let status = serde_json::from_value(value).unwrap();
            let plan = spec(desktop, &status);
            if state == "drift" {
                assert!(
                    plan.is_none(),
                    "an unresolved model conflict must still block uninstall"
                );
                continue;
            }
            let plan = plan.unwrap();
            assert_eq!(plan.management_skill, None);
            assert!(matches!(plan.collaboration, AgentFacetIntent::Keep));
            if state == "configured" {
                assert!(matches!(plan.model, AgentFacetIntent::Restore { .. }));
            } else {
                assert!(matches!(plan.model, AgentFacetIntent::Keep));
            }
        }

        let mut missing =
            serde_json::to_value(status("desktop", "configured", "not_configured")).unwrap();
        missing["restore_point_ref"] = serde_json::Value::Null;
        assert!(spec(desktop, &serde_json::from_value(missing).unwrap()).is_none());

        let wrong_shape = json!({"schema":"hiroute.agent-collaboration-only-settings-status/v2",
            "context_id":"desktop", "collaboration":{
                "schema":"hiroute.agent-collaboration-settings-status/v2", "state":"not_configured",
                "operation_id":null, "operation_state":null, "restore_point_ref":null}});
        assert!(spec(desktop, &serde_json::from_value(wrong_shape).unwrap()).is_none());
    }

    #[test]
    fn blocked_codex_contexts_report_one_agent_without_hiding_other_agents() {
        let mut blocked: Vec<_> = contexts(&codex("profile"))
            .unwrap()
            .into_iter()
            .map(|context| context.agent)
            .collect();
        blocked.insert(1, "agent_claude_default".into());
        let error = decision(blocked);
        assert_eq!(error.kind(), io::ErrorKind::Other);
        let message = error.to_string();
        let agents: Vec<String> =
            serde_json::from_str(message.strip_prefix("AGENT_RESTORE_REQUIRED:").unwrap()).unwrap();
        assert_eq!(agents, ["agent_claude_default", "agent_codex_default"]);
        assert_eq!(
            decision(vec!["agent_codex_default".into()]).to_string(),
            "AGENT_RESTORE_REQUIRED:[\"agent_codex_default\"]"
        );
    }

    #[test]
    fn root_conflicts_and_missing_restore_points_still_require_a_user_decision() {
        let contexts = contexts(&codex("profile")).unwrap();
        let root = contexts.iter().find(|c| c.id == "root").unwrap();
        for state in ["drift", "pending", "needs_attention"] {
            assert!(spec(root, &status("root", "not_configured", state)).is_none());
        }
        let mut missing = status("root", "not_configured", "configured");
        if let AgentSettingsStatusV2::Model(model) = &mut missing {
            model.collaboration.as_mut().unwrap().restore_point_ref = None;
        }
        assert!(spec(root, &missing).is_none());
    }

    #[test]
    fn ordinary_contexts_keep_both_facets_and_ambiguous_codex_ownership_is_refused() {
        let ordinary =
            json!({"agents":[{"agent_id":"agent_claude_default","context_id":"claude"}]});
        let contexts = contexts(&ordinary).unwrap();
        let plan = spec(&contexts[0], &status("claude", "configured", "configured")).unwrap();
        assert!(matches!(plan.model, AgentFacetIntent::Restore { .. }));
        assert!(matches!(
            plan.collaboration,
            AgentFacetIntent::Restore { .. }
        ));
        for invalid in [
            codex("other"),
            json!({"agents":[{"agent_id":"agent_codex_default","context_id":"profile"}]}),
            json!({"agents":[{"agent_id":"agent_codex_default","context_id":"root","codex_access":{"root_context_id":"root","profile_context_id":"root"}}]}),
        ] {
            assert!(super::contexts(&invalid).is_err());
        }
    }
}
