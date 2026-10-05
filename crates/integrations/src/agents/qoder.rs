//! Native Qoder context and transient routing. No account, model catalogue or auth file reads.
//!
//! The settings shape is an adapter detail, checked through real Responses requests. It is not
//! a public Qoder configuration API or a persisted HiRoute producer contract.
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

#[path = "qoder_budget.rs"]
mod budget;
pub use budget::{QoderTokenBudget, pi_plan_token_budget, qoder_plan_token_budget};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Qoder native check failed: {stage}")]
pub struct QoderNativeError {
    pub stage: &'static str,
}

impl From<hiroute_domain::QoderBudgetError> for QoderNativeError {
    fn from(error: hiroute_domain::QoderBudgetError) -> Self {
        qoder_error(error.stage)
    }
}

pub(super) fn qoder_error(stage: &'static str) -> QoderNativeError {
    QoderNativeError { stage }
}

/// The product instance chooses this context; the CLI location never chooses another HOME.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QoderNativeContext {
    pub home: PathBuf,
    pub config_root: PathBuf,
}

impl QoderNativeContext {
    pub fn from_selected(
        home: &Path,
        config_root: Option<&Path>,
    ) -> Result<Self, QoderNativeError> {
        let config_root = config_root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".qoder"));
        for path in [home, config_root.as_path()] {
            if !path.is_absolute()
                || path.to_str().is_none_or(|value| value.contains('\0'))
                || path.components().any(|part| part == Component::ParentDir)
            {
                return Err(qoder_error("selected native context"));
            }
        }
        Ok(Self {
            home: home.to_owned(),
            config_root,
        })
    }

    pub fn skill_target(&self) -> PathBuf {
        self.config_root
            .join("skills/hiroute-collaboration/SKILL.md")
    }
}

pub struct QoderTransientRouteInput<'a> {
    pub protocol: hiroute_domain::AgentIngressProtocolV1,
    pub provider_id: &'a str,
    /// Explicit numeric loopback origin followed by /v1, without query or credentials.
    pub endpoint: &'a str,
    pub alias: &'a str,
    /// Environment variable name only. Credential bytes are never accepted by this renderer.
    pub credential_env: &'a str,
    pub context_window_tokens: Option<u64>,
    /// Exact Worker Plan upper bound, or the independent probe's synthetic output budget.
    pub max_output_tokens: u64,
}

pub struct QoderTransientRoute {
    pub settings: Value,
    /// Only the native selector receives this prefix; Gateway and Plan retain the raw alias.
    pub native_model_id: String,
}

pub fn render_qoder_transient_route(
    input: QoderTransientRouteInput<'_>,
) -> Result<QoderTransientRoute, QoderNativeError> {
    super::qoder_provider::validate_route(input.provider_id, input.endpoint)?;
    if input.credential_env.is_empty()
        || input.credential_env.len() > 128
        || !input
            .credential_env
            .bytes()
            .enumerate()
            .all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
            })
    {
        return Err(qoder_error("transient route input"));
    }
    let native_model_id = format!("{}/{}", input.provider_id, input.alias);
    let model = super::qoder_provider::model(
        input.alias,
        input.context_window_tokens,
        input.max_output_tokens,
    )?;
    let own_alias = json!({"modelConfig": {"model": native_model_id}});
    let mut provider = super::qoder_provider::provider(
        input.endpoint,
        &format!("${{{}}}", input.credential_env),
        vec![model],
        input.protocol,
    );
    provider["model"] = input.alias.into();
    provider["routing"] = json!({
        "utility":input.alias,"session_title":input.alias,"summary":input.alias,
        "compact":input.alias,"subagent":input.alias,
    });
    let settings = json!({
        "general": {
            "enableAutoUpdate": false,
            "sessionRetention": {"enabled": false},
            "plan": {"modelRouting": false}
        },
        "disableAllHooks": true,
        "agent": "",
        "plugins": {"autoUpdate": false},
        "autoMemoryEnabled": false,
        "autoMemoryUserScopeEnabled": false,
        "dream": {"enabled": false},
        "promptSuggestionEnabled": false,
        "model": {"name": native_model_id},
        "modelConfigs": {
            "aliases": {(native_model_id.clone()): own_alias},
            "customAliases": {(native_model_id.clone()): own_alias},
            "overrides": [],
            "customOverrides": []
        },
        "providers": {(input.provider_id): provider}
    });
    Ok(QoderTransientRoute {
        settings,
        native_model_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_alias_is_preserved_and_credentials_remain_environment_references() {
        let route = render_qoder_transient_route(QoderTransientRouteInput {
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            provider_id: "hiroute-worker-123",
            endpoint: "http://127.0.0.1:1234/v1",
            alias: "plan/branch:cheap",
            credential_env: "HIROUTE_RUN_TOKEN",
            context_window_tokens: Some(100_000),
            max_output_tokens: 4096,
        })
        .unwrap();
        assert_eq!(
            route.native_model_id,
            "hiroute-worker-123/plan/branch:cheap"
        );
        let provider = &route.settings["providers"]["hiroute-worker-123"];
        assert_eq!(provider["model"], "plan/branch:cheap");
        assert_eq!(provider["apiKey"], "${HIROUTE_RUN_TOKEN}");
        assert_eq!(provider["models"][0]["contextWindow"], 100_000);
        assert_eq!(provider["models"][0]["maxOutputTokens"], 4096);
        for purpose in ["utility", "session_title", "summary", "compact", "subagent"] {
            assert_eq!(provider["routing"][purpose], "plan/branch:cheap");
        }
    }

    #[test]
    fn transient_route_rejects_foreign_origin_or_inline_credentials() {
        for endpoint in [
            "https://example.com/v1",
            "http://localhost:1234/v1",
            "http://secret@127.0.0.1:1234/v1",
            "http://127.0.0.1:1234/v1?token=x",
        ] {
            assert!(
                render_qoder_transient_route(QoderTransientRouteInput {
                    protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
                    provider_id: "hiroute",
                    endpoint,
                    alias: "plan/one",
                    credential_env: "HIROUTE_RUN_TOKEN",
                    context_window_tokens: None,
                    max_output_tokens: 2048,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn explicit_context_does_not_recover_another_home() {
        let context = QoderNativeContext::from_selected(
            Path::new("/pilot/home"),
            Some(Path::new("/pilot/qoder-context")),
        )
        .unwrap();
        assert_eq!(context.home, Path::new("/pilot/home"));
        assert_eq!(
            context.skill_target(),
            Path::new("/pilot/qoder-context/skills/hiroute-collaboration/SKILL.md")
        );
        assert!(QoderNativeContext::from_selected(Path::new("relative"), None).is_err());
    }
}
