//! Validation shared by live sealing and durable Operation reconstruction.
use super::*;

pub(crate) fn validate_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if !secrets.is_empty() || !runtime.is_empty() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    validate_agent_change_spec(spec)?;
    let control = decode_control(control)?;
    let transaction = AgentConnectionTransactionKindV1::parse(&control.transaction)?;
    if spec.command_id != transaction.command_id()
        || control.schema != CONTROL_SCHEMA
        || !valid_digest(&control.change_spec_digest)
        || control.change_spec_digest != CanonicalDigest::of(spec)?
        || !valid_digest(&control.payload_digest)
        || control.payload_digest != CanonicalDigest::of(&control.payload)?
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let subject = decode_subject(control.subject)?;
    validate_payload_value(&control.payload)?;
    validate_agent_access_grants(spec, transaction, &control.agent_access_grants)?;
    match (
        &control.agent_access_grants_digest,
        control.agent_access_grants.is_empty(),
    ) {
        (None, true) => {}
        (Some(digest), false)
            if valid_digest(digest)
                && *digest == CanonicalDigest::of(&control.agent_access_grants)? => {}
        _ => return Err(OperationValidationError::UnregisteredEffectPlan),
    }

    let mut roles = BTreeSet::new();
    for effect in external {
        validate_external_components(
            &effect.effect_id,
            effect.kind,
            &effect.target,
            &effect.desired,
            effect.desired_mode,
            effect.sensitive,
        )?;
        let envelope = decode_effect(&effect.desired)?;
        let role = AgentConnectionEffectRoleV1::parse(&envelope.role)?;
        if transaction == AgentConnectionTransactionKindV1::Settings && role.is_skill() {
            settings::validate_skill_selection(spec, role, &envelope.payload)?;
        }
        if AgentConnectionTransactionKindV1::parse(&envelope.transaction)? != transaction
            || decode_subject(envelope.subject)? != subject
            || envelope.change_spec_digest != control.change_spec_digest
            || !roles.insert(role)
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
    }

    if transaction == AgentConnectionTransactionKindV1::Settings {
        return settings::validate_effects(spec, &control.payload, &roles);
    }

    let required = BTreeSet::from([
        AgentConnectionEffectRoleV1::GrantScopedPublication,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
    ]);
    if !required.is_subset(&roles) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let routing_roles = [
        AgentConnectionEffectRoleV1::ModelCatalog,
        AgentConnectionEffectRoleV1::RoutingSkill,
        AgentConnectionEffectRoleV1::InstructionOverlay,
    ];
    let routing_count = routing_roles
        .iter()
        .filter(|role| roles.contains(role))
        .count();
    if !matches!(routing_count, 0 | 3)
        || (roles.contains(&AgentConnectionEffectRoleV1::SpawnGuidanceRewrite)
            && routing_count != 3)
        || roles.len() != external.len()
        || !matches!(roles.len(), 2 | 5 | 6)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

pub(crate) fn agent_access_grants_from_control(
    control: &Value,
) -> Result<Vec<AgentAccessGrantMutationV1>, OperationValidationError> {
    let control = decode_control(control)?;
    Ok(control.agent_access_grants)
}

