//! The controller and all owner commands use a real logged-on standard account.
//! No privilege enabling, alternate logon, linked/restricted token or impersonation
//! belongs to this gate. Console closure, RDP reconnect and login triggers are
//! separate externally observed phases; toggling autostart does not prove login.
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::thread;
use std::time::{Duration, Instant};

use hiroute_host_runtime::{StandaloneInstallRecordV1, StandaloneLayout};
use hiroute_platform::windows::{background, lifecycle};
use serde_json::Value;
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, GetLengthSid, IsValidSid, LookupAccountNameW, SID_NAME_USE,
    SidTypeUser, TOKEN_GROUPS, TokenGroups, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_INFO_CLASS, WTSActive, WTSConnectState, WTSDisconnected,
    WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
};

use super::{
    Inputs, fixture, windows_account as account, windows_gateway_recovery,
    windows_management_service, windows_subscription_authorization, windows_subscription_login,
};

/// Invoke the existing owner oracle as ourselves, using normal CreateProcessW.
/// This deliberately avoids fixture::owner and its alternate-token/staged path.
pub(super) fn current_owner(request: fixture::Request, bound: Duration) -> Value {
    let stage = match request.action.as_str() {
        "layout" => "layout",
        "command" => "command",
        "install" | "uninstall" => "maintenance",
        "inspect" => "inspect",
        "crash" => "crash",
        "forced-stop" => "forced-stop",
        "transaction-snapshot" | "transaction-verify" => "upgrade-snapshot",
        "transaction-fail-install" | "transaction-upgrade-install" => "upgrade-install",
        "upgrade-business-save" => "upgrade-business-save",
        "upgrade-business-verify" => "upgrade-business-verify",
        "upgrade-business-cleanup" => "upgrade-business-cleanup",
        "cleanup-state" => "preserve-state",
        _ => panic!("unsupported standard-owner fixture action"),
    };
    let body = serde_json::to_vec(&request).expect("standard-owner request encoding failed");
    let result = fixture::process(
        None,
        &std::env::current_exe().expect("standard-owner executable is unavailable"),
        &[
            "--exact".into(),
            fixture::ENTRY.into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ],
        Some(&body),
        false,
        bound,
        true,
    );
    if result["code"] != 0 || !result["result"].is_object() {
        let output = result["stdout"].as_str().unwrap_or_default();
        let error = result["stderr"].as_str().unwrap_or_default();
        let marker = "HIROUTE_NATIVE_BACKGROUND_RESULT=";
        let failure = if error.contains("oracle requires a bounded content-length request") {
            "oracle-rejected-transfer-encoding"
        } else if error.contains("native exit timestamp must precede") {
            "forced-stop-missed-native-exit-bound"
        } else if error.contains("forced user stop must terminate") {
            "forced-stop-runtime-remained"
        } else if error.contains("forced stop returned an unexpected public exit") {
            "forced-stop-unexpected-public-exit"
        } else {
            "unclassified"
        };
        // Keep the panic location useful without forwarding native settings,
        // provider text or a private grant from the nested process's stderr.
        let (origin, line) = [
            ("windows_background_upgrade_business.rs", "upgrade-business"),
            ("windows_background_upgrade_io.rs", "upgrade-io"),
            ("windows_background_upgrade_oracle.rs", "upgrade-oracle"),
            ("windows_business_http.rs", "business-http"),
        ]
        .into_iter()
        .find_map(|(file, label)| {
            error.lines().find_map(|text| {
                let (_, location) = text.split_once(file)?;
                let line = location
                    .strip_prefix(':')?
                    .split(':')
                    .next()?
                    .parse::<u32>()
                    .ok()?;
                Some((label, line))
            })
        })
        .unwrap_or(("unknown", 0));
        let (http_phase, http_status) = error
            .lines()
            .find_map(|text| {
                let text = text.strip_prefix("upgrade Gateway HTTP facts: phase=")?;
                let (phase, status) = text.split_once(", status=")?;
                let phase = phase.parse::<u8>().ok()?;
                let status = status.parse::<u16>().ok()?;
                (phase < 3 && (100..600).contains(&status)).then_some((phase, status))
            })
            .unwrap_or((0, 0));
        let public_stage = [
            "restore-preview",
            "restore-apply",
            "connect-status",
            "connect-change",
            "operation-read",
            "compute",
            "routing",
            "receipts",
            "other",
        ]
        .into_iter()
        .find(|stage| error.contains(&format!("upgrade public CLI failure: stage={stage}, ")))
        .unwrap_or("none");
        eprintln!(
            "standard-owner frame facts: stage={stage}, public_stage={public_stage}, failure={failure}, origin={origin}, line={line}, http_phase={http_phase}, http_status={http_status}, stdout_bytes={}, stderr_bytes={}, marker_seen={}, exact_line_count={}",
            output.len(),
            error.len(),
            output.contains(marker),
            output
                .lines()
                .filter(|line| line.starts_with(marker))
                .count(),
        );
    }
    assert!(
        result["code"] == 0 && result["result"].is_object(),
        "standard-owner oracle failed: stage={stage}, exit={:?}, result_present={}",
        result["code"].as_i64(),
        result["result"].is_object()
    );
    result["result"].clone()
}

