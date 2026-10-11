use std::time::Duration;

use hiroute_application_api::{
    ErrorCode, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MachineStatus,
    STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1, StandaloneProtectedInputRequestV1,
    release_manifest,
};
use hiroute_host_runtime::{
    GatewayListenerDesiredV1, GatewayListenerStore, StandaloneLayout, connect_address,
};
use serde_json::json;

use crate::args::{Globals, OutputMode};
use crate::{CliExecution, LocalControlClient, failure, service, success};

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    let family = arguments.first()?.as_str();
    if !matches!(
        family,
        "service" | "gateway" | "protected-input" | "subscription-proxy"
    ) {
        return None;
    }
    if globals.help {
        return Some(CliExecution {
            exit_code: 0,
            stdout: host_help(family).to_owned(),
            stderr: String::new(),
        });
    }
    let request_id = globals.request_id.clone();
    if globals.agent.is_some()
        || globals.capability_fd.is_some()
        || globals.output != OutputMode::Json
    {
        return Some(failure(ErrorCode::InvalidArguments, request_id));
    }
    let result = match family {
        "service" => service_command(&arguments[1..], globals),
        "subscription-proxy" => subscription_proxy_command(&arguments[1..]),
        "gateway" => gateway_command(&arguments[1..], globals),
        "protected-input" => protected_input_command(&arguments[1..], globals),
        _ => unreachable!(),
    };
    Some(match result {
        Ok(value) => success(value, request_id),
        Err(error) => failure(error, request_id),
    })
}

fn host_help(family: &str) -> &'static str {
    match family {
        "service" => {
            "Usage\n  hiroute service start|stop|restart|status|logs|doctor --output json\n  hiroute service autostart enable|disable|status --output json\n  hiroute service run\n"
        }
        "gateway" => {
            "Usage\n  hiroute gateway show --output json\n  hiroute gateway set --address <IPV4> --port auto|<PORT> [--accept-remote-risk] --output json\n  hiroute gateway recover --output json\n"
        }
        "subscription-proxy" => {
            "Usage\n  hiroute subscription-proxy show --output json\n  hiroute subscription-proxy set --mode inherit|direct|manual [--url <HTTP_PROXY_URL>] [--no-proxy <BYPASS_LIST>] --output json\n  Restart the service to apply saved settings: hiroute service restart --output json\n"
        }
        "protected-input" => {
            "Usage\n  hiroute protected-input register --candidate <REF> --secret-fd <FD> --output json\n  hiroute protected-input release --candidate <REF> --output json\n"
        }
        _ => unreachable!(),
    }
}

fn service_command(
    arguments: &[String],
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
    let timeout = globals.timeout.unwrap_or(Duration::from_secs(30));
    let status = match arguments {
        [verb] if verb == "start" => json_value(service::start(timeout).map_err(map)?)?,
        [verb] if verb == "stop" => json_value(service::stop(timeout).map_err(map)?)?,
        [verb] if verb == "restart" => json_value(service::restart(timeout).map_err(map)?)?,
        [verb] if verb == "status" => json_value(service::status().map_err(map)?)?,
        [verb] if verb == "logs" => service::logs().map_err(map)?,
        [verb] if verb == "doctor" => service::doctor(),
        [autostart, verb] if autostart == "autostart" && verb == "enable" => {
            json_value(service::set_autostart(true).map_err(map)?)?
        }
        [autostart, verb] if autostart == "autostart" && verb == "disable" => {
            json_value(service::set_autostart(false).map_err(map)?)?
        }
        [autostart, verb] if autostart == "autostart" && verb == "status" => {
            json_value(service::status().map_err(map)?)?
        }
        _ => return Err(ErrorCode::InvalidArguments),
    };
    Ok(status)
}

fn gateway_command(
    arguments: &[String],
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
    let layout = StandaloneLayout::from_environment().map_err(|_| ErrorCode::GatewayUnavailable)?;
    hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(|_| ErrorCode::GatewayUnavailable)?;
    let store = GatewayListenerStore::new(layout.gateway_config_root());
    match arguments {
        [verb] if verb == "show" => gateway_status(&store),
        [verb] if verb == "recover" => {
            store.recover().map_err(|_| ErrorCode::GatewayUnavailable)?;
            if service::restart(globals.timeout.unwrap_or(Duration::from_secs(30))).is_err() {
                let _ = store.mark_failed("SERVICE_RESTART_FAILED");
                return Err(ErrorCode::DaemonUnavailable);
            }
            gateway_status(&store)
        }
        [verb, options @ ..] if verb == "set" => {
            let desired = parse_gateway_settings(options)?;
            store
                .configure(desired)
                .map_err(|_| ErrorCode::GatewayUnavailable)?;
            if service::restart(globals.timeout.unwrap_or(Duration::from_secs(30))).is_err() {
                let _ = store.mark_failed("SERVICE_RESTART_FAILED");
                return Err(ErrorCode::DaemonUnavailable);
            }
            gateway_status(&store)
        }
        _ => Err(ErrorCode::InvalidArguments),
    }
}

