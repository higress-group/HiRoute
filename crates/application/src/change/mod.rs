use std::collections::BTreeSet;

use hiroute_application_api::{
    ClassifierHeaderSecretInputV1, CommandKind, PreviewRequestV1, PreviewResultV1, command_by_id,
};
use hiroute_domain::{
    CanonicalDigest, ComputeSourceV1, ConnectorRegistryBundleV1, CredentialPoolIdentityV1,
    CredentialPoolMutationKind, CredentialPoolMutationV1, CredentialPoolV1, CredentialRefV1,
    ExternalEffectIntentV1, ExternalEffectPort, OwnedEffectKind, PortResult, ProtectedSecret,
    RevisionSetV1, RuntimeMutationV1, RuntimeStatePort, SecretMutationV1, SecretStorePort,
    TransactionPlanV1, canonicalize_json, normalize_change,
};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::compute_management::ProtectedInputSourceDescriptorV1;

mod compute;

/// Reads a logical Secret slot through a protected adapter. Slots are safe labels, not paths,
/// provider locators, or SecretStore references.
pub trait ProtectedInputPort {
    fn read_secret(&self, input_slot: &str) -> PortResult<ProtectedSecret>;

    /// Revalidates the complete, non-wire discovery evidence associated with a protected slot.
    /// This is separate from `read_secret`: the credential file can remain unchanged while a
    /// higher-precedence endpoint/model configuration or the client-bundled catalog changes.
    fn validate_discovery_evidence(
        &self,
        input_slot: &str,
        expected_evidence: &CanonicalDigest,
    ) -> PortResult<()>;
}