fn action(name: &str) -> Value {
    current_owner(
        fixture::Request {
            action: name.into(),
            program: None,
            arguments: vec![],
        },
        Duration::from_secs(30),
    )
}

fn command(program: &Path, arguments: Vec<String>, bound: Duration) -> Value {
    current_owner(
        fixture::Request {
            action: "command".into(),
            program: Some(program.to_owned()),
            arguments,
        },
        bound,
    )
}

fn accepted(result: Value, stage: &'static str) -> Value {
    assert!(
        result["code"] == 0 && result["result"].is_object(),
        "standard-owner production command failed: stage={stage}, exit={:?}, result_present={}, error_present={}",
        result["code"].as_i64(),
        result["result"].is_object(),
        result["result"]["error"].is_object()
    );
    result["result"].clone()
}

fn cli(program: &Path, arguments: &[&str], stage: &'static str) -> Value {
    let envelope = accepted(
        command(
            program,
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .chain([
                    "--output".into(),
                    "json".into(),
                    "--timeout".into(),
                    "15s".into(),
                ])
                .collect(),
            Duration::from_secs(30),
        ),
        stage,
    );
    assert!(
        envelope["status"] == "succeeded" && envelope["data"].is_object(),
        "standard-owner CLI envelope failed: stage={stage}, data_present={}, error_present={}",
        envelope["data"].is_object(),
        envelope["error"].is_object()
    );
    envelope
}

/// Invoke the real public CLI with our unchanged standard token. Requests and
/// authorization URLs remain bounded in memory; failure diagnostics never
/// interpolate an envelope, URL, input, provider text or native configuration.
fn public_call(program: &Path, arguments: &[&str], request: Option<&Value>) -> (i64, Value) {
    // Only fixed command labels enter diagnostics, never request or OAuth bytes.
    let stage = match arguments {
        ["compute", "scan", ..] => "scan",
        ["compute", "list", ..] => "compute-list",
        ["compute", "connection", "options", ..] => "connection-options",
        ["compute", "connection", "authorize", ..] => "authorization",
        ["compute", "connection", "login", ..] => "subscription-login",
        ["decision", "services", "apply", ..] => "decision-apply",
        ["service", "restart", ..] => "service-restart",
        ["gateway", ..] => "gateway-recovery",
        _ => "other",
    };
    eprintln!("standard background public command: stage={stage}");
    let mut args: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    if request.is_some() && !arguments.contains(&"--request-stdin") {
        args.push("--request-stdin".into());
    }
    if !arguments.contains(&"--output") {
        args.extend(["--output".into(), "json".into()]);
    }
    if !arguments.contains(&"--timeout") {
        args.extend(["--timeout".into(), "45s".into()]);
    }
    let input = request.map(|value| serde_json::to_vec(value).unwrap());
    let result = fixture::process(
        None,
        program,
        &args,
        input.as_deref(),
        false,
        Duration::from_secs(60),
        false,
    );
    let exit = result["code"]
        .as_i64()
        .expect("public CLI returned no process exit");
    if stage == "connection-options" {
        eprintln!(
            "standard background catalog response: stdout_bytes={}",
            result["stdout"].as_str().unwrap_or_default().len()
        );
    }
    assert!(
        result["result"].is_object(),
        "public CLI returned no parsed envelope"
    );
    (exit, result["result"].clone())
}

