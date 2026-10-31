//! Decision onboarding presentation over the existing service/Secret/Plan owners.
use super::*;

pub(super) const HELP: &str = "Decision service choices\n\nUsage\n  hiroute decision services options [--provider ID]\n\nRead-only installed presets and saved connection IDs/revisions. Presets do not prove current provider availability or account entitlement. Saved connections are not tested by this query. Reuse an exact saved revision and explicitly test it when authorized. Otherwise choose a preset, System One compatible or custom extension; ordinary Chat endpoints are not decision services.\n\nCredential step (the user runs this in their own terminal, outside chat/Agent tools):\n  hiroute protected-input register --candidate REF --prompt --format bearer\nThen continue with decision services apply --help, decision services test --help and routing preview --help. Local rules remain an explicit alternative without another service. Catalog access works offline; saved state is unavailable when Local Control cannot be queried.\n";

pub(super) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    if !arguments.starts_with(&["decision".into(), "services".into(), "options".into()]) {
        return None;
    }
    if globals.help {
        return Some(CliExecution {
            exit_code: 0,
            stdout: HELP.into(),
            stderr: String::new(),
        });
    }
    let result: Result<Reply> = (|| {
        validate_globals(globals)?;
        let flags = Flags::parse(&arguments[3..], &["--provider"], &[], &[])?;
        let mut view = options(client(globals).ok().as_ref());
        if let Some(provider) = flags.get("--provider") {
            let selected = view["providers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == provider)
                .ok_or_else(|| choices(&["--provider"], view.clone()))?;
            view["selected_provider"] = selected.clone();
            if provider == "custom" {
                view["credential_step"]["arguments"] = Value::Null;
                view["credential_step"]["required_choices"] =
                    json!(["credential format raw|bearer OR explicit --no-auth"]);
                view["credential_step"]["argument_template"] = json!([
                    "protected-input",
                    "register",
                    "--candidate",
                    "CANDIDATE_REF",
                    "--prompt",
                    "--format",
                    "SELECTED_FORMAT"
                ]);
            }
        }
        Ok(MachineEnvelopeV2::succeeded(
            view,
            globals.request_id.clone(),
        ))
    })();
    Some(render_reply(result.unwrap_or_else(|reply| *reply), globals))
}

pub(super) fn options(client: Option<&LocalControlClient>) -> Value {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../assets/decision-services/providers.v1.json"
    ))
    .expect("bundled decision provider catalog is valid");
    let mut providers = catalog["providers"].as_array().unwrap().clone();
    // This legacy command creates V1 System One services only. V2 reads the same
    // catalog with its explicit codec and must not be reinterpreted by V1.
    providers.retain(|provider| {
        provider
            .get("wire_protocol")
            .is_none_or(|value| value == "system_one")
    });
    for provider in &mut providers {
        provider["kind"] = json!("system_one");
        provider["credential_format"] = json!("bearer");
        let mut required = vec!["--id", "--name", "--candidate"];
        if provider["endpoint"] == "" {
            required.push("--endpoint");
        }
        if provider["model"] == "" {
            required.push("--model");
        }
        provider["required_new_connection_flags"] = json!(required);
    }
    providers.push(
        json!({"id":"custom","name":"Custom extension","kind":"custom",
        "required_new_connection_flags":["--id","--name","--endpoint","--candidate OR --no-auth"],
        "credential_format":"raw (complete header value), or bearer for a bare API key"}),
    );
    let saved = client.and_then(|client| data(client, "ListDecisionServices", json!({})).ok());
    json!({
        "schema":"hiroute.cli-decision-options/v1", "effects":[],
        "providers":providers,
        "saved_services_state":if saved.is_some() {"known"} else {"unavailable"},
        "saved_services":saved.as_ref().map(|v| &v["services"]),
        "saved_connection_test_state":"unknown_not_probed",
        "guidance":"Reuse a selected exact ID@REVISION. Saved is not reachable or qualified; explicitly test before binding when authorized. Presets are installed integration metadata, not live provider or entitlement promises. Ordinary Chat is not the decision protocol. Never send credentials through chat or Agent tools.",
        "credential_step":{"actor":"user_in_own_terminal","arguments":["protected-input","register","--candidate","CANDIDATE_REF","--prompt","--format","bearer"],"output":"registration result only; continue with the same candidate reference"},
        "next_commands":[["decision","services","apply","--help"],["decision","services","test","--help"],["routing","preview","--help"]],
        "alternative":{"decision":"local_rules","effect":"explicit choice; no decision-model call; built-in heuristics remain active even with empty custom keywords"}
    })
}

