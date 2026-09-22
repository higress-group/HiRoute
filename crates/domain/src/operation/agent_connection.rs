use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;
mod validation;
pub(super) use validation::{
    agent_access_grants_from_control, validate_external_components, validate_plan,
};
use validation::{
    registered_payload, validate_agent_access_grants, validate_agent_change_spec,
    validate_payload_value,
};
mod settings;
mod settings_publication;
pub use settings_publication::{
    settings_model_publication_intent, settings_model_publication_record,
    validate_settings_model_publication_intent,
};

const CONTROL_SCHEMA: &str = "hiroute.agent-connection-control/v1";
const EFFECT_SCHEMA: &str = "hiroute.agent-connection-effect/v1";
const MAX_REGISTERED_PAYLOAD_BYTES: usize = 1_048_576;

/// The registered AgentConnection commands that may enter the generic transaction engine.
///
/// This enum is intentionally only a planner-side value. It has no `Deserialize` implementation,
/// so a wire request cannot select a transaction variant without a registered typed handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConnectionTransactionKindV1 {
    Apply,
    Restore,
    Settings,
}

impl AgentConnectionTransactionKindV1 {
    pub const fn command_id(self) -> &'static str {
        match self {
            Self::Apply => "agents.connect.apply",
            Self::Restore => "agents.restore.apply",
            Self::Settings => "agents.settings.apply",
        }
    }

    fn parse(value: &str) -> Result<Self, OperationValidationError> {
        match value {
            "apply" => Ok(Self::Apply),
            "restore" => Ok(Self::Restore),
            "settings" => Ok(Self::Settings),
            _ => Err(OperationValidationError::UnregisteredEffectPlan),
        }
    }
}

/// Exact, registry-resolved Agent/profile identity shared by control and every owned effect.
/// Private fields and the absence of `Deserialize` prevent caller-supplied JSON from becoming an
/// effects-plan identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentConnectionTransactionSubjectV1 {
    agent_id: String,
    profile_id: String,
    integration_profile_ref: String,
}

impl AgentConnectionTransactionSubjectV1 {
    pub fn from_registered_profile(
        agent_id: impl Into<String>,
        profile_id: impl Into<String>,
        integration_profile_ref: impl Into<String>,
    ) -> Result<Self, OperationValidationError> {
        let subject = Self {
            agent_id: agent_id.into(),
            profile_id: profile_id.into(),
            integration_profile_ref: integration_profile_ref.into(),
        };
        validate_scope_identifier(&subject.agent_id)?;
        validate_scope_identifier(&subject.profile_id)?;
        validate_scope_identifier(&subject.integration_profile_ref)?;
        Ok(subject)
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn integration_profile_ref(&self) -> &str {
        &self.integration_profile_ref
    }
}

/// Server-owned control mutation for an AgentConnection transaction.
///
/// `payload` is produced by the downstream command-specific typed planner. It remains private,
/// is bound to the exact public ChangeSpec, and is checked for secret-bearing fields before it can
/// enter the durable journal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentConnectionControlIntentV1 {
    schema: &'static str,
    transaction: AgentConnectionTransactionKindV1,
    subject: AgentConnectionTransactionSubjectV1,
    change_spec_digest: CanonicalDigest,
    payload_digest: CanonicalDigest,
    payload: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    agent_access_grants: Vec<AgentAccessGrantMutationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_access_grants_digest: Option<CanonicalDigest>,
}

