//! Registration must reach the real exact-plan Gateway authority for every supported Harness.
use super::*;
use hiroute_application::delegation::safety::{RunSafetyBinding, RunSafetyProjection};
use hiroute_application::publication::admission::SharedAdmissionGate;
use hiroute_domain::delegation::*;
use hiroute_domain::{
    AgentPlanAuthoringV2, AgentPlanStrategyV2, CandidateSelectionV1, CapabilityRequirementsV1,
    ExactNativeReasoningV1, PLAN_AUTHORING_SCHEMA_V2, PlanEditorMode, ReasoningSelectionV1,
    WorkerPlanV1, WorkspaceId,
};

use crate::delegation::credentials::{RunCredentialPair, RunCredentialRecord};

fn version(harness: WorkerHarnessV1) -> PlanVersionV1 {
    let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
        "../../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let compiled = publication
        .plans
        .into_iter()
        .find(|plan| plan.agent_plan_id().as_str() == "plan/custom")
        .unwrap();
    let candidates = compiled.body.materialized.attempt_owned.groups[0]
        .candidates
        .iter()
        .map(|candidate| CandidateSelectionV1 {
            binding_id: candidate.binding_id.clone(),
            reasoning: match &candidate.exact_reasoning {
                ExactNativeReasoningV1::Fixed { .. } => None,
                ExactNativeReasoningV1::Toggle { enabled, .. } => {
                    Some(ReasoningSelectionV1::Toggle { enabled: *enabled })
                }
                ExactNativeReasoningV1::Profile { profile, .. } => {
                    Some(ReasoningSelectionV1::Profile {
                        profile: profile.clone(),
                    })
                }
                ExactNativeReasoningV1::Budget { tokens, .. } => {
                    Some(ReasoningSelectionV1::Budget { tokens: *tokens })
                }
            },
        })
        .collect();
    PlanVersionV1::new(
        WorkspaceId::default(),
        AgentPlanAuthoringV2 {
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            display_name: compiled.body.identity.display_name.clone(),
            purpose: compiled.body.identity.purpose.clone(),
            mode: PlanEditorMode::FixedModel,
            requirements: CapabilityRequirementsV1::default(),
            limits: compiled.body.materialized.attempt_owned.limits.clone(),
            strategy: AgentPlanStrategyV2::Custom { candidates },
            delegation_enabled: true,
            work: Some(WorkerPlanV1 {
                harness,
                protocol: if harness == WorkerHarnessV1::ClaudeCode {
                    AgentIngressProtocolV1::Messages
                } else {
                    AgentIngressProtocolV1::Responses
                },
            }),
        },
        compiled,
    )
    .unwrap()
}

struct Fixture {
    version: Arc<PlanVersionV1>,
    task: DelegationTaskV1,
    run: DelegationRunV1,
    pair: RunCredentialPair,
    verifier: Arc<RunCredentialVerifier>,
}

