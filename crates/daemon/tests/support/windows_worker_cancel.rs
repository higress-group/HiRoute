//! Real public cancellation after a native tool child has actually started. The
//! same Job/Worker boundary must stop that child before publishing cancellation.
use super::{InstalledProduct, command, command_outcome, exec_args, oracle, run, write_new};
use hiroute_application_api::{
    MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, WORKER_COMMAND_DATA_SCHEMA_V1,
    WorkerCommandDataV1, WorkerReadDataV1,
};
use hiroute_platform::windows::security;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

pub(super) fn opencode_command(workspace: &Path, nonce: &str) -> String {
    use base64::Engine;
    let literal = |text: &str| {
        format!(
            "Buffer.from('{}','base64').toString('utf8')",
            base64::engine::general_purpose::STANDARD.encode(text)
        )
    };
    // OpenCode exposes its Windows shell as bash even when it selects PowerShell
    // or cmd. Reuse the runner's pinned Node in PATH; encode user paths so this
    // one native command crosses all three shells without interpreting them.
    format!(
        "node.exe -e \"const fs=require('fs');fs.writeFileSync({},{});setTimeout(()=>fs.writeFileSync({},{}),60000)\"",
        literal(workspace.join("cancel-started.txt").to_str().unwrap()),
        literal(&format!("started-{nonce}")),
        literal(workspace.join("cancel-late.txt").to_str().unwrap()),
        literal(&format!("late-{nonce}")),
    )
}

pub(super) struct CancellationReceipts {
    pub late: Vec<PathBuf>,
    pub residual_task: String,
}

pub(super) fn verify(
    product: &InstalledProduct,
    workspace: &Path,
    plan: &str,
    nonce: &str,
) -> CancellationReceipts {
    verify_for(product, workspace, plan, nonce, "pi")
}

pub(crate) fn verify_opencode(
    product: &InstalledProduct,
    workspace: &Path,
    plan: &str,
    nonce: &str,
) {
    let _ = verify_for(product, workspace, plan, nonce, "opencode");
}