impl AgentConnectionControlIntentV1 {
    pub fn from_registered_planner<T: Serialize>(
        transaction: AgentConnectionTransactionKindV1,
        subject: AgentConnectionTransactionSubjectV1,
        spec: &ChangeSpecV1,
        payload: &T,
    ) -> Result<Self, OperationValidationError> {
        if spec.command_id != transaction.command_id() {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        validate_agent_change_spec(spec)?;
        let payload = registered_payload(payload)?;
        Ok(Self {
            schema: CONTROL_SCHEMA,
            transaction,
            subject,
            change_spec_digest: CanonicalDigest::of(spec)?,
            payload_digest: CanonicalDigest::of(&payload)?,
            payload,
            agent_access_grants: Vec::new(),
            agent_access_grants_digest: None,
        })
    }

    pub const fn transaction(&self) -> AgentConnectionTransactionKindV1 {
        self.transaction
    }

    pub fn subject(&self) -> &AgentConnectionTransactionSubjectV1 {
        &self.subject
    }

    pub fn change_spec_digest(&self) -> &CanonicalDigest {
        &self.change_spec_digest
    }

    pub fn payload_digest(&self) -> &CanonicalDigest {
        &self.payload_digest
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    fn bind_agent_access_grants(
        &mut self,
        spec: &ChangeSpecV1,
        mutations: Vec<AgentAccessGrantMutationV1>,
    ) -> Result<(), OperationValidationError> {
        validate_agent_access_grants(spec, self.transaction, &mutations)?;
        self.agent_access_grants_digest = (!mutations.is_empty())
            .then(|| CanonicalDigest::of(&mutations))
            .transpose()?;
        self.agent_access_grants = mutations;
        Ok(())
    }
}

/// Exact external ownership slots available to the AgentConnection typed handler.
///
/// Grant and AccessPoint facts stay in one aggregate publication effect so activation cannot
/// expose a grant/publication split. Native-routing artifacts are an all-or-none trio; the spawn
/// guidance rewrite remains an optional, separately owned optimization.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConnectionEffectRoleV1 {
    GrantScopedPublication,
    ManagedConfiguration,
    ModelCatalog,
    RoutingSkill,
    InstructionOverlay,
    SpawnGuidanceRewrite,
    /// The host-executed resident login item for the first managed connection. Unlike the
    /// artifact roles, the action runs in the Desktop host under the native confirmation; the
    /// Operation records the checked before/after state and the host compensates creations.
    LoginItem,
}

impl AgentConnectionEffectRoleV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GrantScopedPublication => "grant_scoped_publication",
            Self::ManagedConfiguration => "managed_configuration",
            Self::ModelCatalog => "model_catalog",
            Self::RoutingSkill => "routing_skill",
            Self::InstructionOverlay => "instruction_overlay",
            Self::SpawnGuidanceRewrite => "spawn_guidance_rewrite",
            Self::LoginItem => "login_item",
        }
    }

    /// Derives the immutable ownership target used by the registered renderer. Adapters use
    /// this only to observe the exact pre-Preview fingerprint; the sealed planner still fixes
    /// every target when it creates the effect intent.
    pub fn target_for(
        self,
        subject: &AgentConnectionTransactionSubjectV1,
    ) -> Result<String, OperationValidationError> {
        effect_target(subject, self)
    }

    pub fn settings_target_for(
        self,
        subject: &AgentConnectionTransactionSubjectV1,
    ) -> Result<String, OperationValidationError> {
        effect_target(subject, self)
    }

    /// The payload digest also names a settings catalog artifact, so fact capture can observe
    /// that exact existing target before Preview without inventing a second path convention.
    pub fn settings_payload_target_for(
        self,
        subject: &AgentConnectionTransactionSubjectV1,
        payload_digest: &CanonicalDigest,
    ) -> Result<String, OperationValidationError> {
        let target = self.settings_target_for(subject)?;
        if self == Self::ModelCatalog {
            return Ok(format!("{target}/{}", &payload_digest.as_str()[7..]));
        }
        Ok(target)
    }

    const fn effect_id(self) -> &'static str {
        match self {
            Self::GrantScopedPublication => "agent-connection-grant-publication",
            Self::ManagedConfiguration => "agent-connection-managed-configuration",
            Self::ModelCatalog => "agent-connection-model-catalog",
            Self::RoutingSkill => "agent-connection-routing-skill",
            Self::InstructionOverlay => "agent-connection-instruction-overlay",
            Self::SpawnGuidanceRewrite => "agent-connection-spawn-guidance",
            Self::LoginItem => "agent-connection-login-item",
        }
    }

    const fn target_name(self) -> &'static str {
        match self {
            Self::GrantScopedPublication => "publication/current",
            Self::ManagedConfiguration => "managed-configuration",
            Self::ModelCatalog => "model-catalog",
            Self::RoutingSkill => "routing-skill",
            Self::InstructionOverlay => "instruction-overlay",
            Self::SpawnGuidanceRewrite => "spawn-guidance",
            Self::LoginItem => "login-item",
        }
    }

    const fn owned_kind(self) -> OwnedEffectKind {
        match self {
            Self::GrantScopedPublication => OwnedEffectKind::Publication,
            Self::ManagedConfiguration
            | Self::ModelCatalog
            | Self::RoutingSkill
            | Self::InstructionOverlay
            | Self::SpawnGuidanceRewrite => OwnedEffectKind::AgentArtifact,
            Self::LoginItem => OwnedEffectKind::LoginItem,
        }
    }

    fn parse(value: &str) -> Result<Self, OperationValidationError> {
        match value {
            "grant_scoped_publication" => Ok(Self::GrantScopedPublication),
            "managed_configuration" => Ok(Self::ManagedConfiguration),
            "model_catalog" => Ok(Self::ModelCatalog),
            "routing_skill" => Ok(Self::RoutingSkill),
            "instruction_overlay" => Ok(Self::InstructionOverlay),
            "spawn_guidance_rewrite" => Ok(Self::SpawnGuidanceRewrite),
            "login_item" => Ok(Self::LoginItem),
            _ => Err(OperationValidationError::UnregisteredEffectPlan),
        }
    }
}

