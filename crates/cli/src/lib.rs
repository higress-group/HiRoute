#![forbid(unsafe_code)]

//! Production `hiroute` presentation/client layer.
//!
//! Released product commands cross owner-only Local Control. This crate owns only parameter
//! parsing, output, and exit codes and depends on no Application implementation or adapter.

mod args;
mod client;
mod control_plane;
mod host_commands;
mod managed_launch;
mod observation;
mod service;
mod worker;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use args::{Globals, split_globals};
pub use client::{LocalControlClient, LocalControlClientError};
use hiroute_application_api::{
    AgentCheckRequestV1, AgentCheckScopeV1, AgentCheckSuiteV1, AgentLaunchDescriptorRequestV1,
    CanonicalDigest, CommandDescriptorV1, CommandLifecycle, ErrorCode, ErrorV1, FallbackPolicyV1,
    FreePoolModeV1, HIDDEN_AGENT_GRANT_HELPER_VERB_V1, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlWireRequestV2, MachineEnvelopeV2, ManagedClaudeLaunchDescriptorV2,
    OperationCancelRequestV1, OperationLookupV1, PrincipalKind, ProtectedClientGrantV2,
    RoutingModeV1, SetupApplyRequestV1, SetupRequestV1, SetupSelectionV1, command_by_id,
    descriptor_digest, released_commands,
};
use serde::Serialize;
use serde_json::{Value, json};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliExecution {
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

pub fn execute<I, S>(arguments: I) -> CliExecution
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let arguments = arguments
        .into_iter()
        .map(Into::into)
        .collect::<Vec<OsString>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "agent")
        && arguments
            .get(1)
            .is_some_and(|argument| argument == "launch")
    {
        if arguments.len() == 3
            && arguments
                .get(2)
                .is_some_and(|argument| argument == "--help" || argument == "-h")
        {
            let descriptor = command_by_id("agent.launch")
                .expect("released managed-launch descriptor is registered");
            return CliExecution {
                exit_code: 0,
                stdout: descriptor.help_document(),
                stderr: String::new(),
            };
        }
        return execute_managed_launch(&arguments[2..]);
    }
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| ErrorCode::InvalidArguments)
        })
        .collect::<Result<Vec<_>, _>>();
    let Ok(arguments) = arguments else {
        return failure(ErrorCode::InvalidArguments, None);
    };
    let (path_and_options, globals) = match split_globals(arguments) {
        Ok(split) => split,
        Err(error) if error.worker => {
            return worker::input_failure(error.code, error.globals);
        }
        Err(error) => return failure(error.code, error.globals.request_id),
    };
    if path_and_options.is_empty() {
        return if globals.help {
            CliExecution {
                exit_code: 0,
                stdout: root_help(),
                stderr: String::new(),
            }
        } else {
            failure(ErrorCode::InvalidArguments, globals.request_id)
        };
    }
    if let Some(execution) = host_commands::execute(&path_and_options, &globals) {
        return execution;
    }
    let Some((descriptor, option_start)) = resolve_callable(&path_and_options) else {
        return failure(ErrorCode::UnknownCommand, globals.request_id);
    };
    if globals.help {
        return CliExecution {
            exit_code: 0,
            stdout: descriptor.help_document(),
            stderr: String::new(),
        };
    }
    let options = &path_and_options[option_start..];
    if descriptor.command_id.starts_with("worker.") {
        return worker::execute(&descriptor.command_id, options, globals);
    }
    if globals.output != args::OutputMode::Json {
        return failure(ErrorCode::InvalidArguments, globals.request_id);
    }
    match descriptor.command_id.as_str() {
        "schema.list" => execute_schema_list(options, globals.request_id),
        "schema.show" => execute_schema_show(options, globals.request_id),
        _ => execute_control(descriptor, options, globals),
    }
}

