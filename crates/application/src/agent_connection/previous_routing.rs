//! The retained settings chain supplies evidence; it never grants mutation authority.
use super::*;
use crate::control::{AgentConnectionControlPort, ControlReadError, ControlStatePort};
use hiroute_application_api::{
    AgentPreviousRoutingPreviewV1 as Preview, AgentPreviousRoutingQueryV1,
};
use hiroute_domain::*;
use std::collections::{BTreeMap, BTreeSet};

const MAX_HISTORY: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct PreviousRoutingHead {
    pub context_id: String,
    pub operation: Option<OperationId>,
}

struct Node {
    id: OperationId,
    previous: Option<OperationId>,
    settings: AgentModelSelectionV2,
    resolved: BTreeMap<String, serde_json::Value>,
    spec: AgentSettingsSpecV2,
}

fn node(operation: &OperationV1, context: &str, agent: &str) -> Result<Node, &'static str> {
    if operation.state != OperationState::Succeeded {
        return Err("history_not_succeeded");
    }
    let input = operation.agent_input();
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(input.spec.desired_state.clone()).map_err(|_| "history_invalid")?;
    if spec.context_id != context {
        return Err("context_mismatch");
    }
    let AgentFacetIntent::Configure { settings } = &spec.model else {
        return Err("history_not_configuration");
    };
    let mut intents = input
        .external
        .iter()
        .filter(|i| i.effect_id() == "agent-connection-managed-configuration");
    let intent = intents.next().ok_or("history_missing")?;
    if intents.next().is_some() || intent.desired()["subject"]["agent_id"] != agent {
        return Err("context_mismatch");
    }
    let mut resolved = BTreeMap::new();
    let previous = match settings {
        AgentModelSelectionV2::ClaudeLauncher { .. } => {
            let payload = settings_claude_model_file_for_operation(operation, intent)
                .map_err(|_| "history_invalid")?;
            let ClaudeModelFileAction::Configure {
                previous_operation,
                snapshot,
                ..
            } = payload.change
            else {
                return Err("history_not_configuration");
            };
            resolved.insert("opus".into(), serde_json::json!(snapshot.presets.opus));
            resolved.insert("sonnet".into(), serde_json::json!(snapshot.presets.sonnet));
            resolved.insert("haiku".into(), serde_json::json!(snapshot.presets.haiku));
            previous_operation
        }
        AgentModelSelectionV2::CodexDefault { .. } => {
            let payload = settings_codex_model_file_for_operation(operation, intent)
                .map_err(|_| "history_invalid")?;
            let CodexModelFileAction::Configure {
                previous_operation,
                model,
                ..
            } = payload.change
            else {
                return Err("history_not_configuration");
            };
            resolved.insert("default".into(), serde_json::json!(model));
            previous_operation
        }
        _ => {
            let payload = settings_additional_model_file_for_operation(operation, intent)
                .map_err(|_| "history_invalid")?;
            let AdditionalModelFileAction::Configure {
                previous_operation,
                models,
                ..
            } = payload.change
            else {
                return Err("history_not_configuration");
            };
            resolved.insert("models".into(), serde_json::json!(models));
            previous_operation
        }
    };
    Ok(Node {
        id: operation.operation_id.clone(),
        previous,
        settings: settings.clone(),
        resolved,
        spec,
    })
}

fn unavailable(reason: &str) -> Preview {
    Preview::Unavailable {
        reason: reason.into(),
        restore_is_separate: true,
    }
}

