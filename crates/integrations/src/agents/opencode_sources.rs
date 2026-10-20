//! Passive OpenCode API-key import; no SDK/plugin/helper execution or native OAuth ownership.
use super::pi_sources::Document;
use super::{
    AgentFilesystemScanError as Error, DiscoveredAuthSource, DiscoveredCredentialRefV1,
    FILESYSTEM_AGENT_SCANNER_ID_V1, FILESYSTEM_AGENT_SCANNER_VERSION_V1, FilesystemAgentScannerV1,
    NativeApiSource,
};
use hiroute_domain::{CanonicalDigest, ProtectedSecret, UpstreamProtocol};
use serde_json::Value;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

type Material = Option<(Zeroizing<String>, ProtectedSecret)>;

impl FilesystemAgentScannerV1 {
    pub fn opencode_api_sources(&self) -> Result<Vec<NativeApiSource>, Error> {
        self.opencode_sources_inner(None).map(|(items, _)| items)
    }
    pub fn read_opencode_api_source(
        &self,
        selected: &NativeApiSource,
    ) -> Result<(Zeroizing<String>, ProtectedSecret), Error> {
        let (items, material) = self.opencode_sources_inner(Some(&selected.source_ref))?;
        if !items.iter().any(|i| i == selected) {
            return Err(Error::SourceChanged);
        }
        let material = material.ok_or(Error::SourceUnavailable)?;
        if !self.opencode_api_sources()?.iter().any(|i| i == selected) {
            return Err(Error::SourceChanged);
        }
        Ok(material)
    }
    pub(super) fn read_opencode_secret(
        &self,
        descriptor: &DiscoveredCredentialRefV1,
    ) -> Result<ProtectedSecret, Error> {
        let selected = self
            .opencode_api_sources()?
            .into_iter()
            .find(|s| s.credential.as_ref() == Some(descriptor))
            .ok_or(Error::SourceChanged)?;
        self.read_opencode_api_source(&selected)
            .map(|(_, secret)| secret)
    }
    fn opencode_sources_inner(
        &self,
        selected: Option<&str>,
    ) -> Result<(Vec<NativeApiSource>, Material), Error> {
        let native = self.opencode_native_context()?;
        let config = native.read_config(None)?;
        let dynamic_keys = super::opencode::dynamic_keys(&config.value);
        let remote = native.has_remote_configuration()?;
        let auth = super::filesystem_config::read_validated_config_bytes(
            &native.data_root.join("auth.json"),
        )?;
        let auth_digest =
            CanonicalDigest::of_bytes(auth.as_ref().map_or(&[][..], |(b, _)| b.as_slice()));
        let auth = Document(match auth {
            Some((bytes, _)) => serde_json::from_slice(&bytes).map_err(|_| Error::InvalidConfig)?,
            None => serde_json::json!({}),
        });
        let Some(providers) = config.value.get("provider").and_then(Value::as_object) else {
            return Ok((Vec::new(), None));
        };
        if providers.len() > 256 {
            return Err(Error::ConfigTooLarge);
        }
        let mut items = Vec::new();
        let mut material = None;
        for (provider_id, provider) in providers {
            if provider_id.starts_with("hiroute-") {
                continue;
            }
            if !bounded(provider_id) {
                continue;
            }
            let mut origin = self.opencode_user_models_target();
            for (path, _) in &config.documents {
                if let Some((bytes, _)) = super::filesystem_config::read_system_config_bytes(path)?
                {
                    let doc = Document(
                        super::opencode::parse_jsonc(&bytes).map_err(|_| Error::InvalidConfig)?,
                    );
                    if doc.0["provider"][provider_id]["options"]
                        .get("apiKey")
                        .is_some()
                    {
                        origin = path.clone();
                    }
                }
            }
            let options = &provider["options"];
            let stored = &auth.0[provider_id];
            let mut key_evidence = Vec::new();
            let mut key_available = true;
            let (authentication, secret) = if let Some(value) = options.get("apiKey") {
                match resolve_key(value.as_str(), &origin, &native.home, &mut key_evidence) {
                    Ok(key) => key,
                    Err(_) => {
                        // A provider-local credential cannot hide independent candidates.
                        // No descriptor/material is issued; selected reads still compare
                        // the full fresh candidate and reject any changed evidence.
                        key_available = false;
                        (DiscoveredAuthSource::Missing, None)
                    }
                }
            } else if stored["type"] == "api" {
                (
                    DiscoveredAuthSource::InlineToken,
                    stored["key"]
                        .as_str()
                        .filter(|s| valid_key(s))
                        .map(|s| Zeroizing::new(s.into())),
                )
            } else if stored["type"] == "oauth" {
                (DiscoveredAuthSource::NativeSessionNeedsConfirmation, None)
            } else {
                let env = provider["env"]
                    .as_array()
                    .filter(|v| v.len() == 1)
                    .and_then(|v| v[0].as_str())
                    .or_else(|| match provider["npm"].as_str() {
                        Some("@ai-sdk/openai") => Some("OPENAI_API_KEY"),
                        Some("@ai-sdk/anthropic") => Some("ANTHROPIC_API_KEY"),
                        _ => None,
                    });
                match env {
                    Some(name) if env_name(name) => (
                        DiscoveredAuthSource::EnvironmentKey,
                        std::env::var(name)
                            .ok()
                            .filter(|s| valid_key(s))
                            .map(Zeroizing::new),
                    ),
                    _ => (DiscoveredAuthSource::Missing, None),
                }
            };
            let clean = |v: &Value| v.is_null() || v.as_object().is_some_and(|v| v.is_empty());
            let representable = key_available
                && !remote
                && !dynamic_keys
                && stored["type"] != "oauth"
                && clean(&options["headers"])
                && options.as_object().is_none_or(|o| {
                    o.keys()
                        .all(|k| matches!(k.as_str(), "baseURL" | "apiKey" | "headers"))
                })
                && !config.executable_plugins;
            let declarations = provider["models"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v)).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![(String::new(), &Value::Null)]);
            for (native_id, model) in declarations {
                if items.len() >= 256 {
                    return Err(Error::ConfigTooLarge);
                }
                let id = model["id"].as_str().unwrap_or(&native_id);
                if !id.is_empty() && !bounded(id) {
                    continue;
                }
                let protocol = match model["provider"]["npm"]
                    .as_str()
                    .or_else(|| provider["npm"].as_str())
                {
                    Some("@ai-sdk/openai") => Some(UpstreamProtocol::Responses),
                    Some("@ai-sdk/anthropic") => Some(UpstreamProtocol::Messages),
                    Some("@ai-sdk/openai-compatible") => Some(UpstreamProtocol::ChatCompletions),
                    _ => None,
                };
                let endpoint = options["baseURL"]
                    .as_str()
                    .or_else(|| model["provider"]["api"].as_str())
                    .or_else(|| provider["api"].as_str());
                let endpoint = endpoint.filter(|s| {
                    s.len() <= 8192
                        && !s.contains(['{', '}', '\r', '\n'])
                        && (s.starts_with("https://") || s.starts_with("http://"))
                });
                let supported = representable
                    && !native_id.is_empty()
                    && !id.is_empty()
                    && !id.contains(['{', '}'])
                    && endpoint.is_some()
                    && protocol.is_some()
                    && clean(&model["headers"])
                    && clean(&model["options"]);
                let source_digest = CanonicalDigest::of(&(native, provider_id, &native_id))
                    .map_err(|_| Error::InvalidConfig)?;
                let source_ref = format!("opencode-source/{}", &source_digest.as_str()[7..]);
                let evidence_digest = CanonicalDigest::of(&(
                    "opencode-api-evidence/v1",
                    &source_ref,
                    &config.digest,
                    &auth_digest,
                    &key_evidence,
                    secret
                        .as_ref()
                        .map(|s| CanonicalDigest::of_bytes(s.as_bytes())),
                    remote,
                ))
                .map_err(|_| Error::InvalidConfig)?;
                let revision = u64::from_str_radix(&evidence_digest.as_str()[7..23], 16)
                    .map_err(|_| Error::InvalidConfig)?
                    .max(1);
                let credential =
                    secret
                        .as_ref()
                        .filter(|_| supported)
                        .map(|_| DiscoveredCredentialRefV1 {
                            source: "discovered_config".into(),
                            scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.into(),
                            scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.into(),
                            discovered_source_ref: source_ref.clone(),
                            field_selector: "opencode.api-key".into(),
                            observed_revision: revision,
                        });
                if selected == Some(&source_ref)
                    && supported
                    && let (Some(endpoint), Some(secret)) = (endpoint, secret.as_ref())
                {
                    material = Some((
                        Zeroizing::new(endpoint.into()),
                        ProtectedSecret::new(secret.as_bytes().to_vec())
                            .map_err(|_| Error::InvalidConfig)?,
                    ));
                }
                items.push(NativeApiSource {
                    point_binding: endpoint.map(|endpoint| super::NativeApiPointBindingV1 {
                        native_scope: CanonicalDigest::of(&("opencode-native-scope/v1", native))
                            .expect("native scope"),
                        provider_definition: format!("provider/{provider_id}"),
                        endpoint: endpoint.into(),
                        authentication: if protocol == Some(UpstreamProtocol::Messages) {
                            hiroute_domain::GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                                header: "x-api-key".into(),
                            }
                        } else {
                            hiroute_domain::GatewayAuthenticationSemanticsV1::Bearer
                        },
                        endpoint_selector: if options.get("baseURL").is_some() {
                            format!("provider/{provider_id}/options/baseURL")
                        } else if model["provider"].get("api").is_some() {
                            format!("provider/{provider_id}/models/{native_id}/provider/api")
                        } else {
                            format!("provider/{provider_id}/api")
                        },
                        protocol_selector: if model["provider"].get("npm").is_some() {
                            format!("provider/{provider_id}/models/{native_id}/provider/npm")
                        } else {
                            format!("provider/{provider_id}/npm")
                        },
                        model_override_scope: (model["provider"].get("api").is_some()
                            || model["provider"].get("npm").is_some())
                        .then(|| native_id.clone()),
                        credential_binding: format!(
                            "provider/{provider_id}/{}:{}",
                            if options.get("apiKey").is_some() {
                                "configuration-api-key"
                            } else {
                                "auth-json"
                            },
                            CanonicalDigest::of(&origin)
                                .expect("native credential origin")
                                .as_str()
                        ),
                        configured_default_model_id: config.value["model"]
                            .as_str()
                            .and_then(|default| default.strip_prefix(&format!("{provider_id}/")))
                            .and_then(|default| {
                                provider["models"][default]["id"].as_str().or(Some(default))
                            })
                            .map(str::to_owned),
                    }),
                    provider_id: provider_id.clone(),
                    model_id: id.into(),
                    protocol,
                    display_name: model["name"]
                        .as_str()
                        .filter(|s| bounded(s))
                        .unwrap_or(&native_id)
                        .into(),
                    context_tokens: model["limit"]["context"].as_u64(),
                    max_output_tokens: model["limit"]["output"].as_u64(),
                    vision: model["modalities"]["input"]
                        .as_array()
                        .map(|v| v.iter().any(|x| x == "image")),
                    reasoning: model["reasoning"].as_bool(),
                    authentication,
                    source_ref,
                    evidence_digest,
                    revision,
                    credential,
                    supported_auth: supported,
                });
            }
        }
        Ok((items, material))
    }
}

