//! Stateless tool IDs and request-local delivery bookkeeping.
use super::ProtocolAdapterError;
use crate::server::core_runtime::model_ir::{ExactProviderPathV1, ModelIrError};
use crate::server::request_plan::IngressProtocol;
use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// CPA-style deterministic adaptation, only across protocols. A digest avoids
/// the collisions introduced by replacing arbitrary characters with '_'.
pub(crate) fn project_tool_id(
    id: &str,
    from: IngressProtocol,
    to: IngressProtocol,
) -> Result<String, ModelIrError> {
    if id.trim().is_empty() {
        return Err(ModelIrError::InvalidField("tool id"));
    }
    if from == to {
        return Ok(id.to_owned());
    }
    let invalid = to == IngressProtocol::Messages
        && !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !invalid {
        return Ok(id.to_owned());
    }
    let prefix: String = id
        .chars()
        .take(31)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(id.as_bytes());
    let suffix: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{prefix}_{suffix}"))
}

#[derive(Clone, Debug)]
pub(crate) struct ToolIdProjection {
    downstream: IngressProtocol,
}
impl ToolIdProjection {
    pub(crate) fn new(downstream: IngressProtocol) -> Self {
        Self { downstream }
    }
    pub(crate) fn project(
        &self,
        native_id: &str,
        owner: &ExactProviderPathV1,
    ) -> Result<String, ProtocolAdapterError> {
        Ok(project_tool_id(
            native_id,
            owner.upstream_protocol,
            self.downstream,
        )?)
    }
}

struct PendingTool {
    id: String,
    pattern: Vec<u8>,
    charge: Arc<Reservation>,
}

