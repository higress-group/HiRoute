use std::cell::Cell;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    LoseApplyResponseAndDisconnect,
    LoseBeforeApply,
}
thread_local! { pub static FAULT: Cell<Fault> = const { Cell::new(Fault::None) }; }
pub fn take(fault: Fault) -> bool {
    FAULT.with(|current| {
        if current.get() == fault {
            current.set(Fault::None);
            true
        } else {
            false
        }
    })
}
