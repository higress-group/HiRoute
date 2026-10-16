//! Selected-root discovery. Legacy no-target scans never gain an unsolicited Hermes row.
use super::*;
use hiroute_domain::{AgentCapability, CanonicalDigest, CapabilityReason, CapabilityState};
use std::collections::BTreeMap;

/// Current static channel models only; native session selection/loading stays unobserved.
pub fn hermes_known_channel_models(
    target: &hiroute_domain::AgentNativeTargetV1,
) -> Result<Vec<String>, HermesNativeError> {
    use super::hermes_yaml::{HermesDocument, hermes_error};
    let target = canonical_hermes_target(target)?;
    let root = std::path::Path::new(target.config_root());
    let read = |name| {
        let observed = super::filesystem_config::read_validated_config_bytes(&root.join(name))
            .map_err(|_| hermes_error("channel configuration access"))?;
        HermesDocument::parse(observed.as_ref().map_or(&[], |(bytes, _)| bytes.as_slice()))
    };
    let config = read("config.yaml")?;
    let legacy = read("gateway.json")?;
    let mut platforms = std::collections::BTreeSet::new();
    for doc in [&config, &legacy] {
        if let Some(values) = doc.value["platforms"].as_object() {
            platforms.extend(values.keys());
        }
    }
    let mut models = std::collections::BTreeSet::new();
    for platform in platforms {
        let modern = &config.value["platforms"][platform];
        // Native config.yaml overrides a platform's legacy map only when that key is present.
        let overrides = modern
            .get("channel_overrides")
            .unwrap_or(&legacy.value["platforms"][platform]["channel_overrides"]);
        if overrides.is_null() {
            continue;
        }
        let entries = overrides
            .as_object()
            .ok_or_else(|| hermes_error("channel overrides"))?;
        if entries.len() > 128 {
            return Err(hermes_error("channel bound"));
        }
        for entry in entries.values() {
            let Some(value) = entry.get("model").filter(|value| !value.is_null()) else {
                continue;
            };
            let id = value
                .as_str()
                .ok_or_else(|| hermes_error("channel model"))?;
            if !id.is_empty() {
                if !hiroute_domain::valid_upstream_model_id(id) {
                    return Err(hermes_error("channel model"));
                }
                models.insert(id.to_owned());
            }
        }
    }
    Ok(models.into_iter().collect())
}

pub fn hermes_settings_installation(
    target: &hiroute_domain::AgentNativeTargetV1,
) -> Result<hiroute_domain::SupportedAgentInstallationV1, HermesNativeError> {
    hermes_installation(target, true, None)
}

pub fn hermes_settings_installation_for_protocol(
    target: &hiroute_domain::AgentNativeTargetV1,
    protocol: hiroute_domain::AgentIngressProtocolV1,
) -> Result<hiroute_domain::SupportedAgentInstallationV1, HermesNativeError> {
    hermes_installation(target, true, Some(protocol))
}

pub fn hermes_management_installation(
    target: &hiroute_domain::AgentNativeTargetV1,
) -> Result<hiroute_domain::SupportedAgentInstallationV1, HermesNativeError> {
    hermes_installation(target, false, None)
}

fn hermes_installation(
    target: &hiroute_domain::AgentNativeTargetV1,
    model: bool,
    protocol: Option<hiroute_domain::AgentIngressProtocolV1>,
) -> Result<hiroute_domain::SupportedAgentInstallationV1, HermesNativeError> {
    let target = canonical_hermes_target(target)?;
    let root = target.config_root();
    let config = if model {
        Some(hermes_configuration_digest(root)?)
    } else {
        None
    };
    let support = hermes_support_facts();
    let mut profile = hermes_profile_v1();
    profile.profile_id = hermes_subject(root)?.profile_id().to_owned();
    let digest = CanonicalDigest::of(&(
        "hermes-profile-observation/v1",
        root,
        config,
        &support.dependency_digest,
    ))
    .map_err(|_| hermes_yaml::hermes_error("profile observation"))?;
    let mut installation = hiroute_domain::SupportedAgentInstallationV1 {
        agent_id: "agent_hermes_default".into(),
        version: support.version,
        profile,
        effective_config: BTreeMap::new(),
        observation_digest: digest,
        capability_evidence: Vec::new(),
    };
    observed_capabilities::attach_target_file_capabilities(
        &mut installation,
        &std::path::Path::new(root).join(if model {
            "config.yaml"
        } else {
            "skills/hiroute-management/SKILL.md"
        }),
    );
    for proof in &mut installation.capability_evidence {
        let available = match proof.capability {
            AgentCapability::IngressAuthentication => Some(
                support.middleware
                    && match protocol {
                        Some(hiroute_domain::AgentIngressProtocolV1::ChatCompletions) => {
                            support.chat_sdk
                        }
                        Some(hiroute_domain::AgentIngressProtocolV1::Responses) => {
                            support.responses_sdk
                        }
                        Some(hiroute_domain::AgentIngressProtocolV1::Messages) => {
                            support.messages_sdk
                        }
                        None => true,
                    },
            ),
            AgentCapability::SkillLoading => Some(support.skills),
            AgentCapability::TrustedCliExecution => Some(support.shell),
            _ => None,
        };
        if let Some(available) = available {
            proof.state = if available {
                CapabilityState::Proven
            } else {
                CapabilityState::Unavailable
            };
            proof.reason = (!available).then_some(CapabilityReason::UnsupportedNativeFeature);
            proof.adapter_contract = "hiroute.hermes-native/v1".into();
        }
    }
    Ok(installation)
}
