//! Versioned decision reads and saved-revision diagnostics.
use super::*;
use hiroute_application_api::{
    ClientEmptyRequestV1, DecisionFailureCodeV2, DecisionServiceTestRequestV2,
    DecisionServiceTestResultSchemaV2, DecisionServiceTestResultV2, DecisionSourceStateV1,
    DecisionTestOutcomeV2, DecisionTestRevisionQueryV1,
};
use hiroute_domain::{DecisionAuthenticationV2, DecisionConnectionV2};

impl ApplicationService {
    pub(super) fn apply_decision_connection(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        let Some(desired) = request
            .payload
            .get("spec")
            .and_then(|spec| spec.get("desired_state"))
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        let valid = match request.operation_id.as_str() {
            "ApplyDecisionServiceV2" => serde_json::from_value::<
                hiroute_domain::DecisionServiceOperationInputV2,
            >(desired.clone())
            .is_ok_and(|input| input.change.validate()),
            "ApplyDecisionServiceV3" => serde_json::from_value::<
                hiroute_domain::DecisionServiceOperationInputV3,
            >(desired.clone())
            .is_ok_and(|input| input.validate()),
            _ => false,
        };
        if !valid {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        if request.payload.get("accept_digest").is_some() {
            self.apply_change(request, "decision.connections.apply")
        } else {
            self.preview_change(request, "decision.connections.apply")
        }
    }
    pub(super) fn list_decision_v2(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(port) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.routing.as_deref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        if request.operation_id == "ListDecisionConnectionSourcesV1" {
            match port.decision_connection_sources(&WorkspaceId::default()) {
                Ok(result) => succeeded(result, request.request_id),
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        } else if request.operation_id == "ListDecisionServicesV3" {
            match port.decision_services_v3(&WorkspaceId::default()) {
                Ok(result) => succeeded(result, request.request_id),
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        } else {
            match port.decision_services_v2(&WorkspaceId::default()) {
                Ok(result) => succeeded(result, request.request_id),
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        }
    }

    pub(super) fn resolve_decision_test_revision(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some()
            || !request
                .principal
                .capabilities
                .iter()
                .any(|capability| capability == "same-os-user:query-preview")
        {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Ok(query) = serde_json::from_value::<DecisionTestRevisionQueryV1>(request.payload)
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if query.id.is_empty()
            || query.id.len() > 128
            || !(1..=i64::MAX as u64).contains(&query.revision)
        {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(routing) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.routing.as_deref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let service = match routing.decision_service_record(
            &WorkspaceId::default(),
            &query.id,
            query.revision,
        ) {
            Ok(Some(service)) => service,
            Ok(None) => return failed(ErrorCode::ResourceNotFound, request.request_id),
            Err(error) => return failed(map_control_error(error), request.request_id),
        };
        let Ok(expected_configuration_digest) = CanonicalDigest::of(&service) else {
            return failed(ErrorCode::Internal, request.request_id);
        };
        succeeded(
            DecisionServiceTestRequestV2 {
                id: query.id,
                revision: query.revision,
                expected_configuration_digest,
            },
            request.request_id,
        )
    }

    pub(super) fn test_decision_v2(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        let network_aware =
            request.operation_id == hiroute_application_api::TEST_DECISION_SERVICE_V3;
        if request
            .protected_grant
            .as_ref()
            .is_some_and(|grant| grant.principal_kind.is_collaboration())
            || (request.protected_grant.is_none()
                && !request
                    .principal
                    .capabilities
                    .iter()
                    .any(|capability| capability == "same-os-user:query-preview"))
        {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(test) = serde_json::from_value::<DecisionServiceTestRequestV2>(request.payload)
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if test.id.is_empty()
            || test.id.len() > 128
            || test.revision == 0
            || CanonicalDigest::parse(test.expected_configuration_digest.as_str()).is_err()
        {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Ok(request_digest) = CanonicalDigest::of(&test) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if let Some(grant) = request.protected_grant.as_ref() {
            let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                Ok(snapshot) => snapshot.revisions,
                Err(error) => return failed(map_control_error(error), request.request_id),
            };
            if let Err(error) = ports.control.validate_protected_capability(
                &grant.capability,
                &WorkspaceId::default(),
                grant.principal_kind,
                &request.operation_id,
                &request_digest,
                &revisions,
            ) {
                return failed(map_control_error(error), request.request_id);
            }
        }
        let Some(routing) = ports.routing.as_deref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let service =
            match routing.decision_service_record(&WorkspaceId::default(), &test.id, test.revision)
            {
                Ok(Some(service)) => service,
                Ok(None) => {
                    return succeeded(
                        rejected(
                            network_aware,
                            test.expected_configuration_digest,
                            DecisionFailureCodeV2::DecisionRevisionConflict,
                        ),
                        request.request_id,
                    );
                }
                Err(error) => return failed(map_control_error(error), request.request_id),
            };
        let Ok(digest) = CanonicalDigest::of(&service) else {
            return failed(ErrorCode::Internal, request.request_id);
        };
        if digest != test.expected_configuration_digest {
            return succeeded(
                rejected(
                    network_aware,
                    digest,
                    DecisionFailureCodeV2::DecisionRevisionConflict,
                ),
                request.request_id,
            );
        }
        if let Some(DecisionConnectionV2::Native {
            authentication:
                DecisionAuthenticationV2::SharedSource {
                    source_id,
                    source_lineage_digest,
                },
            ..
        }) = service.versioned_connection()
        {
            let source = match routing.decision_source_summary(source_id) {
                Ok(source) => source,
                Err(error) => return failed(map_control_error(error), request.request_id),
            };
            let failure = if source.state == DecisionSourceStateV1::Missing {
                Some(DecisionFailureCodeV2::DecisionSourceNotFound)
            } else if source
                .lineage_digest
                .as_ref()
                .is_some_and(|lineage| lineage != source_lineage_digest)
            {
                Some(DecisionFailureCodeV2::DecisionSourceLineageMismatch)
            } else if source.state == DecisionSourceStateV1::Unavailable {
                Some(DecisionFailureCodeV2::DecisionSourceUnavailable)
            } else {
                None
            };
            if let Some(failure) = failure {
                return succeeded(rejected(network_aware, digest, failure), request.request_id);
            }
        }
        let Some(diagnostic) = ports.classifier_diagnostic.as_deref() else {
            return failed(ErrorCode::GatewayUnavailable, request.request_id);
        };
        if network_aware {
            match diagnostic.test_decision_service_v3(&service, &digest) {
                Ok(result) => succeeded(result, request.request_id),
                Err(_) => failed(ErrorCode::GatewayUnavailable, request.request_id),
            }
        } else {
            match diagnostic.test_decision_service_v2(&service, &digest) {
                Ok(result) => succeeded(result, request.request_id),
                Err(_) => failed(ErrorCode::GatewayUnavailable, request.request_id),
            }
        }
    }

    pub(super) fn preview_decision_context(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Ok(input) = serde_json::from_value::<
            hiroute_application_api::DecisionContextPreviewRequestV1,
        >(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if !input.policy.validate()
            || input.endpoint.len() > 2048
            || input.model.is_empty()
            || input.model.len() > 256
        {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(port) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.routing.as_deref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        match port.preview_decision_context(input) {
            Ok(result) => succeeded(result, request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }
}

fn rejected(network_aware: bool, digest: CanonicalDigest, failure: DecisionFailureCodeV2) -> Value {
    let check = DecisionServiceTestResultV2 {
        schema: DecisionServiceTestResultSchemaV2::V2,
        outcome: DecisionTestOutcomeV2::Failed,
        failure_code: Some(failure),
        duration_millis: None,
        tested_configuration_digest: digest,
        tested_credential_state_revision: None,
    };
    if network_aware {
        serde_json::to_value(hiroute_application_api::DecisionServiceTestResultV3 {
            schema: hiroute_application_api::DecisionServiceTestResultSchemaV3::V3,
            check,
            network: None,
            failure: None,
        })
    } else {
        serde_json::to_value(check)
    }
    .expect("safe decision result serialization")
}
