//! Opt-in Plan controls: exact native matches, otherwise the sealed candidate default.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    CandidateProtocolProfile, CapabilityError, NativeReasoningRender, NativeReasoningValue,
    ReasoningAccounting, ReasoningProfileCapability,
};
use crate::server::request_plan::IngressProtocol;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedNativeReasoning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_tokens: Option<u64>,
}

impl RequestedNativeReasoning {
    pub(crate) fn from_document(
        document: &Value,
        ingress: IngressProtocol,
    ) -> Result<Self, CapabilityError> {
        let invalid = || CapabilityError::ReasoningProfileMismatch;
        fn object(
            value: Option<&Value>,
        ) -> Result<Option<&serde_json::Map<String, Value>>, CapabilityError> {
            value
                .map(|v| {
                    v.as_object()
                        .ok_or(CapabilityError::ReasoningProfileMismatch)
                })
                .transpose()
        }
        let (effort, thinking) = match ingress {
            IngressProtocol::Responses => {
                let reasoning = object(document.get("reasoning"))?;
                (reasoning.and_then(|r| r.get("effort")), None)
            }
            IngressProtocol::ChatCompletions => (document.get("reasoning_effort"), None),
            IngressProtocol::Messages => {
                let output = object(document.get("output_config"))?;
                (
                    output.and_then(|r| r.get("effort")),
                    object(document.get("thinking"))?,
                )
            }
        };
        let string = |value: Option<&Value>| {
            value
                .map(|v| {
                    v.as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 64)
                        .map(str::to_owned)
                        .ok_or_else(invalid)
                })
                .transpose()
        };
        let effort = string(effort)?;
        let thinking_type = string(thinking.and_then(|t| t.get("type")))?;
        let budget_tokens = thinking
            .and_then(|t| t.get("budget_tokens"))
            .map(|v| v.as_u64().filter(|v| *v > 0).ok_or_else(invalid))
            .transpose()?;
        if thinking.is_some() && thinking_type.is_none()
            || budget_tokens.is_some() && thinking_type.as_deref() != Some("enabled")
            || effort.is_some() && matches!(thinking_type.as_deref(), Some("disabled"))
        {
            return Err(invalid());
        }
        Ok(Self {
            effort,
            thinking_type,
            budget_tokens,
        })
    }

    /// Durable, bounded native control evidence; never a request-local Replay marker.
    pub(crate) fn native_value(&self, ingress: IngressProtocol) -> Option<Value> {
        if self.is_absent() {
            return None;
        }
        match ingress {
            IngressProtocol::Responses => Some(serde_json::json!({"effort":self.effort})),
            IngressProtocol::ChatCompletions => self
                .effort
                .as_ref()
                .map(|effort| Value::String(effort.clone())),
            IngressProtocol::Messages => {
                let mut value = serde_json::Map::new();
                if let Some(kind) = &self.thinking_type {
                    let mut thinking = serde_json::json!({"type":kind});
                    if let Some(tokens) = self.budget_tokens {
                        thinking["budget_tokens"] = tokens.into();
                    }
                    value.insert("thinking".into(), thinking);
                }
                if let Some(effort) = &self.effort {
                    value.insert("output_config".into(), serde_json::json!({"effort":effort}));
                }
                Some(Value::Object(value))
            }
        }
    }

    pub(crate) fn is_absent(&self) -> bool {
        self == &Self::default()
    }
}

impl CandidateProtocolProfile {
    pub(crate) fn follow_reasoning(
        &self,
        requested: &RequestedNativeReasoning,
        requested_output: Option<u64>,
    ) -> Result<Self, CapabilityError> {
        self.follow_reasoning_with_resolution(requested, requested_output)
            .map(|(profile, _)| profile)
    }