#[derive(Clone)]
pub(crate) struct ActiveResponseDelivery {
    projection: ToolIdProjection,
    provider_states: crate::provider_state::ActiveProviderStates,
    pending_tools: Arc<Mutex<Vec<PendingTool>>>,
    accepted_count: Arc<AtomicUsize>,
}
impl ActiveResponseDelivery {
    pub(crate) fn new(
        downstream: IngressProtocol,
        provider_states: crate::provider_state::ActiveProviderStates,
    ) -> Self {
        Self {
            projection: ToolIdProjection::new(downstream),
            provider_states,
            pending_tools: Default::default(),
            accepted_count: Default::default(),
        }
    }
    pub(crate) fn scanner(&self) -> AcceptedResponseDeliveryScanner {
        AcceptedResponseDeliveryScanner {
            active: self.clone(),
            carry: Vec::new(),
            carry_charge: None,
            provider_states: self.provider_states.scanner(),
        }
    }
    pub(crate) fn accepted_count(&self) -> usize {
        self.accepted_count.load(Ordering::Acquire)
    }
    pub(crate) fn tool_id_projection(&self) -> ToolIdProjection {
        self.projection.clone()
    }
}
tokio::task_local! { static ACTIVE_RESPONSE_DELIVERY: ActiveResponseDelivery; }
pub(crate) async fn with_active_response_delivery<F: Future>(
    active: ActiveResponseDelivery,
    future: F,
) -> F::Output {
    ACTIVE_RESPONSE_DELIVERY.scope(active, future).await
}
pub(super) fn project_delivered_tool_id(
    native_id: &str,
    owner: &ExactProviderPathV1,
) -> Result<String, ProtocolAdapterError> {
    let active = ACTIVE_RESPONSE_DELIVERY.try_with(Clone::clone).ok();
    let Some(active) = active else {
        return Ok(native_id.to_owned());
    };
    let id = active.projection.project(native_id, owner)?;
    let mut pending = active
        .pending_tools
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !pending.iter().any(|entry| entry.id == id) {
        let prefix: &[u8] = match active.projection.downstream {
            IngressProtocol::Responses => b"\"call_id\":",
            _ => b"\"id\":",
        };
        let length = crate::provider_state::json_string_length(&id)?
            .checked_add(prefix.len())
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        let bytes = length
            .checked_mul(8)
            .and_then(|n| n.checked_add(std::mem::size_of::<PendingTool>()))
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        let charge = active
            .provider_states
            .budget()
            .reserve(MemoryRole::SemanticState, bytes)
            .map_err(|_| ModelIrError::BufferLimit(bytes))?;
        let mut pattern = Vec::with_capacity(length);
        pattern.extend_from_slice(prefix);
        serde_json::to_writer(&mut pattern, &id)
            .map_err(|e| ProtocolAdapterError::Serialization(e.to_string()))?;
        pending.push(PendingTool {
            id: id.clone(),
            pattern,
            charge: Arc::new(charge),
        });
    }
    Ok(id)
}
pub(super) fn record_provider_state(
    value: &serde_json::Value,
    owner: &ExactProviderPathV1,
) -> Result<(), ProtocolAdapterError> {
    ACTIVE_RESPONSE_DELIVERY
        .try_with(|active| active.provider_states.record(value, owner))
        .unwrap_or(Ok(()))
        .map_err(Into::into)
}
pub(super) fn record_provider_state_at_acceptance(
    state: &str,
    owner: &ExactProviderPathV1,
    closing_event: &[u8],
) -> Result<(), ProtocolAdapterError> {
    ACTIVE_RESPONSE_DELIVERY
        .try_with(|active| {
            active
                .provider_states
                .record_at_acceptance(state, owner, closing_event)
        })
        .unwrap_or(Ok(()))
        .map_err(Into::into)
}
/// Delivery facts only; no later request queries these patterns.
pub(crate) struct AcceptedResponseDeliveryScanner {
    active: ActiveResponseDelivery,
    carry: Vec<u8>,
    carry_charge: Option<Arc<Reservation>>,
    provider_states: crate::provider_state::AcceptedProviderStateScanner,
}
impl AcceptedResponseDeliveryScanner {
    pub(crate) fn accept_bytes(&mut self, bytes: &[u8], now: Instant) {
        self.provider_states.accept_bytes(bytes, now);
        let mut pending = self
            .active
            .pending_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let keep = pending
            .iter()
            .map(|p| p.pattern.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        let next_charge = pending
            .iter()
            .max_by_key(|p| p.pattern.len())
            .map(|p| Arc::clone(&p.charge));
        let mut boundary = self.carry.clone();
        boundary.extend_from_slice(&bytes[..bytes.len().min(keep)]);
        pending.retain(|entry| {
            let pattern = &entry.pattern;
            let accepted = bytes.windows(pattern.len()).any(|w| w == pattern)
                || boundary.windows(pattern.len()).any(|w| w == pattern);
            if accepted {
                self.active.accepted_count.fetch_add(1, Ordering::AcqRel);
            }
            !accepted
        });
        if bytes.len() >= keep {
            self.carry = bytes[bytes.len() - keep..].to_vec();
        } else {
            self.carry.extend_from_slice(bytes);
            let drop = self.carry.len().saturating_sub(keep);
            self.carry.drain(..drop);
        }
        self.carry_charge = next_charge;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::core_runtime::adapters::{
        NativeResponseDecoder, decode_ingress_request, project_candidate_request,
    };
    use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
    use serde_json::json;

    #[tokio::test]
    async fn tool_delivery_uses_budget_without_fixed_pattern_size_or_count_limits() {
        let budget =
            hiroute_gateway_core::runtime::body::BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
                .unwrap()
                .stream(8 * 1024 * 1024)
                .unwrap();
        let scope = crate::provider_state::ProviderStateScopeV1 {
            authority_id: "a".into(),
            authority_epoch: 1,
            grant_id: "g".into(),
            grant_generation: 1,
            served_model_id: "alias".into(),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 1,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"route"),
            },
        };
        let active = ActiveResponseDelivery::new(
            IngressProtocol::Responses,
            crate::provider_state::ActiveProviderStates::with_budget(
                Arc::new(crate::provider_state::ProviderStateStore::default()),
                scope,
                IngressProtocol::Responses,
                budget.clone(),
            ),
        );
        let owner = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "native",
            fixed_reasoning("fixed"),
        )
        .exact_provider_path()
        .unwrap();
        let long = "n".repeat(300 * 1024);
        with_active_response_delivery(active.clone(), async {
            assert_eq!(project_delivered_tool_id(&long, &owner).unwrap(), long);
            for index in 0..129 {
                let id = format!("call-{index}");
                assert_eq!(project_delivered_tool_id(&id, &owner).unwrap(), id);
            }
        })
        .await;
        assert_eq!(active.pending_tools.lock().unwrap().len(), 130);
        let mut scanner = active.scanner();
        let wire = serde_json::to_vec(&json!({"call_id":long})).unwrap();
        for chunk in wire.chunks(8191) {
            scanner.accept_bytes(chunk, Instant::now());
        }
        assert_eq!(active.accepted_count(), 1);
        assert!(budget.snapshot().unwrap().live > 0);
        drop(scanner);
        drop(active);
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }

    #[test]
    fn repeated_native_ids_preserve_each_result_without_history_pairing() {
        let document = json!({"model":"alias","input":[
            {"type":"function_call","call_id":"same","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"same","output":"first"},
            {"type":"function_call","call_id":"same","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"same","output":"second"}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        for target in [IngressProtocol::Responses, IngressProtocol::Messages] {
            let profile = CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                target,
                "candidate",
                fixed_reasoning("fixed"),
            );
            let wire = project_candidate_request(&request, &profile).unwrap().body;
            if target == IngressProtocol::Responses {
                assert_eq!(wire["input"], document["input"]);
            } else {
                assert_eq!(wire["messages"][1]["content"][0]["content"], "first");
                assert_eq!(wire["messages"][3]["content"][0]["content"], "second");
                assert_eq!(wire["messages"][3]["content"][0]["tool_use_id"], "same");
            }
        }
        // The wire item defines its kind; a same-ID call does not override it.
        let result_only = json!({"model":"alias","input":[
            {"type":"function_call","call_id":"same","namespace":"group","name":"lookup","arguments":"{}"},
            {"type":"custom_tool_call_output","call_id":"same","output":"native output"},
            {"type":"function_call_output","call_id":"unknown","output":"standalone"}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &result_only).unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "candidate",
            fixed_reasoning("fixed"),
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["input"],
            result_only["input"]
        );
    }

    #[test]
    fn tool_history_pairs_without_owner_and_rejects_projected_collisions() {
        let id = "call.with-punctuation";
        let projected_id =
            project_tool_id(id, IngressProtocol::Responses, IngressProtocol::Messages).unwrap();
        let mut document = json!({"model":"alias","input":[
            {"type":"function_call","call_id":id,"name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":id,"output":"done"}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        for model in ["first-candidate", "different-candidate"] {
            let profile = CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                IngressProtocol::Messages,
                model,
                fixed_reasoning("fixed"),
            );
            let wire = project_candidate_request(&request, &profile).unwrap().body;
            assert_eq!(wire["messages"][0]["content"][0]["id"], projected_id);
            assert_eq!(
                wire["messages"][1]["content"][0]["tool_use_id"],
                projected_id
            );
        }
        document["input"].as_array_mut().unwrap().push(json!({
            "type":"function_call", "call_id":projected_id, "name":"lookup", "arguments":"{}"
        }));
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Messages,
            "candidate",
            fixed_reasoning("fixed"),
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap_err(),
            ProtocolAdapterError::ModelIr(ModelIrError::ToolContinuationConflict)
        );
    }

    #[test]
    fn response_long_id_is_stateless_and_returned_history_keeps_the_pair() {
        let id = "native".repeat(20);
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "candidate",
            fixed_reasoning("fixed"),
        );
        let body = json!({"id":"response","model":"candidate","choices":[{
            "index":0,"message":{"role":"assistant","content":null,"tool_calls":[{
                "id":id,"type":"function","function":{"name":"lookup","arguments":"{}"}
            }]},"finish_reason":"tool_calls"
        }]});
        let mut decoder = NativeResponseDecoder::new(&profile, 200, false).unwrap();
        decoder
            .feed(&serde_json::to_vec(&body).unwrap(), true)
            .unwrap();
        let response = decoder.finish().unwrap();
        let wire_id = &response.response.tool_id_map[0].logical_id;
        assert_eq!(wire_id, &"native".repeat(20));
        let request = decode_ingress_request(IngressProtocol::Responses, &json!({"model":"alias",
            "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
            "input":[{"type":"function_call","call_id":wire_id,"name":"lookup","arguments":"{}"},
                     {"type":"function_call_output","call_id":wire_id,"output":"done"}]
        })).unwrap();
        let wire = project_candidate_request(&request, &profile).unwrap().body;
        assert_eq!(wire["messages"][0]["tool_calls"][0]["id"], *wire_id);
        assert_eq!(wire["messages"][1]["tool_call_id"], *wire_id);
    }
    #[test]
    fn native_id_and_stateless_cross_protocol_adaptation() {
        use IngressProtocol::*;
        for protocol in [Responses, Messages, ChatCompletions] {
            assert_eq!(
                project_tool_id("native-call-1", protocol, protocol).unwrap(),
                "native-call-1"
            );
            assert_eq!(
                project_tool_id("native-call-1", protocol, Responses).unwrap(),
                "native-call-1"
            );
        }
        let long = "x".repeat(100);
        let shortened = project_tool_id(&long, Messages, Responses).unwrap();
        assert_eq!(shortened, long);
        assert_eq!(
            project_tool_id(&shortened, Messages, Responses).unwrap(),
            shortened
        );
        assert_ne!(
            project_tool_id("call.a", Responses, Messages).unwrap(),
            project_tool_id("call a", Responses, Messages).unwrap()
        );
        assert_eq!(
            project_tool_id("call.a", Responses, Responses).unwrap(),
            "call.a"
        );
    }
}
