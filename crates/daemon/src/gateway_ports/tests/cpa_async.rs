//! Exercise the async credential boundary with genuinely blocking CPA work.
//! The test thread owns all deadlines so blocked Tokio workers cannot hide a failure.
use super::*;
use hiroute_gateway::ports::CredentialLease;
use hiroute_gateway::server::composition::PortError;
use std::collections::BTreeSet;
use std::sync::{Condvar, mpsc};
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

const OBSERVATION_BOUND: Duration = Duration::from_millis(600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Provider {
    Claude,
    Codex,
}

impl Provider {
    fn connector(self) -> &'static str {
        match self {
            Self::Claude => "connector.cpa.claude",
            Self::Codex => "connector.cpa.codex",
        }
    }

    fn sibling(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Claude,
        }
    }
}

#[derive(Default)]
struct IoState {
    released: bool,
    entered: usize,
    finished: usize,
}

#[derive(Default)]
struct IoGate {
    state: Mutex<IoState>,
    changed: Condvar,
}

impl IoGate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered += 1;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
        state.finished += 1;
        self.changed.notify_all();
    }

    fn wait_entered(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while state.entered < count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "blocking fixture never entered CPA work"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.entered, state.finished)
    }

    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
}

// Declare this guard after Runtime so a failing assertion releases real work
// before Runtime's destructor joins blocking threads.
struct ReleaseOnDrop(Arc<IoGate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct BlockingCpaAuthority {
    blocked: Provider,
    gate: Arc<IoGate>,
    calls: Mutex<Vec<(String, String)>>,
}

impl CpaDownstreamCredentialPort for BlockingCpaAuthority {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        self.calls.lock().unwrap().push((
            request.connector_id.to_owned(),
            request.credential_id.to_owned(),
        ));
        if request.connector_id == self.blocked.connector() {
            self.gate.block();
        }
        Ok(None)
    }
}

type Resolver = GatewayCredentialResolver<FakeNativeAuthority, BlockingCpaAuthority>;

fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn resolver(provider: Provider, gate: &Arc<IoGate>) -> (Arc<Resolver>, Arc<BlockingCpaAuthority>) {
    let authority = Arc::new(BlockingCpaAuthority {
        blocked: provider,
        gate: Arc::clone(gate),
        calls: Mutex::new(Vec::new()),
    });
    let resolver = Arc::new(GatewayCredentialResolver::new(
        Arc::new(FakeNativeAuthority::default()),
        Arc::clone(&authority),
    ));
    (resolver, authority)
}

async fn cpa_lease<C>(
    resolver: &GatewayCredentialResolver<FakeNativeAuthority, C>,
    provider: Provider,
    credential_id: &str,
    scope: &ExecutionScope,
) -> Result<Option<CredentialLease>, PortError>
where
    C: CpaDownstreamCredentialPort + Send + Sync + 'static,
{
    let (model, path, protocol, endpoint, destination) = match provider {
        Provider::Claude => (
            "claude-sonnet",
            "/v1/messages",
            IngressProtocol::Messages,
            "https://api.anthropic.com/v1/messages",
            "connection-option/cpa.claude.v1",
        ),
        Provider::Codex => (
            "gpt-5.4",
            "/v1/responses",
            IngressProtocol::Responses,
            "https://chatgpt.com/backend-api/codex/responses",
            "connection-option/cpa.codex.v1",
        ),
    };
    let target = GatewayOperationalTargetV1::ManagedCpaLoopback {
        uri: format!("http://127.0.0.1:43129{path}"),
        runtime_epoch: 13,
        target_epoch: 21,
    };
    let digest = CanonicalDigest::of(&target).unwrap().to_string();
    let profile = CanonicalDigest::of_bytes(b"async-cpa-profile").to_string();
    let transport_model = format!("hiroute-account/{model}");
    resolver
        .lease_exact(
            CredentialLeaseRequest {
                stable_binding_id: "binding/cpa-async",
                credential_ref: credential_id,
                credential_destination_ref: destination,
                excluded_key_ids: &[],
                connector_runtime: ConnectorRuntimeKind::CpaBridge,
                connector_id: provider.connector(),
                upstream_protocol: protocol,
                upstream_model_id: model,
                native_transport_model: &transport_model,
                logical_endpoint: endpoint,
                operational_target: target.uri(),
                operational_target_digest: &digest,
                runtime_epoch: Some(13),
                target_epoch: Some(21),
                protocol_profile_digest: &profile,
                request_path: path,
                authentication: &AuthenticationSemantics::Bearer,
            },
            scope,
        )
        .await
}

