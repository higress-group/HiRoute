use hiroute_domain::{
    AgentAccessGrantMutationV1, AgentConfigRestorePointV1, AgentConnectionControlIntentV1,
    AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    AgentConnectionTransactionSubjectV1, CanonicalDigest, ChangeSpecV1, ExternalEffectIntentV1,
    RevisionSetV1, SupportedAgentInstallationV1, TransactionPlanV1, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    AGENT_CONNECT_SPEC_SCHEMA_V1, AgentConnectionBeforeFingerprintsV1,
    AgentConnectionPlanningError, AgentConnectionPreviewV1, AgentRestoreSpecV1,
};

pub const AGENT_CONNECTION_RESTORE_POINT_SCHEMA_V1: &str =
    "hiroute.agent-connection-restore-point/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentArtifactRestoreTargetV1 {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_digest: Option<CanonicalDigest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionRestorePointV1 {
    pub schema: String,
    pub restore_point_ref: String,
    pub agent_id: String,
    pub profile_id: String,
    pub integration_profile_ref: String,
    pub installed_version: String,
    pub source_change_digest: CanonicalDigest,
    pub configuration: AgentConfigRestorePointV1,
    pub targets: Vec<AgentArtifactRestoreTargetV1>,
    pub digest: CanonicalDigest,
}

impl AgentConnectionRestorePointV1 {
    pub fn from_preview(
        restore_point_ref: impl Into<String>,
        installed_version: impl Into<String>,
        preview: &AgentConnectionPreviewV1,
        configuration: AgentConfigRestorePointV1,
    ) -> Result<Self, AgentConnectionPlanningError> {
        let restore_point_ref = restore_point_ref.into();
        let installed_version = installed_version.into();
        if configuration.profile_id != preview.connection.profile_id
            || configuration.change.digest != preview.config_change.digest
        {
            return Err(AgentConnectionPlanningError::ProfileMismatch);
        }
        let targets = preview
            .transaction_inputs
            .effects
            .iter()
            .filter(|effect| effect.role != AgentConnectionEffectRoleV1::ManagedConfiguration)
            .map(|effect| AgentArtifactRestoreTargetV1 {
                role: effect.role.as_str().to_owned(),
                previous_digest: effect.before_fingerprint.clone(),
            })
            .collect::<Vec<_>>();
        let mut value = Self {
            schema: AGENT_CONNECTION_RESTORE_POINT_SCHEMA_V1.to_owned(),
            restore_point_ref,
            agent_id: preview.connection.agent_id.clone(),
            profile_id: preview.connection.profile_id.clone(),
            integration_profile_ref: preview.connection.integration_profile_ref.clone(),
            installed_version,
            source_change_digest: preview.change_digest.clone(),
            configuration,
            targets,
            digest: CanonicalDigest::of_bytes(b"pending-agent-restore-point"),
        };
        value.validate_without_digest()?;
        value.digest = value.body_digest()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), AgentConnectionPlanningError> {
        self.validate_without_digest()?;
        if self.body_digest()? != self.digest {
            return Err(AgentConnectionPlanningError::Encoding);
        }
        Ok(())
    }

    fn validate_without_digest(&self) -> Result<(), AgentConnectionPlanningError> {
        if self.schema != AGENT_CONNECTION_RESTORE_POINT_SCHEMA_V1
            || !valid_identifier(&self.restore_point_ref)
            || !valid_identifier(&self.agent_id)
            || !valid_identifier(&self.profile_id)
            || !valid_identifier(&self.integration_profile_ref)
        {
            return Err(AgentConnectionPlanningError::ProfileMismatch);
        }
        self.configuration.validate()?;
        let roles = self
            .targets
            .iter()
            .map(|target| target.role.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let publication = roles.contains("grant_scoped_publication");
        let routing_count = ["model_catalog", "routing_skill", "instruction_overlay"]
            .into_iter()
            .filter(|role| roles.contains(role))
            .count();
        let guidance = roles.contains("spawn_guidance_rewrite");
        if !publication
            || !matches!(routing_count, 0 | 3)
            || (guidance && routing_count != 3)
            || roles.len() != self.targets.len()
            || !matches!(roles.len(), 1 | 4 | 5)
            || self.targets.iter().any(|target| {
                target.role == AgentConnectionEffectRoleV1::GrantScopedPublication.as_str()
                    && target.previous_digest.is_none()
            })
        {
            return Err(AgentConnectionPlanningError::InvalidGrant);
        }
        Ok(())
    }

    fn body_digest(&self) -> Result<CanonicalDigest, AgentConnectionPlanningError> {
        #[derive(Serialize)]
        struct Body<'a> {
            schema: &'a str,
            restore_point_ref: &'a str,
            agent_id: &'a str,
            profile_id: &'a str,
            integration_profile_ref: &'a str,
            installed_version: &'a str,
            source_change_digest: &'a CanonicalDigest,
            configuration: &'a AgentConfigRestorePointV1,
            targets: &'a [AgentArtifactRestoreTargetV1],
        }
        CanonicalDigest::of(&Body {
            schema: &self.schema,
            restore_point_ref: &self.restore_point_ref,
            agent_id: &self.agent_id,
            profile_id: &self.profile_id,
            integration_profile_ref: &self.integration_profile_ref,
            installed_version: &self.installed_version,
            source_change_digest: &self.source_change_digest,
            configuration: &self.configuration,
            targets: &self.targets,
        })
        .map_err(|_| AgentConnectionPlanningError::Encoding)
    }
}

