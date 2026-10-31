use super::*;
use hiroute_domain::{
    ComputeManagementMutation, ComputeManagementRepositoryPort, NativeNetworkErrorV1,
    NativeNetworkRegistryEntryV1, NativeNetworkRegistryV1, NativeProxyEnvironmentV1, OwnedEffectV1,
};

pub(super) fn capture() -> Result<NativeProxyEnvironmentV1, NativeNetworkErrorV1> {
    let environment = hiroute_host_runtime::ServiceProxyEnvironment::capture(std::env::vars_os())
        .map_err(|_| NativeNetworkErrorV1::InvalidEnvironment)?;
    let epoch = delegation_epoch().map_err(|_| NativeNetworkErrorV1::InvalidEnvironment)?;
    NativeProxyEnvironmentV1::from_variables(
        format!("native-network/{epoch}"),
        environment.variables().map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        }),
    )
}

impl LocalControlAdapter {
    pub(super) fn initialize_native_network(&self) -> Result<(), String> {
        let stores = self.stores_lock().map_err(|e| e.to_string())?;
        let snapshot = stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for source in snapshot
            .sources
            .into_iter()
            .filter(|s| s.provenance.is_native())
        {
            let record = stores
                .control()
                .native_source_network(&source.source_id)
                .map_err(|e| e.to_string())?
                .ok_or("native.network.policy_missing")?;
            entries.push(
                NativeNetworkRegistryEntryV1::for_source(&source, record)
                    .map_err(|e| e.to_string())?,
            );
        }
        self.native_network
            .initialize(entries)
            .map_err(|e| e.to_string())
    }

    /// The existing Control effect remains the only writer. Pinning requests cannot observe
    /// the commit-to-publication interval, and only this source is refreshed after the effect.
    pub(super) fn with_native_network_effect<T>(
        &self,
        effect: &OwnedEffectV1,
        apply: impl FnOnce(&hiroute_local_storage::ControlStore) -> PortResult<T>,
    ) -> PortResult<T> {
        let stores = self.stores_lock()?;
        let control = stores.control();
        let id = effect
            .compensation
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| OperationId::parse(id).ok());
        let mutation = id
            .map(|id| control.load_operation(&id))
            .transpose()?
            .flatten()
            .and_then(|op| {
                op.plan
                    .control()
                    .get("compute_management_mutation")
                    .cloned()
            })
            .map(serde_json::from_value::<ComputeManagementMutation>)
            .transpose()
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "native.network.effect"))?;
        let Some(mutation) = mutation else {
            return apply(control);
        };
        let source_id = mutation.source_id();
        let update = self
            .native_network
            .prepare(source_id, None)
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "native.network.prepare"))?;
        self.native_network.activate_reconciled(update, || {
            let result = apply(control)?;
            let entry = match control.compute_management_source(source_id)? {
                Some(source) if source.provenance.is_native() => {
                    let record = control.native_source_network(source_id)?.ok_or_else(|| {
                        PortError::new(PortErrorCode::InvalidData, "native.network.policy_missing")
                    })?;
                    Some(
                        NativeNetworkRegistryEntryV1::for_source(&source, record).map_err(
                            |_| {
                                PortError::new(
                                    PortErrorCode::InvalidData,
                                    "native.network.activation",
                                )
                            },
                        )?,
                    )
                }
                _ => None,
            };
            Ok((result, entry))
        })
    }
}

impl ProductionControlRuntime {
    pub(crate) fn native_network_registry(&self) -> Arc<NativeNetworkRegistryV1> {
        Arc::clone(&self.adapter.native_network)
    }
}
