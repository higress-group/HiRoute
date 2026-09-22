//! Explicit native Messages/helper consumption, with formal protected file restoration.
//! The private helper is a protocol fixture, NOT evidence of the production hiroute CLI bootstrap.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires explicit trusted Claude binary and native platform admission"]
fn native_claude_consumes_helper_and_restores_owned_file() {
    let binary = fs::canonicalize(
        std::env::var_os("HIROUTE_NATIVE_CLAUDE").expect("explicit native binary required"),
    )
    .unwrap();
    let identity = fs::metadata(&binary).unwrap();
    assert!(identity.is_file() && identity.mode() & 0o022 == 0);
    for path in binary.ancestors() {
        let metadata = fs::metadata(path).unwrap();
        assert!(metadata.uid() == 0 || metadata.uid() == rustix::process::geteuid().as_raw());
        assert!(metadata.mode() & 0o002 == 0 || metadata.mode() & 0o1000 != 0);
        assert!(
            metadata.mode() & 0o020 == 0
                || metadata.mode() & 0o1000 != 0
                || metadata.uid() == rustix::process::geteuid().as_raw(),
            "a group-writable system-owned ancestor cannot select the native binary"
        );
    }
    let root = tempfile::Builder::new()
        .prefix("hiroute-native-claude-")
        .tempdir()
        .unwrap()
        .keep();
    let home = root.join("home");
    let workspace = root.join("workspace");
    let config = home.join(".claude");
    for path in [&home, &config, &workspace] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut entropy = [0; 32];
    getrandom::fill(&mut entropy).unwrap();
    let challenge = AgentAccessGrantMaterial::from_csprng_entropy(entropy);
    let token_path = root.join("challenge.private");
    write(&token_path, challenge.expose());
    let helper = root.join("fixture-helper");
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    write(&helper, format!("#!/bin/sh\n[ \"$1\" = '__internal-agent-grant-v1' ] || exit 2\n[ \"$2\" = 'agent-connection/claude/native-test' ] || exit 3\nexec /bin/cat {}\n", quote(&token_path)).as_bytes());
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let model = "hiroute/0011223344556677";
    let path = config.join("settings.json");
    let initial = json!({"theme":"dark","apiKeyHelper":"never-execute-original-helper",
        "env":{"ANTHROPIC_AUTH_TOKEN":"never-send-original-token","UNRELATED":"keep"}});
    write(&path, serde_json::to_vec(&initial).unwrap().as_slice());
    let change = AgentConfigChangeV1::preview(&AgentConfigDocumentV1 { fields: BTreeMap::from([
        ("apiKeyHelper".into(), json!({"configured":true})),
        ("hiroute.auth_environment".into(), json!({"configured":true})),
    ]) }, BTreeMap::from([
        ("apiKeyHelper".into(), Some(json!({"executable":helper, "argv":[
            hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,"agent-connection/claude/native-test"]}))),
        ("hiroute.auth_environment".into(), None),
        ("env.ANTHROPIC_MODEL".into(), Some(json!(model))),
        ("env.ANTHROPIC_BASE_URL".into(), Some(json!(endpoint))),
    ])).unwrap();
    let proto = claude_intent(AgentConnectionTransactionKindV1::Apply, None);
    let artifacts = store(&root, proto.target(), &path);
    let install = claude_intent(
        AgentConnectionTransactionKindV1::Apply,
        artifacts
            .current_external_fingerprint(proto.target())
            .unwrap(),
    );
    let operation = OperationId::parse("op_12345678123456781234567812345678").unwrap();
    let staged = stage_claude_configuration(
        &artifacts,
        &operation,
        &install,
        &CanonicalDigest::of_bytes(&serde_json::to_vec(&initial).unwrap()),
        &change,
    )
    .unwrap();
    artifacts.activate_artifact(&staged).unwrap();
    let output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join("native-output.private"))
        .unwrap();
    let result = run(
        &binary,
        &home,
        &workspace,
        &listener,
        challenge.expose(),
        model,
        output,
    );
    drop(artifacts);
    let artifacts = store(&root, install.target(), &path);
    let restore = claude_intent(
        AgentConnectionTransactionKindV1::Restore,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    let restore_operation = OperationId::parse("op_87654321876543218765432187654321").unwrap();
    let staged = stage_claude_restoration(
        &artifacts,
        &restore_operation,
        &restore,
        &operation,
        &install,
    )
    .unwrap();
    artifacts.activate_artifact(&staged).unwrap();
    let restored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        restored, initial,
        "native auth must be restored before cleanup"
    );
    let after = fs::metadata(&binary).unwrap();
    assert_eq!(
        (
            identity.dev(),
            identity.ino(),
            identity.len(),
            identity.mtime(),
            identity.ctime()
        ),
        (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.ctime()
        )
    );
    assert!(
        result.is_ok(),
        "native Claude probe {result:?}; restored materials retained at {}",
        root.display()
    );
    println!("native_claude_auth_header={}", result.unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn run(
    binary: &Path,
    home: &Path,
    workspace: &Path,
    listener: &TcpListener,
    challenge: &[u8],
    model: &str,
    output: fs::File,
) -> Result<&'static str, &'static str> {
    let observer = output.try_clone().map_err(|_| "output")?;
    let mut child = Command::new(binary)
        .args([
            "--print",
            "--no-session-persistence",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--no-chrome",
            "--permission-mode",
            "dontAsk",
            "--output-format",
            "json",
            "Reply with OK only. Do not use tools.",
        ])
        .env_clear()
        .env("HOME", home)
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("TMPDIR", workspace)
        .env("PATH", "/usr/bin:/bin")
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone().map_err(|_| "output")?))
        .stderr(Stdio::from(output))
        .process_group(0)
        .spawn()
        .map_err(|_| "spawn")?;
    let started = Instant::now();
    let mut seen = None;
    let mut count = 0;
    let result = (|| loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                count += 1;
                if count > 8 {
                    return Err("request limit");
                }
                if let Some(header) = serve(&mut stream, challenge, model)? {
                    seen = Some(header);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err("accept"),
        }
        if let Some(status) = child.try_wait().map_err(|_| "wait")? {
            return if status.success() {
                seen.ok_or("missing messages request")
            } else {
                Err("native exit nonzero")
            };
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err("deadline");
        }
        if observer.metadata().map_err(|_| "output")?.len() > 128 * 1024 {
            return Err("output limit");
        }
        std::thread::sleep(Duration::from_millis(10));
    })();
    if child.try_wait().ok().flatten().is_none()
        && let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.wait();
    result
}

