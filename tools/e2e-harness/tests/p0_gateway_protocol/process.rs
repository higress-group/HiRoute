use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use hiroute_e2e::gateway_fixture::{
    E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener, TestTlsStream,
    sealed_native_candidate, write_dial_config,
};
use hiroute_gateway::server::publication::{
    AliasPlanV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

#[test]
fn protocol_real_hirouted_pingora_decodes_all_ingresses_and_rejects_before_provider() {
    let _serial = process_test_lock();
    let directory = tempfile::tempdir().unwrap();
    let provider = TestTlsListener::bind("protocol-provider.invalid").unwrap();
    provider.set_nonblocking(true).unwrap();
    let publication_path = directory.path().join("publication.json");
    let credentials_path = directory.path().join("credentials.json");
    let lkg_path = directory.path().join("publication-lkg.json");
    std::fs::write(
        &publication_path,
        serde_json::to_vec_pretty(&snapshot(provider.authority())).unwrap(),
    )
    .unwrap();
    write_dial_config(directory.path(), &[&provider]).unwrap();
    std::fs::write(
        directory.path().join("protocol-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "protocol-credential",
            "keys": [{
                "key_id": "protocol-key",
                "generation": 1,
                "authorization": "Bearer protocol-provider-secret"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &credentials_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {
                "protocol-credential": "protocol-credential.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let address = reserve_address();
    let mut process = Hirouted::spawn(
        &exact_hirouted_binary(),
        address,
        &lkg_path,
        &publication_path,
        &credentials_path,
        directory.path(),
    );
    process.wait_ready();

    let cases = [
        (
            "/v1/responses",
            json!({"model":"wire-protocol","input":"hello","stream":false}),
        ),
        (
            "/v1/chat/completions",
            json!({"model":"wire-protocol","messages":[{"role":"user","content":"hello"}],"stream":false}),
        ),
        (
            "/v1/messages",
            json!({"model":"wire-protocol","max_tokens":8,"messages":[{"role":"user","content":"hello"}],"stream":false}),
        ),
    ];
    let provider_thread = serve_provider_responses(
        provider.try_clone().unwrap(),
        vec![
            ("/v1/responses", br#"{"id":"native-provider-responses","model":"provider-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"# as &'static [u8]),
            ("/v1/responses", br#"{"id":"native-provider-chat","model":"provider-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#),
            ("/v1/responses", br#"{"id":"native-provider-messages","model":"provider-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#),
        ],
    );
    for (index, (path, value)) in cases.into_iter().enumerate() {
        let response = fragmented_request(address, path, &serde_json::to_vec(&value).unwrap());
        assert_eq!(
            response.status,
            200,
            "{path}: {}; hirouted stderr: {}",
            String::from_utf8_lossy(&response.body),
            process.stderr()
        );
        let response: Value = serde_json::from_slice(&response.body).unwrap();
        let text = match index {
            0 => &response["output"][0]["content"][0]["text"],
            1 => &response["choices"][0]["message"]["content"],
            2 => &response["content"][0]["text"],
            _ => unreachable!(),
        };
        assert_eq!(text, "ok");
    }
    let invalid = single_write_request(
        address,
        "/v1/responses",
        br#"{"model":"wire-protocol","input":"hello","base_url":"https://not-authorized.invalid"}"#,
    );
    assert_eq!(invalid.status, 400);
    let invalid: Value = serde_json::from_slice(&invalid.body).unwrap();
    assert_eq!(invalid["phase"], "canonical_request");
    assert_eq!(invalid["code"], "PROTOCOL_SEMANTICS_UNSUPPORTED");
    assert_eq!(
        provider_accept_error_kind(&provider),
        std::io::ErrorKind::WouldBlock,
        "authority, canonical decode, and rejection must not connect to Provider"
    );
    process.stop();
    provider_thread.join().unwrap();
}

pub(super) fn process_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static NEXT_RESERVED_PORT: AtomicUsize = AtomicUsize::new(0);

fn snapshot(provider_authority: &str) -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "protocol-authority",
        13,
        55,
        "protocol-renderer/v1",
        vec![AliasPlanV1 {
            served_model_id: "wire-protocol".into(),
            purpose: "canonical protocol projection".into(),
            agent_plan_revision: 89,
            protocols: vec![
                IngressProtocol::Responses,
                IngressProtocol::ChatCompletions,
                IngressProtocol::Messages,
            ],
            overall_timeout_ms: 10_000,
            max_attempts: 1,
            routing: None,
            candidates: vec![sealed_native_candidate(
                1,
                "protocol-target",
                &["protocol-credential".into()],
                provider_authority,
                "protocol-native-model",
                &[
                    (IngressProtocol::Responses, IngressProtocol::Responses),
                    (IngressProtocol::ChatCompletions, IngressProtocol::Responses),
                    (IngressProtocol::Messages, IngressProtocol::Responses),
                ],
            )],
        }],
        [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ]
        .into_iter()
        .map(|protocol| GrantV1 {
            grant_id: format!("protocol-grant-{}", protocol.path()),
            generation: 1,
            bearer_token_sha256: token_sha256(&format!("protocol-token-{}", protocol.path())),
            protocol,
            routes: [(
                "wire-protocol".into(),
                hiroute_gateway::server::publication::ModelRouteV2::Plan {
                    plan_id: "legacy/wire-protocol".into(),
                    alias: "wire-protocol".into(),
                    revision: 89,
                    semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"wire-protocol"),
                },
            )]
            .into(),
        })
        .collect(),
    )
    .unwrap()
}

fn serve_provider_responses(
    listener: TestTlsListener,
    responses: Vec<(&'static str, &'static [u8])>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let expected = responses.len();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let served = Arc::new(AtomicUsize::new(0));
        let mut handlers = Vec::new();
        // This is a total fixture-thread bound across sequential requests, not
        // the timeout for one Gateway attempt. Keep it proportional so a loaded
        // runner cannot close the provider before a healthy later ingress.
        let deadline = Instant::now() + Duration::from_secs(15 * expected as u64);
        while served.load(Ordering::Relaxed) < expected {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let responses = Arc::clone(&responses);
                    let served = Arc::clone(&served);
                    handlers.push(std::thread::spawn(move || {
                        stream.set_nonblocking(false).unwrap();
                        let Some((mut request, expected_length)) =
                            read_provider_request_head(&mut stream)
                        else {
                            return;
                        };
                        assert!(
                            String::from_utf8_lossy(&request)
                                .to_ascii_lowercase()
                                .contains("authorization: bearer protocol-provider-secret")
                        );
                        let path = String::from_utf8_lossy(&request)
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .expect("Provider request path")
                            .to_owned();
                        let (expected_path, body) = responses
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .pop_front()
                            .expect("Provider received an extra request");
                        assert_eq!(path, expected_path);
                        drain_provider_request(&mut stream, &mut request, expected_length);
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                        stream.finish().unwrap();
                        served.fetch_add(1, Ordering::Relaxed);
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "Provider was never invoked");
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("Provider accept failed: {error}"),
            }
        }
        for handler in handlers {
            handler.join().unwrap();
        }
        assert!(
            responses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
    })
}

fn provider_accept_error_kind(listener: &TestTlsListener) -> std::io::ErrorKind {
    match listener.accept() {
        Err(error) => error.kind(),
        Ok(_) => panic!("unexpected Provider connection"),
    }
}

pub(super) fn read_provider_request_head(stream: &mut TestTlsStream) -> Option<(Vec<u8>, usize)> {
    stream
        // A normal production client can open a pooled upstream socket before
        // it dispatches its first request.  Do not mistake that valid idle
        // interval for a failed Provider invocation and close the socket
        // underneath the real request.
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) if request.is_empty() => {
                return None;
            }
            Ok(0) => panic!("Provider request ended before its headers"),
            Ok(read) => read,
            Err(error)
                if request.is_empty()
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
            {
                return None;
            }
            Err(error) => panic!(
                "Provider request read failed ({error}): {}",
                String::from_utf8_lossy(&request)
            ),
        };
        request.extend_from_slice(&buffer[..read]);
        if let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&request[..split]);
            let content_length = head
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .expect("Provider request Content-Length");
            return Some((request, split + 4 + content_length));
        }
    }
}

pub(super) fn drain_provider_request(
    stream: &mut TestTlsStream,
    request: &mut Vec<u8>,
    expected_length: usize,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buffer = [0_u8; 4096];
    while request.len() < expected_length {
        let read = stream.read(&mut buffer).unwrap_or_else(|error| {
            panic!(
                "Provider request body read failed ({error}): {}",
                String::from_utf8_lossy(request)
            )
        });
        assert_ne!(read, 0, "Provider request body ended early");
        request.extend_from_slice(&buffer[..read]);
    }
}

pub(super) struct WireResponse {
    pub(super) status: u16,
    #[allow(dead_code)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

fn fragmented_request(address: SocketAddr, path: &str, body: &[u8]) -> WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{}: {}protocol-token-{path}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if path == "/v1/responses" { "X-HiRoute-Token" } else { "Authorization" },
        if path == "/v1/responses" { "" } else { "Bearer " },
        body.len()
    )
    .unwrap();
    for (index, chunk) in body.chunks(3).enumerate() {
        stream.write_all(chunk).unwrap();
        stream.flush().unwrap();
        if index < 3 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    read_response(stream)
}

fn single_write_request(address: SocketAddr, path: &str, body: &[u8]) -> WireResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n{}: {}protocol-token-{path}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if path == "/v1/responses" { "X-HiRoute-Token" } else { "Authorization" },
        if path == "/v1/responses" { "" } else { "Bearer " },
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    read_response(stream)
}

