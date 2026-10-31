//! Immutable request snapshots. Only the Control owner publishes a prepared source update.
use super::*;
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug)]
pub struct NativeNetworkRegistryEntryV1 {
    record: NativeSourceNetworkRecordV1,
    bindings: Vec<String>,
}

impl NativeNetworkRegistryEntryV1 {
    pub fn for_source(
        source: &crate::ComputeManagementSourceV2,
        record: NativeSourceNetworkRecordV1,
    ) -> Result<Self, NativeNetworkErrorV1> {
        record.validate_for(source)?;
        let bindings = source
            .models
            .iter()
            .map(|m| m.binding_id.clone())
            .collect::<Vec<_>>();
        if bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len() {
            return Err(NativeNetworkErrorV1::InvalidState);
        }
        Ok(Self { record, bindings })
    }
}

#[derive(Clone)]
pub struct NativeNetworkSnapshotV1 {
    environment: Arc<Result<NativeProxyEnvironmentV1, NativeNetworkErrorV1>>,
    sources: BTreeMap<String, Arc<NativeNetworkRegistryEntryV1>>,
    bindings: BTreeMap<String, String>,
    unavailable: BTreeSet<String>,
}

impl NativeNetworkSnapshotV1 {
    /// Opaque startup identity; no environment values or hashes cross this boundary.
    pub fn environment_epoch(&self) -> Result<&str, NativeNetworkErrorV1> {
        self.environment
            .as_ref()
            .as_ref()
            .map(|e| e.epoch())
            .map_err(|e| *e)
    }

    pub fn source(
        &self,
        source: &str,
    ) -> Result<&NativeSourceNetworkRecordV1, NativeNetworkErrorV1> {
        if self.unavailable.contains(source) {
            return Err(NativeNetworkErrorV1::InvalidState);
        }
        self.sources
            .get(source)
            .map(|e| &e.record)
            .ok_or(NativeNetworkErrorV1::InvalidState)
    }

    pub fn resolve_source(
        &self,
        source: &str,
        destination: &NativeNetworkDestinationV1,
    ) -> Result<ResolvedNativeNetworkRouteV1, NativeNetworkErrorV1> {
        self.resolve_policy(&self.source(source)?.policy, destination)
    }

    pub fn resolve_binding(
        &self,
        binding: &str,
        destination: &NativeNetworkDestinationV1,
    ) -> Result<(&NativeSourceNetworkRecordV1, ResolvedNativeNetworkRouteV1), NativeNetworkErrorV1>
    {
        let source = self
            .bindings
            .get(binding)
            .ok_or(NativeNetworkErrorV1::InvalidState)?;
        let record = self.source(source)?;
        Ok((record, self.resolve_policy(&record.policy, destination)?))
    }

    pub fn resolve_policy(
        &self,
        policy: &NativeNetworkPolicyV1,
        destination: &NativeNetworkDestinationV1,
    ) -> Result<ResolvedNativeNetworkRouteV1, NativeNetworkErrorV1> {
        match self.environment.as_ref() {
            Ok(environment) => resolve_native_network_route(policy, environment, destination),
            Err(error) if matches!(policy, NativeNetworkPolicyV1::Inherit) => Err(*error),
            Err(_) => resolve_native_network_route(
                policy,
                &NativeProxyEnvironmentV1 {
                    epoch: "unavailable".into(),
                    values: BTreeMap::new(),
                },
                destination,
            ),
        }
    }

    fn set_source(
        &mut self,
        source: &str,
        entry: Option<NativeNetworkRegistryEntryV1>,
    ) -> Result<(), NativeNetworkErrorV1> {
        if let Some(old) = self.sources.remove(source) {
            for binding in &old.bindings {
                self.bindings.remove(binding);
            }
        }
        self.unavailable.remove(source);
        if let Some(entry) = entry {
            if entry.record.source_id != source {
                return Err(NativeNetworkErrorV1::InvalidState);
            }
            for binding in &entry.bindings {
                if self
                    .bindings
                    .insert(binding.clone(), source.into())
                    .is_some()
                {
                    return Err(NativeNetworkErrorV1::InvalidState);
                }
            }
            self.sources.insert(source.into(), Arc::new(entry));
        }
        Ok(())
    }
}

pub struct NativeNetworkRegistryV1(RwLock<Arc<NativeNetworkSnapshotV1>>);

