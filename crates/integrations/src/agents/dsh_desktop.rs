//! Official Desktop uses its own profile and immutable bundled public CLI.
//! Never redirect Web ownership or require a separately installed npm CLI.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DshDesktopModelCheckError {
    #[error("DSH Desktop patch is unavailable or invalid")]
    Patch,
    #[error("DSH Desktop manifest is unavailable or invalid")]
    Manifest,
    #[error("DSH Desktop profile uses a custom bundle or dependency composition")]
    CustomProfile,
    #[error("DSH Desktop executable or archive is unavailable")]
    Runtime,
    #[error("DSH Desktop bundled CLI default configuration probe failed")]
    Probe,
    #[error("DSH Desktop default composition does not expose the required adapter")]
    MissingAdapter,
    #[error("DSH Desktop configuration changed during validation")]
    SourceChanged,
}

impl DshDesktopModelCheckError {
    fn file_error(error: AgentFilesystemScanError, fallback: Self) -> Self {
        match error {
            AgentFilesystemScanError::SourceChanged => Self::SourceChanged,
            _ => fallback,
        }
    }

    #[cfg(any(unix, windows))]
    fn composition_error(error: super::dsh_native::DshCompositionError) -> Self {
        use super::dsh_native::DshCompositionError;
        match error {
            DshCompositionError::InvalidOutput => Self::Probe,
            DshCompositionError::MissingRequiredPlugin => Self::MissingAdapter,
        }
    }
}

#[cfg(not(windows))]
pub(super) fn installed_executable(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
        None
    }
}

