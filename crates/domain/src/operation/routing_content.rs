//! V2 full-content intent carried by the existing Operation and publication effect.
use super::*;
use crate::{PlanHeadV1, PlanLifecycleV1, PlanVersionV1, PublicationRecordV1};

const CONTROL_SCHEMA: &str = "hiroute.routing-control/v2";
const EFFECT_SCHEMA: &str = "hiroute.routing-publication-effect/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumedPlanDraftV1 {
    pub draft_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanContentControlV2 {
    pub schema: String,
    pub change_spec_digest: CanonicalDigest,
    pub plan_version: PlanVersionV1,
    pub plan_head: PlanHeadV1,
    pub before_head: Option<PlanHeadV1>,
    /// Narrow adoption proof for an unversioned publication record. The name is retained in the
    /// V2 wire shape; the authenticated compiled payload may already have been currentized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_source: Option<PlanVersionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_draft: Option<ConsumedPlanDraftV1>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContentEffectV2 {
    schema: String,
    change_spec: ChangeSpecV1,
    control: PlanContentControlV2,
    publication_record: PublicationRecordV1,
}
impl TransactionPlanV1 {
    pub fn with_legacy_plan_source(
        mut self,
        version: PlanVersionV1,
    ) -> Result<Self, OperationValidationError> {
        let mut control = self
            .plan_content_control()?
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        control.legacy_source = Some(version);
        self.control = crate::canonicalize_json(serde_json::to_value(&control)?);
        let effect = self
            .external
            .get_mut(0)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        std::sync::Arc::make_mut(&mut effect.desired)["control"] = self.control.clone();
        effect.content_publication = Some(std::sync::Arc::new(decode_external(
            &effect.effect_id,
            effect.kind,
            &effect.target,
            &effect.desired,
            effect.desired_mode,
            effect.sensitive,
        )?));
        validate_plan(
            &self.spec,
            &self.control,
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        Ok(self)
    }

    pub fn plan_content_control(
        &self,
    ) -> Result<Option<PlanContentControlV2>, OperationValidationError> {
        if !is_control(&self.control) {
            return Ok(None);
        }
        // Private inputs, no Deserialize and no mutable control access: constructors and the
        // adoption mutation authenticate the complete joined content before it can be read.
        self.external
            .first()
            .and_then(|effect| effect.content_publication.as_ref())
            .map(|effect| Some(effect.control.clone()))
            .ok_or(OperationValidationError::UnregisteredEffectPlan)
    }

    pub fn from_plan_content_planner(
        spec: ChangeSpecV1,
        version: PlanVersionV1,
        head: PlanHeadV1,
        before_head: Option<PlanHeadV1>,
        consumed_draft: Option<ConsumedPlanDraftV1>,
        publication_record: PublicationRecordV1,
        before_publication_digest: Option<CanonicalDigest>,
    ) -> Result<Self, OperationValidationError> {
        let control = PlanContentControlV2 {
            schema: CONTROL_SCHEMA.into(),
            change_spec_digest: CanonicalDigest::of(&spec)?,
            plan_version: version,
            plan_head: head,
            before_head,
            legacy_source: None,
            consumed_draft,
        };
        validate_inputs(&spec, &control, &publication_record)?;
        if (publication_record.publication_revision.get() == 1)
            != before_publication_digest.is_none()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let effect = ContentEffectV2 {
            schema: EFFECT_SCHEMA.into(),
            change_spec: spec.clone(),
            control: control.clone(),
            publication_record,
        };
        let plan = Self {
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
                before_fingerprint: before_publication_digest,
                desired: crate::canonicalize_json(serde_json::to_value(&effect)?).into(),
                content_publication: Some(std::sync::Arc::new(effect)),
                desired_mode: 0o644,
                sensitive: false,
            }],
        };
        Ok(plan)
    }
}
fn validate_inputs(
    spec: &ChangeSpecV1,
    control: &PlanContentControlV2,
    record: &PublicationRecordV1,
) -> Result<(), OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    let version = &control.plan_version;
    let head = &control.plan_head;
    version.validate().map_err(|_| invalid())?;
    head.validate().map_err(|_| invalid())?;
    if spec.command_id != "routing.apply"
        || control.schema != CONTROL_SCHEMA
        || version.compiled.body.schema != crate::AGENT_PLAN_COMPILED_SCHEMA_V3
        || control.change_spec_digest != CanonicalDigest::of(spec)?
        || spec.resource_id.as_deref()
            != Some(&format!(
                "agent-plan/{}",
                version.reference.plan_id.as_str()
            ))
        || head.reference != version.reference
        || head.model_alias != *version.compiled.model_alias()
        || record.workspace_id != version.reference.workspace_id
    {
        return Err(invalid());
    }
    if spec.desired_state.get("schema").and_then(Value::as_str)
        == Some("hiroute.plan-lifecycle-change/v1")
    {
        return validate_lifecycle(spec, control, record);
    }
    if spec.desired_state.get("schema").and_then(Value::as_str)
        != Some("hiroute.plan-content-change/v2")
        || head.status == PlanLifecycleV1::Deleted
    {
        return Err(invalid());
    }
    if let Some(legacy) = &control.legacy_source {
        legacy.validate().map_err(|_| invalid())?;
        if control.before_head.as_ref().map(|h| &h.reference) != Some(&legacy.reference)
            || legacy.compiled.model_alias() != &head.model_alias
            || *legacy
                != PlanVersionV1::from_unversioned_compiled_recovery(
                    version.reference.workspace_id.clone(),
                    legacy.compiled.clone(),
                )
                .map_err(|_| invalid())?
        {
            return Err(invalid());
        }
    }
    let editor: crate::PlanEditorStateV2 = serde_json::from_value(
        spec.desired_state
            .get("editor")
            .cloned()
            .ok_or_else(invalid)?,
    )
    .map_err(|_| invalid())?;
    if editor.effective().map_err(|_| invalid())? != version.configuration {
        return Err(invalid());
    }
    if let Some(before) = &control.before_head {
        before.validate().map_err(|_| invalid())?;
        if before.reference.workspace_id != head.reference.workspace_id
            || before.reference.plan_id != head.reference.plan_id
            || before.model_alias != head.model_alias
            || before.status == PlanLifecycleV1::Deleted
            || before.status != head.status
            || before.head_revision.checked_add(1) != Some(head.head_revision)
            || before.reference.content_revision.checked_add(1)
                != Some(head.reference.content_revision)
        {
            return Err(invalid());
        }
    } else if head.head_revision != 1
        || head.reference.content_revision != 1
        || head.status != PlanLifecycleV1::Enabled
    {
        return Err(invalid());
    }
    if let Some(draft) = &control.consumed_draft {
        crate::AgentPlanId::parse(&draft.draft_id).map_err(|_| invalid())?;
        if draft.revision == 0 {
            return Err(invalid());
        }
    }
    let expected_draft: Option<ConsumedPlanDraftV1> = serde_json::from_value(
        spec.desired_state
            .get("consumed_draft")
            .cloned()
            .unwrap_or(Value::Null),
    )
    .map_err(|_| invalid())?;
    if expected_draft != control.consumed_draft {
        return Err(invalid());
    }
    let publication = record.verify_current().map_err(|_| invalid())?;
    if !publication.plan_heads.contains(head) {
        return Err(invalid());
    }
    if publication
        .plans
        .iter()
        .filter(|p| *p == &version.compiled)
        .count()
        != 1
        || publication
            .alias_registry
            .alias_for(&head.reference.plan_id)
            != Some(&head.model_alias)
    {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleChange {
    schema: String,
    plan_id: crate::AgentPlanId,
    expected_head_revision: u64,
    status: PlanLifecycleV1,
}
fn validate_lifecycle(
    spec: &ChangeSpecV1,
    control: &PlanContentControlV2,
    record: &PublicationRecordV1,
) -> Result<(), OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    let change: LifecycleChange =
        serde_json::from_value(spec.desired_state.clone()).map_err(|_| invalid())?;
    let before = control.before_head.as_ref().ok_or_else(invalid)?;
    before.validate().map_err(|_| invalid())?;
    let head = &control.plan_head;
    if change.schema != "hiroute.plan-lifecycle-change/v1"
        || change.plan_id != head.reference.plan_id
        || change.expected_head_revision != before.head_revision
        || change.status != head.status
        || before.reference != head.reference
        || before.model_alias != head.model_alias
        || before.head_revision.checked_add(1) != Some(head.head_revision)
        || before.status == PlanLifecycleV1::Deleted
        || before.status == head.status
        || control.consumed_draft.is_some()
        || control.legacy_source.is_some()
    {
        return Err(invalid());
    }
    let publication = record.verify_current().map_err(|_| invalid())?;
    if !publication.plan_heads.contains(head) {
        return Err(invalid());
    }
    if head.status == PlanLifecycleV1::Deleted {
        if publication
            .plans
            .iter()
            .any(|p| p.agent_plan_id() == &head.reference.plan_id)
            || !publication
                .alias_registry
                .tombstones
                .contains(&head.model_alias)
            || !publication
                .alias_registry
                .retired_plan_ids
                .contains(&head.reference.plan_id)
        {
            return Err(invalid());
        }
    } else if !publication.plans.contains(&control.plan_version.compiled)
        || publication
            .alias_registry
            .alias_for(&head.reference.plan_id)
            != Some(&head.model_alias)
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

pub(super) fn validate_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if !secrets.is_empty() || !runtime.is_empty() || external.len() != 1 {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let intent = &external[0];
    let effect = decode_external(
        &intent.effect_id,
        intent.kind,
        &intent.target,
        &intent.desired,
        intent.desired_mode,
        intent.sensitive,
    )?;
    let control: PlanContentControlV2 = serde::Deserialize::deserialize(control)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if effect.control != control || effect.change_spec != *spec {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    // decode_external validated these exact immutable inputs; the joins above bind the
    // independently supplied Plan fields without revalidating all nested publications.
    Ok(())
}
pub(super) fn validate_external(
    id: &str,
    kind: OwnedEffectKind,
    target: &str,
    desired: &Value,
    mode: u32,
    sensitive: bool,
) -> Result<(), OperationValidationError> {
    decode_external(id, kind, target, desired, mode, sensitive).map(|_| ())
}

pub(super) fn decode_external(
    id: &str,
    kind: OwnedEffectKind,
    target: &str,
    desired: &Value,
    mode: u32,
    sensitive: bool,
) -> Result<ContentEffectV2, OperationValidationError> {
    if id != "routing-publication"
        || kind != OwnedEffectKind::Publication
        || target != "publication/current"
        || mode != 0o644
        || sensitive
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let effect: ContentEffectV2 = serde::Deserialize::deserialize(desired)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if effect.schema != EFFECT_SCHEMA {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    validate_inputs(
        &effect.change_spec,
        &effect.control,
        &effect.publication_record,
    )?;
    Ok(effect)
}
pub(super) fn record(
    intent: &ExternalEffectIntentV1,
) -> Result<PublicationRecordV1, OperationValidationError> {
    intent
        .content_publication
        .as_ref()
        .map(|effect| effect.publication_record.clone())
        .ok_or(OperationValidationError::UnregisteredEffectPlan)
}
