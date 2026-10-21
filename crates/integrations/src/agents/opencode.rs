//! OpenCode's native path selectors and bounded static configuration, shared by consumers.
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use hiroute_domain::CanonicalDigest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentFilesystemScanError as Error, QoderNativeError, qoder_error};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeNativeContext {
    pub home: PathBuf,
    pub config_root: PathBuf,
    pub data_root: PathBuf,
    pub cache_root: PathBuf,
    pub state_root: PathBuf,
    pub config_file: Option<PathBuf>,
    pub extra_config_root: Option<PathBuf>,
    pub database: PathBuf,
}

impl OpenCodeNativeContext {
    pub fn from_environment(home: &Path) -> Result<Self, QoderNativeError> {
        Self::from_lookup(home, |name| std::env::var_os(name))
    }

    pub fn from_lookup(
        home: &Path,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, QoderNativeError> {
        checked_path(home)?;
        let root = |key: &str, fallback: &str| -> Result<PathBuf, QoderNativeError> {
            let path = lookup(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(fallback));
            checked_path(&path)?;
            Ok(path.join("opencode"))
        };
        let selected = |key: &str| -> Result<Option<PathBuf>, QoderNativeError> {
            lookup(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .map(|p| checked_path(&p).map(|_| p))
                .transpose()
        };
        let data_root = root("XDG_DATA_HOME", ".local/share")?;
        let database = match lookup("OPENCODE_DB").filter(|s| !s.is_empty()) {
            Some(value) => {
                if value == ":memory:" {
                    return Err(qoder_error("nonresumable OpenCode database"));
                }
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    path
                } else {
                    data_root.join(path)
                }
            }
            None => data_root.join("opencode.db"),
        };
        checked_path(&database)?;
        Ok(Self {
            home: home.to_owned(),
            config_root: root("XDG_CONFIG_HOME", ".config")?,
            data_root,
            cache_root: root("XDG_CACHE_HOME", ".cache")?,
            state_root: root("XDG_STATE_HOME", ".local/state")?,
            config_file: selected("OPENCODE_CONFIG")?,
            extra_config_root: selected("OPENCODE_CONFIG_DIR")?,
            database,
        })
    }

    pub fn validate(&self) -> Result<(), QoderNativeError> {
        for path in [
            &self.home,
            &self.config_root,
            &self.data_root,
            &self.cache_root,
            &self.state_root,
            &self.database,
        ]
        .into_iter()
        .chain(self.config_file.iter())
        .chain(self.extra_config_root.iter())
        {
            checked_path(path)?;
        }
        Ok(())
    }

    pub fn resource_root(&self) -> &Path {
        self.extra_config_root
            .as_deref()
            .unwrap_or(&self.config_root)
    }
    pub fn skill_target(&self) -> PathBuf {
        self.resource_root()
            .join("skills/hiroute-collaboration/SKILL.md")
    }
    pub fn model_target(&self) -> PathBuf {
        if let Some(path) = &self.config_file {
            return path.clone();
        }
        let root = self.resource_root();
        let jsonc = root.join("opencode.jsonc");
        let json = root.join("opencode.json");
        if !jsonc.exists() && json.exists() {
            json
        } else {
            jsonc
        }
    }

    /// Native merge order, restricted to the chosen user and optional project ancestry.
    /// Project inspection requires the actual filtered Worker PATH, not the host environment.
    pub fn config_paths(
        &self,
        workspace: Option<(&Path, &str)>,
    ) -> Result<Vec<PathBuf>, QoderNativeError> {
        let mut paths = vec![
            self.config_root.join("config.json"),
            self.config_root.join("opencode.json"),
            self.config_root.join("opencode.jsonc"),
        ];
        paths.extend(self.config_file.iter().cloned());
        if let Some((workspace, search_path)) = workspace {
            checked_path(workspace)?;
            let mut ancestry = project_ancestry(self, workspace, search_path)?;
            ancestry.reverse();
            for root in &ancestry {
                paths.extend([root.join("opencode.json"), root.join("opencode.jsonc")]);
            }
            for root in ancestry.iter().rev() {
                paths.extend([
                    root.join(".opencode/opencode.json"),
                    root.join(".opencode/opencode.jsonc"),
                ]);
            }
        }
        paths.extend([
            self.home.join(".opencode/opencode.json"),
            self.home.join(".opencode/opencode.jsonc"),
        ]);
        if let Some(root) = &self.extra_config_root {
            paths.extend([root.join("opencode.json"), root.join("opencode.jsonc")]);
        }
        let mut seen = BTreeSet::new();
        paths.retain(|p| seen.insert(p.clone()));
        Ok(paths)
    }

    pub fn has_remote_configuration(&self) -> Result<bool, Error> {
        if std::env::var_os("OPENCODE_CONFIG_CONTENT").is_some_and(|v| !v.is_empty())
            || std::env::var_os("OPENCODE_TEST_MANAGED_CONFIG_DIR").is_some()
        {
            return Ok(true);
        }
        #[cfg(target_os = "macos")]
        {
            let username = nix::unistd::User::from_uid(nix::unistd::getuid())
                .map_err(|_| Error::SourceUnavailable)?
                .ok_or(Error::SourceUnavailable)?
                .name;
            if [
                PathBuf::from("/Library/Application Support/opencode"),
                PathBuf::from("/Library/Managed Preferences/ai.opencode.managed.plist"),
                PathBuf::from("/Library/Managed Preferences")
                    .join(username)
                    .join("ai.opencode.managed.plist"),
            ]
            .iter()
            .any(|p| p.exists())
            {
                return Ok(true);
            }
        }
        #[cfg(target_os = "linux")]
        if Path::new("/etc/opencode").exists() {
            return Ok(true);
        }
        #[cfg(windows)]
        if std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("opencode")
            .exists()
        {
            return Ok(true);
        }

        // Native account configuration is stored in SQLite, not a guessed account file.
        // Read only the active-organization flag; never credentials or conversation rows.
        if self
            .database
            .try_exists()
            .map_err(|_| Error::SourceUnavailable)?
        {
            let meta =
                std::fs::symlink_metadata(&self.database).map_err(|_| Error::SourceUnavailable)?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(Error::InvalidConfig);
            }
            let db = rusqlite::Connection::open_with_flags(
                &self.database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|_| Error::SourceUnavailable)?;
            db.busy_timeout(std::time::Duration::from_millis(100))
                .map_err(|_| Error::SourceUnavailable)?;
            let remote: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM account_state WHERE id=1 AND active_account_id IS NOT NULL AND active_org_id IS NOT NULL)",
                [], |r| r.get(0)).map_err(|_| Error::InvalidConfig)?;
            if remote {
                return Ok(true);
            }
        }
        if let Some((bytes, _)) =
            super::filesystem_config::read_system_config_bytes(&self.data_root.join("auth.json"))?
        {
            let mut value: Value =
                serde_json::from_slice(&bytes).map_err(|_| Error::InvalidConfig)?;
            let remote = value
                .as_object()
                .is_none_or(|o| o.values().any(|v| v["type"] == "wellknown"));
            super::pi_sources::clear_native_value(&mut value);
            return Ok(remote);
        }
        Ok(false)
    }

    pub fn read_config(&self, workspace: Option<(&Path, &str)>) -> Result<OpenCodeConfig, Error> {
        let mut value = serde_json::json!({});
        let mut documents = Vec::new();
        let mut schemas_complete = true;
        let mut executable_plugins = false;
        for path in self
            .config_paths(workspace)
            .map_err(|_| Error::InvalidConfig)?
        {
            let raw = super::filesystem_config::read_system_config_bytes(&path)?;
            let Some((bytes, _)) = raw else {
                continue;
            };
            let parsed = parse_jsonc(&bytes).map_err(|_| Error::InvalidConfig)?;
            executable_plugins |= parsed
                .get("plugin")
                .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()));
            // The native loader expands substitutions before testing schema truthiness
            // and writes back to the borrowed file when the result is empty.
            schemas_complete &= parsed
                .get("$schema")
                .and_then(Value::as_str)
                .is_some_and(|schema| !schema.is_empty() && !substitution(schema));
            merge(&mut value, parsed);
            documents.push((path, CanonicalDigest::of_bytes(&bytes)));
        }
        for root in [self.config_root.clone(), self.home.join(".opencode")]
            .into_iter()
            .chain(self.extra_config_root.iter().cloned())
        {
            for name in ["plugin", "plugins"] {
                match std::fs::read_dir(root.join(name)) {
                    Ok(entries) => {
                        for entry in entries {
                            let entry = entry.map_err(|_| Error::SourceUnavailable)?;
                            let bundled = entry.file_name() == "hiroute-routing.js"
                                && entry
                                    .file_type()
                                    .map_err(|_| Error::SourceUnavailable)?
                                    .is_file()
                                && super::filesystem_config::read_system_config_bytes(
                                    &entry.path(),
                                )?
                                .is_some_and(|(bytes, _)| {
                                    bytes.as_slice() == super::OPENCODE_ROUTING_ADAPTER.as_bytes()
                                });
                            executable_plugins |= !bundled;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(Error::SourceUnavailable),
                }
            }
        }
        let digest = CanonicalDigest::of(&documents).map_err(|_| Error::InvalidConfig)?;
        Ok(OpenCodeConfig {
            value,
            documents,
            digest,
            schemas_complete,
            executable_plugins,
        })
    }

    pub(super) fn resource_roots(
        &self,
        workspace: Option<(&Path, &str)>,
    ) -> Result<Vec<PathBuf>, QoderNativeError> {
        let mut roots = vec![self.config_root.clone()];
        if let Some((workspace, search_path)) = workspace {
            roots.extend(
                project_ancestry(self, workspace, search_path)?
                    .iter()
                    .map(|p| p.join(".opencode")),
            );
        }
        roots.push(self.home.join(".opencode"));
        roots.extend(self.extra_config_root.iter().cloned());
        let mut seen = BTreeSet::new();
        roots.retain(|p| seen.insert(p.clone()));
        Ok(roots)
    }
}

