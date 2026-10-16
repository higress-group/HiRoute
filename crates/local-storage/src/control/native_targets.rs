//! Rehydrate explicit native targets from accepted settings, including unfinished tails.
use super::*;

impl ControlStore {
    pub fn agent_settings_native_targets(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Vec<hiroute_domain::AgentNativeTargetV1>> {
        let ids = {
            let connection = self.connection.borrow();
            let mut statement=connection.prepare(
                "SELECT operation_id FROM operations WHERE workspace_id=?1 AND operation_kind IN ('ApplyAgentConnectionChange','ApplyAgentConnectionRestore') ORDER BY rowid DESC"
            ).map_err(|_|port(PortErrorCode::Unavailable,"control.native-targets.query"))?;
            let rows = statement
                .query_map([workspace.as_str()], |r| r.get::<_, String>(0))
                .map_err(|_| port(PortErrorCode::Unavailable, "control.native-targets.query"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.rows"))?
        };
        let mut targets = std::collections::BTreeMap::new();
        for raw in ids {
            let id = OperationId::parse(raw)
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.identity"))?;
            let op = self
                .load_operation(&id)?
                .ok_or_else(|| port(PortErrorCode::Corrupt, "control.native-targets.operation"))?;
            if op.plan.spec().command_id != "agents.settings.apply" {
                continue;
            }
            let spec: hiroute_domain::AgentSettingsSpecV2 =
                serde_json::from_value(op.plan.spec().desired_state.clone())
                    .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.spec"))?;
            if let Some(target) = spec.native_target {
                target
                    .validate()
                    .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.scope"))?;
                let expected = format!(
                    "agent-context/hermes/{}",
                    CanonicalDigest::of_bytes(target.config_root().as_bytes()).as_str()
                );
                if spec.context_id != expected
                    || op.plan.spec().resource_id.as_deref() != Some(expected.as_str())
                {
                    return Err(port(
                        PortErrorCode::Corrupt,
                        "control.native-targets.context",
                    ));
                }
                targets
                    .entry(target.config_root().to_owned())
                    .or_insert(target);
            }
        }
        Ok(targets.into_values().collect())
    }
}
