use super::*;
use crate::authority::HostAuthority;
use hiroute_domain::{
    DecisionAuthenticationV2, DecisionConnectionV1, DecisionConnectionV2, DecisionServiceChangeV1,
    DecisionServiceOperationInputV2, DecisionServiceV1,
};

#[derive(Deserialize, Serialize)]
pub struct DecisionServiceList {
    pub services: Vec<DecisionServiceV1>,
}

impl Session {
    pub async fn decision_connection_sources(
        &self,
    ) -> Result<DecisionConnectionSourcesV1, DesktopFailure> {
        query(
            &self.client,
            "ListDecisionConnectionSourcesV1",
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn decision_services_v2(&self) -> Result<DecisionServicesV2, DesktopFailure> {
        query(
            &self.client,
            "ListDecisionServicesV2",
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn test_decision_service_v3(
        &self,
        request: DecisionServiceTestRequestV2,
    ) -> Result<DecisionServiceTestResultV3, DesktopFailure> {
        query(&self.client, TEST_DECISION_SERVICE_V3, &request).await
    }

    pub async fn test_decision_service_v2(
        &self,
        request: DecisionServiceTestRequestV2,
    ) -> Result<DecisionServiceTestResultV2, DesktopFailure> {
        query(&self.client, "TestDecisionServiceV2", &request).await
    }

    pub async fn save_decision_service_v2(
        &mut self,
        mut change: DecisionServiceChangeV2,
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
                DecisionConnectionV2::Native {
                    authentication: DecisionAuthenticationV2::SharedSource { .. },
                    ..
                } => {
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
        let input = DecisionServiceOperationInputV2 {
            change,
            input_slot: candidate
                .as_ref()
                .map(|candidate| candidate.candidate_ref.clone()),
        };
        let result = self.apply_decision_service_v2(input).await;
        if let Some(candidate) = candidate {
            self.authority.release_model_input(&candidate)?;
        }
        result
    }

    async fn apply_decision_service_v2(
        &self,
        input: DecisionServiceOperationInputV2,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        if !input.change.validate() {
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
            "ApplyDecisionServiceV2",
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
        query(&self.client, "ApplyDecisionServiceV2", &apply).await
    }

    pub async fn decision_services(&self) -> Result<DecisionServiceList, DesktopFailure> {
        query(&self.client, "ListDecisionServices", &serde_json::json!({})).await
    }

    pub async fn save_decision_service(
        &mut self,
        mut change: DecisionServiceChangeV1,
        secret: Option<zeroize::Zeroizing<String>>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        {
            if change.input_slot.is_some() {
                return Err("DECISION_SERVICE_INVALID".into());
            }
            let candidate = if let Some(secret) = secret {
                if secret.is_empty() || secret.contains(['\r', '\n']) {
                    return Err("DECISION_CREDENTIAL_INVALID".into());
                }
                let service = change.service.as_mut().ok_or("DECISION_SERVICE_INVALID")?;
                let reference = format!("decision/{}/{}", service.id, crate::random_id()?);
                let value = match &mut service.connection {
                    DecisionConnectionV1::SystemOne { auth_header, .. } => {
                        auth_header.name = "Authorization".into();
                        auth_header.value_secret_ref = reference;
                        zeroize::Zeroizing::new(format!("Bearer {}", secret.as_str().trim()))
                    }
                    DecisionConnectionV1::Custom { auth_header, .. } => {
                        auth_header
                            .as_mut()
                            .ok_or("DECISION_AUTH_HEADER_REQUIRED")?
                            .value_secret_ref = reference;
                        secret
                    }
                };
                let candidate = self.authority.register_model_input(value)?;
                change.input_slot = Some(candidate.candidate_ref.clone());
                Some(candidate)
            } else {
                None
            };
            let result = self.apply_decision_service_change(change).await;
            if let Some(candidate) = candidate {
                self.authority.release_model_input(&candidate)?;
            }
            result
        }
    }

    async fn apply_decision_service_change(
        &self,
        change: DecisionServiceChangeV1,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        change.validate().map_err(|_| "DECISION_SERVICE_INVALID")?;
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "decision.services.apply".into(),
            resource_id: Some(change.id.clone()),
            desired_state: serde_json::to_value(change).map_err(|_| "DECISION_SERVICE_INVALID")?,
        };
        let preview: PreviewResultV1 = query(
            &self.client,
            "ApplyDecisionService",
            &PreviewRequestV1::new(spec),
        )
        .await?;
        let apply = ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec: preview.normalized_spec,
            accept_digest: preview.change_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: format!("decision-service-{}", crate::random_id()?),
            apply_capability: None,
        };
        query(&self.client, "ApplyDecisionService", &apply).await
    }
}
