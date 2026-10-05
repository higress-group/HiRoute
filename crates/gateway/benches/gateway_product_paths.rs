#![forbid(unsafe_code)]

#[path = "gateway_product_paths/resource_metrics.rs"]
mod resource_metrics;

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hiroute_gateway::server::publication::{
    AliasPlanV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;
use hiroute_gateway::server::test_control::{
    E2E_DIAL_CONFIG_ENV, E2E_DIAL_CONFIG_FILE, TestTlsListener, TestTlsStream,
    sealed_native_candidate, write_dial_config,
};
use resource_metrics::{replay_backing_bytes, require_linux_procfs, resident_kib};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
const MODEL: &str = "gateway-benchmark-model";
const TOKEN: &str = "gateway-benchmark-token";
const RECORD_BYTES: usize = 16 * 1024;
const MIN_ROUNDS: usize = 5;
// Leave room for both ingress and provider JSON envelopes under the 1 MiB
// request-body limit while still exercising a large disk spill.
const NEAR_LIMIT_INPUT_BYTES: usize = 700 * 1024;
const STABILITY_PAYLOAD_BYTES: usize = NEAR_LIMIT_INPUT_BYTES;
const STABILITY_RSS_DELTA_CEILING_KIB: u64 = 64 * 1024;
const STABILITY_REPLAY_CEILING_BYTES: u64 = 8 * 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args()
        .skip(1)
        .filter(|argument| argument != "--bench")
        .collect::<Vec<_>>();
    if arguments.is_empty() {
        return run_fixed_runner_benchmark();
    }
    if arguments.len() == 2 && arguments[0] == "--stability" {
        return run_stability_entrypoint(&arguments[1]);
    }
    Err("usage: gateway_product_paths [--stability sixty-second|ten-minute]".into())
}

fn run_fixed_runner_benchmark() -> Result<(), Box<dyn Error>> {
    let binary = required_binary("HIROUTE_BENCH_HIROUTED_BIN")?;
    let rounds = env_usize("HIROUTE_GATEWAY_PRODUCT_BENCH_ROUNDS", MIN_ROUNDS).max(MIN_ROUNDS);
    let iterations = env_usize("HIROUTE_GATEWAY_PRODUCT_BENCH_ITERATIONS", 5).max(1);
    let payload_bytes = env_usize(
        "HIROUTE_GATEWAY_PRODUCT_BENCH_BYTES",
        NEAR_LIMIT_INPUT_BYTES,
    )
    .max(RECORD_BYTES);
    let binary_sha256 = sha256_file(&binary)?;

    println!(
        "resource\tvariant\tround\titerations\tthroughput_ops_s\tttft_p50_ns\tttft_p99_ns\tp50_ns\tp99_ns\tvariance_ns2\tmean_ci95_ns\tdetail"
    );
    println!("release\ttrue");
    println!("rounds\t{rounds}");
    for (variant, replay_threshold) in [
        (
            "memory-retained",
            payload_bytes.saturating_mul(2).saturating_add(RECORD_BYTES),
        ),
        ("disk-spill", RECORD_BYTES),
    ] {
        for round in 1..=rounds {
            let measurement = measure_product_round(
                &binary,
                &binary_sha256,
                variant,
                replay_threshold,
                iterations,
                payload_bytes,
                round,
            )?;
            println!(
                "gateway_product\t{variant}\t{round}\t{iterations}\t{:.6}\t{}\t{}\t{}\t{}\t{}\t{}\treplay_threshold_bytes={};copy_bytes={};scan_bytes={};peak_rss_delta_kib={};peak_replay_bytes={};terminal_replay_bytes={};hirouted_sha256={}",
                measurement.throughput_ops_s,
                measurement.ttft_p50_ns,
                measurement.ttft_p99_ns,
                measurement.p50_ns,
                measurement.p99_ns,
                measurement.variance_ns2,
                measurement.mean_ci95_ns,
                replay_threshold,
                measurement.copy_bytes,
                measurement.scan_bytes,
                measurement.peak_rss_delta_kib,
                measurement.peak_replay_bytes,
                measurement.terminal_replay_bytes,
                binary_sha256,
            );
        }
    }
    Ok(())
}

fn run_stability_entrypoint(scenario: &str) -> Result<(), Box<dyn Error>> {
    let revision = revision();
    let report = match run_stability(scenario, &revision) {
        Ok(report) => report,
        Err(error) => StabilityReport::failure(scenario, revision, error.to_string()),
    };
    println!("{}", serde_json::to_string(&report)?);
    if report.semantic_status == "green" {
        Ok(())
    } else {
        Err("production stability semantic checks failed".into())
    }
}

