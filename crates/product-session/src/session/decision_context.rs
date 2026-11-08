//! Current save uses the exact server previewed context supplied by the product form.
use super::*;
use hiroute_domain::{
    DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionServiceChangeV3, DecisionServiceOperationInputV3,
};

const APPLY_DECISION_SERVICE: &str = "ApplyDecisionServiceV3";

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
        expected_configuration_digest: Option<CanonicalDigest>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        if change.service.is_none() {
            if secret.is_some() {
                return Err("DECISION_SERVICE_INVALID".into());
            }
            let digest = expected_configuration_digest.ok_or("DECISION_REMOVAL_TARGET_CHANGED")?;
            return self.apply_confirmed_decision_removal(change, digest).await;
        }
        if expected_configuration_digest.is_some() {
            return Err("DECISION_SERVICE_INVALID".into());
        }
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
        let result = match self.prepare_decision_service_v3(input).await {
            Ok(apply) => query(&self.client, APPLY_DECISION_SERVICE, &apply).await,
            Err(error) => Err(error),
        };
        if let Some(candidate) = candidate {
            self.authority.release_model_input(&candidate)?;
        }
        result
    }

    async fn apply_confirmed_decision_removal(
        &mut self,
        change: DecisionServiceChangeV3,
        configuration_digest: CanonicalDigest,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        let intent = IntentEvidence {
            schema: "hiroute.desktop-decision-removal-intent/v1".into(),
            digest: CanonicalDigest::of(&(
                &change.id,
                change.expected_revision,
                &configuration_digest,
            ))
            .map_err(|_| "DECISION_SERVICE_INVALID")?,
        };
        if self.hint.as_ref().and_then(|hint| hint.intent.as_ref()) == Some(&intent) {
            let operation = self.find_pending().await?;
            return self.decision_removal_result(operation);
        }
        let target = DecisionTestRevisionQueryV1 {
            id: change.id.clone(),
            revision: change.expected_revision,
        };
        let apply = self
            .prepare_decision_service_v3(DecisionServiceOperationInputV3 {
                schema: DECISION_SERVICE_OPERATION_INPUT_SCHEMA_V3.into(),
                change,
                input_slot: None,
            })
            .await?;
        // Pin the workspace CAS before reading the exact record. Another save or
        // removal after either read invalidates Apply's expected revisions.
        let current: DecisionServiceTestRequestV2 =
            query(&self.client, RESOLVE_DECISION_TEST_REVISION_V1, &target).await?;
        if current.id != target.id
            || current.revision != target.revision
            || current.expected_configuration_digest != configuration_digest
        {
            return Err("DECISION_REMOVAL_TARGET_CHANGED".into());
        }
        let request_id = crate::random_id()?;
        self.hint = Some(SubmittedOperation {
            plan_id: String::new(),
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: APPLY_DECISION_SERVICE.into(),
            idempotency_key: apply.idempotency_key.clone(),
            accepted_digest: apply.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(intent),
            latest_edit_not_applied: false,
        });
        let response = self
            .client
            .query(APPLY_DECISION_SERVICE, &request_id, &apply)
            .await;
        if let Err(error) = response
            && error.submission == hiroute_client_core::SubmissionState::NotSent
        {
            self.hint = None;
            return Err(error.into());
        }
        let outcome = self.reconcile_apply_response(response).await?;
        self.decision_removal_result(outcome.operation)
    }

    fn decision_removal_result(
        &mut self,
        operation: Option<ClientOperationViewV1>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        if self
            .hint
            .as_ref()
            .is_some_and(|hint| hint.latest_edit_not_applied)
        {
            return Err("LATEST_EDIT_NOT_APPLIED".into());
        }
        let operation = operation.ok_or("OPERATION_RESULT_UNVERIFIED")?;
        if matches!(operation.state.as_str(), "succeeded" | "rolled_back") {
            self.hint = None;
        }
        Ok(ApplyResultV1 {
            operation_id: operation.operation_id,
            accepted_digest: operation.accepted_digest,
            state: operation.state,
        })
    }

    async fn prepare_decision_service_v3(
        &self,
        input: DecisionServiceOperationInputV3,
    ) -> Result<ApplyRequestV1, DesktopFailure> {
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
            APPLY_DECISION_SERVICE,
            &PreviewRequestV1::new(spec),
        )
        .await?;
        Ok(ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec: preview.normalized_spec,
            accept_digest: preview.change_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: format!("decision-connection-{}", crate::random_id()?),
            apply_capability: None,
        })
    }
}
