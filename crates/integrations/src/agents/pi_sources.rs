//! Passive Pi API sources. Only literal/static environment keys are imported; no commands/OAuth.
use super::{
    AgentFilesystemScanError as Error, DiscoveredAuthSource, DiscoveredCredentialRefV1,
    FILESYSTEM_AGENT_SCANNER_ID_V1, FILESYSTEM_AGENT_SCANNER_VERSION_V1, FilesystemAgentScannerV1,
};
use hiroute_domain::{CanonicalDigest, ProtectedSecret, UpstreamProtocol};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct PiApiSource {
    pub provider_id: String,
    pub model_id: String,
    pub protocol: Option<UpstreamProtocol>,
    pub display_name: String,
    pub context_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub authentication: DiscoveredAuthSource,
    pub source_ref: String,
    pub evidence_digest: CanonicalDigest,
    pub revision: u64,
    pub credential: Option<DiscoveredCredentialRefV1>,
    pub supported_auth: bool,
}
type PiSourceSnapshot = (
    Vec<PiApiSource>,
    Option<(Zeroizing<String>, ProtectedSecret)>,
);
/// Raw provider values stay privileged and are cleared after each bounded scan.
struct Document(Value);
impl Drop for Document {
    fn drop(&mut self) {
        fn clear(v: &mut Value) {
            match v {
                Value::String(s) => s.zeroize(),
                Value::Array(a) => a.iter_mut().for_each(clear),
                Value::Object(o) => o.values_mut().for_each(clear),
                _ => {}
            }
        }
        clear(&mut self.0);
    }
}
fn read(path: &std::path::Path) -> Result<(Document, CanonicalDigest), Error> {
    let observed = super::filesystem_config::read_validated_config_bytes(path)?;
    let bytes = observed.as_ref().map_or(&[][..], |(b, _)| b.as_slice());
    let value = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        if path.file_name().is_some_and(|v| v == "models.json") {
            super::additional_native::parse_native_jsonc(bytes).map_err(|_| Error::InvalidConfig)?
        } else {
            serde_json::from_slice(bytes).map_err(|_| Error::InvalidConfig)?
        }
    };
    if !value.is_object() {
        return Err(Error::InvalidConfig);
    }
    Ok((Document(value), CanonicalDigest::of_bytes(bytes)))
}
fn key(
    value: Option<&Value>,
    env: Option<&Value>,
) -> (DiscoveredAuthSource, Option<Zeroizing<String>>) {
    let Some(value) = value.and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return (DiscoveredAuthSource::Missing, None);
    };
    if value.starts_with('!') {
        return (DiscoveredAuthSource::HelperNeedsInput, None);
    }
    // Supported native key templates: $NAME/${NAME}, $$ and $!. Plain names are literal.
    // Resolve only referenced names, never evaluate a shell or enumerate the environment.
    let mut resolved = Zeroizing::new(String::new());
    let mut rest = value;
    let mut used_env = false;
    while let Some(index) = rest.find('$') {
        resolved.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        if let Some(tail) = rest.strip_prefix('$').or_else(|| rest.strip_prefix('!')) {
            resolved.push(rest.chars().next().unwrap());
            rest = tail;
            continue;
        }
        let reference = if let Some(braced) = rest.strip_prefix('{') {
            braced.find('}').map(|end| (&braced[..end], end + 2))
        } else {
            let length = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            (length > 0).then(|| (&rest[..length], length))
        };
        let Some((name, consumed)) = reference.filter(|(name, _)| {
            name.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }) else {
            resolved.push('$');
            // Native templates consume a closed invalid ${...} as one literal,
            // including any inner '$'; only an unclosed brace resumes scanning.
            if rest.starts_with('{')
                && let Some(end) = rest.find('}')
            {
                resolved.push_str(&rest[..=end]);
                rest = &rest[end + 1..];
            }
            continue;
        };
        let configured = env
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty());
        let process = configured
            .is_none()
            .then(|| std::env::var(name).ok())
            .flatten();
        let Some(material) = configured.or(process.as_deref()).filter(|v| !v.is_empty()) else {
            return (DiscoveredAuthSource::Missing, None);
        };
        resolved.push_str(material);
        used_env = true;
        rest = &rest[consumed..];
    }
    resolved.push_str(rest);
    if resolved.is_empty() || resolved.len() > 64 * 1024 {
        return (DiscoveredAuthSource::Missing, None);
    }
    (
        if used_env {
            DiscoveredAuthSource::EnvironmentKey
        } else {
            DiscoveredAuthSource::InlineToken
        },
        Some(resolved),
    )
}
fn bounded(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

impl FilesystemAgentScannerV1 {
    pub fn pi_api_sources(&self) -> Result<Vec<PiApiSource>, Error> {
        self.pi_sources_inner(None).map(|(items, _)| items)
    }
    pub fn read_pi_api_source(
        &self,
        selected: &PiApiSource,
    ) -> Result<(Zeroizing<String>, ProtectedSecret), Error> {
        let (current, material) = self.pi_sources_inner(Some(&selected.source_ref))?;
        if !current.iter().any(|item| item == selected) {
            return Err(Error::SourceChanged);
        }
        let material = material.ok_or(Error::SourceUnavailable)?;
        if !self.pi_api_sources()?.iter().any(|item| item == selected) {
            return Err(Error::SourceChanged);
        }
        Ok(material)
    }
    pub(super) fn read_pi_secret(
        &self,
        descriptor: &DiscoveredCredentialRefV1,
    ) -> Result<ProtectedSecret, Error> {
        let source = self
            .pi_api_sources()?
            .into_iter()
            .find(|s| s.credential.as_ref() == Some(descriptor))
            .ok_or(Error::SourceChanged)?;
        self.read_pi_api_source(&source).map(|(_, key)| key)
    }
    fn pi_sources_inner(&self, selected: Option<&str>) -> Result<PiSourceSnapshot, Error> {
        let (models, models_digest) = read(&self.pi_user_models_target())?;
        let (auth, auth_digest) = read(&self.layout.pi_config_root.join("auth.json"))?;
        let mut providers = models
            .0
            .get("providers")
            .map(|v| v.as_object().cloned().ok_or(Error::InvalidConfig))
            .transpose()?
            .unwrap_or_default();
        // Built-in API models come from the selected, pinned native package's JSON data,
        // not a second checked-in catalog or execution of user/provider JavaScript.
        let sdk = self
            .pi_executable_target()
            .and_then(|p| super::super::pi_cli_installation(&p).ok());
        let mut catalog_digests = BTreeMap::new();
        for (provider_id, variable) in [
            ("openai", "OPENAI_API_KEY"),
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("deepseek", "DEEPSEEK_API_KEY"),
            ("groq", "GROQ_API_KEY"),
            ("mistral", "MISTRAL_API_KEY"),
            ("xai", "XAI_API_KEY"),
            ("cerebras", "CEREBRAS_API_KEY"),
        ] {
            if !providers.contains_key(provider_id)
                && auth.0.get(provider_id).is_none()
                && std::env::var(variable).ok().is_none_or(|v| v.is_empty())
            {
                continue;
            }
            let provider = providers
                .entry(provider_id)
                .or_insert_with(|| serde_json::json!({}));
            if !provider.is_object() {
                return Err(Error::InvalidConfig);
            }
            if provider.get("apiKey").is_none() {
                provider["apiKey"] = format!("${{{variable}}}").into();
            }
            if provider.get("models").is_none()
                && let Some(sdk) = &sdk
            {
                let nested = sdk.package_root.join("node_modules/@earendil-works/pi-ai");
                let sibling = sdk
                    .package_root
                    .parent()
                    .ok_or(Error::SourceUnavailable)?
                    .join("pi-ai");
                let root = if nested.is_dir() { nested } else { sibling };
                let bytes = super::filesystem_config::read_system_config_bytes(
                    &root.join(format!("dist/providers/data/{provider_id}.json")),
                )?
                .ok_or(Error::SourceUnavailable)?
                .0;
                let catalog: Value =
                    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidConfig)?;
                let declarations = catalog
                    .as_object()
                    .ok_or(Error::InvalidConfig)?
                    .values()
                    .filter_map(Value::as_object)
                    .flat_map(|api| api.values())
                    .filter(|m| m["type"] == "chat")
                    .cloned()
                    .collect::<Vec<_>>();
                catalog_digests.insert(provider_id, CanonicalDigest::of_bytes(&bytes));
                provider["models"] = declarations.into();
            }
        }
        let providers = Document(providers.into());
        let providers = providers.0.as_object().ok_or(Error::InvalidConfig)?;
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
                return Err(Error::InvalidConfig);
            }
            let native_auth = auth.0.get(provider_id);
            let (authentication, secret) = if native_auth.is_some_and(|v| v["type"] == "oauth") {
                (DiscoveredAuthSource::NativeSessionNeedsConfirmation, None)
            } else if native_auth.is_some_and(|v| v["type"] == "api_key") {
                key(
                    native_auth.and_then(|v| v.get("key")),
                    native_auth.and_then(|v| v.get("env")),
                )
            } else {
                key(provider.get("apiKey"), None)
            };
            let supported_auth = provider.get("oauth").is_none()
                && provider.get("modelOverrides").is_none()
                && provider
                    .get("headers")
                    .is_none_or(|v| v.as_object().is_some_and(|o| o.is_empty()));
            let unknown = vec![serde_json::json!({"id":"unavailable-native-models"})];
            let declarations = provider
                .get("models")
                .map(|v| v.as_array().ok_or(Error::InvalidConfig))
                .transpose()?
                .unwrap_or(&unknown);
            let mut seen = BTreeSet::new();
            for model in declarations {
                if items.len() >= 256 {
                    return Err(Error::ConfigTooLarge);
                }
                let id = model
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|v| bounded(v))
                    .ok_or(Error::InvalidConfig)?;
                if !seen.insert(id) {
                    return Err(Error::InvalidConfig);
                }
                let protocol = match model
                    .get("api")
                    .or_else(|| provider.get("api"))
                    .and_then(Value::as_str)
                {
                    Some("openai-responses") => Some(UpstreamProtocol::Responses),
                    Some("openai-completions") => Some(UpstreamProtocol::ChatCompletions),
                    Some("anthropic-messages") => Some(UpstreamProtocol::Messages),
                    _ => None,
                };
                let endpoint = model
                    .get("baseUrl")
                    .or_else(|| provider.get("baseUrl"))
                    .and_then(Value::as_str);
                let supported_auth = supported_auth
                    && model.get("headers").is_none()
                    && model.get("oauth").is_none()
                    && model
                        .get("compat")
                        .is_none_or(|_| catalog_digests.contains_key(provider_id.as_str()));
                let source_digest = CanonicalDigest::of(&(
                    "pi-api-source/v1",
                    &self.layout.pi_config_root,
                    provider_id,
                    id,
                ))
                .map_err(|_| Error::InvalidConfig)?;
                let source_ref = format!("pi-source/{}", &source_digest.as_str()[7..]);
                let evidence_digest = CanonicalDigest::of(&(
                    "pi-api-source-evidence/v1",
                    &source_ref,
                    &models_digest,
                    &auth_digest,
                    catalog_digests.get(provider_id.as_str()),
                    secret
                        .as_ref()
                        .map(|s| CanonicalDigest::of_bytes(s.as_bytes())),
                ))
                .map_err(|_| Error::InvalidConfig)?;
                let revision = u64::from_str_radix(&evidence_digest.as_str()[7..23], 16)
                    .map_err(|_| Error::InvalidConfig)?
                    .max(1);
                let credential =
                    secret
                        .as_ref()
                        .filter(|_| supported_auth)
                        .map(|_| DiscoveredCredentialRefV1 {
                            source: "discovered_config".into(),
                            scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.into(),
                            scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.into(),
                            discovered_source_ref: source_ref.clone(),
                            field_selector: "pi.api-key".into(),
                            observed_revision: revision,
                        });
                if selected == Some(source_ref.as_str())
                    && supported_auth
                    && let (Some(endpoint), Some(secret)) = (endpoint, secret.as_ref())
                {
                    material = Some((
                        Zeroizing::new(endpoint.to_owned()),
                        ProtectedSecret::new(secret.as_bytes().to_vec())
                            .map_err(|_| Error::InvalidConfig)?,
                    ));
                }
                items.push(PiApiSource {
                    provider_id: provider_id.clone(),
                    model_id: id.into(),
                    protocol,
                    display_name: model
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|v| bounded(v))
                        .unwrap_or(id)
                        .into(),
                    context_tokens: model.get("contextWindow").and_then(Value::as_u64),
                    max_output_tokens: model.get("maxTokens").and_then(Value::as_u64),
                    vision: model
                        .get("input")
                        .and_then(Value::as_array)
                        .map(|v| v.iter().any(|i| i == "image")),
                    reasoning: model.get("reasoning").and_then(Value::as_bool),
                    authentication,
                    source_ref,
                    evidence_digest,
                    revision,
                    credential,
                    supported_auth: supported_auth && endpoint.is_some(),
                });
            }
        }
        items.sort_by(|a, b| (&a.provider_id, &a.model_id).cmp(&(&b.provider_id, &b.model_id)));
        Ok((items, material))
    }
}
