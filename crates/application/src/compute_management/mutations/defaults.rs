use super::*;
use hiroute_application_api::ComputeManagementChangeV3;
use hiroute_domain::{COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3, ComputeManagementMutationV2};

impl<'a, C, R, S, I> ComputeManagementPlanner<'a, C, R, S, I>
where
    C: ComputeCandidatePort,
    R: ComputeManagementRepositoryPort,
    S: SecretStorePort,
    I: ProtectedInputPort,
{
    pub fn preview_v3(
        &self,
        mut change: ComputeManagementChangeV3,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if change.schema != COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3 {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        change
            .change
            .validate_shape()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let api_link = change.api_link.take();
        let mut metadata_preferences = None;
        let metadata_source = if api_link.is_some()
            && let hiroute_application_api::ComputeManagementSubjectV2::SavedSource { source_id } =
                &change.change.subject
        {
            let saved = self
                .repository
                .compute_management_source(source_id)?
                .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
            let intent = if saved.state == hiroute_domain::MaterializationState::Disabled {
                hiroute_application_api::ComputeManagementIntentV2::SaveDisabled
            } else {
                hiroute_application_api::ComputeManagementIntentV2::SaveReady
            };
            if !matches!(&saved.provenance, hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } if connector_id == "connector.cpa.codex")
                || change.change.edit.is_some()
                || !change.change.key_edits.is_empty()
                || change.change.intent != intent
                || change.change.validation.is_some()
                || !change
                    .change
                    .selected_model_refs
                    .iter()
                    .eq(saved.models.iter().map(|model| &model.model_ref))
            {
                return Err(ComputeManagementPlanningErrorV2::InvalidChange);
            }
            let preferences = self.repository.source_default_preferences(source_id)?;
            // Reproduction receives the full sealed V3 preference map; an initial metadata
            // request may omit it. Neither form may change any saved preference.
            if !change.default_preferences.is_empty() && change.default_preferences != preferences {
                return Err(ComputeManagementPlanningErrorV2::InvalidChange);
            }
            metadata_preferences = Some(preferences);
            Some(saved)
        } else {
            None
        };
        let mut preview = if let Some(saved) = metadata_source {
            let snapshot = self
                .repository
                .compute_management_snapshot(&self.workspace)?;
            if snapshot.revisions != change.change.expected_revisions {
                return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
            }
            let mut desired = saved.clone();
            desired.revision = desired
                .revision
                .checked_add(1)
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
            // Link-only edits preserve the saved authorization exactly; they neither fabricate
            // a check receipt nor request another OAuth check.
            self.seal_preview(
                change.change,
                Some(&saved),
                Some(desired),
                None,
                snapshot.revisions,
                Vec::new(),
                Vec::new(),
            )?
        } else {
            self.preview(change.change)?
        };
        let mutation: ComputeManagementMutationV2 = serde_json::from_value(
            preview
                .plan
                .control()
                .get("compute_management_mutation")
                .cloned()
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let before = match metadata_preferences {
            Some(preferences) => preferences,
            None => self
                .repository
                .source_default_preferences(mutation.source_id())?,
        };
        let mut after = before.clone();
        for (binding, fields) in change.default_preferences {
            after.entry(binding).or_default().extend(fields);
        }
        after.retain(|binding, _| {
            mutation
                .desired()
                .is_some_and(|s| s.models.iter().any(|m| &m.binding_id == binding))
        });
        preview.plan = preview
            .plan
            .with_source_defaults(before, after)
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        preview.result.spec = preview.plan.spec().clone();
        preview.result.accept_digest = CanonicalDigest::of(&(
            "hiroute.compute-management-preview/v3",
            &preview.result.spec,
            &preview.result.expected_revisions,
            preview.plan.control(),
            preview.plan.secrets(),
        ))
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        preview
            .result
            .changes
            .push(hiroute_application_api::ComputeSaveChangeViewV2 {
                resource_kind: "source_defaults".into(),
                resource_id: mutation.source_id().into(),
                action: "update".into(),
            });
        if let Some(intent) = api_link {
            let source = mutation
                .desired()
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
            if !matches!(&source.provenance, hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } if connector_id == "connector.cpa.codex")
            {
                return Err(ComputeManagementPlanningErrorV2::InvalidChange);
            }
            if let hiroute_domain::ComputeSubscriptionApiLinkIntentV1::Associate { api_source_id } =
                &intent
            {
                let target = self
                    .repository
                    .compute_management_source(api_source_id)?
                    .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
                if !target.provenance.is_native() || target.source_id == source.source_id {
                    return Err(ComputeManagementPlanningErrorV2::InvalidChange);
                }
            }
            let before = self
                .repository
                .compute_subscription_api_links(&self.workspace, Some(&source.source_id))?
                .into_iter()
                .next();
            preview.plan = preview
                .plan
                .with_subscription_api_link(before, intent)
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
            preview.result.spec = preview.plan.spec().clone();
            preview.result.accept_digest = CanonicalDigest::of(&(
                "hiroute.compute-management-preview/v4",
                &preview.result.spec,
                &preview.result.expected_revisions,
                preview.plan.control(),
                preview.plan.secrets(),
            ))
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
            preview
                .result
                .changes
                .push(hiroute_application_api::ComputeSaveChangeViewV2 {
                    resource_kind: "subscription_api_link".into(),
                    resource_id: source.source_id.clone(),
                    action: "update".into(),
                });
        }
        Ok(preview)
    }
    pub fn preview_saved_state(
        &self,
        value: serde_json::Value,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if value.get("schema").and_then(serde_json::Value::as_str)
            == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
        {
            let saved: hiroute_application_api::ComputeManagementChangeV4 =
                serde_json::from_value(value)
                    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
            if saved.change.api_link.is_some() {
                return Err(ComputeManagementPlanningErrorV2::InvalidChange);
            }
            let mut change = saved.change;
            change.api_link = Some(saved.api_link);
            self.preview_v3(change)
        } else if value.get("schema").and_then(serde_json::Value::as_str)
            == Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3)
        {
            self.preview_v3(
                serde_json::from_value(value)
                    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
            )
        } else {
            self.preview(
                serde_json::from_value(value)
                    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
            )
        }
    }
}
