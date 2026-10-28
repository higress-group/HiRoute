//! Test-only failure at the terminal journal write, after preceding effects
//! have run inside the same transaction and before they can commit.
use super::*;
use std::cell::Cell;

thread_local! {
    static TERMINAL: Cell<Option<OperationState>> = const { Cell::new(None) };
}

pub(super) struct TerminalFault(Option<OperationState>);

pub(super) fn inject(state: OperationState) -> TerminalFault {
    TerminalFault(TERMINAL.with(|slot| slot.replace(Some(state))))
}

impl Drop for TerminalFault {
    fn drop(&mut self) {
        TERMINAL.with(|slot| slot.set(self.0));
    }
}

pub(super) fn check(operation: &OperationV1) -> PortResult<()> {
    if TERMINAL.with(|slot| slot.get() == Some(operation.state)) {
        return Err(port(
            PortErrorCode::Unavailable,
            "control.test.terminal-write",
        ));
    }
    Ok(())
}