pub struct OpenCodeConfig {
    pub value: Value,
    pub documents: Vec<(PathBuf, CanonicalDigest)>,
    pub digest: CanonicalDigest,
    pub schemas_complete: bool,
    pub executable_plugins: bool,
}
impl Drop for OpenCodeConfig {
    fn drop(&mut self) {
        super::pi_sources::clear_native_value(&mut self.value);
    }
}

pub(super) fn parse_jsonc(bytes: &[u8]) -> Result<Value, QoderNativeError> {
    super::additional_native::parse_opencode_jsonc(bytes)
}

fn merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in right {
                if key == "instructions"
                    && let (Some(old), Some(new)) = (
                        left.get_mut(&key).and_then(Value::as_array_mut),
                        value.as_array(),
                    )
                {
                    for item in new {
                        if !old.contains(item) {
                            old.push(item.clone());
                        }
                    }
                } else {
                    merge(left.entry(key).or_insert(Value::Null), value);
                }
            }
        }
        (target, source) => *target = source,
    }
}

fn checked_path(path: &Path) -> Result<(), QoderNativeError> {
    if !path.is_absolute()
        || path.components().any(|p| matches!(p, Component::ParentDir))
        || path.to_str().is_none_or(|s| s.contains(['\0', '\r', '\n']))
    {
        return Err(qoder_error("OpenCode context path"));
    }
    Ok(())
}