fn run_stability(scenario: &str, revision: &str) -> Result<StabilityReport, Box<dyn Error>> {
    let duration_seconds = match scenario {
        "sixty-second" => 60_u64,
        "ten-minute" => 600_u64,
        _ => return Err(format!("unknown stability scenario: {scenario}").into()),
    };
    require_linux_procfs()?;
    let binary = required_binary("HIROUTE_STABILITY_HIROUTED_BIN")?;
    let binary_sha256 = sha256_file(&binary)?;
    let payload = request_body(STABILITY_PAYLOAD_BYTES, true)?;
    let fixture = Fixture::launch(
        &binary,
        &binary_sha256,
        vec![
            ProviderReply::Complete,
            ProviderReply::SlowSse {
                duration: Duration::from_secs(duration_seconds),
                interval: Duration::from_millis(50),
                delta_bytes: 1024,
            },
        ],
        RECORD_BYTES,
        Duration::from_secs(duration_seconds.saturating_add(180)),
        format!("stability-{scenario}"),
    )?;

    let warmup = request_response(fixture.address, &request_body(1024, false)?, None)?;
    if warmup.status != 200
        || !warmup
            .body
            .windows(b"output_text".len())
            .any(|part| part == b"output_text")
    {
        return Err("warmup did not traverse the production gateway/provider path".into());
    }
    let baseline_rss_kib = resident_kib(fixture.process.id())?;
    let (response, mut observed) = monitor_request(
        fixture.address,
        payload.clone(),
        Some(Duration::from_millis(30)),
        fixture.process.id(),
        &fixture.replay_root,
        Duration::from_millis(25),
    )?;
    observed.peak_rss_kib = observed.peak_rss_kib.max(baseline_rss_kib);
    if response.status != 200 {
        return Err(format!("long-stream response status was {}", response.status).into());
    }
    if !response
        .body
        .windows(b"response.completed".len())
        .any(|part| part == b"response.completed")
    {
        return Err("long stream did not carry a response.completed event".into());
    }
    let terminal_replay_bytes =
        wait_for_empty_replay_root(fixture.process.id(), &fixture.replay_root)?;
    let provider = fixture.finish_provider()?;
    if provider.request_count != 2 {
        return Err(format!(
            "provider observed {} requests, expected 2",
            provider.request_count
        )
        .into());
    }
    if provider.stream_seconds < duration_seconds {
        return Err(format!(
            "upstream stream lasted {} seconds, expected at least {duration_seconds}",
            provider.stream_seconds
        )
        .into());
    }
    let peak_rss_delta_kib = observed.peak_rss_kib.saturating_sub(baseline_rss_kib);
    let errors = stability_errors(
        peak_rss_delta_kib,
        observed.peak_replay_bytes,
        terminal_replay_bytes,
    );
    Ok(StabilityReport {
        schema_version: "hiroute.p0-gateway-production-stability/v2",
        scenario: scenario.into(),
        expected_duration_seconds: duration_seconds,
        revision: revision.into(),
        hirouted: HiroutedEvidence {
            path: binary.display().to_string(),
            sha256: binary_sha256,
            pid: fixture.process.id(),
            product_subprocess: true,
        },
        provider_requests: provider.request_count,
        response_status: response.status,
        long_stream_seconds: provider.stream_seconds,
        rss: ResourceEvidence {
            baseline_kib: baseline_rss_kib,
            peak_kib: observed.peak_rss_kib,
            delta_kib: peak_rss_delta_kib,
            ceiling_kib: STABILITY_RSS_DELTA_CEILING_KIB,
        },
        replay: ReplayEvidence {
            peak_bytes: observed.peak_replay_bytes,
            ceiling_bytes: STABILITY_REPLAY_CEILING_BYTES,
            terminal_bytes: terminal_replay_bytes,
        },
        semantic_status: if errors.is_empty() { "green" } else { "red" }.into(),
        errors,
    })
}

