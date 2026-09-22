use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{
    CanonicalDigest, ChangeSpecV1, CredentialPoolMutationKind, CredentialPoolMutationV1,
    PortResult, RevisionMismatch, RevisionSetV1, RuntimeProbeAcquireOutcomeV1,
    RuntimeProbeLeaseRequestV1, RuntimeProbeLeaseV1, RuntimeStateIdentityV1, RuntimeStateV1,
    WorkspaceId,
};

mod agent_access_grant;
mod agent_config_permission;
mod agent_connection;
mod compute;
mod compute_management;
mod routing;
mod routing_content;
mod routing_draft;
pub use routing_content::{ConsumedPlanDraftV1, PlanContentControlV2};
mod journal;
pub(crate) mod shared_input;
pub use journal::OperationJournalUpdate;
mod source_price;
mod subscription_check;
pub use agent_access_grant::{
    AGENT_ACCESS_GRANT_EFFECT_SCHEMA_V1, AgentAccessGrantMaterial,
    AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationKindV1, AgentAccessGrantMutationV1,
    AgentAccessGrantRefV1, AgentAccessGrantScopeV1, is_agent_access_grant_effect,
    valid_user_agent_token,
};
pub use agent_config_permission::AgentConfigPermissionIntentV1;
pub use agent_connection::{
    ActiveAgentConnectionV1, AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1,
    AgentConnectionTransactionKindV1, AgentConnectionTransactionSubjectV1,
    SETTINGS_SERVICE_COMPLETION_SCHEMA, SettingsServiceCompletionV1,
    agent_connection_publication_record, agent_connection_restore_publication_record,
    is_settings_managed_configuration, is_settings_publication, settings_model_publication_intent,
    settings_model_publication_record, validate_settings_model_publication_intent,
};
pub use compute::ComputeSourceMutationV1;
pub use routing::routing_publication_record;
pub use subscription_check::{
    COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2, COMPUTE_SUBSCRIPTION_EFFECT_ID_V2,
    SubscriptionCheckIntentV2, decode_subscription_check_intent, is_subscription_check_effect,
};

pub const OPERATION_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OperationId(String);

