//! Byte, event-slot and active-source ownership release atomically.
use super::*;

/// Retained input and typed capture work share the content channel's existing
/// byte ceiling with encoded records. This owner allocates no sequence number.
pub struct ObservationWorkReservation {
    pub(super) shared: Arc<ProducerShared>,
    pub(super) bytes: usize,
    class: WorkClass,
}

#[derive(Clone, Copy)]
enum WorkClass {
    Queued,
    Active,
    Control,
}

/// Reserved before assigning a sequence to background encoding. Until transfer,
/// early return or unwinding returns the unused record credit.
pub(super) struct RecordCredit {
    pub(super) shared: Arc<ProducerShared>,
    pub(super) bytes: usize,
    pub(super) content_slot: Option<ContentSlotClass>,
}

impl Drop for RecordCredit {
    fn drop(&mut self) {
        let _gate = lock(&self.shared.credit_gate);
        let previous = self
            .shared
            .queued_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes);
        if let Some(class) = self.content_slot.take() {
            let content = self.shared.content.as_ref().expect("content admission");
            match class {
                ContentSlotClass::Data => {
                    content.slots.fetch_sub(1, Ordering::AcqRel);
                }
                ContentSlotClass::Control => {
                    content.slots.fetch_sub(1, Ordering::AcqRel);
                    content
                        .control_bytes
                        .fetch_sub(self.bytes, Ordering::AcqRel);
                }
                ContentSlotClass::Progress => {
                    content.progress_busy.store(false, Ordering::Release);
                }
            }
        }
        self.shared.credit_available.notify_all();
        self.shared.credit_changed.notify_waiters();
    }
}

impl RecordCredit {
    pub(super) fn shrink_to(&mut self, bytes: usize) {
        let _gate = lock(&self.shared.credit_gate);
        let released = self.bytes - bytes;
        self.shared
            .queued_bytes
            .fetch_sub(released, Ordering::AcqRel);
        if matches!(self.content_slot, Some(ContentSlotClass::Control)) {
            self.shared
                .content
                .as_ref()
                .unwrap()
                .control_bytes
                .fetch_sub(released, Ordering::AcqRel);
        }
        self.bytes = bytes;
        self.shared.credit_available.notify_all();
        self.shared.credit_changed.notify_waiters();
    }
}

impl Drop for ObservationWorkReservation {
    fn drop(&mut self) {
        self.shrink_to(0);
    }
}

impl ObservationWorkReservation {
    pub(super) fn queued(shared: Arc<ProducerShared>, bytes: usize) -> Self {
        Self {
            shared,
            bytes,
            class: WorkClass::Queued,
        }
    }

    pub(super) fn control(shared: Arc<ProducerShared>, bytes: usize) -> Self {
        Self {
            shared,
            bytes,
            class: WorkClass::Control,
        }
    }

    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        let retained = bytes.min(self.bytes);
        let released = self.bytes - retained;
        let _gate = lock(&self.shared.credit_gate);
        match self.class {
            WorkClass::Queued => {
                self.shared.work_bytes.fetch_sub(released, Ordering::AcqRel);
                self.shared
                    .queued_bytes
                    .fetch_sub(released, Ordering::AcqRel);
            }
            WorkClass::Active => {
                self.shared
                    .content
                    .as_ref()
                    .unwrap()
                    .active_source_bytes
                    .fetch_sub(released, Ordering::AcqRel);
            }
            WorkClass::Control => {
                self.shared
                    .content
                    .as_ref()
                    .unwrap()
                    .control_bytes
                    .fetch_sub(released, Ordering::AcqRel);
                self.shared
                    .queued_bytes
                    .fetch_sub(released, Ordering::AcqRel);
            }
        }
        self.bytes = retained;
        self.shared.credit_available.notify_all();
        self.shared.credit_changed.notify_waiters();
    }

    pub(crate) fn activate_with(&mut self, workspace: usize) -> bool {
        if matches!(self.class, WorkClass::Active) {
            return true;
        }
        if !matches!(self.class, WorkClass::Queued) {
            return false;
        }
        let Some(content) = &self.shared.content else {
            return false;
        };
        let _gate = lock(&self.shared.credit_gate);
        let active = content.active_source_bytes.load(Ordering::Relaxed);
        // An already admitted single larger source must remain serviceable.
        // Its measured bytes are reported as an oversized active unit.
        if active != 0
            && active.saturating_add(self.bytes).saturating_add(workspace)
                > content.limits.active_source
        {
            return false;
        }
        content
            .active_source_bytes
            .fetch_add(self.bytes.saturating_add(workspace), Ordering::AcqRel);
        self.shared
            .work_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
        self.shared
            .queued_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
        self.bytes = self.bytes.saturating_add(workspace);
        self.class = WorkClass::Active;
        self.shared.credit_available.notify_all();
        self.shared.credit_changed.notify_waiters();
        true
    }
}
