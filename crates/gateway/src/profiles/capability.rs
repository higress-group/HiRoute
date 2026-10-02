use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::server::core_runtime::model_ir::{ExactProviderPathV1, RequestCapabilityRequirementsV1};
use crate::server::request_plan::IngressProtocol;

use super::{
    ConnectorProfile, ContextLimits, CriticalFact, NativeReasoningRender, ReasoningAccounting,
    ReasoningControlKind, ReasoningProfileCapability, TokenEstimatorProfile,
};

/// Exact bounded-delay contract for protocols that reveal refusal only in
/// their terminal metadata. The block bound also keeps one terminal replay
/// below the decoder's fixed event-drain window.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Exact,
    Normalized,
    GatewayMaterialized,
    Unsupported,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestFeatureProfile {
    pub text: Fidelity,
    pub initial_instructions: Fidelity,
    pub mid_conversation_instructions: Fidelity,
    pub image_url: Fidelity,
    pub image_base64: Fidelity,
    pub image_base64_media_types: CriticalFact<Vec<String>>,
    pub function_tools: Fidelity,
    pub strict_tools: Fidelity,
    pub tool_choice_none: Fidelity,
    pub tool_choice_auto: Fidelity,
    pub tool_choice_required_any: Fidelity,
    pub tool_choice_required_named: Fidelity,
    pub parallel_tools: Fidelity,
    pub tool_roundtrip: Fidelity,
    pub tool_result_text: Fidelity,
    pub tool_result_json: Fidelity,
    pub logical_tool_id_mapping: Fidelity,
    pub provider_state: Fidelity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseFeatureProfile {
    pub text: Fidelity,
    pub reasoning: Fidelity,
    pub refusal: Fidelity,
    pub tool_calls: Fidelity,
    pub logical_tool_id_mapping: Fidelity,
    pub usage: Fidelity,
    pub finish_reason: Fidelity,
    pub typed_error: Fidelity,
    pub provider_state: Fidelity,
    pub stream_refusal: StreamingRefusalSemantics,
    pub stream_text_delta: Fidelity,
    pub stream_tool_argument_delta: Fidelity,
    pub stream_reasoning_delta: Fidelity,
    pub stream_usage: Fidelity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCapabilityProfile {
    pub schema_version: String,
    pub capability_id: String,
    pub capability_revision: String,
    pub upstream_protocol: IngressProtocol,
    pub model_configuration_id: String,
    pub native_model: String,
    pub model_support: hiroute_domain::StoredModelSupportV1,
    pub request: RequestFeatureProfile,
    pub response: ResponseFeatureProfile,
    pub reasoning_profiles: Vec<ReasoningProfileCapability>,
    pub selected_reasoning_profile_id: String,
    pub context: ContextLimits,
    pub native_streaming: CriticalFact<bool>,
    /// Descriptive emission metadata; unknown is not a pre-connect rejection.
    pub native_provider_state: NativeProviderStateEmission,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProtocolProfile {
    pub schema_version: String,
    pub path_id: String,
    pub ingress_protocol: IngressProtocol,
    pub adapter_revision: String,
    pub serializer_revision: String,
    pub decoder_revision: String,
    pub capability: CandidateCapabilityProfile,
    pub connector: ConnectorProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_target: Option<hiroute_domain::GatewayNativeProfileTargetV2>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeProviderStateEmission {
    Never,
    Native,
    Unknown,
}

pub use hiroute_domain::GatewayStreamingRefusalSemanticsV1 as StreamingRefusalSemantics;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientProtocolProfile {
    pub schema_version: String,
    pub protocol: IngressProtocol,
    pub source_protocol: IngressProtocol,
    pub adapter_revision: String,
    pub response: ResponseFeatureProfile,
}

impl CandidateProtocolProfile {
    pub fn exact_portable_path(
        ingress_protocol: IngressProtocol,
        upstream_protocol: IngressProtocol,
        native_model: impl Into<String>,
        reasoning: ReasoningProfileCapability,
    ) -> Self {
        let exact = Fidelity::Exact;
        let native_model = native_model.into();
        let protocol_name = protocol_label(upstream_protocol);
        let connector = ConnectorProfile::exact_json(
            format!("fixture-{protocol_name}-connector"),
            "1",
            upstream_protocol,
        );
        Self {
            schema_version: "hiroute.candidate-protocol-profile/v1".into(),
            path_id: format!("{}-to-{protocol_name}", protocol_label(ingress_protocol)),
            ingress_protocol,
            adapter_revision: "builtin-protocol-adapter/v1".into(),
            serializer_revision: "hiroute-target-json/v1".into(),
            decoder_revision: "hiroute-native-response/v1".into(),
            native_target: None,
            capability: CandidateCapabilityProfile {
                schema_version: "hiroute.candidate-capability/v1".into(),
                capability_id: format!(
                    "exact-{}-to-{protocol_name}",
                    protocol_label(ingress_protocol)
                ),
                capability_revision: "1".into(),
                upstream_protocol,
                model_configuration_id: format!("fixture-model-config-{protocol_name}"),
                native_model,
                model_support: hiroute_domain::StoredModelSupportV1 {
                    tools: hiroute_domain::GatewayCriticalFactV1::Exact(true),
                    vision: hiroute_domain::GatewayCriticalFactV1::Exact(true),
                },
                request: RequestFeatureProfile {
                    text: exact,
                    initial_instructions: exact,
                    mid_conversation_instructions: if upstream_protocol == IngressProtocol::Messages
                    {
                        Fidelity::Unsupported
                    } else {
                        exact
                    },
                    image_url: exact,
                    image_base64: exact,
                    image_base64_media_types: CriticalFact::Exact(vec![
                        "image/gif".into(),
                        "image/jpeg".into(),
                        "image/png".into(),
                        "image/webp".into(),
                    ]),
                    function_tools: exact,
                    strict_tools: if upstream_protocol == IngressProtocol::Messages {
                        Fidelity::Unsupported
                    } else {
                        exact
                    },
                    tool_choice_none: if upstream_protocol == IngressProtocol::Messages {
                        Fidelity::Unsupported
                    } else {
                        exact
                    },
                    tool_choice_auto: exact,
                    tool_choice_required_any: exact,
                    tool_choice_required_named: exact,
                    parallel_tools: exact,
                    tool_roundtrip: exact,
                    tool_result_text: exact,
                    tool_result_json: exact,
                    logical_tool_id_mapping: exact,
                    provider_state: Fidelity::Unsupported,
                },
                response: ResponseFeatureProfile {
                    text: exact,
                    reasoning: exact,
                    refusal: exact,
                    tool_calls: exact,
                    logical_tool_id_mapping: exact,
                    usage: exact,
                    finish_reason: exact,
                    typed_error: exact,
                    provider_state: Fidelity::Unsupported,
                    stream_refusal: if upstream_protocol == IngressProtocol::Messages {
                        StreamingRefusalSemantics::TerminalClassified
                    } else {
                        StreamingRefusalSemantics::ExactDelta
                    },
                    stream_text_delta: exact,
                    stream_tool_argument_delta: exact,
                    stream_reasoning_delta: exact,
                    stream_usage: exact,
                },
                selected_reasoning_profile_id: reasoning.profile_id.clone(),
                reasoning_profiles: vec![reasoning],
                context: ContextLimits {
                    max_input_tokens: CriticalFact::Exact(1024 * 1024),
                    max_output_tokens: CriticalFact::Exact(256),
                    max_total_tokens: CriticalFact::Exact(Some(1024 * 1024 + 256)),
                    estimator: CriticalFact::Exact(TokenEstimatorProfile {
                        revision: "byte-upper-bound/v1".into(),
                        bytes_per_token: 1,
                        fixed_overhead_tokens: 0,
                    }),
                },
                native_streaming: CriticalFact::Exact(true),
                native_provider_state: NativeProviderStateEmission::Never,
            },
            connector,
        }
    }

    pub fn exact_provider_path(&self) -> Result<ExactProviderPathV1, CapabilityError> {
        let owner = ExactProviderPathV1 {
            provider_id: self.connector.provider_id.clone(),
            endpoint_id: self.connector.endpoint_id.clone(),
            entitlement_id: self.connector.entitlement_id.clone(),
            connector_id: self.connector.connector_id.clone(),
            connector_revision: self.connector.connector_revision.clone(),
            capability_id: self.capability.capability_id.clone(),
            capability_revision: self.capability.capability_revision.clone(),
            model_configuration_id: self.capability.model_configuration_id.clone(),
            native_model: self.capability.native_model.clone(),
            upstream_protocol: self.capability.upstream_protocol,
            adapter_revision: self.adapter_revision.clone(),
            serializer_revision: self.serializer_revision.clone(),
            decoder_revision: self.decoder_revision.clone(),
        };
        if owner.is_complete() {
            Ok(owner)
        } else {
            Err(CapabilityError::TargetIdentityUnknown)
        }
    }

    pub fn selected_reasoning(&self) -> Result<&ReasoningProfileCapability, CapabilityError> {
        self.capability
            .reasoning_profiles
            .iter()
            .find(|profile| profile.profile_id == self.capability.selected_reasoning_profile_id)
            .ok_or(CapabilityError::ReasoningProfileUnknown)
    }

    pub fn validate(
        &self,
        requirements: &RequestCapabilityRequirementsV1,
    ) -> Result<&ReasoningProfileCapability, CapabilityError> {
        if self.schema_version != "hiroute.candidate-protocol-profile/v1"
            || self.capability.schema_version != "hiroute.candidate-capability/v1"
            || self.path_id.trim().is_empty()
            || self.adapter_revision.trim().is_empty()
            || self.serializer_revision.trim().is_empty()
            || self.decoder_revision.trim().is_empty()
            || self.capability.capability_id.trim().is_empty()
            || self.capability.capability_revision.trim().is_empty()
            || self.capability.model_configuration_id.trim().is_empty()
            || self.capability.native_model.trim().is_empty()
        {
            return Err(CapabilityError::ProfileUnknown);
        }
        let mut reasoning_ids = BTreeSet::new();
        if self.capability.reasoning_profiles.is_empty()
            || self.capability.reasoning_profiles.iter().any(|profile| {
                !reasoning_ids.insert(profile.profile_id.as_str())
                    || !profile.validate_for(self.capability.upstream_protocol)
            })
        {
            return Err(CapabilityError::ReasoningProfileMismatch);
        }
        if requirements.ingress_protocol != self.ingress_protocol
            || self.capability.upstream_protocol != self.connector.upstream_protocol
        {
            return Err(CapabilityError::ProtocolPathUnavailable);
        }
        self.exact_provider_path()?;
        if !self.connector.critical_facts_are_exact() {
            return Err(CapabilityError::ConnectorFactUnknown);
        }
        // Provider feature metadata is descriptive, not an execution grant.
        // Concrete request serializers and response decoders enforce the
        // conversion they actually implement; upstream validates its payload.
        let reasoning = self.selected_reasoning()?;
        Ok(reasoning)
    }
}

impl ClientProtocolProfile {
    pub fn for_candidate(candidate: &CandidateProtocolProfile) -> Result<Self, CapabilityError> {
        let mut profile = Self::exact_portable(candidate.ingress_protocol);
        profile.source_protocol = candidate.capability.upstream_protocol;
        Ok(profile)
    }

    pub fn exact_portable(protocol: IngressProtocol) -> Self {
        let exact = Fidelity::Exact;
        Self {
            schema_version: "hiroute.client-protocol-profile/v1".into(),
            protocol,
            source_protocol: protocol,
            adapter_revision: "builtin-client-protocol-adapter/v1".into(),
            response: ResponseFeatureProfile {
                text: exact,
                reasoning: exact,
                refusal: exact,
                tool_calls: exact,
                logical_tool_id_mapping: exact,
                usage: exact,
                finish_reason: exact,
                typed_error: exact,
                provider_state: Fidelity::Exact,
                stream_refusal: StreamingRefusalSemantics::ExactDelta,
                stream_text_delta: exact,
                stream_tool_argument_delta: exact,
                stream_reasoning_delta: exact,
                stream_usage: exact,
            },
        }
    }

    pub fn is_complete(&self) -> bool {
        self.schema_version == "hiroute.client-protocol-profile/v1"
            && !self.adapter_revision.trim().is_empty()
    }
}

fn protocol_label(protocol: IngressProtocol) -> &'static str {
    match protocol {
        IngressProtocol::Responses => "responses",
        IngressProtocol::ChatCompletions => "chat_completions",
        IngressProtocol::Messages => "messages",
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CapabilityError {
    #[error("candidate capability profile is unknown or incomplete")]
    ProfileUnknown,
    #[error("candidate connector critical fact is unknown")]
    ConnectorFactUnknown,
    #[error("candidate protocol path is unavailable")]
    ProtocolPathUnavailable,
    #[error("candidate exact target identity is unknown or incomplete")]
    TargetIdentityUnknown,
    #[error("text is unsupported")]
    TextUnsupported,
    #[error("initial instructions are unsupported")]
    InitialInstructionsUnsupported,
    #[error("mid-conversation instructions are unsupported")]
    MidConversationInstructionsUnsupported,
    #[error("image URL is unsupported")]
    ImageUrlUnsupported,
    #[error("base64 image is unsupported")]
    ImageBase64Unsupported,
    #[error("base64 image media type is unsupported or unknown")]
    ImageMediaTypeUnsupported,
    #[error("portable function Tool interface is unsupported")]
    ToolInterfaceUnsupported,
    #[error("strict Tool schema semantics are unsupported")]
    StrictToolsUnsupported,
    #[error("Tool choice is unsupported")]
    ToolChoiceUnsupported,
    #[error("parallel Tool calls are unsupported")]
    ParallelToolsUnsupported,
    #[error("Tool continuation is unsupported")]
    ToolRoundtripUnsupported,
    #[error("text Tool result is unsupported")]
    ToolResultTextUnsupported,
    #[error("JSON Tool result is unsupported")]
    ToolResultJsonUnsupported,
    #[error("logical/native Tool ID mapping is unsupported")]
    LogicalToolIdMappingUnsupported,
    #[error("provider state is unsupported")]
    ProviderStateUnsupported,
    #[error("native streaming is unsupported or unknown")]
    StreamingUnsupported,
    #[error("streaming refusal classification is unsupported, unbounded, or unknown")]
    StreamRefusalUnsupported,
    #[error("text delta streaming is unsupported")]
    StreamTextUnsupported,
    #[error("Tool argument streaming is unsupported")]
    StreamToolUnsupported,
    #[error("reasoning streaming is unsupported")]
    StreamReasoningUnsupported,
    #[error("usage streaming is unsupported")]
    StreamUsageUnsupported,
    #[error("response text is unsupported")]
    ResponseTextUnsupported,
    #[error("response reasoning is unsupported")]
    ResponseReasoningUnsupported,
    #[error("response refusal is unsupported")]
    ResponseRefusalUnsupported,
    #[error("response Tool calls are unsupported")]
    ResponseToolsUnsupported,
    #[error("response logical/native Tool ID mapping is unsupported")]
    ResponseLogicalToolIdMappingUnsupported,
    #[error("response usage is unsupported")]
    ResponseUsageUnsupported,
    #[error("response finish reason is unsupported")]
    ResponseFinishUnsupported,
    #[error("typed response errors are unsupported")]
    ResponseErrorUnsupported,
    #[error("response provider state is unsupported")]
    ResponseProviderStateUnsupported,
    #[error("native response provider state has no exact P0 client projection")]
    NativeProviderStateUnrepresentable,
    #[error("selected reasoning profile is unknown")]
    ReasoningProfileUnknown,
    #[error("selected reasoning profile cannot be rendered exactly")]
    ReasoningProfileMismatch,
}

pub fn fixed_reasoning(profile_id: impl Into<String>) -> ReasoningProfileCapability {
    ReasoningProfileCapability {
        profile_id: profile_id.into(),
        control_kind: ReasoningControlKind::Fixed,
        render: NativeReasoningRender::NoControlParameter,
        accounting: ReasoningAccounting::WithinOutputCap,
        additional_reservation_tokens: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_classification_contract_contains_no_proxy_size_quota() {
        let expected = serde_json::json!({"kind":"terminal_classified"});
        assert_eq!(
            serde_json::to_value(StreamingRefusalSemantics::TerminalClassified).unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(
                hiroute_domain::GatewayStreamingRefusalSemanticsV1::TerminalClassified
            )
            .unwrap(),
            expected
        );
        let obsolete = serde_json::json!({"kind":"terminal_classified","max_buffered_bytes":16777216,"max_buffered_blocks":8});
        let recovered =
            serde_json::from_value::<StreamingRefusalSemantics>(obsolete.clone()).unwrap();
        assert!(matches!(
            recovered,
            StreamingRefusalSemantics::LegacyTerminalClassified { .. }
        ));
        // Recovery must preserve the original capability digest.
        assert_eq!(serde_json::to_value(recovered).unwrap(), obsolete);
        assert!(
            serde_json::from_value::<StreamingRefusalSemantics>(
                serde_json::json!({"kind":"terminal_classified","unregistered_quota":8})
            )
            .is_err()
        );
    }

    #[test]
    fn client_profiles_do_not_require_reasoning_owner_registration() {
        let mut candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::Messages,
            "physical-model",
            fixed_reasoning("fixed"),
        );
        candidate.capability.native_provider_state = NativeProviderStateEmission::Native;
        candidate.capability.request.provider_state = Fidelity::Exact;
        candidate.capability.response.provider_state = Fidelity::Exact;

        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert_eq!(profile.response.provider_state, Fidelity::Exact);

        candidate.ingress_protocol = IngressProtocol::Responses;
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert!(profile.is_complete());

        candidate.ingress_protocol = IngressProtocol::Messages;
        candidate.capability.upstream_protocol = IngressProtocol::Responses;
        candidate.connector.upstream_protocol = IngressProtocol::Responses;
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert_eq!(profile.response.provider_state, Fidelity::Exact);
    }
}
