//! Passive explicit DSH API routes. Catalog/OAuth discovery and dynamic compositions are excluded.
use super::{
    AgentFilesystemScanError as Error, DiscoveredAuthSource, DiscoveredCredentialRefV1,
    FILESYSTEM_AGENT_SCANNER_ID_V1, FILESYSTEM_AGENT_SCANNER_VERSION_V1, FilesystemAgentScannerV1,
    NativeApiSource,
};
use hiroute_domain::{CanonicalDigest, ProtectedSecret, UpstreamProtocol};
use serde_json::{Value, json};
use zeroize::Zeroizing;
type Snapshot = (
    Vec<NativeApiSource>,
    Option<(Zeroizing<String>, ProtectedSecret)>,
);

fn read_credentials(path: &std::path::Path) -> Result<Zeroizing<Vec<u8>>, Error> {
    let Some((bytes, _metadata)) = super::filesystem_config::read_validated_config_bytes(path)?
    else {
        return Ok(Zeroizing::new(Vec::new()));
    };
    Ok(bytes)
}

fn credentials_path(
    root: &std::path::Path,
    home: &super::dsh_config::Patch,
    profile: &super::dsh_config::Patch,
) -> Result<std::path::PathBuf, Error> {
    let profile = profile
        .credentials_config()
        .map_err(|_| Error::InvalidConfig)?;
    let home = home
        .credentials_config()
        .map_err(|_| Error::InvalidConfig)?;
    // Home follows Web; each supplied config replaces the whole earlier object.
    let config = home.or(profile);
    if config.is_some_and(|c| {
        !c.is_object()
            || c.as_object().is_some_and(|o| {
                o.keys()
                    .any(|k| !matches!(k.as_str(), "path" | "dshHome" | "watch" | "debounceMs"))
            })
    }) {
        return Err(Error::InvalidConfig);
    }
    let configured = config.and_then(|c| c.get("path").or_else(|| c.get("dshHome")));
    let path = if let Some(configured) = configured {
        let path = configured
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0'))
            .ok_or(Error::InvalidConfig)?;
        let path = std::path::Path::new(path);
        // Native relative/tilde resolution depends on invocation context; reject
        // instead of borrowing HiRoute's cwd or HOME to guess a different file.
        if !path.is_absolute() {
            return Err(Error::InvalidConfig);
        }
        if config.is_some_and(|c| c.get("path").is_some()) {
            path.to_path_buf()
        } else {
            path.join(".credentials.yaml")
        }
    } else {
        root.join(".credentials.yaml")
    };
    if !path.is_absolute() {
        return Err(Error::InvalidConfig);
    }
    // Match Node's lexical resolve, including `..` before following symlinks.
    let mut resolved = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            _ => resolved.push(component.as_os_str()),
        }
    }
    Ok(resolved)
}
impl FilesystemAgentScannerV1 {
    pub fn dsh_api_sources(&self) -> Result<Vec<NativeApiSource>, Error> {
        Ok(self.dsh_sources_inner(None)?.0)
    }
    pub fn read_dsh_api_source(
        &self,
        selected: &NativeApiSource,
    ) -> Result<(Zeroizing<String>, ProtectedSecret), Error> {
        let (sources, material) = self.dsh_sources_inner(Some(&selected.source_ref))?;
        if !sources.contains(selected) || !self.dsh_api_sources()?.contains(selected) {
            return Err(Error::SourceChanged);
        }
        material.ok_or(Error::SourceUnavailable)
    }
    pub(super) fn read_dsh_secret(
        &self,
        descriptor: &DiscoveredCredentialRefV1,
    ) -> Result<ProtectedSecret, Error> {
        let source = self
            .dsh_api_sources()?
            .into_iter()
            .find(|s| s.credential.as_ref() == Some(descriptor))
            .ok_or(Error::SourceChanged)?;
        Ok(self.read_dsh_api_source(&source)?.1)
    }
    fn dsh_sources_inner(&self, selected: Option<&str>) -> Result<Snapshot, Error> {
        let home = super::dsh_config::read_patch_bytes(
            &self.layout.dsh_config_root.join("cordis.patch.yml"),
        )?;
        let profile = super::dsh_config::read_patch_bytes(&self.dsh_user_models_target())?;
        let home_patch =
            super::dsh_config::Patch::parse(Some(&home)).map_err(|_| Error::InvalidConfig)?;
        let profile_patch =
            super::dsh_config::Patch::parse(Some(&profile)).map_err(|_| Error::InvalidConfig)?;
        let providers = super::pi_sources::Document(
            if home_patch
                .config("llm-pi-ai")
                .map_err(|_| Error::InvalidConfig)?
                .is_some()
            {
                home_patch.providers()
            } else {
                profile_patch.providers()
            }
            .map_err(|_| Error::InvalidConfig)?,
        );
        let credentials_path =
            credentials_path(&self.layout.dsh_config_root, &home_patch, &profile_patch)?;
        let credentials = read_credentials(&credentials_path)?;
        let credentials_value = super::pi_sources::Document(if credentials.is_empty() {
            json!({"version":1})
        } else {
            serde_yaml::from_slice::<Value>(&credentials).map_err(|_| Error::InvalidConfig)?
        });
        if credentials_value.0["version"] != 1 {
            return Err(Error::InvalidConfig);
        }
        let mut sources = Vec::new();
        let mut material = None;
        for (provider_id, provider) in providers.0.as_object().ok_or(Error::InvalidConfig)? {
            if provider_id.starts_with("hiroute-") {
                continue;
            }
            if sources.len() >= 256 {
                return Err(Error::ConfigTooLarge);
            }
            let reference = provider["apiKeyEnv"].as_str().filter(|r| {
                r.bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && r.len() <= 256
                    && r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
            let secret = reference
                .and_then(|r| {
                    std::env::var(r)
                        .ok()
                        .filter(|v| !v.is_empty())
                        .or_else(|| credentials_value.0["refs"][r].as_str().map(str::to_owned))
                })
                .map(Zeroizing::new);
            let protocol = match provider["api"].as_str() {
                Some("openai-responses") => Some(UpstreamProtocol::Responses),
                Some("anthropic-messages") => Some(UpstreamProtocol::Messages),
                Some("openai-completions") => Some(UpstreamProtocol::ChatCompletions),
                _ => None,
            };
            let endpoint = provider["baseURL"].as_str().filter(|s| s.len() <= 4096);
            let supported = endpoint.is_some()
                && provider.get("compat").is_none()
                && provider.get("modelOverrides").is_none()
                && provider
                    .get("headers")
                    .is_none_or(|h| h.as_object().is_some_and(|o| o.is_empty()));
            let Some(models) = provider
                .get("models")
                .map(|m| m.as_array().ok_or(Error::InvalidConfig))
                .transpose()?
            else {
                // Native catalog inheritance is not an explicit, portable model declaration.
                continue;
            };
            let mut seen = std::collections::BTreeSet::new();
            for model in models {
                if sources.len() >= 256 {
                    return Err(Error::ConfigTooLarge);
                }
                let id = model["id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                    .ok_or(Error::InvalidConfig)?;
                if !seen.insert(id) {
                    return Err(Error::InvalidConfig);
                }
                let supported = supported && model.get("compat").is_none();
                let digest = CanonicalDigest::of(&(
                    "dsh-api-source/v1",
                    &self.layout.dsh_config_root,
                    provider_id,
                    id,
                ))
                .map_err(|_| Error::InvalidConfig)?;
                let source_ref = format!("dsh-source/{}", &digest.as_str()[7..]);
                let evidence_digest = CanonicalDigest::of(&(
                    "dsh-api-evidence/v1",
                    &source_ref,
                    CanonicalDigest::of_bytes(&home),
                    CanonicalDigest::of_bytes(&profile),
                    &credentials_path,
                    CanonicalDigest::of_bytes(&credentials),
                    secret
                        .as_ref()
                        .map(|s| CanonicalDigest::of_bytes(s.as_bytes())),
                ))
                .map_err(|_| Error::InvalidConfig)?;
                let revision = u64::from_str_radix(&evidence_digest.as_str()[7..23], 16)
                    .map_err(|_| Error::InvalidConfig)?
                    .max(1);
                let credential = secret
                    .as_ref()
                    .filter(|s| supported && !s.is_empty())
                    .map(|_| DiscoveredCredentialRefV1 {
                        source: "discovered_config".into(),
                        scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.into(),
                        scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.into(),
                        discovered_source_ref: source_ref.clone(),
                        field_selector: "dsh.api-key".into(),
                        observed_revision: revision,
                    });
                if selected == Some(source_ref.as_str())
                    && supported
                    && let (Some(endpoint), Some(secret)) = (endpoint, secret.as_ref())
                {
                    material = Some((
                        Zeroizing::new(endpoint.to_owned()),
                        ProtectedSecret::new(secret.as_bytes().to_vec())
                            .map_err(|_| Error::InvalidConfig)?,
                    ));
                }
                sources.push(NativeApiSource {
                    provider_id: provider_id.clone(),
                    model_id: id.into(),
                    protocol,
                    display_name: model["name"].as_str().unwrap_or(id).into(),
                    context_tokens: model["contextWindow"].as_u64(),
                    max_output_tokens: model["maxTokens"].as_u64(),
                    vision: model["input"]
                        .as_array()
                        .filter(|a| !a.is_empty())
                        .map(|a| a.iter().any(|v| v == "image")),
                    reasoning: None,
                    authentication: if secret.is_some() {
                        DiscoveredAuthSource::InlineToken
                    } else {
                        DiscoveredAuthSource::Missing
                    },
                    source_ref,
                    evidence_digest,
                    revision,
                    credential,
                    supported_auth: supported,
                });
            }
        }
        Ok((sources, material))
    }
}
