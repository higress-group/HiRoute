use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

const CONTROL_SCHEMA: &str = "hiroute.routing-control/v1";
const EFFECT_SCHEMA: &str = "hiroute.routing-publication-effect/v1";
const EFFECT_ID: &str = "routing-publication";
const EFFECT_TARGET: &str = "publication/current";

/// Recovery-only create/update identity for already-durable V1 operation records.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RoutingTransactionIntentV1 {
    agent_plan_id: crate::AgentPlanId,
    expected_revision: Option<u64>,
}

impl RoutingTransactionIntentV1 {
    fn create(agent_plan_id: crate::AgentPlanId) -> Result<Self, OperationValidationError> {
        crate::AgentPlanId::parse(agent_plan_id.as_str())
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        Ok(Self {
            agent_plan_id,
            expected_revision: None,
        })
    }

    fn update(
        agent_plan_id: crate::AgentPlanId,
        expected_revision: u64,
    ) -> Result<Self, OperationValidationError> {
        crate::AgentPlanId::parse(agent_plan_id.as_str())
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        if expected_revision == 0 {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(Self {
            agent_plan_id,
            expected_revision: Some(expected_revision),
        })
    }

    fn agent_plan_id(&self) -> &crate::AgentPlanId {
        &self.agent_plan_id
    }

    fn expected_revision(&self) -> u64 {
        self.expected_revision.unwrap_or(0)
    }
}

impl Serialize for RoutingTransactionIntentV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        #[serde(tag = "intent", rename_all = "snake_case")]
        enum WireIntent<'a> {
            Create {
                agent_plan_id: &'a crate::AgentPlanId,
            },
            Update {
                agent_plan_id: &'a crate::AgentPlanId,
                expected_revision: u64,
            },
        }
        match self.expected_revision {
            Some(expected_revision) => WireIntent::Update {
                agent_plan_id: &self.agent_plan_id,
                expected_revision,
            },
            None => WireIntent::Create {
                agent_plan_id: &self.agent_plan_id,
            },
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
#[derive(Serialize)]
struct RoutingControlV1<'a> {
    schema: &'static str,
    transaction: &'static str,
    intent: &'a RoutingTransactionIntentV1,
    change_spec_digest: CanonicalDigest,
    compiled_plan_digest: CanonicalDigest,
    publication_digest: &'a CanonicalDigest,
    publication_revision: u64,
    compiled_plan: &'a crate::CompiledAgentPlanV1,
}

#[cfg(test)]
#[derive(Serialize)]
struct RoutingPublicationEffectV1<'a> {
    schema: &'static str,
    transaction: &'static str,
    intent: &'a RoutingTransactionIntentV1,
    change_spec_digest: CanonicalDigest,
    compiled_plan_digest: CanonicalDigest,
    publication_digest: &'a CanonicalDigest,
    publication_revision: u64,
    change_spec: &'a ChangeSpecV1,
    compiled_plan: &'a crate::CompiledAgentPlanV1,
    publication_record: &'a crate::PublicationRecordV1,
}

