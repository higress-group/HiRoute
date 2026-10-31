//! Budget policy joins the existing source/Secret operation before Key activation.
use super::*;
use hiroute_application_api::{ComputeKeyBudgetSaveRequestV1, ComputeManagementChangeV6};
use hiroute_domain::{
    ComputeManagementMutation, KeyBudgetAssignmentV1, KeyBudgetDefinitionV1, KeyBudgetEditV1,
    KeyBudgetMutationV1,
};
use std::collections::{BTreeMap, BTreeSet};

impl<C, R, S, I> ComputeManagementPlanner<'_, C, R, S, I>
where
    C: ComputeCandidatePort,
    R: ComputeManagementRepositoryPort,
    S: SecretStorePort,
    I: ProtectedInputPort,
{
    pub fn preview_key_budgets(
        &self,
        request: ComputeKeyBudgetSaveRequestV1,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        let preview = if let Some(network) = request.network_edit {
            self.preview_network(
                serde_json::to_value(request.change)
                    .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
                network,
            )?
        } else {
            self.preview_v3(request.change)?
        };
        self.finish_key_budget_preview(preview, request.key_budget_edits)
    }
    pub fn preview_v6(
        &self,
        change: ComputeManagementChangeV6,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if change.schema != hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V6
            || change
                .change
                .get("schema")
                .and_then(serde_json::Value::as_str)
                == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V6)
        {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        self.finish_key_budget_preview(
            self.preview_saved_state(change.change)?,
            change.key_budget_edits,
        )
    }
    fn finish_key_budget_preview(
        &self,
        mut preview: ComputeManagementPreparedPreviewV2,
        edits: Vec<KeyBudgetEditV1>,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        let invalid = || ComputeManagementPlanningErrorV2::InvalidChange;
        if edits.is_empty() || edits.len() > 256 {
            return Err(invalid());
        }
        let source_mutation: ComputeManagementMutation =
            serde_json::from_value(preview.plan.control()["compute_management_mutation"].clone())
                .map_err(|_| invalid())?;
        let source = source_mutation.desired().ok_or_else(invalid)?;
        if matches!(
            source.provenance,
            hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { .. }
        ) {
            return Err(invalid());
        }
        let bindings_before = self.repository.key_budget_bindings(&source.source_id)?;
        let mut bindings_after = bindings_before.clone();
        bindings_after.retain(|key, _| {
            source
                .credentials
                .iter()
                .any(|credential| &credential.key_id == key)
        });
        let mut before = BTreeMap::<String, KeyBudgetDefinitionV1>::new();
        let mut after = before.clone();
        let mut initial_remaining = BTreeMap::new();
        let mut edited = BTreeSet::new();
        let mut configured = BTreeSet::new();
        for edit in &edits {
            let key_id = edit.key.resolve(&source.source_id).map_err(|_| invalid())?;
            if !source.credentials.iter().any(|key| key.key_id == key_id)
                || !edited.insert(key_id.clone())
            {
                return Err(invalid());
            }
            match &edit.assignment {
                KeyBudgetAssignmentV1::Unlimited => {
                    bindings_after.remove(&key_id);
                }
                KeyBudgetAssignmentV1::Share {
                    budget_id,
                    expected_revision,
                } => {
                    let value = self
                        .repository
                        .key_budget_definition(budget_id)?
                        .ok_or_else(invalid)?;
                    if value.revision != *expected_revision {
                        return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
                    }
                    before.insert(budget_id.clone(), value.clone());
                    after.entry(budget_id.clone()).or_insert(value);
                    bindings_after.insert(key_id, budget_id.clone());
                }
                KeyBudgetAssignmentV1::Configure {
                    budget_id,
                    expected_revision,
                    display_name,
                    policy,
                    initial_remaining_micros,
                } => {
                    policy.validate().map_err(|_| invalid())?;
                    let id = if let Some(id) = budget_id {
                        id.clone()
                    } else {
                        let digest = CanonicalDigest::of(&(
                            "hiroute.key-budget/v1",
                            &source.source_id,
                            &key_id,
                            &preview.result.spec.desired_state,
                        ))
                        .map_err(|_| invalid())?;
                        format!("budget/{}", &digest.as_str()[7..39])
                    };
                    if !configured.insert(id.clone()) {
                        return Err(invalid());
                    }
                    let existing = self.repository.key_budget_definition(&id)?;
                    if existing.as_ref().map_or(0, |value| value.revision) != *expected_revision {
                        return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
                    }
                    if let Some(existing) = existing {
                        if budget_id.is_none()
                            || initial_remaining_micros.is_some()
                            || existing.policy.currency != policy.currency
                        {
                            return Err(invalid());
                        }
                        // A shared budget may only be edited from a Key already linked to it.
                        if bindings_before.get(&key_id) != Some(&id) {
                            return Err(invalid());
                        }
                        before.insert(id.clone(), existing);
                    } else {
                        if budget_id.is_some() || *expected_revision != 0 {
                            return Err(invalid());
                        }
                        let initial = initial_remaining_micros.unwrap_or(policy.amount_micros);
                        if initial > policy.amount_micros {
                            return Err(invalid());
                        }
                        initial_remaining.insert(id.clone(), initial);
                    }
                    let value = KeyBudgetDefinitionV1 {
                        budget_id: id.clone(),
                        revision: expected_revision.checked_add(1).ok_or_else(invalid)?,
                        display_name: display_name.clone(),
                        policy: policy.clone(),
                    };
                    value.validate().map_err(|_| invalid())?;
                    after.insert(id.clone(), value);
                    bindings_after.insert(key_id, id);
                }
            }
        }
        let budgets = KeyBudgetMutationV1 {
            bindings_before,
            bindings_after,
            definitions_before: before.into_values().collect(),
            definitions_after: after.into_values().collect(),
            initial_remaining,
        };
        preview.plan = preview
            .plan
            .with_key_budgets(edits, budgets)
            .map_err(|_| invalid())?;
        preview.result.spec = preview.plan.spec().clone();
        preview.result.accept_digest = CanonicalDigest::of(&(
            "hiroute.compute-management-preview/v6",
            &preview.result.spec,
            &preview.result.expected_revisions,
            preview.plan.control(),
            preview.plan.secrets(),
        ))
        .map_err(|_| invalid())?;
        preview
            .result
            .changes
            .push(hiroute_application_api::ComputeSaveChangeViewV2 {
                resource_kind: "key_budgets".into(),
                resource_id: source_mutation.source_id().into(),
                action: "update".into(),
            });
        Ok(preview)
    }
}