/// Read-only projection of the currently verified Release Connector Registry. The planner never
/// owns a hard-coded provider list: an exact option must be present in the installed current
/// registry before any ProtectedInput, Secret, external, or durable port is touched.
pub trait ConnectionOptionAuthorizationPort {
    fn connection_option_authorization(
        &self,
        connection_option_id: &str,
    ) -> PortResult<Option<ConnectionOptionAuthorizationV1>>;
    fn source_uses_connection_option(
        &self,
        source_id: &str,
        connection_option_id: &str,
    ) -> PortResult<bool>;
    fn is_registered_compute_source(&self, source_id: &str) -> PortResult<bool>;
    fn is_registered_price_target(
        &self,
        offer_ref: &str,
        model_configuration_id: &str,
        currency: &str,
        target_rule_id: Option<&str>,
    ) -> PortResult<bool>;
    /// Returns the already Release-validated identity for one exact Binding, including the pool
    /// that will be created by its first credential Add.
    fn credential_pool_identity(
        &self,
        pool_id: &str,
        binding_id: &str,
    ) -> PortResult<Option<CredentialPoolIdentityV1>>;
    fn credential_pool(&self, pool_id: &str) -> PortResult<Option<CredentialPoolV1>>;
    /// Counts ordinary pool references to the stable CredentialRef. Secret material may be
    /// deleted only when removing the final reference.
    fn credential_reference_count(&self, credential_id: &str) -> PortResult<u64>;
    /// Returns a source produced from discovery facts and validated against the exact installed
    /// current client-bundled registry. The planner revalidates the returned pair before sealing
    /// the journal.
    fn compute_source_materialization(
        &self,
        connection_option_id: &str,
        source_id: &str,
        expected_revision: u64,
        explicit_materialization: bool,
    ) -> PortResult<Option<RegisteredComputeSourceMaterializationV1>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionOptionAuthorizationV1 {
    pub requires_explicit_materialization: bool,
    pub accepts_native_secret: bool,
}

#[derive(Clone)]
pub struct RegisteredComputeSourceMaterializationV1 {
    pub current: Option<ComputeSourceV1>,
    pub desired: ComputeSourceV1,
    pub registry: ConnectorRegistryBundleV1,
    pub explicit_materialization: bool,
}

/// Temporary owner-internal DTO for the one registered P0 planner. Formal command DTOs remain
/// owned by their later handler PROCESS. Unknown fields are rejected and none of these fields can
/// describe an effect target, URL, ciphertext, or generic desired effect.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupPlannerInputV1 {
    connection_option_id: String,
    #[serde(default)]
    secret: Option<SetupSecretInputV1>,
    #[serde(default)]
    runtime_expected_generation: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupSecretInputV1 {
    input_slot: String,
    #[serde(default)]
    expected_generation: u64,
}

struct PendingPoolMutationV1 {
    kind: CredentialPoolMutationKind,
    identity: CredentialPoolIdentityV1,
    current: Option<CredentialPoolV1>,
    credential_id: Option<String>,
}

struct RegisteredEffectPlan {
    control: Value,
    pending_compute_source: Option<RegisteredComputeSourceMaterializationV1>,
    credential_pool: Option<CredentialPoolMutationV1>,
    pending_pool: Option<PendingPoolMutationV1>,
    secret_sources: std::collections::BTreeMap<String, ProtectedInputSourceDescriptorV1>,
    secrets: Vec<SecretMutationV1>,
    runtime: Vec<RuntimeMutationV1>,
    external: Vec<ExternalEffectIntentV1>,
}

impl RegisteredEffectPlan {
    fn finalize_credential_pool(&mut self) -> Result<(), ChangePreparationError> {
        let Some(pending) = self.pending_pool.take() else {
            return Ok(());
        };
        let desired = match pending.kind {
            CredentialPoolMutationKind::Add | CredentialPoolMutationKind::Replace => {
                let credential_id = pending
                    .credential_id
                    .as_deref()
                    .ok_or(ChangePreparationError::InvalidCredentialPool)?;
                let secret = self
                    .secrets
                    .iter()
                    .find(|mutation| mutation.credential().credential_id() == credential_id)
                    .ok_or(ChangePreparationError::InvalidCredentialPool)?;
                let fingerprint = secret
                    .fingerprint()
                    .cloned()
                    .ok_or(ChangePreparationError::SecretFingerprintMismatch)?;
                let generation = secret
                    .expected_generation()
                    .checked_add(1)
                    .ok_or(ChangePreparationError::InvalidCredentialPool)?;
                let replacement = CredentialRefV1::new(
                    credential_id,
                    secret.credential().owner_scope(),
                    secret.credential().subject(),
                    secret.credential().purpose(),
                    secret.credential().allowed_destinations().iter().cloned(),
                    generation,
                )?;
                match (pending.kind, pending.current.as_ref()) {
                    (CredentialPoolMutationKind::Add, Some(current)) => {
                        current.add(current.revision, replacement, fingerprint)?
                    }
                    (CredentialPoolMutationKind::Add, None) => pending
                        .identity
                        .materialize_first(replacement, fingerprint)?,
                    (CredentialPoolMutationKind::Replace, Some(current)) => current.replace(
                        current.revision,
                        credential_id,
                        replacement,
                        fingerprint,
                    )?,
                    _ => return Err(ChangePreparationError::CredentialPoolNotFound),
                }
            }
            CredentialPoolMutationKind::Remove => {
                let current = pending
                    .current
                    .as_ref()
                    .ok_or(ChangePreparationError::CredentialPoolNotFound)?;
                current.remove(
                    current.revision,
                    pending
                        .credential_id
                        .as_deref()
                        .ok_or(ChangePreparationError::InvalidCredentialPool)?,
                )?
            }
            CredentialPoolMutationKind::Reorder => {
                return Err(ChangePreparationError::InvalidCredentialPool);
            }
        };
        let mutation = CredentialPoolMutationV1::from_registered_planner(
            pending.kind,
            pending.current.as_ref(),
            desired,
        )?;
        self.control = json!({"credential_pool_mutation": &mutation});
        self.credential_pool = Some(mutation);
        Ok(())
    }
}

#[derive(serde::Serialize)]
struct SecretPreviewConsentV1<'a> {
    credential_id: &'a str,
    mutation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<&'a CanonicalDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a ProtectedInputSourceDescriptorV1>,
}

pub(crate) struct PreparedPreview {
    pub(crate) result: PreviewResultV1,
    pub(crate) plan: TransactionPlanV1,
}