/// The settings-transaction client model file: the only B effect whose activation follows the
/// gateway publication, so a client file is never switched before its service is serving.
pub fn is_settings_managed_configuration(intent: &ExternalEffectIntentV1) -> bool {
    intent.kind() == OwnedEffectKind::AgentArtifact
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["role"] == "managed_configuration"
}

/// The settings-transaction model publication: B's service segment.
pub fn is_settings_publication(intent: &ExternalEffectIntentV1) -> bool {
    intent.kind() == OwnedEffectKind::Publication && intent.desired()["transaction"] == "settings"
}

pub const SETTINGS_SERVICE_COMPLETION_SCHEMA: &str = "hiroute.settings-service-completion/v1";

/// Durable proof that a settings operation's service segment (secrets, control, runtime,
/// non-client artifacts, publication and protected revocations) finished. It authorizes the
/// client-file tail to be retried or parked without replaying the service segment, and never
/// authorizes an expired publication to keep serving.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsServiceCompletionV1 {
    pub schema: String,
    pub publication_revision: u64,
    pub publication_digest: CanonicalDigest,
    pub completed_effects_digest: CanonicalDigest,
}

impl SettingsServiceCompletionV1 {
    pub fn parse(value: &str) -> Option<Self> {
        serde_json::from_str(value)
            .ok()
            .filter(|receipt: &Self| receipt.schema == SETTINGS_SERVICE_COMPLETION_SCHEMA)
    }
}

#[derive(Serialize)]
struct AgentConnectionEffectEnvelopeV1<'a> {
    schema: &'static str,
    transaction: AgentConnectionTransactionKindV1,
    subject: &'a AgentConnectionTransactionSubjectV1,
    change_spec_digest: &'a CanonicalDigest,
    role: AgentConnectionEffectRoleV1,
    payload_digest: CanonicalDigest,
    payload: Value,
}

impl ExternalEffectIntentV1 {
    /// Builds one exact field-owned AgentConnection effect from a registered typed renderer.
    /// The caller cannot choose an effect ID, ownership kind, target, or sensitivity bit.
    pub fn from_agent_connection_planner<T: Serialize>(
        control: &AgentConnectionControlIntentV1,
        role: AgentConnectionEffectRoleV1,
        before_fingerprint: Option<CanonicalDigest>,
        payload: &T,
        desired_mode: u32,
    ) -> Result<Self, OperationValidationError> {
        if !matches!(desired_mode, 0o600 | 0o640 | 0o644)
            || (role == AgentConnectionEffectRoleV1::GrantScopedPublication
                && desired_mode != 0o644)
        {
            return Err(OperationValidationError::InvalidArtifactMode);
        }
        let payload = registered_payload(payload)?;
        let envelope = AgentConnectionEffectEnvelopeV1 {
            schema: EFFECT_SCHEMA,
            transaction: control.transaction,
            subject: &control.subject,
            change_spec_digest: &control.change_spec_digest,
            role,
            payload_digest: CanonicalDigest::of(&payload)?,
            payload,
        };
        let desired = crate::canonicalize_json(serde_json::to_value(&envelope)?);
        let target = if control.transaction == AgentConnectionTransactionKindV1::Settings {
            role.settings_payload_target_for(&control.subject, &envelope.payload_digest)?
        } else {
            effect_target(&control.subject, role)?
        };
        let intent = Self {
            content_publication: None,
            effect_id: role.effect_id().to_owned(),
            kind: role.owned_kind(),
            target,
            before_fingerprint,
            desired: desired.into(),
            desired_mode,
            sensitive: false,
        };
        validate_external_components(
            &intent.effect_id,
            intent.kind,
            &intent.target,
            &intent.desired,
            intent.desired_mode,
            intent.sensitive,
        )?;
        Ok(intent)
    }
}

