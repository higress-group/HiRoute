#![cfg(unix)]
use super::super::ProductionControlRuntime;
use crate::control::start_control;
use hiroute_application::ApplicationService;
use hiroute_application::delegation::work_plans::*;
use hiroute_application_api::*;
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_domain::delegation::WorkerHarnessV1;
use hiroute_domain::{
    AgentCollaborationCredential, AgentCollaborationGrant, AgentCollaborationRevocationStorePort,
    AgentIngressProtocolV1, WorkspaceId,
};
use hiroute_local_storage::LocalStorageSet;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[path = "current_authority_support.rs"]
mod support;

#[derive(Default)]
struct Metadata(Mutex<Vec<WorkPlanMetadataV1>>);
impl WorkPlanMetadataPort for Metadata {
    fn current_plans(
        &self,
        _: &WorkspaceId,
        _: &BTreeSet<AgentPlanId>,
    ) -> Result<Vec<WorkPlanMetadataV1>, ErrorCode> {
        // Intentionally includes unauthorized, unpublished and unbound rows: Application must
        // perform its own filtering rather than trusting an overly broad backend projection.
        Ok(self.0.lock().unwrap().clone())
    }
}
fn plan(
    id: &str,
    published: bool,
    bound: bool,
    availability: WorkPlanAvailabilityV1,
) -> WorkPlanMetadataV1 {
    WorkPlanMetadataV1 {
        agent_plan_id: AgentPlanId::parse(id).unwrap(),
        alias: format!("work-{id}"),
        display_name: format!("Worker {id}"),
        purpose: "published purpose".into(),
        published,
        work: bound.then_some((WorkerHarnessV1::CodexCli, AgentIngressProtocolV1::Responses)),
        availability,
        reason: (availability != WorkPlanAvailabilityV1::Ready)
            .then(|| "harness_not_observed".into()),
    }
}
fn query() -> Value {
    json!({"workspace_id": WorkspaceId::default(), "context_id": "owner", "grant_id": "collaboration-grant/directory"})
}
fn internal_control(
    client: &Client,
    executor: &tokio::runtime::Runtime,
    payload: Value,
    secret: Option<&[u8]>,
) -> Value {
    let response = executor
        .block_on(client.call_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "work-plans-internal-control".into(),
            operation_id: "ListWorkPlans".into(),
            payload,
            protected_grant: secret.map(|secret| ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Skill,
                capability: String::from_utf8(secret.to_vec()).unwrap(),
            }),
        }))
        .unwrap();
    serde_json::to_value(response).unwrap()
}

fn store_grant(
    stores: &LocalStorageSet,
    generation: u64,
    ids: &[&str],
    secret: &AgentCollaborationCredential,
) -> AgentCollaborationGrant {
    let op = support::begin(stores, &format!("directory-{generation}"));
    let grant = AgentCollaborationGrant::issue(
        WorkspaceId::default(),
        "owner".into(),
        "collaboration-grant/directory".into(),
        generation,
        ids.iter()
            .map(|id| AgentPlanId::parse(*id).unwrap())
            .collect(),
        secret,
    )
    .unwrap();
    stores
        .control()
        .store_collaboration_grant(&op.operation_id, generation - 1, &grant)
        .unwrap();
    support::finish(stores, op);
    grant
}