impl TransactionPlanV1 {
    /// Constructs a historical record only so unit tests can prove that the recovery decoder
    /// remains exact. Production planning is exclusively `from_plan_content_planner` V2.
    #[cfg(test)]
    fn legacy_routing_recovery_fixture(
        spec: ChangeSpecV1,
        intent: RoutingTransactionIntentV1,
        compiled_plan: crate::CompiledAgentPlanV1,
        publication_record: crate::PublicationRecordV1,
        before_publication_digest: Option<CanonicalDigest>,
    ) -> Result<Self, OperationValidationError> {
        validate_inputs(&spec, &intent, &compiled_plan, &publication_record)?;
        if (publication_record.publication_revision.get() == 1)
            != before_publication_digest.is_none()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let control = crate::canonicalize_json(serde_json::to_value(RoutingControlV1 {
            schema: CONTROL_SCHEMA,
            transaction: "apply",
            intent: &intent,
            change_spec_digest: CanonicalDigest::of(&spec)?,
            compiled_plan_digest: CanonicalDigest::of(&compiled_plan)?,
            publication_digest: &publication_record.digest,
            publication_revision: publication_record.publication_revision.get(),
            compiled_plan: &compiled_plan,
        })?);
        let desired = crate::canonicalize_json(serde_json::to_value(RoutingPublicationEffectV1 {
            schema: EFFECT_SCHEMA,
            transaction: "apply",
            intent: &intent,
            change_spec_digest: CanonicalDigest::of(&spec)?,
            compiled_plan_digest: CanonicalDigest::of(&compiled_plan)?,
            publication_digest: &publication_record.digest,
            publication_revision: publication_record.publication_revision.get(),
            change_spec: &spec,
            compiled_plan: &compiled_plan,
            publication_record: &publication_record,
        })?);
        let external = vec![ExternalEffectIntentV1 {
            content_publication: None,
            effect_id: EFFECT_ID.to_owned(),
            kind: OwnedEffectKind::Publication,
            target: EFFECT_TARGET.to_owned(),
            before_fingerprint: before_publication_digest,
            desired: desired.into(),
            desired_mode: 0o644,
            sensitive: false,
        }];
        validate_plan(&spec, &control, &[], &[], &external)?;
        Ok(Self {
            spec,
            control,
            credential_pool: None,
            worker_dependency_selection: None,
            secrets: Vec::new(),
            agent_access_grants: Vec::new(),
            runtime: Vec::new(),
            external,
        })
    }
}

/// Returns the authenticated current publication for a sealed routing transaction. Historical
/// V1 effects are verified against their original bytes and resealed before leaving this seam.
pub fn routing_publication_record(
    intent: &ExternalEffectIntentV1,
) -> Result<Option<crate::PublicationRecordV1>, OperationValidationError> {
    if intent.effect_id != EFFECT_ID {
        return Ok(None);
    }
    if super::routing_content::is_effect(&intent.desired) {
        return super::routing_content::record(intent).map(Some);
    }
    validate_external_components(
        &intent.effect_id,
        intent.kind,
        &intent.target,
        &intent.desired,
        intent.desired_mode,
        intent.sensitive,
    )?;
    let effect: DurableRoutingEffectV1 =
        serde::Deserialize::deserialize(intent.desired.as_ref())
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let legacy_record = effect.publication_record;
    let publication = legacy_record
        .verify()
        .and_then(crate::GatewayPublicationV1::into_current)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let current_record =
        crate::PublicationRecordV1::from_publication(legacy_record.workspace_id, &publication)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(Some(current_record))
}

