#![allow(dead_code)]

mod publication;
mod publication_update;
mod wire;

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hiroute_e2e::gateway_fixture::{
    E2E_ATTEMPT_TIMEOUT_MS_ENV, E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener,
    TestTlsStream, read_complete_http_request, read_http_request_with_body_barrier,
    write_dial_config_with_dns_failures,
};
use serde_json::json;

use publication::{PublicationRouteMode, snapshot};

#[allow(unused_imports)]
pub use wire::{WireResponse, open_request, request, request_with_timeout, wire_header};

pub const MODEL: &str = "runtime-model";
pub const TOKEN: &str = "runtime-token";
pub const CHAT_TOKEN: &str = "runtime-chat-token";
pub const MESSAGES_TOKEN: &str = "runtime-messages-token";

#[derive(Clone)]
pub enum ProviderReply {
    Complete {
        status: u16,
        error_kind: Option<&'static str>,
        body: &'static [u8],
    },
    CompleteWithRetryAfter {
        status: u16,
        retry_after_secs: u64,
        body: &'static [u8],
    },
    CompleteWithRequestId {
        status: u16,
        request_id: &'static str,
        body: &'static [u8],
    },
    DelayedComplete {
        duration: Duration,
        status: u16,
        error_kind: Option<&'static str>,
        body: &'static [u8],
    },
    BodyReadBarrier {
        barrier: ProviderBodyBarrier,
        status: u16,
        error_kind: Option<&'static str>,
        body: &'static [u8],
    },
    EarlyComplete {
        status: u16,
        body: &'static [u8],
    },
    StreamComplete {
        status: u16,
        body: &'static [u8],
    },
    StreamDrip {
        status: u16,
        chunks: Vec<Vec<u8>>,
        interval: Duration,
    },
    CloseBeforeSemantic,
    PartialThenClose {
        body: &'static [u8],
        declared_length: usize,
    },
    StreamThenClose {
        body: &'static [u8],
        declared_length: usize,
    },
    Stall {
        duration: Duration,
    },
}

#[derive(Clone, Default)]
pub struct ProviderBodyBarrier {
    inner: Arc<(Mutex<ProviderBodyBarrierState>, Condvar)>,
}

#[derive(Default)]
struct ProviderBodyBarrierState {
    arrived: usize,
    released: bool,
}

