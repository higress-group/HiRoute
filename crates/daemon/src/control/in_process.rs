//! The console alone receives this dispatcher. External sockets retain their release gate.
use super::*;
use hiroute_client_core::ControlDispatch;
pub(crate) struct InProcessControl {
    pub daemon: LocalControlDaemon,
    pub budgets: ControlConnectionBudgets,
}
impl ControlDispatch for InProcessControl {
    fn dispatch(&self, request: LocalControlWireRequestV2) -> MachineEnvelopeV2<serde_json::Value> {
        let reject =
            |code| MachineEnvelopeV2::failed(ErrorV1::new(code), Some(request.request_id.clone()));
        if request.schema_version != LOCAL_CONTROL_SCHEMA_V2
            || serde_json::to_vec(&request).map_or(true, |bytes| {
                bytes.len() + 1 > LOCAL_CONTROL_MAX_FRAME_BYTES
            })
        {
            return reject(ErrorCode::InvalidArguments);
        }
        let Some(_upgrade) = self.daemon.upgrade.enter(false) else {
            return reject(ErrorCode::DaemonUnavailable);
        };
        let Some(mut admission) = self.budgets.try_accept() else {
            return reject(ErrorCode::DaemonUnavailable);
        };
        let _wait = match admission.classify(&request.operation_id) {
            Ok(permit) => permit,
            Err(code) => return reject(code),
        };
        let mut daemon = self.daemon.clone();
        daemon.released_commands_only = false;
        daemon.diagnostic_client = Some(hiroute_diagnostics::event::ControlClientName::Console);
        daemon.dispatch_wire(request)
    }
}
