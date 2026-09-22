//! Wire confirmation for independent main-Agent settings.
use hiroute_domain::CanonicalDigest;
pub use hiroute_domain::{
    AGENT_SETTINGS_SCHEMA_V2, AgentAccessTokenIntentV1, AgentClaudePresetMappingsV2,
    AgentClaudePresetSelectionV2, AgentClaudePresetValuesV2, AgentCollaborationSelectionV2,
    AgentCollaborationTriggerModeV2, AgentFacetIntent, AgentFixedModelSelectionV2,
    AgentModelDefaultSelectionV2, AgentModelSelectionV2, AgentModelSurfaceV2, AgentSettingsFacet,
    AgentSettingsSpecV2, CodexNativeModelModeV2,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsApplyV2 {
    pub expected_revisions: hiroute_domain::RevisionSetV1,
    pub spec: AgentSettingsSpecV2,
    pub accept_digest: CanonicalDigest,
    pub dependency_digest: CanonicalDigest,
    pub idempotency_key: String,
    /// Host-declared evidence that the resident login item was really checked and established
    /// for this apply. Required exactly when the backend facts mark this the first managed
    /// connection carrying the resident service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_item: Option<AgentLoginItemDeclarationV2>,
}

/// Observed main-app login-item state, mirroring the native ServiceManagement status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoginItemStatusV2 {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

impl AgentLoginItemStatusV2 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRegistered => "not_registered",
            Self::Enabled => "enabled",
            Self::RequiresApproval => "requires_approval",
            Self::NotFound => "not_found",
        }
    }
}

/// The Desktop host's own observation of the login item around a confirmed settings apply.
/// The host performs the real status check and any registration before the Operation is
/// sealed; the daemon never trusts a declaration that does not prove an active item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLoginItemDeclarationV2 {
    /// Status observed before any host action in this apply.
    pub before: AgentLoginItemStatusV2,
    /// Status observed after the host action.
    pub after: AgentLoginItemStatusV2,
    /// True only when this apply's host action created the login item; only a creation is
    /// compensable by this operation, and a pre-existing user item is never removed.
    pub created: bool,
}

impl AgentLoginItemDeclarationV2 {
    /// An acceptable declaration must prove an active login item: either the item was already
    /// enabled (not owned by this apply), or the host registered it and reached enabled.
    /// Unapproved, missing, or failed registration leaves the resident service unavailable.
    pub fn establishes_resident_service(&self) -> bool {
        if self.after != AgentLoginItemStatusV2::Enabled {
            return false;
        }
        match self.before {
            AgentLoginItemStatusV2::Enabled => !self.created,
            AgentLoginItemStatusV2::NotRegistered | AgentLoginItemStatusV2::NotFound => {
                self.created
            }
            AgentLoginItemStatusV2::RequiresApproval => false,
        }
    }