fn serve(
    stream: &mut TcpStream,
    challenge: &[u8],
    model: &str,
) -> Result<Option<&'static str>, &'static str> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|_| "timeout")?;
    let started = Instant::now();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let (end, length, counting, auth_kind) = loop {
        if bytes.len() > 128 * 1024 || started.elapsed() > Duration::from_secs(2) {
            return Err("request bound");
        }
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "encoding")?;
            let mut first = header.lines().next().ok_or("request")?.split_whitespace();
            let method = first.next().ok_or("method")?;
            let path = first
                .next()
                .ok_or("path")?
                .split('?')
                .next()
                .ok_or("path")?;
            let safe_path = path.replace(
                std::str::from_utf8(challenge).map_err(|_| "challenge encoding")?,
                "[redacted]",
            );
            eprintln!(
                "native_claude_request_method={method} path={}",
                safe_path.chars().take(128).collect::<String>()
            );
            // Native connectivity preflight is not an authenticated model-consumption result.
            // Match an ordinary unsupported route instead of inventing a required hello service.
            if method == "HEAD" && path == "/api/hello" {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .map_err(|_| "hello response")?;
                return Ok(None);
            }
            if method != "POST" {
                return Err("method");
            }
            let counting = path == "/v1/messages/count_tokens";
            if !counting && path != "/v1/messages" {
                return Err("path");
            }
            let mut auth = None;
            let mut x_api_key_seen = false;
            let mut length = None;
            for line in header.lines().skip(1) {
                let (key, value) = line.split_once(':').ok_or("header")?;
                let value = value.trim();
                let supplied = if key.eq_ignore_ascii_case("authorization") {
                    Some((
                        "authorization",
                        value.strip_prefix("Bearer ").ok_or("bearer")?,
                    ))
                } else if key.eq_ignore_ascii_case("x-api-key") {
                    // Production core_runtime::inbound_authorization ignores x-api-key and
                    // authenticates only Bearer when both are present. Do not count this field.
                    if x_api_key_seen {
                        return Err("duplicate x-api-key");
                    }
                    x_api_key_seen = true;
                    None
                } else {
                    None
                };
                if let Some(supplied) = supplied
                    && auth.replace(supplied).is_some()
                {
                    return Err("duplicate auth");
                }
                if key.eq_ignore_ascii_case("content-length")
                    && length
                        .replace(value.parse::<usize>().map_err(|_| "length")?)
                        .is_some()
                {
                    return Err("duplicate length");
                }
            }
            let (auth_kind, value) = auth.ok_or("missing authentication")?;
            eprintln!("native_claude_x_api_key_present={x_api_key_seen}");
            if value.as_bytes() != challenge {
                return Err("wrong authentication");
            }
            break (
                end + 4,
                length.filter(|size| *size <= 128 * 1024).ok_or("length")?,
                counting,
                auth_kind,
            );
        }
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| "header read")?;
        if count == 0 {
            return Err("short header");
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    while bytes.len() < end + length {
        if started.elapsed() > Duration::from_secs(2) {
            return Err("body deadline");
        }
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| "body read")?;
        if count == 0 {
            return Err("short body");
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let request: serde_json::Value =
        serde_json::from_slice(&bytes[end..end + length]).map_err(|_| "json")?;
    if request["model"].as_str() != Some(model) {
        return Err("wrong model");
    }
    if counting {
        let body = "{\"input_tokens\":1}";
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).map_err(|_| "response")?;
        return Ok(None);
    }
    let events = [
        json!({"type":"message_start","message":{"id":"msg_probe","type":"message","role":"assistant","content":[],"model":model,"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"OK"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ];
    let body = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).map_err(|_| "response")?;
    Ok(Some(auth_kind))
}