fn stability_errors(
    rss_delta_kib: u64,
    replay_peak_bytes: u64,
    replay_terminal_bytes: u64,
) -> Vec<String> {
    let mut errors = Vec::new();
    if rss_delta_kib > STABILITY_RSS_DELTA_CEILING_KIB {
        errors.push(format!(
            "RSS delta {rss_delta_kib} KiB exceeds {} KiB",
            STABILITY_RSS_DELTA_CEILING_KIB
        ));
    }
    if replay_peak_bytes > STABILITY_REPLAY_CEILING_BYTES {
        errors.push(format!(
            "replay backing {replay_peak_bytes} bytes exceeds {} bytes",
            STABILITY_REPLAY_CEILING_BYTES
        ));
    }
    if replay_peak_bytes == 0 {
        errors.push("long stream did not observe disk-spill replay backing".into());
    }
    if replay_terminal_bytes != 0 {
        errors.push(format!(
            "replay backing retained {replay_terminal_bytes} bytes after stream terminal cleanup"
        ));
    }
    errors
}

fn measure_product_round(
    binary: &Path,
    binary_sha256: &str,
    variant: &str,
    replay_threshold: usize,
    iterations: usize,
    payload_bytes: usize,
    round: usize,
) -> Result<ProductMeasurement, Box<dyn Error>> {
    let fixture = Fixture::launch(
        binary,
        binary_sha256,
        vec![ProviderReply::DelayedComplete(Duration::from_millis(15)); iterations],
        replay_threshold,
        Duration::from_secs(60),
        format!("product-{variant}-{round}"),
    )?;
    let request = request_body(payload_bytes, false)?;
    let baseline_rss_kib = resident_kib(fixture.process.id()).unwrap_or(0);
    let started = Instant::now();
    let mut latency_samples = Vec::with_capacity(iterations);
    let mut ttft_samples = Vec::with_capacity(iterations);
    let mut peak_rss_kib = baseline_rss_kib;
    let mut peak_replay_bytes = 0_u64;
    for _ in 0..iterations {
        let (response, observed) = monitor_request(
            fixture.address,
            request.clone(),
            None,
            fixture.process.id(),
            &fixture.replay_root,
            Duration::from_millis(2),
        )?;
        if response.status != 200
            || !response
                .body
                .windows(b"output_text".len())
                .any(|part| part == b"output_text")
        {
            let provider_diagnostic = fixture.provider_diagnostic();
            return Err(format!(
                "production benchmark request did not receive a native provider response: status={}, body={}, {provider_diagnostic}",
                response.status,
                String::from_utf8_lossy(&response.body)
            )
            .into());
        }
        peak_rss_kib = peak_rss_kib.max(observed.peak_rss_kib);
        peak_replay_bytes = peak_replay_bytes.max(observed.peak_replay_bytes);
        latency_samples.push(response.elapsed.as_nanos());
        ttft_samples.push(response.ttft.as_nanos());
        if wait_for_empty_replay_root(fixture.process.id(), &fixture.replay_root)? != 0 {
            return Err("benchmark request retained replay backing after terminal cleanup".into());
        }
    }
    let provider = fixture.finish_provider()?;
    if provider.request_count != iterations {
        return Err(format!(
            "product benchmark provider observed {} requests, expected {iterations}",
            provider.request_count
        )
        .into());
    }
    if variant == "disk-spill" && peak_replay_bytes == 0 {
        return Err("disk-spill product benchmark did not observe replay backing".into());
    }
    if variant == "memory-retained" && peak_replay_bytes != 0 {
        return Err("memory-retained product benchmark unexpectedly wrote replay backing".into());
    }
    let elapsed = started.elapsed();
    let mean = latency_samples.iter().copied().sum::<u128>() as f64 / iterations as f64;
    let variance = latency_samples
        .iter()
        .map(|sample| (*sample as f64 - mean).powi(2))
        .sum::<f64>()
        / iterations as f64;
    Ok(ProductMeasurement {
        throughput_ops_s: iterations as f64 / elapsed.as_secs_f64(),
        ttft_p50_ns: percentile(&mut ttft_samples, 0.50),
        ttft_p99_ns: percentile(&mut ttft_samples, 0.99),
        p50_ns: percentile(&mut latency_samples, 0.50),
        p99_ns: percentile(&mut latency_samples, 0.99),
        variance_ns2: variance,
        mean_ci95_ns: 1.96 * (variance / iterations as f64).sqrt(),
        copy_bytes: provider.request_body_bytes,
        scan_bytes: request.len() as u64 * iterations as u64,
        peak_rss_delta_kib: peak_rss_kib.saturating_sub(baseline_rss_kib),
        peak_replay_bytes,
        terminal_replay_bytes: 0,
    })
}

fn percentile(samples: &mut [u128], percentile: f64) -> u128 {
    samples.sort_unstable();
    let index = ((samples.len().saturating_sub(1)) as f64 * percentile).ceil() as usize;
    samples[index]
}