/// A bounded inspection is complete only at the same worktree/root stop as native.
fn project_ancestry(
    native: &OpenCodeNativeContext,
    workspace: &Path,
    search_path: &str,
) -> Result<Vec<PathBuf>, QoderNativeError> {
    checked_path(workspace)?;
    let mut roots = Vec::new();
    for root in workspace.ancestors().take(64) {
        roots.push(root.to_owned());
        if root
            .join(".git")
            .try_exists()
            .map_err(|_| qoder_error("OpenCode project ancestry"))?
        {
            super::opencode_project::confirm_root(native, root, search_path)?;
            return Ok(roots);
        }
        if root.parent().is_none() {
            return Ok(roots);
        }
    }
    Err(qoder_error("OpenCode project ancestry bound"))
}

/// Shared native SDK declaration. Ordinary grants use their independent fixed header.
pub fn opencode_provider(
    endpoint: &str,
    model: &hiroute_domain::AdditionalAgentModelV1,
    api_key: &str,
    local_grant: Option<&str>,
) -> Result<Value, QoderNativeError> {
    model
        .validate_pi()
        .map_err(|_| qoder_error("OpenCode budget"))?;
    let sdk = match model.protocol {
        hiroute_domain::AgentIngressProtocolV1::Responses => "@ai-sdk/openai",
        hiroute_domain::AgentIngressProtocolV1::Messages => "@ai-sdk/anthropic",
        hiroute_domain::AgentIngressProtocolV1::ChatCompletions => {
            return Err(qoder_error("OpenCode routing protocol"));
        }
    };
    let mut options = serde_json::json!({"baseURL":endpoint,"apiKey":api_key});
    if let Some(grant) = local_grant {
        options["headers"] =
            serde_json::json!({"X-HiRoute-Token":grant,"X-HiRoute-Agent":"opencode"});
    }
    Ok(
        serde_json::json!({"npm":sdk,"name":"HiRoute","options":options,"models":{(model.alias.clone()):{
            "id":model.alias,"name":model.alias,"tool_call":true,"modalities":{"input":["text"],"output":["text"]},
            "limit":{"context":model.context_window_tokens,"output":model.max_output_tokens},"options":{"store":false}
        }}}),
    )
}