pub(super) fn prepare(client: &LocalControlClient, options: &[String]) -> Result<Value> {
    let flags = Flags::parse(
        options,
        &[
            "--id",
            "--name",
            "--provider",
            "--endpoint",
            "--model",
            "--timeout-ms",
            "--candidate",
            "--header",
        ],
        &[],
        &["--no-auth"],
    )?;
    let services = data(client, "ListDecisionServices", json!({}))?;
    let change = change(&flags, &services)?;
    Ok(json!(PreviewRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "decision.services.apply".into(),
            resource_id: Some(change.id.clone()),
            desired_state: json!(change),
        },
    }))
}

fn change(flags: &Flags, services: &Value) -> Result<DecisionServiceChangeV1> {
    let id = flags.required("--id")?;
    let rows = services["services"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::Internal))?;
    let matches = rows
        .iter()
        .filter(|row| row["id"] == id)
        .collect::<Vec<_>>();
    let existing = match matches.as_slice() {
        [] => None,
        [service] => Some(*service),
        _ => return Err(error(ErrorCode::Internal)),
    };
    let expected = existing.map_or(0, |service| service["revision"].as_u64().unwrap_or(0));
    let revision = expected
        .checked_add(1)
        .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
    let name = flags
        .get("--name")
        .or_else(|| existing.and_then(|s| s["name"].as_str()))
        .ok_or_else(|| choices(&["--name"], json!({})))?;
    let old = existing.map(|s| &s["connection"]);
    let old_provider = old.and_then(|c| {
        if c["kind"] == "custom" {
            Some("custom")
        } else {
            c["provider"].as_str()
        }
    });
    let provider = flags
        .get("--provider")
        .or(old_provider)
        .ok_or_else(|| choices(&["--provider"], options(None)))?;
    let same_kind = old_provider == Some(provider);
    let catalog = options(None);
    let preset = catalog["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == provider);
    let mut connection = if same_kind {
        old.unwrap().clone()
    } else if provider == "custom" {
        json!({"kind":"custom","endpoint":"","timeout_ms":10000,"auth_header":null})
    } else {
        let preset = preset.ok_or_else(|| choices(&["--provider"], catalog.clone()))?;
        json!({"kind":"system_one","provider":provider,"endpoint":preset["endpoint"],
            "model":preset["model"],"timeout_ms":10000,"auth_header":null})
    };
    for (flag, field) in [("--endpoint", "endpoint"), ("--model", "model")] {
        if let Some(value) = flags.get(flag) {
            if field == "model" && provider == "custom" {
                return Err(error(ErrorCode::InvalidArguments));
            }
            connection[field] = json!(value);
        }
        if (field != "model" || provider != "custom")
            && connection[field].as_str().is_none_or(str::is_empty)
        {
            return Err(choices(&[flag], json!({"provider":provider})));
        }
    }
    if let Some(timeout) = flags.get("--timeout-ms") {
        connection["timeout_ms"] = json!(
            timeout
                .parse::<u64>()
                .map_err(|_| error(ErrorCode::InvalidArguments))?
        );
    }
    if flags.has("--no-auth") {
        if provider != "custom" || flags.has("--candidate") || flags.has("--header") {
            return Err(error(ErrorCode::InvalidArguments));
        }
        connection["auth_header"] = Value::Null;
    } else if flags.has("--candidate") {
        let header = flags
            .get("--header")
            .or_else(|| connection["auth_header"]["name"].as_str())
            .unwrap_or("Authorization");
        if provider != "custom" && !header.eq_ignore_ascii_case("Authorization") {
            return Err(error(ErrorCode::InvalidArguments));
        }
        connection["auth_header"] = json!({"name":header,
            "value_secret_ref":format!("decision/{}", fresh_key()?)});
    } else {
        // A new endpoint must never silently receive a previous endpoint's credential.
        let changed_endpoint = old.is_some_and(|old| old["endpoint"] != connection["endpoint"]);
        if flags.has("--header") || !same_kind || changed_endpoint {
            return Err(choices(
                &["--candidate (new credential) OR --no-auth (custom only)"],
                json!({}),
            ));
        }
        if provider != "custom" && connection["auth_header"].is_null() {
            return Err(choices(&["--candidate"], json!({})));
        }
    }
    let change: DecisionServiceChangeV1 = serde_json::from_value(json!({
        "id":id,"expected_revision":expected,"service":{"id":id,"revision":revision,"name":name,"connection":connection},
        "input_slot":flags.get("--candidate"),
    })).map_err(|_| error(ErrorCode::InvalidArguments))?;
    change
        .validate()
        .map_err(|_| error(ErrorCode::InvalidArguments))?;
    Ok(change)
}