fn bounded(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn valid_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 * 1024 && !s.contains(['\0', '\r', '\n'])
}
fn env_name(s: &str) -> bool {
    bounded(s) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn resolve_key(
    value: Option<&str>,
    origin: &Path,
    home: &Path,
    evidence: &mut Vec<(PathBuf, CanonicalDigest)>,
) -> Result<(DiscoveredAuthSource, Option<Zeroizing<String>>), Error> {
    let Some(value) = value else {
        return Ok((DiscoveredAuthSource::Missing, None));
    };
    if let Some(name) = value
        .strip_prefix("{env:")
        .and_then(|s| s.strip_suffix('}'))
        .filter(|s| env_name(s))
    {
        return Ok((
            DiscoveredAuthSource::EnvironmentKey,
            std::env::var(name)
                .ok()
                .filter(|s| valid_key(s))
                .map(Zeroizing::new),
        ));
    }
    if let Some(relative) = value
        .strip_prefix("{file:")
        .and_then(|s| s.strip_suffix('}'))
    {
        if relative.contains(['{', '}', '\0']) {
            return Ok((DiscoveredAuthSource::Missing, None));
        }
        let raw = if let Some(tail) = relative.strip_prefix("~/") {
            home.join(tail)
        } else {
            PathBuf::from(relative)
        };
        let path = if raw.is_absolute() {
            raw
        } else {
            origin.parent().ok_or(Error::InvalidConfig)?.join(raw)
        };
        let Some((bytes, _)) = super::filesystem_config::read_validated_config_bytes(&path)? else {
            return Ok((DiscoveredAuthSource::Missing, None));
        };
        evidence.push((path, CanonicalDigest::of_bytes(&bytes)));
        let value = std::str::from_utf8(&bytes)
            .map_err(|_| Error::InvalidConfig)?
            .trim();
        return Ok((
            DiscoveredAuthSource::InlineToken,
            valid_key(value).then(|| Zeroizing::new(value.into())),
        ));
    }
    Ok((
        DiscoveredAuthSource::InlineToken,
        (!value.contains(['{', '}']) && valid_key(value)).then(|| Zeroizing::new(value.into())),
    ))
}
