use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hiroute_application::delegation::tasks::{DelegationTaskPort, DelegationTasks};
use hiroute_application_api::{
    ClientHelloV1, DELEGATION_CANCEL_SCHEMA_V1, DELEGATION_WAIT_SCHEMA_V1,
    DelegationAcceptedTimeStateV1, DelegationCancelV1, DelegationRunScopeV1, DelegationRunViewV1,
    DelegationWaitV1, ErrorCode, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2,
    MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, MachineStatus, WorkerCancelRequestV1,
    WorkerExecutorPresentationBasisV1, WorkerExecutorPresentationV1, WorkerWaitRequestV1,
};
use hiroute_domain::delegation::{
    DelegationErrorV1, RunCleanupV1, RunStateV1, WorkerPermissionPolicyV1,
};
use serde_json::json;

use super::*;

const TEST_NAME: &str = "control::control_capacity_tests::worker_cancel_remains_available_while_real_wait_transport_is_saturated";
const RUN_ID: &str = "run/control-capacity";

#[derive(Clone, Debug)]
struct WorkerState {
    state: RunStateV1,
    revision: u64,
    wait_calls: usize,
    transitions: Vec<RunStateV1>,
}

struct BlockingWorkerTasks {
    state: Mutex<WorkerState>,
    changed: Condvar,
}

impl BlockingWorkerTasks {
    fn new() -> Self {
        Self {
            state: Mutex::new(WorkerState {
                state: RunStateV1::Running,
                revision: 3,
                wait_calls: 0,
                transitions: vec![RunStateV1::Running],
            }),
            changed: Condvar::new(),
        }
    }

    fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.wait_calls == 0 {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("WorkerWait did not enter the application port");
            let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            assert!(
                !timeout.timed_out(),
                "WorkerWait did not enter the application port"
            );
        }
    }

    fn snapshot(&self) -> WorkerState {
        self.state.lock().unwrap().clone()
    }

    fn view(state: &WorkerState) -> DelegationRunViewV1 {
        DelegationRunViewV1 {
            task_id: "task/control-capacity".into(),
            run_id: RUN_ID.into(),
            ordinal: 1,
            continued_from: None,
            admission_sequence: 1,
            accepted_at_ms: Some(1),
            accepted_time_state: DelegationAcceptedTimeStateV1::Recorded,
            executor: WorkerExecutorPresentationV1 {
                harness: None,
                display_name: None,
                basis: WorkerExecutorPresentationBasisV1::Unavailable,
            },
            state: state.state,
            state_revision: state.revision,
            cleanup: if state.state == RunStateV1::Cancelled {
                RunCleanupV1::Complete
            } else {
                RunCleanupV1::Pending
            },
            result_available: false,
            scope: DelegationRunScopeV1 {
                canonical_cwd: "/tmp/hiroute-control-capacity".into(),
                permission_policy: WorkerPermissionPolicyV1::ApproveAll,
                deadline_ms: 60_000,
            },
        }
    }
}

impl DelegationTaskPort for BlockingWorkerTasks {
    fn worker_wait(
        &self,
        request: &WorkerWaitRequestV1,
    ) -> Result<DelegationWaitV1, DelegationErrorV1> {
        assert_eq!(request.run_id, RUN_ID);
        let mut state = self.state.lock().unwrap();
        state.wait_calls += 1;
        self.changed.notify_all();
        while state.state == RunStateV1::Running {
            state = self.changed.wait(state).unwrap();
        }
        Ok(DelegationWaitV1 {
            schema: DELEGATION_WAIT_SCHEMA_V1.into(),
            run: Self::view(&state),
            changed: true,
            timed_out: false,
        })
    }

    fn worker_cancel(
        &self,
        request: &WorkerCancelRequestV1,
    ) -> Result<DelegationCancelV1, DelegationErrorV1> {
        assert_eq!(request.run_id, RUN_ID);
        let mut state = self.state.lock().unwrap();
        assert_eq!(state.state, RunStateV1::Running);
        state.state = RunStateV1::Cancelling;
        state.revision += 1;
        state.transitions.push(RunStateV1::Cancelling);
        state.state = RunStateV1::Cancelled;
        state.revision += 1;
        state.transitions.push(RunStateV1::Cancelled);
        self.changed.notify_all();
        Ok(DelegationCancelV1 {
            schema: DELEGATION_CANCEL_SCHEMA_V1.into(),
            operation_id: "operation/control-capacity-cancel".into(),
            run: Self::view(&state),
        })
    }
}

