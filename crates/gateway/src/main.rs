#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use hiroute_diagnostics::{
    DiagnosticLevel,
    event::ProcessRole,
    record::Component,
    runtime::{DiagnosticRuntime, RuntimeConfig},
};
#[cfg(feature = "e2e-test-control")]
use hiroute_gateway::server::test_control::E2eControlOptions;
use hiroute_gateway::server::{GatewayLaunchIdentity, GatewayLauncher, GatewayLauncherError};

#[derive(Debug, Parser)]
#[command(name = "hirouted")]
#[command(about = "HiRoute standalone gateway listener")]
struct Cli {
    #[arg(long, default_value = "gateway")]
    role: String,
    #[arg(long, default_value = "127.0.0.1:8317")]
    listen: SocketAddr,
    /// Optional PROCESS-22002 Oracle fixture compatibility mode.
    #[arg(long)]
    fixture: Option<PathBuf>,
    /// Durable aggregate publication LKG used by normal production mode.
    #[arg(long, default_value = "hiroute-gateway-lkg.json")]
    lkg: PathBuf,
    /// Optional sealed v2 publication applied through the production feed at
    /// startup. It is never interpreted as an Oracle fixture.
    #[arg(long, conflicts_with = "fixture")]
    publication: Option<PathBuf>,
    /// Exact-key credential authority used by production Provider attempts.
    #[arg(long, conflicts_with = "fixture")]
    credentials: Option<PathBuf>,
    /// Optional sealed static Planner/profile input for production requests.
    #[arg(long, conflicts_with = "fixture")]
    planner: Option<PathBuf>,
    /// Private local diagnostic root for this standalone process.
    #[arg(long)]
    diagnostics_root: Option<PathBuf>,
    /// Temporary diagnostic level; never changes the saved setting.
    #[arg(long, requires = "diagnostics_root")]
    diagnostic_level_override: Option<DiagnosticLevel>,
    #[cfg(feature = "e2e-test-control")]
    /// Explicit, sealed real-process E2E control endpoint. Hidden so normal
    /// production discovery cannot mistake this for a supported operator API.
    #[arg(
        long,
        hide = true,
        conflicts_with = "fixture",
        requires = "e2e_control_nonce_file"
    )]
    e2e_control_listen: Option<SocketAddr>,
    #[cfg(feature = "e2e-test-control")]
    #[arg(
        long,
        hide = true,
        conflicts_with = "fixture",
        requires = "e2e_control_listen"
    )]
    e2e_control_nonce_file: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    if cli.role != "gateway" {
        return Err(GatewayLauncherError::UnsupportedRole(cli.role).into());
    }
    let diagnostics = cli.diagnostics_root.as_ref().map(|root| {
        DiagnosticRuntime::start(RuntimeConfig {
            root: root.clone(),
            role: ProcessRole::Daemon,
            component: Component::Gateway,
            parent_session_id: None,
            level_override: cli.diagnostic_level_override,
        })
    });
    if let Some(runtime) = &diagnostics {
        hiroute_diagnostics::panic::install_panic_hook(runtime.handle().clone());
    }
    let launch_identity = GatewayLaunchIdentity::from_environment()?;
    let launcher = match cli.fixture {
        Some(fixture) => {
            GatewayLauncher::from_fixture_with_identity(cli.listen, &fixture, launch_identity)?
        }
        None => {
            #[cfg(feature = "e2e-test-control")]
            {
                let test_control = cli
                    .e2e_control_listen
                    .zip(cli.e2e_control_nonce_file)
                    .map(|(listen, nonce_file)| E2eControlOptions { listen, nonce_file });
                GatewayLauncher::production_with_runtime_files_planner_test_control_and_identity(
                    cli.listen,
                    &cli.lkg,
                    cli.publication.as_deref(),
                    cli.credentials.as_deref(),
                    cli.planner.as_deref(),
                    test_control,
                    launch_identity,
                )?
            }
            #[cfg(not(feature = "e2e-test-control"))]
            {
                GatewayLauncher::production_with_runtime_files_and_planner_with_identity(
                    cli.listen,
                    &cli.lkg,
                    cli.publication.as_deref(),
                    cli.credentials.as_deref(),
                    cli.planner.as_deref(),
                    launch_identity,
                )?
            }
        }
    };
    let launcher = match &diagnostics {
        Some(runtime) => launcher.with_diagnostics(runtime.port()),
        None => launcher,
    };
    launcher.serve()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn e2e_control_flags_are_hidden_from_normal_help() {
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("e2e-control"));
    }

    #[cfg(not(feature = "e2e-test-control"))]
    #[test]
    fn normal_build_rejects_test_control_flags() {
        assert!(
            Cli::try_parse_from(["hirouted", "--e2e-control-listen", "127.0.0.1:8318"]).is_err()
        );
    }
}
