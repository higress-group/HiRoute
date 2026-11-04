//! A local presentation command. No HTTP operation can launch a desktop application.
use crate::{
    CliExecution, ErrorCode, ErrorV1, LocalControlClient, MachineEnvelopeV2,
    args::{Globals, OutputMode},
    failure, render, success,
};

const HELP: &str = "Usage\n  hiroute agents desktop launch <CONTEXT_ID> --output json\n\nRun on the HiRoute service computer in its graphical desktop session. Requires the installed ChatGPT/Codex desktop app and a configured independent Desktop connection. Reads current local authority, shares user skill directories and opens the separate instance. Does not configure routes, copy account data or change HOME. Windows uses the current user's MSIX registration; Linux requires DISPLAY or WAYLAND_DISPLAY.\n";

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    if arguments.get(..2)? != ["agents", "desktop"] {
        return None;
    }
    if globals.help {
        return Some(CliExecution {
            exit_code: 0,
            stdout: HELP.into(),
            stderr: String::new(),
        });
    }
    if arguments.len() != 4
        || arguments[2] != "launch"
        || arguments[3].is_empty()
        || arguments[3].len() > 256
        || globals.agent.is_some()
        || globals.has_capability()
        || globals.output != OutputMode::Json
    {
        return Some(failure(
            ErrorCode::InvalidArguments,
            globals.request_id.clone(),
        ));
    }
    let client = match LocalControlClient::cli_from_environment() {
        Ok(client) => client
            .with_timeout(
                globals
                    .timeout
                    .unwrap_or(std::time::Duration::from_secs(30)),
            )
            .shared_client(),
        Err(_) => {
            return Some(failure(
                ErrorCode::DaemonUnavailable,
                globals.request_id.clone(),
            ));
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return Some(failure(ErrorCode::Internal, globals.request_id.clone())),
    };
    Some(
        match runtime.block_on(
            hiroute_desktop_host_effects::launch_configured_codex_instance(&client, &arguments[3]),
        ) {
            Ok(skills) => success(
                serde_json::json!({"launch_requested":true,"preserved_skills":skills.preserved}),
                globals.request_id.clone(),
            ),
            Err(hiroute_product_session::failure::DesktopFailure::Transport { .. }) => {
                failure(ErrorCode::DaemonUnavailable, globals.request_id.clone())
            }
            Err(hiroute_product_session::failure::DesktopFailure::Backend { mut envelope }) => {
                envelope.request_id = globals.request_id.clone();
                render(*envelope)
            }
            Err(hiroute_product_session::failure::DesktopFailure::Native { code }) => {
                let mut detail = ErrorV1::new(ErrorCode::ActionRequired);
                detail.message_key = format!("error.{}", code.to_ascii_lowercase());
                let mut result = render(MachineEnvelopeV2::<serde_json::Value>::failed(
                    detail,
                    globals.request_id.clone(),
                ));
                result.stderr = format!(
                    "{code}: Keep saved routing and native data. Check the independent Desktop connection and installed app; run this command from the service computer's graphical desktop session.\n"
                );
                result
            }
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn codex_desktop_local_launch_help_is_offline_and_rejects_paths_and_agent_authority() {
        let help = crate::execute(["agents", "desktop", "launch", "--help"]);
        assert_eq!(help.exit_code, 0);
        assert!(help.stdout.contains("graphical desktop session"));
        for args in [
            vec!["agents", "desktop", "launch", "context", "--path", "app"],
            vec!["agents", "desktop", "launch"],
            vec![
                "agents",
                "desktop",
                "launch",
                "context",
                "--agent",
                "agent/one",
            ],
        ] {
            assert_ne!(crate::execute(args).exit_code, 0);
        }
    }
}