pub(super) fn validate_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if super::routing_draft::is_control(control) {
        return super::routing_draft::validate_plan(spec, control, secrets, runtime, external);
    }
    if super::routing_content::is_control(control) {
        return super::routing_content::validate_plan(spec, control, secrets, runtime, external);
    }
    if !secrets.is_empty() || !runtime.is_empty() || external.len() != 1 {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let control: DurableRoutingControlV1 = serde_json::from_value(control.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if control.schema != CONTROL_SCHEMA || control.transaction != "apply" {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let intent = control.intent.into_typed()?;
    validate_external_components(
        &external[0].effect_id,
        external[0].kind,
        &external[0].target,
        &external[0].desired,
        external[0].desired_mode,
        external[0].sensitive,
    )?;
    let effect: DurableRoutingEffectV1 =
        serde::Deserialize::deserialize(external[0].desired.as_ref())
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let effect_intent = effect.intent.into_typed()?;
    if effect.schema != EFFECT_SCHEMA
        || effect.transaction != "apply"
        || intent != effect_intent
        || control.change_spec_digest != CanonicalDigest::of(spec)?
        || effect.change_spec_digest != control.change_spec_digest
        || control.compiled_plan_digest != CanonicalDigest::of(&control.compiled_plan)?
        || effect.compiled_plan_digest != control.compiled_plan_digest
        || effect.publication_digest != control.publication_digest
        || effect.publication_revision != control.publication_revision
        || effect.publication_record.digest != control.publication_digest
        || effect.publication_record.publication_revision.get() != control.publication_revision
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    validate_inputs(
        spec,
        &intent,
        &control.compiled_plan,
        &effect.publication_record,
    )
}

pub(super) fn validate_external_components(
    effect_id: &str,
    kind: OwnedEffectKind,
    target: &str,
    desired: &Value,
    desired_mode: u32,
    sensitive: bool,
) -> Result<(), OperationValidationError> {
    if super::routing_content::is_effect(desired) {
        return super::routing_content::validate_external(
            effect_id,
            kind,
            target,
            desired,
            desired_mode,
            sensitive,
        );
    }
    let effect: DurableRoutingEffectV1 = serde_json::from_value(desired.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if effect_id != EFFECT_ID
        || kind != OwnedEffectKind::Publication
        || target != EFFECT_TARGET
        || desired_mode != 0o644
        || sensitive
        || effect.schema != EFFECT_SCHEMA
        || effect.transaction != "apply"
        || effect.change_spec_digest != CanonicalDigest::of(&effect.change_spec)?
        || effect.compiled_plan_digest != CanonicalDigest::of(&effect.compiled_plan)?
        || effect.publication_digest != effect.publication_record.digest
        || effect.publication_revision != effect.publication_record.publication_revision.get()
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let intent = effect.intent.into_typed()?;
    validate_inputs(
        &effect.change_spec,
        &intent,
        &effect.compiled_plan,
        &effect.publication_record,
    )
}

fn validate_inputs(
    spec: &ChangeSpecV1,
    intent: &RoutingTransactionIntentV1,
    compiled_plan: &crate::CompiledAgentPlanV1,
    publication_record: &crate::PublicationRecordV1,
) -> Result<(), OperationValidationError> {
    if spec.command_id != "routing.apply"
        || spec.resource_id.as_deref()
            != Some(&format!("agent-plan/{}", intent.agent_plan_id().as_str()))
        || compiled_plan.agent_plan_id() != intent.agent_plan_id()
        || intent
            .expected_revision()
            .checked_add(1)
            .is_none_or(|revision| compiled_plan.body.agent_plan_revision != revision)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    compiled_plan
        .validate()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if compiled_plan.body.schema != crate::AGENT_PLAN_COMPILED_SCHEMA_V1 {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let publication = publication_record
        .verify()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if publication.validate_current_contract().is_ok() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let exact_plan_count = publication
        .plans
        .iter()
        .filter(|plan| *plan == compiled_plan)
        .count();
    if exact_plan_count != 1
        || publication.alias_registry.alias_for(intent.agent_plan_id())
            != Some(&compiled_plan.body.identity.model_alias)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
enum DurableIntentV1 {
    Create {
        agent_plan_id: crate::AgentPlanId,
    },
    Update {
        agent_plan_id: crate::AgentPlanId,
        expected_revision: u64,
    },
}

impl DurableIntentV1 {
    fn into_typed(self) -> Result<RoutingTransactionIntentV1, OperationValidationError> {
        match self {
            Self::Create { agent_plan_id } => RoutingTransactionIntentV1::create(agent_plan_id),
            Self::Update {
                agent_plan_id,
                expected_revision,
            } => RoutingTransactionIntentV1::update(agent_plan_id, expected_revision),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableRoutingControlV1 {
    schema: String,
    transaction: String,
    intent: DurableIntentV1,
    change_spec_digest: CanonicalDigest,
    compiled_plan_digest: CanonicalDigest,
    publication_digest: CanonicalDigest,
    publication_revision: u64,
    compiled_plan: crate::CompiledAgentPlanV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableRoutingEffectV1 {
    schema: String,
    transaction: String,
    intent: DurableIntentV1,
    change_spec_digest: CanonicalDigest,
    compiled_plan_digest: CanonicalDigest,
    publication_digest: CanonicalDigest,
    publication_revision: u64,
    publication_record: crate::PublicationRecordV1,
    change_spec: ChangeSpecV1,
    compiled_plan: crate::CompiledAgentPlanV1,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        ChangeSpecV1,
        RoutingTransactionIntentV1,
        crate::CompiledAgentPlanV1,
        crate::PublicationRecordV1,
    ) {
        let bytes =
            include_bytes!("../../../../e2e/product/fixtures/routing/current-publication.v3.json");
        let mut publication = serde_json::from_value::<crate::GatewayPublicationV1>(
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap(),
        )
        .unwrap();
        // This retired typed-reader seam requires an authenticated V1 compiled body.
        // The untouched historical source is authenticated by the source-22 reader above;
        // this normalized fixture exercises effect ownership, not a source-format claim.
        for plan in &mut publication.plans {
            let mut body = plan.body.as_ref().clone();
            body.schema = crate::AGENT_PLAN_COMPILED_SCHEMA_V1.into();
            body.compiler_revision = crate::AGENT_PLAN_COMPILER_REVISION_V1.into();
            body.materialized_route_digest = body.materialized.legacy_route_digest().unwrap();
            let digest = CanonicalDigest::of(&body).unwrap();
            *plan = crate::CompiledAgentPlanV1::authenticate_persisted(body, digest).unwrap();
        }
        publication.schema = crate::publication::LEGACY_GATEWAY_PUBLICATION_SCHEMA_V2.into();
        publication.compiler_revision = crate::AGENT_PLAN_COMPILER_REVISION_V1.into();
        publication.grants.clear();
        publication.aliases.clear();
        publication.validate().unwrap();
        let compiled = publication
            .plans
            .iter()
            .find(|plan| plan.body.agent_plan_revision == 1)
            .unwrap()
            .clone();
        let intent = RoutingTransactionIntentV1::create(compiled.agent_plan_id().clone()).unwrap();
        let spec = ChangeSpecV1 {
            schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
            command_id: "routing.apply".to_owned(),
            resource_id: Some(format!("agent-plan/{}", compiled.agent_plan_id().as_str())),
            desired_state: json!({"agent_plan_id": compiled.agent_plan_id()}),
        };
        // Exercise the authenticated pre-stable runtime representation. Genuine source-22
        // bytes are separately authenticated by the storage migration reader.
        let bytes = serde_json::to_vec(&publication).unwrap();
        let record = crate::PublicationRecordV1::from_parts(
            publication.workspace_id.clone(),
            publication.publication_revision,
            CanonicalDigest::of_bytes(&bytes),
            bytes,
        )
        .unwrap();
        (spec, intent, compiled, record)
    }

    #[test]
    fn legacy_routing_recovery_reseals_the_publication_as_current() {
        let (spec, intent, compiled, record) = fixture();
        let legacy_digest = record.digest.clone();
        let before = Some(CanonicalDigest::of_bytes(b"prior-publication"));
        let plan = TransactionPlanV1::legacy_routing_recovery_fixture(
            spec, intent, compiled, record, before,
        )
        .unwrap();
        assert!(plan.secrets().is_empty());
        assert!(plan.runtime().is_empty());
        assert_eq!(plan.external().len(), 1);
        assert_eq!(plan.external()[0].effect_id(), EFFECT_ID);
        assert_eq!(plan.external()[0].target(), EFFECT_TARGET);
        assert_eq!(plan.external()[0].kind(), OwnedEffectKind::Publication);
        let current = routing_publication_record(&plan.external()[0])
            .unwrap()
            .unwrap();
        assert_ne!(current.digest, legacy_digest);
        current
            .verify()
            .unwrap()
            .validate_current_contract()
            .unwrap();
    }

    #[test]
    fn routing_plan_rejects_arbitrary_effect_target_and_mode() {
        let (spec, intent, compiled, record) = fixture();
        let mut plan = TransactionPlanV1::legacy_routing_recovery_fixture(
            spec,
            intent,
            compiled,
            record,
            Some(CanonicalDigest::of_bytes(b"prior-publication")),
        )
        .unwrap();
        plan.external[0].target = "publication/attacker".to_owned();
        plan.external[0].desired_mode = 0o600;
        assert!(matches!(
            validate_registered_plan(
                &plan.spec,
                &plan.control,
                plan.credential_pool.as_ref(),
                &plan.secrets,
                &plan.runtime,
                &plan.external,
            ),
            Err(OperationValidationError::UnregisteredEffectPlan)
        ));
    }

    #[test]
    fn routing_plan_rejects_compiled_revision_not_bound_to_intent() {
        let (spec, _, compiled, record) = fixture();
        let update =
            RoutingTransactionIntentV1::update(compiled.agent_plan_id().clone(), 1).unwrap();
        assert!(matches!(
            TransactionPlanV1::legacy_routing_recovery_fixture(
                spec,
                update,
                compiled,
                record,
                Some(CanonicalDigest::of_bytes(b"prior-publication")),
            ),
            Err(OperationValidationError::UnregisteredEffectPlan)
        ));
    }
}
