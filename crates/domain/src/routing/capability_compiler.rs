//! Current executable protocol features. Both new plans and storage admission use this builder.
use super::*;
use crate::UpstreamProtocol;

impl StoredModelCapabilitiesV1 {
    pub fn compile(
        &self,
        upstream_protocol: UpstreamProtocol,
    ) -> GatewayCandidateCapabilityProfileV1 {
        let exact = GatewayFidelityV1::Exact;
        let unsupported = GatewayFidelityV1::Unsupported;
        let fidelity = |fact: &GatewayCriticalFactV1<bool>| match fact {
            GatewayCriticalFactV1::Exact(true) => exact,
            GatewayCriticalFactV1::Exact(false) => unsupported,
            GatewayCriticalFactV1::Unknown => GatewayFidelityV1::Unknown,
        };
        let tool = fidelity(&self.model_support.tools);
        let vision = fidelity(&self.model_support.vision);
        let provider_state = matches!(
            upstream_protocol,
            UpstreamProtocol::Responses | UpstreamProtocol::Messages
        );
        GatewayCandidateCapabilityProfileV1 {
            schema_version: "hiroute.candidate-capability/v1".into(),
            capability_id: self.capability_id.clone(),
            capability_revision: self.capability_revision.clone(),
            upstream_protocol,
            model_configuration_id: self.model_configuration_id.clone(),
            native_model: self.native_model.clone(),
            model_support: self.model_support.clone(),
            request: GatewayRequestFeatureProfileV1 {
                text: exact,
                initial_instructions: exact,
                mid_conversation_instructions: if upstream_protocol == UpstreamProtocol::Messages {
                    unsupported
                } else {
                    exact
                },
                image_url: vision,
                image_base64: vision,
                image_base64_media_types: match &self.model_support.vision {
                    GatewayCriticalFactV1::Exact(true) => GatewayCriticalFactV1::Exact(vec![
                        "image/gif".into(),
                        "image/jpeg".into(),
                        "image/png".into(),
                        "image/webp".into(),
                    ]),
                    GatewayCriticalFactV1::Exact(false) => GatewayCriticalFactV1::Exact(Vec::new()),
                    GatewayCriticalFactV1::Unknown => GatewayCriticalFactV1::Unknown,
                },
                function_tools: tool,
                strict_tools: if upstream_protocol == UpstreamProtocol::Messages {
                    unsupported
                } else {
                    tool
                },
                tool_choice_none: if upstream_protocol == UpstreamProtocol::Messages {
                    unsupported
                } else {
                    tool
                },
                tool_choice_auto: tool,
                tool_choice_required_any: tool,
                tool_choice_required_named: tool,
                parallel_tools: tool,
                tool_roundtrip: tool,
                tool_result_text: tool,
                tool_result_json: tool,
                logical_tool_id_mapping: tool,
                provider_state: if provider_state { exact } else { unsupported },
            },
            response: GatewayResponseFeatureProfileV1 {
                text: exact,
                reasoning: exact,
                refusal: exact,
                tool_calls: tool,
                logical_tool_id_mapping: tool,
                usage: exact,
                finish_reason: exact,
                typed_error: exact,
                provider_state: if provider_state { exact } else { unsupported },
                stream_refusal: if upstream_protocol == UpstreamProtocol::Messages {
                    GatewayStreamingRefusalSemanticsV1::TerminalClassified
                } else {
                    GatewayStreamingRefusalSemanticsV1::ExactDelta
                },
                stream_text_delta: exact,
                stream_tool_argument_delta: tool,
                stream_reasoning_delta: exact,
                stream_usage: exact,
            },
            reasoning_profiles: self
                .reasoning_profiles
                .iter()
                .map(StoredNativeReasoningV1::compile)
                .collect(),
            selected_reasoning_profile_id: self.selected_reasoning_profile_id.clone(),
            context: GatewayContextLimitsV1 {
                max_input_tokens: self.max_input_tokens.clone(),
                max_output_tokens: self.max_output_tokens.clone(),
                max_total_tokens: self.max_total_tokens.clone(),
                estimator: GatewayCriticalFactV1::Exact(GatewayTokenEstimatorProfileV1 {
                    revision: "byte-upper-bound/v1".into(),
                    bytes_per_token: 1,
                    fixed_overhead_tokens: 0,
                }),
            },
            native_streaming: self.native_streaming.clone(),
            native_provider_state: if provider_state {
                GatewayNativeProviderStateEmissionV1::Native
            } else {
                GatewayNativeProviderStateEmissionV1::Never
            },
        }
    }
}
