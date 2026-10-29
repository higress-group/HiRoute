use std::path::Path;

use hiroute_application_api::{
    AGENT_SETTINGS_SCHEMA_V2, AgentAccessTokenIntentV1, AgentCheckRequestV1, AgentCheckScopeV1,
    AgentCheckSuiteV1, AgentCollaborationSelectionV2, AgentFacetIntent, AgentSettingsApplyV2,
    AgentSettingsPreviewRequestV2, AgentSettingsSpecV2, AgentSettingsStatusRequestV2,
    AgentSettingsStatusV2, CanonicalDigest, ErrorCode, ErrorV1, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlWireRequestV2, MachineEnvelopeV2, MachineStatus, NextActionV1, RevisionSetV1,
};
use hiroute_client_core::{ClientFailure, SubmissionState};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Intent, Invocation, Selection, random_key, receipt};

pub(super) trait Control {
    fn call(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<Value>, ClientFailure>;
}
impl Control for crate::LocalControlClient {
    fn call(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<Value>, ClientFailure> {
        self.call_detailed(request)
    }
}

// Deliberately decode only the safe scan projection this presentation workflow consumes.
// The daemon's discovered credential and model catalog projections are not needed here.
#[derive(Deserialize)]
struct Scan {
    agents: Vec<Agent>,
}
#[derive(Deserialize)]
struct Agent {
    agent_id: String,
    supported: bool,
    context_id: Option<String>,
    codex_access: Option<CodexAccess>,
}
#[derive(Deserialize)]
struct CodexAccess {
    root_context_id: String,
}
#[derive(Deserialize)]
struct Preview {
    schema: String,
    spec: AgentSettingsSpecV2,
    applicable: bool,
    accept_digest: CanonicalDigest,
    dependency_digest: CanonicalDigest,
    expected_revisions: RevisionSetV1,
    blockers: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    #[serde(default)]
    capabilities: Vec<Capability>,
}
#[derive(Deserialize)]
struct Capability {
    capability: String,
}

pub(super) fn run(
    client: &impl Control,
    receipts: &Path,
    invocation: Invocation,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    match workflow(client, receipts, &invocation, &request_id) {
        Ok(response) => response,
        Err(response) => *response,
    }
}
type FlowResult<T> = Result<T, Box<MachineEnvelopeV2<Value>>>;

fn workflow(
    client: &impl Control,
    receipts: &Path,
    invocation: &Invocation,
    request_id: &str,
) -> FlowResult<MachineEnvelopeV2<Value>> {
    let selection = &invocation.selection;
    let key = if invocation.preview || selection.intent == Intent::Status {
        None
    } else {
        Some(
            invocation
                .key
                .clone()
                .map(Ok)
                .unwrap_or_else(random_key)
                .map_err(|code| fail(code, request_id))?,
        )
    };
    if let Some(key) = key.as_deref() {
        // Replay the immutable original request before discovery/check/preview can change its
        // accepted digest. A key reused for a different intent fails before any native check.
        if let Some(saved) =
            receipt::load(receipts, key, selection).map_err(|code| fail(code, request_id))?
        {
            return Ok(submit(client, saved, selection, request_id));
        }
    }
    let scan: Scan = query(client, "ScanAgents", json!({}), request_id)?;
    let mut agents = scan
        .agents
        .into_iter()
        .filter(|agent| agent.agent_id == selection.agent_id);
    let agent = agents
        .next()
        .ok_or_else(|| fail(ErrorCode::ResourceNotFound, request_id))?;
    if agents.next().is_some() {
        return Err(fail(ErrorCode::InvalidArguments, request_id));
    }
    if !agent.supported {
        return Err(fail(ErrorCode::NoSupportedAgent, request_id));
    }
    let context_id = if let Some(access) = agent.codex_access {
        access.root_context_id
    } else {
        agent
            .context_id
            .ok_or_else(|| fail(ErrorCode::ResourceNotFound, request_id))?
    };
    if !super::bounded(&context_id, 256) {
        return Err(fail(ErrorCode::DaemonUnavailable, request_id));
    }
    if selection.intent == Intent::Status {
        return call(
            client,
            "GetAgentConnectionStatus",
            json!(AgentSettingsStatusRequestV2 {
                projection: Default::default(),
                effective: None,
                include_skills: true,
                native_target: None,
                schema_version: AGENT_SETTINGS_SCHEMA_V2,
                context_id
            }),
            request_id,
        );
    }
    let collaboration = match selection.intent {
        Intent::Configure { trigger_mode } => AgentFacetIntent::Configure {
            settings: AgentCollaborationSelectionV2 { trigger_mode },
        },
        Intent::Disable => {
            let status: AgentSettingsStatusV2 = query(
                client,
                "GetAgentConnectionStatus",
                json!(AgentSettingsStatusRequestV2 {
                    projection: Default::default(),
                    effective: None,
                    include_skills: true,
                    native_target: None,
                    schema_version: AGENT_SETTINGS_SCHEMA_V2,
                    context_id: context_id.clone()
                }),
                request_id,
            )?;
            if !status.valid() || status.context_id() != context_id {
                return Err(fail(ErrorCode::DaemonUnavailable, request_id));
            }
            let restore_point_ref = status
                .collaboration()
                .and_then(|facet| facet.restore_point_ref.clone())
                .ok_or_else(|| fail(ErrorCode::ResourceNotFound, request_id))?;
            AgentFacetIntent::Restore { restore_point_ref }
        }
        Intent::Status => unreachable!(),
    };
    let spec = AgentSettingsSpecV2 {
        native_target: None,
        management_skill: None,
        schema_version: AGENT_SETTINGS_SCHEMA_V2,
        context_id,
        model: AgentFacetIntent::Keep,
        collaboration,
        restore_native_model: None,
        protected_native_model_ids: Vec::new(),
        access_token: AgentAccessTokenIntentV1::Keep,
    };
    let restore = spec.is_restore_only();
    let preview_operation = if restore {
        "PreviewAgentConnectionRestore"
    } else {
        "PreviewAgentConnectionChange"
    };
    let mut response = call(
        client,
        preview_operation,
        json!(AgentSettingsPreviewRequestV2 { spec: spec.clone() }),
        request_id,
    )?;
    let mut preview = decode_preview(&response, &spec, request_id)?;
    if invocation.preview {
        return Ok(response);
    }
    // Match Desktop's prerequisite selection, but re-evaluate after each check. A successful
    // local check is not proof that the fresh preview is applicable or that Skill installation
    // has occurred. Each scope is attempted at most once; Live is never part of this workflow.
    let mut checked = Vec::new();
    while !preview.applicable {
        let scope = prerequisite(&preview);
        let Some(scope) = scope.filter(|scope| !checked.contains(scope)) else {
            response.status = MachineStatus::Conflict;
            response.error = Some(ErrorV1::new(ErrorCode::FeatureNotEnabled));
            return Err(Box::new(response));
        };
        call(
            client,
            "CheckAgentConnection",
            json!(AgentCheckRequestV1 {
                agent_id: selection.agent_id.clone(),
                scope,
                suite: AgentCheckSuiteV1::Quick,
                allow_model_call: false,
                target: None
            }),
            request_id,
        )?;
        checked.push(scope);
        response = call(
            client,
            preview_operation,
            json!(AgentSettingsPreviewRequestV2 { spec: spec.clone() }),
            request_id,
        )?;
        preview = decode_preview(&response, &spec, request_id)?;
    }
    let apply = AgentSettingsApplyV2 {
        spec: preview.spec,
        accept_digest: preview.accept_digest,
        dependency_digest: preview.dependency_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: key.expect("a save has a submission key"),
        login_item: None,
    };
    let saved =
        receipt::freeze(receipts, selection, apply).map_err(|code| fail(code, request_id))?;
    Ok(submit(client, saved, selection, request_id))
}

fn decode_preview(
    response: &MachineEnvelopeV2<Value>,
    spec: &AgentSettingsSpecV2,
    request_id: &str,
) -> FlowResult<Preview> {
    let preview: Preview = serde_json::from_value(
        response
            .data
            .clone()
            .ok_or_else(|| fail(ErrorCode::DaemonUnavailable, request_id))?,
    )
    .map_err(|_| fail(ErrorCode::DaemonUnavailable, request_id))?;
    // Preview may seal protected native model names even for model.keep. Preserve those names
    // in Apply, while rejecting any change to the user's context, facets or token intent.
    let mut intent = preview.spec.clone();
    intent.protected_native_model_ids.clear();
    if preview.schema != "hiroute.agent-settings-preview/v2"
        || &intent != spec
        || preview.applicable != preview.blockers.is_empty()
    {
        return Err(fail(ErrorCode::DaemonUnavailable, request_id));
    }
    Ok(preview)
}

fn prerequisite(preview: &Preview) -> Option<AgentCheckScopeV1> {
    let missing = |name| {
        preview
            .blockers
            .iter()
            .flat_map(|block| &block.capabilities)
            .any(|capability| capability.capability == name)
    };
    if missing("ingress_authentication") {
        Some(AgentCheckScopeV1::NativeAuthentication)
    } else if missing("skill_loading") || missing("trusted_cli_execution") {
        Some(AgentCheckScopeV1::Collaboration)
    } else {
        None
    }
}

fn submit(
    client: &impl Control,
    apply: AgentSettingsApplyV2,
    selection: &Selection,
    request_id: &str,
) -> MachineEnvelopeV2<Value> {
    let operation = if apply.spec.is_restore_only() {
        "ApplyAgentConnectionRestore"
    } else {
        "ApplyAgentConnectionChange"
    };
    let mut response = match client.call(wire(operation, json!(apply), request_id)) {
        Ok(response) => response,
        Err(error) => {
            let mut response = *fail(ErrorCode::DaemonUnavailable, request_id);
            response.next_actions.push(NextActionV1 {
                command_id: if apply.spec.is_restore_only() {
                    "agents.restore.apply"
                } else {
                    "agents.connect.apply"
                }
                .into(),
                input: json!(apply),
                reason_code: match error.submission {
                    SubmissionState::NotSent => "SUBMISSION_NOT_SENT",
                    SubmissionState::MayHaveReachedServer => "SUBMISSION_UNKNOWN",
                }
                .into(),
            });
            response
        }
    };
    let submission = json!({"agent_id":selection.agent_id, "context_id":apply.spec.context_id, "idempotency_key":apply.idempotency_key});
    let data = response.data.get_or_insert_with(|| json!({}));
    if let Some(data) = data.as_object_mut() {
        data.insert("cli_submission".into(), submission);
    }
    response
}

fn wire(operation: &str, payload: Value, request_id: &str) -> LocalControlWireRequestV2 {
    LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: request_id.into(),
        operation_id: operation.into(),
        payload,
        protected_grant: None,
    }
}
fn call(
    client: &impl Control,
    operation: &str,
    payload: Value,
    request_id: &str,
) -> FlowResult<MachineEnvelopeV2<Value>> {
    let response = client
        .call(wire(operation, payload, request_id))
        .map_err(|_| fail(ErrorCode::DaemonUnavailable, request_id))?;
    if response.status == MachineStatus::Succeeded {
        Ok(response)
    } else {
        Err(Box::new(response))
    }
}
fn query<T: serde::de::DeserializeOwned>(
    client: &impl Control,
    operation: &str,
    payload: Value,
    request_id: &str,
) -> FlowResult<T> {
    let response = call(client, operation, payload, request_id)?;
    serde_json::from_value(
        response
            .data
            .ok_or_else(|| fail(ErrorCode::DaemonUnavailable, request_id))?,
    )
    .map_err(|_| fail(ErrorCode::DaemonUnavailable, request_id))
}
fn fail(code: ErrorCode, request_id: &str) -> Box<MachineEnvelopeV2<Value>> {
    Box::new(MachineEnvelopeV2::failed(
        ErrorV1::new(code),
        Some(request_id.into()),
    ))
}