pub struct NativeNetworkPreparedUpdateV1 {
    before: Arc<NativeNetworkSnapshotV1>,
    after: NativeNetworkSnapshotV1,
    source: String,
}

impl NativeNetworkRegistryV1 {
    pub fn new(environment: Result<NativeProxyEnvironmentV1, NativeNetworkErrorV1>) -> Self {
        Self(RwLock::new(Arc::new(NativeNetworkSnapshotV1 {
            environment: Arc::new(environment),
            sources: BTreeMap::new(),
            bindings: BTreeMap::new(),
            unavailable: BTreeSet::new(),
        })))
    }

    /// O(1) admission, independent of current source count and all historical state.
    pub fn pin(&self) -> Result<Arc<NativeNetworkSnapshotV1>, NativeNetworkErrorV1> {
        self.0
            .read()
            .map(|s| Arc::clone(&s))
            .map_err(|_| NativeNetworkErrorV1::InvalidState)
    }

    /// Startup only, before the Gateway begins serving. This does not inspect old Operations.
    pub fn initialize(
        &self,
        entries: impl IntoIterator<Item = NativeNetworkRegistryEntryV1>,
    ) -> Result<(), NativeNetworkErrorV1> {
        let before = self.pin()?;
        let mut next = NativeNetworkSnapshotV1 {
            environment: Arc::clone(&before.environment),
            sources: BTreeMap::new(),
            bindings: BTreeMap::new(),
            unavailable: BTreeSet::new(),
        };
        for entry in entries {
            let id = entry.record.source_id.clone();
            if next.sources.contains_key(&id) {
                return Err(NativeNetworkErrorV1::InvalidState);
            }
            next.set_source(&id, Some(entry))?;
        }
        *self
            .0
            .write()
            .map_err(|_| NativeNetworkErrorV1::InvalidState)? = Arc::new(next);
        Ok(())
    }

    /// Build outside the admission write lock. The current request path only clones its Arc.
    pub fn prepare(
        &self,
        source: &str,
        entry: Option<NativeNetworkRegistryEntryV1>,
    ) -> Result<NativeNetworkPreparedUpdateV1, NativeNetworkErrorV1> {
        let before = self.pin()?;
        let mut after = (*before).clone();
        after.set_source(source, entry)?;
        Ok(NativeNetworkPreparedUpdateV1 {
            before,
            after,
            source: source.into(),
        })
    }

    /// Serialize only the durable activation and pointer publication with new admission.
    /// Already pinned work keeps its Arc. A failed/uncertain activation fences the changed
    /// source until its original Operation or startup recovery reestablishes current state.
    pub fn activate<T>(
        &self,
        update: NativeNetworkPreparedUpdateV1,
        commit: impl FnOnce() -> crate::PortResult<T>,
    ) -> crate::PortResult<T> {
        self.activate_inner(update, || commit().map(|r| (r, None)))
    }

    pub fn activate_reconciled<T>(
        &self,
        update: NativeNetworkPreparedUpdateV1,
        commit: impl FnOnce() -> crate::PortResult<(T, Option<NativeNetworkRegistryEntryV1>)>,
    ) -> crate::PortResult<T> {
        self.activate_inner(update, || commit().map(|(r, entry)| (r, Some(entry))))
    }