fn parse_gateway_settings(options: &[String]) -> Result<GatewayListenerDesiredV1, ErrorCode> {
    let mut address = None;
    let mut port = None;
    let mut accept_remote_risk = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--address" => {
                index += 1;
                address = options.get(index).cloned();
            }
            "--port" => {
                index += 1;
                port = options.get(index).cloned();
            }
            "--accept-remote-risk" => accept_remote_risk = true,
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let address = address
        .ok_or(ErrorCode::InvalidArguments)?
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if !address.is_loopback() && !accept_remote_risk {
        return Err(ErrorCode::ActionRequired);
    }
    let desired = match port.as_deref() {
        Some("auto") => GatewayListenerDesiredV1::automatic(address.to_string()),
        Some(value) => GatewayListenerDesiredV1::fixed(
            address.to_string(),
            value
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or(ErrorCode::InvalidArguments)?,
        ),
        None => return Err(ErrorCode::InvalidArguments),
    };
    desired
        .validate()
        .map_err(|_| ErrorCode::InvalidArguments)?;
    Ok(desired)
}

fn gateway_status(store: &GatewayListenerStore) -> Result<serde_json::Value, ErrorCode> {
    let config = store
        .load_or_default()
        .map_err(|_| ErrorCode::GatewayUnavailable)?;
    let applied = config
        .applied_address()
        .map_err(|_| ErrorCode::GatewayUnavailable)?;
    let connect = applied.and_then(|value| connect_address(value).ok());
    let ready = connect.is_some_and(|address| {
        std::net::TcpStream::connect_timeout(&address, Duration::from_millis(750)).is_ok()
    });
    let suggested_agent_base_url = connect.map(|value| format!("http://{value}/v1"));
    let existing_agent_count = discovered_agent_count();
    let update_required = existing_agent_count.is_some_and(|count| count > 0)
        && config.operation.as_ref().is_some_and(|operation| {
            operation.state == hiroute_host_runtime::GatewayListenerOperationStateV1::Succeeded
        });
    let (preview_command_id, apply_command_id) = public_agent_update_commands();
    Ok(json!({
        "schema": "hiroute.gateway-listener-status/v1",
        "config": config,
        "connect_address": connect.map(|value| value.to_string()),
        "ready": ready,
        "agent_update": {
            "required": update_required,
            "existing_agent_count": existing_agent_count,
            "suggested_base_url": suggested_agent_base_url,
            "public_flow_available": preview_command_id.is_some(),
            "preview_command_id": preview_command_id,
            "apply_command_id": apply_command_id,
        },
    }))
}

fn public_agent_update_commands() -> (Option<&'static str>, Option<&'static str>) {
    let release = release_manifest();
    let contains = |id: &str| {
        release
            .commands
            .iter()
            .any(|command| command.command_id == id)
    };
    if contains("agents.connect.preview") && contains("agents.connect.apply") {
        (Some("agents.connect.preview"), Some("agents.connect.apply"))
    } else {
        (None, None)
    }
}

fn discovered_agent_count() -> Option<usize> {
    let request = LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: format!("gateway-agents-{}", std::process::id()),
        operation_id: "ListAgents".into(),
        payload: json!({}),
        protected_grant: None,
    };
    let response = LocalControlClient::cli_from_environment()
        .ok()?
        .with_timeout(Duration::from_millis(750))
        .call(request)
        .ok()?;
    if response.status != MachineStatus::Succeeded {
        return None;
    }
    response.data?.get("agents")?.as_array().map(|agents| {
        agents
            .iter()
            .filter(|agent| {
                agent
                    .get("context_id")
                    .is_some_and(|value| !value.is_null())
            })
            .count()
    })
}

fn protected_input_command(
    arguments: &[String],
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
    let client = LocalControlClient::cli_from_environment()
        .map(|client| client.with_timeout(globals.timeout.unwrap_or(Duration::from_secs(30))))
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let request = match arguments {
        [verb, candidate_flag, candidate, secret_flag, fd]
            if verb == "register"
                && candidate_flag == "--candidate"
                && secret_flag == "--secret-fd" =>
        {
            let fd = fd.parse::<u32>().map_err(|_| ErrorCode::InvalidArguments)?;
            let secret =
                crate::client::read_protected_fd(fd).map_err(|_| ErrorCode::InvalidArguments)?;
            StandaloneProtectedInputRequestV1::register(candidate.clone(), secret)
        }
        [verb, candidate_flag, candidate]
            if verb == "release" && candidate_flag == "--candidate" =>
        {
            StandaloneProtectedInputRequestV1::release(candidate.clone())
        }
        _ => return Err(ErrorCode::InvalidArguments),
    };
    let expected_action = request.action.clone();
    let response = client
        .call_protected_input(request)
        .map_err(|_| ErrorCode::CapabilityDenied)?;
    if response.schema != STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1
        || response.action != expected_action
    {
        return Err(ErrorCode::CapabilityDenied);
    }
    let action = response.action.clone();
    Ok(json!({
        "schema": "hiroute.protected-input-result/v1",
        "action": action,
        "registered": response.registered,
    }))
}