impl TransactionPlanV1 {
    /// Sealed registration seam for PROCESS-25005's command-specific typed handler.
    ///
    /// The handler supplies a control intent and exact owned effects; this constructor fixes all
    /// other mutation channels to empty and enforces the AgentConnection effect-set invariant.
    pub fn from_agent_connection_planner(
        spec: ChangeSpecV1,
        control: AgentConnectionControlIntentV1,
        external: Vec<ExternalEffectIntentV1>,
    ) -> Result<Self, OperationValidationError> {
        Self::from_agent_connection_planner_with_agent_access_grants(
            spec,
            control,
            Vec::new(),
            external,
        )
    }

    /// Sealed AgentConnection plan carrying a dedicated local grant mutation. The mutation is
    /// embedded in the authenticated control envelope so durable recovery reconstructs one source
    /// of truth without teaching generic storage about planner-private types.
    pub fn from_agent_connection_planner_with_agent_access_grants(
        spec: ChangeSpecV1,
        mut control: AgentConnectionControlIntentV1,
        agent_access_grants: Vec<AgentAccessGrantMutationV1>,
        external: Vec<ExternalEffectIntentV1>,
    ) -> Result<Self, OperationValidationError> {
        control.bind_agent_access_grants(&spec, agent_access_grants)?;
        let grants = control.agent_access_grants.clone();
        let control = serde_json::to_value(control)?;
        validate_plan(&spec, &control, &[], &[], &external)?;
        Ok(Self {
            spec,
            control,
            credential_pool: None,
            worker_dependency_selection: None,
            secrets: Vec::new(),
            agent_access_grants: grants,
            runtime: Vec::new(),
            external,
        })
    }

    /// Extracts the exact non-secret AgentConnection state from the sealed control envelope.
    /// This is a read projection of the authenticated Operation input, not another persistence
    /// format, and is intentionally unavailable for any other command family.
    pub fn agent_connection_projection(
        &self,
    ) -> Result<Option<ActiveAgentConnectionV1>, OperationValidationError> {
        if self.spec.command_id != AgentConnectionTransactionKindV1::Apply.command_id() {
            return Ok(None);
        }
        validate_plan(
            &self.spec,
            &self.control,
            &self.secrets,
            &self.runtime,
            &self.external,
        )?;
        let control = decode_control(&self.control)?;
        let subject = decode_subject(control.subject)?;
        let payload: DurableAgentConnectionPayloadV1 = serde_json::from_value(control.payload)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        payload
            .connection
            .validate()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        let desired = self
            .spec
            .desired_state
            .as_object()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        let connection_id = self
            .spec
            .resource_id
            .clone()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        let installed_version = desired
            .get("installed_version")
            .and_then(Value::as_str)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?
            .to_owned();
        let desired_digest = |field: &str| {
            desired
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| CanonicalDigest::parse(value.to_owned()).ok())
                .ok_or(OperationValidationError::UnregisteredEffectPlan)
        };
        let observation_digest = desired_digest("observation_digest")?;
        if connection_id
            != format!(
                "agent-connection/{}/{}",
                payload.connection.agent_id, payload.connection.profile_id
            )
            || subject.agent_id() != payload.connection.agent_id
            || subject.profile_id() != payload.connection.profile_id
            || subject.integration_profile_ref() != payload.connection.integration_profile_ref
            || desired.get("agent_id").and_then(Value::as_str)
                != Some(payload.connection.agent_id.as_str())
            || desired.get("profile_id").and_then(Value::as_str)
                != Some(payload.connection.profile_id.as_str())
            || desired
                .get("integration_profile_ref")
                .and_then(Value::as_str)
                != Some(payload.connection.integration_profile_ref.as_str())
            || desired_digest("grant_digest")? != payload.connection.grant.digest
            || desired_digest("publication_digest")? != payload.publication_digest
            || desired_digest("config_change_digest")? != payload.config_change_digest
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(Some(ActiveAgentConnectionV1 {
            connection_id,
            installed_version,
            observation_digest,
            source_publication_digest: payload.publication_digest,
            connection: payload.connection,
        }))
    }
}

