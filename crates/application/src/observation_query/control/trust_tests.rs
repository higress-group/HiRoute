use super::*;
use crate::control::ControlStateSnapshotV1;
use crate::trust::{
    context::{TrustContextV1, TrustRunV1},
    report::tests::{trial_context, trial_population},
};
use hiroute_domain::*;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Ports {
    control: AtomicU64,
    evidence: AtomicU64,
    visibility: AtomicU64,
    context: AtomicU64,
    reads: AtomicUsize,
    stages: AtomicUsize,
    worker_state: AtomicUsize,
    population_delay_once_ms: AtomicU64,
    fail_first: Mutex<bool>,
}
impl ControlStatePort for Ports {
    fn snapshot(&self, _: &WorkspaceId) -> Result<ControlStateSnapshotV1, ControlReadError> {
        Ok(ControlStateSnapshotV1 {
            revisions: RevisionSetV1 {
                target: self.control.load(Ordering::Relaxed),
                dependencies: Default::default(),
            },
            desired_state: None,
            recoverable_operations: vec![],
        })
    }
    fn operation(&self, _: &OperationId) -> Result<Option<OperationV1>, ControlReadError> {
        Ok(None)
    }
    fn validate_protected_capability(
        &self,
        _: &str,
        _: &WorkspaceId,
        _: PrincipalKind,
        _: &str,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
    ) -> Result<(), ControlReadError> {
        Err(ControlReadError::Denied)
    }
    fn trust_plan_ids(&self, _: &WorkspaceId) -> Result<Vec<String>, ControlReadError> {
        Ok(vec!["plan/oversized".into(), "plan/a".into()])
    }
    fn trust_context(&self, _: &WorkspaceId, id: &str) -> Result<TrustContextV1, ControlReadError> {
        let mut c = trial_context(800);
        c.active.plan_id = AgentPlanId::parse(id).unwrap();
        c.generation = CanonicalDigest::of(&self.context.load(Ordering::Relaxed)).unwrap();
        Ok(c)
    }
    fn trust_runs(
        &self,
        _: &WorkspaceId,
        ids: &[String],
    ) -> Result<BTreeMap<String, TrustRunV1>, ControlReadError> {
        Ok(ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    TrustRunV1 {
                        run_id: id.clone(),
                        task_id: "task/trial".into(),
                        ordinal: 1,
                        state: if self.worker_state.load(Ordering::Relaxed) == 1 {
                            hiroute_domain::delegation::RunStateV1::Running
                        } else {
                            hiroute_domain::delegation::RunStateV1::Succeeded
                        },
                    },
                )
            })
            .collect())
    }
}
impl ApplicationClockPort for Ports {
    fn now_ms(&self) -> Result<i64, ControlReadError> {
        Ok(5000)
    }
}
impl ValueScopePort for Ports {
    fn value_plan_ids(
        &self,
        _: &WorkspaceId,
        _: &ValuePlanScopeV1,
    ) -> Result<Vec<AgentPlanId>, ControlReadError> {
        Ok(vec![])
    }
}
impl ObservationQueryPort for Ports {
    fn observed_trust_generation(
        &self,
        _: &ObservationReaderContext,
        _: i64,
    ) -> Result<(u64, u64), ObservationQueryError> {
        Ok((
            self.evidence.load(Ordering::Relaxed),
            self.visibility.load(Ordering::Relaxed),
        ))
    }
    fn observed_trust_suggestion_population(
        &self,
        _: &ObservationReaderContext,
        q: &TrustPopulationQueryV1,
        _: i64,
    ) -> Result<TrustPopulationV1, ObservationQueryError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(
            self.population_delay_once_ms.swap(0, Ordering::Relaxed),
        ));
        if q.plan_id == "plan/oversized" && *self.fail_first.lock().unwrap() {
            return Err(ObservationQueryError::ScopeTooLarge);
        }
        let mut population = trial_population(self.stages.load(Ordering::Relaxed).max(40), 0.9);
        if self.worker_state.load(Ordering::Relaxed) > 0 {
            population.active_stages[0].sample.run_id = Some("run/trial".into());
        }
        Ok(population)
    }
    fn list_sessions(
        &self,
        _: &WorkspaceId,
        _: &SessionListQueryV1,
    ) -> Result<SessionListV1, ObservationQueryError> {
        unreachable!()
    }
    fn get_session(
        &self,
        _: &WorkspaceId,
        _: &SessionId,
        _: ContentMode,
    ) -> Result<SessionDetailV1, ObservationQueryError> {
        unreachable!()
    }
    fn get_receipt(
        &self,
        _: &WorkspaceId,
        _: &ReceiptId,
    ) -> Result<RoutingReceiptV1, ObservationQueryError> {
        unreachable!()
    }
    fn get_value(
        &self,
        _: &WorkspaceId,
        _: &ValueQueryV1,
    ) -> Result<ValueViewV1, ObservationQueryError> {
        unreachable!()
    }
    fn get_status(&self, _: &WorkspaceId) -> Result<ObservationStatusV1, ObservationQueryError> {
        unreachable!()
    }
}
#[test]
fn trust_suggestion_cache_rechecks_all_mutation_inputs_and_time() {
    let ports = Arc::new(Ports::default());
    let control =
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports.clone());
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        SEVEN_DAYS_MILLIS * 2,
        false,
        false,
    )
    .unwrap();
    let first = control.trust_suggestions(&reader, 5000).unwrap();
    assert_eq!(first["suggestions"].as_array().unwrap().len(), 2);
    assert_eq!(ports.reads.load(Ordering::Relaxed), 2);
    assert_eq!(first, control.trust_suggestions(&reader, 5001).unwrap());
    assert_eq!(ports.reads.load(Ordering::Relaxed), 2);
    for (index, generation) in [
        &ports.evidence,
        &ports.control,
        &ports.context,
        &ports.visibility,
    ]
    .into_iter()
    .enumerate()
    {
        generation.fetch_add(1, Ordering::Relaxed);
        control
            .trust_suggestions(&reader, 5002 + index as i64)
            .unwrap();
        assert_eq!(ports.reads.load(Ordering::Relaxed), 4 + index * 2);
    }
    control
        .trust_suggestions(&reader, SEVEN_DAYS_MILLIS + 1010)
        .unwrap();
    assert_eq!(ports.reads.load(Ordering::Relaxed), 12);
    reader.cancel();
    assert!(control.trust_suggestions(&reader, 5000).is_err());
}
#[test]
fn trust_suggestion_budget_defers_work_without_reporting_a_plan_failure() {
    let ports = Arc::new(Ports::default());
    ports.population_delay_once_ms.store(850, Ordering::Relaxed);
    let control =
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports.clone());
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap();
    let first = control.trust_suggestions(&reader, 5000).unwrap();
    assert_eq!(first["suggestions"].as_array().unwrap().len(), 1);
    assert_eq!(
        first["unavailable"],
        json!([]),
        "Deferred work is not a failed read"
    );
    assert_eq!(first["pending"], json!(["plan/a"]));
    assert_eq!(ports.reads.load(Ordering::Relaxed), 1);
    let completed = control.trust_suggestions(&reader, 5001).unwrap();
    assert_eq!(completed["suggestions"].as_array().unwrap().len(), 2);
    assert_eq!(completed["pending"], json!([]));
    assert_eq!(completed["unavailable"], json!([]));
    assert_eq!(
        ports.reads.load(Ordering::Relaxed),
        2,
        "Continuation re-read completed populations"
    );
}

