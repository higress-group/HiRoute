//! Selected scan scopes are private reread locators, never write authority or native environment.
use super::*;
use hiroute_application::control::ControlReadError;
use hiroute_domain::AgentNativeTargetV1;
use hiroute_integrations::{HermesApiDiscoveryV1, HermesSourceDiagnosticV1, NativeApiSource};

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn remember_hermes_scan_target(
        &self,
        target: &AgentNativeTargetV1,
    ) -> Result<(), ControlReadError> {
        let canonical = hiroute_integrations::canonical_hermes_target(target)
            .map_err(|_| ControlReadError::Denied)?;
        let id = hiroute_integrations::hermes_context_id(canonical.config_root());
        let mut targets = self
            .hermes_scan_targets
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        if targets.len() >= 128 && !targets.contains_key(&id) {
            return Err(ControlReadError::Unavailable);
        }
        targets.insert(id, canonical);
        Ok(())
    }

    pub(in crate::control::runtime) fn hermes_source_facts(
        &self,
    ) -> Result<HermesApiDiscoveryV1, ControlReadError> {
        let targets = self
            .hermes_scan_targets
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone();
        let mut facts = HermesApiDiscoveryV1 {
            sources: Vec::new(),
            diagnostics: Vec::new(),
        };
        for target in targets.values() {
            match hiroute_integrations::hermes_api_sources(target) {
                Ok(mut source) => {
                    facts.sources.append(&mut source.sources);
                    facts.diagnostics.append(&mut source.diagnostics);
                }
                Err(_) => facts.diagnostics.push(HermesSourceDiagnosticV1 {
                    provider_id: None,
                    model_id: None,
                    reason: "selected_profile_configuration_unavailable",
                }),
            }
        }
        Ok(facts)
    }

    pub(in crate::control::runtime) fn read_hermes_source(
        &self,
        expected: &NativeApiSource,
    ) -> Result<(zeroize::Zeroizing<String>, ProtectedSecret), ControlReadError> {
        let targets = self
            .hermes_scan_targets
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone();
        let scope = expected
            .point_binding
            .as_ref()
            .ok_or(ControlReadError::SnapshotChanged)?
            .native_scope
            .clone();
        let target = targets
            .values()
            .find(|target| {
                hiroute_domain::CanonicalDigest::of_bytes(target.config_root().as_bytes()) == scope
            })
            .ok_or(ControlReadError::SnapshotChanged)?;
        hiroute_integrations::read_hermes_api_source(target, expected)
            .map_err(|_| ControlReadError::SnapshotChanged)
    }
}
