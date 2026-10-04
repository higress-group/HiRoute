use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use hiroute_application_api::{
    WORKER_DEPENDENCIES_VIEW_SCHEMA_V1, WorkerDependenciesDiscoverRequestV1,
    WorkerDependenciesSelectRequestV1, WorkerDependenciesViewV1, WorkerDependencyCandidateSourceV1,
    WorkerDependencyCandidateStateV1, WorkerDependencyCandidateV1, WorkerDependencyComponentV1,
    WorkerDependencyInstallHintV1, WorkerDependencySelectionRevisionV1,
};
use hiroute_domain::delegation::{DelegationErrorV1, WorkerHarnessV1};

use super::{WORKER_HARNESSES, WorkerInstallationConfig, WorkerInstallationSelectionSource};

const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_CANDIDATES: usize = 64;
const MAX_PACKAGE_BYTES: u64 = 64 * 1024;
const SCAN_BUDGET: Duration = Duration::from_secs(2);

pub(crate) fn discover(
    source: &dyn WorkerInstallationSelectionSource,
    request: &WorkerDependenciesDiscoverRequestV1,
) -> Result<WorkerDependenciesViewV1, DelegationErrorV1> {
    discover_with_environment(source, request, &ScanEnvironment::current())
}

fn discover_with_environment(
    source: &dyn WorkerInstallationSelectionSource,
    request: &WorkerDependenciesDiscoverRequestV1,
    environment: &ScanEnvironment,
) -> Result<WorkerDependenciesViewV1, DelegationErrorV1> {
    if !request.valid() {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let harnesses = request
        .harness
        .map_or_else(|| WORKER_HARNESSES.to_vec(), |harness| vec![harness]);
    let mut selection_revisions = Vec::with_capacity(harnesses.len());
    let mut selected = Vec::new();
    let mut candidates = Vec::new();
    let mut install_hints = Vec::new();
    for harness in harnesses {
        let snapshot = source.selection(harness)?;
        selection_revisions.push(WorkerDependencySelectionRevisionV1 {
            harness,
            revision: snapshot.as_ref().map_or(0, |selection| selection.revision),
        });
        if let Some(snapshot) = &snapshot {
            selected.push(snapshot.config.public_selection());
        }
        let mut scan = HarnessScan::new(harness, environment.clone());
        if let Some(snapshot) = snapshot {
            scan.add_selected(&snapshot.config);
        }
        scan.add_path_candidates();
        scan.add_common_candidates();
        if harness != WorkerHarnessV1::QoderCli {
            scan.add_npm_prefix_candidates();
            scan.add_version_manager_candidates();
            scan.add_npx_candidates();
        }
        scan.finish_hints(&mut install_hints);
        candidates.extend(scan.candidates);
    }
    let view = WorkerDependenciesViewV1 {
        schema: WORKER_DEPENDENCIES_VIEW_SCHEMA_V1.to_owned(),
        selection_revisions,
        candidates,
        selected,
        install_hints,
    };
    if view.valid() {
        Ok(view)
    } else {
        Err(DelegationErrorV1::StorageUnavailable)
    }
}

#[derive(Clone, Default)]
struct ScanEnvironment {
    path: Option<OsString>,
    home: Option<PathBuf>,
    npm_prefix: Option<PathBuf>,
    npm_cache: Option<PathBuf>,
}

impl ScanEnvironment {
    fn current() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            npm_prefix: std::env::var_os("NPM_CONFIG_PREFIX")
                .or_else(|| std::env::var_os("npm_config_prefix"))
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            npm_cache: std::env::var_os("NPM_CONFIG_CACHE")
                .or_else(|| std::env::var_os("npm_config_cache"))
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
        }
    }
}