    /// A removal declaration proves the host left the login item not active — the last managed
    /// connection's restore releases the item this feature owns. Ownership itself is never
    /// proven here: the backend only requires a removal when journal evidence shows this
    /// feature created the item, and an item the user already removed manually is already
    /// absent.
    pub fn removes_resident_service(&self) -> bool {
        !self.created
            && matches!(
                self.after,
                AgentLoginItemStatusV2::NotRegistered | AgentLoginItemStatusV2::NotFound
            )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsPreviewRequestV2 {
    pub spec: AgentSettingsSpecV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsStatusRequestV2 {
    pub schema_version: hiroute_domain::SchemaVersion,
    pub context_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSettingsStateV2 {
    NotConfigured,
    Configured,
    Restored,
    Drift,
    Pending,
    NeedsAttention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelSettingsStatusV2 {
    pub schema: String,
    pub context_id: String,
    pub state: AgentModelSettingsStateV2,
    pub operation_id: Option<String>,
    pub operation_state: Option<String>,
    pub restore_point_ref: Option<String>,
    pub applied_revision: Option<hiroute_domain::GatewayPublicationRevision>,
    pub surface_results: Vec<AgentModelSurfaceResultV2>,
    /// Exact current-revision model names the backend can offer for one explicit Live check per
    /// currently detected surface. The host shows this scope and call count before consent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_check_targets: Vec<crate::AgentModelCheckTargetV2>,
    /// Configuration and Gateway installation do not prove a real upstream model invocation.
    pub model_verified: bool,
    /// Present only while current managed configuration and authority agree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_selection: Option<AgentModelSelectionV2>,
    /// Original Codex names whose fixed account/model bindings are sealed by the active setup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_native_model_ids: Vec<String>,
    /// Independent collaboration state. Older servers omit it instead of implying that model
    /// configuration also grants Worker authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration: Option<AgentCollaborationSettingsStatusV2>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelCheckStateV2 {
    NotVerified,
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelSurfaceResultV2 {
    pub surface: AgentModelSurfaceV2,
    pub applied_revision: hiroute_domain::GatewayPublicationRevision,
    pub state: AgentModelCheckStateV2,
    pub reason_code: Option<String>,
}

impl AgentModelSettingsStatusV2 {
    pub fn derive_model_verified(
        &self,
        current_revision: Option<hiroute_domain::GatewayPublicationRevision>,
    ) -> bool {
        let Some(revision) = self.applied_revision else {
            return false;
        };
        let Some(selection) = &self.current_selection else {
            return false;
        };
        if self.state != AgentModelSettingsStateV2::Configured
            || current_revision != Some(revision)
            || selection.validate().is_err()
            || self.surface_results.is_empty()
        {
            return false;
        }
        let valid_surface = |surface: &AgentModelSurfaceV2| match selection {
            AgentModelSelectionV2::CodexDefault { .. } => matches!(
                surface,
                AgentModelSurfaceV2::CodexCli | AgentModelSurfaceV2::CodexDesktop
            ),
            AgentModelSelectionV2::ClaudeLauncher { surfaces, .. } => surfaces.contains(surface),
        };
        let mut seen = std::collections::BTreeSet::new();
        self.surface_results.iter().all(|result| {
            valid_surface(&result.surface)
                && seen.insert(result.surface)
                && result.applied_revision == revision
                && result.state == AgentModelCheckStateV2::Passed
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationSettingsStatusV2 {
    pub schema: String,
    pub state: AgentModelSettingsStateV2,
    pub operation_id: Option<String>,
    pub operation_state: Option<String>,
    pub restore_point_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_selection: Option<AgentCollaborationCurrentSelectionV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationEffectV2 {
    pub action: String,
    pub trigger_mode: AgentCollaborationTriggerModeV2,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCollaborationCurrentSelectionV2 {
    pub trigger_mode: AgentCollaborationTriggerModeV2,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{AgentPlanId, GatewayPublicationRevision};

    #[test]
    fn only_an_active_login_item_proves_resident_service() {
        let declaration = |before, after, created| AgentLoginItemDeclarationV2 {
            before,
            after,
            created,
        };
        let enabled = AgentLoginItemStatusV2::Enabled;
        let not_registered = AgentLoginItemStatusV2::NotRegistered;
        assert!(declaration(enabled, enabled, false).establishes_resident_service());
        assert!(declaration(not_registered, enabled, true).establishes_resident_service());
        assert!(
            declaration(AgentLoginItemStatusV2::NotFound, enabled, true)
                .establishes_resident_service()
        );
        // Registration failed, was not performed, or is still pending approval.
        assert!(!declaration(not_registered, not_registered, false).establishes_resident_service());
        assert!(
            !declaration(
                not_registered,
                AgentLoginItemStatusV2::RequiresApproval,
                true
            )
            .establishes_resident_service()
        );
        assert!(
            !declaration(
                AgentLoginItemStatusV2::RequiresApproval,
                AgentLoginItemStatusV2::RequiresApproval,
                false
            )
            .establishes_resident_service()
        );
        // A pre-existing item cannot be claimed as this apply's creation and vice versa.
        assert!(!declaration(enabled, enabled, true).establishes_resident_service());
        assert!(!declaration(not_registered, enabled, false).establishes_resident_service());
    }

    fn status() -> AgentModelSettingsStatusV2 {
        let revision = GatewayPublicationRevision::new(7).unwrap();
        AgentModelSettingsStatusV2 {
            schema: "hiroute.agent-model-settings-status/v2".into(),
            context_id: "agent-context/test".into(),
            state: AgentModelSettingsStateV2::Configured,
            operation_id: None,
            operation_state: None,
            restore_point_ref: None,
            applied_revision: Some(revision),
            surface_results: [
                AgentModelSurfaceV2::CodexCli,
                AgentModelSurfaceV2::CodexDesktop,
            ]
            .into_iter()
            .map(|surface| AgentModelSurfaceResultV2 {
                surface,
                applied_revision: revision,
                state: AgentModelCheckStateV2::Passed,
                reason_code: None,
            })
            .collect(),
            live_check_targets: Vec::new(),
            model_verified: false,
            current_selection: Some(AgentModelSelectionV2::CodexDefault {
                native_model_mode: hiroute_domain::CodexNativeModelModeV2::PreserveAvailable,
                fixed_models: Vec::new(),
                allowed_plan_ids: [AgentPlanId::parse("plan/test").unwrap()].into(),
                default_selection: AgentModelDefaultSelectionV2::PreserveNative,
            }),
            protected_native_model_ids: Vec::new(),
            collaboration: None,
        }
    }

    #[test]
    fn verification_requires_each_reported_surface_at_the_current_publication() {
        let mut value = status();
        assert!(value.derive_model_verified(value.applied_revision));
        assert!(!value.derive_model_verified(Some(GatewayPublicationRevision::new(8).unwrap())));
        assert!(!value.derive_model_verified(None));
        value.surface_results[1].state = AgentModelCheckStateV2::NotVerified;
        assert!(!value.derive_model_verified(value.applied_revision));
        value.surface_results[1].state = AgentModelCheckStateV2::Failed;
        assert!(!value.derive_model_verified(value.applied_revision));
        value.surface_results[1].state = AgentModelCheckStateV2::Passed;
        value.surface_results[1].applied_revision = GatewayPublicationRevision::new(6).unwrap();
        assert!(!value.derive_model_verified(value.applied_revision));
        value = status();
        value.surface_results.truncate(1);
        assert!(value.derive_model_verified(value.applied_revision));
        value.surface_results[0].surface = AgentModelSurfaceV2::ClaudeCli;
        assert!(!value.derive_model_verified(value.applied_revision));
    }

    #[test]
    fn pending_missing_or_duplicate_results_cannot_inherit_success() {
        let mut value = status();
        value.model_verified = true;
        value.state = AgentModelSettingsStateV2::Pending;
        assert!(!value.derive_model_verified(value.applied_revision));
        value.state = AgentModelSettingsStateV2::Configured;
        value.surface_results[1] = value.surface_results[0].clone();
        assert!(!value.derive_model_verified(value.applied_revision));
        value.surface_results.clear();
        assert!(!value.derive_model_verified(value.applied_revision));
        value = status();
        value.current_selection = None;
        assert!(!value.derive_model_verified(value.applied_revision));
    }

    #[test]
    fn live_target_is_explicit_and_cannot_be_attached_to_local_challenges() {
        use crate::{
            AgentCheckRequestV1, AgentCheckScopeV1, AgentCheckSuiteV1, AgentModelCheckTargetV2,
        };
        let mut request = AgentCheckRequestV1 {
            agent_id: "agent_codex_default".into(),
            scope: AgentCheckScopeV1::Live,
            suite: AgentCheckSuiteV1::Quick,
            allow_model_call: true,
            target: None,
        };
        assert!(!request.valid_target());
        request.target = Some(AgentModelCheckTargetV2 {
            context_id: "agent-context/test".into(),
            surface: AgentModelSurfaceV2::CodexDesktop,
            expected_applied_revision: GatewayPublicationRevision::new(7).unwrap(),
            client_model_ids: vec!["Native.Model[1m]".into()],
        });
        assert!(request.valid_target());
        request
            .target
            .as_mut()
            .unwrap()
            .client_model_ids
            .push("Native.Model[1m]".into());
        assert!(!request.valid_target());
        request.target.as_mut().unwrap().client_model_ids.pop();
        request.scope = AgentCheckScopeV1::NativeAuthentication;
        request.allow_model_call = false;
        assert!(!request.valid_target());
        request.target = None;
        assert!(request.valid_target());
    }
}
