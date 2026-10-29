//! Home receives recorded locator hints, never complete settings or execution authority.
use super::*;
use crate::authority::HostAuthority;
use std::collections::{BTreeMap, BTreeSet};

fn association_selection(
    status: AgentModelSettingsStatusV2,
    context_id: &str,
) -> Result<Option<AgentModelSelectionV2>, ()> {
    if status.context_id != context_id || status.schema != "hiroute.agent-model-settings-status/v2"
    {
        return Err(());
    }
    match (status.state, status.current_selection) {
        (AgentModelSettingsStateV2::NotConfigured | AgentModelSettingsStateV2::Restored, None) => {
            Ok(None)
        }
        (AgentModelSettingsStateV2::Configured, Some(selection))
            if selection.validate().is_ok() =>
        {
            Ok(Some(selection))
        }
        _ => Err(()),
    }
}

pub async fn agent_home_summary(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<AgentHomeSummary, DesktopFailure> {
    main_window(window)?;
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let choices = crate::session::read_trial_agent_choices(&read.client).await?;
    let contexts: Vec<String> = choices
        .iter()
        .filter_map(|choice| choice.context_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let request = AgentSettingsSummaryRequestV1 {
        context_ids: contexts.clone(),
    };
    if !request.valid() {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let result: AgentSettingsSummaryV1 =
        crate::session::query(&read.client, AGENT_SETTINGS_SUMMARY_OPERATION_V1, &request).await?;
    let mut by_context = BTreeMap::new();
    for entry in result.entries {
        if !contexts.contains(&entry.context_id)
            || !entry.model.valid()
            || !entry.collaboration.valid()
            || !entry.management_skill.valid()
            || by_context.insert(entry.context_id.clone(), entry).is_some()
        {
            return Err("AGENT_STATUS_UNAVAILABLE".into());
        }
    }
    if by_context.len() != contexts.len() {
        return Err("AGENT_STATUS_UNAVAILABLE".into());
    }
    let agents = choices
        .into_iter()
        .map(|choice| AgentHomeEntry {
            agent_id: choice.agent_id,
            configuration_state: choice.configuration_state,
            facets: choice
                .context_id
                .as_ref()
                .and_then(|id| by_context.get(id).cloned()),
            context_id: choice.context_id,
        })
        .collect();
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    Ok(AgentHomeSummary {
        agents,
        trusted_authority: !matches!(
            read.authority.native_connection(),
            Some(
                crate::NativeConnectionStatus::RecoveryRequired
                    | crate::NativeConnectionStatus::RestartRequired
                    | crate::NativeConnectionStatus::NotOwned
            )
        ),
    })
}

/// The visible routing editor needs only model associations. Missing or failed
/// contexts keep the result explicitly partial and cannot imply "unused".
pub async fn agent_model_associations(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<AgentModelAssociations, DesktopFailure> {
    main_window(window)?;
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let choices = crate::session::read_trial_agent_choices(&read.client).await?;
    let mut complete = true;
    let mut agents = Vec::new();
    for choice in choices {
        let Some(context_id) = choice.context_id else {
            complete = false;
            continue;
        };
        let status: Result<AgentModelSettingsStatusV2, DesktopFailure> = crate::session::query(
            &read.client,
            "GetAgentConnectionStatus",
            &AgentSettingsStatusRequestV2 {
                projection: AgentSettingsStatusProjectionV2::Model,
                effective: None,
                include_skills: false,
                native_target: choice.native_target,
                schema_version: AGENT_SETTINGS_SCHEMA_V2,
                context_id: context_id.clone(),
            },
        )
        .await;
        match status
            .ok()
            .and_then(|status| association_selection(status, &context_id).ok())
        {
            Some(current_selection) => agents.push(AgentModelAssociation {
                agent_id: choice.agent_id,
                current_selection,
            }),
            None => complete = false,
        }
    }
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    Ok(AgentModelAssociations { agents, complete })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(state: &str) -> AgentModelSettingsStatusV2 {
        serde_json::from_value(json!({
            "schema": "hiroute.agent-model-settings-status/v2", "context_id": "context/a",
            "state": state, "operation_id": null, "operation_state": null,
            "restore_point_ref": null, "applied_revision": null,
            "surface_results": [], "model_verified": false
        }))
        .unwrap()
    }

    #[test]
    fn unresolved_or_inconsistent_model_status_never_proves_no_association() {
        for state in ["pending", "needs_attention", "drift", "configured"] {
            assert!(
                association_selection(status(state), "context/a").is_err(),
                "{state}"
            );
        }
        for state in ["not_configured", "restored"] {
            assert_eq!(association_selection(status(state), "context/a"), Ok(None));
            assert!(association_selection(status(state), "context/other").is_err());
        }
    }

    #[test]
    fn home_hints_reject_contradictory_coverage_and_native_claims() {
        let mut facet = AgentSettingsFacetSummaryV1 {
            state: AgentSettingsSummaryStateV1::Absent,
            coverage: AgentSettingsSummaryCoverageV1::Complete,
            action: None,
            native_checked: false,
        };
        assert!(facet.valid());
        facet.coverage = AgentSettingsSummaryCoverageV1::NotReady;
        assert!(!facet.valid());
        facet.state = AgentSettingsSummaryStateV1::NotReady;
        assert!(facet.valid());
        facet.native_checked = true;
        assert!(!facet.valid());
        facet.native_checked = false;
        facet.state = AgentSettingsSummaryStateV1::Recorded;
        facet.coverage = AgentSettingsSummaryCoverageV1::Complete;
        assert!(!facet.valid());
        facet.action = Some(AgentSettingsRecordedActionV1::Configure);
        assert!(facet.valid());
    }
}