fn public_cli(program: &Path, arguments: &[&str], request: Option<&Value>) -> Value {
    let (exit, envelope) = public_call(program, arguments, request);
    assert!(
        exit == 0 && envelope["status"] == "succeeded" && envelope["data"].is_object(),
        "standard-owner public subscription/lifecycle command failed"
    );
    envelope
}

fn status(program: &Path, active: bool) -> Value {
    let envelope = cli(program, &["service", "status"], "status");
    let data = &envelope["data"];
    assert!(
        data["schema"] == "hiroute.standalone-service-status/v1"
            && data["manager"] == "windows-task-user"
            && data["manager_active"] == active
            && data["local_control_ready"] == active,
        "standard-owner status must report the actual Task and Control state"
    );
    data.clone()
}

fn session_bytes(session: u32, class: WTS_INFO_CLASS) -> Vec<u8> {
    let (mut buffer, mut size) = (null_mut(), 0);
    let result = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            class,
            &mut buffer,
            &mut size,
        )
    };
    assert_ne!(result, 0, "standard-owner WTS query failed");
    assert!(!buffer.is_null() && (1..=2048).contains(&size));
    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) }.to_vec();
    unsafe { WTSFreeMemory(buffer.cast()) };
    bytes
}

fn session_name(session: u32, class: WTS_INFO_CLASS) -> String {
    let bytes = session_bytes(session, class);
    assert_eq!(bytes.len() % 2, 0, "WTS name must contain UTF-16 units");
    let units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(units.last(), Some(&0), "WTS name must be terminated");
    assert!(!units[..units.len() - 1].contains(&0));
    String::from_utf16(&units[..units.len() - 1]).expect("WTS name is invalid UTF-16")
}

fn no_administrator_group(token: &std::fs::File) {
    let mut admin = [0_usize; 9];
    let mut size = std::mem::size_of_val(&admin) as u32;
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admin.as_mut_ptr().cast(),
                &mut size,
            )
        },
        0,
        "standard-owner group identity is unavailable"
    );
    let groups = account::information(token, TokenGroups);
    let groups = unsafe { &*groups.as_ptr().cast::<TOKEN_GROUPS>() };
    for group in
        unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize) }
    {
        assert_eq!(
            unsafe { EqualSid(group.Sid, admin.as_mut_ptr().cast()) },
            0,
            "pure standard owner must have no Administrators SID, including deny-only"
        );
    }
}

