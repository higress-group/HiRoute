//! Rehydrate explicit native targets from accepted settings, including unfinished tails.
use super::*;

impl ControlStore {
    pub fn agent_settings_native_targets(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Vec<hiroute_domain::AgentNativeTargetV1>> {
        let connection = self.connection.borrow();
        // Membership, successful proofs and unfinished tails belong to the same
        // read snapshot. A caller transaction already supplies that snapshot.
        let transaction = if connection.is_autocommit() {
            Some(connection.unchecked_transaction().map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.native-targets.snapshot",
                )
            })?)
        } else {
            None
        };
        let ids = {
            let mut statement=connection.prepare(
                "SELECT operation_id,state FROM operations WHERE workspace_id=?1 AND operation_kind IN ('ApplyAgentConnectionChange','ApplyAgentConnectionRestore') ORDER BY rowid DESC"
            ).map_err(|_|port(PortErrorCode::Unavailable,"control.native-targets.query"))?;
            let rows = statement
                .query_map([workspace.as_str()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|_| port(PortErrorCode::Unavailable, "control.native-targets.query"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.rows"))?
        };
        let succeeded: std::collections::BTreeMap<_, _> = self
            .verified_agent_history_for_kinds(
                workspace,
                &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
            )?
            .into_iter()
            .map(|operation| (operation.operation_id.to_string(), operation))
            .collect();
        let mut targets = std::collections::BTreeMap::new();
        for (raw, state) in ids {
            let id = OperationId::parse(raw)
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.identity"))?;
            let tail;
            let plan = if state == "succeeded" {
                succeeded
                    .get(id.as_str())
                    .ok_or_else(|| {
                        port(PortErrorCode::Corrupt, "control.native-targets.operation")
                    })?
                    .plan
                    .spec()
            } else {
                // Pending/failed/restored operations remain visible to discovery;
                // recovery continues to use its original strict journal reader.
                tail = self.load_operation(&id)?.ok_or_else(|| {
                    port(PortErrorCode::Corrupt, "control.native-targets.operation")
                })?;
                tail.plan.spec()
            };
            if plan.command_id != "agents.settings.apply" {
                continue;
            }
            let spec: hiroute_domain::AgentSettingsSpecV2 =
                serde_json::from_value(plan.desired_state.clone())
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
                    || plan.resource_id.as_deref() != Some(expected.as_str())
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
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|_| {
                port(
                    PortErrorCode::Unavailable,
                    "control.native-targets.snapshot",
                )
            })?;
        }
        Ok(targets.into_values().collect())
    }
}