pub(crate) fn validate_selection(
    request: &WorkerDependenciesSelectRequestV1,
) -> Result<WorkerDependenciesSelectRequestV1, DelegationErrorV1> {
    if !request.valid() {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let adapter = request
        .adapter_path
        .as_deref()
        .map(|path| normalize_required(Path::new(path), request.node_path.is_none()))
        .transpose()?;
    let cli = normalize_required(Path::new(&request.cli_path), true)?;
    let node = request
        .node_path
        .as_deref()
        .map(|path| normalize_required(Path::new(path), true))
        .transpose()?;
    Ok(WorkerDependenciesSelectRequestV1 {
        harness: request.harness,
        adapter_path: adapter.map(path_string).transpose()?,
        cli_path: path_string(cli)?,
        node_path: node.map(path_string).transpose()?,
        expected_selection_revision: request.expected_selection_revision,
    })
}

pub(crate) fn validate_persisted_installation(
    selection: &WorkerInstallationConfig,
) -> Result<WorkerInstallationConfig, DelegationErrorV1> {
    let request = WorkerDependenciesSelectRequestV1 {
        harness: selection.harness,
        adapter_path: selection
            .adapter
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        cli_path: selection.harness_binary.to_string_lossy().into_owned(),
        node_path: selection
            .node_binary
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        expected_selection_revision: 0,
    };
    let normalized = validate_selection(&request)?;
    if normalized.adapter_path != request.adapter_path
        || normalized.cli_path != request.cli_path
        || normalized.node_path != request.node_path
    {
        return Err(DelegationErrorV1::DependenciesInvalid);
    }
    Ok(WorkerInstallationConfig {
        harness: normalized.harness,
        adapter: normalized.adapter_path.map(PathBuf::from),
        harness_binary: PathBuf::from(normalized.cli_path),
        node_binary: normalized.node_path.map(PathBuf::from),
    })
}

struct HarnessScan {
    harness: WorkerHarnessV1,
    environment: ScanEnvironment,
    started: Instant,
    directory_entries: usize,
    incomplete: bool,
    candidates: Vec<WorkerDependencyCandidateV1>,
    seen: BTreeSet<(WorkerDependencyComponentV1, PathBuf)>,
}

impl HarnessScan {
    fn new(harness: WorkerHarnessV1, environment: ScanEnvironment) -> Self {
        Self {
            harness,
            environment,
            started: Instant::now(),
            directory_entries: 0,
            incomplete: false,
            candidates: Vec::new(),
            seen: BTreeSet::new(),
        }
    }

    fn add_selected(&mut self, config: &WorkerInstallationConfig) {
        if let Some(adapter) = &config.adapter {
            self.add_explicit(
                WorkerDependencyComponentV1::Adapter,
                adapter,
                WorkerDependencyCandidateSourceV1::Selected,
                config.node_binary.is_none(),
                true,
            );
        }
        self.add_explicit(
            WorkerDependencyComponentV1::Cli,
            &config.harness_binary,
            WorkerDependencyCandidateSourceV1::Selected,
            true,
            true,
        );
        if let Some(node) = &config.node_binary {
            self.add_explicit(
                WorkerDependencyComponentV1::Node,
                node,
                WorkerDependencyCandidateSourceV1::Selected,
                true,
                true,
            );
        }
    }

    fn add_path_candidates(&mut self) {
        let Some(path) = self.environment.path.clone() else {
            return;
        };
        for directory in std::env::split_paths(&path) {
            if self.exhausted() {
                return;
            }
            self.add_bin_directory(&directory, WorkerDependencyCandidateSourceV1::Path);
        }
    }

    fn add_common_candidates(&mut self) {
        let mut directories = Vec::new();
        if let Some(home) = self.environment.home.as_ref() {
            directories.push(home.join(".local/bin"));
            if self.harness == WorkerHarnessV1::QoderCli {
                directories.push(home.join(".qoder/bin"));
            }
        }
        directories.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);
        #[cfg(target_os = "macos")]
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        for directory in directories {
            if self.exhausted() {
                return;
            }
            self.add_bin_directory(&directory, WorkerDependencyCandidateSourceV1::Common);
        }
        if self.harness == WorkerHarnessV1::QoderCli {
            self.add_qoder_standalone_versions();
        }
    }

    fn add_qoder_standalone_versions(&mut self) {
        let Some(home) = self.environment.home.as_ref() else {
            return;
        };
        let entries = match fs::read_dir(home.join(".qoder/bin/qodercli")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.incomplete = true;
                return;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            if self.exhausted() {
                return;
            }
            self.directory_entries += 1;
            match entry {
                Ok(entry)
                    if entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("qodercli-")) =>
                {
                    paths.push(entry.path())
                }
                Ok(_) => {}
                Err(_) => self.incomplete = true,
            }
        }
        paths.sort();
        for path in paths {
            if self.exhausted() {
                return;
            }
            self.add_discovered(
                WorkerDependencyComponentV1::Cli,
                &path,
                WorkerDependencyCandidateSourceV1::Common,
                true,
            );
        }
    }

    fn add_npm_prefix_candidates(&mut self) {
        let mut prefixes = Vec::new();
        if let Some(prefix) = self.environment.npm_prefix.as_ref() {
            prefixes.push(prefix.clone());
        }
        if let Some(home) = self.environment.home.as_ref() {
            prefixes.push(home.join(".local"));
        }
        prefixes.push(PathBuf::from("/usr/local"));
        #[cfg(target_os = "macos")]
        prefixes.push(PathBuf::from("/opt/homebrew"));
        for prefix in prefixes {
            if self.exhausted() {
                return;
            }
            self.add_npm_package(&prefix, WorkerDependencyCandidateSourceV1::NpmGlobal);
        }
    }

    fn add_version_manager_candidates(&mut self) {
        let Some(home) = self.environment.home.clone() else {
            return;
        };
        self.add_version_directories(&home.join(".nvm/versions/node"), |entry| entry.join("bin"));
        self.add_version_directories(&home.join(".local/share/fnm/node-versions"), |entry| {
            entry.join("installation/bin")
        });
    }

    fn add_version_directories(&mut self, root: &Path, bin: impl Fn(&Path) -> PathBuf) {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.incomplete = true;
                return;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            if self.exhausted() {
                return;
            }
            self.directory_entries += 1;
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(_) => self.incomplete = true,
            }
        }
        paths.sort();
        for entry in paths {
            if self.exhausted() {
                return;
            }
            let bin = bin(&entry);
            self.add_bin_directory(&bin, WorkerDependencyCandidateSourceV1::NpmGlobal);
            let Some(prefix) = bin.parent() else {
                continue;
            };
            self.add_npm_package(prefix, WorkerDependencyCandidateSourceV1::NpmGlobal);
        }
    }

    fn add_npx_candidates(&mut self) {
        let Some(package) = package_name(self.harness) else {
            return;
        };
        let root = self
            .environment
            .npm_cache
            .clone()
            .map(|path| path.join("_npx"))
            .or_else(|| {
                self.environment
                    .home
                    .as_ref()
                    .map(|home| home.join(".npm/_npx"))
            });
        let Some(root) = root else {
            return;
        };
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.incomplete = true;
                return;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            if self.exhausted() {
                return;
            }
            self.directory_entries += 1;
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(_) => self.incomplete = true,
            }
        }
        paths.sort();
        for root in paths {
            if self.exhausted() {
                return;
            }
            self.add_npm_package_root(
                &root.join("node_modules").join(package),
                WorkerDependencyCandidateSourceV1::NpxCache,
            );
        }
    }

    fn add_bin_directory(&mut self, directory: &Path, source: WorkerDependencyCandidateSourceV1) {
        let (cli_names, adapter) = names(self.harness);
        for cli in cli_names {
            self.add_discovered(
                WorkerDependencyComponentV1::Cli,
                &directory.join(cli),
                source,
                true,
            );
        }
        let Some(adapter) = adapter else {
            return;
        };
        self.add_discovered(
            WorkerDependencyComponentV1::Adapter,
            &directory.join(adapter),
            source,
            false,
        );
        self.add_discovered(
            WorkerDependencyComponentV1::Node,
            &directory.join("node"),
            source,
            true,
        );
    }

    fn add_npm_package(&mut self, prefix: &Path, source: WorkerDependencyCandidateSourceV1) {
        let Some(package) = package_name(self.harness) else {
            return;
        };
        let package_root = prefix.join("lib/node_modules").join(package);
        self.add_npm_package_root(&package_root, source);
    }

    fn add_npm_package_root(
        &mut self,
        package_root: &Path,
        source: WorkerDependencyCandidateSourceV1,
    ) {
        let Some(adapter) = names(self.harness).1 else {
            return;
        };
        let package_json = package_root.join("package.json");
        let entry = match package_bin(&package_json, adapter) {
            Ok(Some(entry)) => entry,
            Ok(None) => return,
            Err(()) => {
                if package_json.exists() {
                    self.incomplete = true;
                }
                return;
            }
        };
        self.add_discovered(
            WorkerDependencyComponentV1::Adapter,
            &package_root.join(entry),
            source,
            false,
        );
    }

    fn add_explicit(
        &mut self,
        component: WorkerDependencyComponentV1,
        path: &Path,
        source: WorkerDependencyCandidateSourceV1,
        require_executable: bool,
        retain_missing: bool,
    ) {
        if self.candidates.len() >= MAX_CANDIDATES {
            self.incomplete = true;
            return;
        }
        let fact = inspect(path, require_executable);
        if !retain_missing && fact.state == WorkerDependencyCandidateStateV1::Missing {
            return;
        }
        let identity = fact.canonical.clone().unwrap_or_else(|| path.to_path_buf());
        if !identity.is_absolute() || !self.seen.insert((component, identity.clone())) {
            return;
        }
        let Ok(path) = path_string(identity) else {
            self.incomplete = true;
            return;
        };
        self.candidates.push(WorkerDependencyCandidateV1 {
            harness: self.harness,
            component,
            path,
            source,
            state: fact.state,
            reason_code: fact.reason.map(str::to_owned),
        });
    }

    fn add_discovered(
        &mut self,
        component: WorkerDependencyComponentV1,
        path: &Path,
        source: WorkerDependencyCandidateSourceV1,
        require_executable: bool,
    ) {
        self.add_explicit(component, path, source, require_executable, false);
    }

    fn exhausted(&mut self) -> bool {
        let exhausted = self.directory_entries >= MAX_DIRECTORY_ENTRIES
            || self.candidates.len() >= MAX_CANDIDATES
            || self.started.elapsed() >= SCAN_BUDGET;
        self.incomplete |= exhausted;
        exhausted
    }

    fn finish_hints(&self, hints: &mut Vec<WorkerDependencyInstallHintV1>) {
        for component in [
            WorkerDependencyComponentV1::Cli,
            WorkerDependencyComponentV1::Adapter,
            WorkerDependencyComponentV1::Node,
        ] {
            if self.harness == WorkerHarnessV1::QoderCli
                && component != WorkerDependencyComponentV1::Cli
            {
                continue;
            }
            if !self.candidates.iter().any(|candidate| {
                candidate.component == component
                    && candidate.state == WorkerDependencyCandidateStateV1::Found
            }) {
                hints.push(WorkerDependencyInstallHintV1 {
                    harness: self.harness,
                    component,
                    platform: platform().to_owned(),
                    command: None,
                    reason_code: "worker.dependencies.install_required".to_owned(),
                });
            }
        }
        if self.incomplete {
            hints.push(WorkerDependencyInstallHintV1 {
                harness: self.harness,
                component: if self.harness == WorkerHarnessV1::QoderCli {
                    WorkerDependencyComponentV1::Cli
                } else {
                    WorkerDependencyComponentV1::Adapter
                },
                platform: platform().to_owned(),
                command: None,
                reason_code: "worker.dependencies.scan_incomplete".to_owned(),
            });
        }
    }
}