impl ProviderBodyBarrier {
    pub fn wait_for_arrivals(&self, expected: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let (state, changed) = &*self.inner;
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.arrived < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "only {} of {expected} provider body readers reached the barrier",
                state.arrived
            );
            let (next, result) = changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            assert!(
                !result.timed_out() || state.arrived >= expected,
                "only {} of {expected} provider body readers reached the barrier",
                state.arrived
            );
        }
    }

    pub fn release(&self) {
        let (state, changed) = &*self.inner;
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.released = true;
        changed.notify_all();
    }

    fn arrive_and_wait(&self) {
        let (state, changed) = &*self.inner;
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.arrived += 1;
        changed.notify_all();
        while !state.released {
            state = changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

pub struct NativeProvider {
    address: SocketAddr,
    transport: TestTlsListener,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _serial: ProviderSerialLease,
}

impl NativeProvider {
    pub fn start(replies: Vec<ProviderReply>) -> Self {
        let serial = ProviderSerialLease::acquire();
        static PROVIDER_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let authority = format!(
            "runtime-provider-{}.invalid",
            PROVIDER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let listener = TestTlsListener::bind(authority).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let transport = listener.try_clone().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_calls = Arc::clone(&calls);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut replies = VecDeque::from(replies);
            let mut workers = Vec::new();
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .unwrap();
                        thread_calls.fetch_add(1, Ordering::Relaxed);
                        let reply = replies.pop_front().unwrap_or(ProviderReply::Complete {
                            status: 500,
                            error_kind: None,
                            body: b"unexpected extra Attempt",
                        });
                        let worker_requests = Arc::clone(&thread_requests);
                        workers.push(std::thread::spawn(move || {
                            if matches!(&reply, ProviderReply::EarlyComplete { .. }) {
                                let _ = write_provider_reply(&mut stream, reply);
                                let request = read_complete_http_request(
                                    &mut stream,
                                    Duration::from_secs(10),
                                )
                                .unwrap_or_default();
                                worker_requests.lock().unwrap().push(request);
                            } else {
                                let request = match &reply {
                                    ProviderReply::BodyReadBarrier { barrier, .. } => {
                                        read_http_request_with_body_barrier(
                                            &mut stream,
                                            Duration::from_secs(10),
                                            1,
                                            || barrier.arrive_and_wait(),
                                        )
                                    }
                                    _ => read_complete_http_request(
                                        &mut stream,
                                        Duration::from_secs(10),
                                    ),
                                }
                                .unwrap_or_default();
                                worker_requests.lock().unwrap().push(request);
                                let _ = write_provider_reply(&mut stream, reply);
                            }
                            let _ = stream.finish();
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            address,
            transport,
            calls,
            requests,
            stop,
            thread: Some(thread),
            _serial: serial,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn authority(&self) -> &str {
        self.transport.authority()
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    pub fn requests(&self) -> Vec<Vec<u8>> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for NativeProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

struct ProviderSerialLease;

type ProviderSerialState = (Mutex<Option<(std::thread::ThreadId, usize)>>, Condvar);

impl ProviderSerialLease {
    fn acquire() -> Self {
        let owner = std::thread::current().id();
        let (state, available) = provider_serial_state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            match state.as_mut() {
                Some((current, leases)) if *current == owner => {
                    *leases += 1;
                    return Self;
                }
                Some(_) => {
                    state = available
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                None => {
                    *state = Some((owner, 1));
                    return Self;
                }
            }
        }
    }
}

impl Drop for ProviderSerialLease {
    fn drop(&mut self) {
        let (state, available) = provider_serial_state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_, leases) = state.as_mut().expect("provider serial lease is held");
        *leases -= 1;
        if *leases == 0 {
            *state = None;
            available.notify_one();
        }
    }
}

fn provider_serial_state() -> &'static ProviderSerialState {
    static STATE: OnceLock<ProviderSerialState> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(None), Condvar::new()))
}

fn write_provider_reply(stream: &mut TestTlsStream, reply: ProviderReply) -> std::io::Result<()> {
    match reply {
        ProviderReply::Complete {
            status,
            error_kind,
            body,
        } => {
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                422 => "Unprocessable Entity",
                429 => "Too Many Requests",
                503 => "Service Unavailable",
                _ => "Provider Response",
            };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            )?;
            if let Some(kind) = error_kind {
                write!(stream, "x-hiroute-error-kind: {kind}\r\n")?;
            }
            write!(stream, "\r\n")?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::CompleteWithRetryAfter {
            status,
            retry_after_secs,
            body,
        } => {
            write!(
                stream,
                "HTTP/1.1 {status} Provider Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: {retry_after_secs}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::DelayedComplete {
            duration,
            status,
            error_kind,
            body,
        } => {
            std::thread::sleep(duration);
            return write_provider_reply(
                stream,
                ProviderReply::Complete {
                    status,
                    error_kind,
                    body,
                },
            );
        }
        ProviderReply::BodyReadBarrier {
            status,
            error_kind,
            body,
            ..
        } => {
            return write_provider_reply(
                stream,
                ProviderReply::Complete {
                    status,
                    error_kind,
                    body,
                },
            );
        }
        ProviderReply::CompleteWithRequestId {
            status,
            request_id,
            body,
        } => {
            write!(
                stream,
                "HTTP/1.1 {status} Provider Response\r\nContent-Type: application/json\r\nX-Request-Id: {request_id}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::EarlyComplete { status, body } => {
            write!(
                stream,
                "HTTP/1.1 {status} Early\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::StreamComplete { status, body } => {
            write!(
                stream,
                "HTTP/1.1 {status} Provider Response\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::StreamDrip {
            status,
            chunks,
            interval,
        } => {
            let content_length = chunks.iter().map(Vec::len).sum::<usize>();
            write!(
                stream,
                "HTTP/1.1 {status} Provider Response\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            )?;
            for chunk in chunks {
                stream.write_all(&chunk)?;
                stream.flush()?;
                std::thread::sleep(interval);
            }
        }
        ProviderReply::CloseBeforeSemantic => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
            )?;
            stream.flush()?;
        }
        ProviderReply::PartialThenClose {
            body,
            declared_length,
        } => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::StreamThenClose {
            body,
            declared_length,
        } => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
            )?;
            stream.write_all(body)?;
            stream.flush()?;
        }
        ProviderReply::Stall { duration } => {
            std::thread::sleep(duration);
        }
    }
    Ok(())
}

pub struct RuntimeFixture {
    pub process: Hirouted,
    pub address: SocketAddr,
    pub replay_root: PathBuf,
    pub observation_root: Option<PathBuf>,
    pub diagnostics_root: Option<PathBuf>,
    runtime_state_control: Option<(SocketAddr, String)>,
    _serial: std::sync::MutexGuard<'static, ()>,
    _directory: tempfile::TempDir,
}
#[derive(Clone, Copy)]
struct RuntimeLaunchOptions {
    replay_threshold: usize,
    replay_record_bytes: usize,
    overall_timeout_ms: u64,
    classifier_timeout_ms: u64,
    replay_root_mode: ReplayRootMode,
    orphan_ttl_ms: u64,
    observation: Option<ObservationFaults>,
    diagnostic_level: Option<&'static str>,
    test_control: bool,
    classified_route: bool,
    rest_classifier: bool,
    decision_branch_preset: Option<bool>,
    fixed_route: bool,
    reasoning_choices: bool,
    native_profile_targets: bool,
    isolated_fixed_grants: bool,
    attempt_timeout_ms: Option<u64>,
}

#[derive(Clone, Copy)]
pub struct ObservationFaults {
    pub lifecycle_mode: &'static str,
    pub execution_mode: &'static str,
    pub content_mode: &'static str,
    pub otel_mode: &'static str,
    pub queue_bytes: usize,
}

impl ObservationFaults {
    pub const fn healthy() -> Self {
        Self {
            lifecycle_mode: "healthy",
            execution_mode: "healthy",
            content_mode: "healthy",
            otel_mode: "healthy",
            queue_bytes: 512 * 1024,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReplayRootMode {
    Normal,
    UnsafePermissions,
    SeedOrphan,
}

impl Default for RuntimeLaunchOptions {
    fn default() -> Self {
        Self {
            replay_threshold: 64 * 1024,
            replay_record_bytes: 16 * 1024,
            overall_timeout_ms: 30_000,
            classifier_timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
            replay_root_mode: ReplayRootMode::Normal,
            orphan_ttl_ms: 24 * 60 * 60 * 1_000,
            observation: None,
            diagnostic_level: None,
            test_control: false,
            classified_route: false,
            rest_classifier: false,
            decision_branch_preset: None,
            fixed_route: false,
            reasoning_choices: false,
            native_profile_targets: false,
            isolated_fixed_grants: false,
            attempt_timeout_ms: None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PublicationCandidate {
    pub provider_index: usize,
    pub upstream_protocol: &'static str,
    pub statically_enabled: bool,
}

impl RuntimeFixture {
    pub fn launch_builtin_branches(providers: &[&NativeProvider], smart: bool) -> Self {
        Self::launch_configured(
            providers,
            2,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                rest_classifier: true,
                decision_branch_preset: Some(smart),
                observation: Some(ObservationFaults::healthy()),
                test_control: true,
                ..Default::default()
            },
        )
    }

    pub fn launch(providers: &[&NativeProvider], max_attempts: u32) -> Self {
        Self::launch_with_publication_candidates(providers, max_attempts, None)
    }
    pub fn launch_isolated_fixed_grants(providers: &[&NativeProvider]) -> Self {
        assert_eq!(providers.len(), 2);
        Self::launch_configured(
            providers,
            1,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                isolated_fixed_grants: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_reasoning(providers: &[&NativeProvider], fixed: bool) -> Self {
        Self::launch_configured(
            providers,
            2,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                fixed_route: fixed,
                reasoning_choices: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_fixed(
        providers: &[&NativeProvider],
        max_attempts: u32,
        key_counts: &[usize],
        attempt_timeout_ms: Option<u64>,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            Some(key_counts),
            RuntimeLaunchOptions {
                fixed_route: true,
                attempt_timeout_ms,
                ..RuntimeLaunchOptions::default()
            },
        )
    }
    pub fn launch_with_runtime_state_control(
        providers: &[&NativeProvider],
        max_attempts: u32,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                observation: Some(ObservationFaults::healthy()),
                test_control: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_publication_candidates(
        providers: &[&NativeProvider],
        max_attempts: u32,
        publication_candidates: Option<&[PublicationCandidate]>,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            publication_candidates,
            None,
            None,
            RuntimeLaunchOptions::default(),
        )
    }

    pub fn launch_with_attempt_timeout(
        providers: &[&NativeProvider],
        max_attempts: u32,
        attempt_timeout: Duration,
    ) -> Self {
        let attempt_timeout_ms = u64::try_from(attempt_timeout.as_millis())
            .expect("attempt timeout must fit in milliseconds");
        assert!(attempt_timeout_ms > 0, "attempt timeout must be positive");
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                attempt_timeout_ms: Some(attempt_timeout_ms),
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_classified(providers: &[&NativeProvider], max_attempts: u32) -> Self {
        assert_eq!(
            providers.len(),
            2,
            "classified fixture has one candidate per branch"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_classified_with_observation(
        providers: &[&NativeProvider],
        max_attempts: u32,
        faults: ObservationFaults,
    ) -> Self {
        assert_eq!(providers.len(), 2);
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                observation: Some(faults),
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_info_diagnostics(providers: &[&NativeProvider], max_attempts: u32) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                diagnostic_level: Some("info"),
                reasoning_choices: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_classified_with_info_diagnostics(
        providers: &[&NativeProvider],
        max_attempts: u32,
    ) -> Self {
        assert_eq!(providers.len(), 2);
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                diagnostic_level: Some("info"),
                reasoning_choices: true,
                native_profile_targets: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified(providers: &[&NativeProvider], max_attempts: u32) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and classifier services"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified_with_classifier_timeout(
        providers: &[&NativeProvider],
        max_attempts: u32,
        classifier_timeout: Duration,
    ) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and classifier services"
        );
        let classifier_timeout_ms = u64::try_from(classifier_timeout.as_millis())
            .expect("classifier timeout must fit in milliseconds");
        assert!(
            classifier_timeout_ms > 0,
            "classifier timeout must be positive"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                classifier_timeout_ms,
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified_at(
        providers: &[&NativeProvider],
        max_attempts: u32,
        classifier_endpoint: &str,
    ) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and unused classifier services"
        );
        Self::launch_configured_with_classifier_endpoint(
            providers,
            max_attempts,
            None,
            None,
            None,
            Some(classifier_endpoint),
            RuntimeLaunchOptions {
                observation: Some(ObservationFaults::healthy()),
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified_with_overall_timeout(
        providers: &[&NativeProvider],
        max_attempts: u32,
        overall_timeout: Duration,
    ) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and classifier services"
        );
        let overall_timeout_ms = u64::try_from(overall_timeout.as_millis())
            .expect("overall timeout must fit in milliseconds");
        assert!(overall_timeout_ms > 0, "overall timeout must be positive");
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                overall_timeout_ms,
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified_with_observation(
        providers: &[&NativeProvider],
        max_attempts: u32,
        faults: ObservationFaults,
    ) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and classifier services"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                observation: Some(faults),
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_rest_classified_with_replay(
        providers: &[&NativeProvider],
        max_attempts: u32,
        replay_threshold: usize,
        replay_record_bytes: usize,
    ) -> Self {
        assert_eq!(
            providers.len(),
            3,
            "REST-classified fixture has simple, complex and classifier services"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                replay_threshold,
                replay_record_bytes,
                classified_route: true,
                rest_classifier: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_classified_with_publication_candidates(
        providers: &[&NativeProvider],
        max_attempts: u32,
        publication_candidates: &[PublicationCandidate],
    ) -> Self {
        assert_eq!(
            providers.len(),
            2,
            "classified fixture has one candidate per branch"
        );
        Self::launch_configured(
            providers,
            max_attempts,
            Some(publication_candidates),
            None,
            None,
            RuntimeLaunchOptions {
                classified_route: true,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_endpoints(
        providers: &[&NativeProvider],
        max_attempts: u32,
        endpoints: &[String],
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            Some(endpoints),
            None,
            RuntimeLaunchOptions::default(),
        )
    }

    pub fn launch_with_key_counts(
        providers: &[&NativeProvider],
        max_attempts: u32,
        key_counts: &[usize],
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            Some(key_counts),
            RuntimeLaunchOptions::default(),
        )
    }

    pub fn launch_with_replay(
        providers: &[&NativeProvider],
        max_attempts: u32,
        replay_threshold: usize,
        replay_record_bytes: usize,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                replay_threshold,
                replay_record_bytes,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_replay_timeout(
        providers: &[&NativeProvider],
        max_attempts: u32,
        replay_threshold: usize,
        overall_timeout_ms: u64,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                replay_threshold,
                replay_record_bytes: replay_threshold.min(1024),
                overall_timeout_ms,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_observation(
        providers: &[&NativeProvider],
        max_attempts: u32,
        faults: ObservationFaults,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                observation: Some(faults),
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_observation_and_publication_candidates(
        providers: &[&NativeProvider],
        max_attempts: u32,
        faults: ObservationFaults,
        publication_candidates: Option<&[PublicationCandidate]>,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            publication_candidates,
            None,
            None,
            RuntimeLaunchOptions {
                observation: Some(faults),
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    #[cfg(unix)]
    pub fn launch_with_unsafe_replay_root(
        providers: &[&NativeProvider],
        max_attempts: u32,
    ) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                replay_root_mode: ReplayRootMode::UnsafePermissions,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    pub fn launch_with_seed_orphan(providers: &[&NativeProvider], max_attempts: u32) -> Self {
        Self::launch_configured(
            providers,
            max_attempts,
            None,
            None,
            None,
            RuntimeLaunchOptions {
                replay_root_mode: ReplayRootMode::SeedOrphan,
                orphan_ttl_ms: 1,
                ..RuntimeLaunchOptions::default()
            },
        )
    }

    fn launch_configured(
        providers: &[&NativeProvider],
        max_attempts: u32,
        publication_candidates: Option<&[PublicationCandidate]>,
        endpoints: Option<&[String]>,
        key_counts: Option<&[usize]>,
        options: RuntimeLaunchOptions,
    ) -> Self {
        Self::launch_configured_with_classifier_endpoint(
            providers,
            max_attempts,
            publication_candidates,
            endpoints,
            key_counts,
            None,
            options,
        )
    }

    fn launch_configured_with_classifier_endpoint(
        providers: &[&NativeProvider],
        max_attempts: u32,
        publication_candidates: Option<&[PublicationCandidate]>,
        endpoints: Option<&[String]>,
        key_counts: Option<&[usize]>,
        classifier_endpoint: Option<&str>,
        options: RuntimeLaunchOptions,
    ) -> Self {
        // A real hirouted process owns a Pingora worker pool. Serializing
        // fixtures keeps the listener assertions deterministic under the Rust
        // test harness instead of turning scheduler starvation into a request
        // deadline failure.
        let serial = runtime_fixture_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let control = options.test_control.then(|| {
            let mut random = [0_u8; 32];
            getrandom::fill(&mut random).unwrap();
            let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = directory.path().join("runtime-state-control.nonce");
            let mut open = std::fs::OpenOptions::new();
            open.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(0o600);
            }
            let mut file = open.open(&path).unwrap();
            file.write_all(nonce.as_bytes()).unwrap();
            file.sync_all().unwrap();
            ((reserve_address(), nonce), path)
        });
        let route_mode = match (options.classified_route, options.rest_classifier) {
            (false, false) => PublicationRouteMode::Ordered,
            (true, false) => PublicationRouteMode::ClassifiedLocal,
            (true, true) => PublicationRouteMode::ClassifiedRest {
                endpoint_override: classifier_endpoint,
                timeout_ms: options.classifier_timeout_ms,
            },
            (false, true) => panic!("REST classification requires a classified route"),
        };
        let mut publication = snapshot(
            providers,
            max_attempts,
            publication_candidates,
            endpoints,
            key_counts,
            options.overall_timeout_ms,
            route_mode,
        );
        if let Some(smart) = options.decision_branch_preset {
            publication::use_builtin_branch_decision(&mut publication, smart);
        }
        if options.reasoning_choices {
            publication::add_reasoning_choices(&mut publication);
        }
        if options.native_profile_targets {
            publication::add_native_profile_targets(&mut publication);
        }
        if options.fixed_route {
            publication::make_first_route_fixed(&mut publication);
        }
        if options.isolated_fixed_grants {
            publication::make_isolated_fixed_grants(&mut publication);
        }
        if let Some(key_counts) = key_counts {
            assert_eq!(key_counts.len(), providers.len());
            assert!(key_counts.iter().all(|count| *count > 0));
        }
        let publication_path = directory.path().join("publication.json");
        let credentials_path = directory.path().join("credentials.json");
        let lkg_path = directory.path().join("publication-lkg.json");
        let replay_root = directory.path().join("replay");
        let observation_root = options
            .observation
            .map(|_| directory.path().join("observation"));
        let diagnostics_root = options
            .diagnostic_level
            .map(|_| directory.path().join("diagnostics"));
        if let Some(root) = &observation_root {
            std::fs::create_dir(root).unwrap();
        }
        prepare_replay_root(&replay_root, options.replay_root_mode);
        std::fs::write(
            &publication_path,
            serde_json::to_vec_pretty(&publication).unwrap(),
        )
        .unwrap();
        let mut credentials = BTreeMap::new();
        for (index, _) in providers.iter().enumerate() {
            let key_count = key_counts.map_or(1, |counts| counts[index]);
            for (key_index, credential_ref) in
                credential_refs(index, key_count).into_iter().enumerate()
            {
                let suffix = credential_suffix(index, key_index, key_count);
                let file_name = format!("{credential_ref}.json");
                std::fs::write(
                    directory.path().join(&file_name),
                    serde_json::to_vec_pretty(&json!({
                        "schema_version": "hiroute.gateway.credential-leases/v1",
                        "credential_ref": credential_ref,
                        "keys": [{
                            "key_id": format!("key-{suffix}"),
                            "generation": 1,
                            "authorization": format!("Bearer provider-secret-{suffix}"),
                        }],
                    }))
                    .unwrap(),
                )
                .unwrap();
                credentials.insert(credential_ref, file_name);
            }
        }
        std::fs::write(
            &credentials_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "hiroute.gateway.credentials/v1",
                "credentials": credentials,
            }))
            .unwrap(),
        )
        .unwrap();
        let dns_failures = endpoints
            .into_iter()
            .flatten()
            .filter_map(|endpoint| {
                endpoint
                    .contains("does-not-resolve.invalid")
                    .then_some("does-not-resolve.invalid")
            })
            .collect::<Vec<_>>();
        write_dial_config_with_dns_failures(
            directory.path(),
            &providers
                .iter()
                .map(|provider| &provider.transport)
                .collect::<Vec<_>>(),
            &dns_failures,
        )
        .unwrap();
        let address = reserve_address();
        let binary = exact_hirouted_binary();
        let mut process = Hirouted::spawn(HiroutedSpawn {
            binary: &binary,
            address,
            lkg: &lkg_path,
            publication: &publication_path,
            credentials: &credentials_path,
            directory: directory.path(),
            replay_root: &replay_root,
            observation_root: observation_root.as_deref(),
            test_control: control
                .as_ref()
                .map(|((address, _), path)| (*address, path.as_path())),
            options,
        });
        process.wait_ready();
        assert!(control.as_ref().is_none_or(|(_, path)| !path.exists()));
        Self {
            _directory: directory,
            process,
            address,
            replay_root,
            observation_root,
            diagnostics_root,
            runtime_state_control: control.map(|(control, _)| control),
            _serial: serial,
        }
    }

    pub fn request(&self) -> WireResponse {
        self.request_body(br#"{"model":"runtime-model","input":"hello","stream":false}"#)
    }

    pub fn arm_runtime_state_fault(
        &self,
        request_id: &str,
        operation: &str,
        failure_count: u32,
    ) -> WireResponse {
        let (address, nonce) = self
            .runtime_state_control
            .as_ref()
            .expect("runtime-state test control is enabled");
        let body = serde_json::to_vec(&json!({
            "schema_version": "hiroute.gateway.e2e-control-request/v1",
            "request_id": request_id,
            "command": { "kind": "runtime_state_write_fault", "operation": operation,
                "failure_count": failure_count }
        }))
        .unwrap();
        request(
            *address,
            "POST",
            "/_hiroute/e2e-control/v1",
            &[("X-HiRoute-E2E-Control-Nonce", nonce)],
            &body,
        )
    }
    pub fn request_body(&self, body: &[u8]) -> WireResponse {
        request(
            self.address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", "runtime-token")],
            body,
        )
    }

    pub fn request_body_with_timeout(
        &self,
        body: &[u8],
        response_timeout: Duration,
    ) -> WireResponse {
        request_with_timeout(
            self.address,
            "POST",
            "/v1/responses",
            &[("X-HiRoute-Token", "runtime-token")],
            body,
            response_timeout,
        )
    }
}

fn runtime_fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn credential_refs(provider_index: usize, key_count: usize) -> Vec<String> {
    (0..key_count)
        .map(|key_index| {
            let suffix = credential_suffix(provider_index, key_index, key_count);
            format!("credential-{suffix}")
        })
        .collect()
}

fn credential_suffix(provider_index: usize, key_index: usize, key_count: usize) -> String {
    if key_count == 1 {
        format!("{}", provider_index + 1)
    } else {
        format!("{}-{}", provider_index + 1, key_index + 1)
    }
}

pub struct Hirouted {
    child: Option<Child>,
    address: SocketAddr,
    stderr_path: PathBuf,
}

struct HiroutedSpawn<'a> {
    binary: &'a Path,
    address: SocketAddr,
    lkg: &'a Path,
    publication: &'a Path,
    credentials: &'a Path,
    directory: &'a Path,
    replay_root: &'a Path,
    observation_root: Option<&'a Path>,
    test_control: Option<(SocketAddr, &'a Path)>,
    options: RuntimeLaunchOptions,
}

impl Hirouted {
    fn spawn(spawn: HiroutedSpawn<'_>) -> Self {
        let HiroutedSpawn {
            binary,
            address,
            lkg,
            publication,
            credentials,
            directory,
            replay_root,
            observation_root,
            test_control,
            options,
        } = spawn;
        let stdout = File::create(directory.join("hirouted-runtime.stdout")).unwrap();
        let stderr_path = directory.join("hirouted-runtime.stderr");
        let stderr = File::create(&stderr_path).unwrap();
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env(E2E_DIAL_CONFIG_ENV, directory.join(E2E_DIAL_CONFIG_FILE))
            .env("HIROUTE_REPLAY_ROOT", replay_root)
            .env(
                "HIROUTE_REPLAY_MEMORY_THRESHOLD",
                options.replay_threshold.to_string(),
            )
            .env(
                "HIROUTE_REPLAY_RECORD_BYTES",
                options.replay_record_bytes.to_string(),
            )
            .env(
                "HIROUTE_REPLAY_ORPHAN_TTL_MS",
                options.orphan_ttl_ms.to_string(),
            )
            .args(["--listen", &address.to_string(), "--lkg"])
            .arg(lkg)
            .arg("--publication")
            .arg(publication)
            .arg("--credentials")
            .arg(credentials);
        if let Some(level) = options.diagnostic_level {
            command
                .arg("--diagnostics-root")
                .arg(directory.join("diagnostics"))
                .args(["--diagnostic-level-override", level]);
        }
        if let Some(attempt_timeout_ms) = options.attempt_timeout_ms {
            command.env(E2E_ATTEMPT_TIMEOUT_MS_ENV, attempt_timeout_ms.to_string());
        }
        if let Some((address, nonce_file)) = test_control {
            command
                .args(["--e2e-control-listen", &address.to_string()])
                .arg("--e2e-control-nonce-file")
                .arg(nonce_file);
        }
        if let (Some(root), Some(observation)) = (observation_root, options.observation) {
            command
                .env("HIROUTE_E2E_OBSERVATION_CAPTURE", "1")
                .env("HIROUTE_OBSERVATION_DIRECTORY", root)
                .env(
                    "HIROUTE_OBSERVATION_QUEUE_BYTES",
                    observation.queue_bytes.to_string(),
                )
                .env(
                    "HIROUTE_OBSERVATION_LIFECYCLE_SINK",
                    observation.lifecycle_mode,
                )
                .env(
                    "HIROUTE_OBSERVATION_EXECUTION_SINK",
                    observation.execution_mode,
                )
                .env("HIROUTE_OBSERVATION_CONTENT_SINK", observation.content_mode)
                .env("HIROUTE_OBSERVATION_OTEL_SINK", observation.otel_mode);
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap();
        Self {
            child: Some(child),
            address,
            stderr_path,
        }
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if wire::try_request_with_timeout(
                self.address,
                "GET",
                "/_hiroute/ready",
                &[],
                b"",
                Duration::from_millis(250),
            )
            .is_ok_and(|response| response.status == 200)
            {
                return;
            }
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "hirouted exited before readiness"
            );
            assert!(Instant::now() < deadline, "hirouted readiness timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("hirouted child").id()
    }
}

impl Drop for Hirouted {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking()
            && let Ok(stderr) = std::fs::read_to_string(&self.stderr_path)
            && !stderr.is_empty()
        {
            eprintln!("hirouted stderr:\n{stderr}");
        }
    }
}

fn exact_hirouted_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
                let path = PathBuf::from(
                    std::env::var_os("HIROUTE_VALIDATION_GATEWAY_BIN")
                        .expect("prepared Gateway binary missing"),
                );
                assert!(path.is_file());
                return path;
            }
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let output = Command::new(cargo)
                .current_dir(&root)
                .args([
                    "build",
                    "--locked",
                    "--offline",
                    "-p",
                    "hiroute-gateway",
                    "--bin",
                    "hirouted",
                    "--all-features",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "exact hirouted build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }
                })
                .unwrap_or_else(|| root.join("target"));
            target.join("debug").join(if cfg!(windows) {
                "hirouted.exe"
            } else {
                "hirouted"
            })
        })
        .clone()
}

fn reserve_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn prepare_replay_root(path: &Path, mode: ReplayRootMode) {
    match mode {
        ReplayRootMode::Normal => {}
        ReplayRootMode::UnsafePermissions => {
            std::fs::create_dir(path).expect("create unsafe replay root");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                    .expect("set unsafe replay permissions");
            }
        }
        ReplayRootMode::SeedOrphan => {
            std::fs::create_dir(path).expect("create replay root");
            let orphan = path.join("req-seeded-restart-orphan");
            std::fs::create_dir(&orphan).expect("create replay orphan");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .expect("set replay root permissions");
                std::fs::set_permissions(&orphan, std::fs::Permissions::from_mode(0o700))
                    .expect("set orphan permissions");
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }
}
