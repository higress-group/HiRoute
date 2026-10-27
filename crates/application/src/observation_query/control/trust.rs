use super::*;
use hiroute_domain::{
    ObservationReaderContext, PlanTrustQueryV1, TrustPopulationQueryV1, TrustRevisionNameV1,
    TrustRevisionV1,
};

impl ObservationControl {
    pub(super) fn trust_suggestions(
        &self,
        reader: &ObservationReaderContext,
        now: i64,
    ) -> Result<Value, ObservationQueryError> {
        reader.check(now, false, false)?;
        let started = std::time::Instant::now();
        let generation = self
            .state
            .snapshot(&self.workspace_id)
            .map_err(map_control)?
            .revisions;
        let observation = self.service.observed_trust_generation(reader, now)?;
        let binding = reader.binding()?;
        let mut ids = self
            .state
            .trust_plan_ids(&self.workspace_id)
            .map_err(map_control)?;
        let mut cache = {
            let mut guard = self
                .trust_suggestion_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        cache.entries.retain(|id, _| ids.contains(id));
        if !ids.is_empty() {
            let offset = cache.next % ids.len();
            ids.rotate_left(offset);
        }
        let mut suggestions = Vec::new();
        let mut unavailable = Vec::new();
        let mut pending = Vec::new();
        let mut deferred = None;
        for (index, plan_id) in ids.iter().enumerate() {
            let result = (|| {
                let context = self
                    .state
                    .trust_context(&self.workspace_id, plan_id)
                    .map_err(map_control)?;
                let key =
                    CanonicalDigest::of(&(&binding, &generation, observation, &context.generation))
                        .map_err(|_| ObservationQueryError::Corrupt)?;
                if let Some(entry) = cache.entries.get(plan_id)
                    && entry.key == key
                    && now >= entry.created_at
                    && now < entry.expires_at
                {
                    let runs = self
                        .state
                        .trust_runs(&self.workspace_id, &entry.run_ids)
                        .map_err(map_control)?;
                    if CanonicalDigest::of(&runs).map_err(|_| ObservationQueryError::Corrupt)?
                        == entry.runs
                    {
                        return Ok(Some(entry.value.clone()));
                    }
                }
                if started.elapsed() >= std::time::Duration::from_millis(800) {
                    deferred.get_or_insert(index);
                    return Ok(None);
                }
                let query = PlanTrustQueryV1 {
                    plan_id: plan_id.clone(),
                    branch_id: None,
                    revision: Default::default(),
                    from_ms: None,
                    to_ms: None,
                    group_by: Default::default(),
                };
                let population = self.service.observed_trust_suggestion_population(
                    reader,
                    &TrustPopulationQueryV1 {
                        active_revision: context.active.content_revision,
                        plan_id: plan_id.clone(),
                        branch_id: None,
                        revision: Some(context.active.content_revision),
                        from_ms: None,
                        to_ms: None,
                    },
                    now,
                )?;
                let run_ids: Vec<_> = population
                    .active_stages
                    .iter()
                    .filter_map(|s| s.sample.run_id.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let runs = self
                    .state
                    .trust_runs(&self.workspace_id, &run_ids)
                    .map_err(map_control)?;
                let mut value =
                    crate::trust::report::suggestion_evidence(&query, &context, &population, &runs);
                if value["branches"].as_array().is_some_and(|branches| {
                    branches
                        .iter()
                        .any(|b| b["suggestion"]["kind"] == "change_regular_model")
                }) {
                    let targets = self
                        .state
                        .trust_recovery_targets(plan_id)
                        .map_err(map_control)?;
                    if !targets.is_empty() {
                        for branch in value["branches"]
                            .as_array_mut()
                            .ok_or(ObservationQueryError::Corrupt)?
                        {
                            if branch["suggestion"]["kind"] == "change_regular_model" {
                                branch["suggestion"] = json!({"kind":"return_to_previous_routing"});
                            }
                        }
                    }
                    value["previous_routing"] = serde_json::to_value(targets)
                        .map_err(|_| ObservationQueryError::Corrupt)?;
                }
                if self
                    .state
                    .trust_context(&self.workspace_id, plan_id)
                    .map_err(map_control)?
                    .generation
                    != context.generation
                    || self.service.observed_trust_generation(reader, now)? != observation
                {
                    return Err(ObservationQueryError::StalePreview);
                }
                {
                    let expires_at = population
                        .active_stages
                        .iter()
                        .map(|s| {
                            s.sample
                                .last_at_ms
                                .saturating_add(hiroute_domain::SEVEN_DAYS_MILLIS)
                        })
                        .min()
                        .unwrap_or(i64::MAX)
                        .min(now.saturating_add(300_000));
                    cache.entries.insert(
                        plan_id.clone(),
                        SuggestionEntry {
                            key,
                            runs: CanonicalDigest::of(&runs)
                                .map_err(|_| ObservationQueryError::Corrupt)?,
                            run_ids,
                            created_at: now,
                            expires_at,
                            value: value.clone(),
                        },
                    );
                }
                Ok(Some(value))
            })();
            match result {
                Ok(Some(value)) if value["branches"].as_array().is_some_and(|b| !b.is_empty()) => {
                    suggestions.push(value)
                }
                Ok(Some(_)) => {}
                Ok(None) => pending.push(plan_id.clone()),
                Err(error) => {
                    cache.entries.remove(plan_id);
                    unavailable.push(json!({"plan_id":plan_id,"reason":match error { ObservationQueryError::ScopeTooLarge => "scope_too_large", ObservationQueryError::StalePreview => "stale", _ => "unavailable" }}));
                }
            }
        }
        if let Some(index) = deferred {
            cache.next = (cache.next + index) % ids.len().max(1);
        }
        if self
            .state
            .snapshot(&self.workspace_id)
            .map_err(map_control)?
            .revisions
            != generation
        {
            return Err(ObservationQueryError::StalePreview);
        }
        {
            let mut guard = self
                .trust_suggestion_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.entries.extend(cache.entries);
            guard.entries.retain(|id, _| ids.contains(id));
            guard.next = cache.next;
        }
        Ok(json!({"suggestions":suggestions,"unavailable":unavailable,"pending":pending}))
    }

    pub(super) fn trust_result(
        &self,
        reader: &ObservationReaderContext,
        query: &PlanTrustQueryV1,
        thresholds: Option<&[u16]>,
        now: i64,
    ) -> Result<Value, ObservationQueryError> {
        let context = self
            .state
            .trust_context(&self.workspace_id, &query.plan_id)
            .map_err(map_control)?;
        let revision = match &query.revision {
            TrustRevisionV1::Number(n) => Some(*n),
            TrustRevisionV1::Name(TrustRevisionNameV1::Active) => {
                Some(context.active.content_revision)
            }
            TrustRevisionV1::Name(TrustRevisionNameV1::All) => None,
        };
        let population = self.service.observed_trust_population(
            reader,
            &TrustPopulationQueryV1 {
                active_revision: context.active.content_revision,
                plan_id: query.plan_id.clone(),
                branch_id: query.branch_id.clone(),
                revision,
                from_ms: query.from_ms,
                to_ms: query.to_ms,
            },
            now,
        )?;
        let latest = self
            .state
            .trust_context(&self.workspace_id, &query.plan_id)
            .map_err(map_control)?;
        if latest.generation != context.generation {
            return Err(ObservationQueryError::StalePreview);
        }
        if let Some(thresholds) = thresholds {
            return serde_json::to_value(crate::trust::replay::replay(
                &population.stages,
                thresholds,
            ))
            .map_err(|_| ObservationQueryError::Corrupt);
        }
        let ids = population
            .stages
            .iter()
            .chain(population.active_stages.iter())
            .filter_map(|s| s.sample.run_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let runs = self
            .state
            .trust_runs(&self.workspace_id, &ids)
            .map_err(map_control)?;
        let tasks = if query.branch_id.is_none() {
            self.state
                .trust_task_population(
                    &self.workspace_id,
                    &TrustPopulationQueryV1 {
                        active_revision: context.active.content_revision,
                        plan_id: query.plan_id.clone(),
                        branch_id: None,
                        revision,
                        from_ms: Some(population.from_ms),
                        to_ms: Some(population.to_ms),
                    },
                    population.from_ms,
                    population.to_ms,
                )
                .map_err(map_control)?
        } else {
            Default::default()
        };
        if tasks.over_limit {
            return Err(ObservationQueryError::ScopeTooLarge);
        }
        let mut report =
            crate::trust::report::report(query, &context, &population, &runs, &tasks.tasks);
        report.cost = self.trust_cost(
            reader,
            query,
            revision,
            population.from_ms,
            population.to_ms,
            now,
        );
        let mut targets = Vec::new();
        if report.branches.iter().any(|branch| {
            matches!(
                branch.suggestion,
                hiroute_domain::TrustSuggestionV1::ChangeRegularModel
            )
        }) {
            targets = self
                .state
                .trust_recovery_targets(&query.plan_id)
                .map_err(map_control)?;
            if !targets.is_empty() {
                for branch in &mut report.branches {
                    if matches!(
                        branch.suggestion,
                        hiroute_domain::TrustSuggestionV1::ChangeRegularModel
                    ) {
                        branch.suggestion = crate::trust::suggestion::suggest(
                            branch.threshold_millis,
                            &branch.full_history,
                            &branch.upward_cohort,
                            true,
                        );
                    }
                }
            }
        }
        let mut value = serde_json::to_value(report).map_err(|_| ObservationQueryError::Corrupt)?;
        value["previous_routing"] =
            serde_json::to_value(targets).map_err(|_| ObservationQueryError::Corrupt)?;
        Ok(value)
    }
}

#[derive(Default)]
pub(super) struct SuggestionCache {
    entries: std::collections::BTreeMap<String, SuggestionEntry>,
    next: usize,
}
struct SuggestionEntry {
    run_ids: Vec<String>,
    runs: CanonicalDigest,
    key: CanonicalDigest,
    created_at: i64,
    expires_at: i64,
    value: Value,
}
