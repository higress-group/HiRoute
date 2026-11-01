//! Read only this executable's compiled identity, without discovering or starting a daemon.
use crate::{
    CliExecution,
    args::{Globals, OutputMode},
    failure, success,
};
use hiroute_application_api::ErrorCode;
use hiroute_host_runtime::BuildIdentityV1;

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    let verb = arguments.first()?.as_str();
    if !matches!(verb, "version" | "--version" | "-V") {
        return None;
    }
    if arguments.len() != 1 || globals.agent.is_some() || globals.has_capability() {
        return Some(failure(
            ErrorCode::InvalidArguments,
            globals.request_id.clone(),
        ));
    }
    if globals.help {
        return Some(CliExecution { exit_code: 0, stdout: "Usage\n  hiroute version [--output json|text|quiet]\n  hiroute --version\n\nOffline compiled CLI identity; never starts or queries a daemon. --version/-V default to text. The version command defaults to JSON. source_revision is null when the build did not embed a valid source SHA; runtime environment and working-directory Git do not supply it. Inspect the installed daemon separately with hirouted --version.\n".into(), stderr: String::new() });
    }
    let identity = BuildIdentityV1::compiled(
        "cli",
        env!("CARGO_PKG_VERSION"),
        option_env!("HIROUTE_BUILD_SOURCE_SHA"),
    );
    let output = if !globals.output_explicit && verb != "version" {
        OutputMode::Text
    } else {
        globals.output
    };
    Some(match output {
        OutputMode::Json => success(identity, globals.request_id.clone()),
        OutputMode::Text => CliExecution {
            exit_code: 0,
            stdout: identity.text(),
            stderr: String::new(),
        },
        OutputMode::Quiet => CliExecution {
            exit_code: 0,
            stdout: format!("{}\n", identity.version),
            stderr: String::new(),
        },
    })
}
