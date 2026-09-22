//! Registered local Agent discovery with diagnostic-only executable versions.
//!
//! The public result contains only registered non-secret facts and an opaque discovery
//! descriptor. Secret bytes stay in a zeroizing value and can be reread only through the exact
//! descriptor during Preview/Apply.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hiroute_domain::{
    AgentConfigDocumentV1, AgentKindV1, CLAUDE_CODE_MANAGED_LAUNCH_VERSION_V1, CanonicalDigest,
    ConfigLayerV1, ProtectedSecret, SupportedAgentInstallationV1,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use zeroize::Zeroizing;

use super::executable::{ExecutableObservationV1, ExecutableProbe, executable_probe};
use super::filesystem_config::{
    CLAUDE_AUTH_ENVIRONMENT_FIELDS, ClaudeSettingsSubset, ClaudeSource, ObservedClaudeSettings,
    claude_user_change_is_applied, managed_settings_paths, render_claude_user_change,
    same_layer_secret_conflict,
};
use super::managed_launch::ManagedClaudeLaunchPreflightV1;
use super::registration::ClaudeRegistrationIndexV1;
use super::{
    AGENT_SCAN_OBSERVATION_SCHEMA_V1, AgentDiscoveryOutcomeV1, AgentReportOnlyReasonV1,
    AgentScanObservationV1, ConfigObservationV1, resolve_agent_observation,
};

#[path = "filesystem/claude_settings.rs"]
mod claude_settings;
#[path = "claude_observation.rs"]
mod main_observation;
#[path = "settings_discovery.rs"]
mod settings_discovery;

pub const FILESYSTEM_AGENT_SCANNER_ID_V1: &str = "builtin.agent-filesystem";
pub const FILESYSTEM_AGENT_SCANNER_VERSION_V1: &str = "1";

/// Opaque, non-secret source descriptor safe for Preview output and change-digest binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCredentialRefV1 {
    pub source: String,
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub field_selector: String,
    pub observed_revision: u64,
}

/// Registry-projected Claude configuration. The URL and model are emitted only after exact
/// Connector Registry and ModelData matching.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredClaudeConfigurationV1 {
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_configuration_id: Option<String>,
    pub base_url: String,
    pub observed_model_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_model_alias_hints: BTreeMap<String, String>,
    pub configuration_revision: u64,
}

/// Content-free remediation fact for an owner file whose group/other bits are too broad. The
/// source reference is resolved only inside the scanner; no filesystem path or file bytes cross
/// the Local Control boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionHardeningRequiredV1 {
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub observed_identity: CanonicalDigest,
    pub observed_revision: u64,
    pub display_path: String,
    pub required_mode: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionHardeningOutcomeV1 {
    Hardened,
    AlreadyHardened,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemAgentDiscoveryV1 {
    pub outcome: AgentDiscoveryOutcomeV1,
    /// Independent, redacted configuration facet. Executable trust remains authoritative for
    /// `outcome`; a safe read-only configuration can still be recognized when execution is
    /// unavailable or untrusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_issue: Option<AgentReportOnlyReasonV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_configuration: Option<RegisteredClaudeConfigurationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_credential: Option<DiscoveredCredentialRefV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_hardening: Option<PermissionHardeningRequiredV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_launch: Option<ManagedClaudeLaunchPreflightV1>,
}

/// Exact local paths and the three explicitly registered process variables. This value is not
/// serializable or Debug so a process token cannot accidentally enter diagnostics.
#[derive(Clone)]
pub struct AgentFilesystemLayoutV1 {
    /// Separately installed CLI target. Desktop checks never substitute this path for the
    /// application-owned engine selected by the native host.
    pub codex_executable: PathBuf,
    /// Codex Desktop's application-owned engine, supplied by the native host after resolving the
    /// installed bundle through the operating system. `None` means that surface is unavailable.
    pub codex_desktop_executable: Option<PathBuf>,
    pub claude_executable: PathBuf,
    pub codex_user_config: PathBuf,
    pub claude_launch_settings: Option<PathBuf>,
    pub claude_project_settings: Vec<PathBuf>,
    pub claude_user_settings: PathBuf,
    pub claude_managed_settings: Vec<PathBuf>,
    process_environment: BTreeMap<String, Zeroizing<String>>,
    process_environment_presence: BTreeSet<String>,
}

impl AgentFilesystemLayoutV1 {
    pub fn from_process(home: &Path, project: &Path) -> Self {
        let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        let process_environment = [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
        ]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_owned(), Zeroizing::new(value)))
        })
        .collect();
        let process_environment_presence = hiroute_domain::MANAGED_CLAUDE_ENVIRONMENT_REMOVALS_V1
            .into_iter()
            .filter(|name| std::env::var_os(name).is_some())
            .map(str::to_owned)
            .collect();
        Self {
            codex_executable: PathBuf::from("codex"),
            codex_desktop_executable: None,
            claude_executable: PathBuf::from("claude"),
            codex_user_config: codex_config_path(home, std::env::var_os("CODEX_HOME").as_deref()),
            claude_launch_settings: None,
            claude_project_settings: vec![
                project.join(".claude/settings.local.json"),
                project.join(".claude/settings.json"),
            ],
            claude_user_settings: claude_home.join("settings.json"),
            claude_managed_settings: managed_settings_paths(),
            process_environment,
            process_environment_presence,
        }
    }

    #[cfg(test)]
    fn with_process_value(mut self, name: &str, value: &str) -> Self {
        self.process_environment
            .insert(name.to_owned(), Zeroizing::new(value.to_owned()));
        self.process_environment_presence.insert(name.to_owned());
        self
    }
}

