use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, RequestCapabilityRequirementsV1, ToolChoice,
};
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

impl Fidelity {
    fn is_exact(self) -> bool {
        self == Self::Exact
    }
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
    pub state_affinity: StateAffinity,
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
    pub state_affinity: StateAffinity,
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
    pub request: RequestFeatureProfile,
    pub response: ResponseFeatureProfile,
    pub reasoning_profiles: Vec<ReasoningProfileCapability>,
    pub selected_reasoning_profile_id: String,
    pub context: ContextLimits,
    pub native_streaming: CriticalFact<bool>,
    /// Exact state-emission contract for this physical attempt. Unknown is a
    /// pre-connect rejection state; owner-affine emission is accepted only
    /// when request and response profiles also require the same exact owner.
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateAffinity {
    Unsupported,
    ExactOwner,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeProviderStateEmission {
    Never,
    ExactOwnerAffine,
    Unknown,
}

pub use hiroute_domain::GatewayStreamingRefusalSemanticsV1 as StreamingRefusalSemantics;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientProtocolProfile {
    pub schema_version: String,
    pub protocol: IngressProtocol,
    pub adapter_revision: String,
    /// Required for any opaque state projection. Protocol equality alone is
    /// never sufficient ownership evidence.
    pub state_owner: Option<ExactProviderPathV1>,
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
                    state_affinity: StateAffinity::Unsupported,
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
                    state_affinity: StateAffinity::Unsupported,
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
        if self.capability.native_provider_state == NativeProviderStateEmission::Unknown {
            return Err(CapabilityError::NativeProviderStateUnrepresentable);
        }
        if !self.connector.critical_facts_are_exact() {
            return Err(CapabilityError::ConnectorFactUnknown);
        }
        let request = &self.capability.request;
        exact(
            requirements.text,
            request.text,
            CapabilityError::TextUnsupported,
        )?;
        exact(
            requirements.initial_instructions,
            request.initial_instructions,
            CapabilityError::InitialInstructionsUnsupported,
        )?;
        exact(
            requirements.mid_conversation_instructions,
            request.mid_conversation_instructions,
            CapabilityError::MidConversationInstructionsUnsupported,
        )?;
        exact(
            requirements.image_url,
            request.image_url,
            CapabilityError::ImageUrlUnsupported,
        )?;
        exact(
            requirements.image_base64,
            request.image_base64,
            CapabilityError::ImageBase64Unsupported,
        )?;
        if requirements.image_base64 {
            let media_types = request
                .image_base64_media_types
                .exact()
                .ok_or(CapabilityError::ImageMediaTypeUnsupported)?;
            if requirements.image_media_types.iter().any(|required| {
                !media_types
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(required))
            }) {
                return Err(CapabilityError::ImageMediaTypeUnsupported);
            }
        }
        exact(
            requirements.function_tools,
            request.function_tools,
            CapabilityError::ToolInterfaceUnsupported,
        )?;
        exact(
            requirements.strict_tools,
            request.strict_tools,
            CapabilityError::StrictToolsUnsupported,
        )?;
        if requirements.function_tools {
            let fidelity = match requirements.tool_choice {
                ToolChoice::None => request.tool_choice_none,
                ToolChoice::Auto => request.tool_choice_auto,
                ToolChoice::RequiredAny => request.tool_choice_required_any,
                ToolChoice::RequiredNamed { .. } => request.tool_choice_required_named,
            };
            exact(true, fidelity, CapabilityError::ToolChoiceUnsupported)?;
        }
        exact(
            requirements.parallel_tools,
            request.parallel_tools,
            CapabilityError::ParallelToolsUnsupported,
        )?;
        exact(
            requirements.tool_roundtrip,
            request.tool_roundtrip,
            CapabilityError::ToolRoundtripUnsupported,
        )?;
        exact(
            requirements.tool_result_text,
            request.tool_result_text,
            CapabilityError::ToolResultTextUnsupported,
        )?;
        exact(
            requirements.tool_result_json,
            request.tool_result_json,
            CapabilityError::ToolResultJsonUnsupported,
        )?;
        exact(
            requirements.logical_tool_id_mapping,
            request.logical_tool_id_mapping,
            CapabilityError::LogicalToolIdMappingUnsupported,
        )?;
        exact(
            requirements.provider_state,
            request.provider_state,
            CapabilityError::ProviderStateUnsupported,
        )?;
        if requirements.provider_state && request.state_affinity != StateAffinity::ExactOwner {
            return Err(CapabilityError::StateAffinityUnsupported);
        }
        if requirements.streaming && self.capability.native_streaming.exact() != Some(&true) {
            return Err(CapabilityError::StreamingUnsupported);
        }
        let response = &self.capability.response;
        exact(
            true,
            response.text,
            CapabilityError::ResponseTextUnsupported,
        )?;
        exact(
            true,
            response.reasoning,
            CapabilityError::ResponseReasoningUnsupported,
        )?;
        exact(
            true,
            response.refusal,
            CapabilityError::ResponseRefusalUnsupported,
        )?;
        exact(
            requirements.function_tools,
            response.tool_calls,
            CapabilityError::ResponseToolsUnsupported,
        )?;
        exact(
            requirements.function_tools,
            response.logical_tool_id_mapping,
            CapabilityError::ResponseLogicalToolIdMappingUnsupported,
        )?;
        exact(
            true,
            response.usage,
            CapabilityError::ResponseUsageUnsupported,
        )?;
        exact(
            true,
            response.finish_reason,
            CapabilityError::ResponseFinishUnsupported,
        )?;
        exact(
            true,
            response.typed_error,
            CapabilityError::ResponseErrorUnsupported,
        )?;
        if requirements.streaming
            && !matches!(
                (self.capability.upstream_protocol, response.stream_refusal),
                (
                    IngressProtocol::Messages,
                    StreamingRefusalSemantics::TerminalClassified
                        | StreamingRefusalSemantics::LegacyTerminalClassified { .. }
                ) | (
                    IngressProtocol::Responses | IngressProtocol::ChatCompletions,
                    StreamingRefusalSemantics::ExactDelta
                )
            )
        {
            return Err(CapabilityError::StreamRefusalUnsupported);
        }
        if self.capability.native_provider_state == NativeProviderStateEmission::ExactOwnerAffine {
            exact(
                true,
                request.provider_state,
                CapabilityError::ProviderStateUnsupported,
            )?;
            if request.state_affinity != StateAffinity::ExactOwner {
                return Err(CapabilityError::StateAffinityUnsupported);
            }
            exact(
                true,
                response.provider_state,
                CapabilityError::ResponseProviderStateUnsupported,
            )?;
            if response.state_affinity != StateAffinity::ExactOwner {
                return Err(CapabilityError::StateAffinityUnsupported);
            }
        }
        exact(
            requirements.stream_text,
            response.stream_text_delta,
            CapabilityError::StreamTextUnsupported,
        )?;
        exact(
            requirements.stream_tool_arguments,
            response.stream_tool_argument_delta,
            CapabilityError::StreamToolUnsupported,
        )?;
        exact(
            requirements.stream_reasoning,
            response.stream_reasoning_delta,
            CapabilityError::StreamReasoningUnsupported,
        )?;
        exact(
            requirements.stream_usage,
            response.stream_usage,
            CapabilityError::StreamUsageUnsupported,
        )?;
        let reasoning = self.selected_reasoning()?;
        Ok(reasoning)
    }
}

