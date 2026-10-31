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
        change: ComputeManagementChangeV3,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if change.schema != COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3 {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        let mut preview = self.preview(change.change)?;
        let mutation: ComputeManagementMutationV2 = serde_json::from_value(
            preview
                .plan
                .control()
                .get("compute_management_mutation")
                .cloned()
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let before = self
            .repository
            .source_default_preferences(mutation.source_id())?;
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
        Ok(preview)
    }
    pub fn preview_saved_state(
        &self,
        value: serde_json::Value,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if value.get("schema").and_then(serde_json::Value::as_str)
            == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
        {
            let change = value
                .get("change")
                .cloned()
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
            let network: hiroute_domain::NativeNetworkMutationV1 = serde_json::from_value(
                value
                    .get("network")
                    .cloned()
                    .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?,
            )
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
            let policy = network
                .after
                .as_ref()
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?
                .policy
                .clone();
            return self.preview_network(
                change,
                hiroute_application_api::NativeNetworkEditV1 {
                    expected_network_revision: network
                        .before
                        .as_ref()
                        .map_or(0, |r| r.network_revision),
                    policy,
                },
            );
        }
        if value.get("schema").and_then(serde_json::Value::as_str)
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