fn monitor_request(
    address: SocketAddr,
    body: Vec<u8>,
    slow_read_delay: Option<Duration>,
    pid: u32,
    replay_root: &Path,
    interval: Duration,
) -> Result<(TimedResponse, ObservedResources), Box<dyn Error>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let client = thread::spawn(move || {
        let result =
            request_response(address, &body, slow_read_delay).map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    let mut observed = ObservedResources::default();
    let response = loop {
        match receiver.recv_timeout(interval) {
            Ok(result) => break result.map_err(|error| -> Box<dyn Error> { error.into() })?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                observed.observe(pid, replay_root)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(
                    "production client thread disconnected before returning a response".into(),
                );
            }
        }
    };
    client
        .join()
        .map_err(|_| "production client thread panicked")?;
    observed.observe(pid, replay_root)?;
    Ok((response, observed))
}

fn request_response(
    address: SocketAddr,
    body: &[u8],
    slow_read_delay: Option<Duration>,
) -> Result<TimedResponse, Box<dyn Error>> {
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(900)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    write!(
        stream,
        "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nX-HiRoute-Token: {TOKEN}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .map_err(|error| format!("gateway request header write failed: {error}"))?;
    stream
        .write_all(body)
        .map_err(|error| format!("gateway request body write failed: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("gateway request flush failed: {error}"))?;
    let mut wire = Vec::new();
    let mut buffer = [0_u8; 1024];
    let mut header_end = None;
    let mut ttft = None;
    let read_error = loop {
        match stream.read(&mut buffer) {
            Ok(0) => break None,
            Ok(read) => {
                wire.extend_from_slice(&buffer[..read]);
                if header_end.is_none() {
                    header_end = wire.windows(4).position(|window| window == b"\r\n\r\n");
                    if header_end.is_some() {
                        ttft = Some(started.elapsed());
                    }
                }
                if let Some(delay) = slow_read_delay {
                    thread::sleep(delay);
                }
            }
            Err(error) => break Some(error),
        }
    };
    let split = header_end.ok_or("gateway response did not include HTTP headers")?;
    let status = std::str::from_utf8(&wire[..split])?
        .split("\r\n")
        .next()
        .and_then(|status| status.split_whitespace().nth(1))
        .ok_or("gateway response status was malformed")?
        .parse::<u16>()?;
    if let Some(error) = read_error {
        let complete = response_is_complete(&wire, split, status);
        if error.kind() != std::io::ErrorKind::ConnectionReset || !complete {
            return Err(format!(
                "gateway response read failed after {} bytes with status {status} (complete={complete}): {error}",
                wire.len()
            )
            .into());
        }
    }
    Ok(TimedResponse {
        status,
        body: wire[split + 4..].to_vec(),
        ttft: ttft.ok_or("gateway response never produced a first byte")?,
        elapsed: started.elapsed(),
    })
}

struct Fixture {
    root: PathBuf,
    replay_root: PathBuf,
    provider: Option<MockProvider>,
    process: Hirouted,
    address: SocketAddr,
}

impl Fixture {
    fn launch(
        binary: &Path,
        binary_sha256: &str,
        replies: Vec<ProviderReply>,
        replay_threshold: usize,
        overall_timeout: Duration,
        label: String,
    ) -> Result<Self, Box<dyn Error>> {
        let root = fresh_directory(&label)?;
        let provider = MockProvider::start(replies)?;
        write_dial_config(&root, &[provider.listener()])?;
        let replay_root = root.join("replay");
        fs::create_dir(&replay_root)?;
        set_private_permissions(&replay_root)?;
        let publication_path = root.join("publication.json");
        let credentials_path = root.join("credentials.json");
        let lkg_path = root.join("publication-lkg.json");
        let publication = GatewayPublicationSnapshotV3::seal(
            "personal/default",
            "gateway-benchmark-authority",
            1,
            1,
            "gateway-benchmark-renderer/v1",
            vec![AliasPlanV1 {
                served_model_id: MODEL.into(),
                purpose: "production process benchmark".into(),
                agent_plan_revision: 1,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: overall_timeout.as_millis() as u64,
                max_attempts: 1,
                routing: None,
                candidates: vec![sealed_native_candidate(
                    1,
                    "gateway-benchmark-target",
                    &["gateway-benchmark-credential".into()],
                    provider.authority(),
                    "gateway-native",
                    &[(IngressProtocol::Responses, IngressProtocol::Responses)],
                )],
            }],
            vec![GrantV1 {
                route_protocols: Default::default(),
                grant_id: "gateway-benchmark-grant".into(),
                generation: 1,
                bearer_token_sha256: token_sha256(TOKEN),
                protocol: IngressProtocol::Responses,
                routes: [(
                    MODEL.into(),
                    hiroute_gateway::server::publication::ModelRouteV2::Plan {
                        plan_id: format!("legacy/{MODEL}"),
                        alias: MODEL.into(),
                        revision: 1,
                        semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(
                            MODEL.as_bytes(),
                        ),
                    },
                )]
                .into(),
            }],
        )?;
        fs::write(&publication_path, serde_json::to_vec_pretty(&publication)?)?;
        fs::write(
            root.join("gateway-benchmark-credential.json"),
            serde_json::to_vec_pretty(&json!({
                "schema_version": "hiroute.gateway.credential-leases/v1",
                "credential_ref": "gateway-benchmark-credential",
                "keys": [{
                    "key_id": "gateway-benchmark-key",
                    "generation": 1,
                    "authorization": "Bearer gateway-benchmark-provider-secret"
                }]
            }))?,
        )?;
        fs::write(
            &credentials_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "hiroute.gateway.credentials/v1",
                "credentials": {
                    "gateway-benchmark-credential": "gateway-benchmark-credential.json"
                }
            }))?,
        )?;
        let address = reserve_address()?;
        let mut process = Hirouted::spawn(HiroutedSpawn {
            binary,
            binary_sha256,
            address,
            lkg: &lkg_path,
            publication: &publication_path,
            credentials: &credentials_path,
            root: &root,
            replay_root: &replay_root,
            replay_threshold,
        })?;
        process.wait_ready()?;
        Ok(Self {
            root,
            replay_root,
            provider: Some(provider),
            process,
            address,
        })
    }

    fn finish_provider(&self) -> Result<ProviderStats, Box<dyn Error>> {
        self.provider
            .as_ref()
            .ok_or("provider was already joined")?
            .finish()
    }

    fn provider_diagnostic(&self) -> String {
        self.provider
            .as_ref()
            .map_or_else(|| "provider unavailable".into(), MockProvider::diagnostic)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.process.stop();
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Hirouted {
    child: Option<Child>,
    address: SocketAddr,
}

struct HiroutedSpawn<'a> {
    binary: &'a Path,
    binary_sha256: &'a str,
    address: SocketAddr,
    lkg: &'a Path,
    publication: &'a Path,
    credentials: &'a Path,
    root: &'a Path,
    replay_root: &'a Path,
    replay_threshold: usize,
}

impl Hirouted {
    fn spawn(options: HiroutedSpawn<'_>) -> Result<Self, Box<dyn Error>> {
        let stdout = File::create(options.root.join("hirouted.stdout"))?;
        let stderr = File::create(options.root.join("hirouted.stderr"))?;
        let child = Command::new(options.binary)
            .env_clear()
            .env("TMPDIR", options.root)
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env(
                "HIROUTE_LAUNCHER_EXECUTABLE_SHA256",
                format!("sha256:{}", options.binary_sha256),
            )
            .env(E2E_DIAL_CONFIG_ENV, options.root.join(E2E_DIAL_CONFIG_FILE))
            .env("HIROUTE_REPLAY_ROOT", options.replay_root)
            .env(
                "HIROUTE_REPLAY_MEMORY_THRESHOLD",
                options.replay_threshold.to_string(),
            )
            .env("HIROUTE_REPLAY_RECORD_BYTES", RECORD_BYTES.to_string())
            .env("HIROUTE_REPLAY_ORPHAN_TTL_MS", "1000")
            .args(["--listen", &options.address.to_string(), "--lkg"])
            .arg(options.lkg)
            .arg("--publication")
            .arg(options.publication)
            .arg("--credentials")
            .arg(options.credentials)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?;
        Ok(Self {
            child: Some(child),
            address: options.address,
        })
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("hirouted must be live").id()
    }

    fn wait_ready(&mut self) -> Result<(), Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(response) = ready_response(self.address)
                && response.status == 200
            {
                return Ok(());
            }
            if self
                .child
                .as_mut()
                .ok_or("hirouted child was unavailable")?
                .try_wait()?
                .is_some()
            {
                return Err("hirouted exited before readiness".into());
            }
            if Instant::now() >= deadline {
                return Err("hirouted readiness timed out".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Hirouted {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ready_response(address: SocketAddr) -> Result<TimedResponse, Box<dyn Error>> {
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100))?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    write!(
        stream,
        "GET /_hiroute/ready HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )?;
    stream.flush()?;
    let mut wire = Vec::new();
    let read_error = stream.read_to_end(&mut wire).err();
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("ready response did not include headers")?;
    let status = std::str::from_utf8(&wire[..split])?
        .split("\r\n")
        .next()
        .and_then(|status| status.split_whitespace().nth(1))
        .ok_or("ready response status was malformed")?
        .parse::<u16>()?;
    if let Some(error) = read_error
        && (error.kind() != std::io::ErrorKind::ConnectionReset
            || !response_is_complete(&wire, split, status))
    {
        return Err(error.into());
    }
    Ok(TimedResponse {
        status,
        body: wire[split + 4..].to_vec(),
        ttft: started.elapsed(),
        elapsed: started.elapsed(),
    })
}

fn response_is_complete(wire: &[u8], split: usize, status: u16) -> bool {
    let Ok(head) = std::str::from_utf8(&wire[..split]) else {
        return false;
    };
    let body = &wire[split + 4..];
    if matches!(status, 204 | 304) {
        return body.is_empty();
    }
    for line in head.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .is_ok_and(|declared| declared == body.len());
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|coding| coding.trim().eq_ignore_ascii_case("chunked"))
        {
            return body.ends_with(b"0\r\n\r\n");
        }
    }
    false
}

#[derive(Clone)]
enum ProviderReply {
    Complete,
    DelayedComplete(Duration),
    SlowSse {
        duration: Duration,
        interval: Duration,
        delta_bytes: usize,
    },
}

struct MockProvider {
    listener: TestTlsListener,
    join: std::sync::Mutex<Option<thread::JoinHandle<Result<ProviderStats, String>>>>,
}

impl MockProvider {
    fn start(replies: Vec<ProviderReply>) -> Result<Self, Box<dyn Error>> {
        let listener = TestTlsListener::bind("gateway-benchmark.invalid")?;
        listener.set_nonblocking(true)?;
        let join_listener = listener.try_clone()?;
        let join = thread::spawn(move || serve_provider(join_listener, replies));
        Ok(Self {
            listener,
            join: std::sync::Mutex::new(Some(join)),
        })
    }

    fn authority(&self) -> &str {
        self.listener.authority()
    }

    fn listener(&self) -> &TestTlsListener {
        &self.listener
    }

    fn finish(&self) -> Result<ProviderStats, Box<dyn Error>> {
        let join = self
            .join
            .lock()
            .map_err(|_| "provider join lock poisoned")?
            .take()
            .ok_or("provider was already joined")?;
        join.join()
            .map_err(|_| "provider thread panicked")?
            .map_err(|error| error.into())
    }

    fn diagnostic(&self) -> String {
        let Ok(mut join) = self.join.lock() else {
            return "provider join lock poisoned".into();
        };
        let Some(handle) = join.as_ref() else {
            return "provider was already joined".into();
        };
        if !handle.is_finished() {
            return "provider thread remained active".into();
        }
        match join.take().expect("checked provider handle").join() {
            Ok(Ok(stats)) => format!(
                "provider completed unexpectedly after {} requests and {} body bytes",
                stats.request_count, stats.request_body_bytes
            ),
            Ok(Err(error)) => format!("provider failed: {error}"),
            Err(_) => "provider thread panicked".into(),
        }
    }
}

fn serve_provider(
    listener: TestTlsListener,
    replies: Vec<ProviderReply>,
) -> Result<ProviderStats, String> {
    let mut stats = ProviderStats::default();
    for reply in replies {
        let mut stream = accept_provider_stream(&listener)?;
        let request = read_http_request(&mut stream)?;
        if !request
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer gateway-benchmark-provider-secret")
        {
            return Err("provider request did not carry the exact selected credential".into());
        }
        stats.request_count += 1;
        stats.request_body_bytes = stats
            .request_body_bytes
            .checked_add(request.body.len() as u64)
            .ok_or("provider request byte accounting overflow")?;
        match reply {
            ProviderReply::Complete => write_complete(&mut stream)?,
            ProviderReply::DelayedComplete(delay) => {
                thread::sleep(delay);
                write_complete(&mut stream)?;
            }
            ProviderReply::SlowSse {
                duration,
                interval,
                delta_bytes,
            } => {
                stats.stream_seconds =
                    write_slow_sse(&mut stream, duration, interval, delta_bytes)?;
            }
        }
    }
    Ok(stats)
}

fn accept_provider_stream(listener: &TestTlsListener) -> Result<TestTlsStream, String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(60)))
                    .map_err(|error| error.to_string())?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("provider was never invoked by hirouted".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(format!("provider accept failed: {error}")),
        }
    }
}

