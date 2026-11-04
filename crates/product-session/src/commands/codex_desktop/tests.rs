use super::*;
use hiroute_client_core::{Client, ControlDispatch};
use serde_json::json;
use std::sync::{
    Mutex as SyncMutex, OnceLock, Weak,
    atomic::{AtomicBool, AtomicU64, AtomicUsize},
};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

struct Authority(Arc<AtomicBool>);
impl HostAuthority for Authority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn native_connection(&self) -> Option<crate::NativeConnectionStatus> {
        Some(if self.0.load(Ordering::SeqCst) {
            crate::NativeConnectionStatus::Ready
        } else {
            crate::NativeConnectionStatus::RecoveryRequired
        })
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        _: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        Err("UNEXPECTED_MUTATION".into())
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    None,
    Epoch,
    FinalEpoch,
    Replace,
    Native,
    Revision,
    Recovery,
    NotReady,
    Mutation,
    ClientRevoked,
}
struct Dispatch {
    change: Change,
    sessions: Weak<Mutex<Option<Session>>>,
    epoch: Arc<AtomicU64>,
    native: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    statuses: AtomicUsize,
    calls: SyncMutex<Vec<String>>,
}
impl ControlDispatch for Dispatch {
    fn dispatch(&self, request: LocalControlWireRequestV2) -> MachineEnvelopeV2<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(request.operation_id.clone());
        let value = match request.operation_id.as_str() {
            "GetClientServiceStatus" => {
                let n = self.statuses.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    assert!(
                        self.sessions.upgrade().unwrap().try_lock().is_err(),
                        "final service read must fence same-Session writes"
                    );
                    if self.change == Change::FinalEpoch {
                        self.epoch.fetch_add(1, Ordering::SeqCst);
                    }
                    if self.change == Change::ClientRevoked {
                        self.active.store(false, Ordering::SeqCst);
                    }
                }
                let ready =
                    self.change != Change::NotReady && !(self.change == Change::Recovery && n > 0);
                json!({"schema":"hiroute.client-service-status/v1","daemon_role":"all",
                    "recovery_ready":ready,"mutation_available":!(self.change == Change::Mutation && n > 0),
                    "gateway":"ready","active_publication":null,"revisions":{"target":if self.change == Change::Revision && n>0 {2} else {1},"dependencies":{}}})
            }
            "ScanAgents" => {
                assert_eq!(request.payload["context_id"], "context/desktop");
                let sessions = self.sessions.upgrade().unwrap();
                let mut guard = sessions
                    .try_lock()
                    .expect("native scan must not hold the mutable Session");
                match self.change {
                    Change::Epoch => {
                        self.epoch.fetch_add(1, Ordering::SeqCst);
                    }
                    Change::Native => self.native.store(false, Ordering::SeqCst),
                    Change::Replace => {
                        let client = guard.as_ref().unwrap().client.clone();
                        *guard = Some(Session::new(
                            client,
                            Arc::new(Authority(self.native.clone())),
                        ));
                    }
                    _ => {}
                }
                json!({"agents":[{"agent_id":"agent_codex_desktop","context_id":"context/desktop",
                    "configuration_state":"registered","version":"fixture","available_surfaces":["codex_desktop"],
                    "codex_desktop":{"codex_home":"/instance/codex","user_data":"/instance/desktop","source_codex_home":"/original/.codex","shared_skills":"/original/.agents/skills"}}]})
            }
            "GetAgentConnectionStatus" => {
                assert_eq!(request.payload["context_id"], "context/desktop");
                assert_eq!(request.payload["projection"], "model");
                assert!(request.payload.get("include_skills").is_none());
                json!({"schema":"hiroute.agent-model-settings-status/v2","context_id":"context/desktop",
                    "state":"configured","operation_id":"operation/desktop","operation_state":"succeeded",
                    "restore_point_ref":"restore/desktop","applied_revision":1,"surface_results":[],"model_verified":false,
                    "current_selection":{"mode":"codex_default","native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":["plan/one"],"default_selection":{"kind":"plan","plan_id":"plan/one"}}})
            }
            other => panic!("Native launch must not depend on a Plan catalog or Skills: {other}"),
        };
        MachineEnvelopeV2::succeeded(value, Some(request.request_id))
    }
}

async fn exercise(change: Change) {
    let sessions = Arc::new(Mutex::new(None));
    let epoch = Arc::new(AtomicU64::new(0));
    let native = Arc::new(AtomicBool::new(true));
    let active = Arc::new(AtomicBool::new(true));
    let dispatch = Arc::new(Dispatch {
        change,
        sessions: Arc::downgrade(&sessions),
        epoch: epoch.clone(),
        native: native.clone(),
        active: active.clone(),
        statuses: AtomicUsize::new(0),
        calls: SyncMutex::new(vec![]),
    });
    let client = Client::in_process("native-desktop-test", dispatch.clone());
    *sessions.lock().await = Some(Session::new(client, Arc::new(Authority(native))));
    let error = OnceLock::new();
    let diagnostics = DiagnosticHandle::noop();
    let state = CommandState(&sessions, &error, &epoch, &diagnostics);
    let broker = crate::web_confirmation::ConfirmationBroker::default();
    let is_active = || active.load(Ordering::SeqCst);
    let window = ClientContext {
        id: "main",
        confirmations: &broker,
        active: &is_active,
        visible: &|| true,
        notify: None,
    };
    let result = native_codex_desktop_launch_target(&window, &state, "context/desktop").await;
    assert_eq!(result.is_ok(), change == Change::None, "{change:?}");
    if change == Change::None {
        assert_eq!(result.unwrap().operation, "operation/desktop");
        assert_eq!(
            *dispatch.calls.lock().unwrap(),
            [
                "GetClientServiceStatus",
                "ScanAgents",
                "GetAgentConnectionStatus",
                "GetClientServiceStatus"
            ]
        );
    }
}

#[tokio::test]
async fn codex_desktop_native_host_launch_reads_no_plan_catalog_or_skills() {
    exercise(Change::None).await;
}

#[tokio::test]
async fn codex_desktop_native_host_launch_rejects_session_window_authority_and_service_changes() {
    for change in [
        Change::Epoch,
        Change::FinalEpoch,
        Change::Replace,
        Change::Native,
        Change::Revision,
        Change::Recovery,
        Change::NotReady,
        Change::Mutation,
        Change::ClientRevoked,
    ] {
        exercise(change).await;
    }
}