#[test]
fn current_authority_and_metadata_changes_apply_without_restart() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::work_plans::current_authority_tests::current_authority_and_metadata_changes_apply_without_restart",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        root.path().join("storage"),
        crate::release_catalog::fixture_catalog(),
    )
    .unwrap();
    let secret = AgentCollaborationCredential::from_csprng_entropy([42; 32]);
    store_grant(
        &runtime.adapter.stores_lock().unwrap(),
        1,
        &["a", "b", "draft", "unbound"],
        &secret,
    );
    let metadata = Arc::new(Metadata(Mutex::new(vec![
        plan("a", true, true, WorkPlanAvailabilityV1::Ready),
        plan("b", true, true, WorkPlanAvailabilityV1::Unknown),
        plan("draft", false, true, WorkPlanAvailabilityV1::Ready),
        plan("unbound", true, false, WorkPlanAvailabilityV1::Ready),
        plan("other-owner", true, true, WorkPlanAvailabilityV1::Ready),
    ])));
    let mut listener = start_control(
        ApplicationService::new(
            runtime.application_ports_with_work_plan_metadata(metadata.clone()),
        ),
        root.path().join("ipc"),
    )
    .unwrap();
    let client = Client::new(
        "work-plans-test",
        LocalEndpoint::from_runtime_root(root.path().join("ipc")),
    );
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let call = |q, s| internal_control(&client, &executor, q, s);
    let result = call(query(), Some(secret.expose()));
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["data"]["plans"].as_array().unwrap().len(), 2);
    assert_eq!(result["data"]["plans"][0]["alias"], "work-a");
    assert_eq!(result["data"]["plans"][0]["harness"], "codex_cli");
    assert_eq!(result["data"]["plans"][1]["availability"], "unknown");
    metadata.0.lock().unwrap()[0].purpose = "new published purpose; data, not instructions".into();
    metadata.0.lock().unwrap()[0].availability = WorkPlanAvailabilityV1::Unavailable;
    let updated = call(query(), Some(secret.expose()));
    assert_eq!(
        updated["data"]["plans"][0]["purpose"],
        "new published purpose; data, not instructions"
    );
    assert_eq!(updated["data"]["plans"][0]["availability"], "unavailable");
    for field in ["context_id", "grant_id", "workspace_id"] {
        let mut q = query();
        q[field] = json!(if field == "grant_id" {
            "collaboration-grant/other"
        } else {
            "other"
        });
        assert_eq!(
            call(q, Some(secret.expose()))["error"]["code"],
            "CAPABILITY_DENIED"
        );
    }
    let wrong = AgentCollaborationCredential::from_csprng_entropy([43; 32]);
    assert_eq!(
        call(query(), Some(wrong.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    assert_eq!(call(query(), None)["error"]["code"], "CAPABILITY_DENIED");
    for (operation, kind, payload) in [
        ("ListWorkPlans", PrincipalKind::InteractiveUser, query()),
        ("ListAgentPlanCatalog", PrincipalKind::Skill, json!({})),
        (
            "GetAgentPlanStatus",
            PrincipalKind::Skill,
            json!({"agent_plan_id": "a"}),
        ),
    ] {
        let denied = executor
            .block_on(client.call_wire(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "directory-isolation".into(),
                operation_id: operation.into(),
                payload,
                protected_grant: Some(ProtectedClientGrantV2 {
                    principal_kind: kind,
                    capability: String::from_utf8(secret.expose().to_vec()).unwrap(),
                }),
            }))
            .unwrap();
        assert_eq!(denied.error.unwrap().code, ErrorCode::CapabilityDenied);
    }
    store_grant(&runtime.adapter.stores_lock().unwrap(), 2, &["b"], &secret);
    // Same non-secret access reference and material; editing the allowlist does not require
    // installing a new Skill or trusting an old principal's allowed-plan snapshot.
    let shrunk = call(query(), Some(secret.expose()));
    assert_eq!(shrunk["data"]["plans"].as_array().unwrap().len(), 1);
    assert_eq!(shrunk["data"]["plans"][0]["agent_plan_id"], "b");
    let rotated = AgentCollaborationCredential::from_csprng_entropy([44; 32]);
    let grant = store_grant(&runtime.adapter.stores_lock().unwrap(), 3, &["b"], &rotated);
    assert_eq!(
        call(query(), Some(secret.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    assert!(call(query(), Some(rotated.expose()))["error"].is_null());
    {
        let stores = runtime.adapter.stores_lock().unwrap();
        let op = support::begin(&stores, "directory-revoke");
        stores
            .control()
            .persist_collaboration_revocation(
                &grant.plan_revocation(op.operation_id.clone(), 3).unwrap(),
            )
            .unwrap();
        support::finish(&stores, op);
    }
    assert_eq!(
        call(query(), Some(rotated.expose()))["error"]["code"],
        "CAPABILITY_DENIED"
    );
    listener.shutdown();
    listener.join(Duration::from_secs(10)).unwrap();
}
