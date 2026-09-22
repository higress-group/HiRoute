use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::runtime::body::{BudgetTree, StreamBudget};
use hiroute_gateway_core::transport::{
    GatewayRequestHead, GatewayResponseHead, GatewaySession, HttpProtocol, TransportError,
};
use http::{HeaderMap, Method};

use super::*;
use crate::provider_state::ProviderStateScopeV1;
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

struct FailSecondBodyWrite {
    writes: usize,
    accepted: Vec<Bytes>,
}

#[async_trait]
impl GatewaySession for FailSecondBodyWrite {
    fn request_head(&self) -> Result<GatewayRequestHead, TransportError> {
        Ok(request_head())
    }

    async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError> {
        Ok(None)
    }

    async fn write_response_head(
        &mut self,
        _head: GatewayResponseHead,
    ) -> Result<(), TransportError> {
        Ok(())
    }

    async fn write_response_body(
        &mut self,
        body: Bytes,
        _end_stream: bool,
    ) -> Result<(), TransportError> {
        self.writes += 1;
        if self.writes == 2 {
            return Err(TransportError::Io("controlled downstream reset".into()));
        }
        self.accepted.push(body);
        Ok(())
    }
}

#[tokio::test]
async fn production_response_sink_keeps_only_the_tool_unit_accepted_before_reset() {
    let scope = ProviderStateScopeV1 {
        authority_id: "authority:runtime-test".into(),
        authority_epoch: 1,
        grant_id: "grant:runtime-test".into(),
        grant_generation: 1,
        served_model_id: "runtime-test".into(),
        route: hiroute_domain::ModelRequestRouteV2::Plan {
            revision: 1,
            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"runtime-test-plan"),
        },
    };
    let first_id = "native-first".to_owned();
    let second_id = "native-second".to_owned();
    let active = adapters::ActiveResponseDelivery::new(
        IngressProtocol::Responses,
        crate::provider_state::ActiveProviderStates::new(
            Arc::new(crate::provider_state::ProviderStateStore::default()),
            scope,
            IngressProtocol::Responses,
        ),
    );
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "native-runtime-test",
        fixed_reasoning("fixed"),
    );
    let tool_frames = adapters::with_active_response_delivery(active.clone(), async {
        let mut frames = Vec::new();
        for id in [&first_id, &second_id] {
            let mut projector = adapters::NativeResponseProjector::new_for_attempt(
                &profile, true, "runtime-test".into(), None, budget(),
            ).unwrap();
            projector.feed(b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"response\",\"model\":\"native-runtime-test\"}}\n\n", false).unwrap();
            let item = serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"type":"function_call","call_id":id,"id":"item","name":"weather","arguments":"","status":"in_progress"}});
            // Native whitespace and escaped IDs must not hide a delivered tool.
            let encoded = serde_json::to_string(&item).unwrap().replace(":", ": ").replace("native-f", "native-\\u0066");
            let units = projector.feed(format!("data: {encoded}\n\n").as_bytes(), false).unwrap();
            assert_eq!(units.len(), 1);
            frames.push(units[0].bytes.clone());
        }
        frames
    }).await;
    let observation = observation::accepted_request_for_runtime_test();
    let budget = budget();
    let mut downstream = FailSecondBodyWrite {
        writes: 0,
        accepted: Vec::new(),
    };
    let mut session = ReplayBodySession {
        inner: &mut downstream,
        request_head: request_head(),
        request_body: None,
        budget,
        response_capture: AcceptedResponseCapture::new(observation.clone()),
        response_started: true,
        runtime_state_authority: Default::default(),
        continuation_scanner: active.scanner(),
    };

    session
        .write_response_body(Bytes::from(tool_frames[0].clone()), false)
        .await
        .unwrap();
    assert!(observation.has_accepted_attempt());
    assert_eq!(
        observation::accepted_frame_count_for_runtime_test(&observation),
        1
    );

    let error = session
        .write_response_body(Bytes::from(tool_frames[1].clone()), true)
        .await
        .unwrap_err();
    assert!(matches!(error, TransportError::Io(_)));
    assert_eq!(
        observation::accepted_frame_count_for_runtime_test(&observation),
        1
    );
    drop(session);
    assert_eq!(active.accepted_count(), 1);
    assert_eq!(downstream.accepted.len(), 1);
}

fn request_head() -> GatewayRequestHead {
    GatewayRequestHead {
        method: Method::POST,
        path_and_query: Arc::from("/v1/responses"),
        authority: Some(Arc::from("gateway.test")),
        headers: HeaderMap::new(),
        protocol: HttpProtocol::Http1,
    }
}

fn budget() -> StreamBudget {
    BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(512 * 1024)
        .unwrap()
}
