use super::*;
use hiroute_diagnostics::{
    DiagnosticRecordV1,
    event::ProcessRole,
    level::DiagnosticLevel,
    record::Component,
    runtime::{DiagnosticRuntime, RuntimeConfig},
};
use std::sync::{OnceLock, atomic::AtomicU64};
use tokio::sync::Mutex;

#[tokio::test]
async fn final_read_rejections_are_recorded_without_changing_the_returned_failure() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("diagnostics");
    let diagnostics = DiagnosticRuntime::start(RuntimeConfig {
        root: root.clone(),
        role: ProcessRole::Desktop,
        component: Component::Desktop,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    });
    let sessions = Arc::new(Mutex::new(None));
    let error = OnceLock::new();
    let epoch = AtomicU64::new(0);
    let state = CommandState(&sessions, &error, &epoch, diagnostics.handle());
    let broker = crate::web_confirmation::ConfirmationBroker::default();
    let context = ClientContext {
        id: "main",
        confirmations: &broker,
        active: &|| true,
        visible: &|| true,
        notify: None,
    };
    assert!(matches!(desktop_snapshot(&context, &state).await,
        Err(DesktopFailure::Native { code }) if code == "RESIDENT_UNAVAILABLE"));
    let context = ClientContext { id: "", ..context };
    let request =
        ObservationReadRequestV2::new(ObservationReadIntentV2::Status(ObservationStatusQueryV2 {}));
    assert!(matches!(observation_read(&context, &state, request).await,
        Err(DesktopFailure::Native { code }) if code == "CLIENT_CONTEXT_INVALID"));
    diagnostics.shutdown();
    let bytes = std::fs::read_to_string(root.join("desktop/current.jsonl")).unwrap();
    let reads: Vec<_> = bytes
        .lines()
        .filter_map(|line| {
            let record = DiagnosticRecordV1::parse_line(line.as_bytes()).unwrap();
            match record.event {
                DiagnosticEvent::ProductReadEnd(read) => Some((read.operation, read.error)),
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        reads,
        vec![
            (
                ControlOperation::DesktopSnapshot,
                Some(EventErrorCode::ResidentUnavailable)
            ),
            (
                ControlOperation::ObservationRead,
                Some(EventErrorCode::ClientContextInvalid)
            ),
        ]
    );
}
