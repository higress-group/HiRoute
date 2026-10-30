//! Validation and sealing for managed compute source transactions.

use std::collections::BTreeMap;

use super::*;

impl TransactionPlanV1 {
    pub fn with_source_network(
        mut self,
        network: crate::NativeNetworkMutationV1,
    ) -> Result<Self, OperationValidationError> {
        validate_management_plan(
            &self.spec,
            &self.control,
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        let source_mutation = serde_json::from_value(
            self.control
                .get("compute_management_mutation")
                .cloned()
                .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
        )
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        self.spec.desired_state = json!({
            "schema": crate::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4,
            "change": self.spec.desired_state, "network": network,
        });
        let mutation = crate::ComputeManagementMutationV4 {
            schema: crate::COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V4.into(),
            source_mutation: Box::new(source_mutation),
            network,
            change_spec_digest: CanonicalDigest::of(&self.spec)
                .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?,
        };
        mutation
            .validate(&self.spec)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        self.control = json!({"compute_management_mutation": mutation});
        validate_management_plan(
            &self.spec,
            &self.control,
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        Ok(self)
    }
    /// Seals one source-level management mutation into the existing recoverable transaction.
    /// Secret bytes remain outside the plan; exact HMAC fingerprints and generations bind the
    /// staged management record to the Secret effects applied by the same Operation.
    pub fn from_compute_management_planner(
        spec: ChangeSpecV1,
        current: Option<&crate::ComputeManagementSourceV2>,
        desired: crate::ComputeManagementSourceV2,
        secrets: Vec<SecretMutationV1>,
    ) -> Result<Self, OperationValidationError> {
        Self::from_compute_management_change(spec, current, Some(desired), secrets)
    }

    pub fn from_compute_management_change(
        spec: ChangeSpecV1,
        current: Option<&crate::ComputeManagementSourceV2>,
        desired: Option<crate::ComputeManagementSourceV2>,
        secrets: Vec<SecretMutationV1>,
    ) -> Result<Self, OperationValidationError> {
        let mutation =
            crate::ComputeManagementMutationV2::from_optional_planner(&spec, current, desired)
                .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        let control = json!({"compute_management_mutation": &mutation});
        validate_management_plan(&spec, &control, &secrets, &[], &[])?;
        Ok(Self {
            spec,
            control,
            credential_pool: None,
            worker_dependency_selection: None,
            secrets,
            agent_access_grants: Vec::new(),
            runtime: Vec::new(),
            external: Vec::new(),
        })
    }
    pub fn with_source_defaults(
        mut self,
        preferences_before: crate::SourceDefaultPreferencesV1,
        preferences_after: crate::SourceDefaultPreferencesV1,
    ) -> Result<Self, OperationValidationError> {
        validate_registered_plan(
            &self.spec,
            &self.control,
            self.credential_pool.as_ref(),
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        let source_mutation: crate::ComputeManagementMutationV2 = serde_json::from_value(
            self.control
                .get("compute_management_mutation")
                .cloned()
                .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
        )
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        self.spec.desired_state = json!({"schema": crate::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3, "change": self.spec.desired_state, "default_preferences": preferences_after});
        let mutation = crate::ComputeManagementMutationV3 {
            schema: crate::COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V3.into(),
            source_mutation,
            preferences_before,
            preferences_after,
            change_spec_digest: CanonicalDigest::of(&self.spec)
                .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?,
        };
        mutation
            .validate(&self.spec)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        self.control = json!({"compute_management_mutation": mutation});
        validate_registered_plan(
            &self.spec,
            &self.control,
            self.credential_pool.as_ref(),
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        Ok(self)
    }
}

pub(super) fn validate_management_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        compute_management_mutation: crate::ComputeManagementMutation,
    }

    if !runtime.is_empty() || !external.is_empty() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let envelope: Envelope = serde_json::from_value(control.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let mutation = envelope.compute_management_mutation;
    match &mutation {
        crate::ComputeManagementMutation::V4(value) => value.validate(spec),
        crate::ComputeManagementMutation::V2(value) => value.validate_shape(spec),
        crate::ComputeManagementMutation::V3(value) => value.validate(spec),
    }
    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if control != &json!({"compute_management_mutation": &mutation}) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }

    let expected = mutation
        .expected()
        .map(|source| {
            source
                .credentials
                .iter()
                .map(|credential| (credential.key_id.as_str(), credential))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let desired = mutation
        .desired()
        .map(|source| {
            source
                .credentials
                .iter()
                .map(|credential| (credential.key_id.as_str(), credential))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let destinations = mutation
        .desired()
        .map(|source| source.native_destinations())
        .transpose()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?
        .unwrap_or_default();

    let mut secret_by_id = BTreeMap::new();
    for secret in secrets {
        if secret_by_id
            .insert(secret.credential().credential_id(), secret)
            .is_some()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
    }
    let changed_count = expected
        .keys()
        .chain(desired.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key_id| match (expected.get(key_id), desired.get(key_id)) {
            (Some(before), Some(after)) => {
                before.credential.generation() != after.credential.generation()
                    || before.fingerprint != after.fingerprint
                    || before.credential.allowed_destinations()
                        != after.credential.allowed_destinations()
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        })
        .count();
    if changed_count != secrets.len() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }

    for key_id in expected
        .keys()
        .chain(desired.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        match (expected.get(key_id), desired.get(key_id)) {
            (None, Some(after)) => {
                validate_management_upsert(
                    mutation.source_id(),
                    &destinations,
                    0,
                    after,
                    secret_by_id.get(key_id).copied(),
                )?;
            }
            (Some(before), Some(after))
                if before.credential.generation() != after.credential.generation()
                    || before.fingerprint != after.fingerprint
                    || before.credential.allowed_destinations()
                        != after.credential.allowed_destinations() =>
            {
                if before.fingerprint == after.fingerprint
                    && before.credential.allowed_destinations()
                        != after.credential.allowed_destinations()
                {
                    let secret = secret_by_id
                        .get(key_id)
                        .copied()
                        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
                    if secret.kind() != SecretMutationKind::Rebind
                        || secret.credential() != &before.credential
                        || secret.expected_generation() != before.credential.generation()
                        || secret.new_allowed_destinations() != Some(&destinations)
                        || secret.fingerprint() != Some(&before.fingerprint)
                        || secret.input_slot().is_some()
                        || after.credential.generation()
                            != before.credential.generation().saturating_add(1)
                        || after.credential.allowed_destinations() != &destinations
                    {
                        return Err(OperationValidationError::UnregisteredEffectPlan);
                    }
                } else {
                    validate_management_upsert(
                        mutation.source_id(),
                        &destinations,
                        before.credential.generation(),
                        after,
                        secret_by_id.get(key_id).copied(),
                    )?;
                }
            }
            (Some(before), None) => {
                let secret = secret_by_id
                    .get(key_id)
                    .copied()
                    .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
                if secret.kind() != SecretMutationKind::Delete
                    || secret.expected_generation() != before.credential.generation()
                    || secret.input_slot().is_some()
                    || secret.fingerprint().is_some()
                    || secret.credential() != &before.credential
                {
                    return Err(OperationValidationError::UnregisteredEffectPlan);
                }
            }
            (Some(before), Some(after)) if before != after => {
                // Ordering and enabled state are management-only edits. Credential authority must
                // remain byte-for-byte identical when no Secret mutation accompanies the key.
                if before.key_id != after.key_id
                    || before.credential != after.credential
                    || before.fingerprint != after.fingerprint
                {
                    return Err(OperationValidationError::UnregisteredEffectPlan);
                }
            }
            (Some(_), Some(_)) => {}
            (None, None) => unreachable!(),
        }
    }
    Ok(())
}

fn validate_management_upsert(
    source_id: &str,
    destinations: &BTreeSet<String>,
    expected_generation: u64,
    desired: &crate::ComputeManagedCredentialV2,
    secret: Option<&SecretMutationV1>,
) -> Result<(), OperationValidationError> {
    let secret = secret.ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    let reference = secret.credential();
    if secret.kind() != SecretMutationKind::Upsert
        || secret.expected_generation() != expected_generation
        || secret.input_slot().is_none()
        || secret.fingerprint() != Some(&desired.fingerprint)
        || desired.credential.generation() != expected_generation.saturating_add(1)
        || reference.credential_id() != desired.key_id
        || reference.owner_scope() != format!("source/{source_id}")
        || reference.subject() != "hirouted"
        || reference.purpose() != "provider-auth"
        || reference.allowed_destinations() != destinations
        || secret.new_allowed_destinations().is_some()
        || reference.generation() != expected_generation
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}
