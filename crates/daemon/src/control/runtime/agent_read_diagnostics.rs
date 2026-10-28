//! Safe aggregate diagnostics for current Agent reads.
use super::LocalControlAdapter;
use hiroute_diagnostics::{agent_read::Timer, runtime::DiagnosticsPort};
use hiroute_local_storage::CapturedAgentLocator;

impl LocalControlAdapter {
    pub(super) fn agent_read_diagnostics(&self) -> DiagnosticsPort {
        self.publication_diagnostics
            .lock()
            .map(|port| port.clone())
            .unwrap_or_default()
    }
}

pub(super) fn count_locator(timer: &mut Timer, capture: &CapturedAgentLocator) {
    let sizes = capture.sizes();
    timer.counts.operations = sizes.operations;
    timer.counts.operation_bytes = sizes.operation_bytes;
    timer.counts.payloads = sizes.payloads;
    timer.counts.payload_bytes = sizes.payload_bytes;
}