#[derive(Clone, Debug)]
pub struct AgentConnectionRestorePlanningFactsV1 {
    pub installation: SupportedAgentInstallationV1,
    pub current_revisions: RevisionSetV1,
    pub current_fingerprints: AgentConnectionBeforeFingerprintsV1,
    /// Exact active generation that the restore must revoke. Zero represents an already
    /// disconnected connection and cannot be promoted into an executable restore operation.
    pub expected_grant_generation: u64,
}

#[derive(Clone, Debug)]
pub struct AgentConnectionRestorePreviewV1 {
    pub restore_point: AgentConnectionRestorePointV1,
    pub change_spec: ChangeSpecV1,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    effects: Vec<RestoreEffectV1>,
}

#[derive(Clone, Debug)]
struct RestoreEffectV1 {
    role: AgentConnectionEffectRoleV1,
    before_fingerprint: Option<CanonicalDigest>,
    payload: Value,
    mode: u32,
}

#[derive(Clone, Debug)]
pub enum AgentConnectionRestorePreviewOutcomeV1 {
    Ready(Box<AgentConnectionRestorePreviewV1>),
    ReportOnlyUnknownVersion,
}

pub fn preview_agent_connection_restore(
    spec: AgentRestoreSpecV1,
    restore_point: AgentConnectionRestorePointV1,
    facts: AgentConnectionRestorePlanningFactsV1,
) -> Result<AgentConnectionRestorePreviewOutcomeV1, AgentConnectionPlanningError> {
    if restore_point.schema != AGENT_CONNECTION_RESTORE_POINT_SCHEMA_V1
        || restore_point.configuration.schema != hiroute_domain::AGENT_CONFIG_RESTORE_SCHEMA_V1
    {
        return Ok(AgentConnectionRestorePreviewOutcomeV1::ReportOnlyUnknownVersion);
    }
    restore_point.validate()?;
    facts
        .installation
        .require_action(hiroute_domain::AgentAction::RestoreModel)
        .map_err(AgentConnectionPlanningError::UnprovenCapabilities)?;
    let profile = &facts.installation.profile;
    if AGENT_CONNECT_SPEC_SCHEMA_V1
        .negotiate(spec.schema_version)
        .is_none()
    {
        return Err(AgentConnectionPlanningError::SchemaIncompatible);
    }
    if spec.agent_id != restore_point.agent_id
        || spec.profile_id != restore_point.profile_id
        || spec.restore_point_ref != restore_point.restore_point_ref
        || facts.installation.agent_id != restore_point.agent_id
        || profile.profile_id != restore_point.profile_id
        || profile.integration_profile_ref != restore_point.integration_profile_ref
    {
        return Err(AgentConnectionPlanningError::ProfileMismatch);
    }
    if facts.expected_grant_generation == 0 {
        return Err(AgentConnectionPlanningError::InvalidGrant);
    }
    let change_spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: AgentConnectionTransactionKindV1::Restore
            .command_id()
            .to_owned(),
        resource_id: Some(format!(
            "agent-connection/{}/{}",
            restore_point.agent_id, restore_point.profile_id
        )),
        desired_state: json!({
            "agent_id": restore_point.agent_id,
            "profile_id": restore_point.profile_id,
            "installed_version": restore_point.installed_version,
            "restore_point_ref": restore_point.restore_point_ref,
            "restore_point_digest": restore_point.digest,
            "source_change_digest": restore_point.source_change_digest,
            "expected_grant_generation": facts.expected_grant_generation,
        }),
    };
    let change_digest = change_spec
        .canonical_digest(&facts.current_revisions)
        .map_err(|_| AgentConnectionPlanningError::Encoding)?;
    let mut effects = Vec::with_capacity(restore_point.targets.len() + 1);
    for target in &restore_point.targets {
        let role = parse_role(&target.role)?;
        effects.push(RestoreEffectV1 {
            role,
            before_fingerprint: fingerprint_for(role, &facts.current_fingerprints),
            payload: json!({
                "restore_point_ref": restore_point.restore_point_ref,
                "role": target.role,
                "previous_digest": target.previous_digest,
                "remove_if_absent": target.previous_digest.is_none(),
            }),
            mode: 0o644,
        });
    }
    effects.push(RestoreEffectV1 {
        role: AgentConnectionEffectRoleV1::ManagedConfiguration,
        before_fingerprint: facts.current_fingerprints.managed_configuration,
        payload: json!({
            "profile_id": restore_point.profile_id,
            "restore_point": restore_point.configuration,
        }),
        mode: 0o600,
    });
    Ok(AgentConnectionRestorePreviewOutcomeV1::Ready(Box::new(
        AgentConnectionRestorePreviewV1 {
            restore_point,
            change_spec,
            change_digest,
            expected_revisions: facts.current_revisions,
            effects,
        },
    )))
}

