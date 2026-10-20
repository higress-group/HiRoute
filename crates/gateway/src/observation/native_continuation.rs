use super::*;
use hiroute_domain::{
    HermesCompressionLineageV1, NATIVE_CONTINUATION_SCHEMA_V1, NativeContinuationEvidenceV1,
    NativeProofObservationV1,
};
use http::HeaderMap;
use std::collections::BTreeSet;

fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii() && (32..127).contains(&b))
}
fn stamp(s: &NativeProofObservationV1) -> bool {
    identifier(&s.observer_id) && s.sequence > 0 && s.observed_at_ms >= 0 && s.age_ms <= 3_600_000
}

pub(crate) fn read(headers: &HeaderMap, current: &str) -> Option<HermesCompressionLineageV1> {
    if headers
        .get_all("x-hermes-compression-lineage")
        .iter()
        .count()
        != 1
    {
        return None;
    }
    let value = headers.get("x-hermes-compression-lineage")?.as_bytes();
    if value.len() > 8192 {
        return None;
    }
    let proof: HermesCompressionLineageV1 = serde_json::from_slice(value).ok()?;
    if proof.version != 1
        || proof.ids.is_empty()
        || proof.ids.len() > 32
        || proof.ids.last()?.as_str() != current
        || !stamp(&proof.proof_observation)
    {
        return None;
    }
    let mut ids = BTreeSet::new();
    for id in &proof.ids {
        if !identifier(id) || !ids.insert(id.as_str()) {
            return None;
        }
    }
    if let Some(state) = &proof.current_state
        && (!stamp(&state.observation)
            || state.observation.age_ms > 1000
            || state
                .started_at_ms
                .is_some_and(|t| t < 0 || t > state.observation.observed_at_ms))
    {
        return None;
    }
    if let Some(negative) = &proof.invalid_edges {
        if negative.edges.len() > 31 || !stamp(&negative.observation) {
            return None;
        }
        for [parent, child] in &negative.edges {
            if parent == child || !identifier(parent) || !identifier(child) {
                return None;
            }
            ids.insert(parent);
            ids.insert(child);
        }
    }
    if ids.len() > 32 {
        return None;
    }
    Some(proof)
}

impl GatewayObservation {
    pub(super) fn observe_native_continuation(
        &self,
        authorized: &AuthorizedRequestPlan,
        ingress: IngressProtocol,
        identity: &ContextIdentityFacts,
        headers: &HeaderMap,
        current_hmac: &str,
        request_id: &str,
    ) {
        if !self.enabled || !authorized.is_hermes_consumer() {
            return;
        }
        let proof = match identity.observation_identity() {
            Some(("hermes_session", native)) => {
                native.first().and_then(|current| read(headers, current))
            }
            _ => None,
        };
        let receipt = authorized.receipt();
        let route = match &authorized.planner_policy().identity {
            crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Plan {
                plan_id, ..
            } => plan_id.as_str(),
            crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Fixed {
                binding_digest,
            } => binding_digest.as_str(),
        };
        let protocol = match ingress {
            IngressProtocol::Responses => "responses",
            IngressProtocol::ChatCompletions => "chat_completions",
            IngressProtocol::Messages => "messages",
        };
        let hmac = |id: &str| {
            reliable_observation_session_id(
                &self.key,
                receipt.workspace_id.as_ref(),
                receipt.authority_id.as_ref(),
                receipt.grant_id.as_ref(),
                route,
                receipt.served_model_id.as_ref(),
                protocol,
                "hermes_session",
                &[id],
            )
        };
        if proof
            .as_ref()
            .and_then(|p| p.ids.last())
            .is_some_and(|current| hmac(current) != current_hmac)
        {
            return;
        }
        let ids = proof
            .as_ref()
            .map(|p| p.ids.iter().map(|id| hmac(id)).collect())
            .unwrap_or_else(|| vec![current_hmac.to_owned()]);
        let complete = proof.as_ref().is_some_and(|p| p.complete);
        let (proof_observation, current_state, negative) = proof
            .map(|p| (Some(p.proof_observation), p.current_state, p.invalid_edges))
            .unwrap_or_default();
        let negative = negative.map(|mut n| {
            n.edges = n
                .edges
                .into_iter()
                .map(|[p, c]| [hmac(&p), hmac(&c)])
                .collect();
            n
        });
        let workspace = receipt.workspace_id.to_string();
        let request = request_id.to_owned();
        let current = current_hmac.to_owned();
        let key = self.key;
        let ingress_at_ms = i64::try_from(unix_nanos() / 1_000_000).unwrap_or(i64::MAX);
        self.channels
            .native_continuation
            .publish(
                move |producer, sequence, _loss| NativeContinuationEvidenceV1 {
                    schema_version: NATIVE_CONTINUATION_SCHEMA_V1.into(),
                    workspace_id: workspace,
                    request_id: request.clone(),
                    current_session_id: current,
                    ids,
                    complete,
                    proof_observation,
                    current_state,
                    invalid_edges: negative,
                    ingress_at_ms,
                    producer_id: producer.producer_id,
                    producer_epoch: producer.producer_epoch,
                    stream_id: producer.stream_id,
                    sequence,
                    event_id: stable_id(
                        "native-relation-event",
                        &key,
                        b"native-continuation/v1",
                        &[request.as_bytes()],
                    ),
                },
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_optional_headers_never_become_evidence() {
        let mut h = HeaderMap::new();
        h.insert(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_static("{}"),
        );
        assert!(read(&h, "s").is_none());
        let p = serde_json::json!({"version":1,"ids":["a","s"],"complete":true,"proof_observation":{"observer_id":"o","sequence":1,"observed_at_ms":1,"age_ms":5000}});
        h.insert(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_str(&p.to_string()).unwrap(),
        );
        assert!(read(&h, "s").is_some());
        assert!(read(&h, "other").is_none());
        let mut with_origin = p.clone();
        with_origin["current_state"] = serde_json::json!({"value":"open",
            "observation":{"observer_id":"o","sequence":2,"observed_at_ms":10000,"age_ms":0},
            "started_at_ms":5000});
        h.insert(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_str(&with_origin.to_string()).unwrap(),
        );
        assert_eq!(
            read(&h, "s").unwrap().current_state.unwrap().started_at_ms,
            Some(5000)
        );
        with_origin["current_state"]["started_at_ms"] = serde_json::json!(20000);
        h.insert(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_str(&with_origin.to_string()).unwrap(),
        );
        assert!(
            read(&h, "s").is_none(),
            "future creation is not native origin proof"
        );
        h.insert(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_str(&p.to_string()).unwrap(),
        );
        h.append(
            "x-hermes-compression-lineage",
            http::HeaderValue::from_static("{}"),
        );
        assert!(read(&h, "s").is_none());
    }
}
