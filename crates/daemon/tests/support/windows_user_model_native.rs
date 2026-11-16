//! Ordinary pinned Pi / DSH invocations. No Worker profile, provider settings or
//! credential override enters argv/environment; selectors refer to saved models.
use super::{InstalledProduct, assert_no_secret, security};
use hiroute_platform::windows::job::{Child, Launch, Stream};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const BOUND: u64 = 4 * 1024 * 1024;
const NATIVE_BOUND: Duration = Duration::from_secs(120);

pub struct Contexts {
    pub pi: PathBuf,
    pub dsh: PathBuf,
    pub workspace: PathBuf,
    node: PathBuf,
    pi_cli: PathBuf,
    dsh_cli: PathBuf,
    variables: Vec<(OsString, OsString)>,
}
impl Contexts {
    pub fn prepare(product: &InstalledProduct) -> Self {
        let node = selected("HIROUTE_WORKER_NODE");
        super::super::windows_execution_host::image(&node);
        let pi_cli = selected("HIROUTE_WORKER_PI_BINARY");
        let dsh_cli = selected("HIROUTE_WORKER_DSH_BINARY");
        let mut directories = vec![node.parent().unwrap().to_owned()];
        for (entry, package, name) in [
            (&pi_cli, "@earendil-works/pi-coding-agent", "pi"),
            (&dsh_cli, "@deepseek-ai/dsh", "dsh"),
        ] {
            directories.push(declared_shim_directory(entry, package, name));
        }
        if let Some(path) = std::env::var_os("PATH") {
            directories.extend(std::env::split_paths(&path));
        }
        let root = product.root.join("ordinary native 用户 models");
        let pi = root.join("pi");
        let dsh = root.join("dsh");
        let workspace = root.join("workspace");
        for path in [&root, &pi, &dsh, &workspace] {
            security::create_owner_only_directory(path).unwrap();
        }
        // These are empty test-owned directories; only the production Agent
        // settings transaction may create or edit native configuration files.
        let profiles = dsh.join("profiles");
        let web = profiles.join("web");
        for path in [&profiles, &web] {
            security::create_owner_only_directory(path).unwrap();
        }
        let variables = vec![
            ("PI_CODING_AGENT_DIR".into(), pi.as_os_str().to_owned()),
            ("DSH_HOME".into(), dsh.as_os_str().to_owned()),
            ("PATH".into(), std::env::join_paths(directories).unwrap()),
        ];
        Self {
            pi,
            dsh,
            workspace,
            node,
            pi_cli,
            dsh_cli,
            variables,
        }
    }
    pub fn variables(&self) -> &[(OsString, OsString)] {
        &self.variables
    }
    fn environment(&self) -> Vec<(OsString, OsString)> {
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| {
                let name = name.to_string_lossy().to_ascii_uppercase();
                // Native clients and their user tools receive only OS/profile
                // selectors. In particular, no CI token, provider credentials,
                // proxy override or arbitrary inherited SDK setting is admitted.
                matches!(
                    name.as_str(),
                    "SYSTEMROOT"
                        | "WINDIR"
                        | "COMSPEC"
                        | "PATHEXT"
                        | "SYSTEMDRIVE"
                        | "USERPROFILE"
                        | "HOME"
                        | "HOMEDRIVE"
                        | "HOMEPATH"
                        | "APPDATA"
                        | "LOCALAPPDATA"
                        | "PROGRAMDATA"
                        | "PROGRAMFILES"
                        | "PROGRAMFILES(X86)"
                        | "COMMONPROGRAMFILES"
                        | "COMMONPROGRAMFILES(X86)"
                        | "TEMP"
                        | "TMP"
                        | "USERNAME"
                        | "USERDOMAIN"
                        | "COMPUTERNAME"
                )
            })
            .collect();
        for (name, value) in &self.variables {
            environment.retain(|(key, _)| {
                !key.as_encoded_bytes()
                    .eq_ignore_ascii_case(name.as_encoded_bytes())
            });
            environment.push((name.clone(), value.clone()));
        }
        environment
    }
    pub fn invoke(&self, pi: bool, provider: &str, alias: &str, prompt: &str, receipt: &str) {
        eprintln!(
            "Windows ordinary native checkpoint: harness={}, stage=invoke",
            if pi { "pi" } else { "dsh" }
        );
        if pi {
            let args: Vec<OsString> = [self.pi_cli.as_os_str().to_owned()]
                .into_iter()
                .chain(
                    [
                        "--provider",
                        provider,
                        "--model",
                        alias,
                        "--mode",
                        "json",
                        "--print",
                        "--no-session",
                        "--no-extensions",
                        "--offline",
                        "--tools",
                        "read,bash",
                        prompt,
                    ]
                    .map(OsString::from),
                )
                .collect();
            let output = run(
                &self.node,
                &args,
                &self.environment(),
                &self.workspace,
                None,
                NATIVE_BOUND,
            )
            .1;
            let rows: Vec<Value> = std::str::from_utf8(&output)
                .unwrap()
                .lines()
                .filter(|line| line.starts_with('{'))
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                rows.iter().filter(|row| row["type"] == "session").count(),
                1
            );
            let endings: Vec<_> = rows
                .iter()
                .filter(|row| row["type"] == "agent_end")
                .collect();
            assert_eq!(
                endings.len(),
                1,
                "ordinary Pi must publish one actual terminal"
            );
            let last = endings[0]["messages"].as_array().unwrap().last().unwrap();
            assert_eq!(last["role"], "assistant");
            assert_eq!(last["stopReason"], "stop");
            assert!(
                text(&last["content"]).trim() == receipt,
                "ordinary Pi lost the independent upstream receipt"
            );
        } else {
            let args = vec![
                self.dsh_cli.as_os_str().to_owned(),
                "--profile".into(),
                "acp".into(),
                "--patch".into(),
                self.dsh
                    .join("profiles/web/cordis.patch.yml")
                    .into_os_string(),
            ];
            let mut acp = Acp::spawn(&self.node, &args, &self.environment(), &self.workspace);
            let initialized = acp.request(
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{},
                "clientInfo":{"name":"hiroute-native-acceptance","version":"1"}}),
            );
            assert_eq!(initialized["protocolVersion"], 1);
            let session = acp.request("session/new", json!({"cwd":self.workspace,"mcpServers":[]}));
            let id = session["sessionId"].as_str().unwrap().to_owned();
            let option = session["configOptions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["category"] == "model" || option["id"] == "model")
                .unwrap();
            let selected = serde_json::to_string(&[provider, alias]).unwrap();
            let present = option["options"].as_array().unwrap().iter().any(|option| {
                option["value"] == selected
                    || option["options"]
                        .as_array()
                        .is_some_and(|group| group.iter().any(|option| option["value"] == selected))
            });
            assert!(
                present,
                "ordinary DSH must advertise the exact saved provider/alias"
            );
            acp.request(
                "session/set_config_option",
                json!({"sessionId":id,"configId":option["id"],"value":selected}),
            );
            acp.active = Some(id.clone());
            let result = acp.request(
                "session/prompt",
                json!({"sessionId":id,"prompt":[{"type":"text","text":prompt}]}),
            );
            assert_eq!(result["stopReason"], "end_turn");
            assert!(
                acp.text.trim() == receipt,
                "ordinary DSH lost the independent upstream receipt"
            );
            acp.stop();
        }
    }
}
fn selected(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).expect("required fixed native artifact input"));
    assert!(path.is_absolute());
    security::open_identity(&path, false).expect("selected native artifact must be reparse-free");
    path
}
pub(crate) fn declared_shim_directory(entry: &Path, package: &str, name: &str) -> PathBuf {
    let package_root = entry
        .ancestors()
        .take(6)
        .find(|root| {
            let path = root.join("package.json");
            path.is_file()
                && serde_json::from_slice::<Value>(&std::fs::read(path).unwrap())
                    .is_ok_and(|value| value["name"] == package)
        })
        .expect("fixed entry must belong to its actual npm producer");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(package_root.join("package.json")).unwrap()).unwrap();
    let declared = manifest["bin"]
        .as_str()
        .or_else(|| manifest["bin"][name].as_str())
        .unwrap();
    let actual = security::open_identity(entry, false).unwrap();
    let declared = security::open_identity(&package_root.join(declared), false).unwrap();
    assert_eq!(
        security::file_identity(&actual).unwrap(),
        security::file_identity(&declared).unwrap()
    );
    let modules = package_root
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "node_modules"))
        .unwrap();
    let directory = modules.join(".bin");
    security::open_identity(&directory.join(format!("{name}.cmd")), false).expect(
        "npm ci must provide the actual known-producer discovery shim; it is never executed",
    );
    directory
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessPhase {
    PublicPiCollaborationCheck,
    PublicDshCollaborationCheck,
    OrdinaryPiModel,
    Other,
}
fn process_phase(args: &[OsString]) -> ProcessPhase {
    let at = |index, expected| args.get(index).is_some_and(|value| value == expected);
    let collaboration = args
        .windows(2)
        .any(|pair| pair[0] == "--scope" && pair[1] == "collaboration");
    if at(0, "agents") && at(1, "check") && collaboration {
        if at(2, "agent_pi_default") {
            return ProcessPhase::PublicPiCollaborationCheck;
        }
        if at(2, "agent_dsh_default") {
            return ProcessPhase::PublicDshCollaborationCheck;
        }
    }
    if args
        .windows(2)
        .any(|pair| pair[0] == "--mode" && pair[1] == "json")
        && args.iter().any(|argument| argument == "--print")
    {
        ProcessPhase::OrdinaryPiModel
    } else {
        ProcessPhase::Other
    }
}