fn verify_for(
    product: &InstalledProduct,
    workspace: &Path,
    plan: &str,
    nonce: &str,
    harness: &str,
) -> CancellationReceipts {
    let executors = command(
        product,
        &["worker", "executors"],
        None,
        Duration::from_secs(30),
    );
    let pi = executors["data"]["executors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|executor| executor["harness"] == harness)
        .unwrap();
    assert_eq!(pi["start_approve_all"]["state"], "ready");
    assert_eq!(pi["cancel"]["state"], "ready");
    if harness == "pi" {
        assert_eq!(pi["continue_session"]["state"], "unknown");
        assert_eq!(pi["continue_session"]["reason"], "capability_unverified");
    }
    let prompt = workspace.join("cancel-task.txt");
    write_new(
        &prompt,
        format!(
            "{}: Perform the bounded cancellation receipt task using the native shell tool.\n",
            oracle::CANCEL
        )
        .as_bytes(),
    );
    let accepted = command(
        product,
        &exec_args(plan, workspace, &prompt, "windows-public-worker-cancel"),
        None,
        Duration::from_secs(180),
    );
    let run_id = accepted["data"]["run_id"].as_str().unwrap();
    let started = workspace.join("cancel-started.txt");
    let late = workspace.join("cancel-late.txt");
    let deadline = Instant::now() + Duration::from_secs(30);
    let expected_started = format!("started-{nonce}");
    loop {
        let ready = started_receipt_ready(&started, &expected_started)
            .expect("native cancellation receipt could not be read through its held identity");
        assert!(
            Instant::now() < deadline,
            "real native tool child did not publish its complete receipt before the cancellation deadline"
        );
        if ready {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!late.exists());
    let cancelled = command(
        product,
        &[
            "worker",
            "cancel",
            "--run",
            run_id,
            "--idempotency-key",
            "windows-public-cancel",
        ],
        None,
        Duration::from_secs(30),
    );
    assert_eq!(cancelled["data"]["run_id"], run_id);
    let mut current = command(
        product,
        &["worker", "wait", "--run", run_id, "--wait-timeout", "20"],
        None,
        Duration::from_secs(30),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while current["data"]["run_state"] != "cancelled" {
        // ACP interruption may publish Unknown before the owned stop checkpoint
        // confirms Cancelled. Wait for that proof within the existing bound;
        // persistent uncertainty still fails and is never accepted as cancellation.
        assert!(!matches!(
            current["data"]["run_state"].as_str(),
            Some("failed" | "succeeded")
        ));
        assert!(
            Instant::now() < deadline,
            "cancelled public Worker did not settle its exact native scope"
        );
        current = command(
            product,
            &["worker", "wait", "--run", run_id, "--wait-timeout", "5"],
            None,
            Duration::from_secs(10),
        );
    }
    assert!(
        !current["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["command_id"] == "worker.cleanup.confirm"),
        "known stopped Job must not be reported as residual cleanup success"
    );
    let result = command(
        product,
        &["worker", "result", "--run", run_id],
        None,
        Duration::from_secs(30),
    );
    assert_eq!(result["data"]["run_id"], run_id);
    assert_eq!(result["data"]["run_state"], "cancelled");
    assert!(
        !late.exists(),
        "native tool continued writing after public cancellation"
    );
    let read = command(
        product,
        &["worker", "read", "--run", run_id, "--max-bytes", "1024"],
        None,
        Duration::from_secs(30),
    );
    let revision = read["data"]["state_revision"].as_u64().unwrap().to_string();
    let arguments = [
        "worker",
        "cleanup",
        "confirm",
        "--run",
        run_id,
        "--expected-revision",
        &revision,
        "--idempotency-key",
        "windows-clean-run-cannot-ack-residual",
        "--handled",
        "--output",
        "json",
    ];
    let (exit, bytes, _) = run(
        &product.stable,
        &arguments.iter().map(OsString::from).collect::<Vec<_>>(),
        product.stable.parent(),
        None,
        Duration::from_secs(35),
    );
    let rejected: Value = serde_json::from_slice(&bytes).unwrap();
    assert_ne!(
        exit, 0,
        "known-clean state must reject fabricated residual confirmation"
    );
    assert_eq!(rejected["error"]["code"], "REVISION_CONFLICT");
    assert_eq!(rejected["status"], json!("conflict"));
    if harness == "opencode" {
        return CancellationReceipts {
            late: vec![late],
            residual_task: String::new(),
        };
    }
    let (residual_late, residual_task) = residual(product, workspace, plan, nonce);
    CancellationReceipts {
        late: vec![late, residual_late],
        residual_task,
    }
}

/// Real Windows sharing failure, followed by explicit user handling and the
/// released confirmation producer. Confirmation acknowledges the occupied slot;
/// it does not perform filesystem cleanup or claim that the run root was deleted.
fn residual(
    product: &InstalledProduct,
    workspace: &Path,
    plan: &str,
    nonce: &str,
) -> (PathBuf, String) {
    let prompt = workspace.join("residual-task.txt");
    write_new(
        &prompt,
        format!(
            "{}: Start the bounded native cancellation receipt task.\n",
            oracle::RESIDUAL
        )
        .as_bytes(),
    );
    let accepted = command(
        product,
        &exec_args(plan, workspace, &prompt, "windows-public-worker-residual"),
        None,
        Duration::from_secs(180),
    );
    let run_id = accepted["data"]["run_id"].as_str().unwrap();
    let task_id = accepted["data"]["task_id"].as_str().unwrap();
    let started = workspace.join("residual-started.json");
    let late = workspace.join("residual-late.txt");
    let deadline = Instant::now() + Duration::from_secs(30);
    let expected = format!("started-{nonce}");
    let binding: Value = loop {
        let pending = match residual_binding(&started, &expected) {
            Ok((_, Some(value))) => break value,
            Ok((pending, None)) => pending,
            Err(error) => {
                residual_diagnostics(product, task_id, run_id, error.stage, Some(&error.error));
                panic!(
                    "native residual binding could not be read through its bounded held identity"
                );
            }
        };
        if Instant::now() >= deadline {
            residual_diagnostics(product, task_id, run_id, pending, None);
            panic!("native residual tool did not produce its bounded process binding");
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(binding["receipt"], format!("started-{nonce}"));
    let pid = u32::try_from(binding["pid"].as_u64().unwrap()).unwrap();
    assert_ne!(pid, 0);
    // The locator comes only from the actual native tool's $PID. Pin its kernel
    // object while it is live, and observe that same handle after cancellation.
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    assert!(
        !process.is_null(),
        "cannot pin the actual native residual tool process"
    );
    // SAFETY: OpenProcess returned a fresh owned kernel handle; File closes it once.
    let process = unsafe { File::from_raw_handle(process) };
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );

    let temporary = PathBuf::from(binding["temp"].as_str().unwrap());
    assert!(temporary.is_absolute());
    assert_eq!(temporary.file_name().unwrap(), "tmp");
    let root = std::fs::canonicalize(temporary.parent().unwrap()).unwrap();
    let base = std::fs::canonicalize(
        product
            .layout
            .storage_root()
            .join("delegation-workers/runs"),
    )
    .unwrap();
    assert_eq!(root.parent().unwrap(), base);
    let name = root.file_name().unwrap().to_str().unwrap();
    assert!(name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let base_handle = security::open_owner_only(&base, true).unwrap();
    let root_handle = security::open_owner_only(&root, true).unwrap();
    let parent_handle = security::open_owner_only(root.parent().unwrap(), true).unwrap();
    assert_eq!(
        security::file_identity(&parent_handle).unwrap(),
        security::file_identity(&base_handle).unwrap()
    );
    let sdk = root.join("pi-sdk-contract.mjs");
    let mut deny_delete = security::open_owner_only(&sdk, false).unwrap();
    let sdk_identity = security::file_identity(&deny_delete).unwrap();
    // Exactly the production material producer, without a second fixture copy.
    let expected = include_bytes!("../../../integrations/src/agents/pi_sdk_contract.mjs");
    assert_eq!(deny_delete.metadata().unwrap().len(), expected.len() as u64);
    let mut actual = Vec::new();
    (&mut deny_delete)
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut actual)
        .unwrap();
    assert_eq!(actual, expected);
    assert!(!late.exists());

    command(
        product,
        &[
            "worker",
            "cancel",
            "--run",
            run_id,
            "--idempotency-key",
            "windows-public-residual-cancel",
        ],
        None,
        Duration::from_secs(30),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    // The held material deliberately prevents complete cleanup. The shared
    // ProcessStopped checkpoint keeps this interrupted run Unknown until residual
    // handling; it only confirms Cancelled when the managed scope AND cleanup are
    // complete. Acknowledgement must preserve that execution uncertainty.
    loop {
        let observed = command(
            product,
            &["worker", "wait", "--run", run_id, "--wait-timeout", "5"],
            None,
            Duration::from_secs(10),
        );
        if observed["data"]["run_state"] == "unknown" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native residual cancellation did not settle"
        );
        assert!(
            !matches!(
                observed["data"]["run_state"].as_str(),
                Some("succeeded" | "failed" | "cancelled")
            ),
            "locked residual must retain execution uncertainty"
        );
    }
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 10_000) },
        WAIT_OBJECT_0,
        "the exact native tool process remains live after cancellation"
    );
    assert!(!late.exists());
    let listed = command(product, &["worker", "list"], None, Duration::from_secs(30));
    let task = listed["data"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["task_id"] == task_id)
        .unwrap();
    assert_eq!(task["run"]["run_id"], run_id);
    assert_eq!(task["run"]["state"], "unknown");
    assert_eq!(
        task["run"]["cleanup"], "unknown",
        "a real deny-delete conflict must remain observable as residual cleanup"
    );
    let read = command(
        product,
        &["worker", "read", "--run", run_id, "--max-bytes", "1024"],
        None,
        Duration::from_secs(30),
    );
    assert_eq!(read["data"]["run_state"], "unknown");
    let revision = read["data"]["state_revision"].as_u64().unwrap().to_string();
    drop(deny_delete);
    // Handle only this test's exact locked material. The daemon retains its run
    // directory identity; an acknowledgement does not assert automatic root removal.
    let handled = security::open_delete_at(&root_handle, sdk.file_name().unwrap(), false).unwrap();
    assert_eq!(security::file_identity(&handled).unwrap(), sdk_identity);
    security::delete_by_handle(&handled).unwrap();
    drop(handled);
    let args = [
        "worker",
        "cleanup",
        "confirm",
        "--run",
        run_id,
        "--expected-revision",
        &revision,
        "--idempotency-key",
        "windows-public-residual-handled",
        "--handled",
    ];
    let confirmed = command(product, &args, None, Duration::from_secs(30));
    assert_eq!(confirmed["data"]["task_id"], task_id);
    assert_eq!(confirmed["data"]["run_id"], run_id);
    assert_eq!(confirmed["data"]["state"], "unknown");
    assert_eq!(confirmed["data"]["cleanup"], "residual_acknowledged");
    let replay = command(product, &args, None, Duration::from_secs(30));
    assert_eq!(confirmed["data"], replay["data"]);
    assert_eq!(
        confirmed["data"]["state_revision"].as_u64().unwrap(),
        revision.parse::<u64>().unwrap() + 1
    );
    assert!(!late.exists());
    (late, task_id.to_owned())
}

#[derive(Debug)]
struct BindingFailure {
    stage: &'static str,
    error: io::Error,
}
impl BindingFailure {
    fn native(stage: &'static str, error: io::Error) -> Self {
        Self { stage, error }
    }
    fn invalid(stage: &'static str) -> Self {
        Self::native(
            stage,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "native residual binding rejected",
            ),
        )
    }
}

fn residual_binding(
    path: &Path,
    expected: &str,
) -> Result<(&'static str, Option<Value>), BindingFailure> {
    let file = match security::open_identity(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(("file-missing", None)),
        Err(error) if error.raw_os_error() == Some(32) => {
            return Ok(("writer-sharing-conflict", None));
        }
        Err(error) => return Err(BindingFailure::native("open-rejected", error)),
    };
    if file
        .metadata()
        .map_err(|error| BindingFailure::native("metadata-query-rejected", error))?
        .len()
        > 4096
    {
        return Err(BindingFailure::invalid("size-bound"));
    }
    if security::file_link_count(&file)
        .map_err(|error| BindingFailure::native("link-query-rejected", error))?
        != 1
    {
        return Err(BindingFailure::invalid("alias-rejected"));
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|error| BindingFailure::native("read-rejected", error))?;
    if bytes.is_empty() {
        return Ok(("file-empty", None));
    }
    if bytes.len() > 4096 {
        return Err(BindingFailure::invalid("growth-bound"));
    }
    match std::str::from_utf8(&bytes) {
        Ok(_) => {}
        Err(error) if error.error_len().is_none() => return Ok(("utf8-partial", None)),
        Err(_) => return Err(BindingFailure::invalid("utf8-malformed")),
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) if error.is_eof() => return Ok(("json-partial", None)),
        Err(_) => return Err(BindingFailure::invalid("json-malformed")),
    };
    let object = value.as_object();
    if !object.is_some_and(|object| {
        object.len() == 3
            && object.contains_key("pid")
            && object.contains_key("temp")
            && object.contains_key("receipt")
    }) || value["receipt"] != expected
        || !value["pid"]
            .as_u64()
            .is_some_and(|pid| pid > 0 && u32::try_from(pid).is_ok())
        || !value["temp"]
            .as_str()
            .is_some_and(|temp| !temp.is_empty() && temp.len() <= 4096 && !temp.contains('\0'))
    {
        return Err(BindingFailure::invalid("binding-shape-or-nonce"));
    }
    Ok(("binding-complete", Some(value)))
}

