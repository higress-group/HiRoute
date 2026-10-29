use std::time::Duration;

use hiroute_application_api::{
    AGENT_CONNECTION_STATUS_SCHEMA_V1, AGENT_SETTINGS_SCHEMA_V2, AgentConnectionStateV1,
    AgentConnectionStatusRequestV1, AgentConnectionStatusV1, AgentModelSettingsStateV2,
    AgentSettingsStatusRequestV2, AgentSettingsStatusV2, ErrorCode, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlWireRequestV2, MachineStatus, STANDALONE_PROTECTED_INPUT_RESPONSE_SCHEMA_V1,
    StandaloneProtectedInputRequestV1, release_manifest,
};
use hiroute_host_runtime::{
    GatewayListenerDesiredV1, GatewayListenerStore, StandaloneLayout, connect_address,
};
use serde_json::json;

use crate::args::{Globals, OutputMode};
use crate::{CliExecution, LocalControlClient, failure, service, success};

mod console;

pub(crate) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    let family = arguments.first()?.as_str();
    if family == "console" {
        return Some(console::execute(&arguments[1..], globals));
    }
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
    if arguments == ["service", "recovery"]
        && globals.output == OutputMode::Text
        && globals.agent.is_none()
        && !globals.has_capability()
    {
        return Some(match service::recovery() {
            Ok(value)=>CliExecution {exit_code:0,stdout:format!("HiRoute startup recovery\n\n{}\n\nStartup diagnostics: {}\n",value["instructions"].as_str().unwrap_or("No verified startup recovery report is available. Keep all data and inspect startup diagnostics; this does not establish service readiness or permission to downgrade."),value["diagnostics_root"].as_str().unwrap_or("unavailable")),stderr:String::new()},
            Err(error)=>failure(map(error),request_id),
        });
    }
    if globals.agent.is_some() || globals.has_capability() || globals.output != OutputMode::Json {
        return Some(failure(ErrorCode::InvalidArguments, request_id));
    }
    let result = match family {
        "service" => service_command(&arguments[1..], globals),
        "subscription-proxy" => subscription_proxy_command(&arguments[1..]),
        "gateway" => gateway_command(&arguments[1..], globals),
        "protected-input" => protected_input_command(&arguments[1..], globals),
        _ => unreachable!(),
    };
    let mut execution = match result {
        Ok(value) => success(value, request_id),
        Err(error) => failure(error, request_id),
    };
    if execution.exit_code == 6
        && family == "service"
        && matches!(
            arguments.get(1).map(String::as_str),
            Some("start" | "restart")
        )
        && service::recovery().is_ok_and(|report| report["recovery"].is_object())
    {
        execution.stderr.push_str("Storage startup needs attention. Run `hiroute service recovery --output text` for the cause, safe next action and diagnostic location. Keep all data and packages.\n");
    }
    Some(execution)
}

fn host_help(family: &str) -> &'static str {
    match family {
        "service" => {
            "Usage\n  hiroute service start|stop|restart|status|logs|doctor|recovery --output json\n  hiroute service autostart enable|disable|status --output json\n  hiroute service recovery [--output text|json]\n  hiroute service run\n  hiroute service layout --output json\n"
        }
        "gateway" => {
            "Usage\n  hiroute gateway show --output json\n  hiroute gateway set --address <IPV4> --port auto|<PORT> [--accept-remote-risk] --output json\n  hiroute gateway recover --output json\n"
        }
        "subscription-proxy" => {
            "Usage\n  hiroute subscription-proxy show --output json\n  hiroute subscription-proxy set --mode inherit|direct|manual [--url <HTTP_PROXY_URL>] [--no-proxy <BYPASS_LIST>] --output json\n  Restart the service to apply saved settings: hiroute service restart --output json\n"
        }
        "protected-input" => {
            #[cfg(windows)]
            {
                return "Usage\n  hiroute protected-input register --candidate <REF> --secret-handle <INHERITED_HANDLE> --output json\n  hiroute protected-input release --candidate <REF> --output json\nThe inherited handle must name a private non-stdio pipe; payload and EOF are bounded.\n";
            }
            #[cfg(not(windows))]
            "Usage\n  hiroute protected-input register --candidate <REF> --secret-fd <FD> --output json\n  hiroute protected-input register --candidate <REF> --prompt --format bearer|raw --output json\n  hiroute protected-input release --candidate <REF> --output json\n\n--prompt is a human step in the user's own terminal (for example SSH), outside chat and Agent tools. It reads hidden input from the controlling terminal only; no stdin fallback. bearer accepts a bare API key and adds Bearer; raw accepts the complete custom header value. Never put credentials in argv, environment, normal JSON or chat. Cancellation/missing terminal saves no service. Continue using the candidate reference after registered=true. The existing --secret-fd value remains unchanged.\n"
        }
        _ => unreachable!(),
    }
}

