pub(super) mod analysis;
mod api_equivalent;
use super::*;
use hiroute_application_api::{ObservationReadIntentV2, ObservationReadRequestV2};
use hiroute_domain::ObservationReaderContext;

// This identity is part of released v2 cursor bindings, independently of the
// transport's current grant admission. Renaming it invalidates old cursors.
const RELEASED_LOCAL_QUERY_SUBJECT: &str = "protected-local-observer";

impl ObservationControl {
    pub(crate) fn read_v2(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
        operation: &str,
    ) -> Result<Value, ObservationQueryError> {
        let started = std::time::Instant::now();
        let request: ObservationReadRequestV2 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        if request.operation() != operation {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let now = self.clock.now_ms().map_err(map_control)?;
        let analysis = matches!(&request.intent, ObservationReadIntentV2::Analysis(_));
        let search = matches!(
            request.intent,
            ObservationReadIntentV2::Search(_) | ObservationReadIntentV2::SessionSearch(_)
        ) || matches!(&request.intent, ObservationReadIntentV2::Analysis(q) if q.query.is_some());
        let budget = if search {
            ObservationReaderContext::SEARCH_QUERY_BUDGET
        } else {
            ObservationReaderContext::QUERY_BUDGET
        };
        let deadline = started + budget;
        let admitted = if let Some(grant) = grant {
            if grant.principal_kind.is_collaboration() {
                return Err(ObservationQueryError::Unauthorized);
            }
            let digest = hiroute_domain::CanonicalDigest::of(&request)
                .map_err(|_| ObservationQueryError::InvalidQuery)?;
            self.state.try_validate_observation_capability(
                &grant.capability,
                &self.workspace_id,
                grant.principal_kind,
                request.protected_operation(),
                &digest,
                deadline,
            )
        } else {
            self.state
                .try_observation_revision(&self.workspace_id, deadline)
        };
        if std::time::Instant::now() >= deadline {
            return Err(ObservationQueryError::ReadTimeout);
        }
        let generation = match admitted.map_err(map_control)? {
            crate::control::ControlReadAttempt::Ready(revision) => revision,
            crate::control::ControlReadAttempt::Busy => {
                return Err(ObservationQueryError::ReaderBusy);
            }
        };
        let content = matches!(
            request.intent,
            ObservationReadIntentV2::Catalog(_)
                | ObservationReadIntentV2::Ancestry(_)
                | ObservationReadIntentV2::Content(_)
                | ObservationReadIntentV2::Search(_)
                | ObservationReadIntentV2::SessionSearch(_)
        ) || matches!(&request.intent, ObservationReadIntentV2::Analysis(q) if q.view.content() || q.query.is_some());
        // The transport authenticates same-OS-user Local Control. Explicit content
        // selection needs no second GUI-issued ticket; supplied grants are still
        // validated on every page. Visibility, expiry and cursor scopes remain checked.
        let reader = ObservationReaderContext::local_user(
            self.workspace_id.clone(),
            if analysis {
                "local-observer"
            } else {
                RELEASED_LOCAL_QUERY_SUBJECT
            }
            .into(),
            generation,
            now.saturating_add(budget.as_millis() as i64),
            content,
            search,
        )?;
        let suggestions_read = matches!(
            &request.intent,
            ObservationReadIntentV2::PlanTrustSuggestions(_)
        );
        let trust_read = matches!(
            &request.intent,
            ObservationReadIntentV2::RoutingTimeline(_)
                | ObservationReadIntentV2::PlanTrust(_)
                | ObservationReadIntentV2::PlanTrustReplay(_)
                | ObservationReadIntentV2::PlanTrustSuggestions(_)
        );
        let reader = if search {
            reader.with_search_query_deadline(deadline)
        } else {
            reader.with_query_deadline(deadline)
        };
        let result = match request.intent {
            ObservationReadIntentV2::Analysis(q) => {
                if (q.analysis_scope.is_some() || q.cursor.is_some()) && !q.time.is_empty() {
                    return Err(ObservationQueryError::InvalidQuery);
                }
                let range = if q.analysis_scope.is_some() || q.cursor.is_some() {
                    None
                } else {
                    Some(super::super::time::resolve(
                        self.clock.as_ref(),
                        &q.time,
                        now,
                    )?)
                };
                serde_json::to_value(self.analysis_read(&reader, q, range.as_ref(), now)?)
            }
            ObservationReadIntentV2::PlanTrustSuggestions(_) => {
                Ok(self.trust_suggestions(&reader, now)?)
            }
            ObservationReadIntentV2::PlanTrust(q) => Ok(self.trust_result(&reader, &q, None, now)?),
            ObservationReadIntentV2::PlanTrustReplay(q) => {
                if q.thresholds_millis.is_empty()
                    || q.thresholds_millis.len() > 8
                    || q.thresholds_millis.iter().any(|t| *t > 1000)
                {
                    return Err(ObservationQueryError::InvalidQuery);
                }
                Ok(self.trust_result(&reader, &q.scope, Some(&q.thresholds_millis), now)?)
            }
            ObservationReadIntentV2::Status(_) => {
                serde_json::to_value(self.service.observed_maintenance_status(&reader, now)?)
            }
            ObservationReadIntentV2::HomeValue(q) => {
                let from_ms = match q
                    .period
                    .unwrap_or(hiroute_application_api::ValuePeriodV1::Today)
                {
                    hiroute_application_api::ValuePeriodV1::Today => {
                        self.clock.local_day_start_ms(now).map_err(map_control)?
                    }
                    hiroute_application_api::ValuePeriodV1::SevenDays => {
                        // Session details cannot use day archives. Their default window
                        // begins at the first retained millisecond, not one millisecond
                        // before it (which would mark every fresh session partial).
                        now.saturating_sub(7 * DAY_MILLIS)
                            .saturating_add(i64::from(q.session_id.is_some()))
                    }
                    hiroute_application_api::ValuePeriodV1::ThirtyDays => {
                        now.saturating_sub(30 * DAY_MILLIS)
                    }
                };
                serde_json::to_value(self.service.observed_value_totals(
                    &reader,
                    &hiroute_domain::ObservationValueQueryV2 {
                        from_ms,
                        to_ms: now,
                        session_id: q.session_id,
                        plan_id: None,
                        currency: q.currency,
                    },
                    now,
                )?)
            }
            ObservationReadIntentV2::Sessions(q) => {
                serde_json::to_value(self.service.observed_sessions(&reader, &q, now)?)
            }
            ObservationReadIntentV2::RoutingTimeline(q) => {
                serde_json::to_value(self.service.observed_routing_timeline(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Timeline(q) => {
                serde_json::to_value(self.service.observed_timeline(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Catalog(q) => {
                serde_json::to_value(self.service.observed_catalog(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Ancestry(q) => {
                serde_json::to_value(self.service.observed_ancestry(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Content(q) => {
                serde_json::to_value(self.service.observed_content_page(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Facts(q) => {
                serde_json::to_value(self.service.observed_facts(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Search(q) => {
                serde_json::to_value(self.service.search_observed_text(&reader, &q, now)?)
            }
            ObservationReadIntentV2::SessionSearch(q) => {
                serde_json::to_value(self.service.search_observed_sessions(&reader, &q, now)?)
            }
            ObservationReadIntentV2::Value(q) => {
                serde_json::to_value(self.service.observed_value_totals(&reader, &q, now)?)
            }
            ObservationReadIntentV2::ValueReport(q) => {
                if q.from_ms.is_some() && q.period.is_some() {
                    return Err(ObservationQueryError::InvalidQuery);
                }
                let to_ms = q.to_ms.unwrap_or(now);
                let from_ms = match q.from_ms {
                    Some(from_ms) => from_ms,
                    None => match q
                        .period
                        .unwrap_or(hiroute_application_api::ValuePeriodV1::SevenDays)
                    {
                        hiroute_application_api::ValuePeriodV1::Today => {
                            self.clock.local_day_start_ms(to_ms).map_err(map_control)?
                        }
                        hiroute_application_api::ValuePeriodV1::SevenDays => to_ms
                            .saturating_sub(7 * DAY_MILLIS)
                            .saturating_add(i64::from(q.session_id.is_some())),
                        hiroute_application_api::ValuePeriodV1::ThirtyDays => {
                            to_ms.saturating_sub(30 * DAY_MILLIS)
                        }
                    },
                };
                if from_ms >= to_ms {
                    return Err(ObservationQueryError::InvalidQuery);
                }
                serde_json::to_value(self.service.observed_value_report(
                    &reader,
                    &hiroute_domain::ObservationValueQueryV2 {
                        from_ms,
                        to_ms,
                        session_id: q.session_id.map(|id| id.to_string()),
                        plan_id: q.agent_plan_id.map(|id| id.as_str().to_owned()),
                        currency: q.currency,
                    },
                    q.group_by,
                    now,
                )?)
            }
            ObservationReadIntentV2::ApiEquivalentValue(q) => {
                serde_json::to_value(self.api_equivalent_value(&reader, q, now)?)
            }
            ObservationReadIntentV2::PlanQuality(q) => serde_json::to_value(
                self.service
                    .observed_plan_quality_samples(&reader, &q, now)?,
            ),
        }
        .map_err(|_| ObservationQueryError::Corrupt)?;
        if serde_json::to_vec(&result)
            .map_err(|_| ObservationQueryError::Corrupt)?
            .len()
            > 1024 * 1024
        {
            return Err(if trust_read {
                ObservationQueryError::ScopeTooLarge
            } else {
                ObservationQueryError::Unavailable
            });
        }
        if trust_read && !suggestions_read && started.elapsed() > std::time::Duration::from_secs(1)
        {
            return Err(ObservationQueryError::ScopeTooLarge);
        }
        Ok(result)
    }
}

impl ObservationControl {
    pub(crate) fn delete_preview(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        let request: hiroute_application_api::ObservationDeletePreviewRequestV2 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let principal = self.protected_principal(grant, "PreviewSessionDeletionV2", &request)?;
        serde_json::to_value(self.service.preview_session_deletion_v2(
            &principal,
            &request.spec,
            self.clock.now_ms().map_err(map_control)?,
        )?)
        .map_err(|_| ObservationQueryError::Corrupt)
    }
    pub(crate) fn delete_apply(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        let request: hiroute_application_api::ObservationDeleteApplyRequestV2 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let principal = self.protected_principal(grant, "ApplySessionDeletionV2", &request)?;
        serde_json::to_value(self.service.apply_session_deletion_v2(
            &principal,
            &request.preview,
            &request.accepted_digest,
            self.clock.now_ms().map_err(map_control)?,
        )?)
        .map_err(|_| ObservationQueryError::Corrupt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_local_query_binding_survives_renewal_without_widening_authority() {
        // v0.3.1 binds workspace, subject, allowed runs, content/search and
        // authorization generation in this order. Expiry is renewed per page.
        let reader = |generation, expiry, content| {
            ObservationReaderContext::local_user(
                hiroute_domain::WorkspaceId::default(),
                RELEASED_LOCAL_QUERY_SUBJECT.into(),
                generation,
                expiry,
                content,
                false,
            )
            .unwrap()
        };
        let released = "sha256:d583c78c0b0b28da9fd74d4e18e43f62970f10cb581b93dd9e3d0e0f60fe6bc0";
        assert_eq!(reader(7, 1000, false).binding().unwrap(), released);
        assert_eq!(reader(7, 2000, false).binding().unwrap(), released);
        assert_ne!(reader(8, 2000, false).binding().unwrap(), released);
        assert_ne!(reader(7, 2000, true).binding().unwrap(), released);
    }
}