#[derive(serde::Serialize)]
struct PreviewEffect {
    effect_id: String,
    channel: String,
    action: String,
    target: String,
}

pub(crate) fn prepare_preview<S, R, E, I, O>(
    secrets: &S,
    runtime: &R,
    external: &E,
    protected_inputs: &I,
    connection_options: &O,
    request: PreviewRequestV1,
    revisions: RevisionSetV1,
) -> Result<PreparedPreview, ChangePreparationError>
where
    S: SecretStorePort,
    R: RuntimeStatePort,
    E: ExternalEffectPort,
    I: ProtectedInputPort,
    O: ConnectionOptionAuthorizationPort,
{
    let descriptor = command_by_id(&request.spec.command_id)
        .filter(|descriptor| descriptor.kind == CommandKind::Apply)
        .ok_or(ChangePreparationError::UnsupportedCommand)?;

    // This must precede planning, Secret reads, network probes, and every durable write.
    validate_feature_gate(&request.spec.desired_state)?;
    let mut desired = registered_plan(
        connection_options,
        &request.spec.command_id,
        request.spec.desired_state.clone(),
    )?;

    desired.secrets.sort_by(|left, right| {
        left.credential()
            .credential_id()
            .cmp(right.credential().credential_id())
    });
    let mut credential_ids = BTreeSet::new();
    for mutation in &mut desired.secrets {
        let credential = mutation.credential();
        if !credential_ids.insert(credential.credential_id().to_owned()) {
            return Err(ChangePreparationError::DuplicateCredentialId);
        }
        if secrets.generation(credential)? != mutation.expected_generation() {
            return Err(ChangePreparationError::SecretGenerationChanged);
        }
        match mutation.kind() {
            hiroute_domain::SecretMutationKind::Upsert => {
                let input_slot = mutation
                    .input_slot()
                    .ok_or(ChangePreparationError::SecretInputRequired)?;
                validate_input_slot(input_slot)?;
                let protected = protected_inputs.read_secret(input_slot)?;
                let fingerprint = secrets.fingerprint(&protected)?;
                if mutation
                    .fingerprint()
                    .is_some_and(|expected| expected != &fingerprint)
                {
                    return Err(ChangePreparationError::SecretFingerprintMismatch);
                }
                mutation.bind_fingerprint(fingerprint);
            }
            hiroute_domain::SecretMutationKind::Delete => {
                if mutation.input_slot().is_some() || mutation.fingerprint().is_some() {
                    return Err(ChangePreparationError::UnexpectedSecretInput);
                }
            }
            hiroute_domain::SecretMutationKind::Rebind => {
                return Err(ChangePreparationError::UnexpectedSecretInput);
            }
        }
    }
    desired.finalize_credential_pool()?;

    desired
        .runtime
        .sort_by(|left, right| left.key().cmp(right.key()));
    let mut runtime_keys = BTreeSet::new();
    for mutation in &desired.runtime {
        if !runtime_keys.insert(mutation.key().to_owned()) {
            return Err(ChangePreparationError::DuplicateRuntimeKey);
        }
        if runtime.generation(mutation.key())? != mutation.expected_generation() {
            return Err(ChangePreparationError::RuntimeGenerationChanged);
        }
    }

    desired
        .external
        .sort_by(|left, right| left.effect_id().cmp(right.effect_id()));
    let mut effect_ids = BTreeSet::new();
    for effect in &mut desired.external {
        if !effect_ids.insert(effect.effect_id().to_owned()) {
            return Err(ChangePreparationError::DuplicateEffectId);
        }
        let observed = external.current_external_fingerprint(effect.target())?;
        if effect
            .before_fingerprint()
            .is_some_and(|expected| Some(expected) != observed.as_ref())
        {
            return Err(ChangePreparationError::ExternalEffectChanged);
        }
        effect.set_before_fingerprint(observed);
    }

    let control_target = request
        .spec
        .resource_id
        .clone()
        .unwrap_or_else(|| "personal/default".to_owned());
    let mut spec = request.spec;
    // Normalize only the typed, non-secret command input. Internal effects never round-trip into
    // the public ChangeSpec/digest surface.
    spec.desired_state = canonicalize_json(spec.desired_state);
    let normalized = normalize_change(spec, revisions.clone())?;
    let fallback_source = ProtectedInputSourceDescriptorV1::ManualInput;
    let secret_consents = desired
        .secrets
        .iter()
        .map(|mutation| SecretPreviewConsentV1 {
            credential_id: mutation.credential().credential_id(),
            mutation: match mutation.kind() {
                hiroute_domain::SecretMutationKind::Upsert => "upsert",
                hiroute_domain::SecretMutationKind::Delete => "delete",
                hiroute_domain::SecretMutationKind::Rebind => "rebind",
            },
            fingerprint: mutation.fingerprint(),
            source: (mutation.kind() == hiroute_domain::SecretMutationKind::Upsert).then(|| {
                desired
                    .secret_sources
                    .get(mutation.credential().credential_id())
                    .unwrap_or(&fallback_source)
            }),
        })
        .collect::<Vec<_>>();
    let consent_digest = CanonicalDigest::of(&(
        "hiroute.change-preview-consent/v1",
        &normalized.digest,
        &secret_consents,
    ))
    .map_err(|_| ChangePreparationError::ConsentDigest)?;
    let effects = preview_effects(&desired, &control_target);
    let plan = if let Some(source) = desired.pending_compute_source {
        TransactionPlanV1::from_compute_source_planner(
            normalized.spec.clone(),
            source.current.as_ref(),
            source.desired,
            &source.registry,
            source.explicit_materialization,
        )?
    } else {
        TransactionPlanV1::from_registered_typed_planner(
            normalized.spec.clone(),
            desired.control,
            desired.credential_pool,
            desired.secrets,
            desired.runtime,
            desired.external,
        )?
    };
    // PROCESS-25001 froze `PreviewResultV1` but did not publicly re-export its effect item type.
    let result: PreviewResultV1 = serde_json::from_value(json!({
        "normalized_spec": normalized.spec,
        "change_digest": consent_digest,
        "expected_revisions": revisions,
        "effects": effects,
        "blockers": [],
    }))?;
    debug_assert_eq!(descriptor.command_id, result.normalized_spec.command_id);
    Ok(PreparedPreview { result, plan })
}

