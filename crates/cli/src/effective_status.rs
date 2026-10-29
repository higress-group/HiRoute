//! Additive status syntax; released positional status keeps its original response shape.
use crate::request_input;
use hiroute_application_api::{
    AGENT_SETTINGS_SCHEMA_V2, AgentConnectionStatusRequestV1, AgentEffectiveStateRequestV1,
    AgentPlanId, AgentSettingsStatusRequestV2, ErrorCode,
};
use serde_json::Value;

pub(crate) fn payload(options: &[String]) -> Result<Value, ErrorCode> {
    let request = if options
        .iter()
        .any(|option| matches!(option.as_str(), "--request-file" | "--request-stdin"))
    {
        let value = request_input::read(options, request_input::MAX_REQUEST_BYTES)?;
        request_input::deserialize::<AgentSettingsStatusRequestV2>(value)?
    } else if let [context] = options {
        if !request_input::valid_positional(context) {
            return Err(ErrorCode::InvalidArguments);
        }
        if !context.starts_with("agent-context/") {
            return serde_json::to_value(AgentConnectionStatusRequestV1 {
                connection_id: context.clone(),
            })
            .map_err(|_| ErrorCode::Internal);
        }
        AgentSettingsStatusRequestV2 {
            projection: Default::default(),
            include_skills: true,
            native_target: None,
            schema_version: AGENT_SETTINGS_SCHEMA_V2,
            context_id: context.clone(),
            effective: None,
        }
    } else {
        let mut values = std::collections::BTreeMap::new();
        let mut effective = false;
        let mut index = 0;
        while index < options.len() {
            let key = options[index].as_str();
            if key == "--effective" && !effective {
                effective = true;
                index += 1;
                continue;
            }
            if !matches!(
                key,
                "--context" | "--plan" | "--session-id" | "--observed-request-id"
            ) {
                return Err(ErrorCode::InvalidArguments);
            }
            let value = options
                .get(index + 1)
                .filter(|value| request_input::valid_positional(value))
                .ok_or(ErrorCode::InvalidArguments)?;
            if values.insert(key, value.clone()).is_some() {
                return Err(ErrorCode::InvalidArguments);
            }
            index += 2;
        }
        if !effective {
            return Err(ErrorCode::InvalidArguments);
        }
        AgentSettingsStatusRequestV2 {
            projection: Default::default(),
            include_skills: true,
            native_target: None,
            schema_version: AGENT_SETTINGS_SCHEMA_V2,
            context_id: values
                .remove("--context")
                .ok_or(ErrorCode::InvalidArguments)?,
            effective: Some(AgentEffectiveStateRequestV1 {
                plan_id: AgentPlanId::parse(
                    values.remove("--plan").ok_or(ErrorCode::InvalidArguments)?,
                )
                .map_err(|_| ErrorCode::InvalidArguments)?,
                session_id: values.remove("--session-id"),
                request_id: values.remove("--observed-request-id"),
            }),
        }
    };
    if request.schema_version != AGENT_SETTINGS_SCHEMA_V2
        || !request.context_id.starts_with("agent-context/")
        || request
            .effective
            .as_ref()
            .is_some_and(|effective| !effective.valid())
    {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }
    #[test]
    fn legacy_shape_is_unchanged_and_effective_scope_is_explicit() {
        assert!(
            payload(&args(&["agent-context/test"]))
                .unwrap()
                .get("effective")
                .is_none()
        );
        assert_eq!(
            payload(&args(&["connection/legacy"])).unwrap(),
            serde_json::json!({"connection_id":"connection/legacy"})
        );
        let value = payload(&args(&[
            "--effective",
            "--context",
            "agent-context/test",
            "--plan",
            "plan/test",
            "--session-id",
            "session/test",
        ]))
        .unwrap();
        assert_eq!(value["effective"]["plan_id"], "plan/test");
        assert_eq!(value["effective"]["session_id"], "session/test");
        for invalid in [
            vec!["--effective", "--context", "agent-context/test"],
            vec![
                "--effective",
                "--context",
                "agent-context/test",
                "--plan",
                "plan/test",
                "--effective",
            ],
            vec!["--context", "agent-context/test", "--plan", "plan/test"],
        ] {
            assert!(payload(&args(&invalid)).is_err());
        }
    }
}
