use std::collections::BTreeMap;

use hiroute_domain::{
    AGENT_CONNECTION_SCHEMA_V1, AgentAccessGrantMutationV1, AgentAccessGrantScopeV1,
    AgentActivationModeV1, AgentConfigChangeV1, AgentConfigDocumentV1,
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionError,
    AgentConnectionTransactionKindV1, AgentConnectionTransactionSubjectV1, AgentConnectionV1,
    AgentDiscoveryError, AgentIngressProtocolV1, AgentPlanAllowedScopeV1, AgentPlanCatalogV1,
    AgentPlanGrantV1, AgentProfileError, CanonicalDigest, CatalogDeliveryV1, CatalogError,
    ChangeSpecV1, ExternalEffectIntentV1, FunctionToolV1, GuidanceRewriteDispositionV1,
    GuidanceRewriteResultV1, PublishedAgentPlanV1, RevisionSetV1, RoutingInstructionOverlayV1,
    SupportedAgentInstallationV1, TransactionPlanV1, WorkspaceId, rewrite_spawn_guidance,
};
use http::Uri;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use super::{AGENT_CONNECT_SPEC_SCHEMA_V1, AgentConnectSpecV1};

const ROUTING_SKILL_CONTENT: &str =
    include_str!("../../../../assets/skills/hiroute-routing/SKILL.md");
const ROUTING_SKILL_SCHEMA_V1: &str = "hiroute.routing-skill/v1";
const GATEWAY_BASE_PATH: &str = "/v1";

#[derive(Clone, Debug, PartialEq)]
pub struct AgentConnectionPlanningFactsV1 {
    pub installation: SupportedAgentInstallationV1,
    pub gateway: RegisteredGatewayEndpointV1,
    pub publication_digest: CanonicalDigest,
    pub plans: Vec<PublishedAgentPlanV1>,
    pub plan_route_digests: BTreeMap<hiroute_domain::AgentPlanId, CanonicalDigest>,
    pub current_config: AgentConfigDocumentV1,
    pub expected_revisions: RevisionSetV1,
    pub before_fingerprints: AgentConnectionBeforeFingerprintsV1,
    pub registered_tools: Vec<FunctionToolV1>,
    pub activation_mode: AgentActivationModeV1,
    /// Zero means no active local grant. A non-zero value is the exact generation that Apply
    /// must still observe before it may reuse or rotate the grant.
    pub expected_grant_generation: u64,
}

impl AgentConnectionPlanningFactsV1 {
    /// A fresh scan has a new diagnostic observation time even when every consumed fact is
    /// unchanged. Expired or replaced proofs must still change their state/dependency identity.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        let mut first = self.clone();
        let mut second = other.clone();
        for proof in first
            .installation
            .capability_evidence
            .iter_mut()
            .chain(second.installation.capability_evidence.iter_mut())
        {
            proof.observed_at_unix_ms = 0;
        }
        for facts in [&mut first, &mut second] {
            facts.installation.version.clear();
            facts.installation.profile.legacy_exact_versions.clear();
            if let Some(profile) = &mut facts.installation.profile.managed_launch {
                profile.legacy_exact_version.clear();
            }
        }
        first == second
    }
}

