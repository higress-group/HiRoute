//! Public Windows CLI Native API management, with an independently observed upstream.
//! No direct database writes, internal operation dispatch, provider account or fake CLI.
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use hiroute_domain::UpstreamProtocol;
use hiroute_platform::windows::job::{Child, Launch, Stream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{InstalledProduct, SECRET, assert_no_secret};

#[path = "windows_headless_business_upstream.rs"]
mod upstream;
use upstream::Upstream;

const MODEL: &str = "windows-native-api-model";
const NAME: &str = "Windows public Native API";
const RENAMED: &str = "Windows public Native API renamed";
const KEY: &str = "windows-public-native-save";
const NATIVE_CANDIDATE: &str = "candidate/native/windows-public-native";
// The CLI serializes the same compact MachineEnvelope as Local Control. Options
// legitimately include the current model catalog; a smaller fixture-only bound
// would reject a valid production frame before its business assertions run.
const BOUND: u64 = hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES as u64;

pub struct Business {
    upstream: Option<Upstream>,
    candidate_ref: String,
    source_id: String,
    source: Value,
    save_request: Value,
    operation_id: String,
}

impl Business {
    pub fn save(product: &InstalledProduct) -> Self {
        let upstream = Upstream::start();
        Self::save_at(
            product,
            NATIVE_CANDIDATE,
            KEY,
            upstream.address,
            32768,
            UpstreamProtocol::Responses,
            Some(upstream),
        )
    }

    /// The caller retains an independent real upstream oracle for routing/Worker.
    pub fn save_on(
        product: &InstalledProduct,
        candidate_ref: &str,
        key: &str,
        address: SocketAddr,
    ) -> Self {
        Self::save_on_with_context(product, candidate_ref, key, address, 32768)
    }

    /// Explicit user-declared loopback model capacity; callers still use the real
    /// Compute Check/Save path. Claude mappings require a supported >=100k window.
    pub fn save_on_with_context(
        product: &InstalledProduct,
        candidate_ref: &str,
        key: &str,
        address: SocketAddr,
        context_tokens: u64,
    ) -> Self {
        assert!((16384..=1_000_000).contains(&context_tokens));
        Self::save_on_with_protocol(
            product,
            candidate_ref,
            key,
            address,
            context_tokens,
            UpstreamProtocol::Responses,
        )
    }

    /// Declare the actual Native transport and probe it before public Save.
    /// The loopback oracle supplies upstream bytes, never a HiRoute response.
    pub fn save_on_with_protocol(
        product: &InstalledProduct,
        candidate_ref: &str,
        key: &str,
        address: SocketAddr,
        context_tokens: u64,
        protocol: UpstreamProtocol,
    ) -> Self {
        assert!((16384..=1_000_000).contains(&context_tokens));
        Self::save_at(
            product,
            candidate_ref,
            key,
            address,
            context_tokens,
            protocol,
            None,
        )
    }

    pub fn binding_id(&self) -> &str {
        self.source["models"][0]["binding_id"].as_str().unwrap()
    }
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    fn save_at(
        product: &InstalledProduct,
        candidate_ref: &str,
        key: &str,
        address: SocketAddr,
        context_tokens: u64,
        protocol: UpstreamProtocol,
        upstream: Option<Upstream>,
    ) -> Self {
        assert!(
            candidate_ref.starts_with("candidate/native/")
                && !candidate_ref.starts_with("candidate/native/agent-token-"),
            "Native source input must use the model credential domain"
        );
        assert_eq!(
            product.register_inherited_secret_for(candidate_ref, SECRET)["data"]["registered"],
            true
        );
        let before = cli(product, &["compute", "list"], None);
        let before_ids: Vec<_> = before["data"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source["source_id"].as_str().unwrap().to_owned())
            .collect();
        let checked = cli(
            product,
            &["compute", "connection", "test"],
            Some(&json!({
                "kind":"native", "request":{
                    "draft":draft(address, candidate_ref, context_tokens, protocol),
                    "input_candidate":{"candidate_ref":candidate_ref,"candidate_revision":1}
                }
            })),
        );
        check_diagnostic(&checked["data"]);
        if let Some(upstream) = &upstream {
            upstream.diagnostic();
        }
        assert_eq!(checked["data"]["authentication"], "verified");
        assert_eq!(checked["data"]["inference"], "verified");
        assert_eq!(checked["data"]["candidate"]["fact_state"], "complete");
        if let Some(upstream) = &upstream {
            upstream.assert_requests(1, 0);
        }
        let candidate = &checked["data"]["candidate"];
        let model = candidate["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["upstream_model_id"] == MODEL && model["selectable"] == true)
            .expect("the actual checked Native model must be selectable");
        let snapshot = cli(product, &["compute", "list"], None);
        let preview = cli(
            product,
            &["compute", "connection", "preview"],
            Some(&json!({
                "change":{
                    "schema":"hiroute.compute-management-change/v2",
                    "subject":{"kind":"candidate","candidate":candidate["candidate"]},
                    "expected_revisions":snapshot["data"]["revisions"],
                    "selected_model_refs":[model["model_ref"]],"intent":"save_ready","key_edits":[]
                }
            })),
        );
        let save_request = apply_request(&preview["data"], key);
        let applied = cli(
            product,
            &["compute", "connection", "apply"],
            Some(&save_request),
        );
        let operation_id = applied["operation"]["operation_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_operation(product, &operation_id);
        let replay = cli(
            product,
            &["compute", "connection", "apply"],
            Some(&save_request),
        );
        assert_eq!(replay["operation"]["operation_id"], operation_id);
        let mut damaged = save_request.clone();
        damaged["accept_digest"] = json!(format!("sha256:{}", "0".repeat(64)));
        failure(
            product,
            &["compute", "connection", "apply"],
            &damaged,
            "IDEMPOTENCY_KEY_REUSED",
        );
        let found = cli(
            product,
            &["operations", "find"],
            Some(&json!({
                "principal_kind":"interactive_user","operation_kind":"ApplyComputeSave",
                "idempotency_key":key,"accepted_digest":save_request["accept_digest"]
            })),
        );
        assert_eq!(found["data"]["digest_matches"], true);
        assert_eq!(found["data"]["operation"]["operation_id"], operation_id);
        let snapshot = cli(product, &["compute", "list"], None);
        assert_eq!(
            snapshot["data"]["sources"].as_array().unwrap().len(),
            before_ids.len() + 1
        );
        let source = snapshot["data"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| !before_ids.iter().any(|id| source["source_id"] == *id))
            .unwrap()
            .clone();
        assert_eq!(source["display_name"], display_name(NAME, candidate_ref));
        assert_eq!(source["state"], "ready");
        assert_eq!(source["keys"].as_array().unwrap().len(), 1);
        assert_eq!(source["models"][0]["upstream_model_id"], MODEL);
        let source_id = source["source_id"].as_str().unwrap().to_owned();
        let options = cli(product, &["routing", "options"], Some(&json!({})));
        assert!(
            options["data"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |candidate| candidate["binding_id"] == source["models"][0]["binding_id"]
                        && candidate["routable"] == true
                )
        );
        assert_eq!(
            product.cli(&["protected-input", "release", "--candidate", candidate_ref])["data"]["registered"],
            true
        );
        // Rename updates saved control state without another HTTP request or key input.
        let preview = edit_preview(
            product,
            &source_id,
            json!({"action":"rename","display_name":display_name(RENAMED, candidate_ref)}),
        );
        let renamed = cli(
            product,
            &["compute", "connection", "apply"],
            Some(&apply_request(&preview["data"], &format!("{key}-rename"))),
        );
        assert_operation(
            product,
            renamed["operation"]["operation_id"].as_str().unwrap(),
        );
        let original_source = source;
        let source = saved(product, &source_id);
        assert_eq!(source["display_name"], display_name(RENAMED, candidate_ref));
        assert_eq!(source["keys"], original_source["keys"]);
        assert_eq!(
            source["models"][0]["binding_id"],
            original_source["models"][0]["binding_id"]
        );
        if let Some(upstream) = &upstream {
            upstream.assert_requests(1, 0);
        }
        Self {
            upstream,
            candidate_ref: candidate_ref.into(),
            source_id,
            source,
            save_request,
            operation_id,
        }
    }

    pub fn recovered(&self, product: &InstalledProduct, sequence: usize) {
        let actual = saved(product, &self.source_id);
        assert_eq!(actual["display_name"], self.source["display_name"]);
        assert_eq!(actual["revision"], self.source["revision"]);
        assert_eq!(actual["target"], self.source["target"]);
        assert_eq!(actual["keys"], self.source["keys"]);
        assert_eq!(
            actual["models"][0]["binding_id"],
            self.source["models"][0]["binding_id"]
        );
        let replay = cli(
            product,
            &["compute", "connection", "apply"],
            Some(&self.save_request),
        );
        assert_eq!(replay["operation"]["operation_id"], self.operation_id);
        assert_operation(product, &self.operation_id);
        // The protected input was released before restart. The real saved-source
        // consumer must reconstruct the target and read its committed Secret.
        let checked = cli(
            product,
            &["compute", "connection", "test"],
            Some(&self.recheck(sequence)),
        );
        check_diagnostic(&checked["data"]);
        if let Some(upstream) = &self.upstream {
            upstream.diagnostic();
        }
        assert_eq!(checked["data"]["directory"], "available");
        assert_eq!(checked["data"]["authentication"], "verified");
        assert_eq!(
            checked["data"]["candidate"]["existing_source_id"],
            self.source_id
        );
        self.upstream
            .as_ref()
            .expect("recovery gate owns its upstream oracle")
            .assert_requests(1, sequence);
    }

    pub fn delete(&self, product: &InstalledProduct) {
        let preview = edit_preview(product, &self.source_id, json!({"action":"delete"}));
        let deleted = cli(
            product,
            &["compute", "connection", "apply"],
            Some(&apply_request(
                &preview["data"],
                "windows-public-native-delete",
            )),
        );
        assert_operation(
            product,
            deleted["operation"]["operation_id"].as_str().unwrap(),
        );
        assert!(
            cli(product, &["compute", "list"], None)["data"]["sources"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        failure(
            product,
            &["compute", "connection", "test"],
            &self.recheck(3),
            "RESOURCE_NOT_FOUND",
        );
        self.upstream
            .as_ref()
            .expect("delete gate owns its upstream oracle")
            .assert_requests(1, 2);
        println!(
            "Windows public business: authenticated Native probe, saved source/key/model, exact idempotency, rename, graceful/forced recovery, saved credential recheck and deletion with zero further upstream requests"
        );
    }

    fn recheck(&self, sequence: usize) -> Value {
        json!({"kind":"saved","request":{
            "source_id":self.source_id,"expected_source_revision":self.source["revision"],
            "candidate_ref":self.candidate_ref,"edit_revision":sequence+1,
            "check_id":format!("check/windows/recovered-{sequence}")
        }})
    }
}

fn saved(product: &InstalledProduct, source_id: &str) -> Value {
    let shown = cli(product, &["compute", "show", source_id], None);
    assert_eq!(shown["data"]["sources"].as_array().unwrap().len(), 1);
    shown["data"]["sources"][0].clone()
}

fn edit_preview(product: &InstalledProduct, source_id: &str, edit: Value) -> Value {
    let snapshot = cli(product, &["compute", "list"], None);
    cli(
        product,
        &["compute", "connection", "preview"],
        Some(&json!({"change":{
            "schema":"hiroute.compute-management-change/v2",
            "subject":{"kind":"saved_source","source_id":source_id},
            "expected_revisions":snapshot["data"]["revisions"],"selected_model_refs":[],
            "intent":"save_ready","key_edits":[],"edit":edit
        }})),
    )
}

fn apply_request(preview: &Value, key: &str) -> Value {
    json!({"spec":preview["spec"],"accept_digest":preview["accept_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":key})
}

pub(crate) fn assert_operation(product: &InstalledProduct, operation_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let result = cli(product, &["operations", "get", operation_id], None);
        let state = result["data"]["state"].as_str().unwrap();
        if state == "succeeded" {
            return;
        }
        assert!(
            !matches!(state, "failed" | "cancelled" | "unknown"),
            "public management operation did not succeed"
        );
        assert!(
            Instant::now() < deadline,
            "public management operation did not settle"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn draft(
    address: SocketAddr,
    candidate_ref: &str,
    context_tokens: u64,
    protocol: UpstreamProtocol,
) -> Value {
    let declared = |value: Value| json!({"value":value,"basis":"user_declared"});
    let protocol = serde_json::to_value(protocol).unwrap();
    let profile = format!("profile/custom/{}", protocol.as_str().unwrap());
    json!({
        "inference_model_id":MODEL,"candidate_ref":candidate_ref,"lineage_ref":format!("lineage/{candidate_ref}"),
        "display_name":display_name(NAME, candidate_ref),"existing_source_id":null,"edit_revision":1,"check_id":"check/windows/public-native",
        "base_url":format!("http://{address}/v1"),"base_kind":"api_root","request_path_override":null,
        "inventory_path_override":"/v1/models","protocol":protocol,"protocol_profile_id":profile,
        "protocol_profile_revision":1,"authentication":{"kind":"bearer"},"configuration_revision":1,
        "models":[{"upstream_model_id":MODEL,"display_name":"Windows Native model","catalog_configuration_id":null,
            "membership":"user_declared","capabilities":{
                "tool":declared(json!(true)),"vision":declared(json!(false)),"streaming":declared(json!(true)),
                "context_tokens":declared(json!(context_tokens)),"max_output_tokens":declared(json!(4096)),
                "native_reasoning":declared(json!({"kind":"fixed","profile":"provider-default"}))
            }}]
    })
}

fn display_name(prefix: &str, candidate_ref: &str) -> String {
    // The production Rename contract rejects another saved source's name.
    // Multiple retained Sources in one real journey need independent names,
    // including when they share an upstream listener or model ID.
    let digest = Sha256::digest(candidate_ref.as_bytes());
    let suffix: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix} {suffix}")
}

pub(crate) fn cli(product: &InstalledProduct, args: &[&str], payload: Option<&Value>) -> Value {
    let detailed = payload
        .filter(|request| {
            matches!(args, ["sessions", "list" | "show" | "receipt"])
                && request["schema"] == "hiroute.observation.query/v2"
        })
        .cloned()
        .map(|mut request| {
            request["read_failure_details"] = json!(true);
            request
        });
    let payload = detailed.as_ref().or(payload);
    let (mut exit, mut value) = command(product, args, payload);
    for _ in 0..2 {
        if exit == 0
            || detailed.is_none()
            || value["error"]["code"] != "OBSERVATION_UNAVAILABLE"
            || value["error"]["retryable"] != true
            || !matches!(
                value["error"]["observation_read"]["reason"].as_str(),
                Some("timeout" | "reader_busy")
            )
        {
            break;
        }
        // Observe only a bounded transient read failure. The caller still
        // requires the complete session and its exact persisted receipts.
        thread::sleep(Duration::from_millis(100));
        (exit, value) = command(product, args, payload);
    }
    let (command, stage) = checkpoint(args, payload);
    if exit != 0 {
        let status = serde_json::from_value::<hiroute_application_api::MachineStatus>(
            value["status"].clone(),
        )
        .ok();
        let code = serde_json::from_value::<hiroute_application_api::ErrorCode>(
            value["error"]["code"].clone(),
        )
        .ok();
        let reason = serde_json::from_value::<
            hiroute_application_api::ObservationReadFailureReasonV1,
        >(value["error"]["observation_read"]["reason"].clone())
        .ok();
        eprintln!(
            "Windows public business failure: command={command}, stage={stage}, status={status:?}, code={code:?}, observation_reason={reason:?}"
        );
    }
    assert_eq!(
        exit, 0,
        "actual public Windows business CLI failed: command={command}, stage={stage}"
    );
    assert!(matches!(
        value["status"].as_str(),
        Some("succeeded" | "accepted")
    ));
    value
}

/// Closed command/stage labels identify a failing business entry without
/// reflecting resource IDs, request JSON, endpoint URLs or provider output.
fn checkpoint(args: &[&str], payload: Option<&Value>) -> (&'static str, &'static str) {
    let command = [
        (
            "compute.connection.test",
            &["compute", "connection", "test"][..],
        ),
        (
            "compute.connection.preview",
            &["compute", "connection", "preview"][..],
        ),
        (
            "compute.connection.apply",
            &["compute", "connection", "apply"][..],
        ),
        (
            "compute.connection.options",
            &["compute", "connection", "options"][..],
        ),
        ("compute.list", &["compute", "list"][..]),
        ("compute.show", &["compute", "show"][..]),
        ("operations.find", &["operations", "find"][..]),
        ("operations.get", &["operations", "get"][..]),
        ("routing.options", &["routing", "options"][..]),
        ("routing.preview", &["routing", "preview"][..]),
        ("routing.apply", &["routing", "apply"][..]),
        ("routing.list", &["routing", "list"][..]),
        ("routing.show", &["routing", "show"][..]),
        (
            "decision.services.list",
            &["decision", "services", "list"][..],
        ),
        (
            "decision.services.apply",
            &["decision", "services", "apply"][..],
        ),
        (
            "decision.services.test",
            &["decision", "services", "test"][..],
        ),
        ("models.show", &["models", "show"][..]),
        ("agents.scan", &["agents", "scan"][..]),
        (
            "agents.connect.preview",
            &["agents", "connect", "preview"][..],
        ),
        ("agents.connect.apply", &["agents", "connect", "apply"][..]),
        (
            "agents.connect.status",
            &["agents", "connect", "status"][..],
        ),
        (
            "agents.restore.preview",
            &["agents", "restore", "preview"][..],
        ),
        ("agents.restore.apply", &["agents", "restore", "apply"][..]),
        (
            "agents.access-token.show",
            &["agents", "access-token", "show"][..],
        ),
        ("sessions.list", &["sessions", "list"][..]),
        ("sessions.show", &["sessions", "show"][..]),
        ("sessions.receipt", &["sessions", "receipt"][..]),
        ("sessions.status", &["sessions", "status"][..]),
        (
            "observation.plan-quality.samples",
            &["observation", "plan-quality", "samples"][..],
        ),
        ("value.show", &["value", "show"][..]),
    ]
    .into_iter()
    .find_map(|(label, prefix)| args.starts_with(prefix).then_some(label))
    .unwrap_or("other-public-command");
    let stage = payload
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .and_then(|kind| match kind {
            "native" => Some("native-model-check"),
            "saved" => Some("saved-model-recheck"),
            "reference" => Some("model-reference"),
            "ratings" => Some("model-ratings"),
            _ => None,
        })
        .or_else(|| {
            payload.and_then(|value| {
                match value.pointer("/change/edit/action").and_then(Value::as_str) {
                    Some("rename") => Some("saved-source-rename"),
                    Some("delete") => Some("saved-source-delete"),
                    _ => None,
                }
            })
        })
        .unwrap_or("ordinary");
    (command, stage)
}

pub(crate) fn failure(product: &InstalledProduct, args: &[&str], payload: &Value, expected: &str) {
    let (exit, value) = command(product, args, Some(payload));
    assert_ne!(
        exit, 0,
        "real public rejection must have a failing process exit"
    );
    assert_eq!(value["error"]["code"], expected);
}

pub(crate) fn command(
    product: &InstalledProduct,
    args: &[&str],
    payload: Option<&Value>,
) -> (i32, Value) {
    command_capture(product, args, payload, false)
}

pub(crate) fn missing_gateway_manager(product: &InstalledProduct, args: &[&str]) -> (i32, Value) {
    assert!(matches!(
        args,
        ["gateway", "set", ..] | ["gateway", "recover"]
    ));
    command_capture(product, args, None, true)
}

fn command_capture(
    product: &InstalledProduct,
    args: &[&str],
    payload: Option<&Value>,
    missing_manager: bool,
) -> (i32, Value) {
    let (command, stage) = checkpoint(args, payload);
    eprintln!("Windows public business checkpoint: command={command}, stage={stage}");
    // Exercise the released CLI's default request deadline. This business
    // journey verifies effects, not an additional fixture-only latency budget.
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    if payload.is_some() {
        args.push("--request-stdin".into());
    }
    args.extend(["--output", "json"].map(OsString::from));
    let environment: Vec<_> = std::env::vars_os().collect();
    let started = Instant::now();
    let mut child = Child::spawn(Launch {
        program: &product.stable,
        args: &args,
        env: &environment,
        cwd: product.stable.parent(),
        stdin: if payload.is_some() {
            Stream::Piped
        } else {
            Stream::Null
        },
        stdout: Stream::Piped,
        stderr: Stream::Piped,
    })
    .unwrap();
    let capture = |file: std::fs::File| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            file.take(BOUND + 1).read_to_end(&mut bytes).unwrap();
            assert!(
                bytes.len() as u64 <= BOUND,
                "public business CLI capture exceeded its bound"
            );
            assert_no_secret(&bytes);
            bytes
        })
    };
    let stdout = capture(child.stdout.take().unwrap());
    let stderr = capture(child.stderr.take().unwrap());
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(payload).unwrap();
        assert_no_secret(&bytes);
        assert!(bytes.len() <= 65536);
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    let exit = loop {
        if let Some(exit) = child.exit_code().unwrap() {
            break exit;
        }
        if Instant::now() >= deadline {
            child.terminate().unwrap();
            panic!("actual public business CLI exceeded its deadline");
        }
        thread::sleep(Duration::from_millis(10));
    };
    while !child.empty().unwrap() {
        assert!(
            Instant::now() < deadline,
            "public business CLI leaked descendant processes"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let output = stdout.join().unwrap();
    let error = stderr.join().unwrap();
    if !error.is_empty() && !missing_manager {
        let value = serde_json::from_slice::<Value>(&output).ok();
        let status = value.as_ref().and_then(|value| {
            serde_json::from_value::<hiroute_application_api::MachineStatus>(
                value["status"].clone(),
            )
            .ok()
        });
        let code = value.as_ref().and_then(|value| {
            serde_json::from_value::<hiroute_application_api::ErrorCode>(
                value["error"]["code"].clone(),
            )
            .ok()
        });
        // Preserve only closed production enums and numeric process facts.
        eprintln!(
            "Windows public business failure: exit={exit}, elapsed-ms={}, stderr-bytes={}, status={status:?}, code={code:?}",
            started.elapsed().as_millis(),
            error.len()
        );
    }
    let envelope: Value = serde_json::from_slice(&output)
        .expect("public business CLI must return its production JSON envelope");
    if missing_manager {
        assert_eq!(exit, 6);
        assert_eq!(envelope["status"], "unavailable");
        assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");
        assert!(error == "HiRoute 服务不可用。Standalone 请运行 `hiroute service status`，并在需要时运行 `hiroute service start`；Desktop 请启动或恢复应用。隔离实例请检查运行目录。不会自动启动或重放请求。\n".as_bytes(), "expected unavailable-manager hint changed");
    } else {
        assert!(
            error.is_empty(),
            "public business CLI emitted unexpected diagnostics"
        );
    }
    (exit, envelope)
}

/// Only native enums, allowlisted issue codes and numeric facts are diagnostic.
/// Do not print the typed DTO: it also carries target and candidate selectors.
fn check_diagnostic(value: &Value) {
    use hiroute_application_api::ModelConnectionCheckViewV1;
    let check: ModelConnectionCheckViewV1 = serde_json::from_value(value.clone())
        .expect("Native connection check must conform to its production DTO");
    let codes: Vec<_> = check
        .issues
        .iter()
        .take(32)
        .map(|issue| match issue.code.as_str() {
            "NEEDS_CREDENTIAL"
            | "RESPONSE_TOO_LARGE"
            | "AUTH_REJECTED"
            | "DIRECTORY_FORBIDDEN"
            | "DIRECTORY_UNAVAILABLE"
            | "RATE_LIMITED"
            | "REDIRECT_REJECTED"
            | "DIRECTORY_FAILED"
            | "DIRECTORY_INVALID"
            | "MODEL_LIMIT_REACHED"
            | "DIRECTORY_PARTIAL"
            | "DIRECTORY_TRUNCATED"
            | "INVALID_CHECK_LIMITS"
            | "CHECK_CANCELLED"
            | "CHECK_TIMEOUT"
            | "AUTH_INPUT_INVALID"
            | "TRANSPORT_FAILED"
            | "TOOL_CALL_NOT_VERIFIED"
            | "INFERENCE_FAILED" => issue.code.as_str(),
            _ => "UNRECOGNIZED_ISSUE_CODE",
        })
        .collect();
    eprintln!(
        "Windows Native check: reachability={:?}, authentication={:?}, directory={:?}, protocol={:?}, inference={:?}, checked_models={}, invalid_models={}, pages={}, issue_count={}, codes={codes:?}",
        check.reachability,
        check.authentication,
        check.directory,
        check.protocol,
        check.inference,
        check.checked_model_count,
        check.invalid_model_count,
        check.pages_read,
        check.issues.len()
    );
}