fn get_ready(address: SocketAddr) -> std::io::Result<WireResponse> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100))?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(Duration::from_millis(200)))?;
    write!(
        stream,
        "GET /_hiroute/ready HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )?;
    let mut wire = Vec::new();
    stream.take(16 * 1024).read_to_end(&mut wire)?;
    Ok(parse_response(wire))
}

#[test]
fn readiness_requires_http_success_on_the_same_connection() {
    for ready in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 1024);
                let mut byte = [0_u8; 1];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            if ready {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            }
        });
        assert_eq!(
            get_ready(address).is_ok_and(|response| response.status == 200),
            ready
        );
        server.join().unwrap();
    }
}

pub(super) fn read_response(mut stream: TcpStream) -> WireResponse {
    let mut wire = Vec::new();
    if let Err(error) = stream.read_to_end(&mut wire) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::ConnectionReset,
            "response read failed after {} bytes: {error}",
            wire.len()
        );
        assert!(
            wire.windows(4).any(|window| window == b"\r\n\r\n"),
            "connection reset before an HTTP response: {wire:?}"
        );
    }
    parse_response(wire)
}

pub(super) fn parse_response(wire: Vec<u8>) -> WireResponse {
    let Some(split) = wire.windows(4).position(|window| window == b"\r\n\r\n") else {
        return WireResponse {
            status: 0,
            headers: BTreeMap::new(),
            body: wire,
        };
    };
    let mut lines = std::str::from_utf8(&wire[..split]).unwrap().split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: BTreeMap<String, String> = lines
        .map(|line| line.split_once(':').unwrap())
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().into()))
        .collect();
    let body = if headers
        .get("transfer-encoding")
        .is_some_and(|value: &String| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunked(&wire[split + 4..])
    } else {
        wire[split + 4..].to_vec()
    };
    WireResponse {
        status,
        headers,
        body,
    }
}