fn map(error: service::ServiceFailure) -> ErrorCode {
    match error {
        service::ServiceFailure::Unavailable => ErrorCode::DaemonUnavailable,
    }
}

fn json_value(value: impl serde::Serialize) -> Result<serde_json::Value, ErrorCode> {
    serde_json::to_value(value).map_err(|_| ErrorCode::Internal)
}

fn subscription_proxy_command(arguments: &[String]) -> Result<serde_json::Value, ErrorCode> {
    use hiroute_host_runtime::{SubscriptionProxyPolicy, SubscriptionProxyStore};
    let layout = StandaloneLayout::from_environment().map_err(|_| ErrorCode::GatewayUnavailable)?;
    hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
        .map_err(|_| ErrorCode::GatewayUnavailable)?;
    let store = SubscriptionProxyStore::new(layout.gateway_config_root());
    match arguments {
        [verb] if verb == "show" => {
            let mut view = store.view().map_err(|_| ErrorCode::GatewayUnavailable)?;
            view.applied &= service::status().is_ok_and(|status| status.local_control_ready);
            json_value(view)
        }
        [verb, options @ ..] if verb == "set" => {
            let mut mode = None;
            let mut url = None;
            let mut no_proxy = None;
            for pair in options.chunks(2) {
                let [key, value] = pair else {
                    return Err(ErrorCode::InvalidArguments);
                };
                let slot = match key.as_str() {
                    "--mode" => &mut mode,
                    "--url" => &mut url,
                    "--no-proxy" => &mut no_proxy,
                    _ => return Err(ErrorCode::InvalidArguments),
                };
                if slot.replace(value.clone()).is_some() {
                    return Err(ErrorCode::InvalidArguments);
                }
            }
            let policy = match mode.as_deref() {
                Some("inherit") if url.is_none() && no_proxy.is_none() => {
                    SubscriptionProxyPolicy::Inherit
                }
                Some("direct") if url.is_none() && no_proxy.is_none() => {
                    SubscriptionProxyPolicy::Direct
                }
                Some("manual") => SubscriptionProxyPolicy::Manual {
                    url: url.ok_or(ErrorCode::InvalidArguments)?,
                    no_proxy: no_proxy.unwrap_or_default(),
                },
                _ => return Err(ErrorCode::InvalidArguments),
            };
            policy.validate().map_err(|_| ErrorCode::InvalidArguments)?;
            let config = store
                .configure(policy)
                .map_err(|_| ErrorCode::GatewayUnavailable)?;
            Ok(json!({"config":config,"applied":false,"action":"service_restart_required"}))
        }
        _ => Err(ErrorCode::InvalidArguments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_parser_requires_explicit_remote_risk_and_rejects_ipv6() {
        let values = |items: &[&str]| {
            items
                .iter()
                .map(|value| (*value).into())
                .collect::<Vec<_>>()
        };
        assert!(
            parse_gateway_settings(&values(&["--address", "127.0.0.1", "--port", "auto"])).is_ok()
        );
        assert_eq!(
            parse_gateway_settings(&values(&["--address", "0.0.0.0", "--port", "8080"])),
            Err(ErrorCode::ActionRequired)
        );
        assert!(
            parse_gateway_settings(&values(&[
                "--address",
                "0.0.0.0",
                "--port",
                "8080",
                "--accept-remote-risk"
            ]))
            .is_ok()
        );
        assert_eq!(
            parse_gateway_settings(&values(&["--address", "::1", "--port", "auto"])),
            Err(ErrorCode::InvalidArguments)
        );
    }

    #[test]
    fn host_command_help_is_available_without_starting_a_service() {
        for family in [
            "service",
            "gateway",
            "protected-input",
            "subscription-proxy",
        ] {
            let execution = execute(
                &[family.to_owned()],
                &Globals {
                    help: true,
                    ..Globals::default()
                },
            )
            .unwrap();
            assert_eq!(execution.exit_code, 0);
            assert!(execution.stdout.contains("Usage"));
            assert!(execution.stdout.contains(family));
        }
    }

    #[test]
    fn gateway_status_advertises_released_agent_update_commands() {
        assert_eq!(
            public_agent_update_commands(),
            (Some("agents.connect.preview"), Some("agents.connect.apply"))
        );
    }
}