pub(super) fn validate_agent_access_grants(
    spec: &ChangeSpecV1,
    transaction: AgentConnectionTransactionKindV1,
    mutations: &[AgentAccessGrantMutationV1],
) -> Result<(), OperationValidationError> {
    if transaction == AgentConnectionTransactionKindV1::Settings {
        return settings::validate_model_grants(spec, mutations);
    }
    if mutations.len() > 1 {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let expected_connection = spec
        .resource_id
        .as_deref()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    for mutation in mutations {
        mutation.validate()?;
        if mutation.connection_id() != expected_connection
            || !matches!(
                (transaction, mutation.kind()),
                (
                    AgentConnectionTransactionKindV1::Apply,
                    AgentAccessGrantMutationKindV1::Ensure
                ) | (
                    AgentConnectionTransactionKindV1::Restore,
                    AgentAccessGrantMutationKindV1::Revoke
                )
            )
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
    }
    Ok(())
}

pub(crate) fn validate_external_components(
    effect_id: &str,
    kind: OwnedEffectKind,
    target: &str,
    desired: &Value,
    desired_mode: u32,
    sensitive: bool,
) -> Result<(), OperationValidationError> {
    let envelope = decode_effect(desired)?;
    if envelope.schema != EFFECT_SCHEMA
        || sensitive
        || !valid_digest(&envelope.change_spec_digest)
        || !valid_digest(&envelope.payload_digest)
        || envelope.payload_digest != CanonicalDigest::of(&envelope.payload)?
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let transaction = AgentConnectionTransactionKindV1::parse(&envelope.transaction)?;
    let subject = decode_subject(envelope.subject)?;
    let role = AgentConnectionEffectRoleV1::parse(&envelope.role)?;
    validate_payload_value(&envelope.payload)?;
    let expected_target = if transaction == AgentConnectionTransactionKindV1::Settings {
        role.settings_payload_target_for(&subject, &envelope.payload_digest)?
    } else {
        effect_target(&subject, role)?
    };
    if effect_id != role.effect_id()
        || kind != role.owned_kind()
        || target != expected_target
        || !matches!(desired_mode, 0o600 | 0o640 | 0o644)
        || (role == AgentConnectionEffectRoleV1::GrantScopedPublication && desired_mode != 0o644)
        || !matches!(
            transaction,
            AgentConnectionTransactionKindV1::Apply
                | AgentConnectionTransactionKindV1::Restore
                | AgentConnectionTransactionKindV1::Settings
        )
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

pub(super) fn validate_agent_change_spec(
    spec: &ChangeSpecV1,
) -> Result<(), OperationValidationError> {
    if !matches!(
        spec.command_id.as_str(),
        "agents.connect.apply" | "agents.restore.apply" | "agents.settings.apply"
    ) || !spec
        .desired_state
        .as_object()
        .is_some_and(|value| !value.is_empty())
        || contains_internal_effect_dsl(&spec.desired_state)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if spec.command_id == AgentConnectionTransactionKindV1::Settings.command_id() {
        settings::decode_spec(spec)?;
    }
    Ok(())
}

fn contains_internal_effect_dsl(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = normalize_key(key);
            matches!(
                normalized.as_str(),
                "effectid"
                    | "effectkind"
                    | "effecttarget"
                    | "desiredmode"
                    | "sensitive"
                    | "externalintent"
                    | "artifacttarget"
            ) || contains_internal_effect_dsl(value)
        }),
        Value::Array(values) => values.iter().any(contains_internal_effect_dsl),
        _ => false,
    }
}

pub(super) fn registered_payload<T: Serialize>(
    payload: &T,
) -> Result<Value, OperationValidationError> {
    let payload = crate::canonicalize_json(serde_json::to_value(payload)?);
    validate_payload_value(&payload)?;
    Ok(payload)
}

pub(super) fn validate_payload_value(payload: &Value) -> Result<(), OperationValidationError> {
    if !payload.as_object().is_some_and(|object| !object.is_empty())
        || serde_json::to_vec(payload)?.len() > MAX_REGISTERED_PAYLOAD_BYTES
        || contains_sensitive_field(payload)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

fn contains_sensitive_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = normalize_key(key);
            matches!(
                normalized.as_str(),
                "secret"
                    | "plaintext"
                    | "apikey"
                    | "authorization"
                    | "bearer"
                    | "bearertoken"
                    | "password"
                    | "ciphertext"
                    | "secretstorelocator"
                    | "locator"
            ) || contains_sensitive_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_field),
        Value::String(value) => value.contains('\0'),
        _ => false,
    }
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