struct ProviderRequest {
    headers: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TestTlsStream) -> Result<ProviderRequest, String> {
    let mut wire = Vec::new();
    let mut buffer = [0_u8; 8192];
    let (head_end, content_length, chunked) = loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("provider request closed before headers".into());
        }
        wire.extend_from_slice(&buffer[..read]);
        if let Some(split) = wire.windows(4).position(|window| window == b"\r\n\r\n") {
            let header_text =
                std::str::from_utf8(&wire[..split]).map_err(|error| error.to_string())?;
            let expect_continue = header_text.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("expect")
                        && value.trim().eq_ignore_ascii_case("100-continue")
                })
            });
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>())
                    })
                })
                .transpose()
                .map_err(|_| "provider request included an invalid content-length")?;
            let chunked = header_text.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("transfer-encoding")
                        && value
                            .split(',')
                            .any(|coding| coding.trim().eq_ignore_ascii_case("chunked"))
                })
            });
            if content_length.is_some() == chunked {
                return Err("provider request body framing was missing or ambiguous".into());
            }
            if expect_continue {
                stream
                    .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                    .map_err(|error| error.to_string())?;
                stream.flush().map_err(|error| error.to_string())?;
            }
            break (split + 4, content_length, chunked);
        }
    };
    loop {
        let complete = if chunked {
            decode_chunked_request(&wire[head_end..])?.is_some()
        } else {
            wire.len().saturating_sub(head_end)
                >= content_length.expect("validated content-length framing")
        };
        if complete {
            break;
        }
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("provider request closed before its declared body length".into());
        }
        wire.extend_from_slice(&buffer[..read]);
    }
    let body = if chunked {
        decode_chunked_request(&wire[head_end..])?
            .ok_or("provider chunked request remained incomplete")?
    } else {
        let content_length = content_length.expect("validated content-length framing");
        wire[head_end..head_end + content_length].to_vec()
    };
    Ok(ProviderRequest {
        headers: String::from_utf8_lossy(&wire[..head_end]).into_owned(),
        body,
    })
}

