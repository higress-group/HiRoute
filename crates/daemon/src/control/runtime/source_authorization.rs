//! Production adapters for Application's single transaction coordinator.

use hiroute_application::{
    ConnectionOptionAuthorizationPort, ConnectionOptionAuthorizationV1,
    RegisteredComputeSourceMaterializationV1,
};
use hiroute_domain::{
    CredentialPoolIdentityV1, CredentialPoolV1, PortError, PortErrorCode, PortResult,
};

use super::LocalControlAdapter;

impl ConnectionOptionAuthorizationPort for LocalControlAdapter {
    fn connection_option_authorization(
        &self,
        connection_option_id: &str,
    ) -> PortResult<Option<ConnectionOptionAuthorizationV1>> {
        let Some(catalog) = self.release_catalog.as_ref() else {
            return Ok(None);
        };
        let Ok(resolved) = catalog.resolve_connection_option(connection_option_id) else {
            return Ok(None);
        };
        Ok(Some(ConnectionOptionAuthorizationV1 {
            requires_explicit_materialization: resolved
                .option
                .billing_class
                .requires_explicit_materialization(),
            accepts_native_secret: resolved.connector.authentication
                == hiroute_domain::AuthenticationKind::ProviderApiKey,
        }))
    }

    fn source_uses_connection_option(
        &self,
        source_id: &str,
        connection_option_id: &str,
    ) -> PortResult<bool> {
        Ok(self
            .stores_lock()?
            .control()
            .compute_source(source_id)?
            .is_some_and(|source| source.connection_option_id == connection_option_id))
    }

    fn is_registered_compute_source(&self, source_id: &str) -> PortResult<bool> {
        Ok(self
            .stores_lock()?
            .control()
            .compute_source(source_id)?
            .is_some())
    }

    fn is_registered_price_target(
        &self,
        offer_ref: &str,
        model_configuration_id: &str,
        currency: &str,
        target_rule_id: Option<&str>,
    ) -> PortResult<bool> {
        let Some(catalog) = self.release_catalog.as_ref() else {
            return Ok(false);
        };
        let model_data = catalog.model_data();
        let Some(offer) = model_data.offer(offer_ref) else {
            return Ok(false);
        };
        if !offer
            .model_configuration_ids
            .iter()
            .any(|value| value == model_configuration_id)
        {
            return Ok(false);
        }
        Ok(model_data.price_rates.iter().any(|rate| {
            rate.offer_ref == offer_ref
                && rate.model_configuration_id == model_configuration_id
                && rate.currency == currency
                && target_rule_id.is_none_or(|expected| rate.price_rate_id == expected)
        }))
    }

    fn credential_pool_identity(
        &self,
        pool_id: &str,
        binding_id: &str,
    ) -> PortResult<Option<CredentialPoolIdentityV1>> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or_else(|| unavailable("release.catalog.missing"))?;
        let (source, binding) = {
            let stores = self.stores_lock()?;
            let control = stores.control();
            let Some(binding) = control.source_binding(binding_id)? else {
                return Ok(None);
            };
            if binding.credential_pool_id.as_deref() != Some(pool_id) {
                return Ok(None);
            }
            let source = control
                .compute_source(&binding.source_id)?
                .ok_or_else(|| invalid_data("compute.binding.source_missing"))?;
            (source, binding)
        };
        CredentialPoolIdentityV1::for_registered_binding(
            &binding,
            &source,
            catalog.registry(),
            catalog.model_data(),
        )
        .map(Some)
        .map_err(|_| invalid_data("compute.pool.persisted_identity"))
    }

    fn credential_pool(&self, pool_id: &str) -> PortResult<Option<CredentialPoolV1>> {
        self.stores_lock()?.control().credential_pool(pool_id)
    }

    fn credential_reference_count(&self, _credential_id: &str) -> PortResult<u64> {
        // Deletion is not part of the narrow setup path. Returning zero fails delete closed.
        Ok(0)
    }

    fn compute_source_materialization(
        &self,
        connection_option_id: &str,
        source_id: &str,
        expected_revision: u64,
        explicit_materialization: bool,
    ) -> PortResult<Option<RegisteredComputeSourceMaterializationV1>> {
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or_else(|| unavailable("release.catalog.missing"))?;
        for candidate in self
            .registered_compute_candidates()
            .map_err(map_control_read)?
        {
            if candidate.fact.connection_option_id != connection_option_id {
                continue;
            }
            let ids = catalog
                .compute_projection_ids(&candidate)
                .map_err(|_| invalid_data("release.compute.identity"))?;
            if ids.source_id != source_id {
                continue;
            }
            let (expected, current) = {
                let stores = self.stores_lock()?;
                let control = stores.control();
                (
                    control.compute_projection_expectation_with_legacy_lineage(
                        &ids.source_id,
                        &ids.binding_id,
                        &ids.endpoint_profile_id,
                        ids.legacy_v7_source_id(),
                    )?,
                    control.compute_source(source_id)?,
                )
            };
            if expected.source_revision != expected_revision {
                return Ok(None);
            }
            let prepared = catalog
                .prepare_compute_projection(candidate, expected, explicit_materialization)
                .map_err(|_| invalid_data("release.compute.projection"))?;
            return Ok(Some(RegisteredComputeSourceMaterializationV1 {
                current,
                desired: prepared.desired.source,
                registry: catalog.registry().clone(),
                explicit_materialization,
            }));
        }
        for (registered, model_configuration_id) in self
            .registered_cpa_candidates_for_option(connection_option_id)
            .map_err(map_control_read)?
        {
            if registered.source.connection_option_id != connection_option_id
                || registered.source.source_id != source_id
            {
                continue;
            }
            let ids = catalog
                .cpa_compute_projection_ids(&registered, &model_configuration_id)
                .map_err(|_| invalid_data("release.compute.identity"))?;
            let (expected, current) = {
                let stores = self.stores_lock()?;
                let control = stores.control();
                (
                    control.compute_projection_expectation_with_legacy_lineage(
                        ids.source_id(),
                        ids.binding_id(),
                        ids.endpoint_profile_id(),
                        ids.legacy_v7_source_id(),
                    )?,
                    control.compute_source(source_id)?,
                )
            };
            if expected.source_revision != expected_revision {
                return Ok(None);
            }
            let prepared = catalog
                .prepare_cpa_compute_projection(
                    &registered,
                    &model_configuration_id,
                    expected,
                    explicit_materialization,
                )
                .map_err(|_| invalid_data("release.compute.projection"))?;
            return Ok(Some(RegisteredComputeSourceMaterializationV1 {
                current,
                desired: prepared.desired.source,
                registry: catalog.registry().clone(),
                explicit_materialization,
            }));
        }
        Ok(None)
    }
}

fn unavailable(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}

fn invalid_data(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}

fn map_control_read(error: hiroute_application::control::ControlReadError) -> PortError {
    let code = match error {
        hiroute_application::control::ControlReadError::SnapshotChanged => PortErrorCode::Conflict,
        hiroute_application::control::ControlReadError::Unavailable => PortErrorCode::Unavailable,
        hiroute_application::control::ControlReadError::NotFound => PortErrorCode::NotFound,
        hiroute_application::control::ControlReadError::Denied => PortErrorCode::PermissionDenied,
        hiroute_application::control::ControlReadError::Corrupt => PortErrorCode::Corrupt,
    };
    PortError::new(code, "release.compute.discovery")
}
