//! Presentation workflow for the independent collaboration facet. Application still owns
//! capability checks, preview digests, admission and field-owned restore.
mod flow;
mod receipt;
#[cfg(test)]
mod tests;
#[cfg(all(test, windows))]
mod windows_receipt_tests;

use hiroute_application_api::{AgentCollaborationTriggerModeV2, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::args::{Globals, OutputMode};
use crate::{CliExecution, LocalControlClient, failure, render};

const HELP: &str = "Configure task collaboration independently of model settings.\n\nUsage\n  hiroute agents collaboration configure <AGENT_ID> --trigger explicit|delegate-by-default [--preview] [--idempotency-key KEY] --output json\n  hiroute agents collaboration disable <AGENT_ID> [--preview | --yes] [--idempotency-key KEY] --output json\n  hiroute agents collaboration status <AGENT_ID> --output json\n\nUse the exact Agent ID from 'hiroute agents scan'. Codex collaboration uses its original HOME/root context, including when the model profile is occupied.\nConfigure enables or edits the Skill trigger. Save may run missing bounded local checks and then the existing Preview/Apply transaction; it never calls a provider model. --preview has no side effects and never runs a check or writes a submission receipt. Disable requires --yes unless previewing.\nSave freezes its request before submission. Use the same --idempotency-key to retry the same intent; a changed intent needs a new key. The response includes cli_submission.idempotency_key. An accepted Operation is pending: use 'hiroute operations get <OPERATION_ID>' or collaboration status to observe completion.\nThe advanced 'agents connect/restore preview/apply --request-stdin' commands retain their exact context semantics.\n";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Intent {
    Configure {
        trigger_mode: AgentCollaborationTriggerModeV2,
    },
    Disable,
    Status,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    agent_id: String,
    intent: Intent,
}

#[derive(Debug)]
pub(super) struct Invocation {
    selection: Selection,
    preview: bool,
    key: Option<String>,
}

// Explicit, closed test diagnostics for the native debug harness. No request,
// receipt path, key, Agent name, transport text or configuration is logged.
fn diagnostic(stage: &'static str, facts: impl std::fmt::Display) {
    #[cfg(all(windows, debug_assertions))]
    if std::env::var_os("HIROUTE_TEST_COLLABORATION_DIAGNOSTICS").is_some() {
        eprintln!("HiRoute collaboration diagnostic: {stage} {facts}");
    }
    #[cfg(not(all(windows, debug_assertions)))]
    let _ = (stage, facts);
}

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    if arguments.get(..2)? != ["agents", "collaboration"] {
        return None;
    }
    if globals.help {
        return Some(CliExecution {
            exit_code: 0,
            stdout: HELP.into(),
            stderr: String::new(),
        });
    }
    let request_id = globals
        .request_id
        .clone()
        .unwrap_or_else(crate::next_request_id);
    if globals.agent.is_some() || globals.has_capability() || globals.output != OutputMode::Json {
        return Some(failure(ErrorCode::InvalidArguments, Some(request_id)));
    }
    let invocation = match parse(&arguments[2..]) {
        Ok(invocation) => invocation,
        Err(code) => return Some(failure(code, Some(request_id))),
    };
    let client = match LocalControlClient::cli_from_environment() {
        Ok(client) => client.with_timeout(
            globals
                .timeout
                .unwrap_or(std::time::Duration::from_secs(30)),
        ),
        Err(_) => return Some(failure(ErrorCode::DaemonUnavailable, Some(request_id))),
    };
    // Read-only commands do not require writable state or even a state layout. Submission
    // records outlive a login/runtime directory and remain scoped to the selected endpoint.
    let receipts = if invocation.preview || invocation.selection.intent == Intent::Status {
        std::path::PathBuf::new()
    } else {
        match hiroute_host_runtime::StandaloneLayout::from_environment()
            .map_err(|_| ErrorCode::DaemonUnavailable)
            .and_then(|layout| receipt::root(&layout.state_root, &client.endpoint()))
        {
            Ok(root) => root,
            Err(code) => return Some(failure(code, Some(request_id))),
        }
    };
    Some(render(flow::run(
        &client, &receipts, invocation, request_id,
    )))
}

fn parse(options: &[String]) -> Result<Invocation, ErrorCode> {
    let invalid = ErrorCode::InvalidArguments;
    let [verb, agent_id, options @ ..] = options else {
        return Err(invalid);
    };
    if !bounded(agent_id, 256) || agent_id.starts_with('-') {
        return Err(invalid);
    }
    let mut trigger = None;
    let mut preview = false;
    let mut yes = false;
    let mut key = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--preview" if !preview => preview = true,
            "--yes" if !yes => yes = true,
            "--trigger" if trigger.is_none() => {
                index += 1;
                trigger = Some(match options.get(index).map(String::as_str) {
                    Some("explicit") => AgentCollaborationTriggerModeV2::Explicit,
                    Some("delegate-by-default") => {
                        AgentCollaborationTriggerModeV2::DelegateByDefault
                    }
                    _ => return Err(invalid),
                });
            }
            "--idempotency-key" if key.is_none() => {
                index += 1;
                key = Some(
                    options
                        .get(index)
                        .filter(|value| {
                            !value.is_empty()
                                && value.len() <= 200
                                && !value.starts_with('-')
                                && value.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric()
                                        || matches!(byte, b'.' | b'_' | b':' | b'-')
                                })
                        })
                        .ok_or(invalid)?
                        .clone(),
                );
            }
            _ => return Err(invalid),
        }
        index += 1;
    }
    if preview && (yes || key.is_some()) {
        return Err(invalid);
    }
    let intent = match verb.as_str() {
        "configure" if !yes => Intent::Configure {
            trigger_mode: trigger.ok_or(invalid)?,
        },
        "disable" if trigger.is_none() && (preview || yes) => Intent::Disable,
        "status" if trigger.is_none() && !preview && !yes && key.is_none() => Intent::Status,
        _ => return Err(invalid),
    };
    Ok(Invocation {
        selection: Selection {
            agent_id: agent_id.clone(),
            intent,
        },
        preview,
        key,
    })
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.contains(char::is_control)
}

pub(super) fn random_key() -> Result<String, ErrorCode> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).map_err(|_| ErrorCode::DaemonUnavailable)?;
    Ok(format!(
        "collaboration-{}",
        entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}
