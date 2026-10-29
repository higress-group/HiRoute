//! Versioned decision reads and saved-revision diagnostics.
use super::*;
use hiroute_application_api::{
    ClientEmptyRequestV1, DecisionFailureCodeV2, DecisionServiceTestRequestV2,
    DecisionServiceTestResultSchemaV2, DecisionServiceTestResultV2, DecisionSourceStateV1,
    DecisionTestOutcomeV2,
};
use hiroute_domain::{DecisionAuthenticationV2, DecisionConnectionV2, DecisionServiceRecord};

impl ApplicationService {
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
        } else {
            match port.decision_services_v2(&WorkspaceId::default()) {
                Ok(result) => succeeded(result, request.request_id),
                Err(error) => failed(map_control_error(error), request.request_id),
            }
        }
    }

    pub(super) fn test_decision_v2(
        &self,
        request: LocalControlRequestV2,
    ) -> MachineEnvelopeV2<Value> {
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
                "TestDecisionServiceV2",
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
                rejected(digest, DecisionFailureCodeV2::DecisionRevisionConflict),
                request.request_id,
            );
        }
        if let DecisionServiceRecord::V2(service) = &service
            && let DecisionConnectionV2::Native {
                authentication:
                    DecisionAuthenticationV2::SharedSource {
                        source_id,
                        source_lineage_digest,
                    },
                ..
            } = &service.connection
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
                return succeeded(rejected(digest, failure), request.request_id);
            }
        }
        let Some(diagnostic) = ports.classifier_diagnostic.as_deref() else {
            return failed(ErrorCode::GatewayUnavailable, request.request_id);
        };
        match diagnostic.test_decision_service_v2(&service, &digest) {
            Ok(result) => succeeded(result, request.request_id),
            Err(_) => failed(ErrorCode::GatewayUnavailable, request.request_id),
        }
    }
}

fn rejected(
    digest: CanonicalDigest,
    failure: DecisionFailureCodeV2,
) -> DecisionServiceTestResultV2 {
    DecisionServiceTestResultV2 {
        schema: DecisionServiceTestResultSchemaV2::V2,
        outcome: DecisionTestOutcomeV2::Failed,
        failure_code: Some(failure),
        duration_millis: None,
        tested_configuration_digest: digest,
        tested_credential_state_revision: None,
    }
}