    pub(crate) fn follow_reasoning_with_resolution(
        &self,
        requested: &RequestedNativeReasoning,
        requested_output: Option<u64>,
    ) -> Result<(Self, hiroute_domain::ReasoningResolutionV1), CapabilityError> {
        use hiroute_domain::ReasoningResolutionV1::*;
        if requested.is_absent() {
            return Ok((self.clone(), DefaultMissing));
        }
        let mut matches = self
            .capability
            .reasoning_profiles
            .iter()
            .filter_map(|profile| self.match_reasoning(profile, requested, requested_output));
        let Some(selected) = matches.next() else {
            return Ok((self.clone(), DefaultUnsupported));
        };
        // Ambiguous controls never guess a profile or weaken the saved fallback.
        if matches.any(|other| other != selected) {
            return Ok((self.clone(), DefaultUnsupported));
        }
        let mut result = self.clone();
        result.capability.selected_reasoning_profile_id = selected.profile_id.clone();
        result.capability.reasoning_profiles = vec![selected];
        Ok((result, Request))
    }

    fn match_reasoning(
        &self,
        profile: &ReasoningProfileCapability,
        request: &RequestedNativeReasoning,
        requested_output: Option<u64>,
    ) -> Option<ReasoningProfileCapability> {
        let mut selected = profile.clone();
        let protocol = self.capability.upstream_protocol;
        let effort_path: &[&str] = match protocol {
            IngressProtocol::Responses => &["reasoning", "effort"],
            IngressProtocol::ChatCompletions => &["reasoning_effort"],
            IngressProtocol::Messages => &["output_config", "effort"],
        };
        let matches_string = |path: &[&str], value: &str| {
            if profile.control_kind == super::ReasoningControlKind::Toggle {
                let enabled = match profile.profile_id.as_str() {
                    "enabled" => Some(true),
                    "disabled" => Some(false),
                    _ => None,
                };
                if let Some(enabled) = enabled {
                    if protocol == IngressProtocol::Responses && path == ["reasoning", "effort"] {
                        return value == if enabled { "low" } else { "none" };
                    }
                    if protocol == IngressProtocol::Messages && path == ["thinking", "type"] {
                        return value == if enabled { "enabled" } else { "disabled" };
                    }
                }
            }
            profile.render.fields().iter().any(|f| {
                f.path.iter().map(String::as_str).eq(path.iter().copied())
                    && f.value == NativeReasoningValue::String(value.to_owned())
            })
        };
        if request
            .effort
            .as_deref()
            .is_some_and(|effort| !matches_string(effort_path, effort))
            || request.thinking_type.as_deref().is_some_and(|kind| {
                protocol != IngressProtocol::Messages
                    || !matches_string(&["thinking", "type"], kind)
            })
        {
            return None;
        }
        if let Some(tokens) = request.budget_tokens {
            let NativeReasoningRender::ExactBudget {
                fields,
                budget_path,
                selected_tokens,
                min_tokens,
                max_tokens,
                step_tokens,
                ..
            } = &mut selected.render
            else {
                return None;
            };
            let model_limit = self.capability.context.max_output_tokens.exact().copied();
            let output_limit = match (requested_output, model_limit) {
                (Some(client), Some(model)) => Some(client.min(model)),
                (client, model) => client.or(model),
            };
            if output_limit.is_some_and(|limit| tokens >= limit)
                || tokens < 1024
                || *step_tokens == 0
                || tokens < *min_tokens
                || tokens > *max_tokens
                || (tokens - *min_tokens) % *step_tokens != 0
                || budget_path != &["thinking", "budget_tokens"]
            {
                return None;
            }
            let field = fields.iter_mut().find(|f| f.path == *budget_path)?;
            field.value = NativeReasoningValue::U64(tokens);
            *selected_tokens = tokens;
            selected.profile_id = format!("budget-{tokens}");
            if selected.accounting == ReasoningAccounting::Additive {
                selected.additional_reservation_tokens = tokens;
            }
        }
        selected.validate_for(protocol).then_some(selected)
    }
}

#[cfg(test)]
#[path = "follow_reasoning_tests.rs"]
mod tests;
