use serde::{Deserialize, Serialize};

use crate::{ConnectorRuntimeKind, ExactNativeReasoningV1, UpstreamProtocol};

use super::materialized::{AttemptOwnedCandidateV1, CompiledPlanError};

pub const GATEWAY_OPERATIONAL_TARGET_SCHEMA_V1: &str = "hiroute.gateway-operational-target/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayOperationalTargetV1 {
    RegisteredHttps {
        uri: String,
    },
    UserConfiguredNative {
        uri: String,
    },
    ManagedCpaLoopback {
        uri: String,
        runtime_epoch: u64,
        target_epoch: u64,
    },
}

impl GatewayOperationalTargetV1 {
    pub fn uri(&self) -> &str {
        match self {
            Self::RegisteredHttps { uri }
            | Self::UserConfiguredNative { uri }
            | Self::ManagedCpaLoopback { uri, .. } => uri,
        }
    }

    pub fn runtime_epoch(&self) -> Option<u64> {
        match self {
            Self::RegisteredHttps { .. } | Self::UserConfiguredNative { .. } => None,
            Self::ManagedCpaLoopback { runtime_epoch, .. } => Some(*runtime_epoch),
        }
    }

    pub fn target_epoch(&self) -> Option<u64> {
        match self {
            Self::RegisteredHttps { .. } | Self::UserConfiguredNative { .. } => None,
            Self::ManagedCpaLoopback { target_epoch, .. } => Some(*target_epoch),
        }
    }

    pub fn request_path(&self) -> Option<&str> {
        let (_, rest) = self.uri().split_once("://")?;
        let path_start = rest.find('/')?;
        Some(&rest[path_start..])
    }

    /// A registered catalog may publish several protocol paths on the same exact HTTPS
    /// authority. The catalog-bound protocol profile chooses the path; the transport target
    /// and its credential stay scoped to that authority. User-configured targets remain exact.
    pub fn for_protocol_path(&self, path: &str) -> Option<Self> {
        match self {
            Self::RegisteredHttps { uri } => {
                if !valid_executable_endpoint(uri) {
                    return None;
                }
                let prefix = uri.strip_suffix(self.request_path()?)?;
                let selected = format!("{prefix}{path}");
                valid_executable_endpoint(&selected)
                    .then_some(Self::RegisteredHttps { uri: selected })
            }
            Self::UserConfiguredNative { .. } if self.request_path() == Some(path) => {
                Some(self.clone())
            }
            Self::UserConfiguredNative { .. } | Self::ManagedCpaLoopback { .. } => None,
        }
    }

    pub fn validate_for(&self, runtime_kind: ConnectorRuntimeKind, logical_endpoint: &str) -> bool {
        match self {
            Self::RegisteredHttps { uri } => {
                runtime_kind == ConnectorRuntimeKind::BuiltinNative
                    && valid_executable_endpoint(logical_endpoint)
                    && uri == logical_endpoint
            }
            Self::UserConfiguredNative { uri } => {
                runtime_kind == ConnectorRuntimeKind::BuiltinNative
                    && valid_user_configured_native_endpoint(logical_endpoint)
                    && uri == logical_endpoint
            }
            Self::ManagedCpaLoopback {
                uri,
                runtime_epoch,
                target_epoch,
            } => {
                runtime_kind == ConnectorRuntimeKind::CpaBridge
                    && *runtime_epoch > 0
                    && *target_epoch > 0
                    && valid_numeric_loopback_http(uri)
            }
        }
    }
}

fn valid_user_configured_native_endpoint(value: &str) -> bool {
    if valid_executable_endpoint(value) || valid_numeric_loopback_http(value) {
        return true;
    }
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return false;
    };
    valid_user_configured_https_authority(authority)
        && !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['?', '#', '\\', '%'])
        && !path.contains("//")
        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
}

