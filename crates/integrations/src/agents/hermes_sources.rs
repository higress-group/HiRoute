//! Passive selected-profile providers. Native helpers, pools and process-only keys are not run.
use super::{
    DiscoveredAuthSource, DiscoveredCredentialRefV1, FILESYSTEM_AGENT_SCANNER_ID_V1,
    FILESYSTEM_AGENT_SCANNER_VERSION_V1, NativeApiPointBindingV1, NativeApiSource,
    hermes_yaml::{HermesDocument, HermesNativeError, hermes_error},
};
use hiroute_domain::{
    AgentNativeTargetV1, CanonicalDigest, GatewayAuthenticationSemanticsV1, ProtectedSecret,
    UpstreamProtocol,
};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use zeroize::Zeroizing;

pub struct HermesApiDiscoveryV1 {
    pub sources: Vec<NativeApiSource>,
    pub diagnostics: Vec<HermesSourceDiagnosticV1>,
}
pub struct HermesSourceDiagnosticV1 {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reason: &'static str,
}
type Material = Option<(Zeroizing<String>, ProtectedSecret)>;

pub fn hermes_api_sources(
    target: &AgentNativeTargetV1,
) -> Result<HermesApiDiscoveryV1, HermesNativeError> {
    snapshot(target, None).map(|(facts, _)| facts)
}

pub fn read_hermes_api_source(
    target: &AgentNativeTargetV1,
    expected: &NativeApiSource,
) -> Result<(Zeroizing<String>, ProtectedSecret), HermesNativeError> {
    let (facts, material) = snapshot(target, Some(&expected.source_ref))?;
    if !facts.sources.iter().any(|current| current == expected) {
        return Err(hermes_error("source changed"));
    }
    material.ok_or_else(|| hermes_error("static credential unavailable"))
}

fn bytes(path: &Path) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
    super::filesystem_config::read_validated_config_bytes(path)
        .map(|file| file.map_or_else(|| Zeroizing::new(Vec::new()), |(bytes, _)| bytes))
        .map_err(|_| hermes_error("source file access"))
}
fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
}
fn selected<'a>(value: &'a Value, fields: &[&'a str]) -> Option<(&'a str, &'a str)> {
    fields
        .iter()
        .find_map(|field| text(&value[*field]).map(|v| (*field, v)))
}
fn enabled(entry: &Value) -> bool {
    match entry.get("enabled") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => !matches!(
            value.trim().to_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        ),
        Some(Value::Number(value)) => value.as_f64() != Some(0.0),
        _ => false,
    }
}
fn protocol(entry: &Value) -> Option<UpstreamProtocol> {
    // Native resolver gives api_mode precedence over transport. No endpoint guessing.
    let (_, mode) = selected(entry, &["api_mode", "transport"])?;
    match mode {
        "responses" | "codex_responses" => Some(UpstreamProtocol::Responses),
        "messages" | "anthropic_messages" => Some(UpstreamProtocol::Messages),
        "chat_completions" | "openai_chat" => Some(UpstreamProtocol::ChatCompletions),
        _ => None,
    }
}
fn normalized_name(value: &str) -> String {
    value.trim().to_lowercase().replace(' ', "-")
}