    fn activate_inner<T>(
        &self,
        mut update: NativeNetworkPreparedUpdateV1,
        commit: impl FnOnce() -> crate::PortResult<(T, Option<Option<NativeNetworkRegistryEntryV1>>)>,
    ) -> crate::PortResult<T> {
        let mut current = self.0.write().map_err(|_| {
            crate::PortError::new(
                crate::PortErrorCode::Unavailable,
                "native.network.registry_unavailable",
            )
        })?;
        if !Arc::ptr_eq(&current, &update.before) {
            return Err(crate::PortError::new(
                crate::PortErrorCode::Conflict,
                "native.network.registry_conflict",
            ));
        }
        match commit() {
            Ok((result, actual)) => {
                if let Some(actual) = actual
                    && update.after.set_source(&update.source, actual).is_err()
                {
                    update.after.unavailable.insert(update.source);
                    *current = Arc::new(update.after);
                    return Err(crate::PortError::new(
                        crate::PortErrorCode::InvalidData,
                        "native.network.activation_state",
                    ));
                }
                *current = Arc::new(update.after);
                Ok(result)
            }
            Err(error) => {
                update.after.unavailable.insert(update.source);
                *current = Arc::new(update.after);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        source: &str,
        binding: &str,
        policy: NativeNetworkPolicyV1,
        revision: u64,
    ) -> NativeNetworkRegistryEntryV1 {
        NativeNetworkRegistryEntryV1 {
            record: NativeSourceNetworkRecordV1 {
                schema: NATIVE_SOURCE_NETWORK_SCHEMA_V1.into(),
                source_id: source.into(),
                lineage_digest: CanonicalDigest::of_bytes(source.as_bytes()),
                network_revision: revision,
                policy,
            },
            bindings: vec![binding.into()],
        }
    }

    fn registry() -> NativeNetworkRegistryV1 {
        NativeNetworkRegistryV1::new(NativeProxyEnvironmentV1::from_variables("epoch".into(), []))
    }

    #[test]
    fn admitted_work_keeps_old_route_and_new_work_sees_only_activated_policy() {
        let registry = registry();
        registry
            .initialize([
                entry("a", "binding/a", NativeNetworkPolicyV1::Direct, 1),
                entry("b", "binding/b", NativeNetworkPolicyV1::Direct, 1),
            ])
            .unwrap();
        let admitted = registry.pin().unwrap();
        let policy = NativeNetworkPolicyV1::Manual {
            proxy_url: "http://proxy.example:8080/".into(),
            no_proxy: "".into(),
        };
        let update = registry
            .prepare("a", Some(entry("a", "binding/a", policy, 2)))
            .unwrap();
        let before_commit = registry.pin().unwrap();
        assert!(Arc::ptr_eq(&admitted, &before_commit));
        registry.activate(update, || Ok(())).unwrap();
        let destination = NativeNetworkDestinationV1::from_url("https://api.example").unwrap();
        assert!(
            admitted
                .resolve_binding("binding/a", &destination)
                .unwrap()
                .1
                .proxy
                .is_none()
        );
        assert!(
            registry
                .pin()
                .unwrap()
                .resolve_binding("binding/a", &destination)
                .unwrap()
                .1
                .proxy
                .is_some()
        );
        assert!(
            registry
                .pin()
                .unwrap()
                .resolve_binding("binding/b", &destination)
                .unwrap()
                .1
                .proxy
                .is_none()
        );
    }

    #[test]
    fn uncertain_commit_fences_only_changed_source_and_never_rolls_live_snapshot_back() {
        let registry = registry();
        registry
            .initialize([
                entry("a", "binding/a", NativeNetworkPolicyV1::Direct, 1),
                entry("b", "binding/b", NativeNetworkPolicyV1::Direct, 1),
            ])
            .unwrap();
        let admitted = registry.pin().unwrap();
        let stale = registry
            .prepare(
                "a",
                Some(entry("a", "binding/a", NativeNetworkPolicyV1::Direct, 1)),
            )
            .unwrap();
        let update = registry
            .prepare(
                "a",
                Some(entry("a", "binding/a", NativeNetworkPolicyV1::Inherit, 2)),
            )
            .unwrap();
        assert!(
            registry
                .activate(update, || -> crate::PortResult<()> {
                    Err(crate::PortError::new(
                        crate::PortErrorCode::Unavailable,
                        "test.uncertain",
                    ))
                })
                .is_err()
        );
        assert!(registry.pin().unwrap().source("a").is_err());
        assert!(registry.pin().unwrap().source("b").is_ok());
        assert!(admitted.source("a").is_ok());
        let called = std::cell::Cell::new(false);
        assert!(
            registry
                .activate(stale, || {
                    called.set(true);
                    Ok(())
                })
                .is_err()
        );
        assert!(!called.get());
    }

    #[test]
    fn pinning_many_sources_clones_the_same_snapshot_without_visiting_them() {
        let registry = registry();
        registry
            .initialize((0..10000).map(|i| {
                entry(
                    &format!("source/{i}"),
                    &format!("binding/{i}"),
                    NativeNetworkPolicyV1::Direct,
                    1,
                )
            }))
            .unwrap();
        let first = registry.pin().unwrap();
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&first, &registry.pin().unwrap()));
        }
        assert_eq!(first.source("source/9999").unwrap().network_revision, 1);
    }
}