/// Durable non-secret view used by daemon status, descriptor, and raw-grant joins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveAgentConnectionV1 {
    pub connection_id: String,
    pub installed_version: String,
    pub observation_digest: CanonicalDigest,
    pub source_publication_digest: CanonicalDigest,
    pub connection: crate::AgentConnectionV1,
}

/// Materializes the one product publication owned by an AgentConnection Operation after the
/// SecretStore has produced its authenticated, non-secret grant reference.
pub fn agent_connection_publication_record(
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
    active_record: &crate::PublicationRecordV1,
    grant_effect: &OwnedEffectV1,
) -> Result<Option<crate::PublicationRecordV1>, OperationValidationError> {
    if intent.effect_id != AgentConnectionEffectRoleV1::GrantScopedPublication.effect_id() {
        return Ok(None);
    }
    if !operation
        .plan
        .external
        .iter()
        .any(|registered| registered == intent)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let projection = operation
        .plan
        .agent_connection_projection()?
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    let envelope = decode_effect(&intent.desired)?;
    if AgentConnectionEffectRoleV1::parse(&envelope.role)?
        != AgentConnectionEffectRoleV1::GrantScopedPublication
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload: DurableGrantPublicationPayloadV1 = serde_json::from_value(envelope.payload)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let active = active_record
        .verify()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let active_digest = active
        .digest()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let [mutation] = operation.plan.agent_access_grants.as_slice() else {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    };
    let reference = AgentAccessGrantRefV1::from_ensure_effect(grant_effect, mutation)?;
    let scope = reference.scope();
    let model_grant = crate::AgentModelGrantV2::from_plan_ids(
        projection.connection.protocol,
        projection.connection.grant.allowed_agent_plan_ids.clone(),
        &active,
    )
    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if payload.connection != projection.connection
        || payload.publication_digest != projection.source_publication_digest
        || payload.publication_digest != active_digest
        || intent.before_fingerprint.as_ref() != Some(&active_digest)
        || payload.access_point_ref != "access-point/local/default"
        || reference.owner_scope() != operation.workspace_id.as_str()
        || reference.connection_id() != projection.connection_id
        || scope.protocol() != projection.connection.protocol
        || scope.model_grant() != &model_grant
        || operation.plan.spec().desired_state["model_grant_digest"]
            != serde_json::to_value(&model_grant.digest)?
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let exact_active_grant = active.grants.iter().any(|grant| {
        grant.grant_id == reference.grant_id()
            && grant.generation == reference.generation()
            && grant.bearer_token_sha256 == *reference.material_sha256()
            && grant.model_grant == model_grant
    });
    if exact_active_grant {
        return Ok(Some(active_record.clone()));
    }
    let access_grant = crate::GatewayAccessGrantV1::new(
        reference.grant_id(),
        reference.generation(),
        reference.material_sha256().clone(),
        scope.protocol(),
        model_grant,
    )
    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let revision = crate::GatewayPublicationRevision::new(
        active
            .publication_revision
            .get()
            .checked_add(1)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
    )
    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let publication = active
        .next_with_access_grant(revision, access_grant)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    crate::PublicationRecordV1::from_publication(operation.workspace_id.clone(), &publication)
        .map(Some)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)
}