struct MetadataFact {
    canonical: Option<PathBuf>,
    state: WorkerDependencyCandidateStateV1,
    reason: Option<&'static str>,
}

fn inspect(path: &Path, require_executable: bool) -> MetadataFact {
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return MetadataFact {
                canonical: None,
                state: WorkerDependencyCandidateStateV1::Missing,
                reason: Some("worker.dependencies.not_found"),
            };
        }
        Err(_) => {
            return MetadataFact {
                canonical: None,
                state: WorkerDependencyCandidateStateV1::Unavailable,
                reason: Some("worker.dependencies.metadata_unavailable"),
            };
        }
    };
    let metadata = match fs::metadata(&canonical) {
        Ok(metadata) => metadata,
        Err(_) => {
            return MetadataFact {
                canonical: Some(canonical),
                state: WorkerDependencyCandidateStateV1::Unavailable,
                reason: Some("worker.dependencies.metadata_unavailable"),
            };
        }
    };
    if !metadata.is_file() {
        return MetadataFact {
            canonical: Some(canonical),
            state: WorkerDependencyCandidateStateV1::Invalid,
            reason: Some("worker.dependencies.not_regular_file"),
        };
    }
    if require_executable && !executable(&metadata) {
        return MetadataFact {
            canonical: Some(canonical),
            state: WorkerDependencyCandidateStateV1::Invalid,
            reason: Some("worker.dependencies.not_executable"),
        };
    }
    if !require_executable && !readable(&metadata) {
        return MetadataFact {
            canonical: Some(canonical),
            state: WorkerDependencyCandidateStateV1::Invalid,
            reason: Some("worker.dependencies.not_readable"),
        };
    }
    MetadataFact {
        canonical: Some(canonical),
        state: WorkerDependencyCandidateStateV1::Found,
        reason: None,
    }
}

