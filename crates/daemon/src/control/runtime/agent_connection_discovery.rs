use hiroute_application::control::ControlReadError;
use hiroute_domain::{ActiveAgentConnectionV1, WorkspaceId};
use hiroute_integrations::{AgentDiscoveryOutcomeV1, FilesystemAgentDiscoveryV1};

use super::{APPLY_OPERATION, LocalControlAdapter};

impl LocalControlAdapter {
    pub(super) fn latest_connection_operation(
        &self,
        connection_id: &str,
    ) -> Result<
        (
            ActiveAgentConnectionV1,
            hiroute_local_storage::SucceededAgentOperationV1,
        ),
        ControlReadError,
    > {
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        for operation in stores
            .control()
            .succeeded_agent_operations_for_kind(&WorkspaceId::default(), APPLY_OPERATION)
            .map_err(super::super::map_port)?
        {
            let projection = operation
                .plan
                .agent_connection_projection()
                .map_err(|_| ControlReadError::Corrupt)?;
            if let Some(projection) = projection
                && projection.connection_id == connection_id
            {
                return Ok((projection, operation));
            }
        }
        Err(ControlReadError::NotFound)
    }

    pub(super) fn exact_discovery(
        &self,
        agent_id: &str,
        profile_id: &str,
        _installed_version: &str,
    ) -> Result<FilesystemAgentDiscoveryV1, ControlReadError> {
        let mut matches =
            self.scanner
                .scan()
                .into_iter()
                .filter(|discovery| match &discovery.outcome {
                    AgentDiscoveryOutcomeV1::Supported { installation } => {
                        installation.agent_id == agent_id
                            && installation.profile.profile_id == profile_id
                    }
                    AgentDiscoveryOutcomeV1::ReportOnly { .. } => false,
                });
        let exact = matches.next().ok_or(ControlReadError::NotFound)?;
        if matches.next().is_some() {
            return Err(ControlReadError::Corrupt);
        }
        Ok(exact)
    }

    pub(super) fn latest_connection(
        &self,
        connection_id: &str,
    ) -> Result<ActiveAgentConnectionV1, ControlReadError> {
        self.latest_connection_operation(connection_id)
            .map(|(connection, _)| connection)
    }
}