fn residual_diagnostics(
    product: &InstalledProduct,
    task: &str,
    run_id: &str,
    pending: &'static str,
    error: Option<&io::Error>,
) {
    println!(
        "Windows public residual binding facts: {}",
        json!({
            "schema":"hiroute.windows-residual-binding-facts/v1","last_observation":pending,
            "native_code":error.and_then(io::Error::raw_os_error),"native_read_rejected":error.is_some()
        })
    );
    // The original 30-second binding deadline has failed. These independent
    // bounded public reads diagnose it; neither retries the Worker nor admits it.
    let (result_exit, result) = command_outcome(
        product,
        &["worker", "result", "--run", run_id],
        None,
        Duration::from_secs(10),
    );
    let (read_exit, read) = command_outcome(
        product,
        &["worker", "read", "--run", run_id, "--max-bytes", "4096"],
        None,
        Duration::from_secs(10),
    );
    let result_envelope = serde_json::from_value::<MachineEnvelopeV2<WorkerCommandDataV1>>(result)
        .ok()
        .filter(|envelope| envelope.schema_version == MACHINE_ENVELOPE_SCHEMA_V2);
    let read_envelope = serde_json::from_value::<MachineEnvelopeV2<WorkerReadDataV1>>(read)
        .ok()
        .filter(|envelope| envelope.schema_version == MACHINE_ENVELOPE_SCHEMA_V2);
    let result = result_envelope
        .as_ref()
        .and_then(|envelope| envelope.data.as_ref())
        .filter(|data| data.schema == WORKER_COMMAND_DATA_SCHEMA_V1 && data.operation == "result");
    let read = read_envelope
        .as_ref()
        .and_then(|envelope| envelope.data.as_ref())
        .filter(|data| data.valid());
    println!(
        "Windows public residual run facts: {}",
        json!({
            "schema":"hiroute.windows-residual-run-facts/v1","result_exit":result_exit,"read_exit":read_exit,
            "result_status":result_envelope.as_ref().map(|envelope|&envelope.status),
            "read_status":read_envelope.as_ref().map(|envelope|&envelope.status),
            "result_error_code":result_envelope.as_ref().and_then(|envelope|envelope.error.as_ref()).map(|error|&error.code),
            "read_error_code":read_envelope.as_ref().and_then(|envelope|envelope.error.as_ref()).map(|error|&error.code),
            "result_schema_valid":result.is_some(),"read_schema_valid":read.is_some(),
            "result_run_state":result.as_ref().and_then(|data|data.run_state),
            "read_run_state":read.as_ref().map(|data|data.run_state),
            "result_identity_matches":result.as_ref().is_some_and(|data|data.task_id.as_deref()==Some(task)&&data.run_id.as_deref()==Some(run_id)),
            "read_identity_matches":read.as_ref().is_some_and(|data|data.task_id==task&&data.run_id==run_id),
            "result_timed_out":result.as_ref().map(|data|data.timed_out),
            "result_bytes":result.as_ref().and_then(|data|data.result.as_ref()).map(String::len),
            "progress_bytes":read.as_ref().and_then(|data|data.text.as_ref()).map(String::len),
            "hiroute_dependency_hint":read.as_ref().and_then(|data|data.text.as_deref()).is_some_and(|text|text.lines().any(|line|line.starts_with("[HiRoute] Worker dependency check failed: "))),
            "native_error_reason_known":false
        })
    );
}

