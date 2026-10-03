//! Frozen pre-file-first revoke ordering. Uses production ports and stores to create the
//! historical service-complete, file-unstaged checkpoint; never used by current producers.
use super::*;
use hiroute_application::control::ApplicationMutationPort;
use hiroute_application::{
    PreparedTransactionV1, TransactionCoordinator, TransactionError, VerifiedPrincipal,
};
use hiroute_domain::{
    AgentAccessGrantMutationKindV1, ControlRepositoryPort, EffectReconciliation,
    ExternalEffectPort, OperationStepKind, OperationStepStatus, SecretStorePort,
    SettingsServiceCompletionV1,
};

pub(super) struct ServiceFirstRevoke(pub Arc<LocalControlAdapter>);

impl ApplicationMutationPort for ServiceFirstRevoke {
    fn preview_change(
        &self,
        r: api::PreviewRequestV1,
    ) -> Result<api::PreviewResultV1, TransactionError> {
        self.0.preview_change(r)
    }
    fn apply_change(
        &self,
        p: api::PrincipalKind,
        r: api::ApplyRequestV1,
    ) -> Result<OperationV1, TransactionError> {
        self.0.apply_change(p, r)
    }
    fn apply_local_change(&self, r: api::ApplyRequestV1) -> Result<OperationV1, TransactionError> {
        self.0.apply_local_change(r)
    }
    fn apply_prepared_change(
        &self,
        p: api::PrincipalKind,
        r: PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        self.0.apply_prepared_change(p, r)
    }
    fn apply_local_prepared_change(
        &self,
        prepared: PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        let a = self.0.as_ref();
        let coordinator = TransactionCoordinator::new(a, a, a, a, a, &a.admission);
        let accepted = coordinator.accept_prepared(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            prepared,
        )?;
        let mut op = accepted.operation().clone();
        assert_eq!(op.plan.spec().command_id, "agents.settings.apply");
        assert!(op.plan.secrets().is_empty() && op.plan.runtime().is_empty());
        assert_eq!(op.plan.agent_access_grants().len(), 1);
        assert_eq!(
            op.plan.agent_access_grants()[0].kind(),
            AgentAccessGrantMutationKindV1::Revoke
        );
        assert!(op.plan.external().iter().all(|i| {
            matches!(
                i.kind(),
                OwnedEffectKind::AgentArtifact | OwnedEffectKind::Publication
            )
        }));
        for kind in OperationStepKind::ALL {
            op.transition(kind.state()).unwrap();
            op.step_mut(kind).status = OperationStepStatus::Started;
            op.step_mut(kind).attempts += 1;
            a.save_operation(&mut op)?;
            match kind {
                OperationStepKind::Prepare => {}
                OperationStepKind::ApplySecrets => {
                    let mutation = op.plan.agent_access_grants()[0].clone();
                    let effect = a.apply_agent_access_grant(&op.operation_id, &mutation, None)?;
                    record(a, &mut op, kind, effect)?;
                }
                OperationStepKind::MaterializeSources => {
                    let effect = a.apply_control(
                        &op.operation_id,
                        &op.workspace_id,
                        op.expected_revisions.target,
                        op.plan.control(),
                    )?;
                    record(a, &mut op, kind, effect)?;
                }
                OperationStepKind::CompilePublication | OperationStepKind::ApplyAgentArtifacts => {
                    // Old revokes deferred the managed model file until after sealing service.
                    let effect_kind = if kind == OperationStepKind::CompilePublication {
                        OwnedEffectKind::Publication
                    } else {
                        OwnedEffectKind::AgentArtifact
                    };
                    for intent in op
                        .plan
                        .external()
                        .iter()
                        .filter(|i| {
                            i.kind() == effect_kind
                                && !hiroute_domain::is_settings_managed_configuration(i)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                    {
                        let effect = a.apply_external(&op, &intent)?;
                        record(a, &mut op, kind, effect)?;
                    }
                }
                OperationStepKind::Activate => {
                    a.begin_publication_activation(&op)?;
                    let effect = op.step(OperationStepKind::MaterializeSources).effects[0].clone();
                    let effect = a.activate_control(&effect)?;
                    record(a, &mut op, OperationStepKind::MaterializeSources, effect)?;
                    for intent in op
                        .plan
                        .external()
                        .iter()
                        .filter(|i| {
                            i.kind() == OwnedEffectKind::AgentArtifact
                                && !hiroute_domain::is_settings_managed_configuration(i)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                    {
                        a.prepare_agent_artifact_activation(&op, &intent)?;
                        let EffectReconciliation::Staged(effect) =
                            a.observe_external(&op, &intent)?
                        else {
                            panic!("legacy fixture artifact must be staged");
                        };
                        let effect = a.activate_external(&op, &effect)?;
                        record(a, &mut op, OperationStepKind::ApplyAgentArtifacts, effect)?;
                    }
                    let staged = op.step(OperationStepKind::CompilePublication).effects[0].clone();
                    let checkpoint = a.prepare_publication_activation(&op, &staged)?;
                    record(
                        a,
                        &mut op,
                        OperationStepKind::CompilePublication,
                        checkpoint.clone(),
                    )?;
                    let published = a.activate_external(&op, &checkpoint)?;
                    record(
                        a,
                        &mut op,
                        OperationStepKind::CompilePublication,
                        published.clone(),
                    )?;
                    let grant = op.step(OperationStepKind::ApplySecrets).effects[0].clone();
                    let revoked = a.activate_agent_access_grant(&grant)?;
                    record(a, &mut op, OperationStepKind::ApplySecrets, revoked)?;
                    let revision = published.compensation["publication_revision"]
                        .as_u64()
                        .unwrap();
                    let digest = published.after_fingerprint.unwrap();
                    let receipt = SettingsServiceCompletionV1 {
                        schema: hiroute_domain::SETTINGS_SERVICE_COMPLETION_SCHEMA.into(),
                        publication_revision: revision,
                        publication_digest: digest.clone(),
                        // Exact frozen v2 receipt algorithm, including the immutable admitted plan.
                        completed_effects_digest: CanonicalDigest::of(&(
                            "hiroute.settings-service-proof/v2",
                            op.operation_id.as_str(),
                            &op.accepted_digest,
                            revision,
                            &digest,
                            op.stable_input_digest().unwrap(),
                        ))
                        .unwrap(),
                    };
                    op.step_mut(kind).terminal_result =
                        Some(serde_json::to_string(&receipt).unwrap());
                    op.safe_error_code = Some("SETTINGS_TAIL_PENDING".into());
                    a.save_operation(&mut op)?;
                    a.save_operation_tail(&mut op)?;
                    assert!(hiroute_application::settings_service_completion_is_current(
                        &op, revision, &digest
                    ));
                    return Ok(op);
                }
            }
            op.step_mut(kind).status = OperationStepStatus::Applied;
            op.step_mut(kind).terminal_result = Some("applied".into());
            a.save_operation(&mut op)?;
        }
        unreachable!()
    }
}

fn record(
    a: &LocalControlAdapter,
    op: &mut OperationV1,
    step: OperationStepKind,
    effect: hiroute_domain::OwnedEffectV1,
) -> hiroute_domain::PortResult<()> {
    let effects = &mut op.step_mut(step).effects;
    if let Some(old) = effects
        .iter_mut()
        .find(|old| old.effect_id == effect.effect_id)
    {
        *old = effect;
    } else {
        effects.push(effect);
    }
    a.save_operation(op)
}
