//! Exact pre-release cancellation producer fixture; no live database is edited.
use super::*;
use hiroute_domain::delegation::{DelegationRuntimePort, RunCleanupV1, RunStateV1};

#[test]
fn startup_settles_only_cancelled_before_preparation_and_does_not_replay() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::early_cancel_tests::startup_settles_only_cancelled_before_preparation_and_does_not_replay",
    ) {
        return;
    }
    let root = crate::test_support::private_tempdir();
    let workspace = WorkspaceId::default();
    for prepared in [false, true] {
        let storage = root
            .path()
            .join(if prepared { "prepared" } else { "accepted" });
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            &storage,
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let body = runtime
            .adapter
            .persist_task_input(&workspace, "task", "run", "must not execute", now)
            .unwrap();
        let accepted = tests::worker_list_acceptance("task", "run", body, now);
        let mut run = DelegationRuntimePort::accept(runtime.adapter.as_ref(), &accepted).unwrap();
        // Freeze the prior producer's Accepted -> Cancelling revision-2 bytes. A
        // revision-3 record can have crossed Preparing without recording its child.
        run.progress.state = RunStateV1::Cancelling;
        run.progress.revision = if prepared { 3 } else { 2 };
        run.progress.cancel_requested = true;
        run.lease_revoked = true;
        let connection = rusqlite::Connection::open(storage.join("live/runtime.db")).unwrap();
        connection
            .execute(
                "UPDATE delegation_runs SET record_json=?1 WHERE run_id=?2",
                rusqlite::params![serde_json::to_string(&run).unwrap(), run.run_id],
            )
            .unwrap();
        drop(connection);
        // This is the actual startup owner. The unsafe case has no published test Plan;
        // its owner-reconciliation error is unrelated to the cancellation boundary.
        let reconciled = runtime.adapter.reconcile_delegation_plan_versions();
        let saved = DelegationRuntimePort::run(runtime.adapter.as_ref(), &workspace, "run")
            .unwrap()
            .unwrap();
        if prepared {
            assert_eq!(saved, run);
            assert!(!saved.progress.workspace_releasable());
        } else {
            assert_eq!(saved.progress.state, RunStateV1::Cancelled);
            assert_eq!(saved.progress.cleanup, RunCleanupV1::Complete);
            assert!(saved.progress.workspace_releasable());
            assert!(saved.process.is_none());
            assert!(saved.session.is_none());
            assert!(!saved.progress.prompt_may_have_executed);
            reconciled.unwrap();
            drop(runtime);
            let reopened = ProductionControlRuntime::open_with_release_catalog(
                &storage,
                crate::release_catalog::fixture_catalog(),
            )
            .unwrap();
            assert_eq!(
                DelegationRuntimePort::run(reopened.adapter.as_ref(), &workspace, "run")
                    .unwrap()
                    .unwrap(),
                saved
            );
        }
    }
}
