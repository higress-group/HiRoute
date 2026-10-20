//! Optional Linux desktop presentation. Routing policy and service lifecycle keep their owners.
use super::*;
use hiroute_host_runtime::desktop_integration::omarchy::{
    self as host, Context, PublicPluginControl,
};
use hiroute_host_runtime::{ConsoleListenerDesiredV1, ConsoleListenerMode, ConsoleListenerStore};
use std::{
    ffi::OsString,
    io::{IsTerminal, Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};
mod notifications;
pub(crate) use notifications::worker as notifications_worker;
mod reads;
mod summary;

pub(super) fn execute(arguments: &[String], globals: &Globals) -> CliExecution {
    if globals.help {
        return CliExecution {
            exit_code: 0,
            stdout: HELP.into(),
            stderr: String::new(),
        };
    }
    if globals.agent.is_some() || globals.has_capability() {
        return failure(ErrorCode::CapabilityDenied, globals.request_id.clone());
    }
    match command(arguments, globals) {
        Ok(value) => success(value, globals.request_id.clone()),
        Err((code, detail)) => {
            let mut result = failure(code, globals.request_id.clone());
            result.stderr = format!("{detail}\n");
            result
        }
    }
}
const HELP: &str = "Usage\n  hiroute integrations omarchy enable [--console yes|no] [--notifications on|off]\n  hiroute integrations omarchy status\n  hiroute integrations omarchy disable\n  hiroute integrations omarchy configure --notifications on|off\n  hiroute integrations omarchy refresh\n  hiroute integrations omarchy launch --agent codex|claude-code -- <AGENT_ARGS...>\n  hiroute integrations omarchy open-console [--landing first-run|suggestion:<id>|agent:<agent>]\n  hiroute integrations omarchy summary\n  hiroute integrations omarchy notify\n\nAn optional current-user standalone Linux integration. Enable requires OMARCHY_PATH and the public omarchy command. It never starts, stops or restarts HiRoute. Native Agent settings remain owned by their existing configuration flows. On launch, the exact -- preserves native argv; only a caller in HOME moves to an existing ~/Work. A failed routed preflight queues one notification and enters agent launch --native-fallback prompt. No unattended native fallback is permitted. Console initialization is explicit and loopback-only; a saved listener may need a service restart. Launch links are fresh, single-use and never printed by this family. summary is the read-only hiroute.omarchy-summary/v1 indicator view: it reports service unavailable, and exits 0, whenever a current figure cannot be read.\n";
type Failure = (ErrorCode, String);
fn host_error(error: host::Error) -> Failure {
    (ErrorCode::ActionRequired, error.to_string())
}
fn invalid() -> Failure {
    (
        ErrorCode::InvalidArguments,
        "Run hiroute integrations omarchy --help.".into(),
    )
}
fn value(value: impl serde::Serialize) -> Result<serde_json::Value, Failure> {
    serde_json::to_value(value).map_err(|_| {
        (
            ErrorCode::Internal,
            "Integration result could not be encoded.".into(),
        )
    })
}
fn assets(context: &Context) -> Result<Vec<host::Asset>, Failure> {
    context
        .assets_from(
            &context
                .home
                .join(".local/share/hiroute")
                .join(&context.release)
                .join("integrations/omarchy"),
        )
        .map_err(host_error)
}
fn console_store() -> Result<ConsoleListenerStore, Failure> {
    let layout = StandaloneLayout::from_environment().map_err(|_| {
        (
            ErrorCode::ActionRequired,
            "A standalone installation is required.".into(),
        )
    })?;
    Ok(ConsoleListenerStore::new(layout.gateway_config_root()))
}
fn command(args: &[String], globals: &Globals) -> Result<serde_json::Value, Failure> {
    validate(args)?;
    let Some(verb) = args.first() else {
        return Err(invalid());
    };
    let context = Context::from_environment(verb == "enable").map_err(host_error)?;
    let control = PublicPluginControl {
        executable: context.omarchy_command.clone(),
    };
    match args {
        [verb, options @ ..] if verb == "enable" => {
            let (console, notifications) = enable_options(options)?;
            let store = console_store()?;
            let current = store.read().map_err(|_| {
                (
                    ErrorCode::ActionRequired,
                    "Console configuration is unreadable.".into(),
                )
            })?;
            let consent = if current.desired.mode == ConsoleListenerMode::Off {
                match console {
                    Some(value) => value,
                    None => ask_console()?,
                }
            } else {
                false
            };
            // Preflight all optional files before either integration or console effects.
            let owned = assets(&context)?;
            context.preflight_assets(&owned).map_err(host_error)?;
            let manifest = context
                .enable(
                    &owned,
                    &control,
                    notifications,
                    false,
                    Duration::from_secs(5),
                )
                .map_err(host_error)?;
            let mut console_error = None;
            if consent
                && store
                    .configure(ConsoleListenerDesiredV1::loopback())
                    .is_err()
            {
                console_error = Some("console_configuration_unavailable");
            }
            let console = console_status(&store, globals)?;
            Ok(
                json!({"enabled":manifest.enabled,"files":context.status(&owned).map_err(host_error)?,"console":console,"console_error":console_error,"first_run_pending":!manifest.first_run_opened}),
            )
        }
        [verb] if verb == "refresh" => {
            let owned = assets(&context)?;
            context
                .enable(&owned, &control, None, true, Duration::from_secs(5))
                .map_err(host_error)?;
            value(context.status(&owned).map_err(host_error)?)
        }
        [verb] if verb == "disable" => value(context.disable(&control).map_err(host_error)?),
        [verb, flag, choice] if verb == "configure" && flag == "--notifications" => {
            let enabled = on_off(choice)?;
            context
                .configure_notifications(enabled)
                .map_err(host_error)?;
            if !enabled {
                context.take_notifications(false, &[]).map_err(host_error)?;
            }
            Ok(json!({"notifications":enabled}))
        }
        [verb] if verb == "status" => {
            let owned = assets(&context)?;
            let query = reads::Query::new(globals.timeout.unwrap_or(Duration::from_secs(5)))?;
            let (connections, agents_unavailable) = match reads::connections(&query) {
                Ok(value) => (value, false),
                Err(_) => (Vec::new(), true),
            };
            let checks: Vec<_> = connections
                .iter()
                .map(|c| reads::check(&context.stable_cli(), &c.agent, query.remaining()))
                .collect();
            Ok(
                json!({"files":context.status(&owned).map_err(host_error)?,"agents":checks,"agents_unavailable":agents_unavailable,"console":console_status(&console_store()?, globals)?,"account_boundary":host::ACCOUNT_BOUNDARY}),
            )
        }
        [verb, options @ ..] if verb == "open-console" => {
            let landing = match options {
                [] => None,
                [flag, hint]
                    if flag == "--landing"
                        && hiroute_application_api::valid_console_landing(hint) =>
                {
                    Some(hint.clone())
                }
                _ => return Err(invalid()),
            };
            match open_console(&context, landing, globals) {
                Ok(()) => Ok(json!({"opened":true})),
                Err(error) => {
                    let _ = context.queue_console_failure("console_unavailable");
                    let _ = notifications::notify(&context, globals);
                    Err(error)
                }
            }
        }
        [verb] if verb == "summary" => value(summary::read(&context, globals)),
        [verb] if verb == "notify" => value(notifications::notify(&context, globals)?),
        _ => Err(invalid()),
    }
}
fn validate(args: &[String]) -> Result<(), Failure> {
    match args {
        [verb, options @ ..] if verb == "enable" => enable_options(options).map(|_| ()),
        [verb]
            if matches!(
                verb.as_str(),
                "status" | "disable" | "refresh" | "summary" | "notify"
            ) =>
        {
            Ok(())
        }
        [verb, flag, choice] if verb == "configure" && flag == "--notifications" => {
            on_off(choice).map(|_| ())
        }
        [verb] if verb == "open-console" => Ok(()),
        [verb, flag, hint]
            if verb == "open-console"
                && flag == "--landing"
                && hiroute_application_api::valid_console_landing(hint) =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

fn enable_options(options: &[String]) -> Result<(Option<bool>, Option<bool>), Failure> {
    let (mut console, mut notifications) = (None, None);
    let mut it = options.iter();
    while let Some(flag) = it.next() {
        let choice = it.next().ok_or_else(invalid)?;
        match flag.as_str() {
            "--console" if console.is_none() => {
                console = Some(match choice.as_str() {
                    "yes" => true,
                    "no" => false,
                    _ => return Err(invalid()),
                })
            }
            "--notifications" if notifications.is_none() => notifications = Some(on_off(choice)?),
            _ => return Err(invalid()),
        }
    }
    Ok((console, notifications))
}
fn on_off(value: &str) -> Result<bool, Failure> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(invalid()),
    }
}
fn ask_console() -> Result<bool, Failure> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err((ErrorCode::ActionRequired, "Console access is off. Choose --console yes or --console no; enable does not restart the service.".into()));
    }
    eprint!("Configure a loopback-only HiRoute console? This does not restart the service. [y/N] ");
    std::io::stderr().flush().map_err(|_| invalid())?;
    let mut input = [0; 4];
    let n = std::io::stdin().read(&mut input).map_err(|_| invalid())?;
    Ok(matches!(&input[..n], b"y\n" | b"y\r\n"))
}
fn console_status(
    store: &ConsoleListenerStore,
    globals: &Globals,
) -> Result<serde_json::Value, Failure> {
    let config = store.read().map_err(|_| {
        (
            ErrorCode::ActionRequired,
            "Console configuration is unreadable.".into(),
        )
    })?;
    let configured = config.desired.mode != ConsoleListenerMode::Off;
    // The applied address alone can outlive a stopped daemon. Require its owner to be ready.
    let query = reads::Query::new(
        globals
            .timeout
            .unwrap_or(Duration::from_secs(1))
            .min(Duration::from_secs(1)),
    );
    let ready = query.ok().is_some_and(|q| reads::service_ready(&q));
    let listening = ready
        && config.applied_address.is_some_and(|address| {
            address.ip().is_loopback()
                && std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100))
                    .is_ok()
        });
    Ok(
        json!({"configured":configured,"listening":listening,"restart_required":configured && !listening,"loopback_only":config.desired.mode==ConsoleListenerMode::Loopback}),
    )
}
fn open_console(
    context: &Context,
    requested: Option<String>,
    globals: &Globals,
) -> Result<(), Failure> {
    let manifest = context.load().map_err(host_error)?.ok_or_else(invalid)?;
    let hint = requested.or_else(|| (!manifest.first_run_opened).then(|| "first-run".into()));
    let program = host::find_executable("omarchy-launch-webapp")
        .or_else(|| host::find_executable("xdg-open"))
        .ok_or((
            ErrorCode::ActionRequired,
            "No console browser launcher is available.".into(),
        ))?;
    let query = reads::Query::new(globals.timeout.unwrap_or(Duration::from_secs(5)))?;
    let result = query.call(
        "CreateConsoleLaunch",
        json!({"loopback_only":true,"landing":hint}),
    )?;
    let url = result
        .get("url")
        .and_then(serde_json::Value::as_str)
        .filter(|url| host::launch::valid_loopback_launch_url(url))
        .ok_or((
            ErrorCode::ActionRequired,
            "The loopback console is not listening.".into(),
        ))?;
    // Select a fallback before minting. Once a launcher starts, never retry the URL.
    Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            (
                ErrorCode::ActionRequired,
                "The console browser launcher could not start.".into(),
            )
        })?;
    if hint.as_deref() == Some("first-run") {
        context.mark_first_run_opened().map_err(host_error)?;
    }
    Ok(())
}

