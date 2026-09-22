//! Explicit native Agent checks using the existing confirmation and protected channel.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCheckInput {
    pub agent_id: String,
    pub language: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub target: Option<AgentModelCheckTargetV2>,
}

pub struct AgentCheckConfirmation {
    permit: ConfirmationPermit,
    request: AgentCheckRequestV1,
    digest: CanonicalDigest,
    revisions: RevisionSetV1,
    english: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentCheckCompletion {
    pub accepted: bool,
    pub scope: String,
    pub model_call: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub call_count: usize,
    pub requested_call_count: usize,
}

impl AgentCheckCompletion {
    pub(crate) fn passed(&self) -> bool {
        self.accepted && self.state.as_deref() == Some("passed")
    }

    fn cancelled(scope: AgentCheckScopeV1) -> Self {
        Self {
            accepted: false,
            scope: scope_name(scope).into(),
            model_call: false,
            state: None,
            call_count: 0,
            requested_call_count: 0,
        }
    }
}

pub(crate) struct PendingAgentCheck {
    client: hiroute_client_core::Client,
    request: AgentCheckRequestV1,
    capability: zeroize::Zeroizing<String>,
}

pub(crate) enum AgentCheckDispatch {
    Cancelled(AgentCheckCompletion),
    Pending(PendingAgentCheck),
}

impl AgentCheckConfirmation {
    pub fn revision(&self) -> u64 {
        self.revisions.target
    }

    pub fn english(&self) -> bool {
        self.english
    }

    pub fn requires_confirmation(&self) -> bool {
        self.request.scope == AgentCheckScopeV1::Live
    }

    pub fn is_live(&self) -> bool {
        self.request.scope == AgentCheckScopeV1::Live
    }

