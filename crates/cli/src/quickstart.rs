//! Local presentation over existing queries/Preview/Apply contracts; no setup transaction.
use crate::{CliExecution, LocalControlClient, args::Globals};
use hiroute_application_api::*;
use serde_json::{Value, json};

mod apply_file;
mod decision;
mod decision_v2;
mod flags;
mod navigation;
mod plan_restore;
mod preview;
mod settings;
use flags::Flags;

type Reply = MachineEnvelopeV2<Value>;
type Result<T> = std::result::Result<T, Box<Reply>>;

pub(crate) const HELP: &str = "HiRoute Quickstart\n\nUsage\n  hiroute quickstart [--context ID] [--plan ID] [--hermes-root PATH]\n\nRead-only: discover an exact target, reuse existing sources and Plans, then inspect connection state and next commands. Does not prepare credentials, save configuration, start a service, restart an Agent or call a model. Caller and target are independent.\n\nMain path\n  quickstart -> compute scan/connection prepare -> compute connection preview/apply\n  -> routing preview/apply -> agents connect preview/apply -> native activation -> agents connect status --effective\n\nUse --help on each Preview for concise choices. --apply-file PATH creates a new private exact Apply request; review the Preview, then submit that same file. Keep the file/key for Operations lookup and identical replay after uncertain delivery.\n";

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    if let Some(execution) = decision::execute(arguments, globals) {
        return Some(execution);
    }
    if let Some(execution) = decision_v2::execute(arguments, globals) {
        return Some(execution);
    }
    if arguments.first().is_none_or(|v| v != "quickstart") {
        return None;
    }
    if globals.help {
        return Some(CliExecution {
            exit_code: 0,
            stdout: HELP.into(),
            stderr: String::new(),
        });
    }
    let result: Result<Reply> = (|| {
        validate_globals(globals)?;
        let flags = Flags::parse(
            &arguments[1..],
            &["--context", "--plan", "--hermes-root"],
            &[],
            &[],
        )?;
        navigation::run(&flags, globals)
    })();
    Some(render_reply(result.unwrap_or_else(|reply| *reply), globals))
}

pub(crate) fn preview_command(
    command: &str,
    options: &[String],
    globals: &Globals,
) -> Option<CliExecution> {
    if !matches!(
        command,
        "compute.connection.preview"
            | "routing.preview"
            | "agents.connect.preview"
            | "agents.restore.preview"
            | "decision.services.apply"
            | "decision.services.test"
            | "decision.connections.apply"
            | "decision.connections.test"
    ) {
        return None;
    }
    let has_input = options
        .iter()
        .any(|v| matches!(v.as_str(), "--request-file" | "--request-stdin"));
    let has_apply_file = options.iter().any(|v| v == "--apply-file");
    if has_input && !has_apply_file {
        return None;
    }
    if options.is_empty() {
        return None;
    }
    let _diagnostics =
        crate::request_input::diagnostics::Scope::enter(command, globals.error_details);
    let result: Result<Reply> = (|| {
        validate_globals(globals)?;
        let (options, path, key) = flags::submission(options)?;
        if matches!(
            command,
            "decision.services.test" | "decision.connections.test"
        ) && path.is_some()
        {
            return Err(error(ErrorCode::InvalidArguments));
        }
        let client = client(globals)?;
        let payload = if has_input {
            // The existing codec rejects mixed JSON/convenience forms and unknown members.
            crate::command_payload(command, &options).map_err(error)?
        } else {
            preview::prepare(&client, command, &options)?
        };
        if matches!(
            command,
            "decision.services.apply" | "decision.connections.apply"
        ) && path.is_some()
            && payload.get("accept_digest").is_some()
        {
            return Err(choices(
                &[
                    "Preview input for --apply-file; submit an existing Apply file without --apply-file",
                ],
                json!({}),
            ));
        }
        let descriptor = command_by_id(command).ok_or_else(|| error(ErrorCode::UnknownCommand))?;
        let mut response = call_with_id(
            &client,
            &descriptor.operation_id,
            payload.clone(),
            globals.request_id.clone(),
        )?;
        if let Some(path) = path {
            let preview = response
                .data
                .take()
                .ok_or_else(|| error(ErrorCode::Internal))?;
            response.data = Some(apply_file::prepare(command, &payload, preview, &path, key)?);
        }
        Ok(response)
    })();
    Some(render_reply(result.unwrap_or_else(|reply| *reply), globals))
}

fn render_reply(mut reply: Reply, globals: &Globals) -> CliExecution {
    if globals.request_id.is_some() {
        reply.request_id = globals.request_id.clone();
    }
    if reply.data.is_none()
        && reply
            .error
            .as_ref()
            .is_some_and(|error| error.code == ErrorCode::InvalidArguments)
    {
        return crate::request_input::diagnostics::failure(
            ErrorCode::InvalidArguments,
            reply.request_id,
        );
    }
    crate::render(reply)
}

fn validate_globals(globals: &Globals) -> Result<()> {
    if globals.agent.is_some()
        || globals.capability_fd.is_some()
        || globals.output != crate::args::OutputMode::Json
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    Ok(())
}
fn client(globals: &Globals) -> Result<LocalControlClient> {
    LocalControlClient::cli_from_environment()
        .map(|client| {
            client.with_timeout(
                globals
                    .timeout
                    .unwrap_or(std::time::Duration::from_secs(30)),
            )
        })
        .map_err(|_| error(ErrorCode::DaemonUnavailable))
}
fn error(code: ErrorCode) -> Box<Reply> {
    Box::new(MachineEnvelopeV2::failed(ErrorV1::new(code), None))
}
fn call(client: &LocalControlClient, operation: &str, payload: Value) -> Result<Reply> {
    call_with_id(client, operation, payload, None)
}
fn call_with_id(
    client: &LocalControlClient,
    operation: &str,
    payload: Value,
    request_id: Option<String>,
) -> Result<Reply> {
    let request_id = request_id.unwrap_or_else(crate::next_request_id);
    let response = client
        .call(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.clone(),
            operation_id: operation.into(),
            payload,
            protected_grant: None,
        })
        .map_err(|_| {
            MachineEnvelopeV2::failed(ErrorV1::new(ErrorCode::DaemonUnavailable), Some(request_id))
        })?;
    if response.status != MachineStatus::Succeeded {
        return Err(Box::new(response));
    }
    Ok(response)
}
fn data(client: &LocalControlClient, operation: &str, payload: Value) -> Result<Value> {
    call(client, operation, payload)?
        .data
        .ok_or_else(|| error(ErrorCode::Internal))
}
fn choices(fields: &[&str], available: Value) -> Box<Reply> {
    let mut response = error(ErrorCode::InvalidArguments);
    response.data = Some(
        json!({"schema":"hiroute.cli-required-choices/v1", "required_choices":fields, "available":available}),
    );
    response
}
fn fresh_key() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| error(ErrorCode::Internal))?;
    Ok(format!(
        "quickstart-{}",
        bytes.iter().map(|v| format!("{v:02x}")).collect::<String>()
    ))
}
