//! The instance's native context is borrowed; it is never a cleanup target.
use super::{DelegationErrorV1, Path, PathBuf, WorkerHarnessV1, path_string};
use std::ffi::OsString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWorkerContext {
    home: PathBuf,
    config_root: PathBuf,
    borrowed: bool,
    opencode: Option<hiroute_integrations::OpenCodeNativeContext>,
}

impl NativeWorkerContext {
    /// Resolve only context selectors, never credentials or the entire service environment.
    /// Unix retains the instance HOME; Windows uses the native account profile.
    pub fn from_environment(harness: WorkerHarnessV1) -> Result<Self, DelegationErrorV1> {
        #[cfg(windows)]
        {
            let home = {
                #[cfg(test)]
                if std::env::var_os("HIROUTE_ISOLATED_DAEMON_TEST")
                    .is_some_and(|selector| !selector.is_empty())
                {
                    let home = std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .filter(|path| path.is_absolute())
                        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
                    hiroute_platform::windows::security::verify_owner_only(&home, true)
                        .map_err(|_| DelegationErrorV1::PermissionDenied)?;
                    home
                } else {
                    hiroute_platform::windows::background::profile()
                        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?
                }
                #[cfg(not(test))]
                hiroute_platform::windows::background::profile()
                    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?
            };
            return Self::from_lookup(harness, |name| {
                if name == "HOME" {
                    Some(home.clone().into_os_string())
                } else {
                    std::env::var_os(name)
                }
            });
        }
        #[cfg(not(windows))]
        Self::from_lookup(harness, |name| std::env::var_os(name))
    }

    fn from_lookup(
        harness: WorkerHarnessV1,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, DelegationErrorV1> {
        let home = lookup("HOME");
        #[cfg(windows)]
        let home = home.or_else(|| lookup("USERPROFILE"));
        let home = home
            .map(PathBuf::from)
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        if harness == WorkerHarnessV1::OpenCode {
            let context = hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, lookup)
                .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
            return Self::from_opencode(context);
        }
        let (variable, default) = match harness {
            WorkerHarnessV1::CodexCli => ("CODEX_HOME", ".codex"),
            WorkerHarnessV1::ClaudeCode => ("CLAUDE_CONFIG_DIR", ".claude"),
            WorkerHarnessV1::Pi => ("PI_CODING_AGENT_DIR", ".pi/agent"),
            WorkerHarnessV1::DeepseekHarness => ("DSH_HOME", ".dsh"),
            WorkerHarnessV1::OpenCode => return Err(DelegationErrorV1::CapabilityUnavailable),
            WorkerHarnessV1::QoderCli => {
                let selected = lookup("QODER_CONFIG_DIR").map(PathBuf::from);
                let context = hiroute_integrations::agents::QoderNativeContext::from_selected(
                    &home,
                    selected.as_deref(),
                )
                .map_err(|_| DelegationErrorV1::InvalidArguments)?;
                return Self::borrowed(&context.home, &context.config_root);
            }
        };
        let config_root = lookup(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default));
        Self::borrowed(&home, &config_root)
    }

    /// Reconstruct an already bound context without consulting today's environment on Continue.
    pub fn borrowed(home: &Path, config_root: &Path) -> Result<Self, DelegationErrorV1> {
        Self::new(home.to_owned(), config_root.to_owned(), true)
    }

    /// Explicit legacy/probe context. Its directories remain owned by the existing material path.
    pub fn isolated(private_root: &Path, session_root: &Path) -> Result<Self, DelegationErrorV1> {
        Self::new(private_root.join("home"), session_root.to_owned(), false)
    }

    fn new(home: PathBuf, config_root: PathBuf, borrowed: bool) -> Result<Self, DelegationErrorV1> {
        let home = native_path(&home);
        let config_root = native_path(&config_root);
        for path in [&home, &config_root] {
            if !path.is_absolute()
                || path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
            {
                return Err(DelegationErrorV1::InvalidArguments);
            }
            path_string(path)?;
        }
        Ok(Self {
            home,
            config_root,
            borrowed,
            opencode: None,
        })
    }

    pub fn from_opencode(
        context: hiroute_integrations::OpenCodeNativeContext,
    ) -> Result<Self, DelegationErrorV1> {
        context
            .validate()
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let mut native = Self::borrowed(&context.home, &context.config_root)?;
        native.opencode = Some(context);
        Ok(native)
    }
    pub fn opencode(&self) -> Option<&hiroute_integrations::OpenCodeNativeContext> {
        self.opencode.as_ref()
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    pub fn is_borrowed(&self) -> bool {
        self.borrowed
    }
}

