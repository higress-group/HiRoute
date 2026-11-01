//! Rejected proxy attempts from the pinned, account-free packaged CPA journey.
//! No upstream connection or TLS tunnel is created. CONNECT identifies an origin,
//! not its encrypted path, request body, OAuth success or model authorization.
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const HEADER_BOUND: usize = 8 * 1024;
const CONNECTION_BOUND: Duration = Duration::from_secs(2);
const ATTEMPT_BOUND: usize = 64;
// Exact dbb706215a3afd2f0ea3c96482d4233856466caa source:
// cmd/server/main.go:833 starts this updater even with --local-model.
// internal/misc/antigravity_version.go:32,215-232 uses this HTTPS origin through
// Go's default transport. This is deliberately an origin comparison, not a claim
// that the manifest path has been observed through an encrypted tunnel.
const UPDATER: &str = "antigravity-hub-auto-updater-974169037036.us-central1.run.app:443";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Phase {
    BeforeManualRestart,
    ManualStart,
    RestartRecovery,
    LoginStart,
    LoginCancel,
    LoginForget,
    RestoreInherited,
    Stopped,
}
impl Phase {
    fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::BeforeManualRestart,
            1 => Self::ManualStart,
            2 => Self::RestartRecovery,
            3 => Self::LoginStart,
            4 => Self::LoginCancel,
            5 => Self::LoginForget,
            6 => Self::RestoreInherited,
            7 => Self::Stopped,
            _ => unreachable!("only closed fixture phases are published"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Method {
    Connect,
    Get,
    Post,
    Other,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Destination {
    StockVersionUpdaterOrigin,
    // Diagnostic distinctions only: the managed config disables this updater,
    // and neither management origin is an approved account-free attempt.
    ManagementReleaseOrigin,
    ManagementFallbackOrigin,
    CodexOAuth,
    ClaudeOAuth,
    CodexModelOrigin,
    ClaudeModelOrigin,
    Other,
}
fn destination(value: &str) -> Destination {
    match value {
        UPDATER => Destination::StockVersionUpdaterOrigin,
        "api.github.com:443" => Destination::ManagementReleaseOrigin,
        "cpamc.router-for.me:443" => Destination::ManagementFallbackOrigin,
        "auth.openai.com:443" => Destination::CodexOAuth,
        "claude.ai:443" | "console.anthropic.com:443" => Destination::ClaudeOAuth,
        "chatgpt.com:443" => Destination::CodexModelOrigin,
        "api.anthropic.com:443" => Destination::ClaudeModelOrigin,
        _ => Destination::Other,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeaderFacts {
    method: Method,
    destination: Destination,
    valid: bool,
    authentication: bool,
    body: bool,
    unknown_header: bool,
}
fn classify(bytes: &[u8]) -> HeaderFacts {
    let mut facts = HeaderFacts {
        method: Method::Other,
        destination: Destination::Other,
        valid: false,
        authentication: false,
        body: false,
        unknown_header: false,
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return facts;
    };
    let Some((head, rest)) = text.split_once("\r\n\r\n") else {
        return facts;
    };
    facts.body = !rest.is_empty();
    let mut lines = head.split("\r\n");
    let fields: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    if fields.len() != 3 || fields[2] != "HTTP/1.1" {
        return facts;
    }
    facts.method = match fields[0] {
        "CONNECT" => Method::Connect,
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => Method::Other,
    };
    facts.destination = destination(fields[1]);
    let mut hosts = 0;
    let mut agents = 0;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            return facts;
        };
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return facts;
        }
        let value = value.trim_matches(' ');
        match key.to_ascii_lowercase().as_str() {
            "host" => {
                hosts += 1;
                if value != fields[1] {
                    return facts;
                }
            }
            "user-agent" => {
                agents += 1;
                if value != "Go-http-client/1.1" {
                    return facts;
                }
            }
            "authorization" | "proxy-authorization" => facts.authentication = true,
            "content-length" => facts.body |= value != "0",
            "transfer-encoding" => facts.body = true,
            _ => facts.unknown_header = true,
        }
    }
    facts.valid = hosts == 1 && agents == 1;
    facts
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadEnd {
    Complete,
    Eof,
    Deadline,
    Limit,
    Io,
}
#[derive(Clone, Debug)]
struct Receipt {
    phase: Phase,
    facts: HeaderFacts,
    header_bytes: usize,
    read_end: ReadEnd,
    io_kind: Option<io::ErrorKind>,
    native_code: Option<i32>,
    rejected_502: bool,
    peer_eof: bool,
    extra_bytes: bool,
}
impl Receipt {
    fn approved(&self) -> bool {
        self.read_end == ReadEnd::Complete
            && self.facts.valid
            && self.facts.method == Method::Connect
            && self.facts.destination == Destination::StockVersionUpdaterOrigin
            && !self.facts.authentication
            && !self.facts.body
            && !self.facts.unknown_header
            && !self.extra_bytes
            && self.io_kind.is_none()
            && self.rejected_502
            && self.peer_eof
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}
fn observe(mut stream: TcpStream, phase: Phase) -> Receipt {
    let deadline = Instant::now() + CONNECTION_BOUND;
    let mut bytes = Zeroizing::new(Vec::new());
    let mut receipt = Receipt {
        phase,
        facts: classify(&[]),
        header_bytes: 0,
        read_end: ReadEnd::Io,
        io_kind: None,
        native_code: None,
        rejected_502: false,
        peer_eof: false,
        extra_bytes: false,
    };
    let io_result = (|| -> io::Result<()> {
        // Normalize the accepted Winsock stream before bounded header/EOF reads;
        // a read timeout does not clear inherited nonblocking mode.
        stream.set_nonblocking(false)?;
        loop {
            stream.set_read_timeout(Some(remaining(deadline)?))?;
            let mut chunk = [0u8; 1024];
            let count = stream.read(&mut chunk)?;
            if count == 0 {
                receipt.read_end = ReadEnd::Eof;
                receipt.peer_eof = true;
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.len() > HEADER_BOUND {
                receipt.read_end = ReadEnd::Limit;
                break;
            }
            if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                receipt.read_end = ReadEnd::Complete;
                break;
            }
        }
        receipt.header_bytes = bytes.len();
        receipt.facts = classify(&bytes);
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        stream.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        receipt.rejected_502 = true;
        stream.shutdown(Shutdown::Write)?;
        while !receipt.peer_eof {
            stream.set_read_timeout(Some(remaining(deadline)?))?;
            let mut extra = [0u8; 1];
            if stream.read(&mut extra)? == 0 {
                receipt.peer_eof = true;
            } else {
                receipt.extra_bytes = true;
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(())
    })();
    if let Err(error) = io_result {
        receipt.header_bytes = bytes.len();
        receipt.facts = classify(&bytes);
        receipt.io_kind = Some(error.kind());
        receipt.native_code = error.raw_os_error();
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            receipt.read_end = ReadEnd::Deadline;
        }
    }
    receipt
}

#[derive(Default)]
struct Ledger {
    receipts: Vec<Receipt>,
    listener_failed: bool,
    attempt_limit_exceeded: bool,
}

pub struct Observation {
    url: String,
    phase: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    ledger: Arc<Mutex<Ledger>>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}
impl Observation {
    pub fn new() -> Self {
        let source: serde_json::Value =
            serde_json::from_str(include_str!("../../../../vendor/cpa/source.json")).unwrap();
        assert!(
            source["repository"] == "https://github.com/router-for-me/CLIProxyAPI"
                && source["commit"] == "dbb706215a3afd2f0ea3c96482d4233856466caa",
            "updater-origin observation must be reviewed for the actual pinned CPA source"
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let phase = Arc::new(AtomicU8::new(Phase::BeforeManualRestart as u8));
        let stop = Arc::new(AtomicBool::new(false));
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let (sender, done) = mpsc::sync_channel(1);
        let (p, s, l) = (phase.clone(), stop.clone(), ledger.clone());
        let worker = thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if l.lock().unwrap().receipts.len() >= ATTEMPT_BOUND {
                            l.lock().unwrap().attempt_limit_exceeded = true;
                            break;
                        }
                        let receipt = observe(stream, Phase::from_byte(p.load(Ordering::Acquire)));
                        eprintln!("packaged CPA rejected proxy attempt: {receipt:?}");
                        let approved = receipt.approved();
                        l.lock().unwrap().receipts.push(receipt);
                        if !approved {
                            // The scenario is already red. Close the listener
                            // rather than retaining queued unknown connections
                            // through shutdown; no rejected attempt is hidden.
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        // Once the exact foreground Job is empty, drain the
                        // listener backlog before declaring the final ledger.
                        if s.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => {
                        eprintln!(
                            "packaged CPA proxy observation listener failed: io-kind={:?}, native-code={:?}",
                            error.kind(),
                            error.raw_os_error()
                        );
                        l.lock().unwrap().listener_failed = true;
                        break;
                    }
                }
            }
            let _ = sender.send(());
        });
        Self {
            url,
            phase,
            stop,
            ledger,
            done,
            thread: Some(worker),
        }
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Release);
        eprintln!("packaged CPA proxy observation checkpoint: {phase:?}");
        self.assert_approved_only();
    }
    pub fn assert_approved_only(&self) {
        let ledger = self.ledger.lock().unwrap();
        assert!(
            !ledger.listener_failed && !ledger.attempt_limit_exceeded,
            "packaged proxy listener failed or exceeded its fixed attempt bound"
        );
        assert!(
            ledger.receipts.iter().all(Receipt::approved),
            "account-free package generated an unapproved or incomplete proxy attempt"
        );
    }
    pub fn require_updater_origin(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.assert_approved_only();
            if !self.ledger.lock().unwrap().receipts.is_empty() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "real stock CPA did not produce the pinned updater-origin proxy attempt"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn finish(mut self) {
        self.phase(Phase::Stopped);
        self.stop.store(true, Ordering::Release);
        self.done
            .recv_timeout(Duration::from_secs(5))
            .expect("bounded proxy observation did not finish");
        self.thread.take().unwrap().join().unwrap();
        self.assert_approved_only();
        let ledger = self.ledger.lock().unwrap();
        assert!(
            !ledger.receipts.is_empty(),
            "required proxy observation was not executed"
        );
        let before_login = ledger
            .receipts
            .iter()
            .filter(|receipt| {
                matches!(
                    receipt.phase,
                    Phase::BeforeManualRestart | Phase::ManualStart | Phase::RestartRecovery
                )
            })
            .count();
        eprintln!(
            "packaged CPA proxy observation completed: attempts={}, before-manual-login-attempts={before_login}, fixed-updater-origin-only=true, forwarding=false, tls-tunnel=false, oauth-account=not-executed, model-account=not-executed",
            ledger.receipts.len()
        );
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop.store(true, Ordering::Release);
            let stopped = self.done.recv_timeout(Duration::from_secs(5)).is_ok();
            if stopped {
                let _ = self.thread.take().unwrap().join();
            }
            eprintln!(
                "packaged CPA proxy observation failure cleanup: observer-stopped={stopped}, forwarding=false"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(target: &str, extra: &str) -> Vec<u8> {
        format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nUser-Agent: Go-http-client/1.1\r\n{extra}\r\n").into_bytes()
    }
    fn approved(bytes: &[u8]) -> bool {
        Receipt {
            phase: Phase::LoginStart,
            facts: classify(bytes),
            header_bytes: bytes.len(),
            read_end: ReadEnd::Complete,
            io_kind: None,
            native_code: None,
            rejected_502: true,
            peer_eof: true,
            extra_bytes: false,
        }
        .approved()
    }
    #[test]
    fn only_exact_pinned_updater_origin_is_an_approved_rejected_connect() {
        assert!(approved(&request(UPDATER, "")));
        for target in [
            "api.github.com:443",
            "cpamc.router-for.me:443",
            "auth.openai.com:443",
            "claude.ai:443",
            "chatgpt.com:443",
            "api.anthropic.com:443",
            "antigravity-hub-auto-updater-974169037036.us-central1.run.app.evil.test:443",
            "antigravity-hub-auto-updater-974169037036.us-central1.run.app:444",
        ] {
            assert!(!approved(&request(target, "")));
        }
        for method in ["GET", "POST", "connect"] {
            let bytes = String::from_utf8(request(UPDATER, ""))
                .unwrap()
                .replacen("CONNECT", method, 1);
            assert!(!approved(bytes.as_bytes()));
        }
    }
    #[test]
    fn credentials_bodies_unknown_headers_and_ambiguous_hosts_stay_red() {
        const PRIVATE: &str = "private-secret-account-never-emitted";
        for header in [
            format!("Authorization: Bearer {PRIVATE}\r\n"),
            format!("Proxy-Authorization: Basic {PRIVATE}\r\n"),
            "Content-Length: 1\r\n".into(),
            "Transfer-Encoding: chunked\r\n".into(),
            format!("X-Private: {PRIVATE}\r\n"),
            format!("Host: {UPDATER}\r\n"),
        ] {
            let bytes = request(UPDATER, &header);
            assert!(!approved(&bytes));
            assert!(!format!("{:?}", classify(&bytes)).contains(PRIVATE));
        }
        let mut bytes = request(UPDATER, "");
        bytes.extend_from_slice(PRIVATE.as_bytes());
        assert!(!approved(&bytes));
        assert!(!approved(b""));
        assert!(!approved(&request(UPDATER, "")[..20]));
    }
    #[test]
    fn actual_loopback_connect_is_rejected_without_tls_then_observes_peer_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(&request(UPDATER, "")).unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            assert_eq!(
                response,
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        });
        let receipt = observe(listener.accept().unwrap().0, Phase::LoginStart);
        client.join().unwrap();
        assert!(receipt.approved());
    }
    #[test]
    fn accepted_nonblocking_proxy_waits_for_delayed_peer_eof_after_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(CONNECTION_BOUND)).unwrap();
            stream.write_all(&request(UPDATER, "")).unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            assert_eq!(
                response,
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            thread::sleep(Duration::from_millis(50));
        });
        let stream = listener.accept().unwrap().0;
        stream.set_nonblocking(true).unwrap();
        // Buffer the entire CONNECT before observing it. The old mode must fail
        // specifically while awaiting the client's delayed EOF after the 502.
        let deadline = Instant::now() + CONNECTION_BOUND;
        let mut head = [0u8; HEADER_BOUND];
        loop {
            match stream.peek(&mut head) {
                Ok(count) if count >= request(UPDATER, "").len() => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("proxy regression header peek failed: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "proxy regression CONNECT was not buffered"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let receipt = observe(stream, Phase::LoginStart);
        client.join().unwrap();
        assert!(
            receipt.approved(),
            "bounded rejected CONNECT must observe actual peer EOF: {receipt:?}"
        );
    }

    #[test]
    fn empty_closed_socket_is_not_a_successful_proxy_observation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let receipt = observe(listener.accept().unwrap().0, Phase::ManualStart);
        assert!(!receipt.approved());
        assert_eq!(receipt.read_end, ReadEnd::Eof);
        assert_eq!(receipt.header_bytes, 0);
    }
}
