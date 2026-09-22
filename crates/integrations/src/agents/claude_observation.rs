//! Main-agent observations do not inherit the Worker's isolated settings-source policy.
use super::*;

pub(super) fn main_claude_observation(
    version: &str,
    observations: &[ObservedClaudeSettings],
) -> (AgentDiscoveryOutcomeV1, bool) {
    main_claude_observation_with_format(version, observations, false)
}

pub(super) fn main_claude_observation_with_format(
    version: &str,
    observations: &[ObservedClaudeSettings],
    legacy: bool,
) -> (AgentDiscoveryOutcomeV1, bool) {
    let mut observation = base_observation(
        "agent_claude_default",
        AgentKindV1::ClaudeCode,
        version.to_owned(),
    );
    for observed in observations {
        for (path, value) in [
            (
                "env.ANTHROPIC_BASE_URL",
                observed.settings.env.base_url.as_deref(),
            ),
            (
                "env.ANTHROPIC_MODEL",
                observed.settings.env.model.as_deref(),
            ),
            (
                "env.ANTHROPIC_DEFAULT_OPUS_MODEL",
                observed.settings.env.default_opus_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_DEFAULT_SONNET_MODEL",
                observed.settings.env.default_sonnet_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_DEFAULT_HAIKU_MODEL",
                observed.settings.env.default_haiku_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_SMALL_FAST_MODEL",
                observed.settings.env.small_fast_model.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                observation.config.push(ConfigObservationV1 {
                    path: path.to_owned(),
                    layer: observed.layer,
                    value: json!(value),
                    source_digest: observed.digest.clone(),
                });
            }
        }
        if observed.settings.api_key_helper_present {
            observation.config.push(ConfigObservationV1 {
                path: "apiKeyHelper".to_owned(),
                layer: observed.layer,
                // Helper commands can contain credentials or arbitrary shell text. Presence
                // is sufficient for optimistic ownership without exposing the command.
                value: json!({"configured": true}),
                source_digest: observed.digest.clone(),
            });
        }
        if CLAUDE_AUTH_ENVIRONMENT_FIELDS.iter().any(|name| {
            observed
                .settings
                .env
                .present_environment_fields
                .contains(*name)
        }) {
            observation.config.push(ConfigObservationV1 {
                path: "hiroute.auth_environment".to_owned(),
                layer: observed.layer,
                value: json!({"configured": true}),
                source_digest: observed.digest.clone(),
            });
        }
    }

    // Bind even fully shadowed/empty files, so removing an override cannot reuse an old preview.
    let inputs = observations
        .iter()
        .map(|item| (&item.source_ref, &item.digest, item.revision))
        .collect::<Vec<_>>();
    let digest = if legacy {
        CanonicalDigest::of(&(&observation, inputs))
    } else {
        CanonicalDigest::of(&(
            &observation.schema,
            &observation.agent_id,
            observation.kind,
            &observation.config,
            inputs,
        ))
    }
    .expect("bounded native observation");
    let mut outcome = resolve_agent_observation(observation);
    let conflict = same_layer_secret_conflict(observations)
        || matches!(
            &outcome,
            AgentDiscoveryOutcomeV1::ReportOnly {
                reason: AgentReportOnlyReasonV1::ConflictingEffectiveConfig,
                ..
            }
        );
    if !conflict {
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            installation.observation_digest = digest;
        }
        return (outcome, false);
    }
    // Preserve the physical installation and independent Worker preflight. Never project one
    // arbitrarily selected main configuration or credential from ambiguous native layers.
    let mut outcome = resolve_agent_observation(base_observation(
        "agent_claude_default",
        AgentKindV1::ClaudeCode,
        version.to_owned(),
    ));
    if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
        installation.observation_digest = digest;
        installation
            .capability_evidence
            .push(hiroute_domain::CapabilityEvidence {
                capability: hiroute_domain::AgentCapability::EffectiveConfiguration,
                state: hiroute_domain::CapabilityState::Unknown,
                reason: Some(hiroute_domain::CapabilityReason::HigherPrecedenceConflict),
                adapter_contract: "hiroute.claude-native-layers/v1".into(),
                observed_at_unix_ms: 1,
                dependency_digest: installation.observation_digest.clone(),
            });
    }
    (outcome, true)
}

/// Only the explicitly selected native local/project pair shares this precedence relation.
/// Unrelated project roots remain ambiguous rather than being ordered by an arbitrary Vec.
pub(super) fn resolve_project_local_fields(
    layout: &AgentFilesystemLayoutV1,
    observations: &[ObservedClaudeSettings],
) -> Vec<ObservedClaudeSettings> {
    let mut effective = observations.to_vec();
    for local in &layout.claude_project_settings {
        if local.file_name().and_then(|name| name.to_str()) != Some("settings.local.json") {
            continue;
        }
        let Some(parent) = local.parent() else {
            continue;
        };
        let project = parent.join("settings.json");
        if !layout.claude_project_settings.contains(&project) {
            continue;
        }
        let local_ref = ClaudeSource::File(ConfigLayerV1::Project, local.clone()).source_ref();
        let project_ref = ClaudeSource::File(ConfigLayerV1::Project, project).source_ref();
        let Some(local) = observations
            .iter()
            .find(|item| item.source_ref == local_ref)
        else {
            continue;
        };
        let Some(project) = effective
            .iter_mut()
            .find(|item| item.source_ref == project_ref)
        else {
            continue;
        };
        let source = &local.settings.env;
        let target = &mut project.settings.env;
        if source.base_url.is_some() {
            target.base_url = None;
        }
        if source.model.is_some() {
            target.model = None;
        }
        if source.default_opus_model.is_some() {
            target.default_opus_model = None;
        }
        if source.default_sonnet_model.is_some() {
            target.default_sonnet_model = None;
        }
        if source.default_haiku_model.is_some() {
            target.default_haiku_model = None;
        }
        if source.small_fast_model.is_some() {
            target.small_fast_model = None;
        }
        for field in &source.present_environment_fields {
            target.present_environment_fields.remove(field);
        }
        if local.settings.api_key_helper_present {
            project.settings.api_key_helper_present = false;
        }
    }
    effective
}