fn registered_plan<O: ConnectionOptionAuthorizationPort>(
    connection_options: &O,
    command_id: &str,
    desired_state: Value,
) -> Result<RegisteredEffectPlan, ChangePreparationError> {
    match command_id {
        "setup.apply" => plan_setup(connection_options, serde_json::from_value(desired_state)?),
        "compute.connection.apply" => {
            compute::plan_connection(connection_options, serde_json::from_value(desired_state)?)
        }
        "compute.credential.add" | "compute.credential.replace" => compute::plan_credential(
            connection_options,
            command_id,
            serde_json::from_value(desired_state)?,
            false,
        ),
        "compute.credential.remove" => compute::plan_credential(
            connection_options,
            command_id,
            serde_json::from_value(desired_state)?,
            true,
        ),
        "compute.key-pool.apply" => {
            compute::plan_key_pool(connection_options, serde_json::from_value(desired_state)?)
        }
        "prices.override.apply" => {
            compute::plan_price_override(connection_options, serde_json::from_value(desired_state)?)
        }
        "routing.classifier.secret.apply" => {
            plan_classifier_header_secret(serde_json::from_value(desired_state)?)
        }
        // Formal command-specific DTO/handler adapters are owned by later PROCESSes. Unknown
        // commands fail closed instead of falling through a generic Value effects DSL.
        _ => Err(ChangePreparationError::TypedPlannerUnavailable),
    }
}

fn plan_classifier_header_secret(
    input: ClassifierHeaderSecretInputV1,
) -> Result<RegisteredEffectPlan, ChangePreparationError> {
    validate_input_slot(&input.input_slot)?;
    let credential = CredentialRefV1::new(
        input.secret_id,
        "personal/default",
        "hirouted",
        "http-header",
        std::iter::empty(),
        input.expected_generation,
    )?;
    let mutation = SecretMutationV1::upsert(
        credential,
        input.expected_generation,
        input.input_slot,
        None,
    )?;
    Ok(RegisteredEffectPlan {
        control: json!({"classifier_header_secret": mutation.credential().credential_id()}),
        pending_compute_source: None,
        credential_pool: None,
        pending_pool: None,
        secret_sources: Default::default(),
        secrets: vec![mutation],
        runtime: Vec::new(),
        external: Vec::new(),
    })
}

