//! macOS component fault boundaries over real CLI preparation / role-all / storage.
//! These tests exercise request observation and dropped deliveries, NOT GUI interaction.
use crate::{
    bootstrap::Resident,
    session::{RenameInput, Session},
};
use hiroute_application_api::*;
use std::path::Path;

use hiroute_product_session::fault::{FAULT, Fault};
fn prepare() -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix("hr02-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/prepare_plan.py");
    let output = std::process::Command::new("python3")
        .arg(script)
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "production preparation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    root
}
fn open(root: &Path) -> Session {
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    crate::session_from_resident(Resident::open(root, &binary).unwrap())
}
async fn plan(session: &mut Session) -> AgentPlanStatusV2 {
    session.snapshot().await.unwrap().catalog.plans.remove(0)
}
async fn independent_change(session: &mut Session, key: String) -> String {
    let current = plan(session).await;
    let mut editor = current
        .desired
        .editor(Some(current.model_alias.as_str().into()))
        .unwrap();
    editor.purpose = "Independently updated purpose".into();
    let change = PlanContentChangeV2 {
        schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
        target: PlanContentTargetV2::Update {
            plan_id: current.agent_plan_id,
            expected_head_revision: current.head.head_revision,
        },
        editor,
        consumed_draft: None,
    };
    let preview: MachineEnvelopeV2<PlanContentPreviewV2> = session
        .client
        .query(
            "PreviewAgentPlanChange",
            "independent-preview",
            &PlanContentPreviewRequestV2 {
                change: change.clone(),
            },
        )
        .await
        .unwrap();
    let preview = preview.data.unwrap();
    let result = session
        .client
        .call_wire(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "independent-apply".into(),
            operation_id: "ApplyAgentPlanChange".into(),
            payload: serde_json::to_value(PlanContentApplyRequestV2 {
                include_model_budget_conflict: false,
                change,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: key,
            })
            .unwrap(),
            protected_grant: None,
        })
        .await
        .unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    result.data.unwrap()["operation_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
#[tokio::test]
async fn local_save_uses_one_user_identity_across_restart() {
    let root = prepare();
    let stale = root.path().join("pending-intent.json");
    std::fs::write(&stale, b"invalid obsolete client receipt").unwrap();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let preview = session
        .preview(RenameInput {
            plan_id: original.agent_plan_id.as_str().into(),
            display_name: "Same user intent".into(),
            language: "en".into(),
        })
        .await
        .unwrap();
    let outcome = session
        .finish_native_confirmation(preview, true)
        .await
        .unwrap();
    let operation = outcome.operation.unwrap();
    assert_eq!(operation.state, "succeeded");
    let hint = session.snapshot().await.unwrap().pending.unwrap();
    assert_eq!(hint.principal_kind, PrincipalKind::InteractiveUser);
    assert_eq!(
        hint.operation_id.as_deref(),
        Some(operation.operation_id.as_str())
    );
    drop(session);
    let mut restarted = open(root.path());
    assert!(restarted.observe().await.unwrap().is_none());
    assert_eq!(
        std::fs::read(&stale).unwrap(),
        b"invalid obsolete client receipt"
    );
    assert_eq!(
        plan(&mut restarted).await.desired.display_name.as_str(),
        "Same user intent"
    );
    assert_eq!(
        plan(&mut restarted).await.agent_plan_revision,
        original.agent_plan_revision + 1
    );
}
#[tokio::test]
async fn accepted_apply_response_loss_recovers_without_another_apply() {
    let root = prepare();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let preview = session
        .preview(RenameInput {
            plan_id: original.agent_plan_id.as_str().into(),
            display_name: "Accepted response lost".into(),
            language: "en".into(),
        })
        .await
        .unwrap();
    FAULT.with(|f| f.set(Fault::LoseApplyResponseAndDisconnect));
    assert!(
        session
            .finish_native_confirmation(preview, true)
            .await
            .is_err()
    );
    let hint = session.snapshot().await.unwrap().pending.unwrap();
    assert!(hint.operation_id.is_none());
    let op = session.observe().await.unwrap().unwrap();
    assert_eq!(op.state, "succeeded");
    let recovered = session.snapshot().await.unwrap().pending.unwrap();
    assert_eq!(recovered.idempotency_key, hint.idempotency_key);
    assert_eq!(recovered.accepted_digest, op.accepted_digest);
    assert_eq!(
        recovered.operation_id.as_deref(),
        Some(op.operation_id.as_str())
    );
    assert_eq!(
        plan(&mut session).await.agent_plan_revision,
        original.agent_plan_revision + 1
    );
    let lookup: MachineEnvelopeV2<OperationIdempotencyResultV1> = session
        .client
        .query(
            "FindOperationByIdempotency",
            "other-principal",
            &OperationIdempotencyLookupV1 {
                include_model_budget_conflict: true,
                principal_kind: PrincipalKind::Desktop,
                operation_kind: hint.operation_kind,
                idempotency_key: hint.idempotency_key,
                accepted_digest: hint.accepted_digest,
            },
        )
        .await
        .unwrap();
    assert!(
        lookup.data.unwrap().operation.is_none(),
        "principal scopes remain distinct"
    );
    assert_eq!(
        session.observe().await.unwrap().unwrap().operation_id,
        op.operation_id
    );
    assert!(!root.path().join("pending-intent.json").exists());
    drop(session);
    let mut restarted = open(root.path());
    assert!(restarted.observe().await.unwrap().is_none());
    let current = plan(&mut restarted).await;
    assert_eq!(
        current.desired.display_name.as_str(),
        "Accepted response lost"
    );
    assert_eq!(
        current.agent_plan_revision,
        original.agent_plan_revision + 1
    );
}

#[tokio::test]
async fn concurrent_admission_after_lookup_restores_the_winning_operation() {
    let root = prepare();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let preview = session
        .preview(RenameInput {
            plan_id: original.agent_plan_id.as_str().into(),
            display_name: "Latest edit must not win".into(),
            language: "en".into(),
        })
        .await
        .unwrap();
    let key = preview.test_key().to_owned();
    let winner = independent_change(&mut session, key.clone()).await;
    let outcome = session
        .finish_native_confirmation(preview, true)
        .await
        .unwrap();
    assert_eq!(outcome.operation.unwrap().operation_id, winner);
    let hint = session.snapshot().await.unwrap().pending.unwrap();
    assert_eq!(hint.idempotency_key, key);
    assert!(hint.latest_edit_not_applied);
    let current = plan(&mut session).await;
    assert_eq!(
        current.agent_plan_revision,
        original.agent_plan_revision + 1
    );
    assert_eq!(current.desired.display_name, original.desired.display_name);
}

#[tokio::test]
async fn undelivered_save_preserves_retry_identity_without_blocking_a_different_edit() {
    let root = prepare();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let input = || RenameInput {
        plan_id: original.agent_plan_id.as_str().into(),
        display_name: "Undelivered edit".into(),
        language: "en".into(),
    };
    let first = session.preview(input()).await.unwrap();
    let key = first.test_key().to_owned();
    FAULT.with(|f| f.set(Fault::LoseBeforeApply));
    assert!(
        session
            .finish_native_confirmation(first, true)
            .await
            .is_err()
    );
    let retry = session.preview(input()).await.unwrap();
    assert_eq!(retry.test_key(), key);
    session
        .finish_native_confirmation(retry, false)
        .await
        .unwrap();
    let other = session
        .preview(RenameInput {
            plan_id: original.agent_plan_id.as_str().into(),
            display_name: "New explicit edit".into(),
            language: "en".into(),
        })
        .await
        .unwrap();
    assert_ne!(other.test_key(), key);
    assert_eq!(
        session
            .finish_native_confirmation(other, true)
            .await
            .unwrap()
            .operation
            .unwrap()
            .state,
        "succeeded"
    );
    assert_eq!(
        plan(&mut session).await.agent_plan_revision,
        original.agent_plan_revision + 1
    );
    assert_eq!(
        plan(&mut session).await.desired.display_name.as_str(),
        "New explicit edit"
    );
    assert!(!root.path().join("pending-intent.json").exists());
}

#[path = "editor_publication_tests.rs"]
mod editor_publication;
