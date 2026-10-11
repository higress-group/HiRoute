//! Explicit connection lifecycle; legacy saves keep their replacement-selection semantics.
use super::*;
use hiroute_application_api::{ComputeManagementEditV1, ComputeSaveChangeViewV2};
use hiroute_domain::{ComputeManagementSourceV2, RevisionSetV1};

impl<C, R, S, I> ComputeManagementPlanner<'_, C, R, S, I>
where
    C: ComputeCandidatePort,
    R: ComputeManagementRepositoryPort,
    S: SecretStorePort,
    I: ProtectedInputPort,
{
    pub(super) fn validate_name_edit(
        &self,
        change: &ComputeManagementChangeV2,
        resolved: &ResolvedSubject,
        sources: &[ComputeManagementSourceV2],
    ) -> Result<(), ComputeManagementPlanningErrorV2> {
        if let Some(ComputeManagementEditV1::Rename { display_name }) = &change.edit {
            let name = display_name.trim();
            if sources.iter().any(|source| {
                source.source_id != resolved.source_id && source.display_name.trim() == name
            }) {
                return Err(ComputeManagementPlanningErrorV2::InvalidChange);
            }
        }
        Ok(())
    }

    pub(super) fn preview_lifecycle(
        &self,
        change: ComputeManagementChangeV2,
        resolved: ResolvedSubject,
        revisions: RevisionSetV1,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        let current = resolved
            .current
            .as_ref()
            .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
        let mut source = current.clone();
        source.revision = source
            .revision
            .checked_add(1)
            .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
        let mut secrets = Vec::new();
        let mut discovery_guards = Vec::new();
        let desired = match change
            .edit
            .as_ref()
            .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?
        {
            ComputeManagementEditV1::Rename { display_name } => {
                source.display_name = display_name.trim().to_owned();
                Some(source)
            }
            ComputeManagementEditV1::RemoveModels => {
                let selected = select_saved_models(current, &change.selected_model_refs)?;
                source.models.retain(|model| {
                    !selected
                        .iter()
                        .any(|removed| removed.binding_id == model.binding_id)
                });
                if source.models.is_empty() {
                    return Err(ComputeManagementPlanningErrorV2::ModelNotSelectable);
                }
                Some(source)
            }
            ComputeManagementEditV1::Delete => {
                for key in &current.credentials {
                    if self.secrets.generation(&key.credential)? != key.credential.generation() {
                        return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
                    }
                    secrets.push(
                        SecretMutationV1::delete(
                            key.credential.clone(),
                            key.credential.generation(),
                        )
                        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?,
                    );
                }
                None
            }
            ComputeManagementEditV1::AppendModels => {
                let candidate = resolved
                    .candidate
                    .as_ref()
                    .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
                if candidate.existing_source_id.as_deref() != Some(current.source_id.as_str()) {
                    return Err(ComputeManagementPlanningErrorV2::LineageConflict);
                }
                ensure_unique_nonempty(&change.selected_model_refs)?;
                let mut additions = Vec::new();
                for reference in &change.selected_model_refs {
                    let member = candidate
                        .models
                        .iter()
                        .find(|model| &model.model_ref == reference);
                    if current.models.iter().any(|old| {
                        &old.model_ref == reference
                            || member
                                .is_some_and(|new| new.upstream_model_id == old.upstream_model_id)
                    }) {
                        continue;
                    }
                    member.ok_or(ComputeManagementPlanningErrorV2::ModelNotSelectable)?;
                    additions.push(reference.clone());
                }
                if candidate.validation.as_ref() != change.validation.as_ref() {
                    return Err(ComputeManagementPlanningErrorV2::ValidationConflict);
                }
                // Validate the checked connection identity even when every selected member already
                // exists. Append cannot quietly replace the endpoint, credentials or provenance.
                let target = candidate
                    .target
                    .as_ref()
                    .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
                if target.scheme != current.target.scheme
                    || target.authority != current.target.authority
                    || target.port != current.target.port
                    || target.request_path != current.target.request_path
                    || target.upstream_protocol != current.target.upstream_protocol
                    || target.protocol_profile_id != current.target.protocol_profile_id
                    || target.protocol_profile_revision != current.target.protocol_profile_revision
                    || candidate.authentication.as_ref() != Some(&current.authentication)
                    || candidate.additional_native_endpoints != current.additional_native_endpoints
                {
                    return Err(ComputeManagementPlanningErrorV2::LineageConflict);
                }
                self.materialize_primary_candidate_key(
                    candidate,
                    Some(current),
                    &change,
                    &mut source,
                    &mut secrets,
                    &mut discovery_guards,
                )?;
                if !additions.is_empty() {
                    source.models.extend(select_candidate_models(
                        &current.source_id,
                        Some(current),
                        candidate,
                        &additions,
                        change.intent,
                    )?);
                }
                source.validation = map_validation(candidate.validation.as_ref());
                source.last_candidate_ref = candidate.candidate.candidate_ref.clone();
                source.last_candidate_revision = candidate.candidate.candidate_revision;
                Some(source)
            }
        };
        if let Some(source) = &desired {
            source
                .validate()
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        }
        self.seal_preview(
            change,
            Some(current),
            desired,
            resolved
                .candidate
                .as_ref()
                .map(|facts| facts.candidate.clone()),
            revisions,
            secrets,
            discovery_guards,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn seal_preview(
        &self,
        change: ComputeManagementChangeV2,
        current: Option<&ComputeManagementSourceV2>,
        desired: Option<ComputeManagementSourceV2>,
        candidate: Option<ComputeCandidateRefV2>,
        revisions: RevisionSetV1,
        secrets: Vec<SecretMutationV1>,
        discovery_guards: Vec<ComputePreparedDiscoveryGuardV1>,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        let source = desired
            .as_ref()
            .or(current)
            .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
        let changes = if let Some(desired) = &desired {
            preview_changes(current, desired)
        } else {
            let mut changes = vec![ComputeSaveChangeViewV2 {
                resource_kind: "compute_source".into(),
                resource_id: source.source_id.clone(),
                action: "delete".into(),
            }];
            changes.extend(source.models.iter().map(|model| ComputeSaveChangeViewV2 {
                resource_kind: "model_binding".into(),
                resource_id: model.binding_id.clone(),
                action: "remove".into(),
            }));
            changes.extend(
                source
                    .credentials
                    .iter()
                    .map(|key| ComputeSaveChangeViewV2 {
                        resource_kind: "credential_key".into(),
                        resource_id: key.key_id.clone(),
                        action: "remove".into(),
                    }),
            );
            changes
        };
        let removed: Vec<_> = changes
            .iter()
            .filter(|change| change.resource_kind == "model_binding" && change.action == "remove")
            .map(|change| change.resource_id.clone())
            .collect();
        let affected_plan_refs = if removed.is_empty() {
            Vec::new()
        } else {
            self.repository
                .compute_management_references(&self.workspace, &removed)?
        };
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "compute.connection.apply".into(),
            resource_id: Some(source.source_id.clone()),
            desired_state: serde_json::to_value(&change)
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
        };
        let plan = TransactionPlanV1::from_compute_management_change(
            spec.clone(),
            current,
            desired,
            secrets,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        // Keep the released save digest unchanged. The writer re-reads references atomically;
        // a preview is descriptive and never grants permission to bypass that read.
        let accept_digest = CanonicalDigest::of(&(
            "hiroute.compute-management-preview/v2",
            &spec,
            &revisions,
            plan.control(),
            plan.secrets(),
        ))
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        Ok(ComputeManagementPreparedPreviewV2 {
            result: ComputeSavePreviewV2 {
                candidate,
                validation: change.validation,
                spec,
                accept_digest,
                expected_revisions: revisions,
                changes,
                affected_plan_refs,
            },
            plan,
            discovery_guards,
        })
    }
}
