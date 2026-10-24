use super::context::{TrustAnnouncementV1, TrustContextV1, TrustRunV1};
use super::{replay::replay, suggestion::suggest};
use hiroute_domain::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct PlanTrustReportV1 {
    pub plan_id: String,
    pub active_revision: u64,
    pub revision: TrustRevisionV1,
    pub from_ms: i64,
    pub to_ms: i64,
    pub generation: u64,
    pub levels: Vec<TrustLevelV1>,
    pub rows: Vec<TrustRowV1>,
    pub branches: Vec<TrustBranchV1>,
    pub sessions: Vec<TrustSessionV1>,
    pub session_total: usize,
    pub announcements: Vec<TrustAnnouncementCountV1>,
    pub executors: Vec<TrustExecutorV1>,
    pub cost: serde_json::Value,
    pub suggestion_rules: serde_json::Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustRowV1 {
    pub branch_id: String,
    pub group: Option<ExecutionGroupV1>,
    pub model_configuration_id: Option<String>,
    pub reasoning_profile_id: Option<String>,
    pub attribution: AgentTurnAttributionV1,
    pub executor: Option<String>,
    pub identity_stability: String,
    pub counts: TrustCountsV1,
    pub execution_share: Option<f64>,
    pub competence_rate: Option<f64>,
    pub scoring_coverage: Option<f64>,
    pub protections: Vec<TrustProtectionV1>,
    pub availability_relays: u64,
    pub duration_p50_ms: Option<u64>,
    pub duration_p90_ms: Option<u64>,
    pub output_tokens_per_second: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustExecutorV1 {
    pub executor: String,
    pub task_outcomes: BTreeMap<String, u64>,
}
fn executor_outcomes(tasks: &[TrustTaskOutcomeV1]) -> Vec<TrustExecutorV1> {
    let mut groups: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for task in tasks {
        let outcomes = groups.entry(task.executor.clone()).or_default();
        *outcomes.entry("submitted".into()).or_default() += 1;
        let state = match task.state {
            delegation::RunStateV1::Succeeded => "succeeded",
            delegation::RunStateV1::Failed => "failed",
            delegation::RunStateV1::Cancelled => "cancelled",
            delegation::RunStateV1::Unknown => "unknown",
            _ => "running",
        };
        *outcomes.entry(state.into()).or_default() += 1;
        if task.ordinal > 1 {
            *outcomes.entry("continued".into()).or_default() += 1;
        }
    }
    groups
        .into_iter()
        .map(|(executor, task_outcomes)| TrustExecutorV1 {
            executor,
            task_outcomes,
        })
        .collect()
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustProtectionV1 {
    pub segment_id: String,
    pub trigger_segment_id: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustSessionV1 {
    pub session_id: String,
    pub stages: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustAnnouncementCountV1 {
    #[serde(flatten)]
    pub announcement: TrustAnnouncementV1,
    pub before: u64,
    pub announcement_day: u64,
    pub after: u64,
    pub executed_version_verified: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrustBranchV1 {
    pub branch_id: String,
    pub threshold_millis: u16,
    pub level: Option<TrustLevelV1>,
    pub full_history: TrustCountsV1,
    pub upward_cohort: TrustCountsV1,
    pub upward_from_ms: i64,
    pub suggestion: TrustSuggestionV1,
    pub replay: PlanTrustReplayV1,
}

fn terminal(run: Option<&TrustRunV1>) -> bool {
    use hiroute_domain::delegation::RunStateV1 as State;
    run.is_some_and(|r| matches!(r.state, State::Succeeded | State::Failed | State::Cancelled))
}
fn counts(stages: &[&TrustObservedStageV1], runs: &BTreeMap<String, TrustRunV1>) -> TrustCountsV1 {
    let mut counts = TrustCountsV1::default();
    let mut sessions = BTreeSet::new();
    for stage in stages {
        counts.executed += 1;
        match &stage.sample.assessment {
            Some(a) if !a.partial => {
                counts.complete += 1;
                sessions.insert(&stage.sample.session_id);
                let floor = stage
                    .sample
                    .branch_execution
                    .as_ref()
                    .and_then(|e| e.policy.floor_millis)
                    .or_else(|| {
                        stage
                            .sample
                            .selection
                            .as_ref()
                            .and_then(|s| s.policy.as_ref().and_then(|p| p.floor_millis))
                    });
                counts.competence_eligible += u64::from(floor.is_some());
                if floor.is_some_and(|floor| a.score >= f64::from(floor) / 1000.0) {
                    counts.meets_floor += 1;
                }
            }
            Some(_) => counts.partial += 1,
            None if !stage.has_later_turn
                && !terminal(stage.sample.run_id.as_ref().and_then(|id| runs.get(id))) =>
            {
                counts.awaiting_assessment += 1
            }
            None => counts.unscored += 1,
        }
    }
    counts.sessions = sessions.len() as u64;
    counts
}
fn branch(stage: &TrustObservedStageV1) -> &str {
    stage
        .sample
        .executed_branch_id
        .as_deref()
        .unwrap_or(&stage.sample.selected_branch_id)
}
fn regular(stage: &TrustObservedStageV1) -> bool {
    stage
        .sample
        .branch_execution
        .as_ref()
        .is_some_and(|e| e.group == ExecutionGroupV1::Regular)
}
fn percentile(values: &mut [u64], percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[(values.len() * percentile).div_ceil(100).saturating_sub(1)])
}

pub fn report(
    query: &PlanTrustQueryV1,
    context: &TrustContextV1,
    population: &TrustPopulationV1,
    runs: &BTreeMap<String, TrustRunV1>,
    tasks: &[TrustTaskOutcomeV1],
) -> PlanTrustReportV1 {
    let mut grouped: BTreeMap<String, Vec<&TrustObservedStageV1>> = BTreeMap::new();
    let mut sessions: BTreeMap<String, u64> = BTreeMap::new();
    for stage in &population.stages {
        *sessions.entry(stage.sample.session_id.clone()).or_default() += 1;
        let sample = &stage.sample;
        let key = serde_json::to_string(&(
            branch(stage),
            sample.branch_execution.as_ref().map(|e| e.group),
            &sample.model_configuration_id,
            &sample.reasoning_profile_id,
            sample.attribution,
            (query.group_by == TrustGroupByV1::Executor).then_some(&stage.executor),
        ))
        .expect("scalar row key");
        grouped.entry(key).or_default().push(stage);
    }
    let rows = grouped
        .values()
        .map(|stages| {
            let first = stages[0];
            let sample = &first.sample;
            let counts = counts(stages, runs);
            let mut durations: Vec<_> = stages
                .iter()
                .flat_map(|s| s.request_durations_ms.iter().copied())
                .collect();
            let throughput: Vec<_> = stages
                .iter()
                .flat_map(|s| s.output_tokens_per_second.iter().copied())
                .collect();
            TrustRowV1 {
                branch_id: branch(first).into(),
                group: sample.branch_execution.as_ref().map(|e| e.group),
                model_configuration_id: sample.model_configuration_id.clone(),
                reasoning_profile_id: sample.reasoning_profile_id.clone(),
                attribution: sample.attribution,
                executor: if query.group_by == TrustGroupByV1::Executor {
                    first.executor.clone()
                } else {
                    None
                },
                identity_stability: sample
                    .model_configuration_id
                    .as_ref()
                    .and_then(|id| context.identity_stability.get(id))
                    .cloned()
                    .unwrap_or_else(|| "unknown".into()),
                competence_rate: counts.competence_rate(),
                scoring_coverage: counts.scoring_coverage(),
                execution_share: (!population.stages.is_empty())
                    .then(|| counts.executed as f64 / population.stages.len() as f64),
                counts,
                protections: stages
                    .iter()
                    .filter_map(|s| {
                        s.sample
                            .selection
                            .as_ref()
                            .and_then(|p| p.competence_trigger.as_ref())
                            .map(|p| TrustProtectionV1 {
                                segment_id: s.sample.segment_id.clone(),
                                trigger_segment_id: p.segment_id.clone(),
                            })
                    })
                    .collect(),
                availability_relays: stages
                    .iter()
                    .filter(|s| {
                        s.sample
                            .selection
                            .as_ref()
                            .zip(s.sample.branch_execution.as_ref())
                            .is_some_and(|(d, e)| {
                                d.execution_group != e.group
                                    || d.selection_reason == ModelGroupReasonV1::AvailabilityRelay
                            })
                    })
                    .count() as u64,
                duration_p50_ms: percentile(&mut durations, 50),
                duration_p90_ms: percentile(&mut durations, 90),
                output_tokens_per_second: (!throughput.is_empty())
                    .then(|| throughput.iter().sum::<f64>() / throughput.len() as f64),
            }
        })
        .collect();
    let (branches, announcements) = branch_evidence(query, context, population, runs, false);
    PlanTrustReportV1 {
        plan_id: query.plan_id.clone(),
        active_revision: context.active.content_revision,
        revision: query.revision.clone(),
        from_ms: population.from_ms,
        to_ms: population.to_ms,
        generation: population.generation,
        levels: TrustLevelV1::all().to_vec(),
        rows,
        branches,
        session_total: sessions.len(),
        sessions: sessions
            .into_iter()
            .take(200)
            .map(|(session_id, stages)| TrustSessionV1 { session_id, stages })
            .collect(),
        announcements,
        executors: executor_outcomes(tasks),
        cost: serde_json::json!({"kind":"unavailable", "reason":"comparison basis not available"}),
        suggestion_rules: serde_json::json!({"min_scored":30,"upper_rate":0.85,"lower_rate":0.70,"min_coverage":0.60,"min_sessions":3}),
    }
}
fn branch_evidence(
    query: &PlanTrustQueryV1,
    context: &TrustContextV1,
    population: &TrustPopulationV1,
    runs: &BTreeMap<String, TrustRunV1>,
    lean: bool,
) -> (Vec<TrustBranchV1>, Vec<TrustAnnouncementCountV1>) {
    let policies = policies(context);
    let active_scope = matches!(
        &query.revision,
        TrustRevisionV1::Name(TrustRevisionNameV1::Active)
    ) || query.revision
        == TrustRevisionV1::Number(context.active.content_revision);
    let branches: Vec<TrustBranchV1> = policies
        .into_iter()
        .filter(|(id, _, _)| query.branch_id.as_ref().is_none_or(|wanted| wanted == id))
        .map(|(id, threshold, models)| {
            let full: Vec<_> = population
                .active_stages
                .iter()
                .filter(|s| {
                    s.sample.plan_revision == context.active.content_revision
                        && branch(s) == id
                        && regular(s)
                })
                .collect();
            let upward_from_ms = context
                .announcements
                .iter()
                .filter(|a| models.contains(&a.model_configuration_id))
                .map(|a| a.cohort_from_ms)
                .max()
                .unwrap_or(population.retained_from_ms)
                .max(population.retained_from_ms);
            let upward: Vec<_> = full
                .iter()
                .copied()
                .filter(|s| s.sample.first_at_ms >= upward_from_ms)
                .collect();
            let full_history = counts(&full, runs);
            let upward_cohort = counts(&upward, runs);
            let suggestion = if active_scope && context.enabled {
                suggest(threshold, &full_history, &upward_cohort, false)
            } else {
                TrustSuggestionV1::Keep {
                    missing: vec!["active_revision_only".into()],
                }
            };
            let branch_stages: Vec<_> = population
                .active_stages
                .iter()
                .filter(|s| {
                    s.sample.plan_revision == context.active.content_revision && branch(s) == id
                })
                .cloned()
                .collect();
            let thresholds = if lean {
                match &suggestion {
                    TrustSuggestionV1::StepUp { threshold_millis }
                    | TrustSuggestionV1::StepDown { threshold_millis } => vec![*threshold_millis],
                    _ => vec![],
                }
            } else {
                TrustLevelV1::all()
                    .iter()
                    .map(|l| l.threshold_millis)
                    .collect()
            };
            TrustBranchV1 {
                branch_id: id,
                threshold_millis: threshold,
                level: TrustLevelV1::from_threshold(threshold).cloned(),
                full_history,
                upward_cohort,
                upward_from_ms,
                suggestion,
                replay: replay(&branch_stages, &thresholds),
            }
        })
        .collect();
    let announcements = context
        .announcements
        .iter()
        .map(|a| {
            let stages = population.stages.iter().filter(|s| {
                s.sample.model_configuration_id.as_ref() == Some(&a.model_configuration_id)
            });
            let mut result = TrustAnnouncementCountV1 {
                announcement: a.clone(),
                before: 0,
                announcement_day: 0,
                after: 0,
                executed_version_verified: false,
            };
            for s in stages {
                if s.sample.first_at_ms >= a.cohort_from_ms {
                    result.after += 1;
                } else if s.sample.first_at_ms >= a.cohort_from_ms - 86_400_000 {
                    result.announcement_day += 1;
                } else {
                    result.before += 1;
                }
            }
            result
        })
        .collect();
    (branches, announcements)
}

pub fn suggestion_evidence(
    query: &PlanTrustQueryV1,
    context: &TrustContextV1,
    population: &TrustPopulationV1,
    runs: &BTreeMap<String, TrustRunV1>,
) -> serde_json::Value {
    let (branches, announcements) = branch_evidence(query, context, population, runs, true);
    let branches: Vec<_> = branches
        .into_iter()
        .filter(|b| !matches!(b.suggestion, TrustSuggestionV1::Keep { .. }))
        .map(|b| {
            let regular: Vec<_> = population
                .active_stages
                .iter()
                .filter(|s| branch(s) == b.branch_id && regular(s))
                .collect();
            let protections = population
                .active_stages
                .iter()
                .filter(|s| branch(s) == b.branch_id)
                .filter(|s| {
                    s.sample
                        .selection
                        .as_ref()
                        .is_some_and(|s| s.competence_trigger.is_some())
                })
                .map(|s| &s.sample.segment_id)
                .collect::<BTreeSet<_>>()
                .len();
            let identities = regular
                .iter()
                .map(|s| {
                    s.sample
                        .model_configuration_id
                        .as_ref()
                        .and_then(|id| context.identity_stability.get(id))
                        .map(String::as_str)
                        .unwrap_or("unknown")
                })
                .collect::<BTreeSet<_>>();
            let mut value = serde_json::to_value(&b).expect("typed suggestion");
            value["protection_count"] = serde_json::json!(protections);
            value["identity_stabilities"] = serde_json::json!(identities);
            value
        })
        .collect();
    serde_json::json!({"plan_id":query.plan_id,"active_revision":context.active.content_revision,"branches":branches,"announcements":announcements,"previous_routing":[]})
}

fn policies(context: &TrustContextV1) -> Vec<(String, u16, BTreeSet<String>)> {
    let models = |candidates: &[CandidateSelectionV1]| {
        candidates
            .iter()
            .filter_map(|c| context.binding_models.get(&c.binding_id).cloned())
            .collect()
    };
    match &context.configuration.strategy {
        AgentPlanStrategyV2::SmartSaving {
            judgment,
            classifier,
            economy,
            ..
        } if native_mode(classifier) => vec![(
            SMART_SAVING_SCOPE_ID.into(),
            judgment.degree.simple_threshold_millis,
            models(economy),
        )],
        AgentPlanStrategyV2::Branches { routing } if native_mode(&routing.classifier) => routing
            .branches
            .iter()
            .filter(|b| !b.primary_candidates.is_empty())
            .map(|b| {
                (
                    b.id.clone(),
                    b.judgment
                        .as_ref()
                        .unwrap_or(&routing.judgment)
                        .degree
                        .simple_threshold_millis,
                    models(&b.candidates),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
pub(crate) mod tests;
