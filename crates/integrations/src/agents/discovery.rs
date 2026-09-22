use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    AgentKindV1, CanonicalDigest, ConfigLayerV1, EffectiveConfigFieldV1,
    SupportedAgentInstallationV1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::builtin_agent_profiles;

pub const AGENT_SCAN_OBSERVATION_SCHEMA_V1: &str = "hiroute.agent-scan-observation/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigObservationV1 {
    pub path: String,
    pub layer: ConfigLayerV1,
    pub value: Value,
    pub source_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentScanObservationV1 {
    pub schema: String,
    pub agent_id: String,
    pub kind: AgentKindV1,
    pub version: String,
    #[serde(default)]
    pub config: Vec<ConfigObservationV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReportOnlyReasonV1 {
    NotFoundInScope,
    ExecutableNotRunnable,
    ExecutableProbeTimedOut,
    ExecutableProbeUnavailable,
    UnknownObservationSchema,
    UnknownProfile,
    AmbiguousProfile,
    ConflictingEffectiveConfig,
    InvalidObservation,
    SymlinkConfig,
    WrongOwner,
    UnsafeConfigPermissions,
    PermissionHardeningRequired,
    ConfigUnavailable,
    UnregisteredEndpoint,
    UnregisteredModel,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentDiscoveryOutcomeV1 {
    Supported {
        installation: Box<SupportedAgentInstallationV1>,
    },
    ReportOnly {
        agent_id: String,
        kind: AgentKindV1,
        version: String,
        reason: AgentReportOnlyReasonV1,
        observation_digest: CanonicalDigest,
    },
}

pub fn resolve_agent_observation(observation: AgentScanObservationV1) -> AgentDiscoveryOutcomeV1 {
    let observation_digest = CanonicalDigest::of(&(
        &observation.schema,
        &observation.agent_id,
        observation.kind,
        &observation.config,
    ))
    .unwrap_or_else(|_| CanonicalDigest::of_bytes(b"invalid-agent-observation"));
    let report = |reason| AgentDiscoveryOutcomeV1::ReportOnly {
        agent_id: observation.agent_id.clone(),
        kind: observation.kind,
        version: observation.version.clone(),
        reason,
        observation_digest: observation_digest.clone(),
    };
    if observation.schema != AGENT_SCAN_OBSERVATION_SCHEMA_V1 {
        return report(AgentReportOnlyReasonV1::UnknownObservationSchema);
    }
    if !valid_agent_id(&observation.agent_id) {
        return report(AgentReportOnlyReasonV1::InvalidObservation);
    }
    let profiles = builtin_agent_profiles()
        .into_iter()
        .filter(|profile| profile.kind == observation.kind)
        .collect::<Vec<_>>();
    let [profile] = profiles.as_slice() else {
        return report(if profiles.is_empty() {
            AgentReportOnlyReasonV1::UnknownProfile
        } else {
            AgentReportOnlyReasonV1::AmbiguousProfile
        });
    };
    if profile.validate().is_err() {
        return report(AgentReportOnlyReasonV1::AmbiguousProfile);
    }
    let owned = profile
        .owned_config_fields
        .iter()
        .map(|field| field.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut by_path_layer: BTreeMap<(&str, ConfigLayerV1), &ConfigObservationV1> = BTreeMap::new();
    for candidate in &observation.config {
        if CanonicalDigest::parse(candidate.source_digest.as_str().to_owned()).is_err() {
            return report(AgentReportOnlyReasonV1::InvalidObservation);
        }
        if !owned.contains(candidate.path.as_str()) {
            continue;
        }
        if contains_nul(&candidate.value) {
            return report(AgentReportOnlyReasonV1::InvalidObservation);
        }
        let key = (candidate.path.as_str(), candidate.layer);
        if by_path_layer
            .insert(key, candidate)
            .is_some_and(|previous| previous.value != candidate.value)
        {
            return report(AgentReportOnlyReasonV1::ConflictingEffectiveConfig);
        }
    }
    let mut effective_config = BTreeMap::new();
    for field in &profile.owned_config_fields {
        let candidates = observation
            .config
            .iter()
            .filter(|candidate| candidate.path == field.path)
            .collect::<Vec<_>>();
        if let Some(effective) = select_effective(&candidates, observation.kind) {
            effective_config.insert(
                field.path.clone(),
                EffectiveConfigFieldV1 {
                    path: effective.path.clone(),
                    layer: effective.layer,
                    value: effective.value.clone(),
                    source_digest: effective.source_digest.clone(),
                },
            );
        }
    }
    AgentDiscoveryOutcomeV1::Supported {
        installation: Box::new(SupportedAgentInstallationV1 {
            agent_id: observation.agent_id,
            version: observation.version,
            profile: profile.clone(),
            effective_config,
            observation_digest,
            capability_evidence: Vec::new(),
        }),
    }
}

fn select_effective<'a>(
    values: &[&'a ConfigObservationV1],
    kind: AgentKindV1,
) -> Option<&'a ConfigObservationV1> {
    values
        .iter()
        .copied()
        .min_by_key(|value| value.layer.precedence_for(kind))
}

fn valid_agent_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn contains_nul(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_nul),
        Value::Object(values) => values.values().any(contains_nul),
        Value::String(value) => value.contains('\0'),
        _ => false,
    }
}