pub(super) fn real_standard_session() -> (String, u32) {
    let token = account::process_token(std::process::id());
    let sid = account::token_sid(&token);
    let sid_text = background::sid_string(&sid).expect("current native SID is invalid");
    let session = account::assert_standard(&token, &sid_text);
    no_administrator_group(&token);
    assert_ne!(
        session, 0,
        "Task owner must have a real interactive session"
    );
    let state = session_bytes(session, WTSConnectState);
    assert_eq!(state.len(), 4, "WTS session state must be a native DWORD");
    let state = u32::from_ne_bytes(state.try_into().unwrap());
    assert!(
        state == WTSActive as u32 || state == WTSDisconnected as u32,
        "standard-owner WTS session is not a logged-on interactive session"
    );
    let domain = session_name(session, WTSDomainName);
    let user = session_name(session, WTSUserName);
    assert!(
        !domain.is_empty() && !user.is_empty(),
        "WTS owner identity is absent"
    );
    let name = account::wide(&format!("{domain}\\{user}"));
    let mut native_sid = [0_usize; 9];
    let mut sid_size = std::mem::size_of_val(&native_sid) as u32;
    let mut native_domain = [0_u16; 512];
    let mut domain_size = native_domain.len() as u32;
    let mut use_kind: SID_NAME_USE = 0;
    assert_ne!(
        unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                name.as_ptr(),
                native_sid.as_mut_ptr().cast(),
                &mut sid_size,
                native_domain.as_mut_ptr(),
                &mut domain_size,
                &mut use_kind,
            )
        },
        0,
        "WTS owner cannot be resolved to its actual SID"
    );
    // LookupAccountNameW only specifies the required cbSid on insufficient
    // capacity; success can leave our original capacity (72) unchanged. Validate
    // the actual SID before asking GetLengthSid, then bound that native length.
    // https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-lookupaccountnamew
    assert!(
        sid_size as usize <= std::mem::size_of_val(&native_sid) && use_kind == SidTypeUser,
        "WTS owner lookup returned an invalid capacity or account kind"
    );
    let resolved_sid = native_sid.as_ptr().cast_mut().cast();
    assert_ne!(
        unsafe { IsValidSid(resolved_sid) },
        0,
        "WTS owner lookup did not produce a valid native SID"
    );
    let actual_size = unsafe { GetLengthSid(resolved_sid) };
    assert!(
        actual_size != 0
            && actual_size as usize <= std::mem::size_of_val(&native_sid)
            && unsafe { EqualSid(resolved_sid, sid.as_ptr().cast_mut().cast()) } != 0,
        "actual WTS owner SID differs from the controller token"
    );
    println!(
        "standard Task prerequisite: actual_user=true, elevation_zero=true, medium=true, administrators_absent=true, own_wts_session=true"
    );
    (sid_text, session)
}

fn assert_identity(actual: &Value, sid: &str, session: u32) {
    assert!(
        actual["sid"].as_str() == Some(sid)
            && actual["session"].as_u64() == Some(u64::from(session)),
        "actual Task daemon must retain the standard owner and its real session"
    );
}

pub(super) fn require_absent(path: &Path, stage: &'static str) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "standard Task absence check failed: stage={stage}, native_error={:?}",
            error.raw_os_error()
        ),
        Ok(_) => panic!("standard Task gate refuses an existing object: stage={stage}"),
    }
}

struct Cleanup<'a> {
    input: &'a Inputs,
    layout: StandaloneLayout,
    record: StandaloneInstallRecordV1,
    cli: PathBuf,
    armed: bool,
}
impl Cleanup<'_> {
    fn uninstall(&self) -> Value {
        assert!(
            hiroute_host_runtime::read_standalone_install_record(&self.layout.marker_path)
                .ok()
                .as_ref()
                == Some(&self.record),
            "standard Task cleanup refuses a changed or missing candidate marker"
        );
        accepted(
            current_owner(
                fixture::Request {
                    action: "uninstall".into(),
                    program: Some(self.input.admin_cli.clone()),
                    arguments: vec![],
                },
                Duration::from_secs(90),
            ),
            "uninstall",
        )
    }
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if hiroute_host_runtime::read_standalone_install_record(&self.layout.marker_path)
            .ok()
            .as_ref()
            != Some(&self.record)
        {
            eprintln!(
                "standard Task cleanup preserved state: exact installed marker no longer matches"
            );
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.uninstall())).is_err() {
            eprintln!(
                "standard Task exact-candidate uninstall failed; preserve evidence and user state"
            );
        }
    }
}