fn decode_chunked(wire: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut offset = 0;
    loop {
        let line_end = wire[offset..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|relative| offset + relative)
            .expect("chunk size line");
        let size = std::str::from_utf8(&wire[offset..line_end])
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim();
        let size = usize::from_str_radix(size, 16).unwrap();
        offset = line_end + 2;
        if size == 0 {
            break;
        }
        output.extend_from_slice(&wire[offset..offset + size]);
        offset += size;
        assert_eq!(&wire[offset..offset + 2], b"\r\n");
        offset += 2;
    }
    output
}

pub(super) struct Hirouted {
    child: Option<Child>,
    address: SocketAddr,
    stderr_path: PathBuf,
}

struct HiroutedSpawnOptions<'a> {
    binary: &'a Path,
    address: SocketAddr,
    lkg: &'a Path,
    publication: &'a Path,
    credentials: &'a Path,
    directory: &'a Path,
    environment: &'a [(&'a str, &'a str)],
}

impl Hirouted {
    pub(super) fn spawn(
        binary: &Path,
        address: SocketAddr,
        lkg: &Path,
        publication: &Path,
        credentials: &Path,
        directory: &Path,
    ) -> Self {
        Self::spawn_with_options(HiroutedSpawnOptions {
            binary,
            address,
            lkg,
            publication,
            credentials,
            directory,
            environment: &[],
        })
    }

