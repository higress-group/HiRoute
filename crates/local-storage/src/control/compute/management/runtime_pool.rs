//! Current managed Native membership. This never reads or decrypts Secret material.

use hiroute_diagnostics::event::NativeCredentialLookupStage as Stage;
use hiroute_domain::{
    ComputeManagementRepositoryPort, MaterializationState, NativeCredentialAccessDeniedV1,
    NativeCredentialLeaseErrorV1, NativeCredentialLeaseRequestV1, PortError, PortErrorCode,
    WorkspaceId,
};

impl crate::LocalStorageSet {
    pub fn current_native_credential_refs(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> Result<Option<Vec<String>>, NativeCredentialLeaseErrorV1> {
        request.validate().map_err(|_| rejected())?;
        // One transaction binds source state, models, members and ordinal together.
        let snapshot = self
            .control()
            .compute_management_snapshot(&WorkspaceId::default())?;
        let mut owners = snapshot.sources.iter().filter(|source| {
            source
                .models
                .iter()
                .any(|model| model.binding_id == request.stable_binding_id)
        });
        let Some(source) = owners.next() else {
            // Namespace classification only fails closed; it never grants access.
            let managed = request.stable_binding_id.starts_with("binding/managed-")
                || request.credential_id.starts_with("credential/managed-");
            if managed {
                self.control()
                    .native_credential_unavailable(Stage::PoolBindingMissing);
            }
            return Ok(managed.then(Vec::new));
        };
        if owners.next().is_some() || !source.provenance.is_native() {
            return Err(rejected().into());
        }
        self.control()
            .validate_current_key_budgets(&source.source_id)?;
        let model = source
            .models
            .iter()
            .find(|model| model.binding_id == request.stable_binding_id)
            .ok_or_else(rejected)?;
        if model.upstream_model_id != request.upstream_model_id {
            return Err(rejected().into());
        }
        let target_matches = std::iter::once((&source.target, &source.authentication))
            .chain(
                source
                    .additional_native_endpoints
                    .iter()
                    .map(|endpoint| (&endpoint.target, &endpoint.authentication)),
            )
            .any(|(target, authentication)| {
                *authentication == request.authentication
                    && target
                        .credential_destination()
                        .is_ok_and(|destination| destination == request.credential_destination_ref)
                    && request
                        .logical_endpoint
                        .strip_suffix(&request.request_path)
                        .is_some_and(|base| {
                            let host = if target.authority.contains(':')
                                && !target.authority.starts_with('[')
                            {
                                format!("[{}]", target.authority)
                            } else {
                                target.authority.clone()
                            };
                            base.eq_ignore_ascii_case(&format!(
                                "{}://{}:{}",
                                target.scheme, host, target.port
                            )) || ((target.scheme.eq_ignore_ascii_case("https")
                                && target.port == 443
                                || target.scheme.eq_ignore_ascii_case("http") && target.port == 80)
                                && base
                                    .eq_ignore_ascii_case(&format!("{}://{}", target.scheme, host)))
                        })
            });
        if !target_matches {
            return Err(rejected().into());
        }
        if source.state == MaterializationState::Disabled {
            return Err(NativeCredentialLeaseErrorV1::Denied(
                NativeCredentialAccessDeniedV1::SourceDisabled,
            ));
        }
        if source.state != MaterializationState::Ready {
            self.control()
                .native_credential_unavailable(Stage::PoolSourceNotReady);
            return Ok(Some(Vec::new()));
        }
        let destinations = source.native_destinations().map_err(|_| rejected())?;
        let mut credentials: Vec<_> = source
            .credentials
            .iter()
            .filter(|key| key.enabled)
            .collect();
        credentials.sort_by_key(|key| key.ordinal);
        if credentials.is_empty() {
            self.control()
                .native_credential_unavailable(Stage::PoolEmpty);
        }
        for key in &credentials {
            let reference = &key.credential;
            if reference.owner_scope() != format!("source/{}", source.source_id)
                || reference.subject() != "hirouted"
                || reference.purpose() != "provider-auth"
                || reference.allowed_destinations() != &destinations
            {
                return Err(rejected().into());
            }
        }
        Ok(Some(
            credentials
                .into_iter()
                .map(|key| key.credential.credential_id().to_owned())
                .collect(),
        ))
    }
}

fn rejected() -> PortError {
    PortError::new(
        PortErrorCode::PermissionDenied,
        "compute.management.pool.binding",
    )
}