/// Materializes the publication side of an exact AgentConnection restore. The caller supplies
/// the non-secret reference reconstructed from the last succeeded Apply journal; the restore
/// mutation and active publication must bind every field before this removes its verifier.
pub fn agent_connection_restore_publication_record(
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
    active_record: &crate::PublicationRecordV1,
    active_grant: &AgentAccessGrantRefV1,
) -> Result<Option<crate::PublicationRecordV1>, OperationValidationError> {
    if intent.effect_id != AgentConnectionEffectRoleV1::GrantScopedPublication.effect_id() {
        return Ok(None);
    }
    validate_plan(
        operation.plan.spec(),
        operation.plan.control(),
        operation.plan.secrets(),
        operation.plan.runtime(),
        operation.plan.external(),
    )?;
    if operation.plan.spec().command_id != AgentConnectionTransactionKindV1::Restore.command_id()
        || !operation
            .plan
            .external
            .iter()
            .any(|registered| registered == intent)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let envelope = decode_effect(&intent.desired)?;
    if AgentConnectionTransactionKindV1::parse(&envelope.transaction)?
        != AgentConnectionTransactionKindV1::Restore
        || AgentConnectionEffectRoleV1::parse(&envelope.role)?
            != AgentConnectionEffectRoleV1::GrantScopedPublication
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload: DurableGrantRestorePayloadV1 = serde_json::from_value(envelope.payload)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let active = active_record
        .verify()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let active_digest = active
        .digest()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let [mutation] = operation.plan.agent_access_grants.as_slice() else {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    };
    active_grant.validate()?;
    let scope = active_grant.scope();
    let exact_active_grant = active.grants.iter().any(|grant| {
        grant.grant_id == active_grant.grant_id()
            && grant.generation == active_grant.generation()
            && grant.bearer_token_sha256 == *active_grant.material_sha256()
            && &grant.model_grant == scope.model_grant()
    });
    if payload.role != AgentConnectionEffectRoleV1::GrantScopedPublication.as_str()
        || payload.previous_digest.is_none()
        || payload.remove_if_absent
        || intent.before_fingerprint.as_ref() != Some(&active_digest)
        || mutation.kind() != AgentAccessGrantMutationKindV1::Revoke
        || mutation.owner_scope() != operation.workspace_id.as_str()
        || mutation.connection_id() != active_grant.connection_id()
        || mutation.expected_generation() != active_grant.generation()
        || scope.connection_id() != active_grant.connection_id()
        || !exact_active_grant
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let revision = crate::GatewayPublicationRevision::new(
        active
            .publication_revision
            .get()
            .checked_add(1)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
    )
    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let publication = active
        .next_without_access_grant(revision, active_grant.grant_id())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    crate::PublicationRecordV1::from_publication(operation.workspace_id.clone(), &publication)
        .map(Some)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)
}

fn effect_target(
    subject: &AgentConnectionTransactionSubjectV1,
    role: AgentConnectionEffectRoleV1,
) -> Result<String, OperationValidationError> {
    if role == AgentConnectionEffectRoleV1::GrantScopedPublication {
        return Ok(role.target_name().to_owned());
    }
    let digest = CanonicalDigest::of(&("agent-connection-target/v1", subject))?;
    let digest = digest
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    Ok(format!(
        "agent-connections/{}/{}",
        digest,
        role.target_name()
    ))
}

fn valid_digest(digest: &CanonicalDigest) -> bool {
    CanonicalDigest::parse(digest.as_str().to_owned()).is_ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableSubject {
    agent_id: String,
    profile_id: String,
    integration_profile_ref: String,
}

fn decode_subject(
    subject: DurableSubject,
) -> Result<AgentConnectionTransactionSubjectV1, OperationValidationError> {
    AgentConnectionTransactionSubjectV1::from_registered_profile(
        subject.agent_id,
        subject.profile_id,
        subject.integration_profile_ref,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableControl {
    schema: String,
    transaction: String,
    subject: DurableSubject,
    change_spec_digest: CanonicalDigest,
    payload_digest: CanonicalDigest,
    payload: Value,
    #[serde(default)]
    agent_access_grants: Vec<AgentAccessGrantMutationV1>,
    #[serde(default)]
    agent_access_grants_digest: Option<CanonicalDigest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableAgentConnectionPayloadV1 {
    connection: crate::AgentConnectionV1,
    publication_digest: CanonicalDigest,
    config_change_digest: CanonicalDigest,
    #[serde(default, rename = "catalog_delivery")]
    _catalog_delivery: Option<crate::CatalogDeliveryV1>,
    #[serde(rename = "guidance_disposition")]
    _guidance_disposition: crate::GuidanceRewriteDispositionV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableGrantPublicationPayloadV1 {
    connection: crate::AgentConnectionV1,
    publication_digest: CanonicalDigest,
    access_point_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableGrantRestorePayloadV1 {
    #[serde(rename = "restore_point_ref")]
    _restore_point_ref: String,
    role: String,
    previous_digest: Option<CanonicalDigest>,
    remove_if_absent: bool,
}

fn decode_control(value: &Value) -> Result<DurableControl, OperationValidationError> {
    serde_json::from_value(value.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableEffect {
    schema: String,
    transaction: String,
    subject: DurableSubject,
    change_spec_digest: CanonicalDigest,
    role: String,
    payload_digest: CanonicalDigest,
    payload: Value,
}

fn decode_effect(value: &Value) -> Result<DurableEffect, OperationValidationError> {
    serde_json::from_value(value.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)
}

#[cfg(test)]
mod tests;
