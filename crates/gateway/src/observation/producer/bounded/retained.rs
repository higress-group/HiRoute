//! One queue owner survives materialization, short prefixes and sink rejection.
use super::content::{DrainScratch, EncodeScratch, SpillOwner};
use super::*;
use crate::replay::{OverflowRange, ReplayError};

enum Payload {
    Resident(Arc<[u8]>),
    // Ordinary records do not pay for Content-only spill/workspace metadata.
    WithWorkspace(Box<ResidentWorkspace>),
    Spilled(Box<RetainedSpill>),
}

struct ResidentWorkspace {
    bytes: Arc<[u8]>,
    _workspace: EncodeScratch,
}

struct RetainedSpill {
    // Reclaim backing before returning its descriptor credit.
    range: OverflowRange,
    _owner: SpillOwner,
}

pub(super) struct QueuedRecord {
    stamp: ObservationSequenceStamp,
    payload: Payload,
    accounted_bytes: usize,
    timing: Option<super::super::super::latency::AsyncRecordTiming>,
    _credit: RecordCredit,
}

impl QueuedRecord {
    pub(super) fn resident(record: ObservationRecord, credit: RecordCredit) -> Self {
        Self {
            stamp: record.stamp,
            payload: Payload::Resident(record.payload),
            accounted_bytes: record.accounted_bytes,
            timing: record.timing,
            _credit: credit,
        }
    }

    pub(super) fn spilled(
        stamp: ObservationSequenceStamp,
        range: OverflowRange,
        accounted_bytes: usize,
        timing: Option<super::super::super::latency::AsyncRecordTiming>,
        credit: RecordCredit,
        owner: SpillOwner,
    ) -> Self {
        Self {
            stamp,
            payload: Payload::Spilled(Box::new(RetainedSpill {
                range,
                _owner: owner,
            })),
            accounted_bytes,
            timing,
            _credit: credit,
        }
    }

    pub(super) fn sequence(&self) -> u64 {
        self.stamp.sequence
    }

    pub(super) fn accounted_bytes(&self) -> usize {
        self.accounted_bytes
    }

    pub(super) fn materialize(
        &self,
    ) -> Result<(ObservationRecord, Option<DrainScratch>), ReplayError> {
        // The sink owns an independent drain workspace, not another queue
        // admission. Keep original stamp/timing on every short-prefix retry.
        let (payload, scratch) = match &self.payload {
            Payload::Resident(bytes) => (Arc::clone(bytes), None),
            Payload::WithWorkspace(resident) => (Arc::clone(&resident.bytes), None),
            Payload::Spilled(spilled) => {
                let range = &spilled.range;
                // Includes the temporary Vec -> Arc overlap. Legal large
                // single records keep the original one-record exception.
                let mut scratch = self
                    ._credit
                    .shared
                    .content
                    .as_ref()
                    .map(|resources| resources.drain(range.len().saturating_mul(2)));
                let payload: Arc<[u8]> = range.read()?.into();
                if let Some(scratch) = &mut scratch {
                    scratch.shrink_to(payload.len());
                }
                (payload, scratch)
            }
        };
        Ok((
            ObservationRecord {
                stamp: self.stamp.clone(),
                payload,
                accounted_bytes: self.accounted_bytes,
                gap_heartbeat: false,
                timing: self.timing.clone(),
            },
            scratch,
        ))
    }

    pub(super) fn hold_workspace(mut self, workspace: EncodeScratch) -> Self {
        self.payload = match self.payload {
            Payload::Resident(bytes) => Payload::WithWorkspace(Box::new(ResidentWorkspace {
                bytes,
                _workspace: workspace,
            })),
            _ => unreachable!("only a resident record borrows encoder workspace"),
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_record_layout_does_not_include_content_spill_state() {
        assert!(
            std::mem::size_of::<QueuedRecord>()
                <= std::mem::size_of::<ObservationRecord>()
                    + std::mem::size_of::<RecordCredit>()
                    + 2 * std::mem::size_of::<usize>()
        );
        assert!(
            std::mem::size_of::<QueuedRecord>()
                + std::mem::size_of::<RetainedSpill>()
                + 4 * std::mem::size_of::<usize>()
                <= super::super::content::CONTENT_RECORD_BYTES
        );
    }
}
