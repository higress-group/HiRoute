//! Official Desktop uses its own profile and immutable bundled public CLI.
//! Never redirect Web ownership or require a separately installed npm CLI.
use super::*;

pub(super) fn installed_executable(home: &Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    [home.join("Applications"), PathBuf::from("/Applications")]
        .into_iter()
        .map(|root| root.join("DeepSeek Harness.app/Contents/MacOS/DeepSeek Harness"))
        .find(|path| {
            super::super::executable::resolve(path)
                .ok()
                .flatten()
                .is_some()
        })
}

impl FilesystemAgentScannerV1 {
    pub fn dsh_desktop_models_target(&self) -> PathBuf {
        self.layout
            .dsh_config_root
            .join("profiles/desktop/cordis.patch.yml")
    }
    pub fn dsh_desktop_routing_adapter_target(&self) -> PathBuf {
        self.layout
            .dsh_config_root
            .join("profiles/desktop/hiroute-routing.mjs")
    }
    pub fn dsh_desktop_executable_target(&self) -> Option<PathBuf> {
        super::super::executable::resolve(self.layout.dsh_desktop_executable.as_deref()?)
            .ok()
            .flatten()
    }
    fn dsh_desktop_archive(&self) -> Result<PathBuf, AgentFilesystemScanError> {
        let executable = self
            .dsh_desktop_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        Ok(executable
            .parent()
            .and_then(Path::parent)
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?
            .join("Resources/app.asar"))
    }
    #[cfg(any(unix, windows))]
    fn dsh_desktop_command(&self) -> Result<std::process::Command, AgentFilesystemScanError> {
        let mut command = std::process::Command::new(
            self.dsh_desktop_executable_target()
                .ok_or(AgentFilesystemScanError::SourceUnavailable)?,
        );
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("ELECTRON_RUN_AS_NODE", "1")
            .arg("--expose-internals")
            .arg(
                self.dsh_desktop_archive()?
                    .join("dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js"),
            );
        Ok(command)
    }
    pub fn dsh_desktop_settings_discovery(&self) -> FilesystemAgentDiscoveryV1 {
        self.dsh_desktop_discovery(false)
    }
    pub(super) fn scan_dsh_desktop(&self) -> FilesystemAgentDiscoveryV1 {
        self.dsh_desktop_discovery(true)
    }
    fn dsh_desktop_discovery(&self, diagnostic: bool) -> FilesystemAgentDiscoveryV1 {
        let executable = self.dsh_desktop_executable_target();
        if diagnostic && executable.is_none() {
            return probe_report(
                "agent_dsh_desktop",
                AgentKindV1::DeepseekHarness,
                ExecutableProbe::NotFound,
            );
        }
        let mut version = String::new();
        #[cfg(any(unix, windows))]
        if diagnostic && let Ok(mut command) = self.dsh_desktop_command() {
            command.arg("--version");
            if let Ok(bytes) = super::dsh_native::dsh_public_probe(command)
                && let Ok(value) = std::str::from_utf8(&bytes)
                && value.trim().len() <= 128
                && !value.trim().chars().any(char::is_control)
            {
                version = value.trim().into();
            }
        }
        // File recovery remains available after uninstall; Configure independently
        // requires a current composition proof. This is not a live-client assertion.
        let mut outcome = resolve_agent_observation(base_observation(
            "agent_dsh_desktop",
            AgentKindV1::DeepseekHarness,
            version,
        ));
        if let AgentDiscoveryOutcomeV1::Supported { installation } = &mut outcome {
            super::super::observed_capabilities::attach_optional_file_capabilities(
                installation,
                executable.as_deref(),
                &self.dsh_desktop_models_target(),
            );
        }
        FilesystemAgentDiscoveryV1 {
            outcome,
            configuration_issue: None,
            claude_configuration: None,
            discovered_credential: None,
            permission_hardening: None,
            managed_launch: None,
        }
    }
    fn dsh_desktop_contract(&self) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let mut dependencies = Vec::new();
        for path in [
            self.layout.dsh_config_root.join("cordis.patch.yml"),
            self.dsh_desktop_models_target(),
        ] {
            let bytes = super::super::dsh_config::read_patch_bytes(&path)?;
            let patch = super::super::dsh_config::Patch::parse(Some(&bytes)).map_err(invalid)?;
            if path == self.layout.dsh_config_root.join("cordis.patch.yml")
                && patch.config("llm-pi-ai").map_err(invalid)?.is_some()
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
            dependencies.push(CanonicalDigest::of_bytes(&bytes));
        }
        let manifest = self
            .layout
            .dsh_config_root
            .join("profiles/desktop/package.json");
        let bytes = super::super::dsh_config::read_patch_bytes(&manifest)?;
        if !bytes.is_empty() {
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(invalid)?;
            // This is the shipped Desktop composition. Custom bundle loading is not
            // established by the public CLI's default dump and must not be asserted.
            if value["dsh"]["profile"]["bundles"]
                != serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
                || value
                    .get("dependencies")
                    .is_some_and(|v| !v.as_object().is_some_and(|v| v.is_empty()))
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
        }
        dependencies.push(CanonicalDigest::of_bytes(&bytes));
        let executable = self
            .dsh_desktop_executable_target()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        for path in [&executable, &self.dsh_desktop_archive()?] {
            let metadata =
                std::fs::metadata(path).map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
            if !metadata.is_file() {
                return Err(AgentFilesystemScanError::SourceUnavailable);
            }
            dependencies.push(
                CanonicalDigest::of(&(path, metadata.len(), metadata.modified().ok()))
                    .map_err(invalid)?,
            );
        }
        CanonicalDigest::of(&dependencies).map_err(invalid)
    }
    #[cfg(any(unix, windows))]
    pub fn check_dsh_desktop_model_configuration(
        &self,
    ) -> Result<CanonicalDigest, AgentFilesystemScanError> {
        let before = self.dsh_desktop_contract()?;
        let mut command = self.dsh_desktop_command()?;
        // Upstream forbids dumping the Desktop profile. Its standard manifest uses
        // these same Web bundles; inspect that default, never pretend this runs Desktop.
        command.args(["--profile", "web", "--dump-default-config"]);
        super::dsh_native::check_composition(
            &super::dsh_native::dsh_public_probe(command)?,
            &["llm-pi-ai"],
        )?;
        if before != self.dsh_desktop_contract()? {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        Ok(before)
    }
    pub fn dsh_desktop_model_configuration_is_current(
        &self,
        checked: &CanonicalDigest,
    ) -> Result<bool, AgentFilesystemScanError> {
        self.dsh_desktop_contract()
            .map(|current| current == *checked)
    }
}

fn invalid<T>(_: T) -> AgentFilesystemScanError {
    AgentFilesystemScanError::InvalidConfig
}
