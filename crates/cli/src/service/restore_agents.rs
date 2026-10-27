//! Uninstall reuses each context's ordinary restore Preview/Apply; no bulk authority.
use super::restore_plan;
use crate::LocalControlClient;
use hiroute_application_api::*;
use serde_json::{Value, json};
use std::{
    io,
    time::{Duration, Instant},
};

fn call(client: &LocalControlClient, operation: &str, payload: Value) -> io::Result<Value> {
    let envelope = client
        .call(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "windows-uninstall-restore".into(),
            operation_id: operation.into(),
            payload,
            protected_grant: None,
        })
        .map_err(|_| io::Error::other("AGENT_CONTROL_UNAVAILABLE"))?;
    if envelope.error.is_some()
        || !matches!(
            envelope.status,
            MachineStatus::Succeeded | MachineStatus::Accepted
        )
    {
        return Err(io::Error::other("AGENT_RESTORE_UNAVAILABLE"));
    }
    envelope
        .data
        .ok_or_else(|| io::Error::other("AGENT_RESTORE_UNAVAILABLE"))
}
fn preview(client: &LocalControlClient, spec: &AgentSettingsSpecV2) -> io::Result<Value> {
    let value = call(
        client,
        "PreviewAgentConnectionRestore",
        json!({"spec":spec}),
    )?;
    let returned: AgentSettingsSpecV2 =
        serde_json::from_value(value["spec"].clone()).map_err(io::Error::other)?;
    if value["schema"] != "hiroute.agent-settings-preview/v2"
        || value["applicable"] != true
        || value["blockers"].as_array().is_none_or(|b| !b.is_empty())
        || returned.context_id != spec.context_id
        || returned.native_target != spec.native_target
        || returned.management_skill != spec.management_skill
        || returned.model != spec.model
        || returned.collaboration != spec.collaboration
        || returned.restore_native_model != spec.restore_native_model
        || !returned.is_restore_only()
    {
        return Err(io::Error::other("AGENT_RESTORE_DECISION_REQUIRED"));
    }
    Ok(value)
}
pub(super) fn restore(layout: &hiroute_host_runtime::StandaloneLayout) -> io::Result<()> {
    // The caller has admitted the Task registration and starts it for these production queries.
    let client = LocalControlClient::cli_from_environment()
        .map_err(|_| io::Error::other("AGENT_CONTROL_UNAVAILABLE"))?
        .with_timeout(Duration::from_secs(30));
    let until = Instant::now() + Duration::from_secs(30);
    let list = loop {
        match call(&client, "ListAgents", json!({})) {
            Ok(value) => break value,
            Err(error) if Instant::now() < until => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Err(io::Error::other("AGENT_CONTROL_UNAVAILABLE")),
        }
    };
    let _ = layout;
    let contexts = restore_plan::contexts(&list)?;
    let mut blocked = vec![];
    let mut specs = vec![];
    for context in &contexts {
        let name = &context.agent;
        let status = call(
            &client,
            "GetAgentConnectionStatus",
            json!(AgentSettingsStatusRequestV2 {
                projection: Default::default(),
                schema_version: AGENT_SETTINGS_SCHEMA_V2,
                context_id: context.id.clone(),
                native_target: context.native_target.clone(),
                include_skills: true,
                effective: None,
            }),
        )
        .and_then(|v| serde_json::from_value::<AgentSettingsStatusV2>(v).map_err(io::Error::other));
        let Some(spec) = status
            .ok()
            .filter(|s| s.valid() && s.context_id() == context.id)
            .and_then(|s| restore_plan::spec(context, &s))
        else {
            blocked.push(name.clone());
            continue;
        };
        if !spec.is_restore_only() {
            continue;
        }
        if preview(&client, &spec).is_err() {
            blocked.push(name.clone());
        } else {
            specs.push((name.clone(), spec));
        }
    }
    if !blocked.is_empty() {
        return Err(restore_plan::decision(blocked));
    }
    // Refresh Preview before every Apply because prior restores may advance shared revisions.
    for (name, spec) in specs {
        let result = (|| -> io::Result<()> {
            let preview = preview(&client, &spec)?;
            let mut bytes = [0; 16];
            getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
            let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let applied = call(
                &client,
                "ApplyAgentConnectionRestore",
                json!({"expected_revisions":preview["expected_revisions"], "spec":preview["spec"], "accept_digest":preview["accept_digest"], "dependency_digest":preview["dependency_digest"], "idempotency_key":key}),
            )?;
            let operation = applied["operation_id"]
                .as_str()
                .ok_or_else(|| io::Error::other("AGENT_RESTORE_UNAVAILABLE"))?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let view = call(&client, "GetOperation", json!({"operation_id":operation}))?;
                match view["state"].as_str() {
                    Some("succeeded") => break Ok(()),
                    Some("rolled_back" | "needs_attention") | None => {
                        break Err(io::Error::other("AGENT_RESTORE_UNAVAILABLE"));
                    }
                    _ if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    _ => break Err(io::Error::other("AGENT_RESTORE_UNAVAILABLE")),
                }
            }
        })();
        if result.is_err() {
            return Err(restore_plan::decision(vec![name]));
        }
    }
    Ok(())
}