fn substitution(value: &str) -> bool {
    value.contains("{env:") || value.contains("{file:")
}

/// Native substitutions run before JSON parsing, including inside object keys.
/// Keep ambiguous structure outside the bounded reader rather than guessing its expansion.
pub(super) fn dynamic_keys(value: &Value) -> bool {
    match value {
        Value::Object(entries) => entries
            .iter()
            .any(|(key, value)| substitution(key) || dynamic_keys(value)),
        Value::Array(values) => values.iter().any(dynamic_keys),
        _ => false,
    }
}

pub(super) fn validate_references(
    value: &Value,
    provider: &str,
    provider_value: &Value,
    retain: &BTreeSet<String>,
    resource_models: Option<&[String]>,
) -> Result<(), QoderNativeError> {
    if dynamic_keys(value) {
        return Err(qoder_error("OpenCode dynamic configuration keys"));
    }
    let removes_model = provider_value["models"].as_object().is_none_or(|models| {
        models
            .keys()
            .any(|model| !retain.contains(&format!("{provider}/{model}")))
    });
    let mut refs = Vec::new();
    for key in ["model", "small_model"] {
        if let Some(v) = value.get(key) {
            refs.push(v);
        }
    }
    for key in ["agent", "mode"] {
        if let Some(entries) = value.get(key) {
            for entry in entries
                .as_object()
                .ok_or_else(|| qoder_error("OpenCode agents"))?
                .values()
            {
                if let Some(v) = entry.get("model") {
                    refs.push(v);
                }
            }
        }
    }
    if removes_model && resource_models.is_none() {
        // Unknown native frontmatter cannot authorize deleting a model selector.
        return Err(qoder_error("default in use"));
    }
    let check = |selector: &str| {
        if removes_model && substitution(selector) {
            return Err(qoder_error("OpenCode dynamic model selector"));
        }
        if selector.starts_with(&format!("{provider}/")) && !retain.contains(selector) {
            return Err(qoder_error("default in use"));
        }
        Ok(())
    };
    for value in refs {
        check(
            value
                .as_str()
                .ok_or_else(|| qoder_error("OpenCode model selector"))?,
        )?;
    }
    for selector in resource_models.unwrap_or_default() {
        check(selector)?;
    }
    Ok(())
}

