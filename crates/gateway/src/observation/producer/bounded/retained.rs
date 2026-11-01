//! One queue owner survives materialization, short prefixes and sink rejection.
use super::content::{DrainScratch, EncodeScratch, SpillOwner};
use super::*;
use crate::replay::{OverflowRange, ReplayError};

enum Payload {
    Resident(Arc<[u8]>),
    Spilled(OverflowRange),
}

pub(super) struct QueuedRecord {
    stamp: ObservationSequenceStamp,
    payload: Payload,
    accounted_bytes: usize,
    timing: Option<super::super::super::latency::AsyncRecordTiming>,
    _credit: RecordCredit,
    _spill_owner: Option<SpillOwner>,
    _workspace: Option<EncodeScratch>,
}

impl QueuedRecord {
    pub(super) fn resident(record: ObservationRecord, credit: RecordCredit) -> Self {
        Self {
            stamp: record.stamp,
            payload: Payload::Resident(record.payload),
            accounted_bytes: record.accounted_bytes,
            timing: record.timing,
            _credit: credit,
            _spill_owner: None,
            _workspace: None,
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
            payload: Payload::Spilled(range),
            accounted_bytes,
            timing,
            _credit: credit,
            _spill_owner: Some(owner),
            _workspace: None,
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
            Payload::Spilled(range) => {
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
        self._workspace = Some(workspace);
        self
    }
}
