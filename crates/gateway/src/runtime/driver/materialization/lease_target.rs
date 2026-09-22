use std::sync::Arc;

pub(super) fn for_request(
    execution: &crate::server::publication::SealedCandidateExecutionV1,
    request_path: &str,
) -> Result<hiroute_domain::GatewayOperationalTargetV1, Arc<str>> {
    if execution.connector_runtime == hiroute_domain::ConnectorRuntimeKind::BuiltinNative {
        execution
            .operational_target
            .for_protocol_path(request_path)
            .ok_or_else(|| Arc::from(super::super::MATERIALIZATION_PROTOCOL_FAILED))
    } else {
        Ok(execution.operational_target.clone())
    }
}