fn execute_managed_launch(arguments: &[OsString]) -> CliExecution {
    let invocation = match managed_launch::parse_invocation(arguments) {
        Ok(invocation) => invocation,
        Err(managed_launch::ManagedLaunchFailure::AuthPrecedenceConflict) => {
            return failure(ErrorCode::AgentAuthPrecedenceConflict, None);
        }
        Err(_) => return failure(ErrorCode::InvalidArguments, None),
    };
    let request_id = next_request_id();
    let payload = serde_json::to_value(AgentLaunchDescriptorRequestV1 {
        connection_id: invocation.connection_id.clone(),
    })
    .expect("managed-launch request is serializable");
    let request = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: request_id.clone(),
        operation_id: "GetManagedAgentLaunchDescriptor".to_owned(),
        payload,
        protected_grant: None,
    };
    let response =
        match LocalControlClient::cli_from_environment().and_then(|client| client.call(request)) {
            Ok(response) => response,
            Err(_) => return failure(ErrorCode::DaemonUnavailable, Some(request_id)),
        };
    if response.status != hiroute_application_api::MachineStatus::Succeeded {
        return render(response);
    }
    let Some(data) = response.data.as_ref() else {
        return failure(ErrorCode::DaemonUnavailable, Some(request_id));
    };
    let Ok(descriptor) = serde_json::from_value::<ManagedClaudeLaunchDescriptorV2>(data.clone())
    else {
        return failure(ErrorCode::DaemonUnavailable, Some(request_id));
    };
    if descriptor.connection_id != invocation.connection_id {
        return failure(ErrorCode::DaemonUnavailable, Some(request_id));
    }
    let trusted_hiroute_executable = match current_executable() {
        Ok(executable) => executable,
        Err(()) => return failure(ErrorCode::DaemonUnavailable, Some(request_id)),
    };
    let exit_code =
        match managed_launch::launch(&descriptor, &invocation, &trusted_hiroute_executable) {
            Ok(exit_code) => exit_code,
            Err(managed_launch::ManagedLaunchFailure::AuthPrecedenceConflict) => {
                return failure(ErrorCode::AgentAuthPrecedenceConflict, Some(request_id));
            }
            Err(_) => return failure(ErrorCode::DaemonUnavailable, Some(request_id)),
        };
    let warned = response
        .warnings
        .iter()
        .any(|warning| warning.code == "MANAGED_LAUNCH_ENV_SANITIZED")
        || descriptor
            .environment_removals
            .iter()
            .any(|name| std::env::var_os(name).is_some());
    CliExecution {
        exit_code,
        stdout: String::new(),
        stderr: if warned {
            "MANAGED_LAUNCH_ENV_SANITIZED\n".to_owned()
        } else {
            String::new()
        },
    }
}

fn current_executable() -> Result<String, ()> {
    std::fs::canonicalize(std::env::current_exe().map_err(|_| ())?)
        .map_err(|_| ())?
        .into_os_string()
        .into_string()
        .map_err(|_| ())
}