    pub(super) fn spawn_with_environment(
        binary: &Path,
        address: SocketAddr,
        lkg: &Path,
        publication: &Path,
        credentials: &Path,
        directory: &Path,
        environment: &[(&str, &str)],
    ) -> Self {
        Self::spawn_with_options(HiroutedSpawnOptions {
            binary,
            address,
            lkg,
            publication,
            credentials,
            directory,
            environment,
        })
    }

    fn spawn_with_options(options: HiroutedSpawnOptions<'_>) -> Self {
        let HiroutedSpawnOptions {
            binary,
            address,
            lkg,
            publication,
            credentials,
            directory,
            environment,
        } = options;
        let stdout = File::create(directory.join("hirouted-protocol.stdout")).unwrap();
        let stderr_path = directory.join("hirouted-protocol.stderr");
        let stderr = File::create(&stderr_path).unwrap();
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("TMPDIR", directory)
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env(E2E_DIAL_CONFIG_ENV, directory.join(E2E_DIAL_CONFIG_FILE));
        for (name, value) in environment {
            command.env(name, value);
        }
        command
            .args(["--listen", &address.to_string(), "--lkg"])
            .arg(lkg)
            .arg("--publication")
            .arg(publication)
            .arg("--credentials")
            .arg(credentials);
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

    pub(super) fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "hirouted exited before readiness"
            );
            // One actual readiness request. A preliminary TCP connection cannot prove
            // the following connection succeeds while the listener is starting.
            if get_ready(self.address).is_ok_and(|response| response.status == 200) {
                return;
            }
            assert!(Instant::now() < deadline, "hirouted readiness timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }

    pub(super) fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }
}

impl Drop for Hirouted {
    fn drop(&mut self) {
        self.stop();
        if std::thread::panicking()
            && let Ok(stderr) = std::fs::read_to_string(&self.stderr_path)
            && !stderr.is_empty()
        {
            eprintln!("hirouted stderr:\n{stderr}");
        }
    }
}

pub(super) fn exact_hirouted_binary() -> PathBuf {
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
            target.join("debug/hirouted")
        })
        .clone()
}

pub(super) fn reserve_address() -> SocketAddr {
    // Every integration-test executable has its own process-local mutex. A
    // kernel-assigned ephemeral port is therefore still racy across test
    // executables: another live `hirouted` can claim it after this listener is
    // dropped, make readiness look green, and leave the fixture talking to the
    // wrong process. Allocate disjoint PID-scoped test ranges instead.
    const FIRST_PORT: u16 = 20_000;
    const PORTS_PER_PROCESS: usize = 100;
    const PROCESS_BUCKETS: usize = 250;
    let process_bucket = usize::try_from(std::process::id()).unwrap() % PROCESS_BUCKETS;
    let start = usize::from(FIRST_PORT) + process_bucket * PORTS_PER_PROCESS;
    for _ in 0..PORTS_PER_PROCESS {
        let slot = NEXT_RESERVED_PORT.fetch_add(1, Ordering::Relaxed) % PORTS_PER_PROCESS;
        let port = u16::try_from(start + slot).unwrap();
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener.local_addr().unwrap();
        }
    }
    panic!("no free PID-scoped loopback port for protocol fixture");
}
