use super::*;

pub(super) fn targets(client: &LocalControlClient, root: Option<&str>) -> Result<Vec<Value>> {
    let mut agents = Vec::new();
    for (operation, payload) in [
        ("ListAgents", json!({})),
        (
            "ScanAgents",
            root.map_or_else(
                || json!({}),
                |root| json!({"native_target":{"kind":"hermes","config_root":root}}),
            ),
        ),
    ] {
        let found = data(client, operation, payload)?;
        for agent in found["agents"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::Internal))?
        {
            if let Some(index) = agents.iter().position(|a: &Value| {
                a["context_id"].is_string() && a["context_id"] == agent["context_id"]
            }) {
                agents[index] = agent.clone();
            } else if !agents.contains(agent) {
                agents.push(agent.clone());
            }
        }
    }
    Ok(agents)
}

pub(super) fn select(agents: &[Value], context: &str) -> Result<Value> {
    let found = agents
        .iter()
        .filter(|agent| {
            agent["context_id"] == context
                || agent["codex_access"]["profile_context_id"] == context
                || agent["codex_access"]["root_context_id"] == context
        })
        .collect::<Vec<_>>();
    // Multiple identities with the same display name are deliberately not candidates here.
    let [agent] = found.as_slice() else {
        return Err(choices(
            &["exact_available_context"],
            json!({"selected_context":context,"targets":agents}),
        ));
    };
    Ok((*agent).clone())
}

pub(super) fn status(
    client: &LocalControlClient,
    context: &str,
    target: &Value,
    plan: Option<&str>,
) -> Result<Value> {
    let mut payload = json!({"schema_version":AGENT_SETTINGS_SCHEMA_V2,"context_id":context,"include_skills":true});
    if !target["native_target"].is_null() {
        payload["native_target"] = target["native_target"].clone();
    }
    if let Some(plan) = plan {
        payload["effective"] = json!({"plan_id":plan});
    }
    data(client, "GetAgentConnectionStatus", payload)
}