fn execute_control(
    descriptor: CommandDescriptorV1,
    options: &[String],
    globals: Globals,
) -> CliExecution {
    let request_id = globals.request_id.unwrap_or_else(next_request_id);
    if globals.agent.is_some()
        && descriptor.command_id != "work-plans.list"
        && !descriptor.command_id.starts_with("tasks.")
    {
        return failure(ErrorCode::InvalidArguments, Some(request_id));
    }
    if globals.capability_fd.is_some()
        && !descriptor
            .stdin_channels
            .iter()
            .any(|channel| channel == "capability_fd")
        && !control_plane::accepts_capability(&descriptor.command_id)
    {
        return failure(ErrorCode::InvalidArguments, Some(request_id));
    }
    let payload = match command_payload(&descriptor.command_id, options) {
        Ok(payload) => payload,
        Err(code) => return failure(code, Some(request_id)),
    };
    let client = match LocalControlClient::cli_from_environment().map(|client| {
        client.with_timeout(
            globals
                .timeout
                .unwrap_or_else(|| std::time::Duration::from_secs(30)),
        )
    }) {
        Ok(client) => client,
        Err(_) => return failure(ErrorCode::DaemonUnavailable, Some(request_id)),
    };
    let mutation_capability = match globals.capability_fd {
        Some(fd) => match if descriptor.command_id == "work-plans.list"
            || descriptor.command_id.starts_with("tasks.")
        {
            work_plans::credential(fd)
        } else {
            client::read_protected_fd(fd)
        } {
            Ok(value) => Some(value),
            Err(_) => return failure(ErrorCode::CapabilityDenied, Some(request_id)),
        },
        None => {
            if let Some(agent) = globals.agent.as_deref() {
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                match home.and_then(|home| {
                    hiroute_integrations::collaboration_artifact::read_sealed_collaboration(
                        &home, agent,
                    )
                    .ok()
                }) {
                    Some(mut material) => Some(std::mem::take(&mut *material)),
                    None => return failure(ErrorCode::CapabilityDenied, Some(request_id)),
                }
            } else {
                None
            }
        }
    };
    let principal_kind = if globals.agent.is_some() {
        PrincipalKind::SealedCollaboration
    } else if descriptor.command_id == "work-plans.list"
        || descriptor.command_id.starts_with("tasks.")
    {
        PrincipalKind::Skill
    } else {
        PrincipalKind::InteractiveUser
    };
    let request = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: request_id.clone(),
        operation_id: descriptor.operation_id,
        payload,
        protected_grant: mutation_capability.map(|capability| ProtectedClientGrantV2 {
            principal_kind,
            capability,
        }),
    };
    let response = client.call(request).unwrap_or_else(|_| {
        MachineEnvelopeV2::failed(ErrorV1::new(ErrorCode::DaemonUnavailable), Some(request_id))
    });
    render(response)
}

fn command_payload(command_id: &str, options: &[String]) -> Result<Value, ErrorCode> {
    match command_id {
        "system.status"
        | "system.client-status"
        | "agents.scan"
        | "agents.list"
        | "sessions.status" => empty(options),
        "routing.list" if options.is_empty() => empty(options),
        "routing.show" => match options {
            [id] => Ok(json!({"agent_plan_id": id})),
            _ => Err(ErrorCode::InvalidArguments),
        },
        "work-plans.list" => work_plans::payload(options),
        task if task.starts_with("tasks.") => tasks::payload(command_id, options),
        "operations.find" => client_lookup(options),
        "setup.preview" => setup_preview(options),
        "setup.apply" => setup_apply(options),
        "setup.status" | "operations.get" => operation(options, false),
        "operations.watch" => operation(options, true),
        "operations.cancel" => operation_cancel(options),
        "agents.check" => agent_check(options),
        "sessions.list" => observation::sessions_list(options),
        "sessions.show" => observation::session_show(options),
        "sessions.receipt" => observation::session_receipt(options),
        "value.show" => observation::value_show(options),
        "observation.plan-quality.samples" => observation::plan_quality_samples(options),
        _ => control_plane::command_payload(command_id, options)
            .unwrap_or(Err(ErrorCode::UnknownCommand)),
    }
}

fn client_lookup(options: &[String]) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    let mut input = String::new();
    std::io::stdin()
        .take(65_537)
        .read_to_string(&mut input)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if input.len() > 65_536 {
        return Err(ErrorCode::InvalidArguments);
    }
    let value: hiroute_application_api::OperationIdempotencyLookupV1 =
        serde_json::from_str(&input).map_err(|_| ErrorCode::InvalidArguments)?;
    serde_json::to_value(value).map_err(|_| ErrorCode::InvalidArguments)
}

fn empty(options: &[String]) -> Result<Value, ErrorCode> {
    options
        .is_empty()
        .then(|| json!({}))
        .ok_or(ErrorCode::InvalidArguments)
}

