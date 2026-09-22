//! Source drafts from an explicitly selected native Codex context. Never copies auth.json.
use super::source_candidates::safe_origin;
use super::{
    AgentFilesystemScanError as Error, AgentSourceCandidate, CodexSelectionTarget,
    DiscoveredAuthSource, FilesystemAgentScannerV1, ProtectedAgentSource,
};
use hiroute_domain::{AgentIngressProtocolV1, CanonicalDigest, ProtectedSecret};
use toml_edit::Item;
use zeroize::Zeroizing;

struct Draft {
    public: AgentSourceCandidate,
    endpoint: Zeroizing<String>,
    model: Option<Zeroizing<String>>,
    auth: Auth,
}
enum Auth {
    None,
    Environment(String),
    Inline(Zeroizing<String>),
}

impl FilesystemAgentScannerV1 {
    pub fn codex_source_candidates(
        &self,
        target: &CodexSelectionTarget,
    ) -> Result<Vec<AgentSourceCandidate>, Error> {
        let mut scope =
            super::CodexConfigurationScope::user_file(self.layout.codex_user_config.clone());
        scope.selection = target.clone();
        self.codex_source_candidates_in_scope(&scope)
    }
    pub fn codex_source_candidates_in_scope(
        &self,
        scope: &super::CodexConfigurationScope,
    ) -> Result<Vec<AgentSourceCandidate>, Error> {
        Ok(self
            .codex_source_draft(scope)?
            .map(|draft| draft.public)
            .into_iter()
            .collect())
    }
    pub fn read_selected_codex_source(
        &self,
        target: &CodexSelectionTarget,
        selected: &AgentSourceCandidate,
    ) -> Result<ProtectedAgentSource, Error> {
        let mut scope =
            super::CodexConfigurationScope::user_file(self.layout.codex_user_config.clone());
        scope.selection = target.clone();
        self.read_selected_codex_source_in_scope(&scope, selected)
    }
    pub fn read_selected_codex_source_in_scope(
        &self,
        scope: &super::CodexConfigurationScope,
        selected: &AgentSourceCandidate,
    ) -> Result<ProtectedAgentSource, Error> {
        let draft = self
            .codex_source_draft(scope)?
            .ok_or(Error::SourceChanged)?;
        if &draft.public != selected {
            return Err(Error::SourceChanged);
        }
        let credential = match draft.auth {
            Auth::None => None,
            Auth::Environment(key) => {
                let value =
                    Zeroizing::new(std::env::var(key).map_err(|_| Error::SourceUnavailable)?);
                Some(
                    ProtectedSecret::new(value.as_bytes().to_vec())
                        .map_err(|_| Error::SourceUnavailable)?,
                )
            }
            Auth::Inline(value) => Some(
                ProtectedSecret::new(value.as_bytes().to_vec())
                    .map_err(|_| Error::SourceUnavailable)?,
            ),
        };
        if self
            .codex_source_draft(scope)?
            .map(|draft| draft.public)
            .as_ref()
            != Some(selected)
        {
            return Err(Error::SourceChanged);
        }
        Ok(ProtectedAgentSource {
            endpoint: draft.endpoint,
            model: draft.model,
            credential,
        })
    }
    fn codex_source_draft(
        &self,
        scope: &super::CodexConfigurationScope,
    ) -> Result<Option<Draft>, Error> {
        let sampled = super::sample_codex_configuration(scope)?;
        let document = &sampled.source_document;
        // An absent or empty config.toml is a valid Codex configuration: the built-in
        // OpenAI provider and native login still apply.
        let value = |field: &str| -> Result<Option<&str>, Error> {
            document
                .get(field)
                .map(|item| item.as_str().ok_or(Error::InvalidConfig))
                .transpose()
        };
        let provider_id = value("model_provider")?.unwrap_or("openai");
        let model = value("model")?.map(|model| Zeroizing::new(model.to_owned()));
        let provider = document
            .get("model_providers")
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(provider_id))
            .and_then(Item::as_table);
        let provider_value = |field: &str| -> Result<Option<&str>, Error> {
            provider
                .and_then(|provider| provider.get(field))
                .map(|item| item.as_str().ok_or(Error::InvalidConfig))
                .transpose()
        };
        let endpoint = match provider_value("base_url")? {
            Some(value) => value,
            None if provider_id == "openai" => "https://api.openai.com/v1",
            None => return Ok(None),
        };
        if provider_value("wire_api")?.is_some_and(|wire| wire != "responses") {
            return Err(Error::InvalidConfig);
        }
        let inline = provider_value("experimental_bearer_token")?;
        let environment = provider_value("env_key")?;
        if let Some(key) = environment
            && !environment_identifier(key)
        {
            return Err(Error::InvalidConfig);
        }
        let native_auth = provider
            .and_then(|provider| provider.get("requires_openai_auth"))
            .map(|value| value.as_bool().ok_or(Error::InvalidConfig))
            .transpose()?;
        let session = provider_id == "openai" || native_auth.unwrap_or(false);
        let helper = provider.is_some_and(|provider| provider.contains_key("auth"));
        let mechanism_count = usize::from(inline.is_some())
            + usize::from(environment.is_some())
            + usize::from(session)
            + usize::from(helper);
        let (authentication, auth) = if mechanism_count > 1 {
            (DiscoveredAuthSource::Ambiguous, Auth::None)
        } else if session {
            (
                DiscoveredAuthSource::NativeSessionNeedsConfirmation,
                Auth::None,
            )
        } else if helper {
            (DiscoveredAuthSource::HelperNeedsInput, Auth::None)
        } else if let Some(key) = environment {
            (
                DiscoveredAuthSource::EnvironmentKey,
                Auth::Environment(key.to_owned()),
            )
        } else if let Some(token) = inline {
            (
                DiscoveredAuthSource::InlineToken,
                Auth::Inline(Zeroizing::new(token.to_owned())),
            )
        } else {
            (DiscoveredAuthSource::Missing, Auth::None)
        };
        let context = sampled.context_digest;
        let observed_digest = CanonicalDigest::of(&(
            "codex-source-observation/1",
            &context,
            sampled.dependency_digest,
            provider_id,
        ))
        .map_err(|_| Error::InvalidConfig)?;
        Ok(Some(Draft {
            public: AgentSourceCandidate {
                candidate_ref: format!("source/codex/{}", context.as_str()),
                context_ref: format!("agent-context/{}", context.as_str()),
                observed_digest,
                endpoint_origin: safe_origin(endpoint),
                native_ingress: AgentIngressProtocolV1::Responses,
                authentication,
            },
            endpoint: Zeroizing::new(endpoint.to_owned()),
            model,
            auth,
        }))
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn environment_identifier(value: &str) -> bool {
    identifier(value) && !value.contains('-') && !value.as_bytes()[0].is_ascii_digit()
}