pub(super) fn task_absent(name: String, stage: &'static str) {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::BSTR;
    let observation = thread::spawn(move || -> Result<bool, u32> {
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|error| error.code().0 as u32)?;
            let _apartment = Apartment;
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| error.code().0 as u32)?;
            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(|error| error.code().0 as u32)?;
            let folder = service
                .GetFolder(&BSTR::from("\\"))
                .map_err(|error| error.code().0 as u32)?;
            match folder.GetTask(&BSTR::from(name)) {
                Ok(_) => Ok(false),
                Err(error) if error.code().0 as u32 == 0x80070002 => Ok(true),
                Err(error) => Err(error.code().0 as u32),
            }
        }
    })
    .join()
    .expect("read-only Task observer panicked");
    assert!(
        observation == Ok(true),
        "actual Task absence is unconfirmed: stage={stage}, observation={observation:?}"
    );
}

#[test]
fn task_real_standard_logged_on_owner_installs_runs_recovers_and_uninstalls() {
    let (sid, session) = real_standard_session();
    let input = Inputs::load();
    let layout =
        StandaloneLayout::from_environment().expect("actual owner KnownFolders unavailable");
    require_absent(&layout.marker_path, "initial-marker");
    require_absent(
        &layout.data_root.join("service/windows-background.json"),
        "initial-definition",
    );
    let initial = action("layout");
    assert!(
        initial["home"].as_str().map(Path::new) == Some(layout.home.as_path())
            && initial["state"].as_str().map(Path::new) == Some(layout.state_root.as_path())
            && initial["sid"].as_str() == Some(sid.as_str()),
        "owner reexecution must keep the actual profile/SID"
    );
    for name in ["hiroute.exe", "hirouted.exe"] {
        require_absent(
            &layout.home.join(".local/bin").join(name),
            "initial-stable-entry",
        );
    }
    // The name is the production SID-derived namespace, independently of which
    // verified candidate image supplies the read-only constructor. Refuse an
    // orphan registration as well as an existing marker before any install.
    let fresh_name = background::Definition::new(
        background::Manager::Task,
        input.admin_cli.with_file_name("hirouted.exe"),
    )
    .expect("candidate Task namespace is unavailable")
    .name;
    task_absent(fresh_name.clone(), "initial-task");
    let installed = accepted(
        current_owner(input.install_request("install"), Duration::from_secs(90)),
        "install",
    );
    assert!(
        installed["installed"] == true
            && installed["started"] == false
            && installed["manager"] == "task",
        "actual standard install must register demand-only Task"
    );
    let cli_path = input.owner_cli(&initial);
    let record = hiroute_host_runtime::read_standalone_install_record(&layout.marker_path).unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(&input.manifest).unwrap()).unwrap();
    assert!(
        record.install_root == cli_path.parent().unwrap()
            && record.target == hiroute_platform::windows::install::native_target().unwrap()
            && Some(record.version.as_str()) == manifest["version"].as_str()
            && record.service_definition
                == layout.data_root.join("service/windows-background.json")
            && record.cpa_binary.as_deref()
                == Some(
                    record
                        .install_root
                        .join("libexec/cliproxyapi.exe")
                        .as_path()
                )
            && record.cpa_sha256.as_deref() == manifest["cpa"]["sha256"].as_str(),
        "installed marker must belong to the exact selected candidate root"
    );
    let definition = background::read_definition(&record.service_definition).unwrap();
    assert!(
        definition.manager == background::Manager::Task
            && definition.name == fresh_name
            && definition.owner_sid == sid
            && definition.daemon == record.install_root.join("hirouted.exe"),
        "actual registered Task definition must belong to this standard owner"
    );
    assert_eq!(
        definition.arguments(),
        "--role all --standalone",
        "Task startup cannot request the foreground parent console"
    );
    assert_gui_daemon(&definition.daemon);
    let mut cleanup = Cleanup {
        input: &input,
        layout,
        record,
        cli: cli_path,
        armed: true,
    };
    let cli_path = &cleanup.cli;
    assert!(status(cli_path, false)["autostart_enabled"] == false);
    assert!(
        cli(
            cli_path,
            &["service", "autostart", "enable"],
            "autostart-enable"
        )["data"]["autostart_enabled"]
            == true
    );
    assert!(
        status(cli_path, false)["autostart_enabled"] == true,
        "login policy may not demand-start the stopped Task"
    );
    assert!(
        cli(
            cli_path,
            &["service", "autostart", "disable"],
            "autostart-disable"
        )["data"]["autostart_enabled"]
            == false
    );
    assert!(status(cli_path, false)["autostart_enabled"] == false);
    cli(cli_path, &["service", "start"], "start");
    assert!(
        hiroute_platform::windows::task::active(&definition).unwrap(),
        "the actual registered Task action retains its exact background arguments"
    );
    status(cli_path, true);
    let first = action("inspect");
    assert_identity(&first, &sid, session);
    let held = lifecycle::retain_for_stop(
        &cleanup.layout.lifecycle_endpoint(),
        &definition.daemon,
        Duration::from_secs(5),
    )
    .unwrap();
    let token = account::process_token(held.pid);
    no_administrator_group(&token);
    assert!(
        !held.exited().unwrap(),
        "daemon must remain alive during native token inspection"
    );
    cli(cli_path, &["service", "start"], "repeat-start");
    let repeated = action("inspect");
    assert!(
        repeated["pid"] == first["pid"] && repeated["creation_time"] == first["creation_time"],
        "repeated start must retain one actual daemon"
    );
    windows_management_service::run(&mut |args| cli(cli_path, args, "management"));

    // The same public recovery oracle now executes through our real current
    // token. Desired B stays occupied while the native Task recovers applied A.
    let recovered = windows_gateway_recovery::exercise_with(
        &mut |arguments| public_call(cli_path, arguments, None),
        &mut || action("inspect"),
        &first,
        &sid,
        session,
    );
    assert_identity(&recovered, &sid, session);
    assert!(
        held.wait(Duration::from_secs(5)).unwrap(),
        "Gateway recovery must end the retained original daemon"
    );
    status(cli_path, true);
    let restart_held = lifecycle::retain_for_stop(
        &cleanup.layout.lifecycle_endpoint(),
        &definition.daemon,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(recovered["pid"].as_u64(), Some(u64::from(restart_held.pid)));
    assert_eq!(
        recovered["creation_time"].as_u64(),
        Some(restart_held.creation_time)
    );
    no_administrator_group(&account::process_token(restart_held.pid));
    assert!(!restart_held.exited().unwrap());
    windows_subscription_authorization::exercise(&mut |arguments, request| {
        public_call(cli_path, arguments, request)
    });
    let pending = windows_subscription_login::run_before_restart(&mut |arguments, request| {
        public_cli(cli_path, arguments, request)
    });
    public_cli(cli_path, &["service", "restart"], None);
    assert!(
        restart_held.wait(Duration::from_secs(5)).unwrap(),
        "public restart must end its held exact daemon"
    );
    status(cli_path, true);
    let restarted = action("inspect");
    assert_identity(&restarted, &sid, session);
    assert!(
        restarted["pid"] != recovered["pid"]
            || restarted["creation_time"] != recovered["creation_time"],
        "public service restart must produce a new authenticated daemon"
    );
    windows_subscription_login::assert_after_restart(
        &mut |arguments, request| public_cli(cli_path, arguments, request),
        pending,
    );
    // The following original graceful-stop assertion must retain the current
    // daemon, rather than reuse a handle that recovery already made signalled.
    let held = lifecycle::retain_for_stop(
        &cleanup.layout.lifecycle_endpoint(),
        &definition.daemon,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(restarted["pid"].as_u64(), Some(u64::from(held.pid)));
    assert_eq!(
        restarted["creation_time"].as_u64(),
        Some(held.creation_time)
    );
    no_administrator_group(&account::process_token(held.pid));
    assert!(!held.exited().unwrap());
    cli(cli_path, &["service", "stop"], "graceful-stop");
    assert!(
        held.wait(Duration::from_secs(5)).unwrap(),
        "graceful stop must end the retained real daemon"
    );
    status(cli_path, false);
    cli(cli_path, &["service", "start"], "start-before-crash");
    let crashed = action("crash");
    assert_identity(&crashed, &sid, session);
    let deadline = Instant::now() + Duration::from_secs(115);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "real Task PT1M crash recovery did not finish within its gate bound"
        );
        let result = accepted(
            command(
                cli_path,
                vec![
                    "service".into(),
                    "status".into(),
                    "--output".into(),
                    "json".into(),
                    "--timeout".into(),
                    "5s".into(),
                ],
                remaining.min(Duration::from_secs(15)),
            ),
            "recovery-status",
        );
        assert!(
            result["status"] == "succeeded",
            "recovery status must retain a valid public result"
        );
        if result["data"]["manager_active"] == true && result["data"]["local_control_ready"] == true
        {
            let recovered = action("inspect");
            assert_identity(&recovered, &sid, session);
            assert!(
                recovered["pid"] != crashed["pid"]
                    || recovered["creation_time"] != crashed["creation_time"],
                "unexpected failure must create a new actual daemon"
            );
            status(cli_path, true);
            break;
        }
        thread::sleep(
            Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    action("forced-stop");
    status(cli_path, false);
    thread::sleep(Duration::from_secs(65));
    status(cli_path, false);
    let uninstalled = cleanup.uninstall();
    assert!(
        uninstalled["uninstalled"] == true && uninstalled["state_preserved"] == true,
        "uninstall must succeed while preserving user state"
    );
    cleanup.armed = false;
    require_absent(&cleanup.layout.marker_path, "uninstalled-marker");
    require_absent(&cleanup.record.service_definition, "uninstalled-definition");
    for name in ["hiroute.exe", "hirouted.exe"] {
        require_absent(
            &cleanup.layout.home.join(".local/bin").join(name),
            "uninstalled-stable-entry",
        );
    }
    task_absent(definition.name, "uninstalled-task");
    println!(
        "actual standard Task gate: own logged-on token, no Administrators SID, demand-only install, toggle policy, Gateway/Control, doctor/logs, occupied-port recovery, public restart, real packaged CPA pending/cancel/forget/expiry and authorization rejection boundaries, graceful stop, PT1M recovery, forced stop remains stopped, exact uninstall and preserved user state; valid subscription authorization and external console/RDP/logon phases not executed here"
    );
}

/// The real installed image is already SHA-bound to the production package.
/// Read its bounded held PE header rather than infer subsystem from a filename.
fn assert_gui_daemon(path: &Path) {
    let mut file = hiroute_platform::windows::security::open_identity(path, false).unwrap();
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).unwrap();
    assert_eq!(&dos[..2], b"MZ");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    assert!(offset <= 1024 * 1024 && offset + 94 <= file.metadata().unwrap().len());
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut pe = [0u8; 94];
    file.read_exact(&mut pe).unwrap();
    assert_eq!(&pe[..4], b"PE\0\0");
    assert!(u16::from_le_bytes(pe[20..22].try_into().unwrap()) >= 70);
    assert_eq!(
        u16::from_le_bytes(pe[4..6].try_into().unwrap()),
        hiroute_host_runtime::standalone_install::windows_machine(
            hiroute_platform::windows::install::native_target().unwrap()
        )
        .unwrap()
    );
    assert_eq!(u16::from_le_bytes(pe[24..26].try_into().unwrap()), 0x20b);
    assert_eq!(
        u16::from_le_bytes(pe[92..94].try_into().unwrap()),
        2,
        "the actual Task daemon is a GUI-subsystem image"
    );
}
