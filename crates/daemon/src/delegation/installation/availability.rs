use hiroute_application_api::{
    WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1, WorkerExecutorAvailabilityListV1,
    WorkerExecutorAvailabilityReasonV1 as Reason, WorkerExecutorAvailabilityStateV1 as State,
    WorkerExecutorAvailabilityV1, WorkerExecutorCapabilityAvailabilityV1,
};
use hiroute_domain::delegation::WorkerHarnessV1;
use std::sync::Arc;

use super::{WorkerInstallationSelectionSource, check_installation};

pub(crate) struct WorkerExecutorAvailabilityRegistry {
    selections: Arc<dyn WorkerInstallationSelectionSource>,
}

#[cfg(test)]
struct NoWorkerInstallationSelections;

#[cfg(test)]
impl WorkerInstallationSelectionSource for NoWorkerInstallationSelections {
    fn selection(
        &self,
        _harness: WorkerHarnessV1,
    ) -> Result<
        Option<super::WorkerInstallationSelection>,
        hiroute_domain::delegation::DelegationErrorV1,
    > {
        Ok(None)
    }
}

impl WorkerExecutorAvailabilityRegistry {
    pub(crate) fn new(selections: Arc<dyn WorkerInstallationSelectionSource>) -> Self {
        Self { selections }
    }

    /// A test fixture with no configured selection. Production composition supplies the durable
    /// Local Control selection source instead.
    #[cfg(test)]
    pub(crate) fn unconfigured() -> Self {
        Self::new(Arc::new(NoWorkerInstallationSelections))
    }

    pub(crate) fn runtime_unavailable() -> WorkerExecutorAvailabilityListV1 {
        executor_list(
            [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode]
                .map(|harness| unavailable(harness, State::Unknown, Reason::RuntimeUnavailable)),
        )
    }

    pub(crate) fn snapshot(&self) -> WorkerExecutorAvailabilityListV1 {
        executor_list(
            [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode].map(|harness| {
                let selection = match self.selections.selection(harness) {
                    Ok(selection) => selection,
                    Err(_) => {
                        return unavailable(
                            harness,
                            State::Unavailable,
                            Reason::RuntimeUnavailable,
                        );
                    }
                };
                let Some(selection) = selection else {
                    return unavailable(
                        harness,
                        State::Unavailable,
                        Reason::InstallationNotConfigured,
                    );
                };
                if check_installation(&selection.config).is_err() {
                    return unavailable(harness, State::Unavailable, Reason::ArtifactUnavailable);
                }
                let ready = WorkerExecutorCapabilityAvailabilityV1 {
                    state: State::Ready,
                    reason: None,
                };
                WorkerExecutorAvailabilityV1 {
                    harness,
                    state: State::Ready,
                    reason: None,
                    start_approve_all: ready.clone(),
                    cancel: ready,
                    continue_session: WorkerExecutorCapabilityAvailabilityV1 {
                        state: State::Unknown,
                        reason: Some(Reason::CapabilityUnverified),
                    },
                    restricted_policy: WorkerExecutorCapabilityAvailabilityV1 {
                        state: State::Unknown,
                        reason: Some(Reason::RestrictedPolicyUnverified),
                    },
                }
            }),
        )
    }
}

fn unavailable(
    harness: WorkerHarnessV1,
    state: State,
    reason: Reason,
) -> WorkerExecutorAvailabilityV1 {
    let capability = WorkerExecutorCapabilityAvailabilityV1 {
        state,
        reason: Some(reason),
    };
    WorkerExecutorAvailabilityV1 {
        harness,
        state,
        reason: Some(reason),
        start_approve_all: capability.clone(),
        cancel: capability.clone(),
        continue_session: capability.clone(),
        restricted_policy: capability,
    }
}

