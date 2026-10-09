use std::time::Duration;

use hiroute_application_api::ErrorCode;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum OutputMode {
    #[default]
    Json,
    Text,
    Quiet,
}

#[derive(Debug, Default)]
pub(crate) struct Globals {
    pub(crate) request_id: Option<String>,
    pub(crate) help: bool,
    pub(crate) capability_fd: Option<u32>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) agent: Option<String>,
    pub(crate) output: OutputMode,
    pub(crate) output_explicit: bool,
}

#[derive(Debug)]
pub(crate) struct GlobalSplitError {
    pub(crate) code: ErrorCode,
    pub(crate) globals: Globals,
    pub(crate) worker: bool,
}

pub(crate) fn split_globals(
    arguments: Vec<String>,
) -> Result<(Vec<String>, Globals), GlobalSplitError> {
    let worker = is_worker_invocation(&arguments);
    let mut remaining = Vec::new();
    let mut globals = Globals::default();
    let mut index = 0;
    while index < arguments.len() {
        if worker {
            if arguments[index] == "--" {
                remaining.extend(arguments[index..].iter().cloned());
                break;
            }
            if let Some(arity) = worker_option_arity(&arguments[index]) {
                remaining.push(arguments[index].clone());
                if arity == 1 {
                    let Some(value) = arguments.get(index + 1) else {
                        return Err(split_error(globals, worker));
                    };
                    remaining.push(value.clone());
                }
                index += arity + 1;
                continue;
            }
        }
        match arguments[index].as_str() {
            "--help" | "-h" => {
                globals.help = true;
                index += 1;
            }
            "--non-interactive" | "--no-color" => index += 1,
            "--json" => {
                globals.output = OutputMode::Json;
                globals.output_explicit = true;
                index += 1;
            }
            "--agent" => {
                let Some(agent) = arguments
                    .get(index + 1)
                    .filter(|value| matches!(value.as_str(), "codex" | "claude-code"))
                else {
                    return Err(split_error(globals, worker));
                };
                if globals.agent.replace(agent.clone()).is_some() {
                    return Err(split_error(globals, worker));
                }
                index += 2;
            }
            "--output" => {
                globals.output = match arguments.get(index + 1).map(String::as_str) {
                    Some("json") => OutputMode::Json,
                    Some("text") => OutputMode::Text,
                    Some("quiet") => OutputMode::Quiet,
                    _ => return Err(split_error(globals, worker)),
                };
                globals.output_explicit = true;
                index += 2;
            }
            "--request-id" => {
                let Some(request_id) = arguments
                    .get(index + 1)
                    .filter(|value| !value.is_empty())
                    .cloned()
                else {
                    return Err(split_error(globals, worker));
                };
                globals.request_id = Some(request_id);
                index += 2;
            }
            "--timeout" => {
                let Some(value) = arguments.get(index + 1).filter(|value| !value.is_empty()) else {
                    return Err(split_error(globals, worker));
                };
                let Ok(timeout) = parse_duration(value) else {
                    return Err(split_error(globals, worker));
                };
                globals.timeout = Some(timeout);
                index += 2;
            }
            "--capability-fd" => {
                let Some(fd) = arguments
                    .get(index + 1)
                    .and_then(|value| value.parse().ok())
                else {
                    return Err(split_error(globals, worker));
                };
                globals.capability_fd = Some(fd);
                index += 2;
            }
            value if worker && value.starts_with('-') => {
                // An unknown Worker option has unknown arity. Preserve the rest for the typed
                // parser and do not reinterpret any later token as a global flag.
                remaining.extend(arguments[index..].iter().cloned());
                break;
            }
            _ => {
                remaining.push(arguments[index].clone());
                index += 1;
            }
        }
    }
    if globals.agent.is_some() && globals.capability_fd.is_some() {
        return Err(split_error(globals, worker));
    }
    if worker && !globals.output_explicit {
        globals.output = OutputMode::Text;
    }
    Ok((remaining, globals))
}

fn split_error(mut globals: Globals, worker: bool) -> GlobalSplitError {
    if worker && !globals.output_explicit {
        globals.output = OutputMode::Text;
    }
    GlobalSplitError {
        code: ErrorCode::InvalidArguments,
        globals,
        worker,
    }
}