fn snapshot(
    target: &AgentNativeTargetV1,
    selected_source: Option<&str>,
) -> Result<(HermesApiDiscoveryV1, Material), HermesNativeError> {
    let canonical = super::canonical_hermes_target(target)?;
    if &canonical != target {
        return Err(hermes_error("canonical selected root required"));
    }
    let root = Path::new(target.config_root());
    let config = bytes(&root.join("config.yaml"))?;
    let document = HermesDocument::parse(&config)?;
    let env = bytes(&root.join(".env"))?;
    let auth = bytes(&root.join("auth.json"))?;
    let auth_document = if auth.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&auth).map_err(|_| hermes_error("auth metadata"))?
    };
    let auth_document = super::pi_sources::Document(auth_document);
    let environment = environment(&env)?;
    let scope = CanonicalDigest::of_bytes(target.config_root().as_bytes());
    let revision_digest = CanonicalDigest::of(&(
        "hermes-static-source/v1",
        &scope,
        CanonicalDigest::of_bytes(&config),
        CanonicalDigest::of_bytes(&env),
        CanonicalDigest::of_bytes(&auth),
    ))
    .map_err(|_| hermes_error("source snapshot"))?;
    let revision = u64::from_str_radix(&revision_digest.as_str()[7..23], 16)
        .expect("digest")
        .max(1);
    let mut facts = HermesApiDiscoveryV1 {
        sources: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut material = None;
    let main = &document.value["model"];
    let main_provider = text(&main["provider"]);
    let main_model = text(&main["default"]);
    let mut providers = Vec::new();
    if let Some(entries) = document.value["providers"].as_object() {
        for (name, entry) in entries {
            providers.push((name.clone(), format!("providers/{name}"), entry));
        }
    } else if !document.value["providers"].is_null() {
        facts.diagnostics.push(HermesSourceDiagnosticV1 {
            provider_id: None,
            model_id: None,
            reason: "invalid_provider_mapping",
        });
    }
    if let Some(entries) = document.value["custom_providers"].as_array() {
        for (i, entry) in entries.iter().enumerate() {
            if let Some(name) = text(&entry["name"]) {
                // Native providers mapping precedes a legacy list entry of the same identity.
                if !providers
                    .iter()
                    .any(|(id, _, _)| normalized_name(id) == normalized_name(name))
                {
                    providers.push((name.into(), format!("custom_providers/{i}"), entry));
                }
            }
        }
    } else if !document.value["custom_providers"].is_null() {
        facts.diagnostics.push(HermesSourceDiagnosticV1 {
            provider_id: None,
            model_id: None,
            reason: "invalid_custom_provider_list",
        });
    }
    // Hermes also accepts a bare custom endpoint in the main model block. Keep its
    // selector distinct from named providers, whose native precedence is unchanged.
    if main_provider == Some("custom")
        && text(&main["base_url"]).is_some()
        && !providers
            .iter()
            .any(|(id, _, _)| normalized_name(id) == "custom")
    {
        providers.push(("custom".into(), "model".into(), main));
    }
    if providers.len() > 128 {
        return Err(hermes_error("provider bound"));
    }
    for (id, definition, entry) in providers {
        let bare_custom = definition == "model";
        if id.starts_with("hiroute-") {
            continue;
        }
        if !enabled(entry) {
            continue;
        }
        let endpoint_fields: &[&str] = if bare_custom {
            &["base_url"]
        } else {
            &["api", "url", "base_url"]
        };
        let Some((endpoint_field, endpoint)) = selected(entry, endpoint_fields) else {
            facts.diagnostics.push(HermesSourceDiagnosticV1 {
                provider_id: Some(id),
                model_id: None,
                reason: "native_endpoint_unavailable",
            });
            continue;
        };
        // Bare custom Responses has native URL-dependent fallbacks; only the explicit
        // Chat path is qualified here. Do not infer a protocol from its endpoint.
        let native_protocol = protocol(entry)
            .filter(|value| !bare_custom || *value == UpstreamProtocol::ChatCompletions);
        let configured_main = main_provider.is_some_and(|value| {
            value == id
                || value
                    .strip_prefix("custom:")
                    .is_some_and(|value| normalized_name(value) == normalized_name(&id))
        });
        let default = text(&entry["default_model"])
            .or_else(|| text(&entry["model"]))
            .or_else(|| configured_main.then_some(main_model).flatten());
        let mut models: BTreeMap<String, &Value> = match entry.get("models") {
            Some(Value::Object(models)) => models
                .iter()
                .map(|(id, value)| (id.clone(), value))
                .collect(),
            None | Some(Value::Null) => BTreeMap::new(),
            _ => {
                facts.diagnostics.push(HermesSourceDiagnosticV1 {
                    provider_id: Some(id.clone()),
                    model_id: None,
                    reason: "unsupported_model_declarations",
                });
                BTreeMap::new()
            }
        };
        if let Some(default) = default {
            models.entry(default.into()).or_insert(&Value::Null);
        }
        if models.is_empty() {
            facts.diagnostics.push(HermesSourceDiagnosticV1 {
                provider_id: Some(id),
                model_id: None,
                reason: "native_model_unavailable",
            });
            continue;
        }
        // Native pools can replace the inline/env key. Supporting their rotating state would
        // need another native adapter; keep that composition visible and unavailable.
        let pool_present = auth_document.0["credential_pool"]
            .as_object()
            .is_some_and(|pools| {
                pools.iter().any(|(pool, entries)| {
                    let Some(name) = pool.strip_prefix("custom:") else {
                        return false;
                    };
                    let Some(entries) = entries.as_array().filter(|entries| !entries.is_empty())
                    else {
                        return false;
                    };
                    name == normalized_name(&id)
                        || text(&entry["name"])
                            .is_some_and(|display| name == normalized_name(display))
                        || entries
                            .iter()
                            .any(|value| text(&value["base_url"]) == Some(endpoint))
                })
            });
        let secret_sources = document.value["secrets"]
            .as_object()
            .is_some_and(|s| !s.is_empty());
        let (key, key_selector, auth_kind, reason) = credential(entry, &environment, &definition);
        for (model_id, declaration) in models {
            if facts.sources.len() >= 256 {
                return Err(hermes_error("model bound"));
            }
            if model_id.is_empty()
                || model_id.len() > 256
                || model_id.chars().any(char::is_control)
                || (!declaration.is_null() && !declaration.is_object())
            {
                facts.diagnostics.push(HermesSourceDiagnosticV1 {
                    provider_id: Some(id.clone()),
                    model_id: None,
                    reason: "invalid_model_declaration",
                });
                continue;
            }
            // Native model maps describe facts; address/key fields here are not proven runtime
            // overrides. Keep such a member separate and unavailable rather than invent precedence.
            let model_override = [
                "api",
                "url",
                "base_url",
                "api_key",
                "key_env",
                "api_mode",
                "transport",
            ]
            .iter()
            .any(|field| declaration.get(*field).is_some());
            let main_override = !bare_custom && configured_main
                && main_model == Some(model_id.as_str())
                // Named providers resolve their transport from the provider definition.
                // A stale model.api_mode does not override that native path.
                && ["base_url", "api_key"]
                    .iter()
                    .any(|field| main.get(*field).is_some_and(|v| text(v).is_some()));
            let unsupported = pool_present
                || secret_sources
                || (bare_custom
                    && environment
                        .get("CUSTOM_BASE_URL")
                        .is_some_and(|v| !v.is_empty()))
                || (bare_custom
                    && text(&entry["api_key"]).is_none()
                    && text(&entry["api"]).is_some())
                || model_override
                || main_override
                || entry.get("oauth").is_some()
                || ["extra_headers", "extra_body"].iter().any(|field| {
                    entry
                        .get(*field)
                        .is_some_and(|v| !v.as_object().is_some_and(|o| o.is_empty()))
                });
            let supported = !unsupported && native_protocol.is_some() && key.is_some();
            let source_ref = format!(
                "hermes-source/{}/{}/{}",
                &scope.as_str()[7..],
                &CanonicalDigest::of_bytes(definition.as_bytes()).as_str()[7..],
                &CanonicalDigest::of_bytes(model_id.as_bytes()).as_str()[7..]
            );
            let evidence = CanonicalDigest::of(&(
                "hermes-native-leaf/v1",
                &revision_digest,
                &definition,
                &model_id,
                &key_selector,
                key.as_ref()
                    .map(|value| CanonicalDigest::of_bytes(value.as_bytes())),
            ))
            .map_err(|_| hermes_error("source evidence"))?;
            let credential = supported.then(|| DiscoveredCredentialRefV1 {
                source: "discovered_config".into(),
                scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.into(),
                scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.into(),
                discovered_source_ref: source_ref.clone(),
                field_selector: "hermes.api-key".into(),
                observed_revision: revision,
            });
            if selected_source == Some(source_ref.as_str()) && supported {
                material = Some((
                    Zeroizing::new(endpoint.into()),
                    ProtectedSecret::new(key.as_ref().expect("supported key").as_bytes().to_vec())
                        .map_err(|_| hermes_error("credential"))?,
                ));
            }
            if !supported {
                facts.diagnostics.push(HermesSourceDiagnosticV1 {
                    provider_id: Some(id.clone()),
                    model_id: Some(model_id.clone()),
                    reason: if unsupported {
                        "unsupported_native_composition"
                    } else if native_protocol.is_none() {
                        "native_protocol_unresolved"
                    } else {
                        reason
                    },
                });
            }
            facts.sources.push(NativeApiSource {
                point_binding: Some(NativeApiPointBindingV1 {
                    native_scope: scope.clone(),
                    provider_definition: definition.clone(),
                    endpoint: endpoint.into(),
                    authentication: if native_protocol == Some(UpstreamProtocol::Messages) {
                        GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                            header: "x-api-key".into(),
                        }
                    } else {
                        GatewayAuthenticationSemanticsV1::Bearer
                    },
                    endpoint_selector: format!("{definition}/{endpoint_field}"),
                    protocol_selector: format!(
                        "{definition}/{}",
                        if text(&entry["api_mode"]).is_some() {
                            "api_mode"
                        } else {
                            "transport"
                        }
                    ),
                    model_override_scope: (model_override || main_override)
                        .then(|| format!("models/{model_id}")),
                    credential_binding: key_selector.clone(),
                    configured_default_model_id: configured_main
                        .then(|| default.map(str::to_owned))
                        .flatten(),
                }),
                provider_id: id.clone(),
                model_id: model_id.clone(),
                protocol: native_protocol,
                display_name: text(&declaration["name"]).unwrap_or(&model_id).into(),
                context_tokens: declaration["context_length"].as_u64().or_else(|| {
                    (default == Some(model_id.as_str()))
                        .then(|| main["context_length"].as_u64())
                        .flatten()
                }),
                max_output_tokens: None,
                vision: None,
                reasoning: None,
                authentication: auth_kind,
                source_ref,
                evidence_digest: evidence,
                revision,
                credential,
                supported_auth: supported,
            });
        }
    }
    Ok((facts, material))
}