fn normalize_required(path: &Path, require_executable: bool) -> Result<PathBuf, DelegationErrorV1> {
    let fact = inspect(path, require_executable);
    match fact.state {
        WorkerDependencyCandidateStateV1::Found => fact
            .canonical
            .ok_or(DelegationErrorV1::DependenciesUnavailable),
        WorkerDependencyCandidateStateV1::Missing => Err(DelegationErrorV1::DependenciesMissing),
        WorkerDependencyCandidateStateV1::Invalid => Err(DelegationErrorV1::DependenciesInvalid),
        WorkerDependencyCandidateStateV1::Unavailable => {
            Err(DelegationErrorV1::DependenciesUnavailable)
        }
    }
}

fn package_bin(package_json: &Path, expected_bin: &str) -> Result<Option<PathBuf>, ()> {
    let metadata = match fs::metadata(package_json) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !metadata.is_file() || metadata.len() > MAX_PACKAGE_BYTES {
        return Err(());
    }
    let bytes = fs::read(package_json).map_err(|_| ())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let entry = match value.get("bin") {
        Some(serde_json::Value::String(value)) => Some(value.as_str()),
        Some(serde_json::Value::Object(entries)) => entries
            .get(expected_bin)
            .and_then(serde_json::Value::as_str),
        _ => None,
    };
    let Some(entry) = entry else {
        return Ok(None);
    };
    let path = Path::new(entry);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(());
    }
    Ok(Some(path.to_path_buf()))
}

fn names(harness: WorkerHarnessV1) -> (&'static [&'static str], Option<&'static str>) {
    match harness {
        WorkerHarnessV1::CodexCli => (&["codex"], Some("codex-acp")),
        WorkerHarnessV1::ClaudeCode => (&["claude"], Some("claude-agent-acp")),
        WorkerHarnessV1::QoderCli => (&["qoder", "qodercli"], None),
    }
}

fn package_name(harness: WorkerHarnessV1) -> Option<&'static Path> {
    match harness {
        WorkerHarnessV1::CodexCli => Some(Path::new("@agentclientprotocol/codex-acp")),
        WorkerHarnessV1::ClaudeCode => Some(Path::new("@agentclientprotocol/claude-agent-acp")),
        WorkerHarnessV1::QoderCli => None,
    }
}

fn path_string(path: PathBuf) -> Result<String, DelegationErrorV1> {
    path.into_os_string()
        .into_string()
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 4096
                && !value.contains(char::is_control)
                && Path::new(value).is_absolute()
        })
        .ok_or(DelegationErrorV1::DependenciesInvalid)
}

fn executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn readable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o444 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "unsupported"
    }
}

#[cfg(test)]
#[path = "discovery/tests.rs"]
mod tests;