pub(super) fn prepare(
    client: &LocalControlClient,
    command: &str,
    options: &[String],
) -> Result<Value> {
    let restore = command == "agents.restore.preview";
    let flags = if restore {
        Flags::parse(
            options,
            &["--context", "--hermes-root", "--restore-native-model"],
            &["--facet"],
            &[],
        )?
    } else {
        Flags::parse(
            options,
            &[
                "--context",
                "--hermes-root",
                "--plan",
                "--protocol",
                "--native-models",
                "--native-default",
                "--management",
                "--collaboration",
            ],
            &["--preset"],
            &[],
        )?
    };
    let context = flags.required("--context")?;
    let targets = targets(client, flags.get("--hermes-root"))?;
    let target = select(&targets, context)?;
    let current = status(client, context, &target, None)?;
    let mut spec = json!({"schema_version":AGENT_SETTINGS_SCHEMA_V2,"context_id":context});
    if !target["native_target"].is_null() {
        spec["native_target"] = target["native_target"].clone();
    }
    if restore {
        if flags.all("--facet").is_empty() {
            return Err(choices(
                &["--facet"],
                json!(["model", "management_skill", "collaboration"]),
            ));
        }
        for facet in flags.all("--facet") {
            if !matches!(
                facet.as_str(),
                "model" | "management_skill" | "collaboration"
            ) {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let reference = if facet == "model" {
                &current["restore_point_ref"]
            } else {
                &current[facet]["restore_point_ref"]
            };
            let reference = reference
                .as_str()
                .ok_or_else(|| choices(&["owned_restore_point"], current.clone()))?;
            spec[facet] = json!({"intent":"restore","restore_point_ref":reference});
        }
        if let Some(model) = flags.get("--restore-native-model") {
            spec["restore_native_model"] = json!(model);
        }
    } else {
        if let Some(plan) = flags.get("--plan") {
            AgentPlanId::parse(plan).map_err(|_| error(ErrorCode::InvalidArguments))?;
            // The backend status and Preview remain authoritative for availability/protocols.
            let selection = selection(&target, &current, &flags, plan)?;
            spec["model"] = json!({"intent":"configure","settings":selection});
        } else if [
            "--protocol",
            "--native-models",
            "--native-default",
            "--preset",
        ]
        .iter()
        .any(|flag| flags.has(flag))
        {
            return Err(choices(&["--plan"], json!({})));
        }
        if let Some(value) = flags.get("--management") {
            match value {
                "enable" => spec["management_skill"] = json!({"intent":"configure","settings":{}}),
                "keep" => {}
                _ => return Err(error(ErrorCode::InvalidArguments)),
            }
        }
        if let Some(value) = flags.get("--collaboration") {
            match value {
                "explicit" | "delegate_by_default" => {
                    spec["collaboration"] =
                        json!({"intent":"configure","settings":{"trigger_mode":value}})
                }
                "keep" => {}
                _ => return Err(error(ErrorCode::InvalidArguments)),
            }
        }
        if ["model", "management_skill", "collaboration"]
            .iter()
            .all(|facet| spec[facet].is_null())
        {
            return Err(choices(
                &["--plan OR --management enable OR --collaboration"],
                json!({}),
            ));
        }
    }
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(spec).map_err(|_| error(ErrorCode::InvalidArguments))?;
    Ok(json!(AgentSettingsPreviewRequestV2 { spec }))
}

fn selection(target: &Value, current: &Value, flags: &Flags, plan: &str) -> Result<Value> {
    let agent = target["agent_id"]
        .as_str()
        .ok_or_else(|| error(ErrorCode::Internal))?;
    let mut old = current["current_selection"].clone();
    let protocol = flags.get("--protocol");
    if protocol
        .is_some_and(|protocol| !matches!(protocol, "responses" | "chat_completions" | "messages"))
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    if agent != "agent_claude_default" && flags.has("--preset")
        || agent != "agent_codex_default"
            && (flags.has("--native-models") || flags.has("--native-default"))
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    match agent {
        "agent_hermes_default" => Ok(
            json!({"mode":"hermes_default","plan_id":plan,"protocol":flags.required("--protocol")?}),
        ),
        "agent_codex_default" => {
            if protocol.is_some() {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let native = flags.get("--native-models").ok_or_else(|| {
                choices(
                    &["--native-models"],
                    json!(["preserve_available", "hiroute_only"]),
                )
            })?;
            let default = flags.get("--native-default").ok_or_else(|| {
                choices(&["--native-default"], json!(["preserve_native", "plan"]))
            })?;
            if !matches!(native, "preserve_available" | "hiroute_only")
                || !matches!(default, "preserve_native" | "plan")
            {
                return Err(error(ErrorCode::InvalidArguments));
            }
            if old["mode"] != "codex_default" {
                old = json!({"mode":"codex_default","fixed_models":[],"allowed_plan_ids":[]});
            }
            old["native_model_mode"] = json!(native);
            old["default_selection"] = if default == "plan" {
                json!({"kind":"plan","plan_id":plan})
            } else {
                json!({"kind":"preserve_native"})
            };
            add_plan(&mut old, plan)?;
            Ok(old)
        }
        "agent_claude_default" => {
            if protocol.is_some() {
                return Err(error(ErrorCode::InvalidArguments));
            }
            if flags.all("--preset").is_empty() {
                return Err(choices(&["--preset"], json!(["opus", "sonnet", "haiku"])));
            }
            if old["mode"] != "claude_launcher" {
                old = json!({"mode":"claude_launcher","surfaces":["claude_cli"],"fixed_models":[],"preset_mappings":{
                    "opus":{"kind":"preserve_native"},"sonnet":{"kind":"preserve_native"},"haiku":{"kind":"preserve_native"}}});
            }
            for preset in flags.all("--preset") {
                if !matches!(preset.as_str(), "opus" | "sonnet" | "haiku") {
                    return Err(error(ErrorCode::InvalidArguments));
                }
                old["preset_mappings"][preset] = json!({"kind":"plan","plan_id":plan});
            }
            Ok(old)
        }
        "agent_qoder_default" | "agent_pi_default" | "agent_dsh_default" => {
            let protocol = flags.required("--protocol")?;
            let mode = match agent {
                "agent_qoder_default" => "qoder_additional",
                "agent_pi_default" => "pi_additional",
                _ => "dsh_additional",
            };
            if old["mode"] != mode {
                old = json!({"mode":mode,"allowed_plan_ids":[],"plan_protocols":{}});
            }
            if old["plan_protocols"].is_null() {
                old["plan_protocols"] = json!({});
            }
            add_plan(&mut old, plan)?;
            old["plan_protocols"][plan] = json!(protocol);
            Ok(old)
        }
        _ => Err(choices(&["supported_agent_target"], target.clone())),
    }
}
fn add_plan(selection: &mut Value, plan: &str) -> Result<()> {
    let plans = selection["allowed_plan_ids"]
        .as_array_mut()
        .ok_or_else(|| error(ErrorCode::Internal))?;
    if !plans.iter().any(|id| id == plan) {
        plans.push(json!(plan));
    }
    Ok(())
}
