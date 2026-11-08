//! Current save uses the exact server previewed context supplied by the product form.
use super::*;
use hiroute_domain::{
    DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionServiceChangeV3, DecisionServiceOperationInputV3,
};

impl Session {
    pub async fn preview_decision_removal(
        &self,
        input: hiroute_application_api::DecisionRemovalPreviewRequestV1,
    ) -> Result<hiroute_application_api::DecisionRemovalPreviewV1, DesktopFailure> {
        query(&self.client, "PreviewDecisionRemovalV1", &input).await
    }
    pub async fn decision_services_v3(&self) -> Result<DecisionServicesV3, DesktopFailure> {
        query(
            &self.client,
            "ListDecisionServicesV3",
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn preview_decision_context(
        &self,
        input: DecisionContextPreviewRequestV1,
    ) -> Result<hiroute_domain::ResolvedDecisionContextV1, DesktopFailure> {
        query(&self.client, "PreviewDecisionContextV1", &input).await
    }

    pub async fn save_decision_service_v3(
        &mut self,
        mut change: DecisionServiceChangeV3,
        secret: Option<zeroize::Zeroizing<String>>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        let candidate = if let Some(secret) = secret {
            if secret.trim().is_empty() || secret.contains(['\r', '\n']) {
                return Err("DECISION_CREDENTIAL_INVALID".into());
            }
            let service = change.service.as_mut().ok_or("DECISION_SERVICE_INVALID")?;
            let reference = format!("decision/{}/{}", service.id, crate::random_id()?);
            let value = match &mut service.connection {
                DecisionConnectionV2::Native {
                    authentication: DecisionAuthenticationV2::Independent { auth_header },
                    ..
                } => {
                    auth_header.name = "Authorization".into();
                    auth_header.value_secret_ref = reference;
                    zeroize::Zeroizing::new(format!("Bearer {}", secret.as_str().trim()))
                }
                DecisionConnectionV2::Native { .. } => {
                    return Err("DECISION_SERVICE_INVALID".into());
                }
                DecisionConnectionV2::Custom { auth_header, .. } => {
                    auth_header
                        .as_mut()
                        .ok_or("DECISION_AUTH_HEADER_REQUIRED")?
                        .value_secret_ref = reference;
                    secret
                }
            };
            Some(self.authority.register_model_input(value)?)
        } else {
            None
        };
        let input = DecisionServiceOperationInputV3 {
            schema: DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3.into(),
            change,
            input_slot: candidate
                .as_ref()
                .map(|candidate| candidate.candidate_ref.clone()),
        };
        let result = self.apply_decision_service_v3(input).await;
        if let Some(candidate) = candidate {
            self.authority.release_model_input(&candidate)?;
        }
        result
    }

    async fn apply_decision_service_v3(
        &self,
        input: DecisionServiceOperationInputV3,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        if !input.validate() {
            return Err("DECISION_SERVICE_INVALID".into());
        }
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "decision.connections.apply".into(),
            resource_id: Some(input.change.id.clone()),
            desired_state: serde_json::to_value(input).map_err(|_| "DECISION_SERVICE_INVALID")?,
        };
        let preview: PreviewResultV1 = query(
            &self.client,
            "ApplyDecisionServiceV3",
            &PreviewRequestV1::new(spec),
        )
        .await?;
        let apply = ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec: preview.normalized_spec,
            accept_digest: preview.change_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: format!("decision-connection-{}", crate::random_id()?),
            apply_capability: None,
        };
        query(&self.client, "ApplyDecisionServiceV3", &apply).await
    }
}
