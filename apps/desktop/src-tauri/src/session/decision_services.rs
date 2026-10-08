use super::*;
use hiroute_domain::{DecisionConnectionV1, DecisionServiceChangeV1, DecisionServiceV1};

#[derive(Deserialize, Serialize)]
pub struct DecisionServiceList {
    pub services: Vec<DecisionServiceV1>,
}

impl Session {
    pub async fn decision_services(&self) -> Result<DecisionServiceList, DesktopFailure> {
        query(&self.client, "ListDecisionServices", &serde_json::json!({})).await
    }

    pub async fn save_decision_service(
        &mut self,
        mut change: DecisionServiceChangeV1,
        secret: Option<zeroize::Zeroizing<String>>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        #[cfg(unix)]
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
                let candidate = self.resident.register_model_input(value)?;
                change.input_slot = Some(candidate.candidate_ref.clone());
                Some(candidate)
            } else {
                None
            };
            let result = self.apply_decision_service_change(change).await;
            if let Some(candidate) = candidate {
                self.resident.release_model_input(&candidate)?;
            }
            result
        }
        #[cfg(not(unix))]
        {
            let _ = (change, secret);
            Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
        }
    }

    #[cfg(unix)]
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