#[cfg(windows)]
pub(crate) fn windows_workspace(
    path: &str,
) -> Result<(hiroute_domain::delegation::DelegationWorkspaceV1, String), DelegationErrorV1> {
    use hiroute_platform::windows::security::{file_identity, open_identity};
    // Inspect each absolute component through held directory handles before accepting it.
    let requested = std::path::Path::new(path);
    let handle = open_identity(requested, true).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    let (volume, index) =
        file_identity(&handle).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    let canonical = native_path(
        &std::fs::canonicalize(requested).map_err(|_| DelegationErrorV1::InvalidArguments)?,
    );
    let current =
        open_identity(&canonical, true).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    if file_identity(&current).map_err(|_| DelegationErrorV1::InvalidArguments)? != (volume, index)
    {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut ancestry = Vec::new();
    for ancestor in canonical.ancestors() {
        let file =
            open_identity(ancestor, true).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let (volume, index) =
            file_identity(&file).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        ancestry.push(format!("file/{volume}/{index}"));
    }
    ancestry.reverse();
    let root_identity = format!("file/{volume}/{index}");
    if ancestry.last() != Some(&root_identity) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let workspace = hiroute_domain::delegation::DelegationWorkspaceV1 {
        root_identity,
        volume_identity: format!("volume/{volume}"),
        ancestry,
    };
    workspace.validate()?;
    Ok((
        workspace,
        canonical
            .to_str()
            .ok_or(DelegationErrorV1::InvalidArguments)?
            .to_owned(),
    ))
}

/// Compare directory objects through the existing workspace admission boundary. Ordinary
/// and verbatim Win32 spellings can name the same object; spelling alone is not identity.
#[cfg(windows)]
pub(crate) fn same_windows_directory(left: &Path, right: &Path) -> Result<bool, DelegationErrorV1> {
    let left = windows_workspace(left.to_str().ok_or(DelegationErrorV1::InvalidArguments)?)?.0;
    let right = windows_workspace(right.to_str().ok_or(DelegationErrorV1::InvalidArguments)?)?.0;
    Ok(left == right)
}

/// Native Node/CLI cwd and history encoders use ordinary Win32 path spelling.
/// Rust canonicalize's verbatim prefix is an I/O detail, not native history identity.
pub(crate) fn native_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{path}"));
        }
        if let Some(path) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(path);
        }
    }
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_home_and_explicit_native_roots_are_respected_without_reading_other_env() {
        #[cfg(windows)]
        let (home, root) = ("C:\\isolated-pilot", "C:\\explicit-native-root");
        #[cfg(not(windows))]
        let (home, root) = ("/isolated-pilot", "/explicit-native-root");
        for (harness, variable, default) in [
            (WorkerHarnessV1::CodexCli, "CODEX_HOME", ".codex"),
            (WorkerHarnessV1::ClaudeCode, "CLAUDE_CONFIG_DIR", ".claude"),
            (WorkerHarnessV1::QoderCli, "QODER_CONFIG_DIR", ".qoder"),
        ] {
            let lookup = |name: &str| match name {
                "HOME" => Some(OsString::from(home)),
                key if key == variable => Some(OsString::from(root)),
                unexpected => panic!("unexpected ambient environment read: {unexpected}"),
            };
            let explicit = NativeWorkerContext::from_lookup(harness, lookup).unwrap();
            assert_eq!(explicit.home(), Path::new(home));
            assert_eq!(explicit.config_root(), Path::new(root));
            assert!(explicit.is_borrowed());
            let fallback = NativeWorkerContext::from_lookup(harness, |name| {
                (name == "HOME").then(|| OsString::from(home))
            })
            .unwrap();
            assert_eq!(fallback.config_root(), Path::new(home).join(default));
        }
    }

    #[test]
    fn missing_or_invalid_context_does_not_fall_back_to_another_users_home() {
        assert_eq!(
            NativeWorkerContext::from_lookup(WorkerHarnessV1::CodexCli, |_| None),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
        #[cfg(windows)]
        let (instance, native, parent, newline) = (
            "C:\\instance",
            "C:\\native",
            "C:\\root\\..\\elsewhere",
            "C:\\root\nother",
        );
        #[cfg(not(windows))]
        let (instance, native, parent, newline) =
            ("/instance", "/native", "/root/../elsewhere", "/root\nother");
        for invalid in ["", "relative", parent, newline] {
            assert_eq!(
                NativeWorkerContext::borrowed(Path::new(invalid), Path::new(native)),
                Err(DelegationErrorV1::InvalidArguments)
            );
            assert_eq!(
                NativeWorkerContext::borrowed(Path::new(instance), Path::new(invalid)),
                Err(DelegationErrorV1::InvalidArguments)
            );
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn windows_context_respects_home_and_only_falls_back_to_current_userprofile() {
        let fixture = tempfile::tempdir().unwrap();
        let explicit = fixture.path().join("explicit-home");
        let profile = fixture.path().join("userprofile");
        let context =
            NativeWorkerContext::from_lookup(WorkerHarnessV1::CodexCli, |name| match name {
                "HOME" => Some(explicit.clone().into_os_string()),
                "USERPROFILE" => panic!("HOME takes precedence"),
                _ => None,
            })
            .unwrap();
        assert_eq!(context.home(), explicit);
        let context =
            NativeWorkerContext::from_lookup(WorkerHarnessV1::CodexCli, |name| match name {
                "HOME" => None,
                "USERPROFILE" => Some(profile.clone().into_os_string()),
                _ => None,
            })
            .unwrap();
        assert_eq!(context.home(), profile);
        assert_eq!(context.config_root(), profile.join(".codex"));
    }

    #[test]
    fn workspace_identity_uses_native_objects_without_verbatim_history_prefix() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("workspace % ! 空 格");
        std::fs::create_dir(&path).unwrap();
        let extended = std::fs::canonicalize(&path).unwrap();
        let (first, native) = windows_workspace(extended.to_str().unwrap()).unwrap();
        assert!(!native.starts_with(r"\\?\"));
        assert_eq!(windows_workspace(&native).unwrap().0, first);
        let renamed = fixture.path().join("old-workspace");
        std::fs::rename(&path, &renamed).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_ne!(windows_workspace(path.to_str().unwrap()).unwrap().0, first);
    }

    #[test]
    fn same_directory_accepts_verbatim_spelling_but_rejects_a_replaced_object() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("workspace % ! 空 格");
        std::fs::create_dir(&path).unwrap();
        let extended = std::fs::canonicalize(&path).unwrap();
        let ordinary = native_path(&extended);
        assert_ne!(extended, ordinary);
        assert!(same_windows_directory(&extended, &ordinary).unwrap());

        let previous = fixture.path().join("previous-workspace");
        std::fs::rename(&ordinary, &previous).unwrap();
        std::fs::create_dir(&ordinary).unwrap();
        assert!(!same_windows_directory(&previous, &ordinary).unwrap());
        assert!(same_windows_directory(&ordinary, &ordinary.join("missing")).is_err());
    }
}
