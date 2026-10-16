use super::*;

pub(super) fn run(flags: &Flags, globals: &Globals) -> Result<Reply> {
    let mut view = json!({
        "schema":"hiroute.cli-quickstart/v1", "effects":[],
        "caller":"local_cli_user", "selected_context":flags.get("--context"), "selected_plan":flags.get("--plan"),
        "service":{"state":"unknown"}, "targets":[], "sources":[], "plans":[],
        "required_choices":[], "blockers":[], "next_commands":[],
        "independent_facets":["model","management_skill","collaboration"]
    });
    let client = match client(globals) {
        Ok(client) => client,
        Err(mut response) => {
            unavailable(&mut view);
            response.data = Some(view);
            return Err(response);
        }
    };
    let service = match data(&client, "GetSystemStatus", json!({})) {
        Ok(service) => service,
        Err(mut response) => {
            unavailable(&mut view);
            response.data = Some(view);
            return Err(response);
        }
    };
    view["service"] = service;
    view["decision_options"] = decision::options(Some(&client));
    let targets = settings::targets(&client, flags.get("--hermes-root"))?;
    view["targets"] = json!(targets);
    let compute = data(&client, "ListCompute", json!({}))?;
    view["sources"] = compute["sources"].clone();
    let plans = data(&client, "ListAgentPlanCatalog", json!({"limit":128}))?;
    view["plans"] = plans["plans"].clone();
    view["plans_next_cursor"] = plans["next_cursor"].clone();
    let mut required = Vec::new();
    let mut next = Vec::new();
    next.push(json!({"arguments":["decision","services","options"],"effect":"read_only","reason":"reuse_saved_decision_connection_or_choose_supported_onboarding; local_rules_remains_an_explicit_alternative"}));
    if view["service"]["control_store"] != "ready"
        || view["service"]["recoverable_operations"]
            .as_u64()
            .is_some_and(|count| count > 0)
    {
        view["blockers"] = json!(["inspect_control_store_or_recoverable_operations"]);
        next.push(json!({"arguments":["service","doctor"],"effect":"read_only","reason":"inspect_service_recovery"}));
    }
    let target = match flags.get("--context") {
        Some(context) => Some(settings::select(&targets, context)?),
        None => {
            required.push("--context");
            None
        }
    };
    if let Some(plan) = flags.get("--plan") {
        AgentPlanId::parse(plan).map_err(|_| error(ErrorCode::InvalidArguments))?;
        view["selected_plan_state"] =
            data(&client, "GetAgentPlanStatus", json!({"agent_plan_id":plan}))?;
    } else {
        required.push("--plan (reuse) OR routing preview (create)");
        next.push(json!({"arguments":["routing","preview","--help"],"effect":"read_only","reason":"choose_fixed_or_smart_route_directly"}));
    }
    if compute["sources"].as_array().is_some_and(Vec::is_empty) {
        next.push(json!({"arguments":["compute","scan"],"effect":"read_only_discovery","reason":"discover_connection_points_then_explicitly_prepare_credentials"}));
        next.push(json!({"arguments":["compute","connection","prepare","--help"],"effect":"read_only_help","reason":"preparation_is_a_separate_explicit_step"}));
    }
    if let (Some(context), Some(target)) = (flags.get("--context"), target.as_ref()) {
        let status = settings::status(&client, context, target, flags.get("--plan"))?;
        view["connection"] = status.clone();
        if let Some(plan) = flags.get("--plan") {
            let known = status["effective"]["request_evidence"]["state"] == "known";
            let configured = status["state"] == "configured";
            view["stage"] = json!(if known {
                "inspect_recorded_outcome"
            } else if configured {
                "native_activation_or_verification_pending"
            } else {
                "connection_preparation_required"
            });
            if configured {
                next.push(json!({"arguments":["agents","connect","status","--effective","--context",context,"--plan",plan],"effect":"read_only","reason":"inspect_exact_request_evidence; native_actions_are_in_connection.effective.next_actions"}));
            } else {
                next.push(json!({"arguments":["agents","connect","preview","--help"],"effect":"read_only","reason":"choose_native_protocol_preset_or_default; preserve_independent_facets"}));
            }
        }
    }
    if target.is_none() {
        next.push(json!({"arguments":["agents","scan","--help"],"effect":"read_only","reason":"choose_exact_context; Hermes_custom_roots_use_quickstart_--hermes-root"}));
    }
    view["required_choices"] = json!(required);
    view["next_commands"] = json!(next);
    view["reuse_guidance"] = json!(
        "Reuse exact source/Plan/context IDs. Names may repeat; configuration is not native loading or successful inference. Keep prepared Apply files and query their Operation identity before retrying uncertain work."
    );
    Ok(MachineEnvelopeV2::succeeded(
        view,
        globals.request_id.clone(),
    ))
}

fn unavailable(view: &mut Value) {
    view["service"] = json!({"state":"unavailable_or_incompatible"});
    view["blockers"] = json!(["local_control_unavailable"]);
    view["next_commands"] = json!([
        {"arguments":["service","status"],"effect":"read_only","reason":"inspect_current_installation"},
        {"arguments":["service","doctor"],"effect":"read_only","reason":"inspect_recovery_without_replacing_data"},
        {"arguments":["service","start"],"effect":"starts_local_service","reason":"only_if_current_installation_is_stopped; do_not_start_a_second_daemon"}
    ]);
}