#[derive(Debug, Eq, PartialEq)]
enum Outcome {
    NoCapability,
    Rejected,
    UnexpectedCapability,
}

type Completion = (String, Outcome);

fn spawn_lease(
    runtime: &Runtime,
    resolver: &Arc<Resolver>,
    provider: Provider,
    id: &str,
    scope: ExecutionScope,
    completed: &mpsc::Sender<Completion>,
) -> JoinHandle<()> {
    let resolver = Arc::clone(resolver);
    let id = id.to_owned();
    let completed = completed.clone();
    runtime.spawn(async move {
        let outcome = match cpa_lease(&resolver, provider, &id, &scope).await {
            Ok(None) => Outcome::NoCapability,
            Err(_) => Outcome::Rejected,
            Ok(Some(_)) => Outcome::UnexpectedCapability,
        };
        completed.send((id, outcome)).unwrap();
    })
}

fn collect(rx: &mpsc::Receiver<Completion>, wanted: usize) -> Vec<Completion> {
    let deadline = Instant::now() + OBSERVATION_BOUND;
    let mut items = Vec::new();
    while items.len() < wanted {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(item) => items.push(item),
            Err(_) => break,
        }
    }
    items
}

fn join(runtime: &Runtime, tasks: Vec<JoinHandle<()>>) {
    runtime.block_on(async {
        for task in tasks {
            task.await.unwrap();
        }
    });
}

struct GuardedCommitAuthority {
    gate: Arc<IoGate>,
    commits: std::sync::atomic::AtomicUsize,
    completed: mpsc::Sender<()>,
}

impl CpaDownstreamCredentialPort for GuardedCommitAuthority {
    fn lease_downstream_capability(
        &self,
        _request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        panic!("the resolver must preserve the caller's scoped request lifetime")
    }

    fn lease_downstream_capability_scoped(
        &self,
        _request: ExactCpaCredentialRequest<'_>,
        context: &hiroute_cpa_bridge::CpaRequestContext,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        context.run(|| {
            context
                .ensure_active()
                .map_err(|_| CpaAttemptError::Unavailable)?;
            self.gate.block();
            let result = context
                .ensure_active()
                .map(|()| {
                    self.commits
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    None
                })
                .map_err(|_| CpaAttemptError::Unavailable);
            self.completed.send(()).unwrap();
            result
        })
    }
}

#[test]
fn issuer_cancellation_is_visible_before_the_pending_gateway_future_is_polled_again() {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    let runtime = runtime();
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    let (completed, done) = mpsc::channel();
    let authority = Arc::new(GuardedCommitAuthority {
        gate: Arc::clone(&gate),
        commits: std::sync::atomic::AtomicUsize::new(0),
        completed,
    });
    let resolver = GatewayCredentialResolver::new(
        Arc::new(FakeNativeAuthority::default()),
        Arc::clone(&authority),
    );
    let cancellation = CancellationToken::new();
    let scope = ExecutionScope::new(
        Instant::now() + Duration::from_secs(3),
        cancellation.clone(),
    );
    let mut future = Box::pin(cpa_lease(
        &resolver,
        Provider::Claude,
        "cancel-without-poll",
        &scope,
    ));
    let mut context = Context::from_waker(Waker::noop());
    {
        let _entered = runtime.enter();
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    }
    gate.wait_entered(1);
    cancellation.cancel();
    gate.release();
    done.recv_timeout(OBSERVATION_BOUND)
        .expect("blocking authority did not complete");
    assert_eq!(
        authority.commits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "issuer cancellation was invisible until the gateway future was polled or dropped"
    );
    assert!(runtime.block_on(future).is_err());
}

