//! Versioned admission facts. Only validated command codecs can create an execution view.
use super::*;

pub const OPERATION_INPUT_SCHEMA: &str = "hiroute.operation-input/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOperationInputV1 {
    pub schema: String,
    pub spec: ChangeSpecV1,
    pub control: Value,
    pub credential_pool: Option<CredentialPoolMutationV1>,
    pub worker_dependency_selection: Option<WorkerDependencySelectionChangeV1>,
    pub secrets: Vec<StoredSecretMutationV1>,
    pub runtime: Vec<StoredRuntimeMutationV1>,
    pub external: Vec<StoredExternalIntentV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSecretMutationV1 {
    kind: SecretMutationKind,
    credential: CredentialRefV1,
    expected_generation: u64,
    fingerprint_algorithm: SecretFingerprintAlgorithm,
    input_slot: Option<String>,
    fingerprint: Option<CanonicalDigest>,
    new_allowed_destinations: Option<BTreeSet<String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRuntimeMutationV1 {
    key: String,
    value: Value,
    expected_generation: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredExternalIntentV1 {
    pub(super) effect_id: String,
    pub(super) kind: OwnedEffectKind,
    pub(super) target: String,
    pub(super) before_fingerprint: Option<CanonicalDigest>,
    pub(super) desired: Value,
    pub(super) desired_mode: u32,
    pub(super) sensitive: bool,
}

impl StoredOperationInputV1 {
    pub fn freeze(plan: &TransactionPlanV1) -> Result<Self, OperationValidationError> {
        let stored = Self {
            schema: OPERATION_INPUT_SCHEMA.into(),
            spec: plan.spec.clone(),
            control: plan.control.clone(),
            credential_pool: plan.credential_pool.clone(),
            worker_dependency_selection: plan.worker_dependency_selection.clone(),
            secrets: plan
                .secrets
                .iter()
                .map(|s| StoredSecretMutationV1 {
                    kind: s.kind,
                    credential: s.credential.clone(),
                    expected_generation: s.expected_generation,
                    fingerprint_algorithm: s.fingerprint_algorithm,
                    input_slot: s.input_slot.clone(),
                    fingerprint: s.fingerprint.clone(),
                    new_allowed_destinations: s.new_allowed_destinations.clone(),
                })
                .collect(),
            runtime: plan
                .runtime
                .iter()
                .map(|m| StoredRuntimeMutationV1 {
                    key: m.key.clone(),
                    value: m.value.clone(),
                    expected_generation: m.expected_generation,
                })
                .collect(),
            external: plan
                .external
                .iter()
                .map(|e| StoredExternalIntentV1 {
                    effect_id: e.effect_id.clone(),
                    kind: e.kind,
                    target: e.target.clone(),
                    before_fingerprint: e.before_fingerprint.clone(),
                    desired: (*e.desired).clone(),
                    desired_mode: e.desired_mode,
                    sensitive: e.sensitive,
                })
                .collect(),
        };
        Ok(stored)
    }

    pub fn build(&self) -> Result<TransactionPlanV1, OperationValidationError> {
        if self.schema != OPERATION_INPUT_SCHEMA {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let secrets = self
            .secrets
            .iter()
            .map(|s| {
                if s.fingerprint_algorithm != SecretFingerprintAlgorithm::HmacSha256V1 {
                    return Err(OperationValidationError::UnregisteredEffectPlan);
                }
                match s.kind {
                    SecretMutationKind::Upsert if s.new_allowed_destinations.is_none() => {
                        SecretMutationV1::upsert(
                            s.credential.clone(),
                            s.expected_generation,
                            s.input_slot
                                .clone()
                                .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
                            s.fingerprint.clone(),
                        )
                    }
                    SecretMutationKind::Delete
                        if s.input_slot.is_none()
                            && s.fingerprint.is_none()
                            && s.new_allowed_destinations.is_none() =>
                    {
                        SecretMutationV1::delete(s.credential.clone(), s.expected_generation)
                    }
                    SecretMutationKind::Rebind if s.input_slot.is_none() => {
                        SecretMutationV1::rebind(
                            s.credential.clone(),
                            s.new_allowed_destinations
                                .clone()
                                .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
                            s.fingerprint
                                .clone()
                                .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
                        )
                    }
                    _ => Err(OperationValidationError::UnregisteredEffectPlan),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let runtime = self
            .runtime
            .iter()
            .map(|m| {
                RuntimeMutationV1::from_registered_planner(
                    m.key.clone(),
                    m.value.clone(),
                    m.expected_generation,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let external = self
            .external
            .iter()
            .map(|e| {
                ExternalEffectIntentV1::from_registered_adapter(
                    e.effect_id.clone(),
                    e.kind,
                    e.target.clone(),
                    e.before_fingerprint.clone(),
                    e.desired.clone(),
                    e.desired_mode,
                    e.sensitive,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let plan = if let Some(selection) = &self.worker_dependency_selection {
            if self.control != json!({})
                || self.credential_pool.is_some()
                || !secrets.is_empty()
                || !runtime.is_empty()
                || !external.is_empty()
            {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            let selection = WorkerDependencySelectionChangeV1::new(
                selection.before_revision,
                WorkerDependencySelectionRecordV1::new(
                    selection.after_selection.harness,
                    selection.after_selection.adapter_path.clone(),
                    selection.after_selection.cli_path.clone(),
                    selection.after_selection.node_path.clone(),
                )?,
            )?;
            TransactionPlanV1::from_worker_dependency_selection_planner(
                self.spec.clone(),
                selection,
            )?
        } else {
            // The command's existing typed codec strictly validates every control/effect payload.
            TransactionPlanV1::from_registered_typed_planner(
                self.spec.clone(),
                self.control.clone(),
                self.credential_pool.clone(),
                secrets,
                runtime,
                external,
            )?
        };
        if Self::freeze(&plan)? != *self {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(plan)
    }

    pub(super) fn step_inputs(
        &self,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
        grants: &[AgentAccessGrantMutationV1],
    ) -> Result<[Value; 6], OperationValidationError> {
        Ok([
            json!({"spec": &self.spec, "accepted_digest": accepted_digest, "expected_revisions": expected_revisions}),
            serde_json::to_value((&self.secrets, grants))?,
            serde_json::to_value((
                &self.control,
                &self.credential_pool,
                &self.worker_dependency_selection,
            ))?,
            serde_json::to_value(
                self.external
                    .iter()
                    .filter(|effect| effect.kind == OwnedEffectKind::Publication)
                    .collect::<Vec<_>>(),
            )?,
            serde_json::to_value(
                self.external
                    .iter()
                    .filter(|effect| effect.kind == OwnedEffectKind::AgentArtifact)
                    .collect::<Vec<_>>(),
            )?,
            serde_json::to_value(&self.runtime)?,
        ])
    }

    pub fn validate_step_proofs(
        &self,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
        grants: &[AgentAccessGrantMutationV1],
        steps: &[OperationStepV1],
    ) -> Result<(), OperationValidationError> {
        if steps.len() != 6 {
            return Err(OperationValidationError::InvalidDurableJournal);
        }
        for (index, ((step, kind), input)) in steps
            .iter()
            .zip(OperationStepKind::ALL)
            .zip(self.step_inputs(accepted_digest, expected_revisions, grants)?)
            .enumerate()
        {
            if usize::from(step.sequence) != index
                || step.kind != kind
                || step.deterministic_input_digest
                    != CanonicalDigest::of(&("hiroute.operation-step-input/v1", kind, input))?
            {
                return Err(OperationValidationError::InvalidDurableJournal);
            }
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String, OperationValidationError> {
        Ok(crate::canonicalize_json(serde_json::to_value(self)?).to_string())
    }
    pub fn digest(&self) -> Result<CanonicalDigest, OperationValidationError> {
        Ok(CanonicalDigest::of(&(OPERATION_INPUT_SCHEMA, self))?)
    }
}

pub(super) fn serialize_plan<S: serde::Serializer>(
    plan: &Arc<TransactionPlanV1>,
    s: S,
) -> Result<S::Ok, S::Error> {
    let facts = StoredOperationInputV1::freeze(plan).map_err(serde::ser::Error::custom)?;
    crate::canonicalize_json(serde_json::to_value(facts).map_err(serde::ser::Error::custom)?)
        .serialize(s)
}