    pub fn message(&self) -> String {
        let agent = if self.request.agent_id == "agent_claude_default" {
            "Claude Code"
        } else {
            "Codex"
        };
        if self.request.scope == AgentCheckScopeV1::Live {
            let target = self
                .request
                .target
                .as_ref()
                .expect("Live check confirmation always has a target");
            let surface = match target.surface {
                AgentModelSurfaceV2::CodexCli => "Codex CLI",
                AgentModelSurfaceV2::CodexDesktop => "Codex Desktop",
                AgentModelSurfaceV2::ClaudeCli => "Claude Code CLI",
            };
            let models = target.client_model_ids.join(", ");
            return if self.english {
                format!(
                    "Verify {surface} model access?\n\nHiRoute will launch the actual configured client and make up to {} short model calls through applied revision {}. Provider usage may be charged.\n\nModels: {models}\n\nOnly complete responses with matching Gateway publication, grant, route, account and receipt evidence can pass.",
                    target.client_model_ids.len(),
                    target.expected_applied_revision.get(),
                )
            } else {
                format!(
                    "验证 {surface} 模型接入？\n\nHiRoute 将启动实际已配置客户端，并通过已应用版本 {} 最多发起 {} 次简短模型调用，可能产生提供方费用。\n\n模型：{models}\n\n只有完整响应与 Gateway 发布、授权、路由、账号及回执证据全部一致时才会通过。",
                    target.expected_applied_revision.get(),
                    target.client_model_ids.len(),
                )
            };
        }
        if self.request.scope == AgentCheckScopeV1::Collaboration && self.english {
            return format!(
                "Check local {agent} task delegation compatibility?\nLaunches the installed {agent} client in a private environment and verifies isolated Skill loading plus read-only trusted CLI execution. No provider call or daily configuration change."
            );
        } else if self.request.scope == AgentCheckScopeV1::Collaboration {
            return format!(
                "检查本机 {agent} 任务委派兼容性？\n将在私有环境中启动已安装的 {agent} 客户端，验证隔离 Skill 加载与只读受信 CLI 执行。不调用上游提供方，不改写日常配置。"
            );
        }
        if self.english {
            format!(
                "Check local {agent} compatibility?\nLaunches {agent} in a private empty environment and tests authentication against a local challenge endpoint. No provider call or daily configuration change. This does not verify model access."
            )
        } else {
            format!(
                "检查本机 {agent} 兼容性？\n将在私有空环境中启动 {agent}，向本机挑战端点验证认证。不调用上游提供方，不改写日常配置。这不代表模型接入已经验证。"
            )
        }
    }
}

impl PendingAgentCheck {
    pub(crate) async fn execute(self) -> Result<AgentCheckCompletion, DesktopFailure> {
        let scope = self.request.scope;
        let target = self.request.target.clone();
        let envelope: MachineEnvelopeV2<Value> = self
            .client
            .call_wire(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: crate::random_id()?,
                operation_id: "CheckAgentConnection".into(),
                payload: serde_json::to_value(&self.request).map_err(|_| "REQUEST_INVALID")?,
                protected_grant: Some(ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: self.capability.to_string(),
                }),
            })
            .await?;
        if envelope.error.is_some() {
            return Err(DesktopFailure::backend(envelope));
        }
        let data = envelope.data.ok_or("RESPONSE_DATA_MISSING")?;
        if scope == AgentCheckScopeV1::Live {
            let target = target.ok_or("RESPONSE_DATA_INVALID")?;
            let state = data["state"].as_str().ok_or("RESPONSE_DATA_INVALID")?;
            let checked: Vec<String> = serde_json::from_value(data["checked_model_ids"].clone())
                .map_err(|_| "RESPONSE_DATA_INVALID")?;
            let call_count = data["call_count"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("RESPONSE_DATA_INVALID")?;
            let requested_call_count = data["requested_call_count"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("RESPONSE_DATA_INVALID")?;
            let valid = data["scope"] == "live"
                && data["model_call"] == true
                && data["surface"]
                    == serde_json::to_value(target.surface).map_err(|_| "RESPONSE_DATA_INVALID")?
                && data["applied_revision"]
                    == serde_json::to_value(target.expected_applied_revision)
                        .map_err(|_| "RESPONSE_DATA_INVALID")?
                && matches!(state, "passed" | "failed")
                && call_count == checked.len()
                && requested_call_count == target.client_model_ids.len()
                && checked
                    .iter()
                    .all(|model| target.client_model_ids.contains(model))
                && (state != "passed" || checked == target.client_model_ids);
            if !valid {
                return Err("RESPONSE_DATA_INVALID".into());
            }
            return Ok(AgentCheckCompletion {
                accepted: true,
                scope: "live".into(),
                model_call: true,
                state: Some(state.into()),
                call_count,
                requested_call_count,
            });
        }
        let valid = if scope == AgentCheckScopeV1::Collaboration {
            data["scope"] == "collaboration"
                && data["skill_loading"] == "proven"
                && data["trusted_cli_execution"] == "proven"
                && data["model_call"] == false
        } else {
            data["scope"] == "native_authentication"
                && data["native_authentication"] == "proven"
                && data["model_verified"] == false
        };
        if !valid {
            return Err("RESPONSE_DATA_INVALID".into());
        }
        Ok(AgentCheckCompletion {
            accepted: true,
            scope: scope_name(scope).into(),
            model_call: false,
            state: Some("passed".into()),
            call_count: 0,
            requested_call_count: 0,
        })
    }
}

impl Session {
    pub async fn prepare_agent_check(
        &mut self,
        input: AgentCheckInput,
    ) -> Result<AgentCheckConfirmation, DesktopFailure> {
        if !matches!(input.language.as_str(), "zh" | "en") {
            return Err("AGENT_INPUT_INVALID".into());
        }
        let scope = match input.scope.as_deref() {
            None | Some("native_authentication") => AgentCheckScopeV1::NativeAuthentication,
            Some("collaboration") => AgentCheckScopeV1::Collaboration,
            Some("live") => AgentCheckScopeV1::Live,
            Some(_) => return Err("AGENT_INPUT_INVALID".into()),
        };
        if !matches!(
            input.agent_id.as_str(),
            "agent_codex_default" | "agent_claude_default"
        ) {
            return Err("AGENT_INPUT_INVALID".into());
        }
        if scope == AgentCheckScopeV1::Live {
            let target = input.target.as_ref().ok_or("AGENT_INPUT_INVALID")?;
            let valid_surface = match input.agent_id.as_str() {
                "agent_claude_default" => target.surface == AgentModelSurfaceV2::ClaudeCli,
                "agent_codex_default" => target.surface != AgentModelSurfaceV2::ClaudeCli,
                _ => false,
            };
            if !valid_surface {
                return Err("AGENT_INPUT_INVALID".into());
            }
        } else if input.target.is_some() {
            return Err("AGENT_INPUT_INVALID".into());
        }
        let snapshot = self.snapshot().await?;
        if !snapshot.trusted_authority || !snapshot.service.mutation_available {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        #[cfg(unix)]
        if !self.resident.has_authority() {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        let request = AgentCheckRequestV1 {
            agent_id: input.agent_id,
            scope,
            suite: AgentCheckSuiteV1::Quick,
            allow_model_call: scope == AgentCheckScopeV1::Live,
            target: input.target,
        };
        if !request.valid_target() {
            return Err("AGENT_INPUT_INVALID".into());
        }
        let digest = CanonicalDigest::of(&request).map_err(|_| "AGENT_INPUT_INVALID")?;
        Ok(AgentCheckConfirmation {
            permit: self.confirmation.begin()?,
            request,
            digest,
            revisions: snapshot.service.revisions,
            english: input.language == "en",
        })
    }

    pub fn begin_agent_check(
        &mut self,
        context: AgentCheckConfirmation,
        accepted: bool,
    ) -> Result<AgentCheckDispatch, DesktopFailure> {
        let scope = context.request.scope;
        if !self.confirmation.finish(&context.permit, accepted)? {
            return Ok(AgentCheckDispatch::Cancelled(
                AgentCheckCompletion::cancelled(scope),
            ));
        }
        #[cfg(unix)]
        let capability = self
            .resident
            .register_agent_check(&context.digest, &context.revisions)?;
        #[cfg(not(unix))]
        let capability: zeroize::Zeroizing<String> = return Err("UNSUPPORTED_PLATFORM".into());
        Ok(AgentCheckDispatch::Pending(PendingAgentCheck {
            client: self.client.clone(),
            request: context.request,
            capability,
        }))
    }
}

fn scope_name(scope: AgentCheckScopeV1) -> &'static str {
    match scope {
        AgentCheckScopeV1::Configuration => "configuration",
        AgentCheckScopeV1::NativeAuthentication => "native_authentication",
        AgentCheckScopeV1::Collaboration => "collaboration",
        AgentCheckScopeV1::Live => "live",
    }
}