impl Fixture {
    fn new(harness: WorkerHarnessV1) -> Self {
        let version = Arc::new(version(harness));
        let now = now_ms().unwrap();
        let run = DelegationRunV1 {
            workspace_id: WorkspaceId::default(),
            task_id: "task/one".into(),
            run_id: "run/one".into(),
            ordinal: 1,
            continued_from: None,
            idempotency_key: "start/one".into(),
            request_digest: CanonicalDigest::of_bytes(b"request"),
            admission_sequence: 1,
            accepted_at_ms: Some(now),
            execution_owner_ref: "delegation-run/run/one".into(),
            lease_id: "lease/one".into(),
            daemon_epoch: "epoch".into(),
            permit_id: "run-config/run/one".into(),
            permit_generation: 1,
            configuration: DelegationRunConfigurationV1 {
                format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
                scope_id: "run-config/run/one".into(),
                generation: 1,
                canonical_workspace_path: "/workspace".into(),
                permission_policy: WorkerPermissionPolicyV1::ApproveAll,
            },
            execution: WorkerExecutionIntentV1 {
                root_identity: "root".into(),
                access: WorkspaceAccessV1::TrustedNative,
                tools: vec![WorkerToolV1::Read],
                network: WorkerNetworkV1::Allowed,
                duration_ms: 30_000,
                delegation_depth: 1,
            },
            deadline_ms: now + 30_000,
            lease_revoked: false,
            launch_nonce: "launch/one".into(),
            process: None,
            session: None,
            progress: Default::default(),
            stop_evidence: None,
            result_body: None,
            result_incomplete: false,
        };
        let task = DelegationTaskV1 {
            workspace_id: run.workspace_id.clone(),
            task_id: run.task_id.clone(),
            parent_task_ref: None,
            plan: DelegationPlanBindingV1 {
                authority_id: "test-authority".into(),
                plan_id: version.reference.plan_id.clone(),
                plan_revision: version.reference.content_revision,
                plan_digest: version.reference.content_digest.clone(),
                publication_revision: 1,
                publication_digest: CanonicalDigest::of_bytes(b"publication"),
                exact_reference: exact_reference(&version.reference),
                model_alias: version.compiled.model_alias().as_str().into(),
                harness,
                harness_configuration_digest: CanonicalDigest::of(
                    version.configuration.work.as_ref().unwrap(),
                )
                .unwrap(),
            },
            workspace: DelegationWorkspaceV1 {
                root_identity: "root".into(),
                volume_identity: "volume".into(),
                ancestry: vec!["root".into()],
            },
            created_at_ms: now,
            latest_run_id: run.run_id.clone(),
            latest_admission_sequence: 1,
            title: None,
            session: None,
            resume_until_ms: run.deadline_ms,
            required_body_ids: vec![],
            body_refs: vec![],
            native_history_paths: vec![],
        };
        let safety = Arc::new(
            RunSafetyProjection::new(Arc::new(SharedAdmissionGate::new()), "epoch".into()).unwrap(),
        );
        safety.finish_startup_recovery();
        let pair = RunCredentialPair::generate().unwrap();
        let verifier = Arc::new(
            RunCredentialVerifier::new(
                RunCredentialRecord {
                    task_id: run.task_id.clone(),
                    run_id: run.run_id.clone(),
                    lease_id: run.lease_id.clone(),
                    safety: RunSafetyBinding {
                        workspace: run.workspace_id.clone(),
                        daemon_epoch: run.daemon_epoch.clone(),
                        permit_id: run.permit_id.clone(),
                        permit_generation: run.permit_generation,
                        expires_at_ms: run.deadline_ms,
                    },
                    model_alias: task.plan.model_alias.clone(),
                    protocol: version.configuration.work.as_ref().unwrap().protocol,
                    fingerprints: pair.fingerprints(),
                },
                safety,
            )
            .unwrap(),
        );
        Self {
            version,
            task,
            run,
            pair,
            verifier,
        }
    }
}

#[test]
fn every_selected_harness_registers_the_exact_run_and_keeps_credential_boundaries() {
    for harness in [
        WorkerHarnessV1::CodexCli,
        WorkerHarnessV1::ClaudeCode,
        WorkerHarnessV1::QoderCli,
    ] {
        let fixture = Fixture::new(harness);
        let authority = DelegationRunAuthority::default();
        authority
            .register(
                &fixture.task,
                &fixture.run,
                fixture.version.clone(),
                fixture.verifier.clone(),
            )
            .unwrap();
        let bearer = format!(
            "Bearer {}",
            std::str::from_utf8(fixture.pair.model.expose()).unwrap()
        );
        let protocol = gateway_protocol(
            fixture
                .version
                .configuration
                .work
                .as_ref()
                .unwrap()
                .protocol,
        )
        .unwrap();
        let other_protocol = if protocol == IngressProtocol::Responses {
            IngressProtocol::Messages
        } else {
            IngressProtocol::Responses
        };
        let verified = authority.authenticate_run(&bearer, protocol).unwrap();
        assert_eq!(verified.locator().task_id(), fixture.task.task_id);
        assert_eq!(verified.locator().run_id(), fixture.run.run_id);
        assert_eq!(
            verified.locator().exact_plan_reference(),
            fixture.task.plan.exact_reference
        );
        assert!(authority.authenticate_run(&bearer, other_protocol).is_err());
        let query_bearer = format!(
            "Bearer {}",
            std::str::from_utf8(fixture.pair.self_query.expose()).unwrap()
        );
        assert!(authority.authenticate_run(&query_bearer, protocol).is_err());
        fixture.verifier.revoke();
        assert!(authority.authenticate_run(&bearer, protocol).is_err());
    }
}

#[test]
fn qoder_task_cannot_register_against_a_different_harness_version() {
    let mut fixture = Fixture::new(WorkerHarnessV1::CodexCli);
    fixture.task.plan.harness = WorkerHarnessV1::QoderCli;
    // Keep every digest, exact version and lease unchanged: only the harness is mismatched.
    assert_eq!(
        DelegationRunAuthority::default().register(
            &fixture.task,
            &fixture.run,
            fixture.version,
            fixture.verifier
        ),
        Err(DelegationErrorV1::Conflict)
    );
}