fn plan_setup<O: ConnectionOptionAuthorizationPort>(
    connection_options: &O,
    input: SetupPlannerInputV1,
) -> Result<RegisteredEffectPlan, ChangePreparationError> {
    let authorization =
        registered_option_authorization(connection_options, &input.connection_option_id)?;
    if input.secret.is_some() && !authorization.accepts_native_secret {
        return Err(ChangePreparationError::SecretNotAcceptedByConnectionOption);
    }
    let secret = input
        .secret
        .map(|secret| {
            validate_input_slot(&secret.input_slot)?;
            let credential = CredentialRefV1::new(
                format!("credential/{}", input.connection_option_id),
                format!("connection/{}", input.connection_option_id),
                "hirouted",
                "provider-auth",
                ["provider-api".to_owned()],
                secret.expected_generation,
            )?;
            SecretMutationV1::upsert(
                credential,
                secret.expected_generation,
                secret.input_slot,
                None,
            )
            .map_err(ChangePreparationError::from)
        })
        .transpose()?;
    Ok(RegisteredEffectPlan {
        control: json!({"connection_option_id": input.connection_option_id}),
        pending_compute_source: None,
        credential_pool: None,
        pending_pool: None,
        secret_sources: Default::default(),
        secrets: secret.into_iter().collect(),
        runtime: vec![RuntimeMutationV1::from_registered_planner(
            "active/setup",
            json!({"ready": true}),
            input.runtime_expected_generation,
        )?],
        external: vec![
            ExternalEffectIntentV1::from_registered_adapter(
                "publication-setup",
                OwnedEffectKind::Publication,
                "publication/current",
                None,
                json!({"setup": "active"}),
                0o644,
                false,
            )?,
            ExternalEffectIntentV1::from_registered_adapter(
                "agent-setup",
                OwnedEffectKind::AgentArtifact,
                "agents/codex",
                None,
                json!({"configured": true}),
                0o640,
                false,
            )?,
        ],
    })
}

fn preview_effects(desired: &RegisteredEffectPlan, control_target: &str) -> Vec<PreviewEffect> {
    let mut effects = vec![PreviewEffect {
        effect_id: format!("control:{control_target}"),
        channel: "control_database".to_owned(),
        action: "replace_desired".to_owned(),
        target: control_target.to_owned(),
    }];
    effects.extend(desired.secrets.iter().map(|secret| {
        PreviewEffect {
            effect_id: format!("secret:{}", secret.credential().credential_id()),
            channel: "secret_store".to_owned(),
            action: match secret.kind() {
                hiroute_domain::SecretMutationKind::Upsert => "upsert",
                hiroute_domain::SecretMutationKind::Delete => "delete",
                hiroute_domain::SecretMutationKind::Rebind => "rebind",
            }
            .to_owned(),
            target: secret.credential().credential_id().to_owned(),
        }
    }));
    effects.extend(desired.external.iter().map(|effect| {
        PreviewEffect {
            effect_id: effect.effect_id().to_owned(),
            channel: match effect.kind() {
                OwnedEffectKind::Publication => "publication",
                OwnedEffectKind::AgentArtifact => "agent_artifacts",
                _ => unreachable!("constructor restricts external effect kind"),
            }
            .to_owned(),
            action: "replace_owned".to_owned(),
            target: effect.target().to_owned(),
        }
    }));
    effects.extend(desired.runtime.iter().map(|mutation| PreviewEffect {
        effect_id: format!("runtime:{}", mutation.key()),
        channel: "runtime_state".to_owned(),
        action: "compare_and_swap".to_owned(),
        target: mutation.key().to_owned(),
    }));
    effects
}

fn validate_identifier(value: &str) -> Result<(), ChangePreparationError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b':' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(ChangePreparationError::InvalidIdentifier)
    }
}

