use super::*;
use hiroute_application_api::{ComputeManagementChangeV4, NativeNetworkEditV1};
use hiroute_domain::{
    ComputeManagementMutation, NativeNetworkMutationV1, NativeSourceNetworkRecordV1,
};

impl<C, R, S, I> ComputeManagementPlanner<'_, C, R, S, I>
where
    C: ComputeCandidatePort,
    R: ComputeManagementRepositoryPort,
    S: SecretStorePort,
    I: ProtectedInputPort,
{
    pub fn preview_v4(
        &self,
        change: ComputeManagementChangeV4,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if change.schema != hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4 {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        self.preview_network(change.change, change.network_edit)
    }

    pub fn preview_network(
        &self,
        change: serde_json::Value,
        edit: NativeNetworkEditV1,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        if !matches!(
            change.get("schema").and_then(serde_json::Value::as_str),
            Some(
                hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2
                    | hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3
            )
        ) {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        let mut preview = self.preview_saved_state(change)?;
        let mutation: ComputeManagementMutation =
            serde_json::from_value(preview.plan.control()["compute_management_mutation"].clone())
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let desired = mutation
            .desired()
            .filter(|s| s.provenance.is_native())
            .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
        let before = self
            .repository
            .native_source_network(mutation.source_id())?;
        if edit.expected_network_revision != before.as_ref().map_or(0, |r| r.network_revision) {
            return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
        }
        let mut after = before.clone().unwrap_or(
            NativeSourceNetworkRecordV1::initial(desired)
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
        );
        let policy = edit
            .policy
            .normalized()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let changed = before.as_ref().is_none_or(|r| r.policy != policy);
        if before.is_some() && changed {
            after.network_revision = after
                .network_revision
                .checked_add(1)
                .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
        }
        after.policy = policy;
        let network = NativeNetworkMutationV1 {
            schema: hiroute_domain::NATIVE_NETWORK_MUTATION_SCHEMA_V1.into(),
            before,
            after: Some(after),
        };
        preview.plan = preview
            .plan
            .with_source_network(network)
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
        if changed {
            preview
                .result
                .changes
                .push(hiroute_application_api::ComputeSaveChangeViewV2 {
                    resource_kind: "native_network_policy".into(),
                    resource_id: mutation.source_id().into(),
                    action: "update".into(),
                });
        }
        Ok(preview)
    }
}