fn credential(
    entry: &Value,
    environment: &BTreeMap<String, Zeroizing<String>>,
    definition: &str,
) -> (
    Option<Zeroizing<String>>,
    String,
    DiscoveredAuthSource,
    &'static str,
) {
    // The native bare-custom resolver prefers model.api_key to model.key_env.
    // Named provider precedence remains owned by the existing provider reader.
    if let Some(name) = text(&entry["key_env"])
        .filter(|_| definition != "model" || text(&entry["api_key"]).is_none())
    {
        if let Some(value) = environment.get(name).filter(|value| !value.is_empty()) {
            return (
                Some(Zeroizing::new(value.to_string())),
                format!("{definition}/key_env:{name}:profile.env"),
                DiscoveredAuthSource::EnvironmentKey,
                "static_credential_available",
            );
        }
        // A missing referenced profile value could be filled by Hermes' own process/project
        // environment. Do not choose the inline fallback using the daemon's unrelated state.
        return (
            None,
            format!("{definition}/key_env:{name}:unresolved"),
            DiscoveredAuthSource::Missing,
            "profile_environment_required",
        );
    }
    match text(&entry["api_key"])
        .filter(|value| !value.starts_with('!') && !value.contains("${") && !value.contains("$("))
    {
        Some(value)
            if !matches!(
                value,
                "no-key-required" | "not-required" | "no_key_required"
            ) =>
        {
            (
                Some(Zeroizing::new(value.into())),
                format!("{definition}/api_key:inline"),
                DiscoveredAuthSource::InlineToken,
                "static_credential_available",
            )
        }
        _ => (
            None,
            format!("{definition}/api_key:unresolved"),
            DiscoveredAuthSource::Missing,
            "static_credential_required",
        ),
    }
}

/// A bounded literal dotenv subset. No interpolation from ambient process state or shell execution.
fn environment(bytes: &[u8]) -> Result<BTreeMap<String, Zeroizing<String>>, HermesNativeError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| hermes_error("profile environment encoding"))?;
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            continue;
        }
        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            let quote = value.as_bytes()[0] as char;
            let Some(end) = value[1..].find(quote) else {
                continue;
            };
            let tail = value[end + 2..].trim();
            if !tail.is_empty() && !tail.starts_with('#') {
                continue;
            }
            &value[1..end + 1]
        } else {
            value.split(" #").next().unwrap_or(value).trim_end()
        };
        if value.contains(['$', '\\']) || value.len() > 16384 || value.chars().any(char::is_control)
        {
            values.remove(name);
            continue;
        }
        if values.len() >= 256 && !values.contains_key(name) {
            return Err(hermes_error("environment bound"));
        }
        values.insert(name.into(), Zeroizing::new(value.into()));
    }
    Ok(values)
}