#[derive(Clone)]
pub struct FilesystemAgentScannerV1 {
    pub(super) layout: AgentFilesystemLayoutV1,
    registry: ClaudeRegistrationIndexV1,
    #[cfg(unix)]
    codex_ingress: std::sync::Arc<std::sync::Mutex<Option<super::CodexIngressEvidence>>>,
    #[cfg(unix)]
    claude_ingress: std::sync::Arc<std::sync::Mutex<Option<super::ClaudeIngressEvidence>>>,
}

impl FilesystemAgentScannerV1 {
    fn located_codex_engine(&self) -> Result<Option<PathBuf>, super::executable::ProbeFailure> {
        let mut first_failure = None;
        for candidate in self
            .layout
            .codex_desktop_executable
            .iter()
            .chain(std::iter::once(&self.layout.codex_executable))
        {
            match super::executable::resolve(candidate) {
                Ok(Some(path)) => return Ok(Some(path)),
                Ok(None) => {}
                Err(error) => {
                    first_failure.get_or_insert(error);
                }
            };
        }
        match first_failure {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    pub(super) fn claude_observations(
        &self,
    ) -> Result<Vec<ObservedClaudeSettings>, AgentFilesystemScanError> {
        let observations = self
            .claude_sources()
            .iter()
            .map(|source| {
                source.read(
                    &self.layout.process_environment,
                    &self.layout.process_environment_presence,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(main_observation::resolve_project_local_fields(
            &self.layout,
            &observations,
        ))
    }

    pub fn new(layout: AgentFilesystemLayoutV1, registry: ClaudeRegistrationIndexV1) -> Self {
        Self {
            layout,
            registry,
            #[cfg(unix)]
            codex_ingress: Default::default(),
            #[cfg(unix)]
            claude_ingress: Default::default(),
        }
    }

    /// Attach backend evidence from an explicit native check whose file effect was restored.
    /// Scanning remains read-only; an expired/changed binary automatically loses this evidence.
    #[cfg(unix)]
    pub fn with_codex_ingress_evidence(mut self, evidence: super::CodexIngressEvidence) -> Self {
        self.codex_ingress = std::sync::Arc::new(std::sync::Mutex::new(Some(evidence)));
        self
    }

    /// Only an explicitly admitted check calls this; discovery never performs the challenge.
    #[cfg(unix)]
    pub fn check_codex_native_authentication(&self) -> Result<(), super::NativeIngressProbeError> {
        let engine = self
            .located_codex_engine()
            .map_err(|_| super::native_ingress_probe::fail("Codex engine location"))?
            .ok_or_else(|| super::native_ingress_probe::fail("Codex engine location"))?;
        let evidence = super::CodexNativeIngressProbe::bind()?.run_ephemeral(&engine)?;
        let mut cache = self
            .codex_ingress
            .lock()
            .map_err(|_| super::native_ingress_probe::cache_error())?;
        *cache = Some(evidence);
        Ok(())
    }

    /// Only an explicitly admitted check calls this; discovery never launches Claude.
    #[cfg(unix)]
    pub fn check_claude_native_authentication(&self) -> Result<(), super::NativeIngressProbeError> {
        let evidence = super::ClaudeNativeIngressProbe::run(&self.layout.claude_executable)?;
        let mut cache = self
            .claude_ingress
            .lock()
            .map_err(|_| super::native_ingress_probe::cache_error())?;
        *cache = Some(evidence);
        Ok(())
    }

    #[cfg(unix)]
    pub fn check_codex_collaboration(
        &self,
        cli: &std::path::Path,
    ) -> Result<(), super::NativeIngressProbeError> {
        let evidence = super::CodexNativeIngressProbe::bind()?
            .run_collaboration(&self.layout.codex_executable, cli)?;
        *self
            .codex_ingress
            .lock()
            .map_err(|_| super::native_ingress_probe::cache_error())? = Some(evidence);
        Ok(())
    }

    #[cfg(unix)]
    pub fn check_claude_collaboration(
        &self,
        cli: &std::path::Path,
    ) -> Result<(), super::NativeIngressProbeError> {
        let evidence = super::ClaudeNativeIngressProbe::run_collaboration(
            &self.layout.claude_executable,
            cli,
        )?;
        *self
            .claude_ingress
            .lock()
            .map_err(|_| super::native_ingress_probe::cache_error())? = Some(evidence);
        Ok(())
    }

    #[cfg(unix)]
    pub fn attach_claude_collaboration_evidence(
        &self,
        executable: &std::path::Path,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if let Ok(cache) = self.claude_ingress.lock()
            && let Some(evidence) = cache.as_ref()
        {
            evidence.attach_collaboration(executable, installation);
        }
    }

    pub fn scan(&self) -> Vec<FilesystemAgentDiscoveryV1> {
        let mut results = vec![self.scan_codex()];
        match executable_probe(&self.layout.claude_executable) {
            ExecutableProbe::Installed(executable) => {
                results.push(self.scan_claude(Some(executable), None, false));
            }
            other => {
                let report = probe_report("agent_claude_default", AgentKindV1::ClaudeCode, other);
                results.push(self.scan_claude(None, Some(report.outcome), false));
            }
        }
        results.sort_by(|left, right| {
            outcome_agent_id(&left.outcome).cmp(outcome_agent_id(&right.outcome))
        });
        for result in &mut results {
            if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut result.outcome {
                let binary = match installation.profile.kind {
                    AgentKindV1::Codex => {
                        super::observed_capabilities::attach_target_file_capabilities(
                            installation,
                            &self.layout.codex_user_config,
                        );
                        self.located_codex_engine()
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| self.layout.codex_executable.clone())
                    }
                    AgentKindV1::ClaudeCode => {
                        super::observed_capabilities::attach_file_capabilities(
                            installation,
                            &self.layout.claude_executable,
                            &self.layout.claude_user_settings,
                        );
                        self.layout.claude_executable.clone()
                    }
                };
                #[cfg(unix)]
                if installation.profile.kind == AgentKindV1::Codex
                    && let Ok(cache) = self.codex_ingress.lock()
                    && let Some(evidence) = cache.as_ref()
                {
                    evidence.attach(installation);
                }
                #[cfg(unix)]
                if installation.profile.kind == AgentKindV1::ClaudeCode
                    && let Ok(cache) = self.claude_ingress.lock()
                    && let Some(evidence) = cache.as_ref()
                {
                    evidence.attach(&binary, installation);
                }
            }
        }
        results
    }

    fn scan_codex(&self) -> FilesystemAgentDiscoveryV1 {
        match self.located_codex_engine() {
            Ok(Some(_)) => {
                let mut observation = base_observation(
                    "agent_codex_default",
                    AgentKindV1::Codex,
                    "not-probed".to_owned(),
                );
                match super::sample_codex_configuration(&super::CodexConfigurationScope::user_file(
                    self.layout.codex_user_config.clone(),
                )) {
                    Ok(sample) => {
                        observation.config = sample.observations;
                        FilesystemAgentDiscoveryV1 {
                            outcome: resolve_agent_observation(observation),
                            configuration_issue: None,
                            claude_configuration: None,
                            discovered_credential: None,
                            permission_hardening: None,
                            managed_launch: None,
                        }
                    }
                    Err(error) => report_only_agent(
                        "agent_codex_default",
                        AgentKindV1::Codex,
                        "not-probed".to_owned(),
                        error.report_reason(),
                    ),
                }
            }
            Ok(None) => probe_report(
                "agent_codex_default",
                AgentKindV1::Codex,
                ExecutableProbe::NotFound,
            ),
            Err(reason) => probe_report(
                "agent_codex_default",
                AgentKindV1::Codex,
                ExecutableProbe::Unknown(reason),
            ),
        }
    }

    pub fn read_discovered_secret(
        &self,
        descriptor: &DiscoveredCredentialRefV1,
    ) -> Result<ProtectedSecret, AgentFilesystemScanError> {
        validate_descriptor(descriptor)?;
        let source = self
            .claude_sources()
            .into_iter()
            .find(|source| source.source_ref() == descriptor.discovered_source_ref)
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let (token, revision) = source.read_auth_field(
            descriptor
                .field_selector
                .strip_prefix("env.")
                .ok_or(AgentFilesystemScanError::InvalidDescriptor)?,
            &self.layout.process_environment,
            &self.layout.process_environment_presence,
        )?;
        if revision != descriptor.observed_revision {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        ProtectedSecret::new(token.as_bytes().to_vec())
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)
    }

    /// Applies only the scanner-issued 0600 remediation. The implementation opens with
    /// `O_NOFOLLOW`, rechecks the exact regular-file identity on the descriptor, and never reads
    /// or writes file content.
    pub fn harden_discovered_permissions(
        &self,
        finding: &PermissionHardeningRequiredV1,
    ) -> Result<PermissionHardeningOutcomeV1, AgentFilesystemScanError> {
        validate_hardening_finding(finding)?;
        let source = self
            .claude_sources()
            .into_iter()
            .find(|source| source.source_ref() == finding.discovered_source_ref)
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        source.harden_permissions(finding)
    }

    /// Exact native Codex target selected by this scanner, including CODEX_HOME resolution.
    /// Backend composition uses this path; public requests cannot choose an arbitrary target.
    pub fn codex_user_config_target(&self) -> PathBuf {
        self.layout.codex_user_config.clone()
    }

    /// Exact engine selected for one Codex surface. This is target location only: callers must
    /// never turn a version, digest, or installation proof into product admission.
    pub fn codex_engine_target(
        &self,
        surface: hiroute_domain::AgentModelSurfaceV2,
    ) -> Option<PathBuf> {
        match surface {
            hiroute_domain::AgentModelSurfaceV2::CodexCli => {
                Some(self.layout.codex_executable.clone())
            }
            hiroute_domain::AgentModelSurfaceV2::CodexDesktop => {
                self.layout.codex_desktop_executable.clone()
            }
            hiroute_domain::AgentModelSurfaceV2::ClaudeCli => None,
        }
    }

    /// Actual independently launchable surfaces. This is a location fact only; it never runs a
    /// version, digest, identity, or installation admission probe.
    pub fn available_model_surfaces(
        &self,
        agent_id: &str,
    ) -> BTreeSet<hiroute_domain::AgentModelSurfaceV2> {
        let available = |path: &Path| matches!(super::executable::resolve(path), Ok(Some(_)));
        match agent_id {
            "agent_codex_default" => {
                let mut surfaces = BTreeSet::new();
                if self
                    .layout
                    .codex_desktop_executable
                    .as_deref()
                    .is_some_and(available)
                {
                    surfaces.insert(hiroute_domain::AgentModelSurfaceV2::CodexDesktop);
                }
                if available(&self.layout.codex_executable) {
                    surfaces.insert(hiroute_domain::AgentModelSurfaceV2::CodexCli);
                }
                surfaces
            }
            "agent_claude_default" if available(&self.layout.claude_executable) => {
                [hiroute_domain::AgentModelSurfaceV2::ClaudeCli]
                    .into_iter()
                    .collect()
            }
            _ => BTreeSet::new(),
        }
    }

    pub fn codex_catalog_summary(
        &self,
    ) -> Result<super::CodexCatalogSummaryV1, super::CodexCatalogError> {
        let scope =
            super::CodexConfigurationScope::user_file(self.layout.codex_user_config.clone());
        let explicit_model = super::sample_codex_configuration(&scope)
            .map_err(|_| super::CodexCatalogError::InvalidCatalog)?
            .source_document
            .get("model")
            .and_then(toml_edit::Item::as_str)
            .map(str::to_owned);
        super::sample_codex_catalog_summary(&scope, explicit_model.as_deref())
    }

    /// Returns the exact user-level native target owned by the Claude integration adapter.
    /// The path never crosses a public wire contract; Local Control uses it only for the
    /// recoverable external effect selected by the registered Claude profile.
    pub fn claude_user_settings_target(&self) -> PathBuf {
        self.layout.claude_user_settings.clone()
    }

    fn scan_claude(
        &self,
        executable: Option<ExecutableObservationV1>,
        executable_outcome: Option<AgentDiscoveryOutcomeV1>,
        for_settings: bool,
    ) -> FilesystemAgentDiscoveryV1 {
        let version = executable
            .as_ref()
            .map(|value| value.version.clone())
            .unwrap_or_default();
        // Settings writes bind the actual executable and target file, not the native
        // endpoint/model registration used by ordinary account discovery. A user may
        // switch an otherwise runnable Claude installation from an unknown provider.
        let preserve_executable_outcome = executable_outcome.is_some() || for_settings;
        let configuration_failure_outcome = |reason: AgentReportOnlyReasonV1| {
            executable_outcome.clone().unwrap_or_else(|| {
                report_only_agent(
                    "agent_claude_default",
                    AgentKindV1::ClaudeCode,
                    version.clone(),
                    reason.clone(),
                )
                .outcome
            })
        };
        let mut observations = Vec::new();
        for source in self.claude_sources() {
            match source.read(
                &self.layout.process_environment,
                &self.layout.process_environment_presence,
            ) {
                Ok(observed) => observations.push(observed),
                Err(error) => {
                    return FilesystemAgentDiscoveryV1 {
                        outcome: configuration_failure_outcome(error.report_reason()),
                        configuration_issue: Some(error.report_reason()),
                        claude_configuration: None,
                        discovered_credential: None,
                        permission_hardening: None,
                        managed_launch: None,
                    };
                }
            }
        }
        let main_observations =
            main_observation::resolve_project_local_fields(&self.layout, &observations);
        let (observed_outcome, main_conflict) =
            main_observation::main_claude_observation(&version, &main_observations);
        let mut outcome = executable_outcome.unwrap_or(observed_outcome);
        let managed_launch = if let Some(executable) = executable
            .as_ref()
            .filter(|_| for_settings || version == CLAUDE_CODE_MANAGED_LAUNCH_VERSION_V1)
        {
            let profile = match &outcome {
                AgentDiscoveryOutcomeV1::Supported { installation } => {
                    installation.profile.managed_launch.as_ref()
                }
                AgentDiscoveryOutcomeV1::ReportOnly { .. } => None,
            };
            let Some(profile) = profile else {
                return FilesystemAgentDiscoveryV1 {
                    outcome,
                    configuration_issue: None,
                    claude_configuration: None,
                    discovered_credential: None,
                    permission_hardening: None,
                    managed_launch: None,
                };
            };
            Some(ManagedClaudeLaunchPreflightV1::from_observations(
                executable.canonical_path.clone(),
                profile.exact_version.clone(),
                profile,
                &observations,
            ))
        } else {
            None
        };
        if let (Some(preflight), AgentDiscoveryOutcomeV1::Supported { installation }) =
            (&managed_launch, &mut outcome)
        {
            installation.observation_digest = CanonicalDigest::of(&(
                "hiroute.managed-launch-observation/v1",
                &installation.observation_digest,
                preflight,
            ))
            .unwrap_or_else(|_| CanonicalDigest::of_bytes(b"invalid-managed-launch-observation"));
        }
        if main_conflict {
            return FilesystemAgentDiscoveryV1 {
                outcome,
                configuration_issue: Some(AgentReportOnlyReasonV1::ConflictingEffectiveConfig),
                claude_configuration: None,
                discovered_credential: None,
                permission_hardening: None,
                managed_launch,
            };
        }
        // Worker preflight above consumes the original layers; main fields use native local overrides.
        let observations = main_observations;
        let endpoint = select_effective(&observations, |settings| settings.env.base_url.as_deref());
        let model = select_effective(&observations, |settings| settings.env.model.as_deref());
        let secret = select_effective_source(&observations, |settings| {
            settings
                .env
                .present_environment_fields
                .contains("ANTHROPIC_AUTH_TOKEN")
        });
        let default_opus_model = select_effective(&observations, |settings| {
            settings.env.default_opus_model.as_deref()
        });
        let (Some(endpoint), Some(model)) = (endpoint, model) else {
            return FilesystemAgentDiscoveryV1 {
                outcome,
                configuration_issue: None,
                claude_configuration: None,
                discovered_credential: None,
                permission_hardening: None,
                managed_launch,
            };
        };
        if !valid_observed_model(model.0) {
            if !preserve_executable_outcome {
                outcome = report_only_agent(
                    "agent_claude_default",
                    AgentKindV1::ClaudeCode,
                    version.clone(),
                    AgentReportOnlyReasonV1::UnregisteredModel,
                )
                .outcome;
            }
            return FilesystemAgentDiscoveryV1 {
                outcome,
                configuration_issue: Some(AgentReportOnlyReasonV1::UnregisteredModel),
                claude_configuration: None,
                discovered_credential: None,
                permission_hardening: None,
                managed_launch,
            };
        }
        let model_candidates =
            registered_model_candidates(model.0, default_opus_model.map(|(value, _)| value));
        let Some(registered) = self.registry.resolve(endpoint.0, &model_candidates) else {
            let reason = if self.registry.has_endpoint(endpoint.0) {
                AgentReportOnlyReasonV1::UnregisteredModel
            } else {
                AgentReportOnlyReasonV1::UnregisteredEndpoint
            };
            if !preserve_executable_outcome {
                outcome = report_only_agent(
                    "agent_claude_default",
                    AgentKindV1::ClaudeCode,
                    version.clone(),
                    reason.clone(),
                )
                .outcome;
            }
            return FilesystemAgentDiscoveryV1 {
                outcome,
                configuration_issue: Some(reason),
                claude_configuration: None,
                discovered_credential: None,
                permission_hardening: None,
                managed_launch,
            };
        };
        let configuration_revision = endpoint.1.revision.max(model.1.revision);
        let resolved_model_id = registered
            .resolved_upstream_model_id
            .clone()
            .unwrap_or_else(|| model.0.to_owned());
        let mut provider_model_alias_hints: BTreeMap<String, String> = default_opus_model
            .map(|(value, _)| [("default_opus_model".to_owned(), value.to_owned())].into())
            .unwrap_or_default();
        if resolved_model_id != model.0 {
            provider_model_alias_hints.insert("configured_model".to_owned(), model.0.to_owned());
        }
        let discovered_credential = secret.map(discovered_credential);
        FilesystemAgentDiscoveryV1 {
            outcome,
            configuration_issue: None,
            claude_configuration: Some(RegisteredClaudeConfigurationV1 {
                connection_option_id: registered.connection_option_id.clone(),
                endpoint_profile_id: registered.endpoint_profile_id.clone(),
                endpoint_profile_revision: registered.endpoint_profile_revision,
                model_configuration_id: registered.model_configuration_id.clone(),
                base_url: registered.base_url.clone(),
                observed_model_id: resolved_model_id,
                provider_model_alias_hints,
                configuration_revision,
            }),
            discovered_credential,
            permission_hardening: None,
            managed_launch,
        }
    }

    pub(super) fn claude_sources(&self) -> Vec<ClaudeSource> {
        let mut sources = vec![ClaudeSource::Process];
        if let Some(path) = &self.layout.claude_launch_settings {
            sources.push(ClaudeSource::File(ConfigLayerV1::Launch, path.clone()));
        }
        sources.extend(
            self.layout
                .claude_project_settings
                .iter()
                .cloned()
                .map(|path| ClaudeSource::File(ConfigLayerV1::Project, path)),
        );
        sources.push(ClaudeSource::File(
            ConfigLayerV1::User,
            self.layout.claude_user_settings.clone(),
        ));
        sources.extend(
            self.layout
                .claude_managed_settings
                .iter()
                .cloned()
                .map(|path| ClaudeSource::File(ConfigLayerV1::Managed, path)),
        );
        sources
    }
}

fn discovered_credential(source: &ObservedClaudeSettings) -> DiscoveredCredentialRefV1 {
    DiscoveredCredentialRefV1 {
        source: "discovered_config".to_owned(),
        scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.to_owned(),
        scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.to_owned(),
        discovered_source_ref: source.source_ref.clone(),
        field_selector: "env.ANTHROPIC_AUTH_TOKEN".to_owned(),
        observed_revision: source.revision,
    }
}

fn registered_model_candidates(primary: &str, alias_hint: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    for candidate in [Some(primary), alias_hint].into_iter().flatten() {
        if !candidates.iter().any(|existing| existing == candidate) {
            candidates.push(candidate.to_owned());
        }
        // Some provider-facing Claude aliases append a context selector such as `[1m]`. The
        // suffix is accepted only when its stripped value matches an exact catalog upstream model
        // below; this never expands the release catalog or guesses an unregistered model.
        if let Some((canonical, suffix)) = candidate.rsplit_once('[')
            && suffix.ends_with(']')
            && !canonical.is_empty()
            && !candidates.iter().any(|existing| existing == canonical)
        {
            candidates.push(canonical.to_owned());
        }
    }
    candidates
}

fn valid_observed_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && !value.chars().any(char::is_control)
}

fn select_effective<'a>(
    values: &'a [ObservedClaudeSettings],
    field: impl Fn(&'a ClaudeSettingsSubset) -> Option<&'a str>,
) -> Option<(&'a str, &'a ObservedClaudeSettings)> {
    values
        .iter()
        .filter_map(|value| field(&value.settings).map(|field| (field, value)))
        .min_by_key(|(_, value)| value.layer.precedence_for(AgentKindV1::ClaudeCode))
}

fn select_effective_source<'a>(
    values: &'a [ObservedClaudeSettings],
    field: impl Fn(&'a ClaudeSettingsSubset) -> bool,
) -> Option<&'a ObservedClaudeSettings> {
    values
        .iter()
        .filter(|value| field(&value.settings))
        .min_by_key(|value| value.layer.precedence_for(AgentKindV1::ClaudeCode))
}

fn validate_descriptor(
    descriptor: &DiscoveredCredentialRefV1,
) -> Result<(), AgentFilesystemScanError> {
    if descriptor.source != "discovered_config"
        || descriptor.scanner_id != FILESYSTEM_AGENT_SCANNER_ID_V1
        || descriptor.scanner_version != FILESYSTEM_AGENT_SCANNER_VERSION_V1
        || !matches!(
            descriptor.field_selector.as_str(),
            "env.ANTHROPIC_AUTH_TOKEN" | "env.ANTHROPIC_API_KEY"
        )
        || descriptor.observed_revision == 0
    {
        return Err(AgentFilesystemScanError::InvalidDescriptor);
    }
    Ok(())
}

fn validate_hardening_finding(
    finding: &PermissionHardeningRequiredV1,
) -> Result<(), AgentFilesystemScanError> {
    if finding.scanner_id != FILESYSTEM_AGENT_SCANNER_ID_V1
        || finding.scanner_version != FILESYSTEM_AGENT_SCANNER_VERSION_V1
        || !finding
            .discovered_source_ref
            .starts_with("claude/settings/")
        || finding.observed_revision == 0
        || finding.required_mode != 0o600
        || CanonicalDigest::parse(finding.observed_identity.as_str().to_owned()).is_err()
    {
        return Err(AgentFilesystemScanError::InvalidDescriptor);
    }
    Ok(())
}

fn base_observation(agent_id: &str, kind: AgentKindV1, version: String) -> AgentScanObservationV1 {
    AgentScanObservationV1 {
        schema: AGENT_SCAN_OBSERVATION_SCHEMA_V1.to_owned(),
        agent_id: agent_id.to_owned(),
        kind,
        version,
        config: Vec::new(),
    }
}

fn report_only_agent(
    agent_id: &str,
    kind: AgentKindV1,
    version: String,
    reason: AgentReportOnlyReasonV1,
) -> FilesystemAgentDiscoveryV1 {
    let observation_digest = CanonicalDigest::of_bytes(
        format!("agent-report-only\0{agent_id}\0{version}\0{reason:?}").as_bytes(),
    );
    FilesystemAgentDiscoveryV1 {
        outcome: AgentDiscoveryOutcomeV1::ReportOnly {
            agent_id: agent_id.to_owned(),
            kind,
            version,
            reason,
            observation_digest,
        },
        configuration_issue: None,
        claude_configuration: None,
        discovered_credential: None,
        permission_hardening: None,
        managed_launch: None,
    }
}

fn outcome_agent_id(outcome: &AgentDiscoveryOutcomeV1) -> &str {
    match outcome {
        AgentDiscoveryOutcomeV1::Supported { installation } => &installation.agent_id,
        AgentDiscoveryOutcomeV1::ReportOnly { agent_id, .. } => agent_id,
    }
}

#[derive(Debug, Error)]
pub enum AgentFilesystemScanError {
    #[error("Agent configuration is a symlink")]
    SymlinkConfig,
    #[error("Agent configuration is owned by another user")]
    WrongOwner,
    #[error("Agent configuration permissions expose protected values")]
    UnsafePermissions,
    #[error("Agent configuration is invalid")]
    InvalidConfig,
    #[error("Agent configuration exceeds the scanner bound")]
    ConfigTooLarge,
    #[error("discovered source is unavailable")]
    SourceUnavailable,
    #[error("discovered source changed")]
    SourceChanged,
    #[error("discovered source descriptor is invalid")]
    InvalidDescriptor,
    #[error("Release registry is invalid")]
    InvalidRegistry,
    #[error("Agent configuration I/O failed")]
    Io(#[from] std::io::Error),
}

impl AgentFilesystemScanError {
    fn report_reason(&self) -> AgentReportOnlyReasonV1 {
        match self {
            Self::SymlinkConfig => AgentReportOnlyReasonV1::SymlinkConfig,
            Self::WrongOwner => AgentReportOnlyReasonV1::WrongOwner,
            Self::UnsafePermissions => AgentReportOnlyReasonV1::UnsafeConfigPermissions,
            _ => AgentReportOnlyReasonV1::ConfigUnavailable,
        }
    }
}

#[cfg(test)]
#[path = "filesystem_tests.rs"]
mod tests;

fn probe_report(
    agent_id: &str,
    kind: AgentKindV1,
    result: ExecutableProbe,
) -> FilesystemAgentDiscoveryV1 {
    let reason = match result {
        ExecutableProbe::NotFound => AgentReportOnlyReasonV1::NotFoundInScope,
        ExecutableProbe::Unknown(reason) => match reason {
            super::executable::ProbeFailure::NotExecutable => {
                AgentReportOnlyReasonV1::ExecutableNotRunnable
            }
            super::executable::ProbeFailure::TimedOut => {
                AgentReportOnlyReasonV1::ExecutableProbeTimedOut
            }
            _ => AgentReportOnlyReasonV1::ExecutableProbeUnavailable,
        },
        ExecutableProbe::Installed(_) => unreachable!("installed handled by native scanner"),
    };
    report_only_agent(agent_id, kind, String::new(), reason)
}

fn codex_config_path(home: &Path, configured: Option<&std::ffi::OsStr>) -> PathBuf {
    configured
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
        .join("config.toml")
}