pub fn seal_agent_connection_restore(
    preview: &AgentConnectionRestorePreviewV1,
    accepted_digest: &CanonicalDigest,
    accepted_revisions: &RevisionSetV1,
    current_revisions: &RevisionSetV1,
) -> Result<TransactionPlanV1, AgentConnectionPlanningError> {
    if accepted_digest != &preview.change_digest
        || accepted_revisions != &preview.expected_revisions
        || current_revisions != &preview.expected_revisions
    {
        return Err(AgentConnectionPlanningError::PreviewStale);
    }
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        preview.restore_point.agent_id.clone(),
        preview.restore_point.profile_id.clone(),
        preview.restore_point.integration_profile_ref.clone(),
    )?;
    let control = AgentConnectionControlIntentV1::from_registered_planner(
        AgentConnectionTransactionKindV1::Restore,
        subject,
        &preview.change_spec,
        &json!({
            "restore_point_ref": preview.restore_point.restore_point_ref,
            "restore_point_digest": preview.restore_point.digest,
            "source_change_digest": preview.restore_point.source_change_digest,
        }),
    )?;
    let external = preview
        .effects
        .iter()
        .map(|effect| {
            ExternalEffectIntentV1::from_agent_connection_planner(
                &control,
                effect.role,
                effect.before_fingerprint.clone(),
                &effect.payload,
                effect.mode,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let connection_id = preview
        .change_spec
        .resource_id
        .clone()
        .ok_or(AgentConnectionPlanningError::InvalidGrant)?;
    let expected_generation = preview
        .change_spec
        .desired_state
        .get("expected_grant_generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or(AgentConnectionPlanningError::InvalidGrant)?;
    let revoke = AgentAccessGrantMutationV1::revoke(
        WorkspaceId::DEFAULT,
        connection_id,
        expected_generation,
    )?;
    TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        preview.change_spec.clone(),
        control,
        vec![revoke],
        external,
    )
    .map_err(Into::into)
}

fn parse_role(role: &str) -> Result<AgentConnectionEffectRoleV1, AgentConnectionPlanningError> {
    match role {
        "grant_scoped_publication" => Ok(AgentConnectionEffectRoleV1::GrantScopedPublication),
        "model_catalog" => Ok(AgentConnectionEffectRoleV1::ModelCatalog),
        "routing_skill" => Ok(AgentConnectionEffectRoleV1::RoutingSkill),
        "instruction_overlay" => Ok(AgentConnectionEffectRoleV1::InstructionOverlay),
        "spawn_guidance_rewrite" => Ok(AgentConnectionEffectRoleV1::SpawnGuidanceRewrite),
        _ => Err(AgentConnectionPlanningError::InvalidGrant),
    }
}

fn fingerprint_for(
    role: AgentConnectionEffectRoleV1,
    values: &AgentConnectionBeforeFingerprintsV1,
) -> Option<CanonicalDigest> {
    match role {
        AgentConnectionEffectRoleV1::GrantScopedPublication => values.grant_publication.clone(),
        AgentConnectionEffectRoleV1::ManagedConfiguration => values.managed_configuration.clone(),
        AgentConnectionEffectRoleV1::ModelCatalog => values.model_catalog.clone(),
        AgentConnectionEffectRoleV1::RoutingSkill => values.routing_skill.clone(),
        AgentConnectionEffectRoleV1::InstructionOverlay => values.instruction_overlay.clone(),
        AgentConnectionEffectRoleV1::SpawnGuidanceRewrite => values.spawn_guidance.clone(),
        // The login item is a settings-transaction host effect and never rides a V1
        // connect/restore plan, so no V1 fingerprint slot exists for it.
        AgentConnectionEffectRoleV1::LoginItem => None,
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}