fn registered_option_authorization<O: ConnectionOptionAuthorizationPort>(
    connection_options: &O,
    value: &str,
) -> Result<ConnectionOptionAuthorizationV1, ChangePreparationError> {
    validate_identifier(value)?;
    connection_options
        .connection_option_authorization(value)?
        .ok_or(ChangePreparationError::UnknownConnectionOption)
}

fn validate_input_slot(value: &str) -> Result<(), ChangePreparationError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        });
    if valid {
        Ok(())
    } else {
        Err(ChangePreparationError::InvalidIdentifier)
    }
}

fn disabled_token(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "budgetedpaid"
            | "paidbudgetspec"
            | "budgetquote"
            | "budgetlease"
            | "cashbudget"
            | "cashbudgetlimit"
            | "debt"
            | "externalsource"
    )
}

fn contains_disabled_input(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            disabled_token(key)
                || contains_disabled_input(value)
                || value
                    .as_str()
                    .is_some_and(|string| disabled_token(string) || looks_like_url(string))
        }),
        Value::Array(values) => values.iter().any(contains_disabled_input),
        Value::String(value) => disabled_token(value) || looks_like_url(value),
        _ => false,
    }
}

fn looks_like_url(value: &str) -> bool {
    value.contains("://") || value.starts_with("//")
}

pub(crate) fn validate_feature_gate(value: &Value) -> Result<(), ChangePreparationError> {
    if contains_disabled_input(value) {
        Err(ChangePreparationError::FeatureNotEnabled)
    } else {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ChangePreparationError {
    #[error("ChangeSpec command_id is not a frozen Apply command")]
    UnsupportedCommand,
    #[error("the command has no registered typed planner in this MVP composition")]
    TypedPlannerUnavailable,
    #[error("cash budget, ExternalSource, and arbitrary URL inputs are not enabled in P0")]
    FeatureNotEnabled,
    #[error("typed command desired_state is invalid: {0}")]
    InvalidDesiredState(#[from] serde_json::Error),
    #[error("transaction identifier is invalid")]
    InvalidIdentifier,
    #[error("connection_option_id is not registered in this P0 Release schema")]
    UnknownConnectionOption,
    #[error("the exact Source does not match the verified Connection Option")]
    SourceIdentityMismatch,
    #[error("paid/subscription connection requires explicit materialization")]
    ExplicitMaterializationRequired,
    #[error("this Connection Option does not accept a native Secret")]
    SecretNotAcceptedByConnectionOption,
    #[error("price override target is not present in the verified ModelData slice")]
    UnknownPriceTarget,
    #[error("upsert requires a protected logical input slot")]
    SecretInputRequired,
    #[error("a Secret source or fingerprint was supplied for a delete")]
    UnexpectedSecretInput,
    #[error("protected Secret no longer matches the Preview fingerprint")]
    SecretFingerprintMismatch,
    #[error("Secret generation changed since Preview")]
    SecretGenerationChanged,
    #[error("runtime generation changed since Preview")]
    RuntimeGenerationChanged,
    #[error("Secret mutation credential IDs must be unique")]
    DuplicateCredentialId,
    #[error("runtime mutation keys must be unique")]
    DuplicateRuntimeKey,
    #[error("external effect identifiers must be unique")]
    DuplicateEffectId,
    #[error("managed artifact changed since Preview")]
    ExternalEffectChanged,
    #[error("CredentialPool projection is missing")]
    CredentialPoolNotFound,
    #[error("CredentialPool revision changed since Preview")]
    CredentialPoolChanged,
    #[error("typed CredentialPool transition is invalid")]
    InvalidCredentialPool,
    #[error("protected input consent digest cannot be computed")]
    ConsentDigest,
    #[error(transparent)]
    ComputeContract(#[from] hiroute_domain::ComputeContractError),
    #[error(transparent)]
    DomainChange(#[from] hiroute_domain::ChangeValidationError),
    #[error(transparent)]
    DomainOperation(#[from] hiroute_domain::OperationValidationError),
    #[error(transparent)]
    Port(#[from] hiroute_domain::PortError),
}