fn service_command(
    arguments: &[String],
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
    // Only the private Rust transaction is an installation entry. The dormant
    // manager adapters remain in the service module for low-level SCM tests.
    if arguments.first().is_some_and(|verb| {
        matches!(
            verb.as_str(),
            "install" | "uninstall" | "prepare-install" | "publish-install" | "write-record"
        )
    }) {
        return Err(ErrorCode::InvalidArguments);
    }
    let timeout = globals.timeout.unwrap_or(Duration::from_secs(30));
    let status = match arguments {
        [verb] if verb == "start" => json_value(service::start(timeout).map_err(map)?)?,
        [verb] if verb == "stop" => json_value(service::stop(timeout).map_err(map)?)?,
        [verb] if verb == "restart" => json_value(service::restart(timeout).map_err(map)?)?,
        [verb] if verb == "status" => json_value(service::status().map_err(map)?)?,
        [verb] if verb == "logs" => service::logs().map_err(map)?,
        [verb] if verb == "recovery" => service::recovery().map_err(map)?,
        [verb] if verb == "doctor" => service::doctor(),
        [verb] if verb == "layout" => service::windows_layout().map_err(map)?,
        [verb, manager_flag, manager, daemon_flag, daemon]
            if verb == "describe-install"
                && manager_flag == "--manager"
                && daemon_flag == "--daemon" =>
        {
            service::describe_windows_manager(manager, std::path::Path::new(daemon)).map_err(map)?
        }
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
        [verb] if verb == "show" => gateway_status(&store, globals),
        [verb] if verb == "recover" => {
            store.recover().map_err(|_| ErrorCode::GatewayUnavailable)?;
            if service::restart(globals.timeout.unwrap_or(Duration::from_secs(30))).is_err() {
                let _ = store.mark_failed("SERVICE_RESTART_FAILED");
                return Err(ErrorCode::DaemonUnavailable);
            }
            gateway_status(&store, globals)
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
            gateway_status(&store, globals)
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

fn gateway_status(
    store: &GatewayListenerStore,
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
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
    let client = LocalControlClient::cli_from_environment()
        .ok()
        // Native discovery can run executable probes; use the normal control-call deadline.
        .map(|client| client.with_timeout(globals.timeout.unwrap_or(Duration::from_secs(30))));
    let (update_required, existing_agent_count) = agent_update_status(|operation_id, payload| {
        let response = client
            .as_ref()
            .ok_or(ErrorCode::DaemonUnavailable)?
            .call(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: format!("gateway-agents-{}", std::process::id()),
                operation_id: operation_id.into(),
                payload,
                protected_grant: None,
            })
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        if response.status == MachineStatus::Succeeded {
            response.data.ok_or(ErrorCode::Internal)
        } else {
            Err(response
                .error
                .map_or(ErrorCode::Internal, |error| error.code))
        }
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

fn agent_update_status(
    mut query: impl FnMut(&str, serde_json::Value) -> Result<serde_json::Value, ErrorCode>,
) -> (bool, Option<usize>) {
    let Ok(list) = query("ListAgents", json!({})) else {
        return (true, None);
    };
    let Some(agents) = list.get("agents").and_then(serde_json::Value::as_array) else {
        return (true, None);
    };
    let mut contexts = std::collections::BTreeSet::new();
    let mut legacy_connections = std::collections::BTreeSet::new();
    for agent in agents {
        match agent.get("context_id") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(context)) if !context.is_empty() => {
                contexts.insert(context.clone());
                let Some(agent_id) = agent
                    .get("agent_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    return (true, None);
                };
                let Some(profile_id) = agent
                    .get("profile_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    return (true, None);
                };
                legacy_connections.insert((
                    context,
                    format!("agent-connection/{agent_id}/{profile_id}"),
                    agent_id,
                    profile_id,
                ));
            }
            _ => return (true, None),
        }
    }
    let mut required = false;
    let mut legacy_contexts = std::collections::BTreeSet::new();
    for context in &contexts {
        let status = query(
            "GetAgentConnectionStatus",
            json!(AgentSettingsStatusRequestV2 {
                projection: Default::default(),
                effective: None,
                include_skills: true,
                native_target: None,
                schema_version: AGENT_SETTINGS_SCHEMA_V2,
                context_id: context.clone(),
            }),
        )
        .ok()
        .and_then(|value| serde_json::from_value::<AgentSettingsStatusV2>(value).ok())
        .filter(|status| status.valid() && status.context_id() == context);
        // Unavailable inspection is not evidence that installed endpoints are current.
        if status
            .as_ref()
            .and_then(AgentSettingsStatusV2::model)
            .is_some_and(|model| {
                matches!(
                    model.state,
                    AgentModelSettingsStateV2::NotConfigured | AgentModelSettingsStateV2::Restored
                )
            })
        {
            legacy_contexts.insert(context.clone());
        }
        required |= status.is_none_or(|status| {
            status.model().is_some_and(|model| {
                matches!(
                    model.state,
                    AgentModelSettingsStateV2::Drift
                        | AgentModelSettingsStateV2::Pending
                        | AgentModelSettingsStateV2::NeedsAttention
                )
            })
        });
    }
    // Released V1 connections use installation/profile IDs, not V2 context IDs. An absent
    // journal is normal for a fresh installation; an unreadable journal is not a clean result.
    // A current configured V2 target has already proved convergence. Its superseded V1
    // grant can remain in history, so only unconfigured/restored targets need this fallback.
    for (context, connection_id, agent_id, profile_id) in legacy_connections {
        if !legacy_contexts.contains(context) {
            continue;
        }
        let status = query(
            "GetAgentConnectionStatus",
            json!(AgentConnectionStatusRequestV1 {
                connection_id: connection_id.clone(),
            }),
        );
        if matches!(status, Err(ErrorCode::ResourceNotFound)) {
            continue;
        }
        let status = status
            .ok()
            .and_then(|value| serde_json::from_value::<AgentConnectionStatusV1>(value).ok())
            .filter(|status| {
                status.schema == AGENT_CONNECTION_STATUS_SCHEMA_V1
                    && status.connection_id == connection_id
                    && status.agent_id == agent_id
                    && status.profile_id == profile_id
            });
        required |= status.is_none_or(|status| {
            status.state == AgentConnectionStateV1::Disconnected
                || (status.state == AgentConnectionStateV1::Active && !status.launch_ready)
        });
    }
    (required, Some(contexts.len()))
}

fn protected_input_command(
    arguments: &[String],
    globals: &Globals,
) -> Result<serde_json::Value, ErrorCode> {
    let client = LocalControlClient::cli_from_environment()
        .map(|client| client.with_timeout(globals.timeout.unwrap_or(Duration::from_secs(30))))
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let request = match arguments {
        [verb, candidate_flag, candidate, prompt, format_flag, format]
            if verb == "register"
                && candidate_flag == "--candidate"
                && prompt == "--prompt"
                && format_flag == "--format" =>
        {
            StandaloneProtectedInputRequestV1::register(
                candidate.clone(),
                crate::protected_prompt::read(format)?,
            )
        }
        #[cfg(windows)]
        [verb, candidate_flag, candidate, secret_flag, handle]
            if verb == "register"
                && candidate_flag == "--candidate"
                && secret_flag == "--secret-handle" =>
        {
            let handle = handle
                .parse::<u64>()
                .map_err(|_| ErrorCode::InvalidArguments)?;
            let secret = crate::client::read_protected_handle(handle, 512)
                .map_err(|_| ErrorCode::InvalidArguments)?;
            StandaloneProtectedInputRequestV1::register(candidate.clone(), secret)
        }
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
    fn windows_installation_has_no_public_manager_or_record_write_entry() {
        assert!(!host_help("service").contains("--manager"));
        for words in [
            vec!["install", "--manager", "scm", "--password-stdin"],
            vec!["install", "--manager", "task"],
            vec!["uninstall"],
            vec!["publish-install", "--request-stdin"],
            vec!["prepare-install"],
            vec!["write-record", "--path", "foreign", "--request-stdin"],
        ] {
            let args = words.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(matches!(
                service_command(&args, &Globals::default()),
                Err(ErrorCode::InvalidArguments)
            ));
        }
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

    #[test]
    fn agent_update_hint_tracks_current_model_state_and_unknown_reads() {
        let context = "agent-context/pi";
        for (state, required) in [
            ("not_configured", false),
            ("restored", false),
            ("configured", false),
            ("drift", true),
            ("pending", true),
            ("needs_attention", true),
        ] {
            let status = json!({
                "schema":"hiroute.agent-model-settings-status/v2", "context_id":context,
                "state":state, "operation_id":null, "operation_state":null,
                "restore_point_ref":null, "applied_revision":null,
                "surface_results":[], "model_verified":false,
            });
            assert_eq!(
                agent_update_status(|operation, payload| {
                    if operation == "ListAgents" {
                        Ok(discovered_agent(context))
                    } else if payload.get("connection_id").is_some() {
                        Err(ErrorCode::ResourceNotFound)
                    } else {
                        assert_eq!(operation, "GetAgentConnectionStatus");
                        assert_eq!(payload["context_id"], context);
                        Ok(status.clone())
                    }
                }),
                (required, Some(1)),
                "{state}"
            );
        }
        assert_eq!(
            agent_update_status(|_, _| Err(ErrorCode::DaemonUnavailable)),
            (true, None)
        );
        assert_eq!(
            agent_update_status(|operation, _| {
                if operation == "ListAgents" {
                    Ok(discovered_agent(context))
                } else {
                    Err(ErrorCode::DaemonUnavailable)
                }
            }),
            (true, Some(1))
        );
        assert_eq!(
            agent_update_status(|_, _| Ok(json!({"agents":[{}, {"context_id":null}]}))),
            (false, Some(0))
        );
    }

    #[test]
    fn collaboration_only_and_duplicate_targets_do_not_require_model_updates() {
        let context = "agent-context/qoder";
        let (required, count) = agent_update_status(|operation, payload| {
            assert!(
                payload.get("connection_id").is_none(),
                "collaboration-only has no model fallback"
            );
            Ok(if operation == "ListAgents" {
                let mut list = discovered_agent(context);
                let agent = list["agents"][0].clone();
                list["agents"].as_array_mut().unwrap().push(agent);
                list
            } else {
                json!({"schema":"hiroute.agent-collaboration-only-settings-status/v2",
                "context_id":context, "collaboration":{
                    "schema":"hiroute.agent-collaboration-settings-status/v2",
                    "state":"configured", "operation_id":null,"operation_state":null,
                    "restore_point_ref":null,
                }})
            })
        });
        assert!(!required);
        assert_eq!(count, Some(1));
        assert_eq!(
            agent_update_status(|operation, _| Ok(if operation == "ListAgents" {
                discovered_agent(context)
            } else {
                json!({"schema":"unknown","context_id":context})
            })),
            (true, Some(1))
        );
    }

    fn discovered_agent(context: &str) -> serde_json::Value {
        json!({"agents":[{"context_id":context,"agent_id":"agent_claude_default","profile_id":"claude-messages-v1"}, {"context_id":null}]})
    }

    #[test]
    fn legacy_connections_are_checked_after_unconfigured_or_restored_settings() {
        let context = "agent-context/claude";
        let connection = "agent-connection/agent_claude_default/claude-messages-v1";
        for current in ["not_configured", "restored", "configured"] {
            for (legacy, ready, expected) in [
                ("active", true, false),
                ("active", false, true),
                ("disconnected", false, true),
                ("revoked", false, false),
            ] {
                let mut legacy_reads = 0;
                let result = agent_update_status(|operation, payload| {
                    if operation == "ListAgents" {
                        Ok(discovered_agent(context))
                    } else if payload.get("context_id").is_some() {
                        Ok(
                            json!({"schema":"hiroute.agent-model-settings-status/v2","context_id":context,"state":current,
                            "operation_id":null,"operation_state":null,"restore_point_ref":null,"applied_revision":null,
                            "surface_results":[],"model_verified":false}),
                        )
                    } else {
                        legacy_reads += 1;
                        assert_eq!(payload, json!({"connection_id":connection}));
                        Ok(
                            json!({"schema":AGENT_CONNECTION_STATUS_SCHEMA_V1,"connection_id":connection,
                            "agent_id":"agent_claude_default","profile_id":"claude-messages-v1","installed_version":"fixture",
                            "state":legacy,"activation_mode":"managed_configuration","revision":1,"launch_ready":ready}),
                        )
                    }
                });
                assert_eq!(
                    result,
                    (current != "configured" && expected, Some(1)),
                    "{current}/{legacy}/{ready}"
                );
                assert_eq!(legacy_reads, usize::from(current != "configured"));
            }
        }
        for (legacy, expected) in [
            (Err(ErrorCode::ResourceNotFound), false),
            (Err(ErrorCode::DaemonUnavailable), true),
            (Ok(json!({"schema":"unknown"})), true),
        ] {
            assert_eq!(
                agent_update_status(|operation, payload| {
                    if operation == "ListAgents" {
                        Ok(discovered_agent(context))
                    } else if payload.get("context_id").is_some() {
                        Ok(
                            json!({"schema":"hiroute.agent-model-settings-status/v2","context_id":context,"state":"not_configured",
                        "operation_id":null,"operation_state":null,"restore_point_ref":null,"applied_revision":null,
                        "surface_results":[],"model_verified":false}),
                        )
                    } else {
                        legacy.clone()
                    }
                }),
                (expected, Some(1))
            );
        }
    }
}