/// Shell redirection creates the file before printf publishes its bytes. Retain a
/// reparse-free read handle, but allow the real producer to finish writing. Empty
/// and partial contents are pending within the original deadline, never success.
fn started_receipt_ready(path: &Path, expected: &str) -> io::Result<bool> {
    let file = match security::open_identity(path, false) {
        Ok(file) => file,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(32) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    const BOUND: u64 = 4096;
    if file.metadata()?.len() > BOUND || security::file_link_count(&file)? != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native cancellation receipt exceeds its bound or aliases another file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(BOUND + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > BOUND {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native cancellation receipt grew beyond its bound",
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "native receipt is not UTF-8"))?;
    Ok(text.trim() == expected)
}

#[cfg(test)]
mod tests {
    use super::{residual_binding, started_receipt_ready};
    use hiroute_platform::windows::security;
    use std::io::Write;

    #[test]
    fn residual_binding_waits_for_complete_json_and_rejects_unreadable_native_identity() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-residual-binding-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        drop(security::create_owner_only_directory(&root).unwrap());
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let path = root.join("binding.json");
        let expected = "started-fixture-nonce";
        assert_eq!(residual_binding(&path, expected).unwrap().0, "file-missing");
        drop(security::create_owner_only_file(&path).unwrap());
        let mut producer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        assert_eq!(residual_binding(&path, expected).unwrap().0, "file-empty");
        producer.write_all(b"{\"pid\":24,").unwrap();
        producer.flush().unwrap();
        assert_eq!(residual_binding(&path, expected).unwrap().0, "json-partial");
        producer.write_all(b"\"temp\":\"C:/fixture/\xe4").unwrap();
        producer.flush().unwrap();
        assert_eq!(residual_binding(&path, expected).unwrap().0, "utf8-partial");
        producer
            .write_all(b"\xb8\xad/tmp\",\"receipt\":\"started-fixture-nonce\"}")
            .unwrap();
        producer.flush().unwrap();
        assert_eq!(
            residual_binding(&path, expected).unwrap().1.unwrap()["pid"],
            24
        );
        assert_eq!(
            residual_binding(&path, "started-another-fixture")
                .unwrap_err()
                .error
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        drop(producer);
        let inaccessible = root.join("not-a-file");
        drop(security::create_owner_only_directory(&inaccessible).unwrap());
        // The old source21 `if let Ok(open_identity(..))` swallowed this native
        // rejection exactly as a missing receipt until its 30-second timeout.
        assert!(security::open_identity(&inaccessible, false).is_err());
        assert!(residual_binding(&inaccessible, expected).is_err());
        let alias = root.join("binding-alias.json");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(residual_binding(&alias, expected).is_err());
        std::fs::remove_file(&alias).unwrap();
        let producer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        producer.set_len(4097).unwrap();
        assert_eq!(
            residual_binding(&path, expected).unwrap_err().stage,
            "size-bound"
        );
    }

    #[test]
    fn cancellation_waits_for_the_complete_native_receipt_after_file_creation() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-cancel-publication-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        drop(security::create_owner_only_directory(&root).unwrap());
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let path = root.join("cancel-started.txt");
        let expected = "started-publication-nonce";
        assert!(!started_receipt_ready(&path, expected).unwrap());
        drop(security::create_owner_only_file(&path).unwrap());
        let mut producer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        // This is the observed source20 race: is_file is true while the old
        // immediate read sees an empty string. No sleep/scheduler luck is needed.
        assert!(path.is_file());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(!started_receipt_ready(&path, expected).unwrap());
        producer.write_all(b"started-publication-").unwrap();
        producer.flush().unwrap();
        assert!(!started_receipt_ready(&path, expected).unwrap());
        producer.write_all(b"nonce\n").unwrap();
        producer.flush().unwrap();
        assert!(started_receipt_ready(&path, expected).unwrap());
        assert!(!started_receipt_ready(&path, "started-another-nonce").unwrap());
        drop(producer);
        assert!(started_receipt_ready(&path, expected).unwrap());
    }
}