pub(crate) fn launch(arguments: &[OsString]) -> CliExecution {
    let parsed = launch_arguments(arguments);
    let (agent, tail) = match parsed {
        Ok(value) => value,
        Err(error) => return failure(error, None),
    };
    let context = match Context::from_environment(false) {
        Ok(value) => value,
        Err(error) => {
            let mut result = failure(ErrorCode::ActionRequired, None);
            result.stderr = format!("{error}\n");
            return result;
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(value) => value,
        Err(_) => return failure(ErrorCode::ActionRequired, None),
    };
    let working = host::launch::working_directory(&context.home, &cwd);
    let check = reads::check_at(
        &context.stable_cli(),
        &agent,
        &tail,
        Duration::from_secs(5),
        Some(&working),
    );
    if !check.launchable {
        if let Err(error) = context.queue_launch_failure(
            &agent,
            check
                .check_reason_code
                .as_deref()
                .unwrap_or("launch_unavailable"),
        ) {
            // Keep the routed launch's existing refusal/fallback behavior, but
            // do not hide a failed durable notification enqueue from its terminal.
            eprintln!("HiRoute could not queue the launch notification: {error}.");
        }
        let _ = notifications::notify(&context, &Globals::default());
    }
    use std::os::unix::process::CommandExt;
    let error = Command::new(context.stable_cli())
        .current_dir(working)
        .args([
            "agent",
            "launch",
            "--agent",
            &agent,
            "--native-fallback",
            "prompt",
            "--",
        ])
        .args(tail)
        .exec();
    let mut result = failure(ErrorCode::ActionRequired, None);
    result.stderr = format!(
        "The routed launcher could not start ({:?}).\n",
        error.kind()
    );
    result
}
fn launch_arguments(arguments: &[OsString]) -> Result<(String, Vec<OsString>), ErrorCode> {
    match arguments {
        [flag, agent, delimiter, tail @ ..]
            if flag == "--agent"
                && delimiter == "--"
                && matches!(agent.to_str(), Some("codex" | "claude-code")) =>
        {
            Ok((agent.to_string_lossy().into_owned(), tail.to_vec()))
        }
        _ => Err(ErrorCode::InvalidArguments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omarchy_native_tail_and_owner_options_are_not_generic_globals() {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![0xff, b'x']);
        let (agent, tail) = launch_arguments(&[
            "--agent".into(),
            "codex".into(),
            "--".into(),
            "--output".into(),
            raw.clone(),
        ])
        .unwrap();
        assert_eq!(agent, "codex");
        assert_eq!(tail, [OsString::from("--output"), raw]);
        assert!(launch_arguments(&["--agent".into(), "other".into(), "--".into()]).is_err());
        assert_eq!(
            enable_options(&[
                "--console".into(),
                "no".into(),
                "--notifications".into(),
                "off".into()
            ])
            .unwrap(),
            (Some(false), Some(false))
        );
        assert!(
            enable_options(&[
                "--console".into(),
                "yes".into(),
                "--console".into(),
                "no".into()
            ])
            .is_err()
        );
    }
}