/// Skip unchanged mappings independently, so a Sonnet-only edit cannot hide Opus history.
fn recover(
    current: Node,
    selected: &BTreeSet<String>,
    mut read: impl FnMut(&OperationId) -> Result<Node, &'static str>,
) -> Preview {
    let current_operation = current.id.clone();
    let mut spec = current.spec.clone();
    let mut settings = current.settings.clone();
    let mut restored_from = BTreeMap::new();
    let mut visited = BTreeSet::new();
    visited.insert(current.id.clone());
    let mut latest = current;
    for _ in 0..MAX_HISTORY {
        let Some(previous) = latest.previous.as_ref() else {
            return unavailable("no_previous_configuration");
        };
        if !visited.insert(previous.clone()) {
            return unavailable("history_cycle");
        }
        let older = match read(previous) {
            Ok(node) => node,
            Err(reason) => return unavailable(reason),
        };
        for preset in selected {
            if !restored_from.contains_key(preset)
                && latest.resolved.get(preset) != older.resolved.get(preset)
            {
                if overlay(&mut settings, &older.settings, preset).is_err() {
                    return unavailable("history_mapping_incompatible");
                }
                restored_from.insert(preset.clone(), older.id.clone());
            }
        }
        if restored_from.len() == selected.len() {
            spec.model = AgentFacetIntent::Configure { settings };
            spec.collaboration = AgentFacetIntent::Keep;
            spec.access_token = AgentAccessTokenIntentV1::Keep;
            spec.restore_native_model = None;
            return Preview::Available {
                spec: Box::new(spec),
                current_operation,
                restored_from,
            };
        }
        latest = older;
    }
    unavailable("history_bound_reached")
}

fn overlay(
    current: &mut AgentModelSelectionV2,
    previous: &AgentModelSelectionV2,
    preset: &str,
) -> Result<(), ()> {
    match (current, previous) {
        (
            AgentModelSelectionV2::ClaudeLauncher {
                preset_mappings: a, ..
            },
            AgentModelSelectionV2::ClaudeLauncher {
                preset_mappings: b, ..
            },
        ) => match preset {
            "opus" => a.opus = b.opus.clone(),
            "sonnet" => a.sonnet = b.sonnet.clone(),
            "haiku" => a.haiku = b.haiku.clone(),
            _ => return Err(()),
        },
        (
            AgentModelSelectionV2::CodexDefault {
                default_selection: a,
                allowed_plan_ids,
                fixed_models,
                ..
            },
            AgentModelSelectionV2::CodexDefault {
                default_selection: b,
                fixed_models: previous_fixed,
                ..
            },
        ) if preset == "default" => {
            match b {
                AgentModelDefaultSelectionV2::Plan { plan_id } => {
                    allowed_plan_ids.insert(plan_id.clone());
                }
                AgentModelDefaultSelectionV2::FixedModel { client_model_id } => {
                    let previous = previous_fixed
                        .iter()
                        .find(|m| &m.client_model_id == client_model_id)
                        .ok_or(())?;
                    fixed_models.retain(|m| &m.client_model_id != client_model_id);
                    fixed_models.push(previous.clone());
                }
                AgentModelDefaultSelectionV2::PreserveNative => {}
            }
            *a = b.clone();
        }
        (
            a @ (AgentModelSelectionV2::QoderAdditional { .. }
            | AgentModelSelectionV2::PiAdditional { .. }
            | AgentModelSelectionV2::DshAdditional { .. }
            | AgentModelSelectionV2::OpenCodeAdditional { .. }),
            b,
        ) if preset == "models" && std::mem::discriminant(a) == std::mem::discriminant(b) => {
            *a = b.clone();
        }
        _ => return Err(()),
    }
    Ok(())
}