pub fn validate_opencode_composition(
    native: &OpenCodeNativeContext,
    config: &OpenCodeConfig,
    workspace: &Path,
    search_path: &str,
) -> Result<(), QoderNativeError> {
    let unavailable = || qoder_error("unsupported OpenCode native composition");
    if dynamic_keys(&config.value)
        || std::env::var_os("OPENCODE_TEST_MANAGED_CONFIG_DIR").is_some()
        || std::env::var_os("OPENCODE_CONFIG_CONTENT").is_some_and(|v| !v.is_empty())
        || config
            .value
            .get("mode")
            .is_some_and(|v| v.as_object().is_none_or(|o| !o.is_empty()))
        || config.value["skills"]
            .get("urls")
            .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
    {
        return Err(unavailable());
    }
    if config.value.get("enabled_providers").is_some() || config.value.get("server").is_some() {
        return Err(unavailable());
    }
    if !config.schemas_complete
        || native
            .has_remote_configuration()
            .map_err(|_| unavailable())?
    {
        return Err(unavailable());
    }
    if config.value["agent"].as_object().is_some_and(|agents| {
        agents.values().any(|agent| {
            ["options", "permission", "tools", "mode"]
                .iter()
                .any(|key| agent.get(key).is_some())
        })
    }) {
        return Err(unavailable());
    }
    if native.config_root.join("config").exists() {
        return Err(unavailable());
    }
    for root in native.resource_roots(Some((workspace, search_path)))? {
        for dir in ["tool", "tools", "agent", "agents", "mode", "modes"] {
            match std::fs::read_dir(root.join(dir)) {
                Ok(entries) => {
                    if entries.take(1).next().is_some() {
                        return Err(unavailable());
                    }
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(unavailable()),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_ancestry_requires_a_proven_native_stop_within_the_bound() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let native = OpenCodeNativeContext::from_lookup(&base, |_| None).unwrap();
        let search_path = super::super::native_ingress_probe::native_probe_path()
            .into_string()
            .unwrap();
        git(&native, &base, &["init", "-q"], &search_path);
        let at_limit = (0..63).fold(base.clone(), |path, _| path.join("d"));
        std::fs::create_dir_all(&at_limit).unwrap();
        assert_eq!(
            project_ancestry(&native, &at_limit, &search_path)
                .unwrap()
                .len(),
            64
        );
        assert!(project_ancestry(&native, &at_limit.join("d"), &search_path).is_err());
        std::fs::remove_dir_all(base.join(".git")).unwrap();
        assert!(project_ancestry(&native, &at_limit, &search_path).is_err());
    }

    fn git(native: &OpenCodeNativeContext, cwd: &Path, args: &[&str], path: &str) {
        let candidate = std::env::split_paths(std::ffi::OsStr::new(path))
            .map(|dir| dir.join(if cfg!(windows) { "git.exe" } else { "git" }))
            .find(|path| path.is_file())
            .unwrap();
        let binary = super::super::executable::resolve(&candidate)
            .unwrap()
            .unwrap();
        let mut command = std::process::Command::new(binary);
        command
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("HOME", &native.home)
            .env("PATH", path);
        super::super::native_probe_process::capture(
            command,
            false,
            std::time::Duration::from_secs(5),
            4096,
        )
        .unwrap();
    }

    #[test]
    fn opencode_real_git_and_linked_worktree_stop_before_foreign_ancestor_config_and_tools() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let home = base.join("home");
        #[cfg(not(windows))]
        let repository = base.join("repository space'Ω ");
        #[cfg(windows)]
        let repository = base.join("repository space'Ω");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&repository).unwrap();
        let native = OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
        let search_path = super::super::native_ingress_probe::native_probe_path()
            .into_string()
            .unwrap();
        git(&native, &repository, &["init", "-q"], &search_path);
        git(
            &native,
            &repository,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
            &search_path,
        );
        let linked = base.join("linked");
        git(
            &native,
            &repository,
            &[
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD",
            ],
            &search_path,
        );
        let foreign_config = base.join("opencode.json");
        let foreign_bytes = br#"{"$schema":"","server":{"port":9}}"#;
        std::fs::write(&foreign_config, foreign_bytes).unwrap();
        std::fs::create_dir_all(base.join(".opencode/tools")).unwrap();
        let tool = base.join(".opencode/tools/custom.ts");
        std::fs::write(&tool, b"throw new Error('outside confirmed native root')").unwrap();
        for project in [&repository, &linked] {
            let workspace = project.join("nested");
            std::fs::create_dir(&workspace).unwrap();
            let config = native
                .read_config(Some((&workspace, &search_path)))
                .unwrap();
            assert!(
                validate_opencode_composition(&native, &config, &workspace, &search_path).is_ok()
            );
            assert!(
                !config
                    .documents
                    .iter()
                    .any(|(path, _)| path == &foreign_config)
            );
            assert_eq!(
                project_ancestry(&native, &workspace, &search_path).unwrap(),
                [workspace.clone(), project.to_owned()]
            );
            #[cfg(windows)]
            {
                // The production workspace owner uses ordinary Win32 spelling;
                // Rust canonicalize returns a verbatim spelling of the same root.
                let text = workspace.to_str().unwrap();
                let ordinary = Path::new(text.strip_prefix(r"\\?\").unwrap_or(text));
                assert!(project_ancestry(&native, ordinary, &search_path).is_ok());
            }
            assert_eq!(std::fs::read(&foreign_config).unwrap(), foreign_bytes);
            assert!(!native.database.exists());
        }
        // Git can report a different worktree through local configuration. On
        // Unix its trailing space must not be trimmed into the marker's root.
        #[cfg(not(windows))]
        let redirected = PathBuf::from(format!("{} ", repository.display()));
        #[cfg(windows)]
        let redirected = base.join("redirected-worktree");
        std::fs::create_dir(&redirected).unwrap();
        git(
            &native,
            &repository,
            &["config", "core.worktree", redirected.to_str().unwrap()],
            &search_path,
        );
        assert!(
            native
                .read_config(Some((&repository, &search_path)))
                .is_err()
        );
    }

    #[test]
    fn context_selectors_are_additive_and_database_is_data_relative() {
        let home = super::super::test_absolute_path("/native");
        let extra = home.join("extra");
        let context = OpenCodeNativeContext::from_lookup(&home, |key| match key {
            "OPENCODE_CONFIG_DIR" => Some(extra.clone().into_os_string()),
            "OPENCODE_DB" => Some("worker.db".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(context.config_root, home.join(".config/opencode"));
        assert_eq!(
            context.database,
            home.join(".local/share/opencode/worker.db")
        );
        assert_eq!(
            context.skill_target(),
            extra.join("skills/hiroute-collaboration/SKILL.md")
        );
        assert!(
            context
                .config_paths(None)
                .unwrap()
                .contains(&home.join(".config/opencode/config.json"))
        );
        assert!(OpenCodeNativeContext::from_lookup(&home, |_| Some("relative".into())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn opencode_git_fixtures_filter_relative_host_paths_in_an_isolated_child() {
        let inherited = std::env::var_os("PATH").unwrap();
        let path = std::env::join_paths(
            [PathBuf::from("."), PathBuf::from("relative/bin")]
                .into_iter()
                .chain(std::env::split_paths(&inherited)),
        )
        .unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "agents::opencode::tests::opencode_real_git_and_linked_worktree_stop_before_foreign_ancestor_config_and_tools"])
            .env("PATH", path);
        let output = super::super::native_probe_process::capture(
            command,
            true,
            std::time::Duration::from_secs(15),
            16 * 1024,
        )
        .unwrap();
        assert!(
            std::str::from_utf8(&output)
                .unwrap()
                .contains("1 passed; 0 failed")
        );
    }
}
