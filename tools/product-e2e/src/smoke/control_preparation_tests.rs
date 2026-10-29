use super::*;
use crate::smoke::{CaseReport, Execution, SmokeError, build::Source};
use std::sync::{Arc, atomic::AtomicBool};

fn with_context(test: impl FnOnce(&mut Context<'_>)) {
    let directory = tempfile::tempdir().unwrap();
    let source = Source {
        root: directory.path().to_path_buf(),
        revision: "deadline-boundary-unit".into(),
        tree: String::new(),
        inputs: String::new(),
    };
    let mut report = CaseReport {
        id: "control.status-discovery".into(),
        domain: "control".into(),
        scope: "deadline_boundary_unit".into(),
        owner: "control".into(),
        execution: Execution::Executed,
        state: None,
        reason: None,
        steps: vec![],
        build_ms: 123,
        execution_ms: 0,
        integrity_ms: 0,
        unclassified_ms: 0,
        timing_complete: true,
        cleanup: "not_started".into(),
    };
    let mut artifacts = vec![];
    test(&mut Context {
        source: &source,
        run_id: "deadline-boundary-unit",
        private: directory.path().to_path_buf(),
        runtime: directory.path().to_path_buf(),
        deadline: Instant::now(),
        cancel: Arc::new(AtomicBool::new(false)),
        report: &mut report,
        artifacts: &mut artifacts,
        phase: "control_build",
    });
}

fn evidence(ctx: &Context<'_>) -> Value {
    serde_json::from_slice(&fs::read(ctx.private.join("control-storage-prepare.json")).unwrap())
        .unwrap()
}

#[test]
fn progressing_cold_setup_does_not_consume_ordinary_control_deadlines() {
    with_context(|ctx| {
        // Deterministically model the observed boundary: initialization completes
        // after the ordinary 10s watchdog, but inside its own cold-setup budget.
        // This clock test does not claim to run a daemon or initialize a database.
        let finished = Instant::now();
        let started = finished - Duration::from_secs(12);
        let old_ready_deadline = started + Duration::from_secs(10);
        ctx.deadline = old_ready_deadline;
        let mut clock = [started, finished].into_iter();
        prepare_storage(
            ctx,
            "unit-daemon-digest",
            || clock.next().unwrap(),
            |deadline| {
                assert!(finished > old_ready_deadline);
                assert!(finished < deadline);
                assert_eq!(deadline - started, Duration::from_secs(45));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(ctx.deadline, old_ready_deadline);
        // The control owner enters the ordinary scenario after all prerequisite
        // work succeeds; its production budget is independent of cold setup.
        ctx.product_deadline();
        assert!(ctx.deadline >= finished + Duration::from_secs(30));
        let before = Instant::now();
        let step_deadline = ctx.step_deadline();
        let after = Instant::now();
        assert!(step_deadline >= before + Duration::from_secs(10));
        assert!(step_deadline <= after + Duration::from_secs(10));
        assert!(step_deadline < ctx.deadline);
        let value = evidence(ctx);
        assert_eq!(value["elapsed_ms"], 12_000);
        assert_eq!(value["timeout_ms"], 45_000);
        assert_eq!(value["outcome"], "completed");
        assert!(value["reason"].is_null());
        assert_eq!(ctx.report.steps.len(), 1);
        assert_eq!(ctx.report.steps[0].id, STEP);
        assert_eq!(
            ctx.report.steps[0].evidence_digest,
            digest(&serde_json::to_vec(&value).unwrap())
        );
        assert_eq!(
            ctx.report.build_ms, 123,
            "storage setup is never build time"
        );
        // The unchanged outer runner measures all non-build wall time, including
        // this prerequisite; the helper must not subtract or double-count it.
        assert_eq!(ctx.report.execution_ms, 0);
        assert_eq!(ctx.report.unclassified_ms, 0);
        assert!(ctx.report.timing_complete);
    });
}

#[test]
fn failed_or_overdue_cold_setup_never_admits_product_steps() {
    for (elapsed, outcome, expected_reason) in [
        (
            45,
            Err(SmokeError("initial_storage_ready_timeout")),
            "initial_storage_ready_timeout",
        ),
        (
            2,
            Err(SmokeError("child_exited_before_ready")),
            "child_exited_before_ready",
        ),
        (
            2,
            Err(SmokeError("process_cleanup_timeout")),
            "process_cleanup_timeout",
        ),
        (2, Err(SmokeError("cancelled")), "cancelled"),
        (45, Ok(()), "initial_storage_ready_timeout"),
    ] {
        with_context(|ctx| {
            let finished = Instant::now();
            let started = finished - Duration::from_secs(elapsed);
            let previous_deadline = ctx.deadline;
            let mut clock = [started, finished].into_iter();
            let mut product_entered = false;
            let result = prepare_storage(
                ctx,
                "unit-daemon-digest",
                || clock.next().unwrap(),
                |_| outcome,
            )
            .and_then(|()| {
                product_entered = true;
                ctx.product_deadline();
                ctx.record("control_ready", None, None, &json!({}))
            });
            assert_eq!(result.unwrap_err().0, expected_reason);
            assert!(!product_entered);
            assert_eq!(ctx.phase, STEP);
            assert_eq!(ctx.deadline, previous_deadline);
            assert_eq!(ctx.report.steps.len(), 1);
            let value = evidence(ctx);
            assert_eq!(value["outcome"], "failed");
            assert_eq!(value["reason"], expected_reason);
            assert_eq!(value["elapsed_ms"], elapsed * 1000);
            assert_eq!(ctx.report.build_ms, 123);
        });
    }
}