pub fn preview_previous_routing(
    connection: &dyn AgentConnectionControlPort,
    state: &dyn ControlStatePort,
    query: &AgentPreviousRoutingQueryV1,
) -> Result<Preview, ControlReadError> {
    let head = connection.previous_routing_head(&query.agent)?;
    let Some(id) = &head.operation else {
        return Ok(unavailable("no_previous_configuration"));
    };
    let read = |id: &OperationId| {
        let operation = state
            .operation(id)
            .map_err(|_| "history_unavailable")?
            .ok_or("history_missing")?;
        node(&operation, &head.context_id, &query.agent)
    };
    let current = match read(id) {
        Ok(node) => node,
        Err(reason) => return Ok(unavailable(reason)),
    };
    let selected: BTreeSet<String> = if query.presets.is_empty() {
        current.resolved.keys().cloned().collect()
    } else {
        query.presets.iter().cloned().collect()
    };
    if selected.is_empty()
        || selected
            .iter()
            .any(|key| !current.resolved.contains_key(key))
        || selected.len() > 3
    {
        return Err(ControlReadError::Denied);
    }
    let result = recover(current, &selected, read);
    if connection.previous_routing_head(&query.agent)? != head {
        return Err(ControlReadError::SnapshotChanged);
    }
    Ok(result)
}

/// Recovery is offered only for currently mapped presets with a complete retained chain.
/// This supplies navigation hints, never mutation authority; Preview rechecks the chain.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PreviousRoutingTargetV1 {
    pub agent: String,
    pub presets: Vec<String>,
}

pub fn previous_routing_targets(
    connection: &dyn AgentConnectionControlPort,
    state: &dyn ControlStatePort,
    plan_id: &str,
) -> Result<Vec<PreviousRoutingTargetV1>, ControlReadError> {
    let mut targets = Vec::new();
    for agent in [
        "agent_claude_default",
        "agent_codex_default",
        "agent_dsh_default",
        "agent_dsh_desktop",
        "agent_pi_default",
        "agent_qoder_default",
        "agent_opencode_default",
    ] {
        let head = match connection.previous_routing_head(agent) {
            Ok(head) => head,
            Err(ControlReadError::NotFound | ControlReadError::Unavailable) => continue,
            Err(error) => return Err(error),
        };
        let Some(operation_id) = &head.operation else {
            continue;
        };
        let Some(operation) = state.operation(operation_id)? else {
            continue;
        };
        let Ok(current) = node(&operation, &head.context_id, agent) else {
            continue;
        };
        let presets = mapped_presets(&current.settings, plan_id);
        for preset in presets {
            let query = AgentPreviousRoutingQueryV1 {
                agent: agent.into(),
                presets: vec![preset.clone()],
            };
            if matches!(
                preview_previous_routing(connection, state, &query)?,
                Preview::Available { .. }
            ) {
                targets.push(PreviousRoutingTargetV1 {
                    agent: agent.into(),
                    presets: vec![preset],
                });
            }
        }
        if connection.previous_routing_head(agent)? != head {
            return Err(ControlReadError::SnapshotChanged);
        }
    }
    Ok(targets)
}

fn mapped_presets(settings: &AgentModelSelectionV2, plan: &str) -> Vec<String> {
    match settings {
        AgentModelSelectionV2::ClaudeLauncher {
            preset_mappings, ..
        } => [
            ("opus", &preset_mappings.opus),
            ("sonnet", &preset_mappings.sonnet),
            ("haiku", &preset_mappings.haiku),
        ]
        .into_iter()
        .filter_map(|(preset, selection)| match selection {
            AgentClaudePresetSelectionV2::Plan { plan_id } if plan_id.as_str() == plan => {
                Some(preset.into())
            }
            _ => None,
        })
        .collect(),
        AgentModelSelectionV2::CodexDefault {
            default_selection: AgentModelDefaultSelectionV2::Plan { plan_id },
            ..
        } if plan_id.as_str() == plan => vec!["default".into()],
        AgentModelSelectionV2::DshAdditional {
            allowed_plan_ids, ..
        }
        | AgentModelSelectionV2::PiAdditional {
            allowed_plan_ids, ..
        }
        | AgentModelSelectionV2::QoderAdditional {
            allowed_plan_ids, ..
        }
        | AgentModelSelectionV2::OpenCodeAdditional {
            allowed_plan_ids, ..
        } if allowed_plan_ids.iter().any(|id| id.as_str() == plan) => vec!["models".into()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