impl OperationId {
    pub fn derive(
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
        request_digest: &CanonicalDigest,
    ) -> Self {
        let input = format!(
            "operation-v1\0{workspace}\0{}\0{}\0{}\0{request_digest}",
            scope.principal, scope.operation_kind, scope.key
        );
        let digest = CanonicalDigest::of_bytes(input.as_bytes());
        Self(format!("op_{}", &digest.as_str()[7..39]))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, OperationValidationError> {
        let value = value.into();
        let valid = value.len() == 35
            && value.starts_with("op_")
            && value[3..].bytes().all(|byte| byte.is_ascii_hexdigit());
        if valid {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(OperationValidationError::InvalidOperationId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct IdempotencyScopeV1 {
    pub principal: String,
    pub operation_kind: String,
    pub key: String,
}

impl IdempotencyScopeV1 {
    pub fn new(
        principal: impl Into<String>,
        operation_kind: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, OperationValidationError> {
        let scope = Self {
            principal: principal.into(),
            operation_kind: operation_kind.into(),
            key: key.into(),
        };
        validate_scope_identifier(&scope.principal)?;
        validate_scope_identifier(&scope.operation_kind)?;
        validate_idempotency_key(&scope.key)?;
        Ok(scope)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Accepted,
    Preparing,
    ApplyingSecrets,
    MaterializingSources,
    CompilingPublication,
    ApplyingAgentArtifacts,
    Activating,
    RollingBack,
    Succeeded,
    RolledBack,
    NeedsAttention,
}

impl OperationState {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::RolledBack | Self::NeedsAttention
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Preparing => "preparing",
            Self::ApplyingSecrets => "applying_secrets",
            Self::MaterializingSources => "materializing_sources",
            Self::CompilingPublication => "compiling_publication",
            Self::ApplyingAgentArtifacts => "applying_agent_artifacts",
            Self::Activating => "activating",
            Self::RollingBack => "rolling_back",
            Self::Succeeded => "succeeded",
            Self::RolledBack => "rolled_back",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Accepted, Self::Preparing)
                | (Self::Preparing, Self::ApplyingSecrets)
                | (Self::ApplyingSecrets, Self::MaterializingSources)
                | (Self::MaterializingSources, Self::CompilingPublication)
                | (Self::CompilingPublication, Self::ApplyingAgentArtifacts)
                | (Self::ApplyingAgentArtifacts, Self::Activating)
                | (Self::Activating, Self::Succeeded)
                | (Self::Accepted, Self::RollingBack)
                | (Self::Preparing, Self::RollingBack)
                | (Self::ApplyingSecrets, Self::RollingBack)
                | (Self::MaterializingSources, Self::RollingBack)
                | (Self::CompilingPublication, Self::RollingBack)
                | (Self::ApplyingAgentArtifacts, Self::RollingBack)
                | (Self::Activating, Self::RollingBack)
                | (Self::RollingBack, Self::RolledBack)
                | (Self::RollingBack, Self::NeedsAttention)
                // An uncertain terminal rollback may be closed after a later exact recheck.
                | (Self::NeedsAttention, Self::RolledBack)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStepKind {
    Prepare,
    ApplySecrets,
    MaterializeSources,
    CompilePublication,
    ApplyAgentArtifacts,
    Activate,
}

impl OperationStepKind {
    pub const ALL: [Self; 6] = [
        Self::Prepare,
        Self::ApplySecrets,
        Self::MaterializeSources,
        Self::CompilePublication,
        Self::ApplyAgentArtifacts,
        Self::Activate,
    ];

    pub const fn state(self) -> OperationState {
        match self {
            Self::Prepare => OperationState::Preparing,
            Self::ApplySecrets => OperationState::ApplyingSecrets,
            Self::MaterializeSources => OperationState::MaterializingSources,
            Self::CompilePublication => OperationState::CompilingPublication,
            Self::ApplyAgentArtifacts => OperationState::ApplyingAgentArtifacts,
            Self::Activate => OperationState::Activating,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStepStatus {
    Pending,
    Started,
    Applied,
    Compensating,
    Compensated,
    Attention,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedEffectKind {
    Control,
    Secret,
    RuntimeState,
    Publication,
    AgentArtifact,
    /// Host-executed resident-service registration. The Desktop host performs the action under
    /// the native confirmation; this side only owns the durable evidence and compensation
    /// decision, so daemon-side compensation is a deliberate no-op.
    LoginItem,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OwnedEffectV1 {
    pub effect_id: String,
    pub kind: OwnedEffectKind,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_fingerprint: Option<CanonicalDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_fingerprint: Option<CanonicalDigest>,
    /// Bounded, non-secret adapter metadata. Secret ciphertext and source locators are forbidden.
    #[serde(default, with = "shared_input")]
    pub compensation: Arc<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OperationStepV1 {
    pub sequence: u16,
    pub kind: OperationStepKind,
    pub deterministic_input_digest: CanonicalDigest,
    pub status: OperationStepStatus,
    pub attempts: u32,
    #[serde(default)]
    pub effects: Vec<OwnedEffectV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_result: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretMutationKind {
    Upsert,
    Delete,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretFingerprintAlgorithm {
    #[default]
    HmacSha256V1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRefV1 {
    credential_id: String,
    owner_scope: String,
    subject: String,
    purpose: String,
    allowed_destinations: BTreeSet<String>,
    generation: u64,
}

impl CredentialRefV1 {
    pub fn new(
        credential_id: impl Into<String>,
        owner_scope: impl Into<String>,
        subject: impl Into<String>,
        purpose: impl Into<String>,
        allowed_destinations: impl IntoIterator<Item = String>,
        generation: u64,
    ) -> Result<Self, OperationValidationError> {
        let reference = Self {
            credential_id: credential_id.into(),
            owner_scope: owner_scope.into(),
            subject: subject.into(),
            purpose: purpose.into(),
            allowed_destinations: allowed_destinations.into_iter().collect(),
            generation,
        };
        validate_scope_identifier(&reference.credential_id)?;
        validate_scope_identifier(&reference.owner_scope)?;
        validate_scope_identifier(&reference.subject)?;
        validate_scope_identifier(&reference.purpose)?;
        if reference.allowed_destinations.is_empty()
            && !(reference.subject == "hirouted" && reference.purpose == "http-header")
        {
            return Err(OperationValidationError::InvalidCredentialRef);
        }
        for destination in &reference.allowed_destinations {
            validate_scope_identifier(destination)?;
        }
        Ok(reference)
    }

    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    pub fn owner_scope(&self) -> &str {
        &self.owner_scope
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn purpose(&self) -> &str {
        &self.purpose
    }

    pub fn allowed_destinations(&self) -> &BTreeSet<String> {
        &self.allowed_destinations
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Authenticated resolver identity. This type has no serde representation and therefore cannot
/// be supplied by a wire request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSecretSubjectV1 {
    subject: String,
    owner_scope: String,
}

impl VerifiedSecretSubjectV1 {
    pub fn from_authenticated_transport(
        subject: impl Into<String>,
        owner_scope: impl Into<String>,
    ) -> Result<Self, OperationValidationError> {
        let verified = Self {
            subject: subject.into(),
            owner_scope: owner_scope.into(),
        };
        validate_scope_identifier(&verified.subject)?;
        validate_scope_identifier(&verified.owner_scope)?;
        Ok(verified)
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn owner_scope(&self) -> &str {
        &self.owner_scope
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SecretMutationV1 {
    kind: SecretMutationKind,
    credential: CredentialRefV1,
    expected_generation: u64,
    #[serde(default)]
    fingerprint_algorithm: SecretFingerprintAlgorithm,
    /// Safe logical slot resolved by the protected input adapter; never a path or locator.
    #[serde(skip_serializing_if = "Option::is_none")]
    input_slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<CanonicalDigest>,
}

impl SecretMutationV1 {
    pub fn upsert(
        credential: CredentialRefV1,
        expected_generation: u64,
        input_slot: impl Into<String>,
        fingerprint: Option<CanonicalDigest>,
    ) -> Result<Self, OperationValidationError> {
        let input_slot = input_slot.into();
        validate_scope_identifier(&input_slot)?;
        Ok(Self {
            kind: SecretMutationKind::Upsert,
            credential,
            expected_generation,
            fingerprint_algorithm: SecretFingerprintAlgorithm::HmacSha256V1,
            input_slot: Some(input_slot),
            fingerprint,
        })
    }

    pub fn delete(
        credential: CredentialRefV1,
        expected_generation: u64,
    ) -> Result<Self, OperationValidationError> {
        Ok(Self {
            kind: SecretMutationKind::Delete,
            credential,
            expected_generation,
            fingerprint_algorithm: SecretFingerprintAlgorithm::HmacSha256V1,
            input_slot: None,
            fingerprint: None,
        })
    }

    pub const fn kind(&self) -> SecretMutationKind {
        self.kind
    }

    pub fn credential(&self) -> &CredentialRefV1 {
        &self.credential
    }

    pub const fn expected_generation(&self) -> u64 {
        self.expected_generation
    }

    pub const fn fingerprint_algorithm(&self) -> SecretFingerprintAlgorithm {
        self.fingerprint_algorithm
    }

    pub fn input_slot(&self) -> Option<&str> {
        self.input_slot.as_deref()
    }

    pub fn fingerprint(&self) -> Option<&CanonicalDigest> {
        self.fingerprint.as_ref()
    }

    pub fn bind_fingerprint(&mut self, fingerprint: CanonicalDigest) {
        self.fingerprint = Some(fingerprint);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeMutationV1 {
    key: String,
    value: Value,
    expected_generation: u64,
}

impl RuntimeMutationV1 {
    pub fn from_registered_planner(
        key: impl Into<String>,
        value: Value,
        expected_generation: u64,
    ) -> Result<Self, OperationValidationError> {
        let key = key.into();
        if key != "active/setup" || value != json!({"ready": true}) {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(Self {
            key,
            value,
            expected_generation,
        })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub const fn expected_generation(&self) -> u64 {
        self.expected_generation
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExternalEffectIntentV1 {
    effect_id: String,
    kind: OwnedEffectKind,
    /// Adapter-relative stable target, never an arbitrary user-supplied URL.
    target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before_fingerprint: Option<CanonicalDigest>,
    #[serde(serialize_with = "shared_input::serialize")]
    desired: Arc<Value>,
    desired_mode: u32,
    sensitive: bool,
    #[serde(skip)]
    content_publication: Option<std::sync::Arc<routing_content::ContentEffectV2>>,
}

impl PartialEq for ExternalEffectIntentV1 {
    fn eq(&self, other: &Self) -> bool {
        self.effect_id == other.effect_id
            && self.kind == other.kind
            && self.target == other.target
            && self.before_fingerprint == other.before_fingerprint
            && self.desired_mode == other.desired_mode
            && self.sensitive == other.sensitive
            && (Arc::ptr_eq(&self.desired, &other.desired) || self.desired == other.desired)
    }
}

impl ExternalEffectIntentV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_registered_adapter(
        effect_id: impl Into<String>,
        kind: OwnedEffectKind,
        target: impl Into<String>,
        before_fingerprint: Option<CanonicalDigest>,
        desired: Value,
        desired_mode: u32,
        sensitive: bool,
    ) -> Result<Self, OperationValidationError> {
        let effect_id = effect_id.into();
        let target = target.into();
        let desired = crate::canonicalize_json(desired);
        if !matches!(desired_mode, 0o600 | 0o640 | 0o644) {
            return Err(OperationValidationError::InvalidArtifactMode);
        }
        let setup_registered = match kind {
            OwnedEffectKind::Publication => {
                effect_id == "publication-setup"
                    && target == "publication/current"
                    && desired == json!({"setup": "active"})
                    && desired_mode == 0o644
                    && !sensitive
            }
            OwnedEffectKind::AgentArtifact => {
                effect_id == "agent-setup"
                    && target == "agents/codex"
                    && desired == json!({"configured": true})
                    && desired_mode == 0o640
                    && !sensitive
            }
            _ => false,
        };
        let agent_connection_registered = agent_connection::validate_external_components(
            &effect_id,
            kind,
            &target,
            &desired,
            desired_mode,
            sensitive,
        )
        .is_ok();
        let routing_registered = routing::validate_external_components(
            &effect_id,
            kind,
            &target,
            &desired,
            desired_mode,
            sensitive,
        )
        .is_ok();
        let permission_registered = agent_config_permission::validate_external_components(
            &effect_id,
            kind,
            &target,
            before_fingerprint.as_ref(),
            &desired,
            desired_mode,
            sensitive,
        )
        .is_ok();
        let subscription_registered = subscription_check::validate_external_components(
            &effect_id,
            kind,
            &target,
            before_fingerprint.as_ref(),
            &desired,
            desired_mode,
            sensitive,
        )
        .is_ok();
        if !setup_registered
            && !agent_connection_registered
            && !routing_registered
            && !permission_registered
            && !subscription_registered
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let content_publication = if routing_content::is_effect(&desired) {
            Some(std::sync::Arc::new(routing_content::decode_external(
                &effect_id,
                kind,
                &target,
                &desired,
                desired_mode,
                sensitive,
            )?))
        } else {
            None
        };
        Ok(Self {
            content_publication,
            effect_id,
            kind,
            target,
            before_fingerprint,
            desired: desired.into(),
            desired_mode,
            sensitive,
        })
    }

    pub fn effect_id(&self) -> &str {
        &self.effect_id
    }

    pub const fn kind(&self) -> OwnedEffectKind {
        self.kind
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn before_fingerprint(&self) -> Option<&CanonicalDigest> {
        self.before_fingerprint.as_ref()
    }

    pub fn set_before_fingerprint(&mut self, fingerprint: Option<CanonicalDigest>) {
        self.before_fingerprint = fingerprint;
    }

    pub fn desired(&self) -> &Value {
        &self.desired
    }

    pub const fn desired_mode(&self) -> u32 {
        self.desired_mode
    }

    pub const fn sensitive(&self) -> bool {
        self.sensitive
    }
}

/// One complete daemon-local installation tuple selected for a Worker harness. Paths are
/// metadata inputs, not executable identities or ambient authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencySelectionRecordV1 {
    pub harness: crate::delegation::WorkerHarnessV1,
    pub adapter_path: String,
    pub cli_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
}

impl WorkerDependencySelectionRecordV1 {
    pub fn new(
        harness: crate::delegation::WorkerHarnessV1,
        adapter_path: impl Into<String>,
        cli_path: impl Into<String>,
        node_path: Option<String>,
    ) -> Result<Self, OperationValidationError> {
        let selection = Self {
            harness,
            adapter_path: adapter_path.into(),
            cli_path: cli_path.into(),
            node_path,
        };
        if !valid_native_absolute_path(&selection.adapter_path)
            || !valid_native_absolute_path(&selection.cli_path)
            || selection
                .node_path
                .as_deref()
                .is_some_and(|path| !valid_native_absolute_path(path))
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(selection)
    }
}

/// Compare-and-select input owned by the protected Worker dependency management operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerDependencySelectionChangeV1 {
    pub before_revision: u64,
    pub after_selection: WorkerDependencySelectionRecordV1,
}

impl WorkerDependencySelectionChangeV1 {
    pub fn new(
        before_revision: u64,
        after_selection: WorkerDependencySelectionRecordV1,
    ) -> Result<Self, OperationValidationError> {
        before_revision
            .checked_add(1)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        Ok(Self {
            before_revision,
            after_selection,
        })
    }

    pub const fn after_revision(&self) -> Option<u64> {
        self.before_revision.checked_add(1)
    }
}

fn valid_native_absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains('\0')
        && !value.contains(char::is_control)
        && std::path::Path::new(value).is_absolute()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransactionPlanV1 {
    spec: ChangeSpecV1,
    control: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_pool: Option<CredentialPoolMutationV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worker_dependency_selection: Option<WorkerDependencySelectionChangeV1>,
    #[serde(default)]
    secrets: Vec<SecretMutationV1>,
    /// Reconstructed from the authenticated AgentConnection control envelope. Keeping this out
    /// of the outer serde shape avoids two durable sources for one mutation.
    #[serde(skip)]
    agent_access_grants: Vec<AgentAccessGrantMutationV1>,
    #[serde(default)]
    runtime: Vec<RuntimeMutationV1>,
    #[serde(default)]
    external: Vec<ExternalEffectIntentV1>,
}

impl TransactionPlanV1 {
    pub fn from_registered_typed_planner(
        spec: ChangeSpecV1,
        control: Value,
        credential_pool: Option<CredentialPoolMutationV1>,
        secrets: Vec<SecretMutationV1>,
        runtime: Vec<RuntimeMutationV1>,
        external: Vec<ExternalEffectIntentV1>,
    ) -> Result<Self, OperationValidationError> {
        validate_registered_plan(
            &spec,
            &control,
            credential_pool.as_ref(),
            &secrets,
            &runtime,
            &external,
        )?;
        let agent_access_grants = if matches!(
            spec.command_id.as_str(),
            "agents.connect.apply" | "agents.restore.apply" | "agents.settings.apply"
        ) {
            agent_connection::agent_access_grants_from_control(&control)?
        } else {
            Vec::new()
        };
        Ok(Self {
            spec,
            control,
            credential_pool,
            worker_dependency_selection: None,
            secrets,
            agent_access_grants,
            runtime,
            external,
        })
    }

    /// Constructs the only registered plan that can mutate a daemon-local Worker installation
    /// selection. The selected paths remain data until the daemon adapter performs its bounded
    /// metadata validation; this constructor prevents arbitrary wire JSON from becoming a
    /// generic Control effect.
    pub fn from_worker_dependency_selection_planner(
        spec: ChangeSpecV1,
        change: WorkerDependencySelectionChangeV1,
    ) -> Result<Self, OperationValidationError> {
        let harness = change.after_selection.harness;
        let resource = match harness {
            crate::delegation::WorkerHarnessV1::CodexCli => "worker-dependency-selection/codex_cli",
            crate::delegation::WorkerHarnessV1::ClaudeCode => {
                "worker-dependency-selection/claude_code"
            }
        };
        if spec.schema_version.major != crate::CHANGE_SPEC_SCHEMA_V1.major
            || spec.command_id != "worker.dependencies.select"
            || spec.resource_id.as_deref() != Some(resource)
            || spec.desired_state != serde_json::to_value(&change.after_selection)?
            || change.after_revision().is_none()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(Self {
            spec,
            control: json!({}),
            credential_pool: None,
            worker_dependency_selection: Some(change),
            secrets: Vec::new(),
            agent_access_grants: Vec::new(),
            runtime: Vec::new(),
            external: Vec::new(),
        })
    }

    pub fn spec(&self) -> &ChangeSpecV1 {
        &self.spec
    }

    pub fn control(&self) -> &Value {
        &self.control
    }

    pub fn credential_pool(&self) -> Option<&CredentialPoolMutationV1> {
        self.credential_pool.as_ref()
    }

    pub fn worker_dependency_selection(&self) -> Option<&WorkerDependencySelectionChangeV1> {
        self.worker_dependency_selection.as_ref()
    }

    pub fn compute_source(&self) -> Option<ComputeSourceMutationV1> {
        (self.spec.command_id == "compute.connection.apply")
            .then(|| compute::decode_source_mutation(&self.spec, &self.control).ok())
            .flatten()
    }

    pub fn secrets(&self) -> &[SecretMutationV1] {
        &self.secrets
    }

    pub fn agent_access_grants(&self) -> &[AgentAccessGrantMutationV1] {
        &self.agent_access_grants
    }

    pub fn runtime(&self) -> &[RuntimeMutationV1] {
        &self.runtime
    }

    pub fn external(&self) -> &[ExternalEffectIntentV1] {
        &self.external
    }
}

/// Keeps the cross-crate planner seam fail-closed until formal command DTO owners add another
/// registered variant. Public callers cannot turn arbitrary JSON into an internal effects DSL.
fn validate_registered_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    credential_pool: Option<&CredentialPoolMutationV1>,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if spec.schema_version.major != crate::CHANGE_SPEC_SCHEMA_V1.major
        || contains_disabled_or_url(&spec.desired_state)
        || spec
            .resource_id
            .as_deref()
            .is_some_and(|resource| resource.contains("://") || resource.contains("//"))
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    match spec.command_id.as_str() {
        "setup.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            validate_registered_setup_plan(spec, control, secrets, runtime, external)
        }
        "compute.connection.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            compute::validate_connection_plan(spec, control, secrets, runtime, external)
        }
        "compute.credential.add" | "compute.credential.replace" | "compute.credential.remove" => {
            compute::validate_credential_plan(
                spec,
                control,
                credential_pool,
                secrets,
                runtime,
                external,
            )
        }
        "compute.key-pool.apply" => compute::validate_control_only_plan(
            spec,
            control,
            credential_pool,
            secrets,
            runtime,
            external,
        ),
        "prices.override.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            if spec.desired_state.get("schema").and_then(Value::as_str)
                == Some(crate::SOURCE_PRICE_CHANGE_SCHEMA_V2)
            {
                source_price::validate_plan(spec, control, secrets, runtime, external)
            } else {
                compute::validate_control_only_plan(spec, control, None, secrets, runtime, external)
            }
        }
        "agents.connect.apply" | "agents.restore.apply" | "agents.settings.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            agent_connection::validate_plan(spec, control, secrets, runtime, external)
        }
        "agents.config-permissions.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            agent_config_permission::validate_plan(spec, control, secrets, runtime, external)
        }
        "routing.apply" => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            routing::validate_plan(spec, control, secrets, runtime, external)
        }
        "routing.classifier.secret.apply" => validate_classifier_header_secret_plan(
            spec,
            control,
            credential_pool,
            secrets,
            runtime,
            external,
        ),
        COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2 => {
            if credential_pool.is_some() {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            subscription_check::validate_plan(spec, control, secrets, runtime, external)
        }
        _ => Err(OperationValidationError::UnregisteredEffectPlan),
    }
}

fn validate_classifier_header_secret_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    credential_pool: Option<&CredentialPoolMutationV1>,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    let input = spec
        .desired_state
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if input.len() != 3
        || input.keys().any(|key| {
            !matches!(
                key.as_str(),
                "secret_id" | "input_slot" | "expected_generation"
            )
        })
        || spec.resource_id.as_deref() != Some("personal/default")
        || credential_pool.is_some()
        || !runtime.is_empty()
        || !external.is_empty()
        || secrets.len() != 1
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let secret_id = input
        .get("secret_id")
        .and_then(Value::as_str)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    let input_slot = input
        .get("input_slot")
        .and_then(Value::as_str)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    let expected_generation = input
        .get("expected_generation")
        .and_then(Value::as_u64)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    validate_scope_identifier(secret_id)?;
    validate_scope_identifier(input_slot)?;
    let mutation = &secrets[0];
    let credential = mutation.credential();
    if mutation.kind() != SecretMutationKind::Upsert
        || mutation.input_slot() != Some(input_slot)
        || mutation.expected_generation() != expected_generation
        || credential.credential_id() != secret_id
        || credential.owner_scope() != "personal/default"
        || credential.subject() != "hirouted"
        || credential.purpose() != "http-header"
        || !credential.allowed_destinations().is_empty()
        || credential.generation() != expected_generation
        || control != &json!({"classifier_header_secret": secret_id})
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

fn validate_registered_setup_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    let input = spec
        .desired_state
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if input.keys().any(|key| {
        !matches!(
            key.as_str(),
            "connection_option_id" | "secret" | "runtime_expected_generation"
        )
    }) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let connection_option_id = input
        .get("connection_option_id")
        .and_then(Value::as_str)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    validate_scope_identifier(connection_option_id)?;
    if control != &json!({"connection_option_id": connection_option_id}) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let expected_runtime_generation = match input.get("runtime_expected_generation") {
        Some(value) => value
            .as_u64()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
        None => 0,
    };
    if runtime.len() != 1
        || runtime[0].key != "active/setup"
        || runtime[0].value != json!({"ready": true})
        || runtime[0].expected_generation != expected_runtime_generation
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if external.len() != 2
        || !external.iter().any(is_registered_setup_publication)
        || !external.iter().any(is_registered_setup_agent)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    validate_registered_setup_secret(connection_option_id, input.get("secret"), secrets)
}

fn validate_registered_setup_secret(
    connection_option_id: &str,
    input: Option<&Value>,
    secrets: &[SecretMutationV1],
) -> Result<(), OperationValidationError> {
    let Some(input) = input.filter(|value| !value.is_null()) else {
        return if secrets.is_empty() {
            Ok(())
        } else {
            Err(OperationValidationError::UnregisteredEffectPlan)
        };
    };
    let input = input
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if input
        .keys()
        .any(|key| !matches!(key.as_str(), "input_slot" | "expected_generation"))
        || secrets.len() != 1
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let input_slot = input
        .get("input_slot")
        .and_then(Value::as_str)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    let expected_generation = match input.get("expected_generation") {
        Some(value) => value
            .as_u64()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?,
        None => 0,
    };
    let secret = &secrets[0];
    let credential = secret.credential();
    if secret.kind() != SecretMutationKind::Upsert
        || secret.expected_generation() != expected_generation
        || secret.input_slot() != Some(input_slot)
        || credential.credential_id() != format!("credential/{connection_option_id}")
        || credential.owner_scope() != format!("connection/{connection_option_id}")
        || credential.subject() != "hirouted"
        || credential.purpose() != "provider-auth"
        || credential.allowed_destinations() != &BTreeSet::from(["provider-api".to_owned()])
        || credential.generation() != expected_generation
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

fn is_registered_setup_publication(effect: &ExternalEffectIntentV1) -> bool {
    effect.kind == OwnedEffectKind::Publication
        && effect.effect_id == "publication-setup"
        && effect.target == "publication/current"
        && *effect.desired == json!({"setup": "active"})
        && effect.desired_mode == 0o644
        && !effect.sensitive
}

fn is_registered_setup_agent(effect: &ExternalEffectIntentV1) -> bool {
    effect.kind == OwnedEffectKind::AgentArtifact
        && effect.effect_id == "agent-setup"
        && effect.target == "agents/codex"
        && *effect.desired == json!({"configured": true})
        && effect.desired_mode == 0o640
        && !effect.sensitive
}

fn contains_disabled_or_url(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            disabled_plan_token(key)
                || contains_disabled_or_url(value)
                || value.as_str().is_some_and(|text| {
                    disabled_plan_token(text) || text.contains("://") || text.starts_with("//")
                })
        }),
        Value::Array(values) => values.iter().any(contains_disabled_or_url),
        Value::String(value) => {
            disabled_plan_token(value) || value.contains("://") || value.starts_with("//")
        }
        _ => false,
    }
}

fn disabled_plan_token(value: &str) -> bool {
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

/// A bearer capability received over the authenticated local transport. The bytes are
/// zeroized and this type intentionally has no serde or cloning surface.
pub struct ProtectedApplyCapability(Zeroizing<Vec<u8>>);

impl ProtectedApplyCapability {
    pub fn new(value: String) -> Result<Self, OperationValidationError> {
        if value.is_empty() || value.len() > 4096 {
            return Err(OperationValidationError::InvalidApplyCapability);
        }
        Ok(Self(Zeroizing::new(value.into_bytes())))
    }

    pub fn expose(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// Exact authorization established by a trusted capability verifier. The absence of serde and
/// private fields keep raw wire principals/capabilities out of the transaction engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedApplyAuthorizationV1 {
    capability_digest: CanonicalDigest,
    principal: String,
    workspace_id: WorkspaceId,
    operation_kind: String,
    accepted_digest: CanonicalDigest,
    expected_revisions_digest: CanonicalDigest,
    scope: String,
    expires_at_unix: i64,
}

impl VerifiedApplyAuthorizationV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_capability_verifier(
        capability_digest: CanonicalDigest,
        principal: impl Into<String>,
        workspace_id: WorkspaceId,
        operation_kind: impl Into<String>,
        accepted_digest: CanonicalDigest,
        expected_revisions_digest: CanonicalDigest,
        scope: impl Into<String>,
        expires_at_unix: i64,
    ) -> Result<Self, OperationValidationError> {
        let verified = Self {
            capability_digest,
            principal: principal.into(),
            workspace_id,
            operation_kind: operation_kind.into(),
            accepted_digest,
            expected_revisions_digest,
            scope: scope.into(),
            expires_at_unix,
        };
        validate_scope_identifier(&verified.principal)?;
        validate_scope_identifier(&verified.operation_kind)?;
        validate_scope_identifier(&verified.scope)?;
        Ok(verified)
    }

    pub fn capability_digest(&self) -> &CanonicalDigest {
        &self.capability_digest
    }

    pub fn principal(&self) -> &str {
        &self.principal
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn operation_kind(&self) -> &str {
        &self.operation_kind
    }

    pub fn accepted_digest(&self) -> &CanonicalDigest {
        &self.accepted_digest
    }

    pub fn expected_revisions_digest(&self) -> &CanonicalDigest {
        &self.expected_revisions_digest
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub const fn expires_at_unix(&self) -> i64 {
        self.expires_at_unix
    }

    pub fn matches_operation(&self, operation: &OperationV1) -> bool {
        self.principal == operation.idempotency.principal
            && self.workspace_id == operation.workspace_id
            && self.operation_kind == operation.idempotency.operation_kind
            && self.accepted_digest == operation.accepted_digest
            && CanonicalDigest::of(&operation.expected_revisions)
                .is_ok_and(|digest| digest == self.expected_revisions_digest)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct OperationV1 {
    #[serde(skip)]
    journal_checkpoint: Option<Arc<journal::JournalCheckpoint>>,
    pub schema_version: u16,
    pub operation_id: OperationId,
    pub workspace_id: WorkspaceId,
    pub idempotency: IdempotencyScopeV1,
    pub request_digest: CanonicalDigest,
    pub accepted_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    #[serde(serialize_with = "shared_input::serialize")]
    pub plan: Arc<TransactionPlanV1>,
    pub state: OperationState,
    pub generation: u64,
    pub steps: Vec<OperationStepV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_error_code: Option<String>,
}

/// Read-only status, deliberately insufficient to execute or authorize an Operation.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationStatus {
    pub operation_id: OperationId,
    pub state: OperationState,
    pub generation: u64,
    pub accepted_digest: CanonicalDigest,
    pub safe_error_code: Option<String>,
}

impl OperationV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: OperationId,
        workspace_id: WorkspaceId,
        idempotency: IdempotencyScopeV1,
        request_digest: CanonicalDigest,
        accepted_digest: CanonicalDigest,
        expected_revisions: RevisionSetV1,
        plan: TransactionPlanV1,
    ) -> Result<Self, OperationValidationError> {
        let control_step_input = if let Some(selection) = &plan.worker_dependency_selection {
            serde_json::to_value((&plan.control, &plan.credential_pool, selection))?
        } else {
            serde_json::to_value((&plan.control, &plan.credential_pool))?
        };
        let step_inputs = [
            json!({
                "spec": &plan.spec,
                "accepted_digest": &accepted_digest,
                "expected_revisions": &expected_revisions,
            }),
            serde_json::to_value((&plan.secrets, &plan.agent_access_grants))?,
            control_step_input,
            serde_json::to_value(
                plan.external
                    .iter()
                    .filter(|effect| effect.kind == OwnedEffectKind::Publication)
                    .collect::<Vec<_>>(),
            )?,
            serde_json::to_value(
                plan.external
                    .iter()
                    .filter(|effect| effect.kind == OwnedEffectKind::AgentArtifact)
                    .collect::<Vec<_>>(),
            )?,
            serde_json::to_value(&plan.runtime)?,
        ];
        let steps = OperationStepKind::ALL
            .into_iter()
            .zip(step_inputs)
            .enumerate()
            .map(|(index, (kind, input))| {
                Ok(OperationStepV1 {
                    sequence: u16::try_from(index).expect("six steps fit in u16"),
                    kind,
                    deterministic_input_digest: CanonicalDigest::of(&input)?,
                    status: OperationStepStatus::Pending,
                    attempts: 0,
                    effects: Vec::new(),
                    terminal_result: None,
                })
            })
            .collect::<Result<Vec<_>, OperationValidationError>>()?;
        let mut operation = Self {
            journal_checkpoint: None,
            schema_version: OPERATION_SCHEMA_VERSION,
            operation_id,
            workspace_id,
            idempotency,
            request_digest,
            accepted_digest,
            expected_revisions,
            plan: Arc::new(plan),
            state: OperationState::Accepted,
            generation: 0,
            steps,
            safe_error_code: None,
        };
        operation.establish_journal_checkpoint()?;
        Ok(operation)
    }

    pub fn transition(&mut self, next: OperationState) -> Result<(), OperationValidationError> {
        if !self.state.can_transition_to(next) {
            return Err(OperationValidationError::InvalidTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }

    pub fn step(&self, kind: OperationStepKind) -> &OperationStepV1 {
        self.steps
            .iter()
            .find(|step| step.kind == kind)
            .expect("every operation has the fixed six-step journal")
    }

    pub fn step_mut(&mut self, kind: OperationStepKind) -> &mut OperationStepV1 {
        self.steps
            .iter_mut()
            .find(|step| step.kind == kind)
            .expect("every operation has the fixed six-step journal")
    }

    /// Restores journal-only mutable fields after an owner-controlled durable decoder has
    /// reconstructed and validated the immutable operation inputs.
    pub fn restore_durable_state(
        &mut self,
        state: OperationState,
        generation: u64,
        steps: Vec<OperationStepV1>,
        safe_error_code: Option<String>,
    ) -> Result<(), OperationValidationError> {
        if steps.len() != OperationStepKind::ALL.len()
            || steps
                .iter()
                .zip(OperationStepKind::ALL)
                .enumerate()
                .any(|(index, (step, kind))| {
                    step.sequence != u16::try_from(index).expect("six steps fit in u16")
                        || step.kind != kind
                        || step.deterministic_input_digest
                            != self.steps[index].deterministic_input_digest
                })
        {
            return Err(OperationValidationError::InvalidDurableJournal);
        }
        self.state = state;
        self.generation = generation;
        self.steps = steps;
        self.safe_error_code = safe_error_code;
        self.establish_journal_checkpoint()?;
        Ok(())
    }
}

pub fn validate_idempotency_key(value: &str) -> Result<(), OperationValidationError> {
    let valid = !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(OperationValidationError::InvalidIdempotencyKey)
    }
}

pub(super) fn validate_scope_identifier(value: &str) -> Result<(), OperationValidationError> {
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
        Err(OperationValidationError::InvalidIdempotencyScope)
    }
}

#[derive(Debug, Error)]
pub enum OperationValidationError {
    #[error("operation id is malformed")]
    InvalidOperationId,
    #[error("idempotency scope contains an invalid principal or operation kind")]
    InvalidIdempotencyScope,
    #[error("idempotency key is not a bounded portable identifier")]
    InvalidIdempotencyKey,
    #[error("protected Secret input must not be empty")]
    EmptyProtectedSecret,
    #[error("apply capability is empty or unbounded")]
    InvalidApplyCapability,
    #[error("CredentialRef is incomplete or has no allowed destination")]
    InvalidCredentialRef,
    #[error("AgentAccessGrant material is not canonical 256-bit base64url")]
    InvalidAgentAccessGrantMaterial,
    #[error("AgentAccessGrant scope is incomplete or invalid")]
    InvalidAgentAccessGrantScope,
    #[error("AgentAccessGrant mutation is inconsistent")]
    InvalidAgentAccessGrantMutation,
    #[error("AgentAccessGrantRef is incomplete or invalid")]
    InvalidAgentAccessGrantRef,
    #[error("effect intent is not owned by the registered adapter")]
    InvalidExternalEffect,
    #[error("internal effects do not match a registered typed planner")]
    UnregisteredEffectPlan,
    #[error("artifact mode is outside the supported POSIX policy")]
    InvalidArtifactMode,
    #[error("durable Operation journal does not match its immutable plan")]
    InvalidDurableJournal,
    #[error("invalid operation transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: OperationState,
        to: OperationState,
    },
    #[error("operation plan cannot be serialized: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("operation step digest cannot be computed: {0}")]
    Digest(#[from] crate::CanonicalDigestError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BeginOperationOutcome {
    Created,
    ExistingSame(Box<OperationV1>),
    ExistingDifferent,
    RevisionChanged(RevisionMismatch),
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectReconciliation {
    Missing,
    /// An immutable Operation-owned version exists, but is not yet visible through committed
    /// readers. Only the Activate step may publish it.
    Staged(OwnedEffectV1),
    Applied(OwnedEffectV1),
    /// The durable marker identifies the exact effect, but the target fingerprint no longer
    /// proves that this Operation owns the current bytes/state.
    OwnershipLost(OwnedEffectV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompensationOutcome {
    Compensated,
    AlreadyCompensated,
    OwnershipLost,
}

/// Secret bytes never implement `Debug`, `Clone`, `Serialize`, or `Deserialize`.
pub struct ProtectedSecret(Zeroizing<Vec<u8>>);

impl ProtectedSecret {
    pub fn new(bytes: Vec<u8>) -> Result<Self, OperationValidationError> {
        if bytes.is_empty() {
            return Err(OperationValidationError::EmptyProtectedSecret);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub fn expose(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub trait ControlRepositoryPort {
    fn current_revisions(&self, workspace: &WorkspaceId) -> PortResult<RevisionSetV1>;
    fn operation_for_idempotency(
        &self,
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
    ) -> PortResult<Option<OperationV1>>;
    fn verify_apply_authorization(
        &self,
        capability: &ProtectedApplyCapability,
        workspace: &WorkspaceId,
        principal: &str,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> PortResult<VerifiedApplyAuthorizationV1>;
    /// Consumes the capability and admits the Operation in the same durable transaction.
    fn begin_operation(
        &self,
        operation: &OperationV1,
        authorization: &VerifiedApplyAuthorizationV1,
    ) -> PortResult<BeginOperationOutcome>;
    /// Admits one Released mutation after the owner-only Local Control transport has verified the
    /// peer UID. Implementations must preserve the same atomic writer claim, revision CAS, and
    /// idempotency checks as capability-backed admission.
    fn begin_local_operation(&self, _operation: &OperationV1) -> PortResult<BeginOperationOutcome> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "control.local_operation.unsupported",
        ))
    }
    /// Check a value loaded or committed by this writer against the current durable generation.
    /// This does not authorize an Operation or substitute for cold recovery decoding.
    fn operation_is_current(&self, operation: &OperationV1) -> PortResult<bool>;
    fn load_operation(&self, operation_id: &OperationId) -> PortResult<Option<OperationV1>>;
    fn recoverable_operations(&self) -> PortResult<Vec<OperationV1>>;
    /// True until startup reconciliation has resolved every admitted writer claim.
    fn writer_recovery_required(&self) -> PortResult<bool>;
    fn save_operation(&self, operation: &mut OperationV1) -> PortResult<()>;
    /// Reads the independent CAS head for one daemon-local Worker harness. Unselected is revision
    /// zero; this head is deliberately not part of the workspace desired-state revision.
    fn worker_dependency_selection_revision(
        &self,
        _workspace: &WorkspaceId,
        _harness: crate::delegation::WorkerHarnessV1,
    ) -> PortResult<u64> {
        Err(crate::PortError::new(
            crate::PortErrorCode::Unavailable,
            "control.worker_dependency_selection.unsupported",
        ))
    }
    /// Stages the exact protected Worker dependency CAS as this Operation's Control effect.
    fn apply_worker_dependency_selection(
        &self,
        _operation_id: &OperationId,
        _workspace: &WorkspaceId,
        _change: &WorkerDependencySelectionChangeV1,
    ) -> PortResult<OwnedEffectV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::Unavailable,
            "control.worker_dependency_selection.unsupported",
        ))
    }
    fn apply_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_revision: u64,
        desired: &Value,
    ) -> PortResult<OwnedEffectV1>;
    fn observe_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
    ) -> PortResult<EffectReconciliation>;
    fn activate_control(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1>;
    fn compensate_control(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome>;
    /// Persists a terminal state and releases the durable single-writer claim when safe.
    fn finish_operation(&self, operation: &mut OperationV1) -> PortResult<u64>;
    /// Persists a non-terminal settings tail and atomically releases this operation's writer
    /// claim in the same control transaction. Fails when another operation holds the claim.
    fn save_operation_tail(&self, operation: &mut OperationV1) -> PortResult<()>;
    /// Re-acquires the durable writer claim for an explicit same-operation retry when the claim
    /// is free or already owned by this operation.
    fn reclaim_operation_writer(&self, operation_id: &OperationId) -> PortResult<()>;
    /// Persists the current trusted live-check result for one settings context and surface.
    /// The write is silently dropped (returning false) when the installed publication no longer
    /// matches the checked revision, so a stale check never pollutes a newer publication.
    fn save_agent_surface_check(
        &self,
        workspace: &WorkspaceId,
        record: &crate::AgentSurfaceCheckRecordV1,
    ) -> PortResult<bool>;
    /// Reads the current live-check records for one settings context, one per surface.
    fn agent_surface_checks(
        &self,
        workspace: &WorkspaceId,
        context_id: &str,
    ) -> PortResult<Vec<crate::AgentSurfaceCheckRecordV1>>;
}

/// Typed staging seam for a CredentialPool control CAS. The resulting effect remains the single
/// Control effect in the Operation journal, so Secret and pool changes share activation/rollback.
pub trait CredentialPoolControlPort {
    fn apply_credential_pool(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_target_revision: u64,
        mutation: &CredentialPoolMutationV1,
    ) -> PortResult<OwnedEffectV1>;
}

/// Typed staging seam for the sealed compute-source CAS. Storage must stage this mutation in the
/// same Control effect as workspace desired state and activate or compensate both together.
pub trait ComputeSourceControlPort {
    fn apply_compute_source(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_target_revision: u64,
        mutation: &ComputeSourceMutationV1,
    ) -> PortResult<OwnedEffectV1>;
}

pub trait SecretStorePort {
    fn generation(&self, credential: &CredentialRefV1) -> PortResult<u64>;
    fn fingerprint(&self, secret: &ProtectedSecret) -> PortResult<CanonicalDigest>;
    fn resolve_secret(
        &self,
        subject: &VerifiedSecretSubjectV1,
        credential: &CredentialRefV1,
        purpose: &str,
        destination: &str,
        expected_generation: u64,
    ) -> PortResult<ProtectedSecret>;
    fn apply_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
        input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1>;
    fn observe_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
    ) -> PortResult<EffectReconciliation>;
    fn activate_secret(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1>;
    fn compensate_secret(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome>;

    /// Reads only non-secret metadata for one exact canonical connection. Adapters that have not
    /// implemented the dedicated grant store fail closed instead of treating it as a Provider
    /// CredentialRef.
    fn inspect_agent_access_grant(
        &self,
        _owner_scope: &str,
        _connection_id: &str,
    ) -> PortResult<Option<AgentAccessGrantRefV1>> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.inspect.unsupported",
        ))
    }

    fn resolve_agent_access_grant(
        &self,
        _reference: &AgentAccessGrantRefV1,
    ) -> PortResult<AgentAccessGrantMaterial> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.resolve.unsupported",
        ))
    }

    fn apply_agent_access_grant(
        &self,
        _operation_id: &OperationId,
        _mutation: &AgentAccessGrantMutationV1,
        _input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.apply.unsupported",
        ))
    }

    fn observe_agent_access_grant(
        &self,
        _operation_id: &OperationId,
        _mutation: &AgentAccessGrantMutationV1,
    ) -> PortResult<EffectReconciliation> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.observe.unsupported",
        ))
    }

    fn activate_agent_access_grant(&self, _effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.activate.unsupported",
        ))
    }

    fn compensate_agent_access_grant(
        &self,
        _effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "agent_access_grant.compensate.unsupported",
        ))
    }
}

pub trait RuntimeStatePort {
    fn generation(&self, key: &str) -> PortResult<u64>;
    fn apply_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<OwnedEffectV1>;
    fn observe_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<EffectReconciliation>;
    fn activate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1>;
    fn compensate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome>;
}

/// Versioned adapter boundary for the sole writable compute-runtime authority.
pub trait ComputeRuntimeStateStoreV1 {
    fn runtime_state(
        &self,
        identity: &RuntimeStateIdentityV1,
    ) -> PortResult<Option<RuntimeStateV1>>;

    fn compare_and_set_runtime_state(
        &self,
        expected_generation: u64,
        state: &RuntimeStateV1,
    ) -> PortResult<()>;

    fn acquire_runtime_probe(
        &self,
        identity: &RuntimeStateIdentityV1,
        expected_generation: u64,
        request: &RuntimeProbeLeaseRequestV1,
    ) -> PortResult<RuntimeProbeAcquireOutcomeV1>;

    fn complete_runtime_probe(
        &self,
        lease: &RuntimeProbeLeaseV1,
        state: &RuntimeStateV1,
    ) -> PortResult<()>;
}

pub trait ExternalEffectPort {
    /// Install committed source prices before the coordinator reports success. Only the
    /// production control adapter can rebuild this process-local immutable snapshot.
    fn install_source_price_snapshot(
        &self,
        _operation: &OperationV1,
    ) -> PortResult<crate::PriceGenerationRefV1> {
        Err(crate::PortError::new(
            crate::PortErrorCode::Unavailable,
            "prices.snapshot.unwired",
        ))
    }

    fn validate_external_admission(&self, _intent: &ExternalEffectIntentV1) -> PortResult<()> {
        Ok(())
    }
    /// Close new publication pins before activating any of this operation's effects.
    fn begin_publication_activation(&self, _operation: &OperationV1) -> PortResult<()> {
        Ok(())
    }
    /// The coordinator persists this checkpoint before asking the adapter to install it.
    fn prepare_publication_activation(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        Ok(effect.clone())
    }
    /// Return a durable abort checkpoint only when rollback cannot invalidate a live target.
    /// Unavailable preserves the running journal for restart; Conflict requires repair.
    fn prepare_publication_rollback(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        Ok(effect.clone())
    }
    fn finish_publication_activation(&self, _operation: &OperationV1) -> PortResult<()> {
        Ok(())
    }
    /// Persist adapter-owned activation metadata from the admitted plan. Runs under the
    /// existing writer, including replay of an artifact that is already applied.
    fn prepare_agent_artifact_activation(
        &self,
        _operation: &OperationV1,
        _intent: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        Ok(())
    }
    fn current_external_fingerprint(&self, target: &str) -> PortResult<Option<CanonicalDigest>>;
    fn apply_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1>;
    fn observe_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation>;
    fn activate_external(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1>;
    fn compensate_external(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::{CHANGE_SPEC_SCHEMA_V1, RevisionSetV1};

    fn operation() -> OperationV1 {
        let workspace = WorkspaceId::default();
        let request_digest = CanonicalDigest::of_bytes(b"operation-request");
        let accepted_digest = CanonicalDigest::of_bytes(b"operation-change");
        let scope = IdempotencyScopeV1::new("interactive-user", "ApplySetup", "idem-1")
            .expect("valid scope");
        OperationV1::new(
            OperationId::derive(&workspace, &scope, &request_digest),
            workspace,
            scope,
            request_digest,
            accepted_digest,
            RevisionSetV1 {
                target: 0,
                dependencies: BTreeMap::new(),
            },
            TransactionPlanV1::from_registered_typed_planner(
                ChangeSpecV1 {
                    schema_version: CHANGE_SPEC_SCHEMA_V1,
                    command_id: "setup.apply".to_owned(),
                    resource_id: None,
                    desired_state: json!({"connection_option_id": "source-a"}),
                },
                json!({"connection_option_id": "source-a"}),
                None,
                Vec::new(),
                vec![
                    RuntimeMutationV1::from_registered_planner(
                        "active/setup",
                        json!({"ready": true}),
                        0,
                    )
                    .unwrap(),
                ],
                vec![
                    ExternalEffectIntentV1::from_registered_adapter(
                        "publication-setup",
                        OwnedEffectKind::Publication,
                        "publication/current",
                        None,
                        json!({"setup": "active"}),
                        0o644,
                        false,
                    )
                    .unwrap(),
                    ExternalEffectIntentV1::from_registered_adapter(
                        "agent-setup",
                        OwnedEffectKind::AgentArtifact,
                        "agents/codex",
                        None,
                        json!({"configured": true}),
                        0o640,
                        false,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn operation_has_exact_deterministic_step_plan() {
        let first = operation();
        let second = operation();
        assert_eq!(first.steps, second.steps);
        assert_eq!(
            first.steps.iter().map(|step| step.kind).collect::<Vec<_>>(),
            OperationStepKind::ALL
        );
    }

    #[test]
    fn operation_id_is_scoped_to_principal_kind_key_and_request() {
        let operation = operation();
        let mut different = operation.idempotency.clone();
        different.principal = "desktop".to_owned();
        assert_ne!(
            operation.operation_id,
            OperationId::derive(
                &operation.workspace_id,
                &different,
                &operation.request_digest
            )
        );
    }

    #[test]
    fn operation_terminal_state_is_unique_and_transition_checked() {
        let mut operation = operation();
        for state in [
            OperationState::Preparing,
            OperationState::ApplyingSecrets,
            OperationState::MaterializingSources,
            OperationState::CompilingPublication,
            OperationState::ApplyingAgentArtifacts,
            OperationState::Activating,
            OperationState::Succeeded,
        ] {
            operation.transition(state).unwrap();
        }
        assert!(operation.state.is_terminal());
        assert!(operation.transition(OperationState::RolledBack).is_err());
    }

    #[test]
    fn operation_rollback_can_only_end_rolled_back_or_attention() {
        for terminal in [OperationState::RolledBack, OperationState::NeedsAttention] {
            let mut operation = operation();
            operation.transition(OperationState::RollingBack).unwrap();
            operation.transition(terminal).unwrap();
            assert!(operation.state.is_terminal());
        }
    }

    #[test]
    fn protected_secret_has_a_distinct_empty_input_error_and_no_debug_surface() {
        assert!(matches!(
            ProtectedSecret::new(Vec::new()),
            Err(OperationValidationError::EmptyProtectedSecret)
        ));
    }

    #[test]
    fn generic_effect_values_are_not_a_registered_plan() {
        assert!(matches!(
            RuntimeMutationV1::from_registered_planner(
                "active/setup",
                json!({"target": "https://example.invalid"}),
                0,
            ),
            Err(OperationValidationError::UnregisteredEffectPlan)
        ));
        assert!(matches!(
            ExternalEffectIntentV1::from_registered_adapter(
                "agent-setup",
                OwnedEffectKind::AgentArtifact,
                "agents/codex",
                None,
                json!({"url": "https://example.invalid"}),
                0o640,
                false,
            ),
            Err(OperationValidationError::UnregisteredEffectPlan)
        ));
    }
}
