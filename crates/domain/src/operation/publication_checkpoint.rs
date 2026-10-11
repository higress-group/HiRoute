//! Explicit re-publication of identical authority, including an empty workspace.
use super::*;
use crate::{GatewayPublicationRevision, PublicationRecordV1};

pub const PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1: &str =
    "hiroute.publication-checkpoint-change/v1";
const CONTROL_SCHEMA: &str = "hiroute.publication-checkpoint-control/v1";
const EFFECT_SCHEMA: &str = "hiroute.publication-checkpoint-effect/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationCheckpointChangeV1 {
    pub schema: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointControl {
    schema: String,
    change_spec_digest: CanonicalDigest,
    before_digest: CanonicalDigest,
    after_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointEffect {
    schema: String,
    change_spec: ChangeSpecV1,
    control: CheckpointControl,
    before: PublicationRecordV1,
    after: PublicationRecordV1,
}

pub fn checkpoint_publication(
    before: &PublicationRecordV1,
) -> Result<PublicationRecordV1, OperationValidationError> {
    let mut publication = before.verify_current().map_err(|_| invalid())?;
    publication.publication_revision = GatewayPublicationRevision::new(
        publication
            .publication_revision
            .get()
            .checked_add(1)
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    PublicationRecordV1::from_publication(before.workspace_id.clone(), &publication)
        .map_err(|_| invalid())
}

impl TransactionPlanV1 {
    pub fn from_publication_checkpoint(
        spec: ChangeSpecV1,
        before: PublicationRecordV1,
    ) -> Result<Self, OperationValidationError> {
        let after = checkpoint_publication(&before)?;
        let control = CheckpointControl {
            schema: CONTROL_SCHEMA.into(),
            change_spec_digest: CanonicalDigest::of(&spec)?,
            before_digest: before.digest.clone(),
            after_digest: after.digest.clone(),
        };
        let effect = CheckpointEffect {
            schema: EFFECT_SCHEMA.into(),
            change_spec: spec.clone(),
            control: control.clone(),
            before,
            after,
        };
        validate_effect(&effect)?;
        Ok(Self {
            spec,
            control: crate::canonicalize_json(serde_json::to_value(control)?),
            credential_pool: None,
            worker_dependency_selection: None,
            secrets: vec![],
            agent_access_grants: vec![],
            runtime: vec![],
            external: vec![ExternalEffectIntentV1 {
                effect_id: "routing-publication".into(),
                kind: OwnedEffectKind::Publication,
                target: "publication/current".into(),
                before_fingerprint: Some(effect.before.digest.clone()),
                desired: crate::canonicalize_json(serde_json::to_value(effect)?).into(),
                content_publication: None,
                desired_mode: 0o644,
                sensitive: false,
            }],
        })
    }
}

fn invalid() -> OperationValidationError {
    OperationValidationError::UnregisteredEffectPlan
}

fn validate_effect(effect: &CheckpointEffect) -> Result<(), OperationValidationError> {
    let change: PublicationCheckpointChangeV1 =
        serde_json::from_value(effect.change_spec.desired_state.clone()).map_err(|_| invalid())?;
    if effect.schema != EFFECT_SCHEMA
        || effect.control.schema != CONTROL_SCHEMA
        || change.schema != PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1
        || effect.change_spec.command_id != "routing.apply"
        || effect.change_spec.resource_id.as_deref() != Some("publication/current")
        || effect.control.change_spec_digest != CanonicalDigest::of(&effect.change_spec)?
        || effect.control.before_digest != effect.before.digest
        || effect.control.after_digest != effect.after.digest
        || checkpoint_publication(&effect.before)? != effect.after
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn is_control(value: &Value) -> bool {
    value.get("schema").and_then(Value::as_str) == Some(CONTROL_SCHEMA)
}
pub(super) fn is_effect(value: &Value) -> bool {
    value.get("schema").and_then(Value::as_str) == Some(EFFECT_SCHEMA)
}
pub(super) fn validate_external(
    id: &str,
    kind: OwnedEffectKind,
    target: &str,
    desired: &Value,
    mode: u32,
    sensitive: bool,
) -> Result<(), OperationValidationError> {
    if id != "routing-publication"
        || kind != OwnedEffectKind::Publication
        || target != "publication/current"
        || mode != 0o644
        || sensitive
    {
        return Err(invalid());
    }
    let effect: CheckpointEffect =
        serde_json::from_value(desired.clone()).map_err(|_| invalid())?;
    validate_effect(&effect)
}
pub(super) fn record(
    intent: &ExternalEffectIntentV1,
) -> Result<PublicationRecordV1, OperationValidationError> {
    validate_external(
        &intent.effect_id,
        intent.kind,
        &intent.target,
        &intent.desired,
        intent.desired_mode,
        intent.sensitive,
    )?;
    let effect: CheckpointEffect =
        serde_json::from_value(intent.desired.as_ref().clone()).map_err(|_| invalid())?;
    if intent.before_fingerprint.as_ref() != Some(&effect.before.digest) {
        return Err(invalid());
    }
    Ok(effect.after)
}
pub(super) fn validate_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if !secrets.is_empty() || !runtime.is_empty() || external.len() != 1 {
        return Err(invalid());
    }
    record(&external[0])?;
    let effect: CheckpointEffect =
        serde_json::from_value(external[0].desired.as_ref().clone()).map_err(|_| invalid())?;
    let control: CheckpointControl =
        serde_json::from_value(control.clone()).map_err(|_| invalid())?;
    if effect.change_spec != *spec || effect.control != control {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn before() -> PublicationRecordV1 {
        let publication = crate::GatewayPublicationV1::new(
            crate::WorkspaceId::default(),
            GatewayPublicationRevision::new(1).unwrap(),
            crate::AliasRegistryV1::default(),
            vec![],
        )
        .unwrap();
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap()
    }
    fn plan() -> TransactionPlanV1 {
        TransactionPlanV1::from_publication_checkpoint(ChangeSpecV1 {
            schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
            command_id: "routing.apply".into(), resource_id: Some("publication/current".into()),
            desired_state: serde_json::json!({"schema":PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1}),
        }, before()).unwrap()
    }
    #[test]
    fn publication_checkpoint_preserves_empty_authority_and_strictly_advances_once() {
        let plan = plan();
        let mut expected = before().verify_current().unwrap();
        expected.publication_revision = GatewayPublicationRevision::new(2).unwrap();
        assert_eq!(
            record(&plan.external[0]).unwrap().verify_current().unwrap(),
            expected
        );
        assert!(
            plan.secrets.is_empty()
                && plan.runtime.is_empty()
                && plan.agent_access_grants.is_empty()
        );
        validate_plan(&plan.spec, &plan.control, &[], &[], &plan.external).unwrap();
        let effect = &plan.external[0];
        let recovered = ExternalEffectIntentV1::from_registered_adapter(
            effect.effect_id.clone(),
            effect.kind,
            effect.target.clone(),
            effect.before_fingerprint.clone(),
            effect.desired.as_ref().clone(),
            effect.desired_mode,
            effect.sensitive,
        )
        .unwrap();
        assert_eq!(record(&recovered).unwrap(), record(effect).unwrap());
    }
    #[test]
    fn publication_checkpoint_preserves_nonempty_grants_disabled_heads_and_tombstones() {
        let original: crate::GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
            "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
        let mut publication = original.into_current().unwrap();
        assert!(!publication.plans.is_empty());
        assert!(!publication.grants.is_empty());
        publication.plan_heads = publication
            .plans
            .iter()
            .map(|compiled| {
                let version = crate::PlanVersionV1::from_unversioned_compiled_recovery(
                    publication.workspace_id.clone(),
                    compiled.clone(),
                )
                .unwrap();
                crate::PlanHeadV1 {
                    head_revision: version.reference.content_revision,
                    reference: version.reference,
                    model_alias: compiled.model_alias().clone(),
                    status: crate::PlanLifecycleV1::Enabled,
                }
            })
            .collect();
        let mut disabled = publication.plan_heads[0].clone();
        disabled.head_revision += 1;
        disabled.status = crate::PlanLifecycleV1::Disabled;
        publication = publication
            .next_with_plan_lifecycle(
                GatewayPublicationRevision::new(publication.publication_revision.get() + 1)
                    .unwrap(),
                disabled,
            )
            .unwrap();
        publication
            .alias_registry
            .tombstones
            .insert(crate::ModelAlias::parse("retired-checkpoint-fixture").unwrap());
        let record =
            PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
                .unwrap();
        let after = checkpoint_publication(&record)
            .unwrap()
            .verify_current()
            .unwrap();
        publication.publication_revision = after.publication_revision;
        assert_eq!(after, publication);
    }
    #[test]
    fn publication_checkpoint_rejects_changed_authority_and_forged_ownership() {
        for field in ["authority", "before", "revision", "spec", "control"] {
            let mut plan = plan();
            let mut effect: CheckpointEffect =
                serde_json::from_value(plan.external[0].desired.as_ref().clone()).unwrap();
            match field {
                "authority" => {
                    let mut publication = effect.after.verify_current().unwrap();
                    publication
                        .alias_registry
                        .tombstones
                        .insert(crate::ModelAlias::parse("retired-forged").unwrap());
                    effect.after = PublicationRecordV1::from_publication(
                        publication.workspace_id.clone(),
                        &publication,
                    )
                    .unwrap();
                    effect.control.after_digest = effect.after.digest.clone();
                }
                "before" => {
                    plan.external[0].before_fingerprint = Some(CanonicalDigest::of_bytes(b"wrong"))
                }
                "revision" => {
                    let mut publication = effect.after.verify_current().unwrap();
                    publication.publication_revision = GatewayPublicationRevision::new(3).unwrap();
                    effect.after = PublicationRecordV1::from_publication(
                        publication.workspace_id.clone(),
                        &publication,
                    )
                    .unwrap();
                    effect.control.after_digest = effect.after.digest.clone();
                }
                "spec" => effect.change_spec.desired_state["unexpected"] = serde_json::json!(true),
                _ => {
                    plan.control["before_digest"] =
                        serde_json::json!(CanonicalDigest::of_bytes(b"wrong"))
                }
            }
            plan.external[0].desired = serde_json::to_value(effect).unwrap().into();
            assert!(
                validate_plan(&plan.spec, &plan.control, &[], &[], &plan.external).is_err(),
                "{field}"
            );
        }
    }
}