/// Exact adapter facts plus user-visible, content-free managed-launch diagnostics.
/// Application uses this wrapper to keep integration observations out of the planner contract.
#[derive(Clone, Debug)]
pub struct AgentConnectionPlanningInputV1 {
    pub facts: AgentConnectionPlanningFactsV1,
    pub warnings: Vec<hiroute_application_api::WarningV1>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegisteredGatewayEndpointV1 {
    pub access_point_ref: String,
    pub base_endpoint: String,
    /// Exact trusted CLI executable used by native Agent auth helpers. It is a composition fact,
    /// never caller input, and is included in the accepted change digest.
    pub trusted_cli_executable: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentConnectionBeforeFingerprintsV1 {
    pub grant_publication: Option<CanonicalDigest>,
    pub managed_configuration: Option<CanonicalDigest>,
    pub model_catalog: Option<CanonicalDigest>,
    pub routing_skill: Option<CanonicalDigest>,
    pub instruction_overlay: Option<CanonicalDigest>,
    pub spawn_guidance: Option<CanonicalDigest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewEffectStateV1 {
    Planned,
    NoFieldChange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewEffectV1 {
    pub role: String,
    pub state: PreviewEffectStateV1,
    pub desired_digest: CanonicalDigest,
}

#[derive(Clone, Debug)]
pub struct AgentConnectionPreviewV1 {
    pub connection: AgentConnectionV1,
    pub config_change: AgentConfigChangeV1,
    pub catalog: Option<AgentPlanCatalogV1>,
    pub overlay: Option<RoutingInstructionOverlayV1>,
    pub catalog_delivery: Option<CatalogDeliveryV1>,
    pub catalog_etag: Option<CanonicalDigest>,
    pub effective_after: Option<CatalogEffectiveAfterV1>,
    pub rewrite: GuidanceRewriteResultV1,
    pub change_spec: ChangeSpecV1,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub effects: Vec<PreviewEffectV1>,
    pub(super) transaction_inputs: TransactionInputsV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEffectiveAfterV1 {
    Immediate,
    AgentRestart,
}

#[derive(Clone, Debug)]
pub(super) struct TransactionInputsV1 {
    pub(super) control: AgentConnectionControlPayloadV1,
    pub(super) model_grant: hiroute_domain::AgentModelGrantV2,
    pub(super) effects: Vec<PlannedEffectV1>,
}

#[derive(Clone, Debug)]
pub(super) struct PlannedEffectV1 {
    pub(super) role: AgentConnectionEffectRoleV1,
    pub(super) before_fingerprint: Option<CanonicalDigest>,
    pub(super) payload: Value,
    pub(super) mode: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct AgentConnectionControlPayloadV1 {
    connection: AgentConnectionV1,
    publication_digest: CanonicalDigest,
    config_change_digest: CanonicalDigest,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalog_delivery: Option<CatalogDeliveryV1>,
    guidance_disposition: GuidanceRewriteDispositionV1,
}

#[derive(Clone, Debug, Serialize)]
struct GrantPublicationPayloadV1 {
    connection: AgentConnectionV1,
    publication_digest: CanonicalDigest,
    access_point_ref: String,
}

#[derive(Clone, Debug, Serialize)]
struct ManagedConfigurationPayloadV1 {
    profile_id: String,
    change: AgentConfigChangeV1,
}

#[derive(Clone, Debug, Serialize)]
struct ModelCatalogPayloadV1 {
    delivery: CatalogDeliveryV1,
    client_version: String,
    renderer_ref: String,
    catalog_revision: u64,
    etag: CanonicalDigest,
    effective_after: CatalogEffectiveAfterV1,
    catalog: AgentPlanCatalogV1,
}

#[derive(Clone, Debug, Serialize)]
struct RoutingSkillPayloadV1 {
    schema: &'static str,
    version: u16,
    content: &'static str,
    content_digest: CanonicalDigest,
    grant_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Serialize)]
struct InstructionOverlayPayloadV1 {
    overlay: RoutingInstructionOverlayV1,
    grant_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Serialize)]
struct SpawnGuidancePayloadV1 {
    disposition: GuidanceRewriteDispositionV1,
    registration_id: String,
    shape_digest: CanonicalDigest,
    previous_description_digest: CanonicalDigest,
    replacement_description: String,
    replacement_description_digest: CanonicalDigest,
}

pub struct AgentConnectionPlanner;

impl AgentConnectionPlanner {
    pub fn preview_connect(
        spec: AgentConnectSpecV1,
        facts: AgentConnectionPlanningFactsV1,
    ) -> Result<AgentConnectionPreviewV1, AgentConnectionPlanningError> {
        validate_wire_spec(&spec, &facts)?;
        facts
            .installation
            .require_action(hiroute_domain::AgentAction::ConfigureModel)
            .map_err(AgentConnectionPlanningError::UnprovenCapabilities)?;
        let profile = &facts.installation.profile;
        let allowed_agent_plan_ids = match spec.allowed_scope {
            AgentPlanAllowedScopeV1::Selected => spec.allowed_agent_plan_ids.clone(),
            AgentPlanAllowedScopeV1::AllPublished if spec.allowed_agent_plan_ids.is_empty() => {
                facts
                    .plans
                    .iter()
                    .filter(|plan| {
                        plan.active && plan.supported_ingress.contains(&profile.client_protocol())
                    })
                    .map(|plan| plan.agent_plan_id.clone())
                    .collect()
            }
            AgentPlanAllowedScopeV1::AllPublished => {
                return Err(AgentConnectionPlanningError::InvalidGrant);
            }
        };
        let grant = AgentPlanGrantV1::derive_with_scope(
            profile.client_protocol(),
            spec.allowed_scope,
            spec.default_agent_plan_id.clone(),
            allowed_agent_plan_ids,
            &facts.plans,
        )?;

        let routes = grant
            .aliases
            .iter()
            .map(|(plan_id, alias)| {
                let plan = facts
                    .plans
                    .iter()
                    .find(|plan| &plan.agent_plan_id == plan_id)
                    .ok_or(AgentConnectionPlanningError::InvalidGrant)?;
                let semantic_digest = facts
                    .plan_route_digests
                    .get(plan_id)
                    .ok_or(AgentConnectionPlanningError::InvalidGrant)?;
                Ok((
                    alias.as_str().to_owned(),
                    hiroute_domain::AgentModelRouteV2::Plan {
                        plan_id: plan_id.clone(),
                        alias: alias.clone(),
                        revision: plan.agent_plan_revision,
                        semantic_digest: semantic_digest.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, AgentConnectionPlanningError>>()?;
        let model_grant =
            hiroute_domain::AgentModelGrantV2::seal(profile.client_protocol(), routes)?;

        let (catalog, overlay, catalog_delivery) = if spec.native_subagent_routing {
            if !profile.native_subagent_routing {
                return Err(AgentConnectionPlanningError::NativeRoutingUnsupported);
            }
            let delivery = profile
                .catalog_delivery(spec.dynamic_catalog_available)
                .ok_or(AgentConnectionPlanningError::CatalogUnavailable)?;
            let catalog =
                AgentPlanCatalogV1::from_grant(&grant, profile.client_protocol(), &facts.plans)?;
            let overlay = RoutingInstructionOverlayV1::render(&catalog)?;
            (Some(catalog), Some(overlay), Some(delivery))
        } else {
            (None, None, None)
        };

        let desired_config = match facts.activation_mode {
            AgentActivationModeV1::ManagedConfiguration => desired_config(
                profile.client_protocol(),
                &facts,
                &grant,
                catalog.as_ref(),
                catalog_delivery,
            )?,
            AgentActivationModeV1::ManagedLaunch => BTreeMap::new(),
        };
        let writable = facts.installation.writable_values(&desired_config)?;
        let config_change = AgentConfigChangeV1::preview(&facts.current_config, writable)?;
        config_change.validate()?;

        let rewrite = match (&profile.spawn_guidance, &overlay) {
            (Some(guidance_profile), Some(overlay)) => {
                rewrite_spawn_guidance(guidance_profile, &facts.registered_tools, &overlay.content)
            }
            _ => GuidanceRewriteResultV1 {
                disposition: GuidanceRewriteDispositionV1::NoRegisteredTool,
                tools: facts.registered_tools.clone(),
            },
        };
        let catalog_etag = catalog.as_ref().map(|value| value.digest.clone());
        let effective_after = catalog_delivery.map(|delivery| match delivery {
            CatalogDeliveryV1::DynamicWithEtag => CatalogEffectiveAfterV1::Immediate,
            CatalogDeliveryV1::StaticRestartRequired => CatalogEffectiveAfterV1::AgentRestart,
        });
        let routing_skill_digest = spec
            .native_subagent_routing
            .then(|| CanonicalDigest::of_bytes(ROUTING_SKILL_CONTENT.as_bytes()));
        let guidance_result_digest =
            CanonicalDigest::of(&rewrite).map_err(|_| AgentConnectionPlanningError::Encoding)?;

        let connection = AgentConnectionV1 {
            schema: AGENT_CONNECTION_SCHEMA_V1.to_owned(),
            agent_id: spec.agent_id.clone(),
            profile_id: profile.profile_id.clone(),
            integration_profile_ref: profile.integration_profile_ref.clone(),
            protocol: profile.client_protocol(),
            activation_mode: facts.activation_mode,
            grant: grant.clone(),
            native_subagent_routing: spec.native_subagent_routing,
            catalog_digest: catalog.as_ref().map(|value| value.digest.clone()),
            overlay_digest: overlay.as_ref().map(|value| value.digest.clone()),
            revision: facts
                .expected_revisions
                .target
                .checked_add(1)
                .ok_or(AgentConnectionPlanningError::RevisionExhausted)?,
        };
        connection.validate()?;

        let change_spec = ChangeSpecV1 {
            schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
            command_id: AgentConnectionTransactionKindV1::Apply
                .command_id()
                .to_owned(),
            resource_id: Some(format!(
                "agent-connection/{}/{}",
                spec.agent_id, spec.profile_id
            )),
            desired_state: json!({
                "schema_version": {"major": spec.schema_version.major, "minor": spec.schema_version.minor},
                "agent_id": spec.agent_id,
                "profile_id": spec.profile_id,
                "installed_version": spec.installed_version,
                "default_agent_plan_id": spec.default_agent_plan_id,
                "allowed_scope": spec.allowed_scope,
                "allowed_agent_plan_ids": grant.allowed_agent_plan_ids,
                "native_subagent_routing": spec.native_subagent_routing,
                "catalog_delivery": catalog_delivery,
                "catalog_digest": catalog.as_ref().map(|value| &value.digest),
                "catalog_etag": catalog_etag,
                "catalog_effective_after": effective_after,
                "overlay_digest": overlay.as_ref().map(|value| &value.digest),
                "routing_skill_digest": routing_skill_digest,
                "guidance_result_digest": guidance_result_digest,
                "config_change_digest": config_change.digest,
                "integration_profile_ref": profile.integration_profile_ref,
                "protocol": profile.client_protocol(),
                "activation_mode": facts.activation_mode,
                "grant_digest": grant.digest,
                "model_grant_digest": model_grant.digest,
                "expected_grant_generation": facts.expected_grant_generation,
                "publication_digest": facts.publication_digest,
                "observation_digest": facts.installation.observation_digest,
            }),
        };
        let change_digest = change_spec
            .canonical_digest(&facts.expected_revisions)
            .map_err(|_| AgentConnectionPlanningError::Encoding)?;

        let control = AgentConnectionControlPayloadV1 {
            connection: connection.clone(),
            publication_digest: facts.publication_digest.clone(),
            config_change_digest: config_change.digest.clone(),
            catalog_delivery,
            guidance_disposition: rewrite.disposition,
        };
        let mut planned = vec![
            planned_effect(
                AgentConnectionEffectRoleV1::GrantScopedPublication,
                facts.before_fingerprints.grant_publication,
                &GrantPublicationPayloadV1 {
                    connection: connection.clone(),
                    publication_digest: facts.publication_digest,
                    access_point_ref: facts.gateway.access_point_ref,
                },
                0o644,
            )?,
            planned_effect(
                AgentConnectionEffectRoleV1::ManagedConfiguration,
                facts.before_fingerprints.managed_configuration,
                &ManagedConfigurationPayloadV1 {
                    profile_id: profile.profile_id.clone(),
                    change: config_change.clone(),
                },
                0o600,
            )?,
        ];
        if let (Some(catalog), Some(overlay), Some(delivery)) =
            (catalog.clone(), overlay.clone(), catalog_delivery)
        {
            planned.push(planned_effect(
                AgentConnectionEffectRoleV1::ModelCatalog,
                facts.before_fingerprints.model_catalog,
                &ModelCatalogPayloadV1 {
                    delivery,
                    // Diagnostic output must not change an accepted configuration payload.
                    client_version: "not-probed".into(),
                    renderer_ref: format!(
                        "{}/catalog-renderer/v1",
                        profile.integration_profile_ref
                    ),
                    catalog_revision: connection.revision,
                    etag: catalog.digest.clone(),
                    effective_after: effective_after
                        .ok_or(AgentConnectionPlanningError::CatalogUnavailable)?,
                    catalog,
                },
                0o644,
            )?);
            let skill_digest = routing_skill_digest
                .clone()
                .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
            planned.push(planned_effect(
                AgentConnectionEffectRoleV1::RoutingSkill,
                facts.before_fingerprints.routing_skill,
                &RoutingSkillPayloadV1 {
                    schema: ROUTING_SKILL_SCHEMA_V1,
                    version: 1,
                    content: ROUTING_SKILL_CONTENT,
                    content_digest: skill_digest,
                    grant_digest: grant.digest.clone(),
                },
                0o644,
            )?);
            planned.push(planned_effect(
                AgentConnectionEffectRoleV1::InstructionOverlay,
                facts.before_fingerprints.instruction_overlay,
                &InstructionOverlayPayloadV1 {
                    overlay,
                    grant_digest: grant.digest,
                },
                0o644,
            )?);
            if rewrite.disposition == GuidanceRewriteDispositionV1::Rewritten {
                let guidance_profile = profile
                    .spawn_guidance
                    .as_ref()
                    .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
                let previous_tool = facts
                    .registered_tools
                    .iter()
                    .find(|tool| tool.registration_id == guidance_profile.registration_id)
                    .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
                let replacement_tool = rewrite
                    .tools
                    .iter()
                    .find(|tool| tool.registration_id == guidance_profile.registration_id)
                    .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
                planned.push(planned_effect(
                    AgentConnectionEffectRoleV1::SpawnGuidanceRewrite,
                    facts.before_fingerprints.spawn_guidance,
                    &SpawnGuidancePayloadV1 {
                        disposition: rewrite.disposition,
                        registration_id: guidance_profile.registration_id.clone(),
                        shape_digest: previous_tool
                            .shape_digest()
                            .map_err(|_| AgentConnectionPlanningError::Encoding)?,
                        previous_description_digest: CanonicalDigest::of_bytes(
                            previous_tool.description.as_bytes(),
                        ),
                        replacement_description: replacement_tool.description.clone(),
                        replacement_description_digest: CanonicalDigest::of_bytes(
                            replacement_tool.description.as_bytes(),
                        ),
                    },
                    0o644,
                )?);
            }
        }
        let effects = planned
            .iter()
            .map(|effect| PreviewEffectV1 {
                role: effect.role.as_str().to_owned(),
                state: if effect.role == AgentConnectionEffectRoleV1::ManagedConfiguration
                    && config_change.fields.is_empty()
                {
                    PreviewEffectStateV1::NoFieldChange
                } else {
                    PreviewEffectStateV1::Planned
                },
                desired_digest: CanonicalDigest::of(&effect.payload)
                    .expect("planned payload was already encoded"),
            })
            .collect();
        Ok(AgentConnectionPreviewV1 {
            connection,
            config_change,
            catalog,
            overlay,
            catalog_delivery,
            catalog_etag,
            effective_after,
            rewrite,
            change_spec,
            change_digest,
            expected_revisions: facts.expected_revisions,
            effects,
            transaction_inputs: TransactionInputsV1 {
                control,
                model_grant,
                effects: planned,
            },
        })
    }

    pub fn seal_apply(
        preview: &AgentConnectionPreviewV1,
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
            preview.connection.agent_id.clone(),
            preview.connection.profile_id.clone(),
            preview.connection.integration_profile_ref.clone(),
        )?;
        let control = AgentConnectionControlIntentV1::from_registered_planner(
            AgentConnectionTransactionKindV1::Apply,
            subject,
            &preview.change_spec,
            &preview.transaction_inputs.control,
        )?;
        let external = preview
            .transaction_inputs
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
        let grant_scope = AgentAccessGrantScopeV1::new(
            connection_id,
            preview.transaction_inputs.model_grant.clone(),
        )?;
        let grant = AgentAccessGrantMutationV1::ensure(
            WorkspaceId::DEFAULT,
            grant_scope,
            preview
                .change_spec
                .desired_state
                .get("expected_grant_generation")
                .and_then(Value::as_u64)
                .ok_or(AgentConnectionPlanningError::InvalidGrant)?,
        )?;
        TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
            preview.change_spec.clone(),
            control,
            vec![grant],
            external,
        )
        .map_err(Into::into)
    }
}

fn desired_config(
    protocol: AgentIngressProtocolV1,
    facts: &AgentConnectionPlanningFactsV1,
    grant: &AgentPlanGrantV1,
    catalog: Option<&AgentPlanCatalogV1>,
    delivery: Option<CatalogDeliveryV1>,
) -> Result<BTreeMap<String, Option<Value>>, AgentConnectionPlanningError> {
    let profile = &facts.installation.profile;
    let alias = grant
        .aliases
        .get(&grant.default_agent_plan_id)
        .ok_or(AgentConnectionPlanningError::InvalidGrant)?;
    let mut desired = BTreeMap::new();
    match protocol {
        AgentIngressProtocolV1::Responses => {
            insert_field(profile, &mut desired, "provider", Some(json!("hiroute")))?;
            insert_field(profile, &mut desired, "default_model", Some(json!(alias)))?;
            insert_field(
                profile,
                &mut desired,
                "base_endpoint",
                Some(json!(facts.gateway.base_endpoint)),
            )?;
            insert_field(profile, &mut desired, "wire_api", Some(json!("responses")))?;
            let catalog_value = match delivery {
                Some(CatalogDeliveryV1::StaticRestartRequired) => Some(json!(
                    catalog
                        .ok_or(AgentConnectionPlanningError::CatalogUnavailable)?
                        .canonical_json()?
                )),
                _ => None,
            };
            insert_field(profile, &mut desired, "static_catalog", catalog_value)?;
        }
        AgentIngressProtocolV1::Messages => {
            let gateway_origin = facts
                .gateway
                .base_endpoint
                .strip_suffix(GATEWAY_BASE_PATH)
                .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
            insert_field(
                profile,
                &mut desired,
                "base_endpoint",
                Some(json!(gateway_origin)),
            )?;
            insert_field(profile, &mut desired, "default_model", Some(json!(alias)))?;
            for field_id in [
                "default_opus_model",
                "default_sonnet_model",
                "default_haiku_model",
                "small_fast_model",
            ] {
                insert_field(profile, &mut desired, field_id, Some(json!(alias)))?;
            }
            let connection_id = format!(
                "agent-connection/{}/{}",
                facts.installation.agent_id, profile.profile_id
            );
            insert_field(
                profile,
                &mut desired,
                "auth_helper",
                Some(json!({
                    "executable": facts.gateway.trusted_cli_executable,
                    "argv": [
                        hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
                        connection_id,
                    ],
                })),
            )?;
            // The exact Claude renderer removes all prior auth/header/cloud-provider fields only
            // after their upstream credential has been persisted as a ComputeSource.
            insert_field(profile, &mut desired, "auth_environment", None)?;
        }
    }
    Ok(desired)
}

fn insert_field(
    profile: &hiroute_domain::AgentProfileV1,
    desired: &mut BTreeMap<String, Option<Value>>,
    field_id: &str,
    value: Option<Value>,
) -> Result<(), AgentConnectionPlanningError> {
    let field = profile
        .field(field_id)
        .ok_or(AgentConnectionPlanningError::ProfileContractDrift)?;
    desired.insert(field.path.clone(), value);
    Ok(())
}

fn planned_effect<T: Serialize>(
    role: AgentConnectionEffectRoleV1,
    before_fingerprint: Option<CanonicalDigest>,
    payload: &T,
    mode: u32,
) -> Result<PlannedEffectV1, AgentConnectionPlanningError> {
    Ok(PlannedEffectV1 {
        role,
        before_fingerprint,
        payload: serde_json::to_value(payload)
            .map_err(|_| AgentConnectionPlanningError::Encoding)?,
        mode,
    })
}

fn validate_wire_spec(
    spec: &AgentConnectSpecV1,
    facts: &AgentConnectionPlanningFactsV1,
) -> Result<(), AgentConnectionPlanningError> {
    let profile = &facts.installation.profile;
    profile.validate()?;
    if AGENT_CONNECT_SPEC_SCHEMA_V1
        .negotiate(spec.schema_version)
        .is_none()
    {
        return Err(AgentConnectionPlanningError::SchemaIncompatible);
    }
    if spec.agent_id != facts.installation.agent_id
        || spec.profile_id != profile.profile_id
        || !valid_registered_endpoint(&facts.gateway)
    {
        return Err(AgentConnectionPlanningError::ProfileMismatch);
    }
    Ok(())
}

fn valid_registered_endpoint(endpoint: &RegisteredGatewayEndpointV1) -> bool {
    !endpoint.access_point_ref.is_empty()
        && endpoint.access_point_ref.len() <= 256
        && !endpoint.access_point_ref.contains("..")
        && endpoint
            .access_point_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && valid_gateway_base_endpoint(&endpoint.base_endpoint)
}

fn valid_gateway_base_endpoint(value: &str) -> bool {
    let Ok(uri) = value.parse::<Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    if uri.scheme_str() != Some("http")
        || authority.as_str().contains('@')
        || uri.path() != GATEWAY_BASE_PATH
        || uri.query().is_some()
    {
        return false;
    }
    let Some(port) = authority.port_u16() else {
        return false;
    };
    if port == 0 {
        return false;
    }
    let host = match authority.host() {
        "127.0.0.1" => "127.0.0.1",
        "[::1]" => "[::1]",
        _ => return false,
    };

    // Keep the registered P0 spelling exact; URI parsing accepts alternate port/IP spellings and
    // discards fragments, none of which may become a second Agent endpoint form.
    value == format!("http://{host}:{port}{GATEWAY_BASE_PATH}")
}

#[derive(Debug, Error)]
pub enum AgentConnectionPlanningError {
    #[error("native action capabilities are missing, unavailable or stale: {0:?}")]
    UnprovenCapabilities(Vec<hiroute_domain::CapabilityBlock>),
    #[error("AgentConnection spec schema is incompatible")]
    SchemaIncompatible,
    #[error("AgentConnection input does not match the exact discovered profile")]
    ProfileMismatch,
    #[error("the exact Agent profile does not support native subagent routing")]
    NativeRoutingUnsupported,
    #[error("the exact Agent profile has no supported catalog path")]
    CatalogUnavailable,
    #[error("the exact Agent profile config contract is incomplete")]
    ProfileContractDrift,
    #[error("the AgentPlan grant is invalid")]
    InvalidGrant,
    #[error("accepted Preview digest or revisions are stale")]
    PreviewStale,
    #[error("AgentConnection revision is exhausted")]
    RevisionExhausted,
    #[error("canonical encoding failed")]
    Encoding,
    #[error(transparent)]
    Profile(#[from] AgentProfileError),
    #[error(transparent)]
    Connection(#[from] AgentConnectionError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Configuration(#[from] hiroute_domain::AgentConfigError),
    #[error(transparent)]
    Discovery(#[from] AgentDiscoveryError),
    #[error(transparent)]
    Operation(#[from] hiroute_domain::OperationValidationError),
}