fn executor_list(executors: [WorkerExecutorAvailabilityV1; 2]) -> WorkerExecutorAvailabilityListV1 {
    let list = WorkerExecutorAvailabilityListV1 {
        schema: WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1.into(),
        executors: executors.into(),
    };
    debug_assert!(list.valid());
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::installation::{WorkerInstallationConfig, WorkerInstallationSelection};
    use hiroute_domain::delegation::DelegationErrorV1;
    use std::fs;

    struct StaticSelections(Vec<WorkerInstallationConfig>);

    impl WorkerInstallationSelectionSource for StaticSelections {
        fn selection(
            &self,
            harness: WorkerHarnessV1,
        ) -> Result<Option<WorkerInstallationSelection>, DelegationErrorV1> {
            Ok(self
                .0
                .iter()
                .find(|config| config.harness == harness)
                .cloned()
                .map(|config| WorkerInstallationSelection {
                    config,
                    revision: 1,
                }))
        }
    }

    fn registry(
        configurations: Vec<WorkerInstallationConfig>,
    ) -> WorkerExecutorAvailabilityRegistry {
        WorkerExecutorAvailabilityRegistry::new(Arc::new(StaticSelections(configurations)))
    }

    #[test]
    #[cfg(unix)]
    fn worker_installation_accepts_every_diagnostic_version_without_running_it() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("worker");
        let marker = root.path().join("version-was-run");
        for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
            for output in ["0.0.1", "99.1.2", "", "not a version"] {
                fs::write(
                    &binary,
                    format!(
                        "#!/bin/sh\ntouch '{}'\nprintf '%s' '{}'\n",
                        marker.display(),
                        output
                    ),
                )
                .unwrap();
                fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
                let config = WorkerInstallationConfig {
                    harness,
                    adapter: binary.clone(),
                    harness_binary: binary.clone(),
                    node_binary: None,
                };
                assert!(check_installation(&config).is_ok());
                let snapshot = registry(vec![config]).snapshot();
                assert_eq!(
                    snapshot
                        .executors
                        .iter()
                        .find(|item| item.harness == harness)
                        .unwrap()
                        .state,
                    State::Ready
                );
                assert!(!marker.exists());
            }
        }
    }

    #[test]
    fn installed_is_launchable_not_behavior_verified_and_contents_are_not_pinned() {
        let root = tempfile::tempdir().unwrap();
        let adapter = root.path().join("adapter");
        fs::write(&adapter, b"not a working adapter").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&adapter, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = WorkerInstallationConfig {
            harness: WorkerHarnessV1::CodexCli,
            adapter: adapter.clone(),
            harness_binary: adapter.clone(),
            node_binary: None,
        };
        let registry = registry(vec![config]);
        let before = registry.snapshot();
        assert!(before.valid());
        assert_eq!(before.executors[0].state, State::Ready);
        assert_eq!(before.executors[0].continue_session.state, State::Unknown);
        assert_eq!(before.executors[0].restricted_policy.state, State::Unknown);
        assert_eq!(
            before.executors[1].reason,
            Some(Reason::InstallationNotConfigured)
        );
        fs::write(&adapter, b"replacement contents").unwrap();
        assert_eq!(registry.snapshot(), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&adapter, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(
                registry.snapshot().executors[0].reason,
                Some(Reason::ArtifactUnavailable)
            );
            fs::set_permissions(&adapter, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(registry.snapshot(), before);
        }
        fs::remove_file(&adapter).unwrap();
        assert_eq!(
            registry.snapshot().executors[0].reason,
            Some(Reason::ArtifactUnavailable)
        );
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_executable_is_not_opened_and_fifo_is_rejected_without_blocking() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("worker");
        fs::write(&binary, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o100)).unwrap();
        let config = WorkerInstallationConfig {
            harness: WorkerHarnessV1::ClaudeCode,
            adapter: binary.clone(),
            harness_binary: binary,
            node_binary: None,
        };
        assert!(check_installation(&config).is_ok());
        let fifo = root.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
        let registry = registry(vec![WorkerInstallationConfig {
            adapter: fifo,
            ..config
        }]);
        assert_eq!(
            registry.snapshot().executors[1].reason,
            Some(Reason::ArtifactUnavailable)
        );
    }
}
