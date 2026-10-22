//! Native task identity never proves byte-prefix affinity or a fresh user.
use super::{ContextHoldKey, HistoryContinuity, PreparedHold};
use crate::context_hold::routing::{NativeCheckpoint, NativeUserOrigin};

impl PreparedHold {
    pub(crate) fn history_is_proven(&self) -> bool {
        if self.native.user_origin == NativeUserOrigin::Observed {
            return false;
        }
        let continuity = self.history_continuity();
        if self
            .native
            .generation
            .zip(self.snapshot.and_then(|s| s.native.generation))
            .is_some_and(|(current, captured)| current < captured)
        {
            return false;
        }
        if self.native.user_origin == NativeUserOrigin::ExistingTurn
            && !continuity.uses_saved_user()
        {
            return false;
        }
        if self.native.user_origin == NativeUserOrigin::NativeTurn
            && let Some(snapshot) = self.snapshot
            && !continuity.uses_saved_user()
            && (continuity != HistoryContinuity::NativeNewTurn
                || self.measured.semantic_prefix_digest != Some(snapshot.semantic_history_digest)
                || self
                    .measured
                    .latest_user_index
                    .is_none_or(|index| index < snapshot.message_count))
        {
            // A changed opaque turn ID alone cannot recover unknown history or
            // accept a replay. Require an appended user after the captured prefix.
            return false;
        }
        if self.native.user_origin == NativeUserOrigin::NativeTurn
            && self.snapshot.is_none()
            && (self.native.compacted || self.measured.contains_model_history)
        {
            return false;
        }
        self.snapshot.is_none()
            || !self.native.requires_checkpoint
            || self.history_continuity().semantic_continuity()
    }

    pub(crate) fn native_user_unavailable(&self) -> bool {
        self.native.user_origin == NativeUserOrigin::ExistingTurn
            || self.history_continuity().uses_saved_user()
    }

    pub(crate) fn history_can_capture(&self) -> bool {
        self.history_is_proven()
            || matches!(
                self.native.user_origin,
                NativeUserOrigin::Wire | NativeUserOrigin::Observed | NativeUserOrigin::NativeTurn
            )
    }

    pub(crate) fn user_origin_unverified(&self) -> bool {
        // A native turn/window identifies an activation, not who supplied its
        // input. Cache presence must not promote or demote that source claim.
        matches!(
            self.native.user_origin,
            NativeUserOrigin::Wire | NativeUserOrigin::Observed | NativeUserOrigin::NativeTurn
        ) || !self.history_is_proven()
    }

    pub(crate) fn uncaptured_native_prefix(&self) -> bool {
        self.snapshot.is_none()
            && (self.native.compacted
                || self.native.generation.is_some_and(|generation| generation > 0)
                // Hermes's lineage probe also stamps a fresh root session. It
                // is continuity metadata, not by itself a compacted prefix.
                || self.native.requires_checkpoint && self.native.checkpoint.is_some())
    }

    #[cfg(test)]
    pub(crate) fn native_turn(&self) -> Option<[u8; 32]> {
        self.native.turn
    }

    pub(crate) fn set_native(&mut self, native: NativeCheckpoint) {
        self.native = native;
    }

    pub(crate) fn set_history_key(&mut self, key: ContextHoldKey) {
        self.history_key = Some(key);
    }

    #[cfg(test)]
    pub(crate) fn scope_id(&self) -> String {
        self.history_key.as_ref().unwrap_or(&self.key).scope_id()
    }

    pub(crate) fn history_continuity(&self) -> HistoryContinuity {
        let Some(snapshot) = self.snapshot else {
            return HistoryContinuity::NoCheckpoint;
        };
        let same_turn = self.native.turn.is_some() && self.native.turn == snapshot.native.turn;
        let prefix_continues = self.measured.message_count >= snapshot.message_count
            && self.measured.semantic_prefix_digest == Some(snapshot.semantic_history_digest);
        // A compacted continuation can append a synthetic User to an unchanged
        // prefix. The native task wins over that envelope's apparent new user.
        if prefix_continues {
            return if same_turn {
                HistoryContinuity::NativeSameTurn
            } else if self.native.turn.is_some() && snapshot.native.turn.is_some() {
                HistoryContinuity::NativeNewTurn
            } else {
                HistoryContinuity::Continuing
            };
        }
        let checkpoint_changed = self.native.compacted
            || self.native.checkpoint.is_some()
                && self.native.checkpoint != snapshot.native.checkpoint;
        if same_turn && (!self.native.requires_checkpoint || checkpoint_changed) {
            return HistoryContinuity::NativeContinuation;
        }
        if self.native.turn.is_some() && snapshot.native.turn.is_some() && checkpoint_changed {
            return HistoryContinuity::NativeNewTurn;
        }
        if self.measured.message_count < snapshot.message_count {
            HistoryContinuity::Shortened
        } else {
            HistoryContinuity::Rewritten
        }
    }
}