#[test]
fn trust_suggestion_oversized_plan_keeps_healthy_plan_available() {
    let ports = Arc::new(Ports::default());
    *ports.fail_first.lock().unwrap() = true;
    let control =
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports.clone());
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap();
    let result = control.trust_suggestions(&reader, 5000).unwrap();
    assert_eq!(result["suggestions"].as_array().unwrap().len(), 1);
    assert_eq!(result["suggestions"][0]["plan_id"], "plan/a");
    assert_eq!(
        result["unavailable"],
        json!([{"plan_id":"plan/oversized","reason":"scope_too_large"}])
    );
}

#[test]
fn trust_suggestion_cache_rechecks_worker_completion_without_new_observation() {
    let ports = Arc::new(Ports::default());
    ports.worker_state.store(1, Ordering::Relaxed);
    let control =
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports.clone());
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        20_000,
        false,
        false,
    )
    .unwrap();
    control.trust_suggestions(&reader, 5000).unwrap();
    assert_eq!(ports.reads.load(Ordering::Relaxed), 2);
    control.trust_suggestions(&reader, 5001).unwrap();
    assert_eq!(ports.reads.load(Ordering::Relaxed), 2);
    ports.worker_state.store(2, Ordering::Relaxed);
    control.trust_suggestions(&reader, 5002).unwrap();
    assert_eq!(
        ports.reads.load(Ordering::Relaxed),
        4,
        "a completed Worker changes evidence eligibility before another observation write"
    );
}

#[test]
#[ignore = "application cache timing evidence; run explicitly in release mode"]
fn trust_suggestions_cached_read_cost() {
    let ports = Arc::new(Ports::default());
    ports.stages.store(49_999, Ordering::Relaxed);
    let control =
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports.clone());
    let reader = ObservationReaderContext::local_user(
        WorkspaceId::default(),
        "user".into(),
        1,
        100_000,
        false,
        false,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let cold = control.trust_suggestions(&reader, 5000).unwrap();
    let cold_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(cold["suggestions"].as_array().unwrap().len(), 2);
    assert_eq!(ports.reads.load(Ordering::Relaxed), 2);
    let mut times = Vec::new();
    for i in 1..=40 {
        let started = std::time::Instant::now();
        let result = control.trust_suggestions(&reader, 5000 + i).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.);
        assert_eq!(cold, result);
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "SUGGESTION_CACHE plans=2 stages_per_plan=49999 samples=40 cold_application_ms={cold_ms:.3} cached_p50_ms={:.3} cached_p95_ms={:.3}",
        times[20], times[38]
    );
    assert_eq!(
        ports.reads.load(Ordering::Relaxed),
        2,
        "cache hits must not read the population"
    );
    assert!(times[38] < 20.);
}
