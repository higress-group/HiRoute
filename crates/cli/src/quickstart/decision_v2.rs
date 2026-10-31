//! Versioned decision onboarding. Credentials remain protected input references.
use super::*;

pub(super) fn execute(arguments: &[String], globals: &Globals) -> Option<CliExecution> {
    if !arguments.starts_with(&["decision".into(), "connections".into(), "options".into()]) {
        return None;
    }
    if globals.help {
        return Some(CliExecution { exit_code: 0, stdout: "Usage: hiroute decision connections options [--provider ID]\nShows installed decision presets and current saved connections. Presets are not entitlement or connectivity evidence.\n".into(), stderr: String::new() });
    }
    let result: Result<Reply> = (|| {
        validate_globals(globals)?;
        let flags = Flags::parse(&arguments[3..], &["--provider"], &[], &[])?;
        let mut view = provider_options(client(globals).ok().as_ref());
        if let Some(provider) = flags.get("--provider") {
            let selected = view["providers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == provider)
                .ok_or_else(|| choices(&["--provider"], view.clone()))?;
            view["selected_provider"] = selected.clone();
        }
        Ok(MachineEnvelopeV2::succeeded(
            view,
            globals.request_id.clone(),
        ))
    })();
    Some(render_reply(result.unwrap_or_else(|reply| *reply), globals))
}

fn catalog() -> Value {
    serde_json::from_str(include_str!(
        "../../../../assets/decision-services/providers.v1.json"
    ))
    .expect("bundled decision provider catalog is valid")
}

fn provider_options(client: Option<&LocalControlClient>) -> Value {
    let mut providers = catalog()["providers"].as_array().unwrap().clone();
    for provider in &mut providers {
        if provider["id"] == "compatible" {
            provider["wire_protocol"] = Value::Null;
        } else if provider.get("wire_protocol").is_none() {
            provider["wire_protocol"] = json!("system_one");
        }
    }
    providers.push(json!({"id":"custom", "name":"Custom extension", "wire_protocol":null}));
    let services = client.and_then(|client| data(client, "ListDecisionServicesV2", json!({})).ok());
    let sources =
        client.and_then(|client| data(client, "ListDecisionConnectionSourcesV1", json!({})).ok());
    json!({"schema":"hiroute.cli-decision-options/v2", "providers":providers,
        "saved_services":services.as_ref().map(|value| &value["services"]),
        "sources":sources.as_ref().map(|value| &value["sources"]),
        "guidance":"Presets are suggestions, not provider entitlement or connectivity. Use --candidate for a protected independent credential or --source for a managed source. Test the exact saved revision after saving."})
}

pub(super) fn prepare(client: &LocalControlClient, options: &[String]) -> Result<Value> {
    let flags = Flags::parse(
        options,
        &[
            "--id",
            "--name",
            "--provider",
            "--model",
            "--endpoint",
            "--wire-protocol",
            "--source",
            "--candidate",
            "--header",
            "--timeout-ms",
        ],
        &[],
        &["--no-auth", "--delete"],
    )?;
    let id = flags.required("--id")?;
    let services = data(client, "ListDecisionServicesV2", json!({}))?;
    let rows = services["services"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::Internal))?;
    let matches = rows
        .iter()
        .filter(|row| row["service"]["id"] == id)
        .collect::<Vec<_>>();
    let existing = match matches.as_slice() {
        [] => None,
        [row] => Some(*row),
        _ => return Err(error(ErrorCode::Internal)),
    };
    let expected_revision = existing
        .and_then(|row| row["service"]["revision"].as_u64())
        .unwrap_or(0);
    let revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
    let mut input_slot = None;
    let (service, expected_source_revisions) = if flags.has("--delete") {
        if expected_revision == 0 || options.len() != 3 {
            return Err(error(ErrorCode::InvalidArguments));
        }
        let sources = existing.unwrap()["related_sources"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::Internal))?;
        let revisions = sources
            .iter()
            .map(|source| {
                let id = source["source_id"]
                    .as_str()
                    .ok_or_else(|| error(ErrorCode::Internal))?;
                let revision = source["revision"].as_u64().ok_or_else(|| {
                    choices(&["refresh_available_sources"], sources.clone().into())
                })?;
                Ok(json!({"source_id":id,"revision":revision}))
            })
            .collect::<Result<Vec<_>>>()?;
        (Value::Null, revisions)
    } else {
        let name = flags
            .get("--name")
            .or_else(|| existing.and_then(|row| row["service"]["name"].as_str()))
            .ok_or_else(|| choices(&["--name"], json!({})))?;
        let old = existing.map(|row| &row["service"]["connection"]);
        let old_provider = old.and_then(|connection| {
            if connection["kind"] == "custom" {
                Some("custom")
            } else {
                connection["provider"].as_str()
            }
        });
        let provider = flags
            .get("--provider")
            .or(old_provider)
            .ok_or_else(|| choices(&["--provider"], provider_options(None)))?;
        let catalog = catalog();
        let preset = catalog["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == provider);
        if provider != "custom" && preset.is_none() {
            return Err(choices(&["--provider"], provider_options(None)));
        }
        let same_provider = old_provider == Some(provider);
        let timeout = match flags.get("--timeout-ms") {
            Some(value) => value
                .parse::<u64>()
                .map_err(|_| error(ErrorCode::InvalidArguments))?,
            None => old
                .filter(|_| same_provider)
                .and_then(|old| old["timeout_ms"].as_u64())
                .unwrap_or(10_000),
        };
        let endpoint = flags
            .get("--endpoint")
            .or_else(|| {
                old.filter(|_| same_provider)
                    .and_then(|old| old["endpoint"].as_str())
            })
            .or_else(|| preset.and_then(|preset| preset["endpoint"].as_str()))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| choices(&["--endpoint"], json!({"provider":provider})))?;
        if flags.has("--candidate") && (flags.has("--source") || flags.has("--no-auth")) {
            return Err(error(ErrorCode::InvalidArguments));
        }
        let old_header = old.and_then(|connection| {
            if connection["kind"] == "native" {
                connection["authentication"]["auth_header"]
                    .as_object()
                    .map(|_| &connection["authentication"]["auth_header"])
            } else {
                connection["auth_header"]
                    .as_object()
                    .map(|_| &connection["auth_header"])
            }
        });
        let header = if flags.has("--no-auth") {
            Value::Null
        } else if let Some(candidate) = flags.get("--candidate") {
            input_slot = Some(candidate.to_owned());
            json!({"name":flags.get("--header").unwrap_or("Authorization"), "value_secret_ref":format!("decision/{}/{}", id, fresh_key()?)})
        } else {
            old_header
                .filter(|_| same_provider)
                .cloned()
                .unwrap_or(Value::Null)
        };
        if flags.has("--header") && !flags.has("--candidate") {
            return Err(error(ErrorCode::InvalidArguments));
        }
        let mut revisions = Vec::new();
        let connection = if provider == "custom" {
            if flags.has("--source") || flags.has("--model") || flags.has("--wire-protocol") {
                return Err(error(ErrorCode::InvalidArguments));
            }
            if header.is_null() && !flags.has("--no-auth") && !same_provider {
                return Err(choices(&["--candidate OR --no-auth"], json!({})));
            }
            json!({"kind":"custom","endpoint":endpoint,"timeout_ms":timeout,"auth_header":header})
        } else {
            if flags.has("--no-auth") || flags.has("--header") {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let model = flags
                .get("--model")
                .or_else(|| {
                    old.filter(|_| same_provider)
                        .and_then(|old| old["model"].as_str())
                })
                .or_else(|| preset.and_then(|preset| preset["model"].as_str()))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| choices(&["--model"], json!({})))?;
            if provider == "compatible" && !flags.has("--wire-protocol") {
                return Err(choices(
                    &["--wire-protocol"],
                    json!({"wire_protocols":["system_one","openai_decisions"]}),
                ));
            }
            let codec = flags
                .get("--wire-protocol")
                .or_else(|| {
                    old.filter(|_| same_provider)
                        .and_then(|old| old["wire_protocol"].as_str())
                })
                .or_else(|| preset.and_then(|preset| preset["wire_protocol"].as_str()))
                .unwrap_or("system_one");
            if !matches!(codec, "system_one" | "openai_decisions") {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let old_source = old
                .filter(|_| same_provider)
                .and_then(|old| old["authentication"]["source_id"].as_str());
            let source_id = flags.get("--source").or_else(|| {
                if flags.has("--candidate") {
                    None
                } else {
                    old_source
                }
            });
            let authentication = if let Some(source_id) = source_id {
                let sources = data(client, "ListDecisionConnectionSourcesV1", json!({}))?;
                let source = sources["sources"]
                    .as_array()
                    .and_then(|sources| {
                        sources
                            .iter()
                            .find(|source| source["source_id"] == source_id)
                    })
                    .ok_or_else(|| choices(&["--source"], sources.clone()))?;
                let source_revision = source["revision"]
                    .as_u64()
                    .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
                let lineage = source["lineage_digest"]
                    .as_str()
                    .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
                if flags.get("--source").is_none()
                    && old.is_some_and(|old| {
                        old["authentication"]["source_lineage_digest"] != lineage
                    })
                {
                    return Err(choices(&["refresh_and_select_source"], sources));
                }
                revisions.push(json!({"source_id":source_id,"revision":source_revision}));
                json!({"kind":"shared_source","source_id":source_id,"source_lineage_digest":lineage})
            } else {
                if header.is_null() {
                    return Err(choices(&["--candidate OR --source"], json!({})));
                }
                json!({"kind":"independent","auth_header":header})
            };
            json!({"kind":"native","provider":provider,"wire_protocol":codec,"model":model,"endpoint":endpoint,"timeout_ms":timeout,"authentication":authentication})
        };
        (
            json!({"schema":"hiroute.decision-service/v2","id":id,"revision":revision,"name":name,"connection":connection}),
            revisions,
        )
    };
    let input = json!({"change":{"id":id,"expected_revision":expected_revision,"service":service,"expected_source_revisions":expected_source_revisions},"input_slot":input_slot});
    let typed: DecisionServiceOperationInputV2 =
        serde_json::from_value(input).map_err(|_| error(ErrorCode::InvalidArguments))?;
    if !typed.change.validate() {
        return Err(error(ErrorCode::InvalidArguments));
    }
    Ok(json!(PreviewRequestV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        spec: ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "decision.connections.apply".into(),
            resource_id: Some(id.into()),
            desired_state: json!(typed),
        }
    }))
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
        .filter(|revision| *revision > 0)
        .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
    let services = data(client, "ListDecisionServicesV2", json!({}))?;
    let row = services["services"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["service"]["id"] == id && row["service"]["revision"] == revision)
        })
        .ok_or_else(|| choices(&["--service (listed ID@REVISION)"], services.clone()))?;
    Ok(
        json!({"id":id,"revision":revision,"expected_configuration_digest":row["configuration_digest"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_preset_uses_the_explicit_codec_without_changing_legacy_options() {
        let current = provider_options(None);
        let openai = current["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["id"] == "openai")
            .unwrap();
        assert_eq!(openai["endpoint"], "https://api.openai.com/v1/decisions");
        assert_eq!(openai["model"], "gpt-6-luna");
        assert_eq!(openai["wire_protocol"], "openai_decisions");
        let compatible = current["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["id"] == "compatible")
            .unwrap();
        assert!(compatible["wire_protocol"].is_null());
        assert!(
            super::super::decision::options(None)["providers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|provider| provider["id"] != "openai")
        );
    }
}
