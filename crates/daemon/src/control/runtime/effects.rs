//! External-effect composition for production Local Control.
//!
//! The filesystem scanner owns the one mode-only remediation. Product publications are staged
//! and activated in `control.db`; all remaining Agent artifacts use the managed-artifact store.

use hiroute_domain::{
    AgentConfigChangeV1, CanonicalDigest, CompensationOutcome, ControlRepositoryPort,
    EffectReconciliation, ExternalEffectIntentV1, ExternalEffectPort, OperationId,
    OperationStepKind, OperationStepStatus, OwnedEffectKind, OwnedEffectV1, PortError,
    PortErrorCode, PortResult,
};
use hiroute_integrations::{PermissionHardeningOutcomeV1, PermissionHardeningRequiredV1};
use serde::{Deserialize, Serialize};

use super::LocalControlAdapter;
use super::publication::PUBLICATION_TARGET;

const EFFECT_SCHEMA: &str = "hiroute.agent-config-permission-effect/v1";
const EFFECT_ID: &str = "agent-config-permission-hardening";
const TARGET_PREFIX: &str = "scanner-source/";
const SOURCE_PREFIX: &str = "claude/settings/";
const REQUIRED_MODE: u32 = 0o600;
const AGENT_MANAGED_CONFIGURATION_EFFECT_ID: &str = "agent-connection-managed-configuration";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfigurationEnvelopeV1 {
    schema: String,
    transaction: String,
    subject: AgentConfigurationSubjectV1,
    change_spec_digest: CanonicalDigest,
    role: String,
    payload_digest: CanonicalDigest,
    payload: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfigurationSubjectV1 {
    agent_id: String,
    profile_id: String,
    integration_profile_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyAgentConfigurationPayloadV1 {
    profile_id: String,
    change: AgentConfigChangeV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermissionEnvelopeV1 {
    schema: String,
    transaction: String,
    intent: PermissionIntentV1,
    change_spec_digest: CanonicalDigest,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PermissionIntentV1 {
    scanner_id: String,
    scanner_version: String,
    source_ref: String,
    observed_identity: CanonicalDigest,
    observed_revision: u64,
    required_mode: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PermissionEffectMarkerV1 {
    schema: String,
    operation_id: String,
    intent: PermissionIntentV1,
}

impl PermissionIntentV1 {
    fn finding(&self) -> PermissionHardeningRequiredV1 {
        PermissionHardeningRequiredV1 {
            scanner_id: self.scanner_id.clone(),
            scanner_version: self.scanner_version.clone(),
            discovered_source_ref: self.source_ref.clone(),
            observed_identity: self.observed_identity.clone(),
            observed_revision: self.observed_revision,
            // Scanner resolution is exclusively by the opaque source ref. This label never
            // becomes a filesystem locator.
            display_path: "Claude settings".to_owned(),
            required_mode: self.required_mode,
        }
    }
}

impl LocalControlAdapter {
    pub(super) fn native_claude_change(
        &self,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<AgentConfigChangeV1>> {
        if intent.effect_id() != AGENT_MANAGED_CONFIGURATION_EFFECT_ID {
            return Ok(None);
        }
        if super::native_model::is_settings_codex_model(intent) {
            hiroute_application::agent_connection::decode_settings_codex_model_file(intent)?;
            return Ok(None);
        }
        if super::native_claude_model::is_settings_claude_model(intent) {
            hiroute_application::agent_connection::decode_settings_claude_model_file(intent)?;
            return Ok(None);
        }
        if super::native_additional_model::is_settings_additional_model(intent) {
            hiroute_application::agent_connection::decode_settings_additional_model_file(intent)?;
            return Ok(None);
        }
        let envelope: AgentConfigurationEnvelopeV1 =
            serde_json::from_value(intent.desired().clone())
                .map_err(|_| invalid("agent-config.intent.decode"))?;
        let expected_payload_digest = CanonicalDigest::of(&envelope.payload)
            .map_err(|_| invalid("agent-config.intent.digest"))?;
        if envelope.schema != "hiroute.agent-connection-effect/v1"
            || envelope.role != "managed_configuration"
            || envelope.transaction != "apply"
            || envelope.subject.agent_id != "agent_claude_default"
            || envelope.subject.profile_id != "claude-messages-v1"
            || envelope.subject.integration_profile_ref != "builtin/claude-messages/v1"
            || envelope.payload_digest != expected_payload_digest
            || CanonicalDigest::parse(envelope.change_spec_digest.as_str().to_owned()).is_err()
            || intent.desired_mode() != 0o600
        {
            return Err(invalid("agent-config.intent.shape"));
        }
        let payload: ApplyAgentConfigurationPayloadV1 = serde_json::from_value(envelope.payload)
            .map_err(|_| invalid("agent-config.payload.decode"))?;
        if payload.profile_id != envelope.subject.profile_id {
            return Err(invalid("agent-config.payload.profile"));
        }
        payload
            .change
            .validate()
            .map_err(|_| invalid("agent-config.payload.change"))?;
        Ok(Some(payload.change))
    }

    fn permission_intent(
        &self,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<PermissionIntentV1>> {
        if intent.effect_id() != EFFECT_ID {
            return Ok(None);
        }
        let envelope: PermissionEnvelopeV1 = serde_json::from_value(intent.desired().clone())
            .map_err(|_| invalid("permission.intent.decode"))?;
        let suffix = envelope
            .intent
            .source_ref
            .strip_prefix(SOURCE_PREFIX)
            .ok_or_else(|| invalid("permission.intent.source"))?;
        if envelope.schema != EFFECT_SCHEMA
            || envelope.transaction != "harden"
            || envelope.intent.scanner_id != "builtin.agent-filesystem"
            || envelope.intent.scanner_version != "1"
            || envelope.intent.observed_revision == 0
            || envelope.intent.required_mode != REQUIRED_MODE
            || suffix.len() != 32
            || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            || intent.kind() != OwnedEffectKind::AgentArtifact
            || intent.target() != format!("{TARGET_PREFIX}{}", envelope.intent.source_ref)
            || intent.before_fingerprint() != Some(&envelope.intent.observed_identity)
            || intent.desired_mode() != REQUIRED_MODE
            || intent.sensitive()
        {
            return Err(invalid("permission.intent.shape"));
        }
        // Deserializing the digest is intentional validation even though the adapter does not
        // interpret it; the typed domain planner has already bound it to the ChangeSpec.
        let _ = envelope.change_spec_digest;
        Ok(Some(envelope.intent))
    }

    fn permission_effect(
        operation_id: &OperationId,
        intent: &PermissionIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        Ok(OwnedEffectV1 {
            effect_id: EFFECT_ID.to_owned(),
            kind: OwnedEffectKind::AgentArtifact,
            target: format!("{TARGET_PREFIX}{}", intent.source_ref),
            before_fingerprint: Some(intent.observed_identity.clone()),
            // Mode-only remediation preserves file identity.
            after_fingerprint: Some(intent.observed_identity.clone()),
            compensation: serde_json::to_value(PermissionEffectMarkerV1 {
                schema: "hiroute.agent-config-permission-marker/v1".to_owned(),
                operation_id: operation_id.to_string(),
                intent: intent.clone(),
            })
            .map_err(|_| invalid("permission.effect.encode"))?
            .into(),
        })
    }

    fn permission_marker(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<(OperationId, PermissionIntentV1)> {
        let marker: PermissionEffectMarkerV1 =
            serde::Deserialize::deserialize(effect.compensation.as_ref())
                .map_err(|_| invalid("permission.effect.decode"))?;
        let operation_id = OperationId::parse(marker.operation_id)
            .map_err(|_| invalid("permission.effect.operation"))?;
        if marker.schema != "hiroute.agent-config-permission-marker/v1"
            || effect.effect_id != EFFECT_ID
            || effect.kind != OwnedEffectKind::AgentArtifact
            || effect.target != format!("{TARGET_PREFIX}{}", marker.intent.source_ref)
            || effect.before_fingerprint.as_ref() != Some(&marker.intent.observed_identity)
            || effect.after_fingerprint.as_ref() != Some(&marker.intent.observed_identity)
        {
            return Err(invalid("permission.effect.shape"));
        }
        Ok((operation_id, marker.intent))
    }

    fn verify_or_harden(
        &self,
        intent: &PermissionIntentV1,
    ) -> PortResult<PermissionHardeningOutcomeV1> {
        self.scanner
            .harden_discovered_permissions(&intent.finding())
            .map_err(|_| PortError::new(PortErrorCode::Conflict, "permission.identity.changed"))
    }
}

impl ExternalEffectPort for LocalControlAdapter {
    fn install_source_price_snapshot(
        &self,
        operation: &hiroute_domain::OperationV1,
    ) -> PortResult<hiroute_domain::PriceGenerationRefV1> {
        self.install_committed_price_snapshot(operation)
    }

    fn prepare_agent_artifact_activation(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        self.acknowledge_clean_codex_restoration(operation, intent)?;
        if super::native_additional_model::is_settings_additional_model(intent) {
            self.validate_additional_model_dependencies(operation, intent)?;
        }
        if intent.effect_id() == "agent-connection-routing-skill"
            && intent.desired()["transaction"] == "settings"
        {
            let stores = self.stores_lock()?;
            if !stores.control().operation_is_current(operation)? {
                return Err(invalid("skill.activation.binding"));
            }
            hiroute_application::agent_connection::persist_settings_skill_file_record(
                stores.control(),
                &self.artifacts,
                operation,
                intent,
            )?;
        }
        Ok(())
    }
    fn validate_external_admission(&self, intent: &ExternalEffectIntentV1) -> PortResult<()> {
        if self.validate_subscription_effect(intent)? {
            return Ok(());
        }
        if intent
            .desired()
            .get("transaction")
            .and_then(serde_json::Value::as_str)
            == Some("settings")
            && intent.effect_id() == "agent-connection-routing-skill"
        {
            hiroute_application::agent_connection::validate_settings_skill_file_intent(intent)?;
        }
        // Resolve known native adapters before a capability is consumed or an Operation exists.
        self.native_claude_change(intent)?;
        self.permission_intent(intent)?;
        self.validate_publication_admission(intent)
    }
    fn begin_publication_activation(
        &self,
        operation: &hiroute_domain::OperationV1,
    ) -> PortResult<()> {
        self.begin_product_activation(operation)
    }
    fn prepare_publication_activation(
        &self,
        operation: &hiroute_domain::OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.publication_install_checkpoint(operation, effect)
    }
    fn prepare_publication_rollback(
        &self,
        operation: &hiroute_domain::OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.publication_abort_checkpoint(operation, effect)
    }
    fn finish_publication_activation(
        &self,
        operation: &hiroute_domain::OperationV1,
    ) -> PortResult<()> {
        self.finish_product_activation(operation)
    }
    fn current_external_fingerprint(&self, target: &str) -> PortResult<Option<CanonicalDigest>> {
        if target.starts_with("compute-subscription/") {
            return self.current_subscription_fingerprint(target);
        }
        if target == PUBLICATION_TARGET {
            return self.current_publication_fingerprint();
        }
        if let Some(source_ref) = target.strip_prefix(TARGET_PREFIX) {
            return Ok(self
                .permission_findings
                .lock()
                .map_err(|_| PortError::new(PortErrorCode::Unavailable, "permission.cache.lock"))?
                .get(source_ref)
                .map(|finding| finding.observed_identity.clone()));
        }
        self.artifacts.current_external_fingerprint(target)
    }

    fn apply_external(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        self.require_current_operation(operation)?;
        let operation_id = &operation.operation_id;
        if !operation.plan.external().contains(intent) {
            return Err(invalid("effect.intent-binding"));
        }
        if let Some(effect) = self.apply_subscription_effect(operation_id, intent)? {
            return Ok(effect);
        }
        if intent
            .desired()
            .get("transaction")
            .and_then(serde_json::Value::as_str)
            == Some("settings")
            && intent.effect_id() == "agent-connection-routing-skill"
        {
            return hiroute_application::agent_connection::stage_settings_skill_file(
                &self.artifacts,
                operation_id,
                intent,
            );
        }
        if intent.kind() == OwnedEffectKind::Publication {
            return self.apply_publication(operation, intent);
        }
        if super::native_model::is_settings_codex_catalog(intent) {
            return self.stage_settings_codex_catalog(operation, intent);
        }
        if hiroute_application::agent_connection::is_settings_login_item(intent) {
            // The host executed this action under the native confirmation before the Operation
            // was admitted; the daemon only journals the observed before/after evidence.
            return hiroute_application::agent_connection::settings_login_item_effect(intent);
        }
        if super::native_model::is_settings_codex_model(intent) {
            return self.stage_settings_codex_model(operation, intent);
        }
        if super::native_claude_model::is_settings_claude_model(intent) {
            return self.stage_settings_claude_model(operation, intent);
        }
        if super::native_additional_model::is_settings_additional_model(intent) {
            return self.stage_settings_additional_model(operation, intent);
        }
        let Some(permission) = self.permission_intent(intent)? else {
            if let Some(change) = self.native_claude_change(intent)? {
                let rendered = self
                    .scanner
                    .render_claude_user_config_change(&change)
                    .map_err(|_| {
                        PortError::new(PortErrorCode::Conflict, "agent-config.source.changed")
                    })?;
                return self
                    .artifacts
                    .apply_rendered_external(operation_id, intent, &rendered);
            }
            return self.artifacts.apply_artifact(operation_id, intent);
        };
        let outcome = self.verify_or_harden(&permission)?;
        // Apply is the first point allowed to make content readable. Re-scan now both removes
        // the one-shot mode finding and registers the exact Secret descriptor for the following
        // journal step; bytes still remain inside the protected-input adapter.
        self.refresh_discovery()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "permission.rescan"))?;
        match outcome {
            PermissionHardeningOutcomeV1::Hardened
            | PermissionHardeningOutcomeV1::AlreadyHardened => {
                Self::permission_effect(operation_id, &permission)
            }
        }
    }

    fn observe_external(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        self.require_current_operation(operation)?;
        let operation_id = &operation.operation_id;
        if !operation.plan.external().contains(intent) {
            return Err(invalid("effect.intent-binding"));
        }
        if let Some(reconciliation) = self.observe_subscription_effect(operation_id, intent)? {
            return Ok(reconciliation);
        }
        if intent.kind() == OwnedEffectKind::Publication {
            return self.observe_publication(operation, intent);
        }
        if hiroute_application::agent_connection::is_settings_login_item(intent) {
            // The host executed the action before admission; the observation is the evidence
            // itself, so recovery reconstructs the recorded state instead of re-running it.
            return Ok(EffectReconciliation::Applied(
                hiroute_application::agent_connection::settings_login_item_effect(intent)?,
            ));
        }
        let Some(permission) = self.permission_intent(intent)? else {
            return self.artifacts.observe_artifact(operation_id, intent);
        };
        let effect = Self::permission_effect(operation_id, &permission)?;
        if operation
            .step(OperationStepKind::ApplyAgentArtifacts)
            .status
            == OperationStepStatus::Compensated
        {
            return Ok(EffectReconciliation::Missing);
        }

        if let Some(finding) = self
            .permission_findings
            .lock()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "permission.cache.lock"))?
            .get(&permission.source_ref)
            .cloned()
        {
            return if finding.observed_identity == permission.observed_identity
                && finding.observed_revision == permission.observed_revision
            {
                Ok(EffectReconciliation::Missing)
            } else {
                Ok(EffectReconciliation::OwnershipLost(effect))
            };
        }

        if self.verify_or_harden(&permission).is_err() {
            return Ok(EffectReconciliation::OwnershipLost(effect));
        }
        if operation.step(OperationStepKind::Activate).status == OperationStepStatus::Applied {
            Ok(EffectReconciliation::Applied(effect))
        } else {
            Ok(EffectReconciliation::Staged(effect))
        }
    }

    fn activate_external(
        &self,
        operation: &hiroute_domain::OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.require_current_operation(operation)?;
        if !operation
            .steps
            .iter()
            .any(|step| step.effects.contains(effect))
        {
            return Err(invalid("effect.uncommitted"));
        }
        if let Some(effect) = self.activate_subscription_effect(effect)? {
            return Ok(effect);
        }
        if effect.kind == OwnedEffectKind::Publication {
            return self.activate_publication(operation, effect);
        }
        if effect.effect_id != EFFECT_ID {
            for intent in operation.plan.external().iter().filter(|intent| {
                intent.target() == effect.target
                    && super::native_additional_model::is_settings_additional_model(intent)
            }) {
                self.validate_additional_model_dependencies(operation, intent)?;
            }
            return self.artifacts.activate_artifact(effect);
        }
        let (operation_id, intent) = self.permission_marker(effect)?;
        if operation_id != operation.operation_id {
            return Err(invalid("permission.effect.operation-binding"));
        }
        self.verify_or_harden(&intent)?;
        Ok(effect.clone())
    }

    fn compensate_external(
        &self,
        operation: &hiroute_domain::OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.require_current_operation(operation)?;
        if !operation
            .steps
            .iter()
            .any(|step| step.effects.contains(effect))
        {
            return Err(invalid("effect.uncommitted"));
        }
        if let Some(outcome) = self.compensate_subscription_effect(effect)? {
            return Ok(outcome);
        }
        if effect.kind == OwnedEffectKind::Publication {
            return self.compensate_publication(operation, effect);
        }
        if effect.effect_id != EFFECT_ID {
            return self.artifacts.compensate_artifact(effect);
        }
        let (operation_id, intent) = self.permission_marker(effect)?;
        if operation_id != operation.operation_id {
            return Err(invalid("permission.effect.operation-binding"));
        }
        // Security hardening is monotonic. Rollback re-authenticates the exact identity at 0600
        // and records compensation, but intentionally never restores an unsafe mode.
        if self.verify_or_harden(&intent).is_err() {
            return Ok(CompensationOutcome::OwnershipLost);
        }
        if operation
            .step(OperationStepKind::ApplyAgentArtifacts)
            .status
            == OperationStepStatus::Compensated
        {
            Ok(CompensationOutcome::AlreadyCompensated)
        } else {
            Ok(CompensationOutcome::Compensated)
        }
    }
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}

#[cfg(all(test, unix))]
mod tests;

impl LocalControlAdapter {
    pub(super) fn require_current_operation(
        &self,
        operation: &hiroute_domain::OperationV1,
    ) -> PortResult<()> {
        if !self
            .stores_lock()?
            .control()
            .operation_is_current(operation)?
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "operation.committed-current",
            ));
        }
        Ok(())
    }
}