fn setup_preview(options: &[String]) -> Result<Value, ErrorCode> {
    if options == ["--spec-stdin"] {
        return read_json(std::io::stdin());
    }
    let mut spec = SetupRequestV1::default();
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--agent" => spec.agent_ids.push(next(options, &mut index)?),
            "--routing-mode" => {
                spec.routing_mode = match next(options, &mut index)?.as_str() {
                    "smart-saving" => RoutingModeV1::SmartSaving,
                    "free-first" => RoutingModeV1::FreeFirst,
                    "custom" => RoutingModeV1::Custom,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--routing-purpose" => spec.routing_purpose = Some(next(options, &mut index)?),
            "--free-pool-mode" => {
                spec.free_pool_mode = match next(options, &mut index)?.as_str() {
                    "automatic_all_available" => FreePoolModeV1::AutomaticAllAvailable,
                    "manual" => FreePoolModeV1::Manual,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--fallback-policy" => {
                spec.fallback_policy = match next(options, &mut index)?.as_str() {
                    "free_only" => FallbackPolicyV1::FreeOnly,
                    "primary_fallback" => FallbackPolicyV1::PrimaryFallback,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--native-subagent-routing" => {
                spec.native_subagent_routing = selection(&next(options, &mut index)?)?
            }
            "--codex-catalog" => spec.codex_catalog = selection(&next(options, &mut index)?)?,
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    serde_json::to_value(spec).map_err(|_| ErrorCode::Internal)
}

fn setup_apply(options: &[String]) -> Result<Value, ErrorCode> {
    let mut spec = None;
    let mut digest = None;
    let mut revision = None;
    let mut idempotency_key = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--spec-fd" => {
                let fd = next(options, &mut index)?
                    .parse::<u32>()
                    .map_err(|_| ErrorCode::InvalidArguments)?;
                spec = Some(read_json(
                    std::fs::File::open(format!("/dev/fd/{fd}"))
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )?);
            }
            "--accept-digest" => {
                digest = Some(
                    CanonicalDigest::parse(next(options, &mut index)?)
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--expected-revision" => {
                revision = Some(
                    next(options, &mut index)?
                        .parse::<u64>()
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--idempotency-key" => idempotency_key = Some(next(options, &mut index)?),
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    serde_json::to_value(SetupApplyRequestV1 {
        spec: serde_json::from_value(spec.ok_or(ErrorCode::InvalidArguments)?)
            .map_err(|_| ErrorCode::InvalidArguments)?,
        accept_digest: digest.ok_or(ErrorCode::InvalidArguments)?,
        expected_revision: revision.ok_or(ErrorCode::InvalidArguments)?,
        idempotency_key: idempotency_key
            .filter(|value| !value.is_empty())
            .ok_or(ErrorCode::InvalidArguments)?,
    })
    .map_err(|_| ErrorCode::Internal)
}

fn operation(options: &[String], watch: bool) -> Result<Value, ErrorCode> {
    let Some(operation_id) = options.first().filter(|value| !value.is_empty()) else {
        return Err(ErrorCode::InvalidArguments);
    };
    let mut after_sequence = 0;
    if options.len() > 1 {
        if !watch || options.len() != 3 || options[1] != "--after-sequence" {
            return Err(ErrorCode::InvalidArguments);
        }
        after_sequence = options[2]
            .parse()
            .map_err(|_| ErrorCode::InvalidArguments)?;
    }
    serde_json::to_value(OperationLookupV1 {
        operation_id: operation_id.clone(),
        after_sequence,
    })
    .map_err(|_| ErrorCode::Internal)
}

fn operation_cancel(options: &[String]) -> Result<Value, ErrorCode> {
    let Some(operation_id) = options.first().filter(|value| !value.is_empty()) else {
        return Err(ErrorCode::InvalidArguments);
    };
    if options.len() != 3 || options[1] != "--idempotency-key" || options[2].is_empty() {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(OperationCancelRequestV1 {
        operation_id: operation_id.clone(),
        idempotency_key: options[2].clone(),
    })
    .map_err(|_| ErrorCode::Internal)
}

fn agent_check(options: &[String]) -> Result<Value, ErrorCode> {
    let Some(agent_id) = options.first().filter(|value| !value.is_empty()) else {
        return Err(ErrorCode::InvalidArguments);
    };
    let mut request = AgentCheckRequestV1 {
        agent_id: agent_id.clone(),
        scope: AgentCheckScopeV1::Configuration,
        suite: AgentCheckSuiteV1::Quick,
        allow_model_call: false,
        target: None,
    };
    let mut index = 1;
    while index < options.len() {
        match options[index].as_str() {
            "--scope" => {
                request.scope = match next(options, &mut index)?.as_str() {
                    "configuration" => AgentCheckScopeV1::Configuration,
                    "live" => AgentCheckScopeV1::Live,
                    "native-authentication" => AgentCheckScopeV1::NativeAuthentication,
                    "collaboration" => AgentCheckScopeV1::Collaboration,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--suite" => {
                request.suite = match next(options, &mut index)?.as_str() {
                    "quick" => AgentCheckSuiteV1::Quick,
                    "tool" => AgentCheckSuiteV1::Tool,
                    "conformance" => AgentCheckSuiteV1::Conformance,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--allow-model-call" => request.allow_model_call = true,
            "--target" => {
                if request.target.is_some() {
                    return Err(ErrorCode::InvalidArguments);
                }
                request.target = Some(
                    serde_json::from_str(&next(options, &mut index)?)
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                );
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    if !request.valid_target() {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

fn selection(value: &str) -> Result<SetupSelectionV1, ErrorCode> {
    match value {
        "auto" => Ok(SetupSelectionV1::Automatic),
        "on" => Ok(SetupSelectionV1::Enabled),
        "off" => Ok(SetupSelectionV1::Disabled),
        _ => Err(ErrorCode::InvalidArguments),
    }
}

fn next(options: &[String], index: &mut usize) -> Result<String, ErrorCode> {
    *index += 1;
    options
        .get(*index)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or(ErrorCode::InvalidArguments)
}

fn read_json(reader: impl Read) -> Result<Value, ErrorCode> {
    let mut bytes = Vec::new();
    reader
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.len() > 1024 * 1024 {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidArguments)
}

fn resolve_callable(arguments: &[String]) -> Option<(CommandDescriptorV1, usize)> {
    released_commands()
        .into_iter()
        .filter_map(|descriptor| {
            let matches = arguments.len() >= descriptor.path.len()
                && arguments
                    .iter()
                    .zip(&descriptor.path)
                    .all(|(actual, expected)| actual == expected);
            matches.then_some((descriptor.clone(), descriptor.path.len()))
        })
        .max_by_key(|(_, length)| *length)
}

fn execute_schema_list(options: &[String], request_id: Option<String>) -> CliExecution {
    if !options.is_empty() {
        return failure(ErrorCode::InvalidArguments, request_id);
    }
    success(
        json!({
            "descriptor_digest": descriptor_digest(),
            "commands": released_commands().into_iter().map(|command| json!({
                "command_id": command.command_id,
                "path": command.path,
                "operation_id": command.operation_id,
            })).collect::<Vec<_>>(),
        }),
        request_id,
    )
}

fn execute_schema_show(options: &[String], request_id: Option<String>) -> CliExecution {
    let [flag, command_id] = options else {
        return failure(ErrorCode::InvalidArguments, request_id);
    };
    if flag != "--command-id" || command_id.is_empty() {
        return failure(ErrorCode::InvalidArguments, request_id);
    }
    let Some(descriptor) = command_by_id(command_id)
        .filter(|descriptor| descriptor.lifecycle == CommandLifecycle::Released)
    else {
        return failure(ErrorCode::ResourceNotFound, request_id);
    };
    success(descriptor, request_id)
}

fn success<T: Serialize>(data: T, request_id: Option<String>) -> CliExecution {
    render(MachineEnvelopeV2::succeeded(
        serde_json::to_value(data).expect("CLI data is serializable"),
        request_id,
    ))
}

fn failure(code: ErrorCode, request_id: Option<String>) -> CliExecution {
    render(MachineEnvelopeV2::failed(ErrorV1::new(code), request_id))
}

const DAEMON_UNAVAILABLE_HINT: &str = "HiRoute 服务不可用。Standalone 请运行 `hiroute service status`，并在需要时运行 `hiroute service start`；Desktop 请启动或恢复应用。隔离实例请检查运行目录。不会自动启动或重放请求。\n";

fn render(envelope: MachineEnvelopeV2<Value>) -> CliExecution {
    let stderr = if envelope
        .error
        .as_ref()
        .is_some_and(|error| error.code == ErrorCode::DaemonUnavailable)
    {
        DAEMON_UNAVAILABLE_HINT.to_owned()
    } else {
        String::new()
    };
    let mut stdout = serde_json::to_string(&envelope).expect("machine envelope is serializable");
    stdout.push('\n');
    CliExecution {
        exit_code: envelope.status.exit_code(),
        stdout,
        stderr,
    }
}

fn next_request_id() -> String {
    format!(
        "req_{}_{}",
        std::process::id(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn root_help() -> String {
    let mut families = released_commands()
        .into_iter()
        .map(|descriptor| descriptor.path[0].clone())
        .collect::<Vec<_>>();
    families.extend(["gateway".into(), "protected-input".into(), "service".into()]);
    families.sort();
    families.dedup();
    let commands = families
        .into_iter()
        .map(|family| format!("  {family}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "HiRoute headless product CLI\n\nUsage\n  hiroute <command> [options]\n\nPublic command families\n{commands}\n\nHost management: run 'hiroute service --help', 'hiroute gateway --help', or 'hiroute protected-input --help'.\nApplication/Local Control: run 'hiroute schema list --output json' and 'hiroute schema show --command-id <ID> --output json', then append '--help' to a complete command path, for example 'hiroute worker dependencies discover --help'.\n"
    )
}

pub fn main_entry() -> u8 {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == [OsString::from("service"), OsString::from("run")] {
        return service::foreground_run();
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == HIDDEN_AGENT_GRANT_HELPER_VERB_V1)
    {
        return hidden_agent_grant_helper(&arguments[1..]);
    }
    let execution = execute(arguments);
    let _ = std::io::stdout().write_all(execution.stdout.as_bytes());
    let _ = std::io::stderr().write_all(execution.stderr.as_bytes());
    execution.exit_code
}

fn hidden_agent_grant_helper(arguments: &[OsString]) -> u8 {
    let [connection_id] = arguments else {
        let _ = std::io::stderr().write_all(b"hiroute agent grant request invalid\n");
        return 2;
    };
    let Some(connection_id) = connection_id.to_str() else {
        let _ = std::io::stderr().write_all(b"hiroute agent grant request invalid\n");
        return 2;
    };
    let material = LocalControlClient::cli_from_environment()
        .and_then(|client| client.read_agent_grant(connection_id));
    match material {
        Ok(material) => {
            if material.write_to(&mut std::io::stdout()).is_ok() {
                return 0;
            }
            let _ = std::io::stderr().write_all(b"hiroute agent grant unavailable\n");
            6
        }
        Err(_) => {
            let _ = std::io::stderr().write_all(b"hiroute agent grant unavailable\n");
            6
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_help_projects_public_application_and_host_families() {
        let help = root_help();
        assert!(help.contains("  agent"));
        assert!(help.contains("  schema"));
        assert!(help.contains("  worker"));
        assert!(!help.contains("setup"));
        assert!(help.contains("  gateway"));
        assert!(help.contains("  service"));
        assert!(!help.contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
        assert!(help.contains("Host management"));
        assert!(help.contains("hiroute service --help"));
        assert!(help.contains("hiroute gateway --help"));
        assert!(help.contains("hiroute protected-input --help"));
        assert!(help.contains("Application/Local Control"));
        assert!(help.contains("complete command path"));
        assert!(help.contains("hiroute worker dependencies discover --help"));
        assert!(!help.contains("Run a command family with '--help'"));
    }

    #[test]
    fn root_help_points_to_working_leaf_help_instead_of_unsupported_family_help() {
        let family = execute(["worker", "--help"]);
        assert_eq!(family.exit_code, 2);
        let family_error: Value = serde_json::from_str(&family.stdout).unwrap();
        assert_eq!(family_error["error"]["code"], "UNKNOWN_COMMAND");

        let leaf = execute(["worker", "dependencies", "select", "--help"]);
        assert_eq!(leaf.exit_code, 0);
        assert!(leaf.stderr.is_empty());
        assert!(leaf.stdout.contains("expected_selection_revision"));
        assert!(leaf.stdout.contains("replay that same document unchanged"));
    }

    #[test]
    fn schema_list_remains_a_single_public_machine_envelope() {
        let execution = execute(["schema", "list", "--output", "json", "--non-interactive"]);
        assert_eq!(execution.exit_code, 0);
        assert_eq!(execution.stdout.lines().count(), 1);
        let value: Value = serde_json::from_str(&execution.stdout).unwrap();
        assert_eq!(value["data"]["commands"].as_array().unwrap().len(), 49);
        assert!(!execution.stdout.contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
    }

    #[test]
    fn plan_quality_is_discoverable_through_public_schema_and_leaf_help() {
        let help = execute(["observation", "plan-quality", "samples", "--help"]);
        assert_eq!(help.exit_code, 0, "{}", help.stdout);
        assert!(help.stdout.contains("--session-id"));
        assert!(help.stdout.contains("--competence"));
        let list = execute(["schema", "list", "--output", "json"]);
        let list: Value = serde_json::from_str(&list.stdout).unwrap();
        assert!(
            list["data"]["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| {
                    entry["command_id"] == "observation.plan-quality.samples"
                        && entry["operation_id"] == "GetPlanQualitySamples"
                })
        );
        let descriptor = execute([
            "schema",
            "show",
            "--command-id",
            "observation.plan-quality.samples",
        ]);
        assert_eq!(descriptor.exit_code, 0, "{}", descriptor.stdout);
        let descriptor: Value = serde_json::from_str(&descriptor.stdout).unwrap();
        assert_eq!(descriptor["data"]["kind"], "query");
    }

    #[test]
    fn managed_launch_rejects_auth_routing_arguments_before_daemon_access() {
        let execution = execute([
            "agent",
            "launch",
            "--agent",
            "claude-code",
            "--context",
            "agent-context/claude/default",
            "--",
            "--use-bedrock",
        ]);
        assert_eq!(execution.exit_code, 3);
        let value: Value = serde_json::from_str(&execution.stdout).unwrap();
        assert_eq!(value["error"]["code"], "AGENT_AUTH_PRECEDENCE_CONFLICT");

        // Model selection, setting sources, and fallback stay caller-directed: the parse
        // succeeds and the command proceeds to (and fails at) Local Control, never at the
        // argument gate.
        let missing_delimiter = execute([
            "agent",
            "launch",
            "--agent",
            "claude-code",
            "--context",
            "agent-context/claude/default",
            "--print",
        ]);
        assert_eq!(missing_delimiter.exit_code, 2);
        let unknown_agent = execute([
            "agent",
            "launch",
            "--agent",
            "codex",
            "--context",
            "agent-context/claude/default",
            "--",
            "--print",
        ]);
        assert_eq!(unknown_agent.exit_code, 2);
    }

    #[test]
    fn hidden_helper_is_not_a_public_cli_command() {
        let execution = execute([HIDDEN_AGENT_GRANT_HELPER_VERB_V1, "agent-connection/claude"]);
        assert_eq!(execution.exit_code, 2);
        let value: Value = serde_json::from_str(&execution.stdout).unwrap();
        assert_eq!(value["error"]["code"], "UNKNOWN_COMMAND");
        assert!(!root_help().contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
    }

    #[test]
    fn unstaged_planned_command_is_not_callable() {
        let execution = execute(["settings", "show", "--output", "json"]);
        assert_eq!(execution.exit_code, 2);
    }

    #[test]
    fn released_leaf_help_is_publicly_callable() {
        let execution = execute(["compute", "connection", "preview", "--help"]);
        assert_eq!(execution.exit_code, 0);
        assert!(execution.stdout.contains("--request-stdin"));
    }

    #[test]
    fn capability_value_has_no_argv_or_environment_fallback() {
        let (_, globals) = split_globals(vec![
            "setup".into(),
            "apply".into(),
            "--capability-fd".into(),
            "9".into(),
        ])
        .unwrap();
        assert_eq!(globals.capability_fd, Some(9));
    }
}

mod tasks;
mod work_plans;
