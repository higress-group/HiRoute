use super::*;

impl<'a, C, S, R, E, I> TransactionCoordinator<'a, C, S, R, E, I>
where
    C: ControlRepositoryPort
        + ComputeSourceControlPort
        + CredentialPoolControlPort
        + ConnectionOptionAuthorizationPort,
    S: SecretStorePort,
    R: RuntimeStatePort,
    E: ExternalEffectPort,
    I: ProtectedInputPort,
{
    pub(super) fn run_step(
        &self,
        operation: &mut OperationV1,
        kind: OperationStepKind,
    ) -> Result<(), TransactionError> {
        if operation.state != kind.state() {
            operation.transition(kind.state())?;
        }
        {
            let step = operation.step_mut(kind);
            step.status = OperationStepStatus::Started;
            step.attempts = step.attempts.saturating_add(1);
        }
        self.control.save_operation(operation)?;

        match kind {
            OperationStepKind::Prepare => {}
            OperationStepKind::ApplySecrets => {
                for mutation in operation.plan.secrets().to_vec() {
                    let effect = match self
                        .secrets
                        .observe_secret(&operation.operation_id, &mutation)?
                    {
                        EffectReconciliation::Staged(effect)
                        | EffectReconciliation::Applied(effect) => effect,
                        EffectReconciliation::Missing => {
                            let input = if mutation.kind() == SecretMutationKind::Upsert {
                                let slot = mutation
                                    .input_slot()
                                    .ok_or(TransactionError::ProtectedInputUnavailable)?;
                                let input =
                                    self.protected_inputs.read_secret(slot).map_err(|error| {
                                        if error.code == hiroute_domain::PortErrorCode::NotFound {
                                            TransactionError::ProtectedInputUnavailable
                                        } else {
                                            TransactionError::Port(error)
                                        }
                                    })?;
                                let observed = self.secrets.fingerprint(&input)?;
                                if mutation.fingerprint() != Some(&observed) {
                                    return Err(TransactionError::ChangePreviewStale);
                                }
                                Some(input)
                            } else {
                                None
                            };
                            self.secrets.apply_secret(
                                &operation.operation_id,
                                &mutation,
                                input.as_ref(),
                            )?
                        }
                        EffectReconciliation::OwnershipLost(effect) => {
                            self.record_effect(operation, kind, effect)?;
                            return Err(TransactionError::EffectOwnershipLost);
                        }
                    };
                    self.record_effect(operation, kind, effect)?;
                }
                self.stage_agent_access_grants(operation)?;
            }
            OperationStepKind::MaterializeSources => {
                let effect = match self
                    .control
                    .observe_control(&operation.operation_id, &operation.workspace_id)?
                {
                    EffectReconciliation::Staged(effect)
                    | EffectReconciliation::Applied(effect) => effect,
                    EffectReconciliation::Missing => {
                        if let Some(selection) = operation.plan.worker_dependency_selection() {
                            self.control.apply_worker_dependency_selection(
                                &operation.operation_id,
                                &operation.workspace_id,
                                selection,
                            )?
                        } else if let Some(source) = operation.plan.compute_source() {
                            self.control.apply_compute_source(
                                &operation.operation_id,
                                &operation.workspace_id,
                                operation.expected_revisions.target,
                                &source,
                            )?
                        } else if let Some(pool) = operation.plan.credential_pool() {
                            self.control.apply_credential_pool(
                                &operation.operation_id,
                                &operation.workspace_id,
                                operation.expected_revisions.target,
                                pool,
                            )?
                        } else {
                            self.control.apply_control(
                                &operation.operation_id,
                                &operation.workspace_id,
                                operation.expected_revisions.target,
                                operation.plan.control(),
                            )?
                        }
                    }
                    EffectReconciliation::OwnershipLost(effect) => {
                        self.record_effect(operation, kind, effect)?;
                        return Err(TransactionError::EffectOwnershipLost);
                    }
                };
                self.record_effect(operation, kind, effect)?;
            }
            OperationStepKind::CompilePublication => {
                // The host registered the login item under native confirmation before this
                // Operation was sealed; journal that evidence before any publication effect
                // can point a client at the resident service.
                self.run_external_kind(operation, kind, OwnedEffectKind::LoginItem)?;
                self.run_external_kind(operation, kind, OwnedEffectKind::Publication)?;
            }
            OperationStepKind::ApplyAgentArtifacts => {
                self.run_external_kind(operation, kind, OwnedEffectKind::AgentArtifact)?;
            }
            OperationStepKind::Activate => {
                // Runtime values are staged and then every exact-owned effect is published in
                // the single serialized activation boundary.
                for mutation in operation.plan.runtime().to_vec() {
                    let effect = match self
                        .runtime
                        .observe_runtime(&operation.operation_id, &mutation)?
                    {
                        EffectReconciliation::Staged(effect)
                        | EffectReconciliation::Applied(effect) => effect,
                        EffectReconciliation::Missing => self
                            .runtime
                            .apply_runtime(&operation.operation_id, &mutation)?,
                        EffectReconciliation::OwnershipLost(effect) => {
                            self.record_effect(operation, kind, effect)?;
                            return Err(TransactionError::EffectOwnershipLost);
                        }
                    };
                    self.record_effect(operation, kind, effect)?;
                }
                self.activate_exact_effects(operation)?;
            }
        }

        let step = operation.step_mut(kind);
        step.status = OperationStepStatus::Applied;
        if hiroute_domain::SettingsServiceCompletionV1::parse(
            step.terminal_result.as_deref().unwrap_or_default(),
        )
        .is_none()
        {
            step.terminal_result = Some("applied".to_owned());
        }
        self.control.save_operation(operation)?;
        Ok(())
    }

    fn run_external_kind(
        &self,
        operation: &mut OperationV1,
        step: OperationStepKind,
        effect_kind: OwnedEffectKind,
    ) -> Result<(), TransactionError> {
        let intents = operation
            .plan
            .external()
            .iter()
            .filter(|intent| intent.kind() == effect_kind)
            .cloned()
            .collect::<Vec<_>>();
        for intent in intents {
            let effect = match self.external.observe_external(operation, &intent)? {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    effect
                }
                EffectReconciliation::Missing => {
                    self.external.apply_external(operation, &intent)?
                }
                EffectReconciliation::OwnershipLost(effect) => {
                    self.record_effect(operation, step, effect)?;
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, step, effect)?;
        }
        Ok(())
    }

    fn activate_exact_effects(&self, operation: &mut OperationV1) -> Result<(), TransactionError> {
        self.external.begin_publication_activation(operation)?;
        for mutation in operation.plan.secrets().to_vec() {
            let effect = match self
                .secrets
                .observe_secret(&operation.operation_id, &mutation)?
            {
                EffectReconciliation::Staged(effect) => self.secrets.activate_secret(&effect)?,
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
        }
        let revokes_agent_access_grant = operation
            .plan
            .agent_access_grants()
            .iter()
            .any(|mutation| mutation.kind() == AgentAccessGrantMutationKindV1::Revoke);
        if !revokes_agent_access_grant {
            self.activate_agent_access_grants(operation)?;
        }

        let control = match self
            .control
            .observe_control(&operation.operation_id, &operation.workspace_id)?
        {
            EffectReconciliation::Staged(effect) => self.control.activate_control(&effect)?,
            EffectReconciliation::Applied(effect) => effect,
            EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
            EffectReconciliation::OwnershipLost(_) => {
                return Err(TransactionError::EffectOwnershipLost);
            }
        };
        self.record_effect(operation, OperationStepKind::MaterializeSources, control)?;

        // Agent artifacts must become durable before an executable publication can expose them.
        // The settings client model file is deliberately excluded here: it activates only after
        // the publication below is serving, so a client file never precedes its own service.
        for intent in operation
            .plan
            .external()
            .iter()
            .filter(|intent| {
                intent.kind() == OwnedEffectKind::AgentArtifact
                    && !hiroute_domain::is_settings_managed_configuration(intent)
            })
            .cloned()
            .collect::<Vec<_>>()
        {
            self.external
                .prepare_agent_artifact_activation(operation, &intent)?;
            let effect = match self.external.observe_external(operation, &intent)? {
                EffectReconciliation::Staged(effect) => {
                    self.external.activate_external(operation, &effect)?
                }
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::ApplyAgentArtifacts, effect)?;
        }

        for mutation in operation.plan.runtime().to_vec() {
            let effect = match self
                .runtime
                .observe_runtime(&operation.operation_id, &mutation)?
            {
                EffectReconciliation::Staged(effect) => self.runtime.activate_runtime(&effect)?,
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::Activate, effect)?;
        }

        for intent in operation
            .plan
            .external()
            .iter()
            .filter(|intent| intent.kind() == OwnedEffectKind::Publication)
            .cloned()
            .collect::<Vec<_>>()
        {
            let effect = match self.external.observe_external(operation, &intent)? {
                EffectReconciliation::Staged(effect) => {
                    let checkpoint = self
                        .external
                        .prepare_publication_activation(operation, &effect)?;
                    self.record_effect(
                        operation,
                        OperationStepKind::CompilePublication,
                        checkpoint.clone(),
                    )?;
                    self.external.activate_external(operation, &checkpoint)?
                }
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::CompilePublication, effect)?;
        }
        // A restore first removes the verifier from the active product/Gateway publication.
        // Only then may it revoke the protected material, so a still-active publication never
        // accepts a bearer whose lifecycle has already been destroyed.
        if revokes_agent_access_grant {
            self.activate_agent_access_grants(operation)?;
        }
        // The service segment is complete: seal the durable completion receipt before the only
        // effects that may follow it, then switch the client file.
        self.seal_settings_service_completion(operation)?;
        self.activate_settings_file_tail(operation)?;
        Ok(())
    }

    /// Activates the settings client model file effects, the sole segment that follows the
    /// sealed service receipt. A failure here must park the tail instead of rolling back.
    pub(super) fn activate_settings_file_tail(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        for intent in operation
            .plan
            .external()
            .iter()
            .filter(|intent| hiroute_domain::is_settings_managed_configuration(intent))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.external
                .prepare_agent_artifact_activation(operation, &intent)?;
            let effect = match self.external.observe_external(operation, &intent)? {
                EffectReconciliation::Staged(effect) => {
                    self.external.activate_external(operation, &effect)?
                }
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::ApplyAgentArtifacts, effect)?;
        }
        Ok(())
    }

    fn seal_settings_service_completion(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        let file_intents = operation
            .plan
            .external()
            .iter()
            .any(hiroute_domain::is_settings_managed_configuration);
        if !file_intents {
            return Ok(());
        }
        let Some((revision, digest)) = operation
            .step(OperationStepKind::CompilePublication)
            .effects
            .iter()
            .filter(|effect| effect.kind == OwnedEffectKind::Publication)
            .find_map(|effect| {
                Some((
                    effect.compensation["publication_revision"].as_u64()?,
                    effect.after_fingerprint.clone()?,
                ))
            })
        else {
            return Err(TransactionError::EffectMissing);
        };
        let receipt = hiroute_domain::SettingsServiceCompletionV1 {
            schema: hiroute_domain::SETTINGS_SERVICE_COMPLETION_SCHEMA.into(),
            publication_revision: revision,
            publication_digest: digest.clone(),
            completed_effects_digest: settings_service_completion_digest(
                operation, revision, &digest,
            )?,
        };
        operation
            .step_mut(OperationStepKind::Activate)
            .terminal_result =
            Some(serde_json::to_string(&receipt).map_err(|_| TransactionError::InvalidArguments)?);
        self.control.save_operation(operation)?;
        Ok(())
    }
}