fn is_worker_invocation(arguments: &[String]) -> bool {
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "worker" => return true,
            "--help" | "-h" | "--non-interactive" | "--no-color" | "--json" => index += 1,
            "--agent" | "--output" | "--request-id" | "--timeout" | "--capability-fd" => index += 2,
            _ => return false,
        }
    }
    false
}

fn worker_option_arity(option: &str) -> Option<usize> {
    match option {
        "--no-wait" | "--request-stdin" | "--handled" => Some(0),
        "--title"
        | "--cursor"
        | "--limit"
        | "--plan"
        | "--cwd"
        | "--permission-policy"
        | "--run-timeout"
        | "--run-timeout-secs"
        | "--wait-timeout"
        | "--wait-timeout-secs"
        | "--file"
        | "--submission-key"
        | "--idempotency-key"
        | "--parent-task"
        | "--task"
        | "--expected-latest-run"
        | "--expected-revision"
        | "--run"
        | "--after-revision"
        | "--offset"
        | "--max-bytes"
        | "--reason"
        | "--submission"
        | "--operation"
        | "--harness"
        | "--tools" => Some(1),
        _ => None,
    }
}

fn parse_duration(value: &str) -> Result<Duration, ErrorCode> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1_u64)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1_000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else {
        return Err(ErrorCode::InvalidArguments);
    };
    let milliseconds = number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .filter(|milliseconds| (1..=3_600_000).contains(milliseconds))
        .ok_or(ErrorCode::InvalidArguments)?;
    Ok(Duration::from_millis(milliseconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_bounded_and_typed() {
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("0s"), Err(ErrorCode::InvalidArguments));
        assert_eq!(parse_duration("2h"), Err(ErrorCode::InvalidArguments));
    }

    #[test]
    fn worker_defaults_to_text_but_explicit_output_wins() {
        let (_, defaults) = split_globals(vec!["worker".into(), "list".into()]).unwrap();
        assert_eq!(defaults.output, OutputMode::Text);
        assert!(!defaults.output_explicit);

        let (_, explicit) = split_globals(vec![
            "worker".into(),
            "list".into(),
            "--output".into(),
            "json".into(),
        ])
        .unwrap();
        assert_eq!(explicit.output, OutputMode::Json);
        assert!(explicit.output_explicit);
    }

    #[test]
    fn worker_option_values_are_never_reinterpreted_as_global_flags() {
        let (remaining, globals) = split_globals(vec![
            "worker".into(),
            "exec".into(),
            "--title".into(),
            "--json".into(),
        ])
        .unwrap();
        assert_eq!(remaining, ["worker", "exec", "--title", "--json"]);
        assert_eq!(globals.output, OutputMode::Text);

        let (remaining, globals) = split_globals(vec![
            "worker".into(),
            "dependencies".into(),
            "select".into(),
            "--request-stdin".into(),
            "--output".into(),
            "json".into(),
        ])
        .unwrap();
        assert_eq!(remaining[3..], ["--request-stdin"]);
        assert_eq!(globals.output, OutputMode::Json);

        let (remaining, globals) = split_globals(vec![
            "worker".into(),
            "list".into(),
            "--unknown".into(),
            "value".into(),
            "--json".into(),
        ])
        .unwrap();
        assert_eq!(remaining[2..], ["--unknown", "value", "--json"]);
        assert_eq!(globals.output, OutputMode::Text);
    }

    #[test]
    fn cleanup_confirmation_accepts_trailing_global_output() {
        let (remaining, globals) = split_globals(
            [
                "worker",
                "cleanup",
                "confirm",
                "--run",
                "run/one",
                "--expected-revision",
                "3",
                "--handled",
                "--idempotency-key",
                "cleanup-one",
                "--output",
                "json",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
        .unwrap();
        assert_eq!(globals.output, OutputMode::Json);
        assert_eq!(remaining.last().map(String::as_str), Some("cleanup-one"));
        assert!(remaining.iter().any(|value| value == "--handled"));
        assert!(remaining.iter().any(|value| value == "--expected-revision"));
    }
}