fn decode_chunked_request(wire: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let Some(relative_end) = wire[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
        else {
            return Ok(None);
        };
        let size_end = cursor + relative_end;
        let size_text = std::str::from_utf8(&wire[cursor..size_end])
            .map_err(|_| "provider chunk size is not UTF-8")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or_default(), 16)
            .map_err(|_| "provider chunk size is invalid")?;
        cursor = size_end + 2;
        if size == 0 {
            if wire.len() < cursor + 2 {
                return Ok(None);
            }
            if wire.get(cursor..cursor + 2) != Some(b"\r\n") {
                return Err("provider terminal chunk has no terminator".into());
            }
            return Ok(Some(decoded));
        }
        if wire.len() < cursor + size + 2 {
            return Ok(None);
        }
        decoded.extend_from_slice(&wire[cursor..cursor + size]);
        cursor += size;
        if wire.get(cursor..cursor + 2) != Some(b"\r\n") {
            return Err("provider chunk has no terminator".into());
        }
        cursor += 2;
    }
}

fn write_complete(stream: &mut TestTlsStream) -> Result<(), String> {
    let body = br#"{"id":"gateway-benchmark-provider","model":"gateway-native","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())
}

fn write_slow_sse(
    stream: &mut TestTlsStream,
    duration: Duration,
    interval: Duration,
    delta_bytes: usize,
) -> Result<u64, String> {
    let events = (duration.as_millis() / interval.as_millis()).max(1) as usize;
    let delta = "x".repeat(delta_bytes);
    let created = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"gateway-stability\",\"model\":\"gateway-native\"}}\n\n";
    let completed = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";
    let one_delta = format!(
        "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"{delta}\"}}\n\n"
    );
    let content_length = created
        .len()
        .checked_add(one_delta.len().saturating_mul(events))
        .and_then(|bytes| bytes.checked_add(completed.len()))
        .ok_or("stability SSE content length overflow")?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| error.to_string())?;
    stream
        .write_all(created)
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let started = Instant::now();
    for _ in 0..events {
        stream
            .write_all(one_delta.as_bytes())
            .map_err(|error| error.to_string())?;
        stream.flush().map_err(|error| error.to_string())?;
        thread::sleep(interval);
    }
    stream
        .write_all(completed)
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    Ok(started.elapsed().as_secs())
}

