use super::*;
use hiroute_diagnostics::event::{NativeRoutingEvidence, NativeRoutingHeader, NativeRoutingOrigin};

#[cfg(test)]
#[path = "diagnostic_tests.rs"]
mod tests;

impl ContextIdentityFacts {
    /// No raw header, ID, proof text or credential can enter the closed event.
    /// A present header with no adapter verdict is explicitly not proof of loading.
    pub(crate) fn routing_evidence(&self, headers: &HeaderMap) -> NativeRoutingEvidence {
        use super::super::routing::NativeUserOrigin;
        let header = match super::single_header(headers, "x-hiroute-routing-context", 4096) {
            Field::Absent => NativeRoutingHeader::Missing,
            Field::Invalid => NativeRoutingHeader::Invalid,
            Field::Value(_) => NativeRoutingHeader::Present,
        };
        let native = self.routing.native;
        NativeRoutingEvidence {
            request_token: None,
            context_token: None,
            header,
            adapter: self.routing.adapter,
            track_history: self.routing.track,
            origin: match native.user_origin {
                NativeUserOrigin::Wire => NativeRoutingOrigin::Wire,
                NativeUserOrigin::NativeTurn => NativeRoutingOrigin::NativeTurn,
                NativeUserOrigin::CurrentUser => NativeRoutingOrigin::CurrentUser,
                NativeUserOrigin::ExistingTurn => NativeRoutingOrigin::ExistingTurn,
                NativeUserOrigin::Observed => NativeRoutingOrigin::Observed,
            },
            turn_applied: native.turn.is_some(),
            checkpoint_applied: native.checkpoint.is_some(),
            compression_applied: native.compacted,
            identity_conflict: self.routing.identity_conflict,
            projection: hiroute_diagnostics::event::NativeInputProjection::Unobserved,
        }
    }
}
