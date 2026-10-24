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
                "SELECT operation_id,state,generation FROM operations WHERE workspace_id=?1 AND operation_kind IN ('ApplyAgentConnectionChange','ApplyAgentConnectionRestore') ORDER BY rowid DESC"
            ).map_err(|_|port(PortErrorCode::Unavailable,"control.native-targets.query"))?;
            let rows = statement
                .query_map([workspace.as_str()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u64>(2)?,
                    ))
                })
                .map_err(|_| port(PortErrorCode::Unavailable, "control.native-targets.query"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.rows"))?
        };
        let succeeded: std::collections::BTreeMap<_, _> = self
            .verified_agent_history_for_discovery(
                workspace,
                &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
            )
            // This cache is optional for discovery. The immutable-input reader
            // below remains responsible for every row without a verified proof.
            .unwrap_or_default()
            .into_iter()
            .map(|operation| (operation.operation_id.to_string(), operation))
            .collect();
        let mut targets = std::collections::BTreeMap::new();
        for (raw, state, generation) in ids {
            let id = OperationId::parse(raw)
                .map_err(|_| port(PortErrorCode::Corrupt, "control.native-targets.identity"))?;
            let tail;
            let plan = if let Some(proven) = succeeded.get(id.as_str()) {
                proven.plan.spec()
            } else {
                // Unfinished tails and successful inputs lacking provenance
                // remain discovery hints, never verified execution authority.
                tail = self.load_operation(&id)?.ok_or_else(|| {
                    port(PortErrorCode::Corrupt, "control.native-targets.operation")
                })?;
                if tail.operation_id != id
                    || tail.workspace_id != *workspace
                    || tail.state.as_str() != state
                    || tail.generation != generation
                {
                    return Err(port(
                        PortErrorCode::Corrupt,
                        "control.native-targets.identity",
                    ));
                }
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