#[derive(Default)]
struct ProviderStats {
    request_count: usize,
    request_body_bytes: u64,
    stream_seconds: u64,
}

struct TimedResponse {
    status: u16,
    body: Vec<u8>,
    ttft: Duration,
    elapsed: Duration,
}

#[derive(Default)]
struct ObservedResources {
    peak_rss_kib: u64,
    peak_replay_bytes: u64,
}

impl ObservedResources {
    fn observe(&mut self, pid: u32, replay_root: &Path) -> Result<(), Box<dyn Error>> {
        self.peak_rss_kib = self.peak_rss_kib.max(resident_kib(pid)?);
        self.peak_replay_bytes = self
            .peak_replay_bytes
            .max(replay_backing_bytes(pid, replay_root)?);
        Ok(())
    }
}

fn wait_for_empty_replay_root(pid: u32, root: &Path) -> Result<u64, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let bytes = replay_backing_bytes(pid, root)?;
        if bytes == 0 {
            return Ok(0);
        }
        if Instant::now() >= deadline {
            return Ok(bytes);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn request_body(input_bytes: usize, stream: bool) -> Result<Vec<u8>, Box<dyn Error>> {
    #[derive(Serialize)]
    struct BenchmarkRequest {
        model: &'static str,
        input: String,
        stream: bool,
    }

    Ok(serde_json::to_vec(&BenchmarkRequest {
        model: MODEL,
        input: "x".repeat(input_bytes),
        stream,
    })?)
}

fn required_binary(variable: &str) -> Result<PathBuf, Box<dyn Error>> {
    let binary = env::var_os(variable)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{variable} must name the release hirouted binary"))?;
    if !binary.is_file() {
        return Err(format!("{variable} does not name a file: {}", binary.display()).into());
    }
    Ok(binary)
}

fn sha256_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn revision() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_owned())
        .unwrap_or_else(|| "unavailable".into())
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn reserve_address() -> Result<SocketAddr, Box<dyn Error>> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?)
}

