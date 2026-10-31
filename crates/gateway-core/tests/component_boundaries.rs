use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::core::execution_plan::{
    AttemptBodyPlans, AttemptTimeouts, PlanRevision, ResolvedTargetBindingId,
};
use hiroute_gateway_core::core::publication::{InstallError, PrepareOutcome, PublicationInstaller};
use hiroute_gateway_core::runtime::attempt::{
    AttemptError, AttemptExchange, AttemptGeneration, AttemptTransport, Disposition,
    PreparedAttemptBody, PreparedAttemptHttpRequest, PreparedRequestHead, RequestId,
    TransportPrecommitEvent, WriterGate,
};
use hiroute_gateway_core::runtime::body::{
    BodyPlan, BudgetTree, ChargedBodyQueue, ChargedBytes, MemoryRole, RequestLeaseBook,
};
use hiroute_gateway_core::test_support::contracts::{
    CompiledSourceFake, ConsumingProviderFake, SelectorPublisherFake, SourceDeliveryKind,
};
use hiroute_gateway_core::test_support::{BootstrapPublicationBuilder, plain_target};
use hiroute_gateway_core::transport::HttpProtocol;
use http::{HeaderMap, Method, StatusCode};
use tokio_util::sync::CancellationToken;

fn envelope(
    plan: u64,
    revision: u64,
    full_snapshot: bool,
) -> hiroute_gateway_core::core::publication::CompiledGatewayPublicationEnvelope {
    BootstrapPublicationBuilder::new(plan, revision)
        .full_snapshot(full_snapshot)
        .route(
            "example.test",
            "/",
            1,
            plain_target("127.0.0.1:18080".parse().unwrap(), revision),
        )
        .unwrap()
        .build()
        .unwrap()
}

fn apply(
    installer: &PublicationInstaller,
    envelope: hiroute_gateway_core::core::publication::CompiledGatewayPublicationEnvelope,
) -> Result<bool, InstallError> {
    let cancel = CancellationToken::new();
    match installer.prepare(envelope, &cancel, Instant::now() + Duration::from_secs(1))? {
        PrepareOutcome::Prepared(prepared) => {
            installer.publish(prepared, &cancel, Instant::now() + Duration::from_secs(1))?;
            Ok(true)
        }
        PrepareOutcome::Duplicate(_) => Ok(false),
    }
}

#[test]
fn every_source_style_only_delivers_compiled_envelopes_and_reconciliation_closes_gaps() {
    let mut source = CompiledSourceFake::default();
    source.enqueue(SourceDeliveryKind::InMemory, envelope(1, 1, true));
    source.enqueue(SourceDeliveryKind::Poll, envelope(2, 2, false));
    source.enqueue(
        SourceDeliveryKind::MessageHint { sequence: 20 },
        envelope(2, 2, false),
    );
    source.enqueue(
        SourceDeliveryKind::WatchHint { sequence: 10 },
        envelope(1, 1, false),
    );
    source.enqueue(
        SourceDeliveryKind::MessageHint { sequence: 40 },
        envelope(4, 4, false),
    );
    source.enqueue(
        SourceDeliveryKind::PeriodicReconciliation,
        envelope(4, 4, true),
    );

    let installer = PublicationInstaller::new();
    assert!(apply(&installer, source.next_compiled().unwrap().envelope).unwrap());
    assert!(apply(&installer, source.next_compiled().unwrap().envelope).unwrap());
    assert!(!apply(&installer, source.next_compiled().unwrap().envelope).unwrap());
    assert_eq!(
        apply(&installer, source.next_compiled().unwrap().envelope).unwrap_err(),
        InstallError::RollbackNotAuthorized
    );
    assert_eq!(
        apply(&installer, source.next_compiled().unwrap().envelope).unwrap_err(),
        InstallError::ResyncRequired {
            active: hiroute_gateway_core::core::execution_plan::ConfigRevision(2),
            candidate: hiroute_gateway_core::core::execution_plan::ConfigRevision(4),
        }
    );
    assert!(apply(&installer, source.next_compiled().unwrap().envelope).unwrap());
    assert_eq!(installer.active().unwrap().plan_revision, PlanRevision(4));
    assert!(source.is_empty());
}

#[derive(Debug, Default)]
struct TransportFacts {
    connects: usize,
    resets: usize,
}

