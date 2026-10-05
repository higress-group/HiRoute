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

    fn from_live_result(
        target: &AgentModelCheckTargetV2,
        data: &Value,
    ) -> Result<Self, DesktopFailure> {
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
        Ok(Self {
            accepted: true,
            scope: "live".into(),
            model_call: true,
            state: Some(state.into()),
            call_count,
            requested_call_count,
        })
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
        let agent = match self.request.agent_id.as_str() {
            "agent_claude_default" => "Claude Code",
            "agent_qoder_default" => "Qoder",
            "agent_pi_default" => "Pi",
            _ => "Codex",
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
                AgentModelSurfaceV2::QoderCli => "Qoder CLI",
                AgentModelSurfaceV2::PiCli => "Pi CLI",
            };
            let models = target.client_model_ids.join(", ");
            let message = if self.english {
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
            return if target.surface == AgentModelSurfaceV2::QoderCli {
                format!(
                    "{message}\n\n{}",
                    if self.english {
                        "This checks only the selected additional HiRoute routes. Your native models, current default model and login configuration stay unchanged."
                    } else {
                        "仅验证所选的附加 HiRoute 路由；原生模型、当前默认模型和登录配置保持不变。"
                    }
                )
            } else {
                message
            };
        }
        if self.request.scope == AgentCheckScopeV1::Collaboration
            && self.request.agent_id == "agent_qoder_default"
        {
            return if self.english {
                "Check Qoder task collaboration?\nUses the selected Qoder CLI with your normal login and user Skills. The verification uses a separate workspace and a local test endpoint; it does not call an upstream provider or execute a delegated task. If collaboration is enabled, it checks your installed collaboration Skill. Otherwise it checks the capability to enable it. Your daily configuration is not changed.".into()
            } else {
                "检查 Qoder 任务协作？\n使用所选 Qoder CLI 的正常登录状态和用户技能，在独立验证目录中访问本机测试端点，不调用上游提供方，也不执行委派任务。已启用协作时检查已安装的用户协作技能；尚未启用时检查启用能力。不改写日常配置。".into()
            };
        }
        if self.request.scope == AgentCheckScopeV1::Collaboration
            && self.request.agent_id == "agent_pi_default"
        {
            return if self.english {
                "Check local Pi task delegation compatibility?\nReads the selected official SDK, native Skill settings and trusted sibling HiRoute CLI. No model call, delegated task or native configuration change.".into()
            } else {
                "检查本机 Pi 任务委派兼容性？\n读取所选官方 SDK、原生技能设置和同一安装中的 HiRoute CLI。不发起模型调用、不执行委派任务、不修改原生配置。".into()
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
            return AgentCheckCompletion::from_live_result(&target, &data);
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

impl AgentCheckInput {
    fn request(&self) -> Result<AgentCheckRequestV1, DesktopFailure> {
        if !matches!(self.language.as_str(), "zh" | "en") {
            return Err("AGENT_INPUT_INVALID".into());
        }
        let scope = match self.scope.as_deref() {
            None | Some("native_authentication") => AgentCheckScopeV1::NativeAuthentication,
            Some("collaboration") => AgentCheckScopeV1::Collaboration,
            Some("live") => AgentCheckScopeV1::Live,
            Some(_) => return Err("AGENT_INPUT_INVALID".into()),
        };
        let supported = match self.agent_id.as_str() {
            "agent_codex_default" | "agent_claude_default" => true,
            "agent_pi_default" => scope == AgentCheckScopeV1::Collaboration,
            "agent_qoder_default" => matches!(
                scope,
                AgentCheckScopeV1::Collaboration | AgentCheckScopeV1::Live
            ),
            _ => false,
        };
        if !supported {
            return Err("AGENT_INPUT_INVALID".into());
        }
        if scope == AgentCheckScopeV1::Live {
            let target = self.target.as_ref().ok_or("AGENT_INPUT_INVALID")?;
            let valid_surface = match self.agent_id.as_str() {
                "agent_claude_default" => target.surface == AgentModelSurfaceV2::ClaudeCli,
                "agent_codex_default" => matches!(
                    target.surface,
                    AgentModelSurfaceV2::CodexCli | AgentModelSurfaceV2::CodexDesktop
                ),
                "agent_qoder_default" => target.surface == AgentModelSurfaceV2::QoderCli,
                _ => false,
            };
            if !valid_surface {
                return Err("AGENT_INPUT_INVALID".into());
            }
        } else if self.target.is_some() {
            return Err("AGENT_INPUT_INVALID".into());
        }
        let request = AgentCheckRequestV1 {
            agent_id: self.agent_id.clone(),
            scope,
            suite: AgentCheckSuiteV1::Quick,
            allow_model_call: scope == AgentCheckScopeV1::Live,
            target: self.target.clone(),
        };
        if !request.valid_target() {
            return Err("AGENT_INPUT_INVALID".into());
        }
        Ok(request)
    }
}

impl Session {
    pub async fn prepare_agent_check(
        &mut self,
        input: AgentCheckInput,
    ) -> Result<AgentCheckConfirmation, DesktopFailure> {
        let request = input.request()?;
        let snapshot = self.snapshot().await?;
        if !snapshot.trusted_authority || !snapshot.service.mutation_available {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        #[cfg(unix)]
        if !self.resident.has_authority() {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
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

    pub(crate) fn begin_agent_check(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn live_target() -> AgentModelCheckTargetV2 {
        serde_json::from_value(json!({
            "context_id": "agent-context/qoder/test",
            "surface": "qoder_cli",
            "expected_applied_revision": 7,
            "client_model_ids": ["route-one", "route-two"]
        }))
        .unwrap()
    }

    #[test]
    fn additional_agents_keep_collaboration_independent_and_require_explicit_live_targets() {
        for agent in ["agent_qoder_default", "agent_pi_default"] {
            let mut input = AgentCheckInput {
                agent_id: agent.into(),
                language: "en".into(),
                scope: Some("collaboration".into()),
                target: None,
            };
            let request = input
                .request()
                .expect("Task routing can reach its local prerequisite check");
            assert_eq!(request.scope, AgentCheckScopeV1::Collaboration);
            assert!(!request.allow_model_call);
            assert!(request.target.is_none());
            input.target = Some(live_target());
            assert!(
                input.request().is_err(),
                "Task routing cannot borrow a model target"
            );
            input.target = None;
            for scope in [None, Some("native_authentication"), Some("live")] {
                input.scope = scope.map(str::to_owned);
                assert!(input.request().is_err());
            }
            if agent == "agent_pi_default" {
                input.target = Some(live_target());
                input.target.as_mut().unwrap().surface = AgentModelSurfaceV2::PiCli;
                assert!(input.request().is_err(), "Pi has no paid Live check");
            }
        }
        let mut input = AgentCheckInput {
            agent_id: "agent_qoder_default".into(),
            language: "en".into(),
            scope: Some("live".into()),
            target: Some(live_target()),
        };
        let request = input.request().expect("Explicit Qoder Live target");
        assert!(request.allow_model_call);
        assert_eq!(request.target, input.target);
        input
            .target
            .as_mut()
            .unwrap()
            .client_model_ids
            .push("route-one".into());
        assert!(
            input.request().is_err(),
            "Duplicate models are not an exact Live target"
        );
        input.target = None;
        for agent in ["agent_codex_default", "agent_claude_default"] {
            input.agent_id = agent.into();
            input.scope = None;
            assert!(
                input.request().is_ok(),
                "Existing clients keep native authentication"
            );
        }
    }

    #[test]
    fn live_checks_accept_only_the_requested_agents_exact_surfaces() {
        for (agent, allowed) in [
            (
                "agent_codex_default",
                vec![
                    AgentModelSurfaceV2::CodexCli,
                    AgentModelSurfaceV2::CodexDesktop,
                ],
            ),
            ("agent_claude_default", vec![AgentModelSurfaceV2::ClaudeCli]),
            ("agent_qoder_default", vec![AgentModelSurfaceV2::QoderCli]),
            ("agent_pi_default", vec![]),
        ] {
            for surface in [
                AgentModelSurfaceV2::CodexCli,
                AgentModelSurfaceV2::CodexDesktop,
                AgentModelSurfaceV2::ClaudeCli,
                AgentModelSurfaceV2::QoderCli,
                AgentModelSurfaceV2::PiCli,
            ] {
                let mut target = live_target();
                target.surface = surface;
                let input = AgentCheckInput {
                    agent_id: agent.into(),
                    language: "en".into(),
                    scope: Some("live".into()),
                    target: Some(target),
                };
                assert_eq!(
                    input.request().is_ok(),
                    allowed.contains(&surface),
                    "{agent}: {surface:?}"
                );
            }
        }
    }

    #[test]
    fn qoder_live_confirmation_describes_the_exact_paid_check_and_native_preservation() {
        for (language, required) in [
            (
                "en",
                [
                    "up to 2",
                    "revision 7",
                    "may be charged",
                    "additional HiRoute routes",
                    "current default model and login configuration stay unchanged",
                ],
            ),
            (
                "zh",
                [
                    "最多发起 2 次",
                    "已应用版本 7",
                    "可能产生提供方费用",
                    "附加 HiRoute 路由",
                    "当前默认模型和登录配置保持不变",
                ],
            ),
        ] {
            let input = AgentCheckInput {
                agent_id: "agent_qoder_default".into(),
                language: language.into(),
                scope: Some("live".into()),
                target: Some(live_target()),
            };
            let request = input.request().unwrap();
            let mut gate = ConfirmationGate::default();
            let context = AgentCheckConfirmation {
                permit: gate.begin().unwrap(),
                digest: CanonicalDigest::of(&request).unwrap(),
                request,
                revisions: RevisionSetV1 {
                    target: 3,
                    dependencies: Default::default(),
                },
                english: language == "en",
            };
            assert!(context.requires_confirmation());
            let message = context.message();
            for text in required
                .into_iter()
                .chain(["Qoder CLI", "route-one", "route-two"])
            {
                assert!(message.contains(text), "Missing {text}: {message}");
            }
            assert_eq!(
                context.revision(),
                3,
                "Service revision remains separate from applied revision"
            );
        }
    }

    #[test]
    fn qoder_live_completion_requires_the_confirmed_target_and_complete_success() {
        let target = live_target();
        let result = json!({
            "scope": "live", "model_call": true, "surface": "qoder_cli",
            "applied_revision": 7, "state": "passed",
            "checked_model_ids": ["route-one", "route-two"],
            "call_count": 2, "requested_call_count": 2
        });
        let completion = AgentCheckCompletion::from_live_result(&target, &result).unwrap();
        assert!(completion.passed());
        assert!(completion.model_call);
        for (field, value) in [
            ("surface", json!("codex_cli")),
            ("applied_revision", json!(8)),
            (
                "checked_model_ids",
                json!(["route-one", "unselected-route"]),
            ),
            ("call_count", json!(1)),
            ("requested_call_count", json!(1)),
            ("model_call", json!(false)),
        ] {
            let mut changed = result.clone();
            changed[field] = value;
            assert!(
                AgentCheckCompletion::from_live_result(&target, &changed).is_err(),
                "{field}"
            );
        }
        let mut partial = result;
        partial["checked_model_ids"] = json!(["route-one"]);
        partial["call_count"] = json!(1);
        assert!(AgentCheckCompletion::from_live_result(&target, &partial).is_err());
        partial["state"] = json!("failed");
        let failed = AgentCheckCompletion::from_live_result(&target, &partial).unwrap();
        assert!(!failed.passed());
        assert_eq!((failed.call_count, failed.requested_call_count), (1, 2));
    }
}