#[test]
fn slow_cpa_work_keeps_two_gateway_workers_and_the_other_provider_available() {
    for provider in [Provider::Claude, Provider::Codex] {
        let runtime = runtime();
        let gate = Arc::new(IoGate::default());
        let _release = ReleaseOnDrop(Arc::clone(&gate));
        let (resolver, _) = resolver(provider, &gate);
        let (completed, completions) = mpsc::channel();
        let mut tasks = (0..2)
            .map(|index| {
                spawn_lease(
                    &runtime,
                    &resolver,
                    provider,
                    &format!("credential/cpa/blocked/{index}"),
                    scope(),
                    &completed,
                )
            })
            .collect::<Vec<_>>();
        gate.wait_entered(2);
        tasks.push(spawn_lease(
            &runtime,
            &resolver,
            provider.sibling(),
            "credential/cpa/independent",
            scope(),
            &completed,
        ));
        let ticker = completed.clone();
        tasks.push(runtime.spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticker
                .send(("ticker".into(), Outcome::NoCapability))
                .unwrap();
        }));
        let native_resolver = Arc::clone(&resolver);
        let native = completed.clone();
        tasks.push(runtime.spawn(async move {
            let lease = native_resolver
                .lease_header_secret(
                    HeaderSecretLeaseRequest {
                        secret_ref: "secret/native-independent",
                        header_name: "x-classifier-key",
                    },
                    &scope(),
                )
                .await
                .unwrap()
                .unwrap();
            let mut headers = HeaderMap::new();
            lease.apply_authorization(&mut headers).unwrap();
            assert_eq!(headers["x-classifier-key"], "native-sentinel");
            native
                .send(("native".into(), Outcome::NoCapability))
                .unwrap();
        }));
        let progress = collect(&completions, 3);
        let while_blocked = gate.counts();
        gate.release();
        join(&runtime, tasks);
        assert_eq!(while_blocked, (2, 0));
        assert_eq!(
            progress.len(),
            3,
            "independent work stalled for {provider:?}"
        );
        assert!(
            progress
                .iter()
                .all(|(_, result)| *result == Outcome::NoCapability)
        );
        let ids = progress
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ids,
            BTreeSet::from(["ticker", "native", "credential/cpa/independent"])
        );
    }
}

#[test]
fn running_cancellation_returns_before_io_but_preserves_capacity_until_io_exits() {
    let runtime = runtime();
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    let (resolver, authority) = resolver(Provider::Claude, &gate);
    let (completed, completions) = mpsc::channel();
    let cancellation = CancellationToken::new();
    let mut tasks = (0..2)
        .map(|index| {
            spawn_lease(
                &runtime,
                &resolver,
                Provider::Claude,
                &format!("credential/cpa/cancelled/{index}"),
                ExecutionScope::new(
                    Instant::now() + Duration::from_secs(5),
                    cancellation.clone(),
                ),
                &completed,
            )
        })
        .collect::<Vec<_>>();
    gate.wait_entered(2);
    cancellation.cancel();
    let early_cancel = collect(&completions, 2);
    tasks.push(spawn_lease(
        &runtime,
        &resolver,
        Provider::Claude,
        "credential/cpa/must-wait-for-real-exit",
        ExecutionScope::new(
            Instant::now() + Duration::from_millis(150),
            CancellationToken::new(),
        ),
        &completed,
    ));
    let queued_deadline = collect(&completions, 1);
    let while_cancelled = gate.counts();
    gate.release();
    join(&runtime, tasks);
    assert_eq!(
        early_cancel.len(),
        2,
        "cancelled requests waited for blocking IO"
    );
    assert!(
        early_cancel
            .iter()
            .all(|(_, result)| *result == Outcome::Rejected)
    );
    assert_eq!(
        queued_deadline,
        vec![(
            "credential/cpa/must-wait-for-real-exit".into(),
            Outcome::Rejected
        )]
    );
    assert_eq!(
        while_cancelled,
        (2, 0),
        "cancellation released still-running IO capacity"
    );
    assert_eq!(
        authority.calls.lock().unwrap().len(),
        2,
        "expired queued work entered CPA"
    );
}