fn valid_user_configured_https_authority(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 253
        || value.contains(['@', '/', '?', '#'])
        || value.chars().any(char::is_whitespace)
    {
        return false;
    }
    if let Some(rest) = value.strip_prefix('[') {
        let Some((host, suffix)) = rest.split_once(']') else {
            return false;
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        return suffix.is_empty()
            || suffix
                .strip_prefix(':')
                .and_then(|port| port.parse::<u16>().ok())
                .is_some_and(|port| port != 0);
    }
    let (host, port) = value
        .rsplit_once(':')
        .map_or((value, None), |(host, port)| (host, Some(port)));
    if host.contains(':')
        || port.is_some_and(|port| port.parse::<u16>().map_or(true, |parsed| parsed == 0))
    {
        return false;
    }
    if host
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return host.parse::<std::net::Ipv4Addr>().is_ok();
    }
    !host.is_empty()
        && !host.ends_with('.')
        && !host.contains("..")
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum GatewayCriticalFactV1<T> {
    Exact(T),
    Unknown,
}

impl<T> GatewayCriticalFactV1<T> {
    pub fn exact(&self) -> Option<&T> {
        match self {
            Self::Exact(value) => Some(value),
            Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayFidelityV1 {
    Exact,
    Normalized,
    GatewayMaterialized,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayStateAffinityV1 {
    Unsupported,
    ExactOwner,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayNativeProviderStateEmissionV1 {
    Never,
    ExactOwnerAffine,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayStreamingRefusalSemanticsV1 {
    ExactDelta,
    TerminalClassified,
    /// Preserves authenticated historical digests; execution ignores both values.
    #[doc(hidden)]
    #[serde(rename = "terminal_classified")]
    LegacyTerminalClassified {
        max_buffered_bytes: u64,
        max_buffered_blocks: u64,
    },
    Unsupported,
    Unknown,
}

impl<'de> Deserialize<'de> for GatewayStreamingRefusalSemanticsV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Read-only recovery of persisted capability snapshots and operation
        // journals. Keep their canonical bytes for digest validation; current
        // producers construct TerminalClassified without these retired quotas.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            ExactDelta,
            TerminalClassified {
                #[serde(default, rename = "max_buffered_bytes")]
                retired_bytes: Option<u64>,
                #[serde(default, rename = "max_buffered_blocks")]
                retired_blocks: Option<u64>,
            },
            Unsupported,
            Unknown,
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::ExactDelta => Self::ExactDelta,
            Wire::TerminalClassified {
                retired_bytes,
                retired_blocks,
            } => match (retired_bytes, retired_blocks) {
                (None, None) => Self::TerminalClassified,
                (Some(max_buffered_bytes), Some(max_buffered_blocks)) => {
                    Self::LegacyTerminalClassified {
                        max_buffered_bytes,
                        max_buffered_blocks,
                    }
                }
                _ => {
                    return Err(serde::de::Error::custom(
                        "incomplete retired terminal classification descriptor",
                    ));
                }
            },
            Wire::Unsupported => Self::Unsupported,
            Wire::Unknown => Self::Unknown,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayRequestFeatureProfileV1 {
    pub text: GatewayFidelityV1,
    pub initial_instructions: GatewayFidelityV1,
    pub mid_conversation_instructions: GatewayFidelityV1,
    pub image_url: GatewayFidelityV1,
    pub image_base64: GatewayFidelityV1,
    pub image_base64_media_types: GatewayCriticalFactV1<Vec<String>>,
    pub function_tools: GatewayFidelityV1,
    pub strict_tools: GatewayFidelityV1,
    pub tool_choice_none: GatewayFidelityV1,
    pub tool_choice_auto: GatewayFidelityV1,
    pub tool_choice_required_any: GatewayFidelityV1,
    pub tool_choice_required_named: GatewayFidelityV1,
    pub parallel_tools: GatewayFidelityV1,
    pub tool_roundtrip: GatewayFidelityV1,
    pub tool_result_text: GatewayFidelityV1,
    pub tool_result_json: GatewayFidelityV1,
    pub logical_tool_id_mapping: GatewayFidelityV1,
    pub provider_state: GatewayFidelityV1,
    pub state_affinity: GatewayStateAffinityV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayResponseFeatureProfileV1 {
    pub text: GatewayFidelityV1,
    pub reasoning: GatewayFidelityV1,
    pub refusal: GatewayFidelityV1,
    pub tool_calls: GatewayFidelityV1,
    pub logical_tool_id_mapping: GatewayFidelityV1,
    pub usage: GatewayFidelityV1,
    pub finish_reason: GatewayFidelityV1,
    pub typed_error: GatewayFidelityV1,
    pub provider_state: GatewayFidelityV1,
    pub state_affinity: GatewayStateAffinityV1,
    pub stream_refusal: GatewayStreamingRefusalSemanticsV1,
    pub stream_text_delta: GatewayFidelityV1,
    pub stream_tool_argument_delta: GatewayFidelityV1,
    pub stream_reasoning_delta: GatewayFidelityV1,
    pub stream_usage: GatewayFidelityV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayReasoningControlKindV1 {
    Fixed,
    Toggle,
    Discrete,
    Budget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayReasoningAccountingV1 {
    WithinOutputCap,
    Additive,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum GatewayNativeReasoningValueV1 {
    Bool(bool),
    String(String),
    U64(u64),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayNativeReasoningFieldAssignmentV1 {
    pub path: Vec<String>,
    pub value: GatewayNativeReasoningValueV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayNativeReasoningRenderV1 {
    NoControlParameter,
    ExactFields {
        protocol: UpstreamProtocol,
        fields: Vec<GatewayNativeReasoningFieldAssignmentV1>,
    },
    ExactBudget {
        protocol: UpstreamProtocol,
        fields: Vec<GatewayNativeReasoningFieldAssignmentV1>,
        budget_path: Vec<String>,
        selected_tokens: u64,
        min_tokens: u64,
        max_tokens: u64,
        step_tokens: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayReasoningProfileCapabilityV1 {
    pub profile_id: String,
    pub control_kind: GatewayReasoningControlKindV1,
    pub render: GatewayNativeReasoningRenderV1,
    pub accounting: GatewayReasoningAccountingV1,
    pub additional_reservation_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayTokenEstimatorProfileV1 {
    pub revision: String,
    pub bytes_per_token: u64,
    pub fixed_overhead_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayContextLimitsV1 {
    pub max_input_tokens: GatewayCriticalFactV1<u64>,
    pub max_output_tokens: GatewayCriticalFactV1<u64>,
    pub max_total_tokens: GatewayCriticalFactV1<Option<u64>>,
    pub estimator: GatewayCriticalFactV1<GatewayTokenEstimatorProfileV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayAuthenticationSemanticsV1 {
    Bearer,
    ApiKeyHeader { header: String },
    None,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayHeaderSemanticsV1 {
    pub content_type: String,
    pub required_headers: Vec<(String, String)>,
    pub forbidden_forward_headers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayErrorSemanticsV1 {
    pub http_status_typed: bool,
    pub sse_error_typed: bool,
    pub retry_after_header: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConnectorProfileV1 {
    pub schema_version: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub entitlement_id: String,
    pub connector_id: String,
    pub connector_revision: String,
    pub upstream_protocol: UpstreamProtocol,
    pub request_path: String,
    pub authentication: GatewayCriticalFactV1<GatewayAuthenticationSemanticsV1>,
    pub headers: GatewayCriticalFactV1<GatewayHeaderSemanticsV1>,
    pub errors: GatewayCriticalFactV1<GatewayErrorSemanticsV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCandidateCapabilityProfileV1 {
    pub schema_version: String,
    pub capability_id: String,
    pub capability_revision: String,
    pub upstream_protocol: UpstreamProtocol,
    pub model_configuration_id: String,
    /// Operational model value rendered by the protocol adapter. The sealed
    /// candidate also carries it as `native_transport_model`; it is not the
    /// catalog-bound logical `upstream_model_id` for a managed CPA target.
    pub native_model: String,
    pub request: GatewayRequestFeatureProfileV1,
    pub response: GatewayResponseFeatureProfileV1,
    pub reasoning_profiles: Vec<GatewayReasoningProfileCapabilityV1>,
    pub selected_reasoning_profile_id: String,
    pub context: GatewayContextLimitsV1,
    pub native_streaming: GatewayCriticalFactV1<bool>,
    pub native_provider_state: GatewayNativeProviderStateEmissionV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCandidateProtocolProfileV1 {
    pub schema_version: String,
    pub path_id: String,
    pub ingress_protocol: UpstreamProtocol,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
    pub capability: GatewayCandidateCapabilityProfileV1,
    pub connector: GatewayConnectorProfileV1,
}

impl GatewayCandidateProtocolProfileV1 {
    pub fn reasoning_profile_for(
        &self,
        reasoning: &ExactNativeReasoningV1,
    ) -> Result<&GatewayReasoningProfileCapabilityV1, CompiledPlanError> {
        let mut matching = self.capability.reasoning_profiles.iter().filter(|profile| {
            reasoning_profile_matches(profile, reasoning, self.capability.upstream_protocol)
        });
        let selected = matching.next().ok_or(CompiledPlanError::InvalidCandidate)?;
        if matching.next().is_some() {
            return Err(CompiledPlanError::InvalidCandidate);
        }
        Ok(selected)
    }

    pub fn validate_for_candidate(
        &self,
        candidate: &AttemptOwnedCandidateV1,
    ) -> Result<(), CompiledPlanError> {
        let exact_reasoning = self.reasoning_profile_for(&candidate.exact_reasoning)?;
        let selected =
            self.capability.reasoning_profiles.iter().filter(|profile| {
                profile.profile_id == self.capability.selected_reasoning_profile_id
            });
        if self.schema_version != "hiroute.candidate-protocol-profile/v1"
            || self.capability.schema_version != "hiroute.candidate-capability/v1"
            || self.connector.schema_version != "hiroute.connector-profile/v1"
            || self.path_id.trim().is_empty()
            || self.adapter_revision.trim().is_empty()
            || self.serializer_revision.trim().is_empty()
            || self.decoder_revision.trim().is_empty()
            || (self.capability.capability_id != candidate.capability_id
                && !matches!(
                    &candidate.operational_target,
                    GatewayOperationalTargetV1::RegisteredHttps { .. }
                ))
            || self.capability.capability_id.trim().is_empty()
            || self
                .capability
                .capability_revision
                .parse::<u64>()
                .ok()
                .is_none_or(|value| value == 0)
            || (self.capability.capability_id == candidate.capability_id
                && self.capability.capability_revision != candidate.capability_revision.to_string())
            || self.capability.model_configuration_id != candidate.model_configuration_id
            || self.capability.native_model != candidate.native_transport_model
            || self.capability.upstream_protocol != self.connector.upstream_protocol
            || (candidate.connector_runtime != ConnectorRuntimeKind::CpaBridge
                && self.capability.upstream_protocol != candidate.upstream_protocol
                && !matches!(
                    &candidate.operational_target,
                    GatewayOperationalTargetV1::RegisteredHttps { .. }
                ))
            || self.connector.connector_id != candidate.connector_id
            || self.connector.connector_revision != candidate.connector_revision.to_string()
            || (candidate.connector_runtime != ConnectorRuntimeKind::CpaBridge
                && candidate
                    .operational_target
                    .for_protocol_path(&self.connector.request_path)
                    .is_none())
            || self.capability.native_provider_state
                == GatewayNativeProviderStateEmissionV1::Unknown
            || self.capability.native_streaming.exact().is_none()
            || self.connector.authentication.exact().is_none()
            || self.connector.headers.exact().is_none()
            || self.connector.errors.exact().is_none()
            || selected.count() != 1
            || exact_reasoning.profile_id != self.capability.selected_reasoning_profile_id
        {
            return Err(CompiledPlanError::InvalidCandidate);
        }
        Ok(())
    }
}

fn reasoning_profile_matches(
    profile: &GatewayReasoningProfileCapabilityV1,
    exact: &ExactNativeReasoningV1,
    protocol: UpstreamProtocol,
) -> bool {
    match (exact, profile.control_kind, &profile.render) {
        (
            ExactNativeReasoningV1::Fixed { profile: fixed, .. },
            GatewayReasoningControlKindV1::Fixed,
            GatewayNativeReasoningRenderV1::NoControlParameter,
        ) => profile.profile_id == *fixed,
        (
            ExactNativeReasoningV1::Toggle { enabled, .. },
            GatewayReasoningControlKindV1::Toggle,
            GatewayNativeReasoningRenderV1::ExactFields {
                protocol: rendered,
                fields,
            },
        ) => {
            profile.profile_id == if *enabled { "enabled" } else { "disabled" }
                && *rendered == protocol
                && fields
                    .iter()
                    .any(|field| field.value == GatewayNativeReasoningValueV1::Bool(*enabled))
        }
        (
            ExactNativeReasoningV1::Profile {
                profile: selected, ..
            },
            GatewayReasoningControlKindV1::Discrete,
            GatewayNativeReasoningRenderV1::ExactFields {
                protocol: rendered,
                fields,
            },
        ) => {
            profile.profile_id == *selected
                && *rendered == protocol
                && fields.iter().any(|field| {
                    matches!(
                        &field.value,
                        GatewayNativeReasoningValueV1::String(value) if value == selected
                    )
                })
        }
        (
            ExactNativeReasoningV1::Budget { tokens, .. },
            GatewayReasoningControlKindV1::Budget,
            GatewayNativeReasoningRenderV1::ExactBudget {
                protocol: rendered,
                selected_tokens,
                ..
            },
        ) => {
            profile.profile_id == format!("budget-{tokens}")
                && *rendered == protocol
                && *selected_tokens == u64::from(*tokens)
        }
        _ => false,
    }
}

pub(super) fn valid_executable_endpoint(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    !host.is_empty()
        && host.len() <= 253
        && !host.ends_with('.')
        && !host.contains(':')
        && !host.contains('@')
        && !host.contains("..")
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('?')
        && !path.contains('#')
        && !path.contains("..")
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
}

fn valid_numeric_loopback_http(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("http://") else {
        return false;
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return false;
    };
    if authority.is_empty()
        || authority.contains('@')
        || path.is_empty()
        || path.starts_with('/')
        || path.contains(['?', '#'])
        || path.contains("..")
    {
        return false;
    }
    let address = if authority.starts_with('[') {
        let Some((host, port)) = authority
            .strip_prefix('[')
            .and_then(|value| value.split_once("]:"))
        else {
            return false;
        };
        let Ok(ip) = host.parse::<std::net::Ipv6Addr>() else {
            return false;
        };
        let Ok(port) = port.parse::<u16>() else {
            return false;
        };
        std::net::SocketAddr::new(ip.into(), port)
    } else {
        let Some((host, port)) = authority.rsplit_once(':') else {
            return false;
        };
        let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
            return false;
        };
        let Ok(port) = port.parse::<u16>() else {
            return false;
        };
        std::net::SocketAddr::new(ip.into(), port)
    };
    address.ip().is_loopback()
        && address.port() != 0
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReasoningRenderModeV1;

    #[test]
    fn registered_protocol_path_keeps_authority_and_user_target_stays_exact() {
        let registered = GatewayOperationalTargetV1::RegisteredHttps {
            uri: "https://open.bigmodel.cn/api/v1/responses".into(),
        };
        assert_eq!(
            registered
                .for_protocol_path("/api/anthropic/v1/messages")
                .unwrap()
                .uri(),
            "https://open.bigmodel.cn/api/anthropic/v1/messages"
        );
        for unsafe_path in [
            "//other.invalid/v1/messages",
            "/api/../messages",
            "/api/messages?token=x",
        ] {
            assert!(registered.for_protocol_path(unsafe_path).is_none());
        }
        let configured = GatewayOperationalTargetV1::UserConfiguredNative {
            uri: "https://custom.invalid/v1/responses".into(),
        };
        assert!(configured.for_protocol_path("/v1/messages").is_none());
        assert_eq!(
            configured.for_protocol_path("/v1/responses"),
            Some(configured.clone())
        );
    }

    #[test]
    fn discrete_reasoning_matches_a_protocol_owned_nested_wire_path() {
        let profile = GatewayReasoningProfileCapabilityV1 {
            profile_id: "max".into(),
            control_kind: GatewayReasoningControlKindV1::Discrete,
            render: GatewayNativeReasoningRenderV1::ExactFields {
                protocol: UpstreamProtocol::Responses,
                fields: vec![GatewayNativeReasoningFieldAssignmentV1 {
                    path: vec!["reasoning".into(), "effort".into()],
                    value: GatewayNativeReasoningValueV1::String("max".into()),
                }],
            },
            accounting: GatewayReasoningAccountingV1::WithinOutputCap,
            additional_reservation_tokens: 0,
        };
        let exact = ExactNativeReasoningV1::Profile {
            parameter: "reasoning_effort".into(),
            profile: "max".into(),
            render_mode: ReasoningRenderModeV1::ExplicitNative,
        };

        assert!(reasoning_profile_matches(
            &profile,
            &exact,
            UpstreamProtocol::Responses
        ));
    }
}