struct ContractTransport {
    facts: Arc<Mutex<TransportFacts>>,
    events: VecDeque<TransportPrecommitEvent>,
    head_written: bool,
}

#[async_trait]
impl AttemptTransport for ContractTransport {
    async fn connect(
        &mut self,
        _target: &hiroute_gateway_core::core::execution_plan::TransportTarget,
        _address: SocketAddr,
    ) -> Result<(), AttemptError> {
        self.facts.lock().unwrap().connects += 1;
        Ok(())
    }

    async fn write_request_head(
        &mut self,
        _head: &PreparedRequestHead,
    ) -> Result<(), AttemptError> {
        self.head_written = true;
        Ok(())
    }

    async fn write_request_body(
        &mut self,
        _body: Bytes,
        _end_stream: bool,
    ) -> Result<(), AttemptError> {
        Ok(())
    }

    async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        Ok(())
    }

    fn poll_precommit(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        if self.head_written {
            Poll::Ready(Ok(self.events.pop_front()))
        } else {
            Poll::Pending
        }
    }

    async fn cancel_reset(&mut self) -> Result<(), AttemptError> {
        self.facts.lock().unwrap().resets += 1;
        Ok(())
    }

    fn protocol(&self) -> HttpProtocol {
        HttpProtocol::Http1
    }
}

fn new_exchange(
    selected: hiroute_gateway_core::test_support::contracts::SelectedAttempt,
    transport: ContractTransport,
    leases: &RequestLeaseBook,
    body: Bytes,
) -> AttemptExchange<ContractTransport> {
    let body_plans = AttemptBodyPlans {
        attempt_request: BodyPlan::StreamingReplay {
            max_chunk_bytes: 16 * 1024,
            max_replay_bytes: 64 * 1024,
        },
        attempt_response_precommit: BodyPlan::PassThrough {
            max_chunk_bytes: 64 * 1024,
        },
    };
    let budgets = BudgetTree::new(1024 * 1024, 1024 * 1024).unwrap();
    let budget = budgets.stream(512 * 1024).unwrap();
    let mut chunks = ChargedBodyQueue::new(
        &budget,
        MemoryRole::AttemptWire,
        &body_plans.attempt_request,
        64 * 1024,
        8,
    )
    .unwrap();
    chunks
        .push_back(ChargedBytes::copy_from_opaque(&budget, MemoryRole::AttemptWire, &body).unwrap())
        .unwrap();
    AttemptExchange::new(
        selected.request_id,
        selected.attempt_id,
        selected.generation,
        selected.binding.plan_revision(),
        plain_target("127.0.0.1:18080".parse().unwrap(), 1),
        transport,
        PreparedAttemptHttpRequest {
            network_route: None,
            head: PreparedRequestHead {
                method: Method::POST,
                path_and_query: Arc::from("/v1/test"),
                headers: HeaderMap::new(),
            },
            body: PreparedAttemptBody::new(chunks, leases.acquire().unwrap()).unwrap(),
        },
        body_plans,
        AttemptTimeouts {
            request_write: Duration::from_secs(1),
            first_byte: Duration::from_secs(1),
            stream_idle: Duration::from_secs(1),
        },
        Instant::now() + Duration::from_secs(5),
        16,
        budget,
        16 * 1024,
    )
    .unwrap()
}