#[cfg(windows)]
pub(super) fn windows_installation_roots(
    home: &Path,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let local = lookup("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join("AppData/Local"));
    let mut roots = vec![local.join("Programs/DeepSeek Harness")];
    for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(path) = lookup(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            let root = path.join("DeepSeek Harness");
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

#[cfg(windows)]
fn windows_installed_executable(root: &Path) -> Option<PathBuf> {
    if !root.is_absolute() {
        return None;
    }
    super::super::executable::resolve(&root.join("DeepSeek Harness.exe"))
        .ok()
        .flatten()
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
        if let Some(executable) = &self.layout.dsh_desktop_executable {
            return super::super::executable::resolve(executable).ok().flatten();
        }
        #[cfg(windows)]
        {
            let roots = &self.layout.dsh_desktop_installation_roots;
            if roots.is_empty() {
                return None;
            }
            roots
                .iter()
                .find_map(|root| windows_installed_executable(root))
                .or_else(|| {
                    hiroute_platform::windows::installed_apps::uninstall_locations(
                        "DeepSeek Harness",
                    )
                    .iter()
                    .find_map(|root| windows_installed_executable(root))
                })
        }
        #[cfg(not(windows))]
        None
    }
    fn dsh_desktop_archive(executable: &Path) -> Result<PathBuf, AgentFilesystemScanError> {
        #[cfg(not(windows))]
        let archive = executable
            .parent()
            .and_then(Path::parent)
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?
            .join("Resources/app.asar");
        #[cfg(windows)]
        let archive = {
            let path = executable
                .parent()
                .ok_or(AgentFilesystemScanError::SourceUnavailable)?
                .join("resources/app.asar");
            super::super::managed_launch::windows_executable_path(&path)
                .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
        };
        Ok(archive)
    }
    #[cfg(any(unix, windows))]
    fn dsh_desktop_command(executable: &Path, archive: &Path) -> std::process::Command {
        let mut command = std::process::Command::new(executable);
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("ELECTRON_RUN_AS_NODE", "1")
            .arg("--expose-internals")
            .arg(archive.join("dsh/node_modules/@deepseek-ai/dsh-desktop-host/lib/cli.js"));
        command
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
        if diagnostic
            && let Some(executable) = executable.as_deref()
            && let Ok(archive) = Self::dsh_desktop_archive(executable)
        {
            let mut command = Self::dsh_desktop_command(executable, &archive);
            command.arg("--version");
            if let Ok(bytes) = super::dsh_native::dsh_public_probe(command, false)
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
    fn dsh_desktop_patch_dependencies(
        &self,
        skills: bool,
    ) -> Result<Vec<CanonicalDigest>, AgentFilesystemScanError> {
        let mut dependencies = Vec::new();
        for path in [
            self.layout.dsh_config_root.join("cordis.patch.yml"),
            self.dsh_desktop_models_target(),
        ] {
            let bytes = super::super::dsh_config::read_patch_bytes(&path)?;
            let patch = super::super::dsh_config::Patch::parse(Some(&bytes)).map_err(invalid)?;
            if skills {
                let filesystem = patch
                    .resource_config("skill-filesystem", "@deepseek-ai/dsh-skill-filesystem")
                    .map_err(invalid)?;
                let tool = patch
                    .resource_config("tool-skill", "@deepseek-ai/dsh-tool-skill")
                    .map_err(invalid)?;
                let skill = patch
                    .resource_config("skill", "@deepseek-ai/dsh-skill")
                    .map_err(invalid)?;
                if filesystem.is_some_and(|v| {
                    v["includeDefaultRoots"] == false
                        || v.get("dshHome")
                            .is_some_and(|p| p.as_str() != self.layout.dsh_config_root.to_str())
                }) {
                    return Err(AgentFilesystemScanError::InvalidConfig);
                }
                dependencies
                    .push(CanonicalDigest::of(&(filesystem, tool, skill)).map_err(invalid)?);
                continue;
            }
            if path == self.layout.dsh_config_root.join("cordis.patch.yml")
                && patch.config("llm-pi-ai").map_err(invalid)?.is_some()
            {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
            dependencies.push(CanonicalDigest::of_bytes(&bytes));
        }
        Ok(dependencies)
    }

    fn dsh_desktop_contract(
        &self,
        skills: bool,
    ) -> Result<(CanonicalDigest, PathBuf, PathBuf, bool), DshDesktopModelCheckError> {
        use DshDesktopModelCheckError as Check;
        let mut dependencies = self
            .dsh_desktop_patch_dependencies(skills)
            .map_err(|error| Check::file_error(error, Check::Patch))?;
        let manifest = self
            .layout
            .dsh_config_root
            .join("profiles/desktop/package.json");
        let bytes = super::super::dsh_config::read_patch_bytes(&manifest)
            .map_err(|error| Check::file_error(error, Check::Manifest))?;
        let mut factory = true;
        if !bytes.is_empty() {
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| Check::Manifest)?;
            let bundles = value["dsh"]["profile"]["bundles"]
                .as_array()
                .filter(|entries| entries.iter().all(serde_json::Value::is_string))
                .ok_or(Check::Manifest)?;
            let has_dependencies = match value.get("dependencies") {
                Some(serde_json::Value::Object(entries)) => !entries.is_empty(),
                Some(_) => return Err(Check::Manifest),
                None => false,
            };
            let base = bundles
                .iter()
                .position(|entry| entry == "@deepseek-ai/dsh-base");
            let web = bundles
                .iter()
                .position(|entry| entry == "@deepseek-ai/dsh-web-app");
            factory = base == Some(0) && web == Some(1) && bundles.len() == 2 && !has_dependencies;
            if !matches!((base, web), (Some(base), Some(web)) if base < web) || (skills && !factory)
            {
                return Err(Check::CustomProfile);
            }
        }
        dependencies.push(CanonicalDigest::of_bytes(&bytes));
        let executable = self.dsh_desktop_executable_target().ok_or(Check::Runtime)?;
        let archive = Self::dsh_desktop_archive(&executable).map_err(|_| Check::Runtime)?;
        for path in [&executable, &archive] {
            let metadata = std::fs::metadata(path).map_err(|_| Check::Runtime)?;
            if !metadata.is_file() {
                return Err(Check::Runtime);
            }
            dependencies.push(
                CanonicalDigest::of(&(path, metadata.len(), metadata.modified().ok()))
                    .map_err(|_| Check::Runtime)?,
            );
        }
        let digest = CanonicalDigest::of(&dependencies).map_err(|_| Check::Runtime)?;
        Ok((digest, executable, archive, factory))
    }
    #[cfg(any(unix, windows))]
    pub fn check_dsh_desktop_model_configuration(
        &self,
    ) -> Result<(CanonicalDigest, bool), DshDesktopModelCheckError> {
        use DshDesktopModelCheckError as Check;
        let (before, executable, archive, factory) = self.dsh_desktop_contract(false)?;
        let mut command = std::process::Command::new(&executable);
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("ELECTRON_RUN_AS_NODE", "1")
            .args([
                "--expose-internals",
                "--input-type=module",
                "--eval",
                concat!(
                    include_str!("../../../../assets/agents/dsh/hiroute-routing.mjs"),
                    "\n",
                    "await loadDesktopProviderApi();\n",
                    "console.log('HIRoute Desktop provider ready');\n",
                ),
            ]);
        let independent_provider =
            super::dsh_native::dsh_public_probe(command, false).is_ok_and(|output| {
                std::str::from_utf8(&output)
                    .is_ok_and(|text| text.trim() == "HIRoute Desktop provider ready")
            });
        if !independent_provider {
            if !factory {
                return Err(Check::MissingAdapter);
            }
            // Older carriers can still use the shipped Web composition for a factory profile.
            let mut command = Self::dsh_desktop_command(&executable, &archive);
            command.args(["--profile", "web", "--dump-default-config"]);
            let output =
                super::dsh_native::dsh_public_probe(command, false).map_err(|_| Check::Probe)?;
            super::dsh_native::check_composition(&output, &["llm-pi-ai"])
                .map_err(Check::composition_error)?;
        }
        if !self.dsh_desktop_model_configuration_is_current(&before)? {
            return Err(Check::SourceChanged);
        }
        Ok((before, independent_provider))
    }
    pub fn dsh_desktop_model_configuration_is_current(
        &self,
        checked: &CanonicalDigest,
    ) -> Result<bool, DshDesktopModelCheckError> {
        self.dsh_desktop_contract(false)
            .map(|(current, _, _, _)| current == *checked)
    }

    #[cfg(any(unix, windows))]
    pub(super) fn check_dsh_desktop_management(
        &self,
    ) -> Result<CanonicalDigest, DshDesktopModelCheckError> {
        use DshDesktopModelCheckError as Check;
        let (before, executable, archive, _) = self.dsh_desktop_contract(true)?;
        let mut command = Self::dsh_desktop_command(&executable, &archive);
        command.args(["--profile", "web", "--dump-default-config"]);
        let output =
            super::dsh_native::dsh_public_probe(command, false).map_err(|_| Check::Probe)?;
        super::dsh_native::check_composition(&output, &["skill-filesystem", "tool-skill", "skill"])
            .map_err(Check::composition_error)?;
        if before != self.dsh_desktop_contract(true)?.0 {
            return Err(Check::SourceChanged);
        }
        Ok(before)
    }
}

fn invalid<T>(_: T) -> AgentFilesystemScanError {
    AgentFilesystemScanError::InvalidConfig
}

#[cfg(all(test, windows))]
#[path = "dsh_desktop_windows_tests.rs"]
mod windows_tests;