#[test]
fn queued_cancellation_and_deadline_never_enter_cpa_after_capacity_returns() {
    let runtime = runtime();
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    let (resolver, authority) = resolver(Provider::Codex, &gate);
    let (completed, completions) = mpsc::channel();
    let mut tasks = (0..2)
        .map(|index| {
            spawn_lease(
                &runtime,
                &resolver,
                Provider::Codex,
                &format!("credential/cpa/active/{index}"),
                scope(),
                &completed,
            )
        })
        .collect::<Vec<_>>();
    gate.wait_entered(2);
    let cancelled = CancellationToken::new();
    tasks.push(spawn_lease(
        &runtime,
        &resolver,
        Provider::Codex,
        "credential/cpa/queued-cancel",
        ExecutionScope::new(Instant::now() + Duration::from_secs(5), cancelled.clone()),
        &completed,
    ));
    tasks.push(spawn_lease(
        &runtime,
        &resolver,
        Provider::Codex,
        "credential/cpa/queued-deadline",
        ExecutionScope::new(
            Instant::now() + Duration::from_millis(150),
            CancellationToken::new(),
        ),
        &completed,
    ));
    cancelled.cancel();
    let rejected = collect(&completions, 2);
    let before_release = gate.counts();
    gate.release();
    join(&runtime, tasks);
    assert_eq!(
        rejected.len(),
        2,
        "queued scope termination waited for active IO"
    );
    assert!(
        rejected
            .iter()
            .all(|(_, result)| *result == Outcome::Rejected)
    );
    assert_eq!(before_release, (2, 0));
    assert_eq!(
        authority.calls.lock().unwrap().len(),
        2,
        "a terminated queue entry entered CPA later"
    );
    assert!(
        runtime
            .block_on(cpa_lease(
                &resolver,
                Provider::Codex,
                "credential/cpa/valid-after-drain",
                &scope()
            ))
            .unwrap()
            .is_none()
    );
    assert_eq!(authority.calls.lock().unwrap().len(), 3);
}

#[test]
fn active_cpa_deadline_is_bounded_by_the_request_instead_of_blocking_io() {
    let runtime = runtime();
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    let (resolver, _) = resolver(Provider::Claude, &gate);
    let (completed, completions) = mpsc::channel();
    let task = spawn_lease(
        &runtime,
        &resolver,
        Provider::Claude,
        "credential/cpa/active-deadline",
        ExecutionScope::new(
            Instant::now() + Duration::from_millis(150),
            CancellationToken::new(),
        ),
        &completed,
    );
    gate.wait_entered(1);
    let early = collect(&completions, 1);
    let before_release = gate.counts();
    gate.release();
    join(&runtime, vec![task]);
    assert_eq!(
        early,
        vec![("credential/cpa/active-deadline".into(), Outcome::Rejected)]
    );
    assert_eq!(before_release, (1, 0));
}

#[test]
fn provider_queue_rejects_overflow_without_creating_more_blocking_work() {
    const REQUESTS: usize = 64;
    const ADMITTED: usize = 2 + 16;
    let runtime = runtime();
    let gate = Arc::new(IoGate::default());
    let _release = ReleaseOnDrop(Arc::clone(&gate));
    let (resolver, authority) = resolver(Provider::Claude, &gate);
    let (completed, completions) = mpsc::channel();
    let tasks = (0..REQUESTS)
        .map(|index| {
            spawn_lease(
                &runtime,
                &resolver,
                Provider::Claude,
                &format!("credential/cpa/burst/{index}"),
                scope(),
                &completed,
            )
        })
        .collect::<Vec<_>>();
    gate.wait_entered(2);
    let overflow = collect(&completions, REQUESTS - ADMITTED);
    let before_release = gate.counts();
    gate.release();
    join(&runtime, tasks);
    let admitted = collect(&completions, ADMITTED);
    assert_eq!(
        before_release,
        (2, 0),
        "provider burst created unbounded blocking work"
    );
    assert_eq!(
        overflow.len(),
        REQUESTS - ADMITTED,
        "provider overflow did not reject promptly"
    );
    assert!(
        overflow
            .iter()
            .all(|(_, result)| *result == Outcome::Rejected)
    );
    assert_eq!(admitted.len(), ADMITTED);
    assert!(
        admitted
            .iter()
            .all(|(_, result)| *result == Outcome::NoCapability)
    );
    assert_eq!(
        authority.calls.lock().unwrap().len(),
        ADMITTED,
        "rejected overflow reached CPA"
    );
}