pub(super) fn test(client: &LocalControlClient, options: &[String]) -> Result<Value> {
    let flags = Flags::parse(options, &["--service"], &[], &[])?;
    let selected = flags.required("--service")?;
    let (id, revision) = selected
        .rsplit_once('@')
        .ok_or_else(|| choices(&["--service ID@REVISION"], json!({})))?;
    let revision = revision
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
    let services = data(client, "ListDecisionServices", json!({}))?;
    let matches = services["services"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::Internal))?
        .iter()
        .filter(|s| s["id"] == id && s["revision"] == revision)
        .collect::<Vec<_>>();
    let [service] = matches.as_slice() else {
        return Err(choices(&["--service (exact listed ID@REVISION)"], services));
    };
    Ok(json!({"schema":CLASSIFIER_DECISION_TEST_SCHEMA_V1,
        "classifier":{"kind":"decision_service","service":service}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(args: &[&str]) -> Flags {
        Flags::parse(
            &args.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            &[
                "--id",
                "--name",
                "--candidate",
                "--header",
                "--endpoint",
                "--provider",
            ],
            &[],
            &["--no-auth"],
        )
        .unwrap()
    }
    fn saved(custom: bool) -> Value {
        let connection = if custom {
            json!({"kind":"custom","endpoint":"https://first.example/decide","timeout_ms":1742,
                "auth_header":{"name":"X-Custom-Key","value_secret_ref":"decision/old/r3"}})
        } else {
            json!({"kind":"system_one","provider":"existing-private-compatible-provider","model":"original-model",
                "endpoint":"https://first.example/systemone","timeout_ms":1742,
                "auth_header":{"name":"Authorization","value_secret_ref":"decision/old/r3"}})
        };
        json!({"services":[{"id":"saved","revision":3,"name":"Original","connection":connection}]})
    }
    #[test]
    fn edit_preserves_unlisted_provider_and_complete_existing_connection() {
        let before = saved(false);
        let changed = change(&flags(&["--id", "saved", "--name", "Renamed"]), &before).unwrap();
        assert_eq!(changed.expected_revision, 3);
        let service = json!(changed.service);
        assert_eq!(service["revision"], 4);
        assert_eq!(service["connection"], before["services"][0]["connection"]);
        assert!(changed.input_slot.is_none());
    }
    #[test]
    fn endpoint_changes_never_reuse_a_saved_secret_implicitly() {
        for custom in [false, true] {
            let changed = change(
                &flags(&[
                    "--id",
                    "saved",
                    "--endpoint",
                    "https://other.example/decide",
                ]),
                &saved(custom),
            );
            assert!(changed.is_err());
        }
        let changed = change(
            &flags(&[
                "--id",
                "saved",
                "--endpoint",
                "https://other.example/decide",
                "--no-auth",
            ]),
            &saved(true),
        )
        .unwrap();
        assert!(json!(changed.service)["connection"]["auth_header"].is_null());
        assert!(change(&flags(&["--id", "saved", "--no-auth"]), &saved(false)).is_err());
    }
    #[test]
    fn explicit_rotation_preserves_custom_header_and_uses_an_immutable_new_reference() {
        let changed = change(
            &flags(&["--id", "saved", "--candidate", "candidate/new"]),
            &saved(true),
        )
        .unwrap();
        let service = json!(changed.service);
        assert_eq!(service["connection"]["auth_header"]["name"], "X-Custom-Key");
        assert_ne!(
            service["connection"]["auth_header"]["value_secret_ref"],
            "decision/old/r3"
        );
        assert_eq!(service["connection"]["timeout_ms"], 1742);
        assert_eq!(changed.input_slot.as_deref(), Some("candidate/new"));
    }
}