impl ClientProtocolProfile {
    pub fn for_candidate(candidate: &CandidateProtocolProfile) -> Result<Self, CapabilityError> {
        if client_can_represent_provider_state(
            candidate.ingress_protocol,
            candidate.capability.upstream_protocol,
        ) && candidate.capability.native_provider_state
            == NativeProviderStateEmission::ExactOwnerAffine
        {
            return candidate
                .exact_provider_path()
                .map(|owner| Self::exact_owner_affine(candidate.ingress_protocol, owner));
        }
        Ok(Self::exact_portable(candidate.ingress_protocol))
    }

    pub fn exact_portable(protocol: IngressProtocol) -> Self {
        let exact = Fidelity::Exact;
        Self {
            schema_version: "hiroute.client-protocol-profile/v1".into(),
            protocol,
            adapter_revision: "builtin-client-protocol-adapter/v1".into(),
            state_owner: None,
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
                state_affinity: StateAffinity::Unsupported,
                stream_refusal: StreamingRefusalSemantics::ExactDelta,
                stream_text_delta: exact,
                stream_tool_argument_delta: exact,
                stream_reasoning_delta: exact,
                stream_usage: exact,
            },
        }
    }

    pub fn exact_owner_affine(protocol: IngressProtocol, owner: ExactProviderPathV1) -> Self {
        let mut profile = Self::exact_portable(protocol);
        profile.state_owner = Some(owner);
        profile.response.provider_state = Fidelity::Exact;
        profile.response.state_affinity = StateAffinity::ExactOwner;
        profile
    }

    pub fn is_complete(&self) -> bool {
        self.schema_version == "hiroute.client-protocol-profile/v1"
            && !self.adapter_revision.trim().is_empty()
            && match self.state_owner.as_ref() {
                Some(owner) => {
                    owner.is_complete()
                        && client_can_represent_provider_state(
                            self.protocol,
                            owner.upstream_protocol,
                        )
                        && self.response.provider_state == Fidelity::Exact
                        && self.response.state_affinity == StateAffinity::ExactOwner
                }
                None => {
                    self.response.provider_state == Fidelity::Unsupported
                        && self.response.state_affinity == StateAffinity::Unsupported
                }
            }
    }
}

