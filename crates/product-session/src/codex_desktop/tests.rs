use super::*;
use hiroute_application_api::{LocalControlWireRequestV2, MachineEnvelopeV2};
use hiroute_client_core::ControlDispatch;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn agent() -> Value {
    json!({"agent_id":"agent_codex_desktop", "context_id":"context/desktop", "version":"fixture", "configuration_state":"registered", "available_surfaces":["codex_desktop"],
        "codex_desktop":{"codex_home":"/instance/codex","user_data":"/instance/desktop","source_codex_home":"/original/.codex","shared_skills":"/original/.agents/skills"}})
}
fn model() -> Value {
    json!({"schema":"hiroute.agent-model-settings-status/v2", "context_id":"context/desktop", "state":"configured", "operation_id":"operation/one", "operation_state":"succeeded", "restore_point_ref":"restore/one", "applied_revision":1,"surface_results":[],"model_verified":false,
        "current_selection":{"mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":["plan/one"],"default_selection":{"kind":"plan","plan_id":"plan/one"}}})
}
struct Dispatch {
    stale: bool,
    statuses: AtomicUsize,
}
impl ControlDispatch for Dispatch {
    fn dispatch(&self, request: LocalControlWireRequestV2) -> MachineEnvelopeV2<Value> {
        let value = match request.operation_id.as_str() {
            "GetClientServiceStatus" => {
                let n = self.statuses.fetch_add(1, Ordering::SeqCst);
                json!({"schema":"hiroute.client-service-status/v1","daemon_role":"all","recovery_ready":true,"mutation_available":true,"gateway":"ready","active_publication":null,"revisions":{"target":if self.stale && n>0 {2} else {1},"dependencies":{}}})
            }
            "ScanAgents" => {
                assert_eq!(request.payload["context_id"], "context/desktop");
                json!({"agents":[agent()]})
            }
            "GetAgentConnectionStatus" => {
                assert_eq!(request.payload["projection"], "model");
                assert!(request.payload.get("include_skills").is_none());
                model()
            }
            other => panic!("Unrelated launch dependency: {other}"),
        };
        MachineEnvelopeV2::succeeded(value, Some(request.request_id))
    }
}
#[tokio::test]
async fn codex_desktop_launch_reads_only_current_exact_model_authority() {
    for stale in [false, true] {
        let client = Client::in_process(
            "desktop-test",
            Arc::new(Dispatch {
                stale,
                statuses: AtomicUsize::new(0),
            }),
        );
        assert_eq!(
            read_launch_target(&client, "context/desktop").await.is_ok(),
            !stale
        );
    }
    let mut value = agent();
    value["settings"] = model();
    let entry = || serde_json::from_value(value.clone()).unwrap();
    assert!(launch_target(vec![entry()], "context/desktop", true).is_ok());
    assert!(launch_target(vec![entry()], "context/other", true).is_err());
    assert!(launch_target(vec![entry()], "context/desktop", false).is_err());
    for state in ["pending", "not_configured", "drift", "needs_attention"] {
        let mut changed = value.clone();
        changed["settings"]["state"] = state.into();
        assert!(
            launch_target(
                vec![serde_json::from_value(changed).unwrap()],
                "context/desktop",
                true
            )
            .is_err()
        );
    }
}