fn fresh_directory(label: &str) -> Result<PathBuf, Box<dyn Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = env::temp_dir().join(format!(
        "hiroute-gateway-production-bench-{}-{}-{nonce}",
        std::process::id(),
        label
    ));
    fs::create_dir(&root)?;
    set_private_permissions(&root)?;
    Ok(root)
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
}

struct ProductMeasurement {
    throughput_ops_s: f64,
    ttft_p50_ns: u128,
    ttft_p99_ns: u128,
    p50_ns: u128,
    p99_ns: u128,
    variance_ns2: f64,
    mean_ci95_ns: f64,
    copy_bytes: u64,
    scan_bytes: u64,
    peak_rss_delta_kib: u64,
    peak_replay_bytes: u64,
    terminal_replay_bytes: u64,
}

#[derive(Serialize)]
struct StabilityReport {
    schema_version: &'static str,
    scenario: String,
    expected_duration_seconds: u64,
    revision: String,
    hirouted: HiroutedEvidence,
    provider_requests: usize,
    response_status: u16,
    long_stream_seconds: u64,
    rss: ResourceEvidence,
    replay: ReplayEvidence,
    semantic_status: String,
    errors: Vec<String>,
}

impl StabilityReport {
    fn failure(scenario: &str, revision: String, error: String) -> Self {
        Self {
            schema_version: "hiroute.p0-gateway-production-stability/v2",
            scenario: scenario.into(),
            expected_duration_seconds: 0,
            revision,
            hirouted: HiroutedEvidence {
                path: String::new(),
                sha256: String::new(),
                pid: 0,
                product_subprocess: false,
            },
            provider_requests: 0,
            response_status: 0,
            long_stream_seconds: 0,
            rss: ResourceEvidence {
                baseline_kib: 0,
                peak_kib: 0,
                delta_kib: 0,
                ceiling_kib: STABILITY_RSS_DELTA_CEILING_KIB,
            },
            replay: ReplayEvidence {
                peak_bytes: 0,
                ceiling_bytes: STABILITY_REPLAY_CEILING_BYTES,
                terminal_bytes: 0,
            },
            semantic_status: "red".into(),
            errors: vec![error],
        }
    }
}

#[derive(Serialize)]
struct HiroutedEvidence {
    path: String,
    sha256: String,
    pid: u32,
    product_subprocess: bool,
}

#[derive(Serialize)]
struct ResourceEvidence {
    baseline_kib: u64,
    peak_kib: u64,
    delta_kib: u64,
    ceiling_kib: u64,
}

#[derive(Serialize)]
struct ReplayEvidence {
    peak_bytes: u64,
    ceiling_bytes: u64,
    terminal_bytes: u64,
}
