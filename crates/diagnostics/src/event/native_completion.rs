//! Loss of native completion proof; independent of optional content capture.
use super::{IngressProtocol, ResponseEventShape};
use crate::identity::CorrelationToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCompletionUnknown {
    pub request_token: Option<CorrelationToken>,
    pub attempt_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_token: Option<CorrelationToken>,
    pub upstream_protocol: IngressProtocol,
    pub evidence: NativeCompletionEvidence,
}

/// Fixed-size first-cause evidence. No IDs, content, hashes or dynamic field names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCompletionEvidence {
    pub reason: NativeCompletionReason,
    /// One-based visited SSE frame, including transport-only frames; absent for JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ResponseEventShape>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCompletionReason {
    EventTypeMissing,
    EventTypeMismatch,
    UnownedEvent,
    UntrackedToolItem,
    TerminalStatus,
    MissingOutput,
    MissingOutputItem,
    ItemIdentity,
    ItemSnapshot,
    AddedTextPrefix,
    InvalidDelta,
    DeltaPosition,
    DeltaIdentity,
    DeltaAfterDone,
    DuplicateDone,
    DeltaFinalType,
    DeltaFinalIdentity,
    DeltaFinalText,
    DoneFinalText,
    EvidenceBudget,
    PostTerminal,
    /// Protocol completion is unknown without finer first-cause instrumentation.
    Generic,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::DiagnosticEvent;

    #[test]
    fn native_completion_event_is_closed_and_preserves_existing_failure_bytes() {
        let legacy = r#"{"response_failure":{"request_token":null,"attempt_index":1,"stage":"decode","reason":"invalid_json"}}"#;
        let old: DiagnosticEvent = serde_json::from_str(legacy).unwrap();
        assert_eq!(serde_json::to_string(&old).unwrap(), legacy);
        let event = DiagnosticEvent::NativeCompletionUnknown(NativeCompletionUnknown {
            request_token: None,
            attempt_index: 1,
            attempt_token: None,
            upstream_protocol: IngressProtocol::OpenAiResponses,
            evidence: NativeCompletionEvidence {
                reason: NativeCompletionReason::ItemSnapshot,
                frame_index: Some(95),
                event: None,
            },
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serde_json::from_value::<DiagnosticEvent>(json.clone()).unwrap(),
            event
        );
        let mut extra = json.clone();
        extra["native_completion_unknown"]["evidence"]["raw"] = "SECRET".into();
        assert!(serde_json::from_value::<DiagnosticEvent>(extra).is_err());
        let mut dynamic = json;
        dynamic["native_completion_unknown"]["evidence"]["reason"] = "SECRET".into();
        assert!(serde_json::from_value::<DiagnosticEvent>(dynamic).is_err());
    }
}
