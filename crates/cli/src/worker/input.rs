use std::fmt::Write as _;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use hiroute_application_api::{
    AgentPlanId, DEFAULT_WORKER_RUN_SECS, DEFAULT_WORKER_WAIT_SECS,
    DelegationSubmissionOperationV1, ErrorCode, MAX_DELEGATION_INPUT_BYTES, WorkerCancelRequestV1,
    WorkerDependenciesDiscoverRequestV1, WorkerDependenciesSelectRequestV1, WorkerHarnessV1,
    WorkerListRequestV1, WorkerPermissionPolicyV1, WorkerReadRequestV1,
    WorkerResidualConfirmRequestV1, WorkerResultRequestV1, WorkerStatusRequestV1,
    WorkerWaitRequestV1,
};

const MAX_FILE_BYTES: u64 = (MAX_DELEGATION_INPUT_BYTES + 1) as u64;
const MAX_DEPENDENCY_SELECTION_BYTES: u64 = 64 * 1024;

pub(super) fn parse_dependencies_discover(
    options: &[String],
) -> Result<WorkerDependenciesDiscoverRequestV1, ErrorCode> {
    let mut harness = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--harness" => {
                let parsed = match next(options, &mut index)?.as_str() {
                    "codex_cli" => WorkerHarnessV1::CodexCli,
                    "claude_code" => WorkerHarnessV1::ClaudeCode,
                    "qoder_cli" => WorkerHarnessV1::QoderCli,
                    "pi" => WorkerHarnessV1::Pi,
                    "deepseek_harness" => WorkerHarnessV1::DeepseekHarness,
                    _ => return Err(ErrorCode::InvalidArguments),
                };
                harness = unique(harness, parsed)?;
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerDependenciesDiscoverRequestV1 { harness };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_dependency_selection(
    options: &[String],
) -> Result<WorkerDependenciesSelectRequestV1, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_DEPENDENCY_SELECTION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_DEPENDENCY_SELECTION_BYTES {
        return Err(ErrorCode::InvalidArguments);
    }
    let request: WorkerDependenciesSelectRequestV1 =
        serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidArguments)?;
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_list(options: &[String]) -> Result<WorkerListRequestV1, ErrorCode> {
    let mut title = None;
    let mut cursor = None;
    let mut limit = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--title" => title = unique(title, next(options, &mut index)?)?,
            "--cursor" => cursor = unique(cursor, next(options, &mut index)?)?,
            "--limit" => limit = unique(limit, parse_u16(&next(options, &mut index)?)?)?,
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerListRequestV1 {
        title,
        cursor,
        limit,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_status(options: &[String]) -> Result<WorkerStatusRequestV1, ErrorCode> {
    let mut task_id = None;
    let mut run_id = None;
    let mut submission_key = None;
    let mut submission_operation = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--task" => task_id = unique(task_id, next(options, &mut index)?)?,
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--submission" | "--submission-key" => {
                submission_key = unique(submission_key, next(options, &mut index)?)?
            }
            "--operation" => {
                let operation = match next(options, &mut index)?.as_str() {
                    "exec" | "start" => DelegationSubmissionOperationV1::Start,
                    "continue" => DelegationSubmissionOperationV1::Continue,
                    _ => return Err(ErrorCode::InvalidArguments),
                };
                if submission_operation.replace(operation).is_some() {
                    return Err(ErrorCode::InvalidArguments);
                }
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerStatusRequestV1 {
        task_id,
        run_id,
        submission_key,
        submission_operation,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_wait(options: &[String]) -> Result<WorkerWaitRequestV1, ErrorCode> {
    let mut run_id = None;
    let mut after_revision = None;
    let mut wait_timeout_secs = DEFAULT_WORKER_WAIT_SECS;
    let mut wait_set = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--after-revision" => {
                after_revision = unique(after_revision, parse_u64(&next(options, &mut index)?)?)?
            }
            "--wait-timeout" | "--wait-timeout-secs" if !wait_set => {
                wait_timeout_secs = parse_u32(&next(options, &mut index)?)?;
                wait_set = true;
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerWaitRequestV1 {
        run_id: run_id.ok_or(ErrorCode::InvalidArguments)?,
        after_revision,
        wait_timeout_secs,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_result(options: &[String]) -> Result<WorkerResultRequestV1, ErrorCode> {
    let mut run_id = None;
    let mut offset = None;
    let mut max_bytes = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--offset" => offset = unique(offset, parse_u32(&next(options, &mut index)?)?)?,
            "--max-bytes" => {
                max_bytes = unique(max_bytes, parse_u32(&next(options, &mut index)?)?)?
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerResultRequestV1 {
        run_id: run_id.ok_or(ErrorCode::InvalidArguments)?,
        offset,
        max_bytes,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_residual_confirm(
    options: &[String],
) -> Result<WorkerResidualConfirmRequestV1, ErrorCode> {
    let mut run_id = None;
    let mut expected_revision = None;
    let mut idempotency_key = None;
    let mut handled = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--expected-revision" => {
                expected_revision = unique(expected_revision, next(options, &mut index)?)?
            }
            "--idempotency-key" => {
                idempotency_key = unique(idempotency_key, next(options, &mut index)?)?
            }
            "--handled" if !handled => handled = true,
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerResidualConfirmRequestV1 {
        run_id: run_id.ok_or(ErrorCode::InvalidArguments)?,
        expected_revision: expected_revision
            .ok_or(ErrorCode::InvalidArguments)?
            .parse()
            .map_err(|_| ErrorCode::InvalidArguments)?,
        idempotency_key: match idempotency_key {
            Some(key) => key,
            None => random_key("worker-residual")?.replace('/', ":"),
        },
        user_confirmed: handled,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_cancel(
    options: &[String],
    _request_id: &str,
) -> Result<WorkerCancelRequestV1, ErrorCode> {
    let mut run_id = None;
    let mut idempotency_key = None;
    let mut reason = String::new();
    let mut reason_set = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--idempotency-key" => {
                idempotency_key = unique(idempotency_key, next(options, &mut index)?)?
            }
            "--reason" if !reason_set => {
                reason = next(options, &mut index)?;
                reason_set = true;
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerCancelRequestV1 {
        run_id: run_id.ok_or(ErrorCode::InvalidArguments)?,
        idempotency_key: match idempotency_key {
            Some(key) => key,
            None => random_key("worker-cancel")?.replace('/', ":"),
        },
        reason,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) fn parse_read(options: &[String]) -> Result<WorkerReadRequestV1, ErrorCode> {
    let mut run_id = None;
    let mut cursor = None;
    let mut max_bytes = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--run" => run_id = unique(run_id, next(options, &mut index)?)?,
            "--cursor" => cursor = unique(cursor, next(options, &mut index)?)?,
            "--max-bytes" => {
                max_bytes = unique(max_bytes, parse_u32(&next(options, &mut index)?)?)?
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let request = WorkerReadRequestV1 {
        run_id: run_id.ok_or(ErrorCode::InvalidArguments)?,
        cursor,
        max_bytes,
    };
    request
        .valid()
        .then_some(request)
        .ok_or(ErrorCode::InvalidArguments)
}

pub(super) struct ParsedSubmission {
    pub(super) plan: Option<AgentPlanId>,
    pub(super) task: Option<String>,
    pub(super) expected_latest_run: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) permission_policy: WorkerPermissionPolicyV1,
    pub(super) run_timeout_secs: u32,
    pub(super) prompt: String,
    pub(super) submission_key: String,
    pub(super) parent_task_ref: Option<String>,
    pub(super) title: Option<String>,
    pub(super) no_wait: bool,
    pub(super) wait_timeout_secs: u32,
}

pub(super) fn parse_submission(
    options: &[String],
    continuation: bool,
    _request_id: &str,
) -> Result<ParsedSubmission, ErrorCode> {
    let mut plan = None;
    let mut task = None;
    let mut expected_latest_run = None;
    let mut cwd = None;
    let mut permission_policy = WorkerPermissionPolicyV1::ApproveAll;
    let mut permission_policy_set = false;
    let mut run_timeout_secs = DEFAULT_WORKER_RUN_SECS;
    let mut run_timeout_set = false;
    let mut file = None;
    let mut positional = None;
    let mut submission_key = None;
    let mut parent_task_ref = None;
    let mut title = None;
    let mut no_wait = false;
    let mut wait_timeout_secs = DEFAULT_WORKER_WAIT_SECS;
    let mut wait_timeout_set = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--plan" if !continuation => {
                let value = next(options, &mut index)?;
                plan = unique(
                    plan,
                    AgentPlanId::parse(value).map_err(|_| ErrorCode::InvalidArguments)?,
                )?;
            }
            "--task" if continuation => task = unique(task, next(options, &mut index)?)?,
            "--expected-latest-run" if continuation => {
                expected_latest_run = unique(expected_latest_run, next(options, &mut index)?)?
            }
            "--cwd" => cwd = unique(cwd, next(options, &mut index)?)?,
            "--permission-policy" if !permission_policy_set => {
                permission_policy = match next(options, &mut index)?.as_str() {
                    "approve-all" => WorkerPermissionPolicyV1::ApproveAll,
                    "approve-reads" => WorkerPermissionPolicyV1::ApproveReads,
                    "deny-all" => WorkerPermissionPolicyV1::DenyAll,
                    _ => return Err(ErrorCode::InvalidArguments),
                };
                permission_policy_set = true;
            }
            "--run-timeout" | "--run-timeout-secs" if !run_timeout_set => {
                run_timeout_secs = parse_u32(&next(options, &mut index)?)?;
                run_timeout_set = true;
            }
            "--wait-timeout" | "--wait-timeout-secs" if !wait_timeout_set && !no_wait => {
                wait_timeout_secs = parse_u32(&next(options, &mut index)?)?;
                wait_timeout_set = true;
            }
            "--no-wait" if !no_wait && !wait_timeout_set => no_wait = true,
            "--file" => file = unique(file, next(options, &mut index)?)?,
            "--submission-key" | "--idempotency-key" => {
                submission_key = unique(submission_key, next(options, &mut index)?)?
            }
            "--parent-task" if !continuation => {
                parent_task_ref = unique(parent_task_ref, next(options, &mut index)?)?
            }
            "--title" if !continuation => title = unique(title, next(options, &mut index)?)?,
            "--" => {
                if positional.is_some() || index + 2 != options.len() {
                    return Err(ErrorCode::InvalidArguments);
                }
                index += 1;
                positional = Some(options[index].clone());
            }
            value if !value.starts_with('-') => positional = unique(positional, value.to_owned())?,
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    if continuation {
        if task.is_none() || expected_latest_run.is_none() || plan.is_some() {
            return Err(ErrorCode::InvalidArguments);
        }
    } else if plan.is_none() || cwd.is_none() || task.is_some() || expected_latest_run.is_some() {
        return Err(ErrorCode::InvalidArguments);
    }
    let cwd = cwd.map(absolute_path).transpose()?;
    let prompt = read_prompt(positional, file)?;
    Ok(ParsedSubmission {
        plan,
        task,
        expected_latest_run,
        cwd,
        permission_policy,
        run_timeout_secs,
        prompt,
        submission_key: match submission_key {
            Some(key) => key,
            None => random_key(if continuation {
                "worker-continue"
            } else {
                "worker-exec"
            })?,
        },
        parent_task_ref,
        title,
        no_wait,
        wait_timeout_secs,
    })
}

fn read_prompt(positional: Option<String>, file: Option<String>) -> Result<String, ErrorCode> {
    if positional.is_some() && file.is_some() {
        return Err(ErrorCode::InvalidArguments);
    }
    let mut prompt = if let Some(prompt) = positional {
        prompt
    } else if let Some(path) = file {
        if path == "-" {
            read_bounded(std::io::stdin())?
        } else {
            read_bounded(std::fs::File::open(path).map_err(|_| ErrorCode::InvalidArguments)?)?
        }
    } else {
        if std::io::stdin().is_terminal() {
            return Err(ErrorCode::InvalidArguments);
        }
        read_bounded(std::io::stdin())?
    };
    if prompt.ends_with('\n') {
        prompt.pop();
        if prompt.ends_with('\r') {
            prompt.pop();
        }
    }
    if prompt.trim().is_empty() || prompt.contains('\0') {
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(prompt)
}

fn read_bounded(reader: impl Read) -> Result<String, ErrorCode> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.len() > MAX_DELEGATION_INPUT_BYTES {
        return Err(ErrorCode::InvalidArguments);
    }
    String::from_utf8(bytes).map_err(|_| ErrorCode::InvalidArguments)
}

fn next(options: &[String], index: &mut usize) -> Result<String, ErrorCode> {
    *index += 1;
    options
        .get(*index)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or(ErrorCode::InvalidArguments)
}

fn unique<T>(slot: Option<T>, value: T) -> Result<Option<T>, ErrorCode> {
    slot.is_none()
        .then_some(Some(value))
        .ok_or(ErrorCode::InvalidArguments)
}

fn parse_u32(value: &str) -> Result<u32, ErrorCode> {
    value.parse().map_err(|_| ErrorCode::InvalidArguments)
}

fn parse_u16(value: &str) -> Result<u16, ErrorCode> {
    value.parse().map_err(|_| ErrorCode::InvalidArguments)
}

fn parse_u64(value: &str) -> Result<u64, ErrorCode> {
    value.parse().map_err(|_| ErrorCode::InvalidArguments)
}

fn absolute_path(value: String) -> Result<String, ErrorCode> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|_| ErrorCode::CapabilityUnavailable)?
            .join(path)
    };
    path.into_os_string()
        .into_string()
        .map_err(|_| ErrorCode::InvalidArguments)
}

fn random_key(prefix: &str) -> Result<String, ErrorCode> {
    let mut entropy = [0_u8; 24];
    getrandom::fill(&mut entropy).map_err(|_| ErrorCode::CapabilityUnavailable)?;
    let mut key = String::with_capacity(prefix.len() + 1 + entropy.len() * 2);
    key.push_str(prefix);
    key.push('/');
    for byte in entropy {
        write!(&mut key, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_requires_exactly_one_selector_form() {
        assert!(parse_status(&["--task".into(), "task/one".into()]).is_ok());
        assert!(parse_status(&["--run".into(), "run/one".into()]).is_ok());
        assert!(
            parse_status(&[
                "--submission".into(),
                "worker/one".into(),
                "--operation".into(),
                "start".into(),
            ],)
            .is_ok()
        );
        assert!(
            parse_status(&[
                "--task".into(),
                "task/one".into(),
                "--submission".into(),
                "worker/one".into(),
                "--operation".into(),
                "start".into(),
            ],)
            .is_err()
        );
    }

    #[test]
    fn list_accepts_only_the_bounded_exclusive_page_contract() {
        let parsed = parse_list(&[
            "--cursor".into(),
            "sequence/42".into(),
            "--limit".into(),
            "25".into(),
        ])
        .unwrap();
        assert_eq!(parsed.cursor.as_deref(), Some("sequence/42"));
        assert_eq!(parsed.limit, Some(25));
        assert!(parse_list(&["--limit".into(), "0".into()]).is_err());
        assert!(parse_list(&["--limit".into(), "201".into()]).is_err());
        assert_eq!(
            parse_list(&["--cursor".into(), "42".into()])
                .unwrap()
                .cursor
                .as_deref(),
            Some("42")
        );
        assert!(parse_list(&["--cursor".into(), String::new()]).is_err());
        assert!(parse_list(&["--plan".into(), "worker".into()]).is_err());
    }

    #[test]
    fn submission_defaults_to_approve_all_and_accepts_explicit_restriction() {
        let cwd = std::env::current_dir().unwrap();
        let parsed = parse_submission(
            &[
                "--plan".into(),
                "worker".into(),
                "--cwd".into(),
                cwd.to_string_lossy().into_owned(),
                "--permission-policy".into(),
                "approve-reads".into(),
                "--run-timeout".into(),
                "60".into(),
                "goal".into(),
            ],
            false,
            "request",
        )
        .unwrap();
        assert_eq!(
            parsed.permission_policy,
            WorkerPermissionPolicyV1::ApproveReads
        );
        assert_eq!(parsed.run_timeout_secs, 60);
        assert!(std::path::Path::new(&parsed.cwd.unwrap()).is_absolute());

        let defaulted = parse_submission(
            &[
                "--plan".into(),
                "worker".into(),
                "--cwd".into(),
                cwd.to_string_lossy().into_owned(),
                "goal".into(),
            ],
            false,
            "request",
        )
        .unwrap();
        assert_eq!(
            defaulted.permission_policy,
            WorkerPermissionPolicyV1::ApproveAll
        );

        assert!(
            parse_submission(
                &[
                    "--plan".into(),
                    "worker".into(),
                    "--cwd".into(),
                    cwd.to_string_lossy().into_owned(),
                    "--tools".into(),
                    "read".into(),
                    "goal".into(),
                ],
                false,
                "request",
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_default_key_is_accepted_by_the_operation_boundary() {
        let options = ["--run", "run/test"].map(str::to_owned);
        assert!(parse_cancel(&options, "request-test").unwrap().valid());
        let invalid = ["--run", "run/test", "--idempotency-key", "cancel/test"].map(str::to_owned);
        assert!(parse_cancel(&invalid, "request-test").is_err());
    }

    #[test]
    fn residual_confirmation_requires_explicit_user_action_and_revision() {
        let options = [
            "--run",
            "run/test",
            "--expected-revision",
            "9",
            "--handled",
            "--idempotency-key",
            "confirm-test",
        ]
        .map(str::to_owned);
        let parsed = parse_residual_confirm(&options).unwrap();
        assert_eq!(parsed.expected_revision, 9);
        assert!(parsed.user_confirmed);
        assert!(parse_residual_confirm(&options[..5]).unwrap().valid());
        let mut invalid_key = options.clone();
        invalid_key[6] = "confirm/test".into();
        assert!(parse_residual_confirm(&invalid_key).is_err());
        assert!(parse_residual_confirm(&options[..4]).is_err());
        let mut stale = options.clone();
        stale[3] = "0".into();
        assert!(parse_residual_confirm(&stale).is_err());
        assert!(
            parse_residual_confirm(&["--run".into(), "run/test".into(), "--handled".into()])
                .is_err()
        );
    }

    #[test]
    fn generated_submission_keys_use_fresh_os_entropy() {
        let first = random_key("worker-exec").unwrap();
        let second = random_key("worker-exec").unwrap();
        assert!(first.starts_with("worker-exec/"));
        assert_ne!(first, second);
    }
}