fn exchange_control_request(
    socket: &Path,
    request_id: &str,
    operation_id: &str,
    payload: serde_json::Value,
    response_timeout: Duration,
) -> MachineEnvelopeV2<serde_json::Value> {
    let mut stream = UnixStream::connect(socket).unwrap();
    write_frame(
        &mut stream,
        &ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "capacity-test".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    )
    .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    read_frame(&mut reader, Instant::now() + Duration::from_secs(1)).unwrap();
    write_frame(
        &mut stream,
        &LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.to_owned(),
            operation_id: operation_id.to_owned(),
            payload,
            protected_grant: None,
        },
    )
    .unwrap();
    let response = read_frame(&mut reader, Instant::now() + response_timeout).unwrap();
    serde_json::from_str(&response).unwrap()
}

#[test]
fn worker_cancel_remains_available_while_real_wait_transport_is_saturated() {
    if crate::test_support::isolated_agent_home(TEST_NAME) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        temp.path().join("storage"),
        crate::release_catalog::current_fixture_catalog(),
    )
    .unwrap();
    let tasks = Arc::new(BlockingWorkerTasks::new());
    let ports = runtime
        .application_ports()
        .with_delegation_tasks(Arc::new(DelegationTasks::new(tasks.clone())));
    let socket = temp.path().join("wait-capacity.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let budgets = ControlConnectionBudgets::new(4, 1, 2);
    let server_budgets = budgets.clone();
    let server = std::thread::spawn(move || {
        serve_listener_with_budgets(
            listener,
            LocalControlDaemon::new(ApplicationService::new(ports)),
            Some(3),
            server_budgets,
        )
        .unwrap();
    });

    let wait_socket = socket.clone();
    let active_wait = std::thread::spawn(move || {
        exchange_control_request(
            &wait_socket,
            "active-worker-wait",
            "WorkerWait",
            json!({
                "run_id": RUN_ID,
                "wait_timeout_secs": 30,
            }),
            Duration::from_secs(3),
        )
    });
    tasks.wait_until_entered();
    assert_eq!(budgets.waits.active(), 1);
    let before_excess = tasks.snapshot();

    let rejected = exchange_control_request(
        &socket,
        "excess-worker-wait",
        "WorkerWait",
        json!({
            "run_id": RUN_ID,
            "wait_timeout_secs": 30,
        }),
        Duration::from_secs(1),
    );
    assert_eq!(
        rejected.error.unwrap().code,
        ErrorCode::ControlWaitCapacityExceeded
    );
    let after_excess = tasks.snapshot();
    assert_eq!(after_excess.state, RunStateV1::Running);
    assert_eq!(after_excess.revision, before_excess.revision);
    assert_eq!(after_excess.wait_calls, 1);

    let cancelled = exchange_control_request(
        &socket,
        "worker-cancel",
        "WorkerCancel",
        json!({
            "run_id": RUN_ID,
            "idempotency_key": "cancel:control-capacity",
            "reason": "test-cancel",
        }),
        Duration::from_secs(1),
    );
    assert_eq!(cancelled.status, MachineStatus::Succeeded, "{cancelled:?}");
    assert_eq!(cancelled.data.unwrap()["run"]["state"], "cancelled");

    let waited = active_wait.join().unwrap();
    assert_eq!(waited.status, MachineStatus::Succeeded, "{waited:?}");
    assert_eq!(waited.data.unwrap()["run"]["state"], "cancelled");
    assert_eq!(
        tasks.snapshot().transitions,
        [
            RunStateV1::Running,
            RunStateV1::Cancelling,
            RunStateV1::Cancelled
        ]
    );

    server.join().unwrap();
    let drained = Instant::now() + Duration::from_secs(1);
    while budgets.active_total() != 0 && Instant::now() < drained {
        std::thread::yield_now();
    }
    assert_eq!(budgets.active_total(), 0);
    assert_eq!(budgets.waits.active(), 0);
    assert_eq!(budgets.unclassified.active(), 0);
}