fn client_can_represent_provider_state(client: IngressProtocol, upstream: IngressProtocol) -> bool {
    client == upstream
        || (client == IngressProtocol::Messages && upstream == IngressProtocol::Responses)
}

fn exact(
    required: bool,
    fidelity: Fidelity,
    error: CapabilityError,
) -> Result<(), CapabilityError> {
    if required && !fidelity.is_exact() {
        Err(error)
    } else {
        Ok(())
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
    #[error("provider state exact-owner affinity is unsupported or unknown")]
    StateAffinityUnsupported,
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
    fn client_profile_preserves_only_explicitly_representable_exact_owner_state() {
        let mut candidate = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::Messages,
            "physical-model",
            fixed_reasoning("fixed"),
        );
        candidate.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
        candidate.capability.request.provider_state = Fidelity::Exact;
        candidate.capability.request.state_affinity = StateAffinity::ExactOwner;
        candidate.capability.response.provider_state = Fidelity::Exact;
        candidate.capability.response.state_affinity = StateAffinity::ExactOwner;

        let expected_owner = candidate.exact_provider_path().unwrap();
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert_eq!(profile.state_owner, Some(expected_owner));
        assert_eq!(profile.response.provider_state, Fidelity::Exact);
        assert_eq!(profile.response.state_affinity, StateAffinity::ExactOwner);

        candidate.ingress_protocol = IngressProtocol::Responses;
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert_eq!(profile.state_owner, None);
        assert_eq!(profile.response.provider_state, Fidelity::Unsupported);
        assert_eq!(profile.response.state_affinity, StateAffinity::Unsupported);

        candidate.ingress_protocol = IngressProtocol::Messages;
        candidate.capability.upstream_protocol = IngressProtocol::Responses;
        candidate.connector.upstream_protocol = IngressProtocol::Responses;
        let expected_owner = candidate.exact_provider_path().unwrap();
        let profile = ClientProtocolProfile::for_candidate(&candidate).unwrap();
        assert_eq!(profile.state_owner, Some(expected_owner));
        assert_eq!(profile.response.provider_state, Fidelity::Exact);
    }
}
