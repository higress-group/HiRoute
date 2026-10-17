//! Host effects for the seven product settings commands. No caller-controlled path or process.
use super::server::Server;
use hiroute_host_runtime::*;
use hiroute_product_session::{failure::DesktopFailure, session::query};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
pub(super) fn is_host_product(name: &str) -> bool {
    matches!(
        name,
        "gateway_listener_status"
            | "gateway_listener_apply"
            | "gateway_listener_recover"
            | "subscription_proxy_status"
            | "subscription_proxy_apply"
            | "diagnostic_status"
            | "set_diagnostic_level"
    )
}
fn empty(args: Value) -> Result<(), DesktopFailure> {
    let _: hiroute_application_api::ClientEmptyRequestV1 =
        serde_json::from_value(args).map_err(|_| "REQUEST_INVALID")?;
    Ok(())
}
pub(super) async fn command(
    server: &Arc<Server>,
    name: &str,
    args: Value,
) -> Result<Value, DesktopFailure> {
    match name {
        "gateway_listener_status" => {
            empty(args)?;
            let config = GatewayListenerStore::new(server.layout.gateway_config_root())
                .load_or_default()
                .map_err(|e| e.to_string())?;
            let address = config
                .applied_address()
                .map_err(|e| e.to_string())?
                .and_then(|a| connect_address(a).ok())
                .map(|a| a.to_string());
            let agents: Value = query(
                &server.client,
                "ListAgents",
                &hiroute_application_api::ClientEmptyRequestV1 {},
            )
            .await?;
            let count = agents.get("agents").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter(|a| a.get("context_id").is_some_and(|id| !id.is_null()))
                    .count()
            });
            Ok(
                json!({"config":config,"connect_address":address,"suggested_agent_base_url":address.as_ref().map(|a|format!("http://{a}/v1")),"existing_agent_count":count,"ready":address.is_some()}),
            )
        }
        "gateway_listener_apply" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "snake_case")]
            enum Scope {
                Local,
                External,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                scope: Scope,
                #[serde(default)]
                accept_remote_risk: bool,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: Input,
            }
            let input: Arguments = serde_json::from_value(args).map_err(|_| "REQUEST_INVALID")?;
            if matches!(input.input.scope, Scope::External) && !input.input.accept_remote_risk {
                return Err("REMOTE_LISTENER_CONFIRMATION_REQUIRED".into());
            }
            let store = GatewayListenerStore::new(server.layout.gateway_config_root());
            let mut desired = store.load_or_default().map_err(|e| e.to_string())?.desired;
            desired.address = if matches!(input.input.scope, Scope::External) {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            }
            .into();
            store.configure(desired).map_err(|e| e.to_string())?;
            restart(server)?;
            Ok(Value::Null)
        }
        "gateway_listener_recover" => {
            empty(args)?;
            GatewayListenerStore::new(server.layout.gateway_config_root())
                .recover()
                .map_err(|e| e.to_string())?;
            restart(server)?;
            Ok(Value::Null)
        }
        "subscription_proxy_status" => {
            empty(args)?;
            serde_json::to_value(
                SubscriptionProxyStore::new(
                    server
                        .layout
                        .storage_root()
                        .parent()
                        .ok_or("APP_DATA_UNAVAILABLE")?,
                )
                .view()
                .map_err(|e| e.to_string())?,
            )
            .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "subscription_proxy_apply" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                policy: SubscriptionProxyPolicy,
            }
            let input: Arguments = serde_json::from_value(args).map_err(|_| "REQUEST_INVALID")?;
            SubscriptionProxyStore::new(
                server
                    .layout
                    .storage_root()
                    .parent()
                    .ok_or("APP_DATA_UNAVAILABLE")?,
            )
            .configure(input.policy)
            .map_err(|e| e.to_string())?;
            restart(server)?;
            Ok(Value::Null)
        }
        "diagnostic_status" => {
            empty(args)?;
            diagnostics(server)
        }
        "set_diagnostic_level" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                expected_revision: u64,
                level: hiroute_diagnostics::level::DiagnosticLevel,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: Input,
            }
            let input: Arguments = serde_json::from_value(args).map_err(|_| "REQUEST_INVALID")?;
            let runtime = server
                .diagnostic_runtime
                .clone()
                .ok_or("DIAGNOSTICS_UNAVAILABLE")?;
            tokio::task::spawn_blocking(move || {
                if runtime.override_active() {
                    return Err(DesktopFailure::from("DIAGNOSTIC_OVERRIDE_ACTIVE"));
                }
                if !runtime.wait_armed(std::time::Duration::from_secs(2)) || !runtime.owns_role() {
                    return Err("DIAGNOSTICS_UNAVAILABLE".into());
                }
                let settings = runtime.settings().ok_or("DIAGNOSTICS_UNAVAILABLE")?;
                let saved = settings
                    .save(input.input.expected_revision, input.input.level)
                    .map_err(|e| {
                        DesktopFailure::from(
                            hiroute_diagnostics::settings::stable_code(&e)
                                .as_str()
                                .to_owned(),
                        )
                    })?;
                runtime.apply_saved_level(saved.level, saved.revision);
                Ok(())
            })
            .await
            .map_err(|_| "DIAGNOSTICS_UNAVAILABLE")??;
            diagnostics(server)
        }
        _ => Err("COMMAND_UNKNOWN".into()),
    }
}
fn diagnostics(server: &Server) -> Result<Value, DesktopFailure> {
    let runtime = server
        .diagnostic_runtime
        .as_ref()
        .ok_or("DIAGNOSTICS_UNAVAILABLE")?;
    let status = runtime.status();
    Ok(
        json!({"schema":"hiroute.diagnostic-status/v1","settings":{"revision":status.revision,"level":status.level,"error":status.settings_error.or(status.unavailable).map(hiroute_diagnostics::error::StableErrorCode::from_subsystem_reason)},"logs_directory_available":true}),
    )
}
fn restart(server: &Server) -> Result<(), DesktopFailure> {
    let marker = read_standalone_install_record(&server.layout.marker_path)
        .map_err(|_| "SERVICE_UNAVAILABLE")?;
    let cli = standalone_cli_binary(&marker.install_root);
    std::thread::Builder::new()
        .name("hiroute-console-restart".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = std::process::Command::new(cli)
                .args(["service", "restart", "--output", "json"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        })
        .map_err(|_| "SERVICE_UNAVAILABLE")?;
    Ok(())
}
