//! The instance's native context is borrowed; it is never a cleanup target.
use super::{DelegationErrorV1, Path, PathBuf, WorkerHarnessV1, path_string};
use std::ffi::OsString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWorkerContext {
    home: PathBuf,
    config_root: PathBuf,
    borrowed: bool,
}

impl NativeWorkerContext {
    /// Resolve only context selectors, never credentials or the entire service environment.
    /// An explicit Pilot/test HOME is the effective user context, not a hint to search elsewhere.
    pub fn from_environment(harness: WorkerHarnessV1) -> Result<Self, DelegationErrorV1> {
        Self::from_lookup(harness, |name| std::env::var_os(name))
    }

    fn from_lookup(
        harness: WorkerHarnessV1,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, DelegationErrorV1> {
        let home = lookup("HOME")
            .map(PathBuf::from)
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        let (variable, default) = match harness {
            WorkerHarnessV1::CodexCli => ("CODEX_HOME", ".codex"),
            WorkerHarnessV1::ClaudeCode => ("CLAUDE_CONFIG_DIR", ".claude"),
            WorkerHarnessV1::Pi => ("PI_CODING_AGENT_DIR", ".pi/agent"),
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
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_home_and_explicit_native_roots_are_respected_without_reading_other_env() {
        for (harness, variable, default) in [
            (WorkerHarnessV1::CodexCli, "CODEX_HOME", ".codex"),
            (WorkerHarnessV1::ClaudeCode, "CLAUDE_CONFIG_DIR", ".claude"),
            (WorkerHarnessV1::QoderCli, "QODER_CONFIG_DIR", ".qoder"),
        ] {
            let lookup = |name: &str| match name {
                "HOME" => Some(OsString::from("/isolated-pilot")),
                key if key == variable => Some(OsString::from("/explicit-native-root")),
                unexpected => panic!("unexpected ambient environment read: {unexpected}"),
            };
            let explicit = NativeWorkerContext::from_lookup(harness, lookup).unwrap();
            assert_eq!(explicit.home(), Path::new("/isolated-pilot"));
            assert_eq!(explicit.config_root(), Path::new("/explicit-native-root"));
            assert!(explicit.is_borrowed());
            let fallback = NativeWorkerContext::from_lookup(harness, |name| {
                (name == "HOME").then(|| OsString::from("/isolated-pilot"))
            })
            .unwrap();
            assert_eq!(
                fallback.config_root(),
                Path::new("/isolated-pilot").join(default)
            );
        }
    }

    #[test]
    fn missing_or_invalid_context_does_not_fall_back_to_another_users_home() {
        assert_eq!(
            NativeWorkerContext::from_lookup(WorkerHarnessV1::CodexCli, |_| None),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
        for invalid in ["", "relative", "/root/../elsewhere", "/root\nother"] {
            assert_eq!(
                NativeWorkerContext::borrowed(Path::new(invalid), Path::new("/native")),
                Err(DelegationErrorV1::InvalidArguments)
            );
            assert_eq!(
                NativeWorkerContext::borrowed(Path::new("/instance"), Path::new(invalid)),
                Err(DelegationErrorV1::InvalidArguments)
            );
        }
    }
}