#[tokio::test]
async fn rate_limit_requires_a_second_explicit_selection_and_attempt_exchange() {
    let request_id = RequestId(77);
    let generation = AttemptGeneration(9);
    let mut selector = SelectorPublisherFake::new([
        ResolvedTargetBindingId::new(PlanRevision(8), 1),
        ResolvedTargetBindingId::new(PlanRevision(8), 2),
    ]);
    let facts = Arc::new(Mutex::new(TransportFacts::default()));
    let leases = RequestLeaseBook::new();
    let mut provider = ConsumingProviderFake::default();
    let mut ir = provider.parse_consuming(Bytes::from_static(b"opaque request"));
    let prepared = provider.materialize_consuming(&mut ir).unwrap();

    let first = selector.select_next(request_id, generation).unwrap();
    assert_eq!(selector.selected().len(), 1);
    let first_transport = ContractTransport {
        facts: Arc::clone(&facts),
        events: VecDeque::from([TransportPrecommitEvent::ResponseHead {
            status: StatusCode::TOO_MANY_REQUESTS,
            headers: HeaderMap::new(),
        }]),
        head_written: false,
    };
    let mut first_exchange = new_exchange(first, first_transport, &leases, prepared.wire_bytes);
    first_exchange.drive_writer_once().await.unwrap();
    let (_, first_disposition) = provider.classify_consuming(
        Bytes::from_static(b"opaque 429 event"),
        StatusCode::TOO_MANY_REQUESTS,
    );
    assert_eq!(first_disposition, Disposition::Continue);
    first_exchange
        .submit_disposition_candidate(first_disposition)
        .unwrap();
    let first_permit = match first_exchange
        .wait_writer_gate(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap()
    {
        WriterGate::ReadyToPublishNonAccept { permit, .. } => permit,
        other => panic!("unexpected writer gate: {other:?}"),
    };
    let first_published = first_exchange
        .publish_disposition(first_disposition, first_permit)
        .unwrap();
    selector.observe_published(&first_published).unwrap();
    first_exchange.finish_or_abort().await.unwrap();
    assert_eq!(first_exchange.snapshot().semantic_upstream_calls, 1);
    assert_eq!(selector.selected().len(), 1, "#9 cannot select fallback");

    let second = selector
        .select_next(request_id, AttemptGeneration(generation.0 + 1))
        .unwrap();
    let second_transport = ContractTransport {
        facts: Arc::clone(&facts),
        events: VecDeque::new(),
        head_written: false,
    };
    let mut second_exchange = new_exchange(
        second,
        second_transport,
        &leases,
        Bytes::from_static(b"second explicit attempt"),
    );
    for _ in 0..3 {
        second_exchange.drive_writer_once().await.unwrap();
    }
    second_exchange
        .submit_disposition_candidate(Disposition::Accept)
        .unwrap();
    let second_permit = match second_exchange
        .wait_writer_gate(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap()
    {
        WriterGate::ReadyToPublishAccept { permit, .. } => permit,
        other => panic!("unexpected writer gate: {other:?}"),
    };
    let second_published = second_exchange
        .publish_disposition(Disposition::Accept, second_permit)
        .unwrap();
    selector.observe_published(&second_published).unwrap();
    second_exchange.begin_accepted_response_scope().unwrap();
    second_exchange
        .finish_accepted_response(true)
        .await
        .unwrap();

    assert_eq!(selector.selected().len(), 2);
    assert_eq!(selector.published().len(), 2);
    assert_eq!(second_exchange.snapshot().semantic_upstream_calls, 1);
    assert_eq!(facts.lock().unwrap().connects, 2);
    assert_eq!(facts.lock().unwrap().resets, 1);
    assert_eq!(leases.outstanding(), 0);
}

#[test]
fn provider_encoder_is_consuming_and_cannot_run_before_matching_publication() {
    let mut provider = ConsumingProviderFake::default();
    let (decoded, disposition) =
        provider.classify_consuming(Bytes::from_static(b"data: opaque\n\n"), StatusCode::OK);
    let readiness = provider.establish_readiness(decoded, disposition);
    let mismatched = hiroute_gateway_core::runtime::attempt::PublishedDisposition {
        request_id: RequestId(1),
        attempt_id: hiroute_gateway_core::runtime::attempt::AttemptId(1),
        generation: AttemptGeneration(1),
        disposition: Disposition::Continue,
    };
    assert!(
        provider
            .encode_after_publication(readiness, &mismatched)
            .is_err()
    );
    assert_eq!(provider.counts().encoded, 0);
}

#[test]
fn gateway_core_manifest_has_no_storage_or_remote_client_dependency() {
    const MANIFEST: &str = include_str!("../Cargo.toml");
    for forbidden in ["rusqlite", "sqlx", "rocksdb", "redis =", "reqwest"] {
        assert!(
            !MANIFEST.contains(forbidden),
            "gateway-core must not directly depend on {forbidden}"
        );
    }
}

#[test]
fn gateway_core_enables_pingora_transport_by_default() {
    const MANIFEST: &str = include_str!("../Cargo.toml");
    assert!(
        MANIFEST.contains("default = [\"pingora-transport\"]"),
        "ordinary gateway-core builds must include the production Pingora transport"
    );
}