#[derive(Debug)]
struct PublicFailure {
    envelope_json: bool,
    status: Option<hiroute_application_api::MachineStatus>,
    code: Option<hiroute_application_api::ErrorCode>,
    collaboration_reason: Option<hiroute_application_api::AgentCollaborationCheckFailureReasonV1>,
}
fn public_failure(bytes: &[u8]) -> PublicFailure {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    PublicFailure {
        envelope_json: value.is_some(),
        status: value
            .as_ref()
            .and_then(|value| serde_json::from_value(value["status"].clone()).ok()),
        code: value
            .as_ref()
            .and_then(|value| serde_json::from_value(value["error"]["code"].clone()).ok()),
        // The production closed failure is in envelope.data. Unknown fields,
        // schemas and native messages never become diagnostic text.
        collaboration_reason: value.as_ref().and_then(|value| {
            serde_json::from_value::<hiroute_application_api::AgentCollaborationCheckFailureV1>(
                value["data"].clone(),
            )
            .ok()
            .filter(hiroute_application_api::AgentCollaborationCheckFailureV1::valid)
            .map(|failure| failure.reason)
        }),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum NodeFailure {
    Unknown,
    ModuleNotFound,
    PackageExportUnavailable,
    Ambiguous,
}
fn node_failure(bytes: &[u8]) -> NodeFailure {
    // Fixed Node 22.19.0 captions from lib/internal/errors.js and
    // lib/internal/modules/cjs/loader.js; do not print the interpolated path.
    let text = String::from_utf8_lossy(bytes);
    let module =
        text.contains("Error [ERR_MODULE_NOT_FOUND]:") || text.contains("code: 'MODULE_NOT_FOUND'");
    let exports = text.contains("Error [ERR_PACKAGE_PATH_NOT_EXPORTED]:");
    match (module, exports) {
        (true, false) => NodeFailure::ModuleNotFound,
        (false, true) => NodeFailure::PackageExportUnavailable,
        (true, true) => NodeFailure::Ambiguous,
        _ => NodeFailure::Unknown,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum PiSdkStage {
    Successful,
    Import,
    Interfaces,
    ExecutableCapability,
    Unknown,
}
fn pi_sdk_stage(exit: i32, expected_stdout: bool) -> PiSdkStage {
    // Exact closed exits from the production --check entry, never native stderr.
    match (exit, expected_stdout) {
        (0, true) => PiSdkStage::Successful,
        (21, _) => PiSdkStage::Import,
        (22, _) => PiSdkStage::Interfaces,
        (23, _) => PiSdkStage::ExecutableCapability,
        _ => PiSdkStage::Unknown,
    }
}

fn wait_diagnostic(child: &mut Child, deadline: Instant) -> std::io::Result<i32> {
    loop {
        if let Some(exit) = child.exit_code()?
            && child.empty()?
        {
            return Ok(exit);
        }
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn pi_sdk_after_failed_public_check(cwd: &Path, env: &[(OsString, OsString)]) {
    // This runs only AFTER the real public check failed. Even a successful
    // diagnostic cannot replace its failing exit/assertion or warm its first run.
    // Use the exact production public --check script, not a recreated SDK/mock.
    const SCRIPT: &str = include_str!("../../../integrations/src/agents/pi_sdk_contract.mjs");
    let result = (|| -> std::io::Result<()> {
        let selected = |name| -> std::io::Result<PathBuf> {
            let path = std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| std::io::Error::other("missing fixed diagnostic selector"))?;
            security::open_identity(&path, false)?;
            Ok(path)
        };
        let node = selected("HIROUTE_WORKER_NODE")?;
        let cli = selected("HIROUTE_WORKER_PI_BINARY")?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| std::io::Error::other("diagnostic nonce unavailable"))?;
        let name: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let root = cwd.join(format!(".hiroute-sdk-after-failure-{name}"));
        let directory = security::create_owner_only_directory(&root)?;
        let script = root.join("capability.mjs");
        // Track only objects created in this private stage. Cleanup runs even
        // when writing, spawning or observing the held Job fails.
        let mut writer = None;
        let mut source = None;
        let mut source_identity = None;
        let mut output = None;
        let diagnostic = (|| -> std::io::Result<()> {
            writer = Some(security::create_owner_only_file_at(
                &directory,
                std::ffi::OsStr::new("capability.mjs"),
            )?);
            let file = writer.as_mut().unwrap();
            source_identity = Some(security::file_identity(file)?);
            file.write_all(SCRIPT.as_bytes())?;
            file.sync_all()?;
            drop(writer.take()); // Node must read the complete private source.
            source = Some(security::open_identity_at(
                &directory,
                std::ffi::OsStr::new("capability.mjs"),
                false,
            )?);
            if Some(security::file_identity(source.as_ref().unwrap())?) != source_identity {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            output = Some(security::create_owner_only_file_at(
                &directory,
                std::ffi::OsStr::new("stdout"),
            )?);
            let output = output.as_mut().unwrap();
            let args = [
                script.into_os_string(),
                "--check".into(),
                cli.into_os_string(),
                "collaboration".into(),
            ];
            let mut environment: Vec<_> = env
                .iter()
                .filter(|(name, _)| {
                    matches!(
                        name.to_string_lossy().to_ascii_uppercase().as_str(),
                        "SYSTEMROOT"
                            | "WINDIR"
                            | "COMSPEC"
                            | "PATH"
                            | "PATHEXT"
                            | "SYSTEMDRIVE"
                            | "USERPROFILE"
                            | "HOME"
                            | "HOMEDRIVE"
                            | "HOMEPATH"
                            | "APPDATA"
                            | "LOCALAPPDATA"
                            | "PROGRAMDATA"
                            | "PROGRAMFILES"
                            | "PROGRAMFILES(X86)"
                            | "COMMONPROGRAMFILES"
                            | "COMMONPROGRAMFILES(X86)"
                            | "TEMP"
                            | "TMP"
                            | "PI_CODING_AGENT_DIR"
                    )
                })
                .cloned()
                .collect();
            environment.extend([
                ("PI_OFFLINE".into(), "1".into()),
                ("PI_TELEMETRY".into(), "0".into()),
            ]);
            let mut child = Child::spawn(Launch {
                program: &node,
                args: &args,
                env: &environment,
                cwd: Some(cwd),
                stdin: Stream::Null,
                stdout: Stream::File(output),
                stderr: Stream::Null,
            })?;
            let deadline = Instant::now() + Duration::from_secs(15);
            let outcome = (|| -> std::io::Result<i32> {
                loop {
                    if output.metadata()?.len() > 128 {
                        return Err(std::io::ErrorKind::InvalidData.into());
                    }
                    if let Some(exit) = child.exit_code()?
                        && child.empty()?
                    {
                        return Ok(exit);
                    }
                    if Instant::now() >= deadline {
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })();
            let bounded_success = outcome.is_ok();
            let deadline_expired = outcome
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::TimedOut);
            let exit = match &outcome {
                Ok(exit) => *exit,
                Err(_) => {
                    child.terminate()?;
                    wait_diagnostic(&mut child, Instant::now() + Duration::from_secs(10))?
                }
            };
            // The 15-second production stage covers natural root AND Job exit.
            // Forced cleanup is observed separately and never a successful SDK.
            output.rewind()?;
            let mut bytes = Vec::new();
            std::io::Read::by_ref(output)
                .take(129)
                .read_to_end(&mut bytes)?;
            let expected_stdout = bytes == b"hiroute.pi-sdk-capability/v1:ok\n";
            eprintln!(
                "Windows ordinary public-check after-failure SDK: stage={:?}, exit={exit}, expected-stdout={expected_stdout}, stdout-limit-exceeded={}, deadline-expired={deadline_expired}, natural-bounded-exit={bounded_success}, owned-job-empty=true",
                if bounded_success {
                    pi_sdk_stage(exit, expected_stdout)
                } else {
                    PiSdkStage::Unknown
                },
                bytes.len() > 128
            );
            outcome.map(|_| ())
        })();
        // The source reader denies delete; release it before reopening the
        // exact created identity relative to our retained private directory.
        // Never adopt a substituted object, recursively remove a path or turn
        // failed/forced stop evidence into a natural successful SDK result.
        drop(source.take());
        let source_cleanup = (|| -> std::io::Result<()> {
            if let Some(file) = writer.take() {
                return security::delete_by_handle(&file);
            }
            if let Some(expected) = source_identity {
                let file = security::open_delete_at(
                    &directory,
                    std::ffi::OsStr::new("capability.mjs"),
                    false,
                )?;
                if security::file_identity(&file)? != expected {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                security::delete_by_handle(&file)?;
            }
            Ok(())
        })();
        let output_cleanup = output
            .take()
            .map(|file| security::delete_by_handle(&file))
            .unwrap_or(Ok(()));
        let directory_cleanup = security::delete_by_handle(&directory);
        drop(directory);
        eprintln!(
            "Windows ordinary public-check after-failure SDK cleanup: source-deleted={}, output-deleted={}, directory-deleted={}",
            source_cleanup.is_ok(),
            output_cleanup.is_ok(),
            directory_cleanup.is_ok()
        );
        // Preserve the first operational failure while attempting every owned
        // deletion; only fixed IO kinds/codes reach the caller's diagnostic.
        diagnostic
            .and(source_cleanup)
            .and(output_cleanup)
            .and(directory_cleanup)
    })();
    if let Err(error) = result {
        eprintln!(
            "Windows ordinary public-check after-failure SDK: stage=unavailable, io-kind={:?}, win32={:?}",
            error.kind(),
            error.raw_os_error()
        );
    }
}

pub fn run(
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
    input: Option<&[u8]>,
    bound: Duration,
) -> (i32, Vec<u8>) {
    let phase = process_phase(args);
    let started = Instant::now();
    eprintln!("Windows ordinary process checkpoint: phase={phase:?}, stage=spawn");
    let mut child = Child::spawn(Launch {
        program,
        args,
        env,
        cwd: Some(cwd),
        stdin: if input.is_some() {
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
            assert!(bytes.len() as u64 <= BOUND);
            assert_no_secret(&bytes);
            bytes
        })
    };
    let output = capture(child.stdout.take().unwrap());
    let error = capture(child.stderr.take().unwrap());
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let deadline = Instant::now() + bound;
    let exit = wait(&mut child, deadline);
    let output = output.join().unwrap();
    let error = error.join().unwrap();
    // wait() has already observed the held process exit and an empty owned Job.
    // A public check's status/error is distinct from a native client's status.
    eprintln!(
        "Windows ordinary process checkpoint: phase={phase:?}, stage=exited, exit={exit}, exit-u32={}, elapsed-ms={}, stdout-bytes={}, stderr-bytes={}, owned-job-empty=true",
        exit as u32,
        started.elapsed().as_millis(),
        output.len(),
        error.len()
    );
    if exit != 0 {
        if matches!(
            phase,
            ProcessPhase::PublicPiCollaborationCheck | ProcessPhase::PublicDshCollaborationCheck
        ) {
            if let Some(root) = std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR") {
                // Raw native process output is private development evidence,
                // never production diagnostics or public test output.
                let retained = (|| -> std::io::Result<()> {
                    let root = PathBuf::from(root);
                    security::open_identity(&root, true)?;
                    for (kind, bytes) in [("stdout", &output), ("stderr", &error)] {
                        let path = root.join(format!("native-{phase:?}-{kind}.bin"));
                        let mut file = security::create_owner_only_file(&path)?;
                        file.write_all(bytes)?;
                        file.sync_all()?;
                    }
                    Ok(())
                })();
                if retained.is_err() {
                    eprintln!("Windows native failure output retention could not complete");
                }
            }
            let diagnostic = public_failure(&output);
            eprintln!(
                "Windows ordinary public check failure: phase={phase:?}, envelope-json={}, status={:?}, code={:?}, collaboration-reason={:?}",
                diagnostic.envelope_json,
                diagnostic.status,
                diagnostic.code,
                diagnostic.collaboration_reason
            );
        }
        eprintln!(
            "Windows ordinary vendor failure: phase={phase:?}, node={:?}",
            node_failure(&error)
        );
        if phase == ProcessPhase::PublicPiCollaborationCheck {
            pi_sdk_after_failed_public_check(cwd, env);
        }
    }
    assert_eq!(exit, 0, "actual native/public CLI process must succeed");
    (exit, output)
}
fn wait(child: &mut Child, deadline: Instant) -> i32 {
    let exit = loop {
        if let Some(exit) = child.exit_code().unwrap() {
            break exit;
        }
        if Instant::now() >= deadline {
            child.terminate().unwrap();
            panic!("owned native process exceeded its fixed deadline");
        }
        thread::sleep(Duration::from_millis(10));
    };
    while !child.empty().unwrap() {
        assert!(
            Instant::now() < deadline,
            "actual native descendant cleanup must complete"
        );
        thread::sleep(Duration::from_millis(10));
    }
    exit
}
struct Acp {
    child: Child,
    messages: Receiver<Value>,
    reader: Option<JoinHandle<()>>,
    error: Option<JoinHandle<()>>,
    deadline: Instant,
    id: u64,
    active: Option<String>,
    text: String,
}
impl Acp {
    fn spawn(program: &Path, args: &[OsString], env: &[(OsString, OsString)], cwd: &Path) -> Self {
        let mut child = Child::spawn(Launch {
            program,
            args,
            env,
            cwd: Some(cwd),
            stdin: Stream::Piped,
            stdout: Stream::Piped,
            stderr: Stream::Piped,
        })
        .unwrap();
        let (send, messages) = mpsc::channel();
        let output = child.stdout.take().unwrap();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                if reader
                    .by_ref()
                    .take(BOUND + 1)
                    .read_until(b'\n', &mut line)
                    .unwrap()
                    == 0
                {
                    break;
                }
                total += line.len();
                assert!(total as u64 <= BOUND, "ordinary ACP output exceeded bound");
                assert_no_secret(&line);
                let value = serde_json::from_slice(&line)
                    .expect("ordinary native ACP must emit actual JSON-RPC");
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let stderr = child.stderr.take().unwrap();
        let error = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.take(BOUND + 1).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() as u64 <= BOUND);
            assert_no_secret(&bytes);
        });
        Self {
            child,
            messages,
            reader: Some(reader),
            error: Some(error),
            deadline: Instant::now() + NATIVE_BOUND,
            id: 0,
            active: None,
            text: String::new(),
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        eprintln!("Windows ordinary native checkpoint: harness=dsh, method={method}");
        let input = json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params});
        let stdin = self.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &input).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        loop {
            let value = self
                .messages
                .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
                .expect("ordinary native ACP did not complete the selected public method");
            if value["method"] == "session/update"
                && self
                    .active
                    .as_ref()
                    .is_some_and(|id| value["params"]["sessionId"] == *id)
            {
                let update = &value["params"]["update"];
                if update["sessionUpdate"] == "agent_message_chunk" {
                    self.text.push_str(&text(&update["content"]));
                }
            }
            if value["id"] == self.id {
                assert!(
                    value.get("error").is_none(),
                    "ordinary ACP rejected the selected saved model/method"
                );
                return value["result"].clone();
            }
            assert!(
                !(value.get("id").is_some() && value.get("method").is_some()),
                "ordinary native unexpectedly requested a client capability"
            );
        }
    }
    fn stop(&mut self) {
        self.child.stdin.take();
        // ACP is a resident native protocol server. After its actual completed
        // prompt the consumer explicitly stops its owned Job, as the POSIX journey does.
        self.child.terminate().unwrap();
        wait(&mut self.child, Instant::now() + Duration::from_secs(10));
        self.reader.take().unwrap().join().unwrap();
        self.error.take().unwrap().join().unwrap();
    }
}
impl Drop for Acp {
    fn drop(&mut self) {
        if self.reader.is_some() {
            let _ = self.child.terminate();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stopped = false;
            while Instant::now() < deadline {
                if self.child.exit_code().is_ok_and(|value| value.is_some())
                    && self.child.empty().unwrap_or(false)
                {
                    stopped = true;
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            // Preserve the original red failure without an unbounded pipe join
            // if native owned-stop evidence is itself unavailable.
            if stopped {
                if let Some(reader) = self.reader.take() {
                    let _ = reader.join();
                }
                if let Some(error) = self.error.take() {
                    let _ = error.join();
                }
            }
        }
    }
}
pub fn text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(values) => values.iter().map(text).collect::<Vec<_>>().join(""),
        Value::Object(values) => values
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "text"
                        | "content"
                        | "input"
                        | "output"
                        | "messages"
                        | "instructions"
                        | "system"
                )
            })
            .map(|(_, value)| text(value))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn public_check_phase_is_distinct_from_ordinary_native_model_invocation() {
        for (agent, expected) in [
            ("agent_pi_default", ProcessPhase::PublicPiCollaborationCheck),
            (
                "agent_dsh_default",
                ProcessPhase::PublicDshCollaborationCheck,
            ),
        ] {
            let args = ["agents", "check", agent, "--scope", "collaboration"].map(OsString::from);
            assert_eq!(process_phase(&args), expected);
        }
        assert_eq!(
            process_phase(
                &[
                    "private-pi-cli",
                    "--mode",
                    "json",
                    "--print",
                    "private prompt"
                ]
                .map(OsString::from)
            ),
            ProcessPhase::OrdinaryPiModel
        );
        assert_eq!(
            process_phase(
                &["agents", "check", "agent_pi_default", "--scope", "live"].map(OsString::from)
            ),
            ProcessPhase::Other
        );
    }

    #[test]
    fn public_failure_exposes_typed_status_and_code_without_private_native_fields() {
        const PRIVATE: &str = "private-account-path-credential-never-emitted";
        let bytes = serde_json::to_vec(&json!({
            "status":"unavailable", "error":{"code":"OBSERVATION_UNAVAILABLE", "message_key":PRIVATE},
            "data":{"schema":"hiroute.agent-collaboration-check-failure/v1","reason":"dependency_unavailable"}
        })).unwrap();
        let diagnostic = public_failure(&bytes);
        assert!(diagnostic.envelope_json);
        assert_eq!(
            diagnostic.status,
            Some(hiroute_application_api::MachineStatus::Unavailable)
        );
        assert_eq!(
            diagnostic.code,
            Some(hiroute_application_api::ErrorCode::ObservationUnavailable)
        );
        assert_eq!(
            diagnostic.collaboration_reason,
            Some(hiroute_application_api::AgentCollaborationCheckFailureReasonV1::DependencyUnavailable)
        );
        assert!(!format!("{diagnostic:?}").contains(PRIVATE));
        for data in [
            json!({"schema":"unknown","reason":"dependency_unavailable"}),
            json!({"schema":"hiroute.agent-collaboration-check-failure/v1","reason":PRIVATE}),
            json!({"schema":"hiroute.agent-collaboration-check-failure/v1","reason":"dependency_unavailable","native_output":PRIVATE}),
        ] {
            let diagnostic = public_failure(
                &serde_json::to_vec(&json!({
                    "status":PRIVATE,"error":{"code":PRIVATE},"data":data,
                }))
                .unwrap(),
            );
            assert!(diagnostic.status.is_none() && diagnostic.code.is_none());
            assert!(diagnostic.collaboration_reason.is_none());
            assert!(!format!("{diagnostic:?}").contains(PRIVATE));
        }
    }

    #[test]
    fn vendor_caption_classification_never_exposes_native_paths_or_guesses_numbers() {
        const PRIVATE: &str = "private-account-path-command-1114-never-emitted";
        for (text, expected) in [
            (
                format!("Error [ERR_MODULE_NOT_FOUND]: {PRIVATE}"),
                NodeFailure::ModuleNotFound,
            ),
            (
                format!("Error: {PRIVATE}\n code: 'MODULE_NOT_FOUND'"),
                NodeFailure::ModuleNotFound,
            ),
            (
                format!("Error [ERR_PACKAGE_PATH_NOT_EXPORTED]: {PRIVATE}"),
                NodeFailure::PackageExportUnavailable,
            ),
            (
                format!(
                    "Error [ERR_MODULE_NOT_FOUND]: {PRIVATE}\n Error [ERR_PACKAGE_PATH_NOT_EXPORTED]: {PRIVATE}"
                ),
                NodeFailure::Ambiguous,
            ),
            (
                format!("{PRIVATE} error 5 permission issue timeout"),
                NodeFailure::Unknown,
            ),
        ] {
            let diagnostic = node_failure(text.as_bytes());
            assert_eq!(diagnostic, expected);
            assert!(!format!("{diagnostic:?}").contains(PRIVATE));
        }
    }

    #[test]
    fn sdk_after_failure_requires_the_exact_success_receipt_and_preserves_closed_stages() {
        assert_eq!(pi_sdk_stage(0, true), PiSdkStage::Successful);
        assert_eq!(pi_sdk_stage(0, false), PiSdkStage::Unknown);
        for (exit, expected) in [
            (21, PiSdkStage::Import),
            (22, PiSdkStage::Interfaces),
            (23, PiSdkStage::ExecutableCapability),
            (6, PiSdkStage::Unknown),
        ] {
            assert_eq!(pi_sdk_stage(exit, false), expected);
        }
    }
}
