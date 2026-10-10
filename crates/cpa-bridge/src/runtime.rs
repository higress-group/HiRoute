use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hiroute_diagnostics::event::{
    CpaFailureCode, CpaLifecycle, CpaPhase, CpaStage, CpaStageKind, CpaStageOutcome,
    DiagnosticEvent,
};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_integrations::{
    CpaAccountMaterializationV1, CpaRegisteredSourceV1, CpaSupervisorError, CpaSupervisorPort,
    TrustedReleaseCatalog, register_cpa_account,
};
use parking_lot::Mutex;

use crate::BorrowedSubscriptionEvidence;
use crate::MANAGED_CPA_ARTIFACT_VERSION;
use crate::accounts::{
    AccountDiscoveryError, AccountSnapshotRecord, CpaControlPlane, StockCpaControlPlane,
};
use crate::artifact::{CpaArtifactError, CpaBinaryLocator, VerifiedCpaBinary};
use crate::borrowed_codex::ManagedAuthLease;
use crate::config::{
    InstanceSecrets, SecretText, ensure_private_dir, private_atomic_write, render_managed_config,
    validate_private_file,
};
use crate::errors::CpaLifecycleError;
use crate::owner::{OwnerClaim, OwnerLease, OwnerRecord};
use crate::process::{
    CpaExit, CpaLaunch, CpaProcessBackend, CpaProcessHandle, StdCpaProcessBackend,
};
use crate::state::{load_account_state, merge_accounts, save_account_state};

mod admission;
mod oauth;
mod recovery;
mod routing_batch;
mod spec;
mod subscriptions;
pub use oauth::{CpaOAuthLogin, CpaOAuthStatus};
pub use routing_batch::CpaRoutingBatch;

use spec::validate_spec;
pub use spec::{CpaRuntimeSpec, RestartPolicy};
pub use subscriptions::CpaSourceManagementState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpaHealth {
    Stopped {
        last_exit: Option<CpaExit>,
    },
    Ready {
        pid: u32,
        address: SocketAddr,
        restart_count: u64,
        adopted: bool,
    },
    Crashed {
        exit: CpaExit,
        restart_count: u64,
    },
    Unhealthy {
        pid: u32,
        restart_count: u64,
    },
    CrashLoop {
        last_exit: CpaExit,
        restart_count: u64,
    },
}

pub struct ManagedCpaRuntime {
    pub(crate) spec: CpaRuntimeSpec,
    pub(crate) catalog: Arc<TrustedReleaseCatalog>,
    locator: Arc<dyn CpaBinaryLocator>,
    backend: Arc<dyn CpaProcessBackend>,
    control: Arc<dyn CpaControlPlane>,
    // Serializes the sole process/auth-file owner, never execution admission.
    pub(crate) inner: Mutex<RuntimeInner>,
    admission: admission::Admission,
    observation: Mutex<admission::Observation>,
    health_invalidated: AtomicBool,
    pub(crate) epochs: Arc<RuntimeEpochState>,
    /// Lifecycle diagnostics; a no-op port keeps library and test hosts unchanged.
    diagnostics: DiagnosticsPort,
    proxy_environment: crate::proxy_environment::ProxyEnvironment,
}

/// Read-only discovery boundary exposed to Local Control. The returned values contain only
/// catalog-validated source facts and opaque credential references; OAuth material never crosses
/// this interface.
pub trait CpaRegisteredSourcePort: Send + Sync {
    /// Best-effort aggregate facts. Exact selections must use provider discovery
    /// so a failed sibling cannot turn an unavailable source into a missing one.
    fn discover_registered_sources(&self) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError>;

    fn discover_registered_sources_for(
        &self,
        kind: crate::CpaAccountKind,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        self.discover_registered_sources().map(|sources| {
            sources
                .into_iter()
                .filter(|source| source.source.connector_id == kind.connector_id())
                .collect()
        })
    }

    fn begin_routing_batch(&self) -> Result<CpaRoutingBatch<'_>, CpaLifecycleError>;
}

pub(crate) struct RuntimeEpochState {
    runtime: AtomicU64,
    target: AtomicU64,
}

impl RuntimeEpochState {
    fn new() -> Self {
        Self {
            runtime: AtomicU64::new(0),
            target: AtomicU64::new(0),
        }
    }

    pub(crate) fn current(&self) -> (u64, u64) {
        (
            self.runtime.load(Ordering::Acquire),
            self.target.load(Ordering::Acquire),
        )
    }

    pub(crate) fn matches(&self, runtime: u64, target: u64) -> bool {
        runtime != 0 && target != 0 && self.current() == (runtime, target)
    }

    fn advance_runtime(&self) {
        self.runtime.fetch_add(1, Ordering::AcqRel);
        self.target.fetch_add(1, Ordering::AcqRel);
    }

    fn advance_target(&self) {
        self.target.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
pub(crate) struct RuntimeInner {
    pub(crate) live: Option<LiveRuntime>,
    last_exit: Option<CpaExit>,
    crashes: VecDeque<Instant>,
    restart_count: u64,
    oauth_state: Option<String>,
    oauth_callback_submitted: bool,
}

pub(crate) struct LiveRuntime {
    lease: OwnerLease,
    auth_lease: Option<ManagedAuthLease>,
    owner_record: OwnerRecord,
    process: Option<Box<dyn CpaProcessHandle>>,
    artifact: VerifiedCpaBinary,
    pub(crate) address: SocketAddr,
    pub(crate) secrets: InstanceSecrets,
    pub(crate) accounts: Vec<AccountSnapshotRecord>,
    // Persisted account facts cannot establish that this process has finished
    // asynchronously registering its account/model inventory.
    accounts_validated_for_process: bool,
    last_health: Option<Instant>,
    published_auth_generation: Option<u64>,
    adopted: bool,
    // A failed initial start keeps ownership solely to finish cleanup. It cannot
    // become execution-ready until that child stops and a normal start commits.
    cleanup_only: bool,
}

struct InstanceLayout {
    work_dir: PathBuf,
    config_path: PathBuf,
    capability_path: PathBuf,
    accounts_path: PathBuf,
    lock_dir: PathBuf,
    auth_dir: PathBuf,
}

impl ManagedCpaRuntime {
    pub fn managed_kind(&self) -> Option<crate::CpaAccountKind> {
        if let Some(kind) = self.spec.managed_oauth {
            Some(kind)
        } else if self.spec.borrowed_claude_auth.is_some() {
            Some(crate::CpaAccountKind::Claude)
        } else if self.spec.borrowed_codex_auth.is_some() {
            Some(crate::CpaAccountKind::Codex)
        } else {
            None
        }
    }
    pub fn inspect_subscription_for_check(
        &self,
    ) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError> {
        let _scope = self.operation_context().enter();
        if let Some(spec) = &self.spec.borrowed_claude_auth {
            return spec.inspect_for_check().map(Into::into);
        }
        self.inspect_subscription()
    }
    pub fn inspect_subscription(&self) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError> {
        let _scope = self.operation_context().enter();
        if let Some(source) = self.managed_oauth_source() {
            let evidence = source.inspect()?;
            self.inspect_managed_authentication(&evidence)?;
            return Ok(BorrowedSubscriptionEvidence::Managed(evidence));
        }
        if let Some(spec) = &self.spec.borrowed_claude_auth {
            return spec.inspect().map(Into::into);
        }
        self.spec
            .borrowed_codex_auth
            .as_ref()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .inspect()
            .map(Into::into)
    }

    pub fn new(
        spec: CpaRuntimeSpec,
        catalog: Arc<TrustedReleaseCatalog>,
        locator: Arc<dyn CpaBinaryLocator>,
    ) -> Result<Self, CpaLifecycleError> {
        Self::with_components(
            spec,
            catalog,
            locator,
            Arc::new(StdCpaProcessBackend),
            Arc::new(StockCpaControlPlane),
        )
    }

    fn with_components(
        spec: CpaRuntimeSpec,
        catalog: Arc<TrustedReleaseCatalog>,
        locator: Arc<dyn CpaBinaryLocator>,
        backend: Arc<dyn CpaProcessBackend>,
        control: Arc<dyn CpaControlPlane>,
    ) -> Result<Self, CpaLifecycleError> {
        validate_spec(&spec, &catalog)?;
        Ok(Self {
            spec,
            catalog,
            locator,
            backend,
            control,
            inner: Mutex::new(RuntimeInner::default()),
            admission: admission::Admission::default(),
            observation: Mutex::new(admission::Observation::default()),
            health_invalidated: AtomicBool::new(false),
            epochs: Arc::new(RuntimeEpochState::new()),
            diagnostics: DiagnosticsPort::default(),
            proxy_environment: crate::proxy_environment::ProxyEnvironment::capture(
                std::env::vars_os(),
            ),
        })
    }

    /// Record the CPA lifecycle without CPA stdout, command paths or credential locators.
    pub fn with_diagnostics(mut self, diagnostics: DiagnosticsPort) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Override only the CPA child's proxy environment with a host-owned snapshot.
    /// This must be configured before the runtime starts.
    pub fn with_proxy_environment(
        mut self,
        values: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        self.proxy_environment = crate::proxy_environment::ProxyEnvironment::capture(values);
        if let Some(spec) = self.spec.borrowed_claude_auth.as_mut() {
            spec.set_proxy_environment(self.proxy_environment.clone());
        }
        self
    }

    fn emit(&self, event: DiagnosticEvent) {
        self.diagnostics.handle().try_emit(event);
    }

    /// One CPA start step: entered before the step blocks, then completed or failed with a
    /// stable code. No path, credential or adapter output enters the record.
    fn emit_stage(
        &self,
        stage: CpaStageKind,
        outcome: CpaStageOutcome,
        elapsed_ms: u64,
        restart_generation: u64,
    ) {
        self.emit(DiagnosticEvent::CpaStage(CpaStage {
            stage,
            elapsed_ms,
            restart_generation,
            outcome,
        }));
    }

    pub fn start(&self) -> Result<CpaHealth, CpaLifecycleError> {
        self.start_expected(None)
    }

    pub(crate) fn start_expected(
        &self,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<CpaHealth, CpaLifecycleError> {
        let _scope = self.operation_context().enter();
        self.prepare_native_subscription()?;
        let mut inner = self.lock_lifecycle()?;
        self.start_expected_locked(&mut inner, expected)
    }

    fn start_expected_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<CpaHealth, CpaLifecycleError> {
        if inner.live.as_ref().is_some_and(|live| live.cleanup_only) {
            let live = inner.live.as_mut().expect("retained failed start");
            let exit = self.shutdown_live_process(live)?;
            live.lease
                .release()
                .map_err(|_| CpaLifecycleError::OwnerState)?;
            inner.live = None;
            inner.last_exit = Some(exit);
        } else if inner.live.is_some() {
            return self.health_locked(inner);
        }
        let generation = inner.restart_count;
        let started = std::time::Instant::now();
        self.emit(DiagnosticEvent::CpaLifecycle(CpaLifecycle {
            phase: CpaPhase::Start,
            elapsed_ms: 0,
            exit_code: None,
            signal: None,
        }));
        let artifact_started = std::time::Instant::now();
        self.emit_stage(
            CpaStageKind::ArtifactLocate,
            CpaStageOutcome::Entered,
            0,
            generation,
        );
        let artifact = match self.locator.locate() {
            Ok(artifact) => {
                self.emit_stage(
                    CpaStageKind::ArtifactLocate,
                    CpaStageOutcome::Completed,
                    artifact_started.elapsed().as_millis() as u64,
                    generation,
                );
                artifact
            }
            Err(error) => {
                self.emit_stage(
                    CpaStageKind::ArtifactLocate,
                    CpaStageOutcome::Failed {
                        code: locate_failure_code(&error),
                    },
                    artifact_started.elapsed().as_millis() as u64,
                    generation,
                );
                return Err(error.into());
            }
        };
        self.emit_stage(
            CpaStageKind::ArtifactValidate,
            CpaStageOutcome::Entered,
            0,
            generation,
        );
        if let Err(error) = ensure_supported_artifact(&artifact) {
            self.emit_stage(
                CpaStageKind::ArtifactValidate,
                CpaStageOutcome::Failed {
                    code: artifact_failure_code(&error),
                },
                artifact_started.elapsed().as_millis() as u64,
                generation,
            );
            return Err(error);
        }
        self.emit_stage(
            CpaStageKind::ArtifactValidate,
            CpaStageOutcome::Completed,
            artifact_started.elapsed().as_millis() as u64,
            generation,
        );
        let layout = self.prepare_layout()?;
        let nonce = SecretText::generate()?.expose().to_owned();
        let mut fresh_record = OwnerRecord::claim(
            std::process::id(),
            nonce.clone(),
            artifact.version().to_string(),
            artifact.sha256_hex().to_owned(),
        )
        .map_err(|_| CpaLifecycleError::OwnerState)?;
        fresh_record.proxy_environment_sha256 = Some(self.proxy_environment.digest());
        let claim = OwnerLease::acquire(&layout.lock_dir, &fresh_record, nonce.clone())
            .map_err(|_| CpaLifecycleError::OwnerState)?;
        let mut live = match claim {
            OwnerClaim::Acquired(lease) => {
                let record = fresh_record.clone();
                // The step reports its own real terminal at its own boundary; nothing here
                // adds a second one for a step that already ended.
                self.start_fresh(inner, lease, record, artifact, &layout, expected)?
            }
            OwnerClaim::Existing(record) => {
                if self.backend.pid_is_running(record.owner_pid)? {
                    return Err(CpaLifecycleError::AlreadyOwned);
                }
                if record.cpa_pid != 0 && self.backend.pid_is_running(record.cpa_pid)? {
                    let transferred = record
                        .clone()
                        .transfer(std::process::id(), nonce.clone())
                        .map_err(|_| CpaLifecycleError::OwnerState)?;
                    let lease =
                        OwnerLease::reclaim_stale(&layout.lock_dir, &record, &transferred, nonce)
                            .map_err(|_| CpaLifecycleError::OwnerState)?;
                    // Authenticate the orphan before adopting or replacing its launch policy.
                    self.adopt_orphan(
                        inner,
                        lease,
                        record,
                        transferred,
                        artifact,
                        &layout,
                        expected,
                    )?
                } else {
                    let replacement = fresh_record;
                    let lease =
                        OwnerLease::reclaim_stale(&layout.lock_dir, &record, &replacement, nonce)
                            .map_err(|_| CpaLifecycleError::OwnerState)?;
                    self.start_fresh(inner, lease, replacement, artifact, &layout, expected)?
                }
            }
        };
        // Application may have replayed a durable disabled/removed decision before this runtime
        // existed. Apply that projection before publishing loaded account state to Attempt.
        subscriptions::apply_management_projection(
            &mut live.accounts,
            &self.admission.snapshot().source_management,
        );
        let health = ready_health(&live, inner.restart_count);
        let mut pending = Some(live);
        if let Err(error) = self.commit_lifecycle(|| {
            if let Some(live) = pending.as_mut() {
                live.cleanup_only = false;
            }
            inner.live = pending.take();
            self.epochs.advance_runtime();
            self.health_invalidated.store(false, Ordering::Release);
        }) {
            if let Some(live) = pending {
                self.cleanup_failed_start(inner, live);
            }
            return Err(error);
        }
        self.emit(DiagnosticEvent::CpaLifecycle(CpaLifecycle {
            phase: CpaPhase::Ready,
            elapsed_ms: started.elapsed().as_millis() as u64,
            exit_code: None,
            signal: None,
        }));
        Ok(health)
    }

    pub fn health(&self) -> Result<CpaHealth, CpaLifecycleError> {
        // A status read never joins a slow lifecycle operation. Reap a known
        // process cheaply when its owner is idle; health probes belong to owner
        // maintenance/recovery, not the observation lock.
        if let Some(mut inner) = self.inner.try_lock() {
            if let Some(live) = inner.live.as_mut()
                && let Some(process) = live.process.as_mut()
                && let Some(exit) = process.try_exit()?
            {
                live.process = None;
                live.accounts_validated_for_process = false;
                live.last_health = None;
                inner.last_exit = Some(exit);
            }
            self.publish_observation(&inner);
        }
        let (observed, sampled_at, sampled_epochs) = {
            let snapshot = self.observation.lock();
            (
                snapshot.health,
                snapshot.health_sample,
                snapshot.live.as_ref().map(|live| live.epochs),
            )
        };
        if (self.admission.snapshot().subscription_execution_suspended
            || self.health_invalidated.load(Ordering::Acquire)
            || sampled_at.is_none_or(|sample| sample.elapsed() >= Duration::from_secs(5))
            || sampled_epochs.is_none_or(|(runtime, target)| !self.epochs.matches(runtime, target)))
            && let CpaHealth::Ready {
                pid, restart_count, ..
            } = observed
        {
            return Ok(CpaHealth::Unhealthy { pid, restart_count });
        }
        Ok(observed)
    }

    pub fn shutdown(&self) -> Result<CpaExit, CpaLifecycleError> {
        let started = std::time::Instant::now();
        self.suspend_subscription_execution();
        let mut inner = self.lock_lifecycle()?;
        let live = inner.live.as_mut().ok_or(CpaLifecycleError::NotStarted)?;
        let exit = self.shutdown_live_process(live)?;
        live.lease
            .release()
            .map_err(|_| CpaLifecycleError::OwnerState)?;
        inner.live.take();
        self.epochs.advance_runtime();
        inner.last_exit = Some(exit);
        let generation = inner.restart_count;
        drop(inner);
        self.emit(DiagnosticEvent::CpaLifecycle(CpaLifecycle {
            phase: CpaPhase::Exit,
            elapsed_ms: started.elapsed().as_millis() as u64,
            exit_code: exit.code,
            signal: None,
        }));
        self.emit_stage(
            CpaStageKind::ControlCall,
            CpaStageOutcome::Completed,
            started.elapsed().as_millis() as u64,
            generation,
        );
        Ok(exit)
    }

    pub fn discover_registered_sources(
        &self,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        let materials = self.discover_materializations(None)?;
        materials
            .iter()
            .map(|material| {
                register_cpa_account(&self.catalog, material)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)
            })
            .collect()
    }

    pub fn last_exit(&self) -> Option<CpaExit> {
        self.observation.lock().last_exit
    }

    fn invalidate_live_accounts(
        &self,
        live: &mut LiveRuntime,
        kind: Option<crate::CpaAccountKind>,
    ) -> bool {
        self.commit_lifecycle(|| {
            let changed = subscriptions::deactivate_accounts(&mut live.accounts, kind);
            if changed {
                self.epochs.advance_target();
            }
            live.last_health = None;
            changed
        })
        .unwrap_or(false)
    }

    fn invalidate_runtime_accounts(
        &self,
        inner: &mut RuntimeInner,
        kind: Option<crate::CpaAccountKind>,
    ) {
        let Some(live) = inner.live.as_mut() else {
            return;
        };
        if self.invalidate_live_accounts(live, kind)
            && let Ok(layout) = self.prepare_layout()
        {
            let _ = save_account_state(&layout.accounts_path, &live.accounts);
        }
    }

    fn prepare_layout(&self) -> Result<InstanceLayout, CpaLifecycleError> {
        let root = ensure_private_dir(&self.spec.state_root)?;
        let instance_dir = ensure_private_dir(&root.join(&self.spec.instance_id))?;
        let work_dir = ensure_private_dir(&instance_dir.join("work"))?;
        let auth_dir = ensure_private_dir(&self.spec.auth_dir)?;
        Ok(InstanceLayout {
            config_path: instance_dir.join("config.yaml"),
            capability_path: instance_dir.join("capability.private"),
            accounts_path: instance_dir.join("accounts.private.json"),
            lock_dir: instance_dir.join("owner.lock"),
            work_dir,
            auth_dir,
        })
    }

    fn start_fresh(
        &self,
        inner: &mut RuntimeInner,
        lease: OwnerLease,
        record: OwnerRecord,
        artifact: VerifiedCpaBinary,
        layout: &InstanceLayout,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<LiveRuntime, CpaLifecycleError> {
        let generation = inner.restart_count;
        let attempt = (|| {
            let auth_lease = ManagedAuthLease::acquire_subscription(
                &layout.auth_dir,
                self.spec.borrowed_codex_auth.as_ref(),
                self.spec.borrowed_claude_auth.as_ref(),
                self.spec.managed_oauth,
                expected,
            )?;
            let secrets = InstanceSecrets::generate()?;
            secrets.write(&layout.capability_path)?;
            let address = reserve_loopback()?;
            let spawn_started = std::time::Instant::now();
            self.emit_stage(
                CpaStageKind::ProcessSpawn,
                CpaStageOutcome::Entered,
                0,
                generation,
            );
            let process = match self.spawn_process(&artifact, layout, address, &secrets) {
                Ok(process) => {
                    self.emit_stage(
                        CpaStageKind::ProcessSpawn,
                        CpaStageOutcome::Completed,
                        spawn_started.elapsed().as_millis() as u64,
                        generation,
                    );
                    process
                }
                Err(error) => {
                    self.emit_stage(
                        CpaStageKind::ProcessSpawn,
                        CpaStageOutcome::Failed {
                            code: spawn_failure_code(&error),
                        },
                        spawn_started.elapsed().as_millis() as u64,
                        generation,
                    );
                    return Err(error);
                }
            };
            Ok((secrets, address, process, auth_lease))
        })();
        let mut live = match attempt {
            Ok((secrets, address, process, auth_lease)) => LiveRuntime {
                lease,
                auth_lease: Some(auth_lease),
                owner_record: record,
                process: Some(process),
                artifact,
                address,
                secrets,
                accounts: Vec::new(),
                accounts_validated_for_process: false,
                last_health: None,
                published_auth_generation: None,
                adopted: false,
                cleanup_only: true,
            },
            Err(error) => {
                let _ = lease.abandon();
                return Err(error);
            }
        };
        // Once spawn succeeds, every fallible step retains both the process and
        // credential lease. A failed stop must remain retryable by this owner.
        let ready = (|| {
            live.owner_record = live
                .owner_record
                .clone()
                .running(
                    live.process.as_ref().expect("spawned process").pid(),
                    live.address,
                )
                .map_err(|_| CpaLifecycleError::OwnerState)?;
            live.lease
                .write(&live.owner_record)
                .map_err(|_| CpaLifecycleError::OwnerState)?;
            self.wait_ready(
                live.process.as_mut().expect("spawned process").as_mut(),
                &live.artifact,
                live.address,
                &live.secrets,
                generation,
            )?;
            validate_runtime_files(layout)?;
            live.accounts = load_account_state(&layout.accounts_path)?;
            live.last_health = Some(Instant::now());
            Ok(())
        })();
        if let Err(error) = ready {
            self.cleanup_failed_start(inner, live);
            return Err(error);
        }
        Ok(live)
    }

    fn cleanup_failed_start(&self, inner: &mut RuntimeInner, mut live: LiveRuntime) {
        live.cleanup_only = true;
        crate::request_context::cleanup(self.spec.shutdown_timeout, || {
            // Cancellation may have prevented the first owner-record write.
            // Record the already-created child for recovery; this grants no work.
            let _ = live.lease.write(&live.owner_record);
            if self.shutdown_live_process(&mut live).is_ok() {
                let _ = live.lease.release();
            } else {
                live.last_health = None;
                inner.live = Some(live);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn adopt_orphan(
        &self,
        inner: &mut RuntimeInner,
        lease: OwnerLease,
        stale_record: OwnerRecord,
        transferred: OwnerRecord,
        artifact: VerifiedCpaBinary,
        layout: &InstanceLayout,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<LiveRuntime, CpaLifecycleError> {
        let generation = inner.restart_count;
        let ready_started = std::time::Instant::now();
        self.emit_stage(
            CpaStageKind::ReadyWait,
            CpaStageOutcome::Entered,
            0,
            generation,
        );
        let attempt = (|| {
            if transferred.binary_version != artifact.version().to_string()
                || transferred.binary_sha256_hex != artifact.sha256_hex()
                || !transferred.address.ip().is_loopback()
            {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            let auth_lease = ManagedAuthLease::acquire_subscription(
                &layout.auth_dir,
                self.spec.borrowed_codex_auth.as_ref(),
                self.spec.borrowed_claude_auth.as_ref(),
                self.spec.managed_oauth,
                expected,
            )?;
            let secrets = InstanceSecrets::read(&layout.capability_path)?;
            validate_runtime_files(layout)?;
            self.control
                .probe_ready(
                    transferred.address,
                    &secrets,
                    &transferred.binary_version,
                    self.spec.control_timeout,
                )
                .map_err(map_control_error)?;
            let mut process = self.backend.attach_authenticated(transferred.cpa_pid)?;
            self.control
                .probe_ready(
                    transferred.address,
                    &secrets,
                    &transferred.binary_version,
                    self.spec.control_timeout,
                )
                .map_err(map_control_error)?;
            if process.pid() != transferred.cpa_pid || process.try_exit()?.is_some() {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            let accounts = load_account_state(&layout.accounts_path)?;
            Ok((transferred, secrets, process, accounts, auth_lease))
        })();
        match attempt {
            Ok((mut transferred, secrets, mut process, accounts, auth_lease)) => {
                self.emit_stage(
                    CpaStageKind::ReadyWait,
                    CpaStageOutcome::Completed,
                    ready_started.elapsed().as_millis() as u64,
                    generation,
                );
                let digest = self.proxy_environment.digest();
                if transferred.proxy_environment_sha256.as_deref() != Some(digest.as_str()) {
                    // The management capability has authenticated this exact orphan twice.
                    // Never serve through an old/unknown proxy policy after owner recovery.
                    if let Err(error) =
                        crate::request_context::cleanup(self.spec.shutdown_timeout, || {
                            process.shutdown(self.spec.shutdown_timeout)
                        })
                    {
                        let _ = lease.restore_stale(&stale_record);
                        return Err(error.into());
                    }
                    drop(auth_lease);
                    transferred.proxy_environment_sha256 = Some(digest);
                    return self.start_fresh(inner, lease, transferred, artifact, layout, expected);
                }
                Ok(LiveRuntime {
                    lease,
                    auth_lease: Some(auth_lease),
                    owner_record: transferred.clone(),
                    process: Some(process),
                    artifact,
                    address: transferred.address,
                    secrets,
                    accounts,
                    accounts_validated_for_process: false,
                    last_health: Some(Instant::now()),
                    published_auth_generation: None,
                    adopted: true,
                    cleanup_only: true,
                })
            }
            Err(error) => {
                self.emit_stage(
                    CpaStageKind::ReadyWait,
                    CpaStageOutcome::Failed {
                        code: ready_failure_code(&error),
                    },
                    ready_started.elapsed().as_millis() as u64,
                    generation,
                );
                let _ = lease.restore_stale(&stale_record);
                Err(error)
            }
        }
    }

    fn spawn_process(
        &self,
        artifact: &VerifiedCpaBinary,
        layout: &InstanceLayout,
        address: SocketAddr,
        secrets: &InstanceSecrets,
    ) -> Result<Box<dyn CpaProcessHandle>, CpaLifecycleError> {
        crate::request_context::check()?;
        let config = render_managed_config(address.port(), &layout.auth_dir, secrets)?;
        private_atomic_write(&layout.config_path, &config)?;
        crate::request_context::check()?;
        self.backend
            .spawn(&CpaLaunch {
                binary: artifact.clone(),
                config_path: layout.config_path.clone(),
                work_dir: layout.work_dir.clone(),
                management_password: secrets.management.clone(),
                proxy_environment: self.proxy_environment.clone(),
            })
            .map_err(CpaLifecycleError::from)
    }

    fn wait_ready(
        &self,
        process: &mut dyn CpaProcessHandle,
        artifact: &VerifiedCpaBinary,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        generation: u64,
    ) -> Result<(), CpaLifecycleError> {
        self.emit_stage(
            CpaStageKind::ReadyWait,
            CpaStageOutcome::Entered,
            0,
            generation,
        );
        let ready_started = Instant::now();
        let deadline = ready_started + self.spec.startup_timeout;
        let outcome = {
            let _scope = crate::CpaRequestContext::new(deadline).enter();
            (|| loop {
                crate::request_context::check()?;
                match process.try_exit() {
                    Ok(Some(exit)) => break Err(CpaLifecycleError::ExitedDuringStartup(exit)),
                    Ok(None) => {}
                    Err(error) => break Err(error.into()),
                }
                let ready = self.control.probe_ready(
                    address,
                    secrets,
                    &artifact.version().to_string(),
                    crate::request_context::remaining(self.spec.control_timeout)?,
                );
                crate::request_context::check()?;
                match ready {
                    Ok(()) => {
                        break Ok(());
                    }
                    Err(AccountDiscoveryError::RunningVersionMismatch)
                    | Err(AccountDiscoveryError::SecretBearingResponse) => {
                        break Err(CpaLifecycleError::UnsafeControlResponse);
                    }
                    Err(_) if Instant::now() < deadline => {
                        crate::request_context::sleep(Duration::from_millis(20))?
                    }
                    Err(_) => break Err(CpaLifecycleError::StartupTimeout),
                }
            })()
        };
        // The startup budget is a business timeout. Only the caller's enclosing
        // lifetime can classify it as cancellation; both paths emit a terminal.
        let outcome = match outcome {
            Err(CpaLifecycleError::OperationCancelled)
                if Instant::now() >= deadline && crate::request_context::check().is_ok() =>
            {
                Err(CpaLifecycleError::StartupTimeout)
            }
            outcome => outcome,
        };
        let elapsed_ms = ready_started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(()) => self.emit_stage(
                CpaStageKind::ReadyWait,
                CpaStageOutcome::Completed,
                elapsed_ms,
                generation,
            ),
            Err(error) => self.emit_stage(
                CpaStageKind::ReadyWait,
                CpaStageOutcome::Failed {
                    code: ready_failure_code(error),
                },
                elapsed_ms,
                generation,
            ),
        }
        outcome
    }

    fn health_locked(&self, inner: &mut RuntimeInner) -> Result<CpaHealth, CpaLifecycleError> {
        let Some(live) = inner.live.as_mut() else {
            return Ok(CpaHealth::Stopped {
                last_exit: inner.last_exit,
            });
        };
        let Some(process) = live.process.as_mut() else {
            return Ok(CpaHealth::Crashed {
                exit: inner.last_exit.unwrap_or(CpaExit { code: None }),
                restart_count: inner.restart_count,
            });
        };
        if live.cleanup_only {
            return Ok(CpaHealth::Unhealthy {
                pid: process.pid(),
                restart_count: inner.restart_count,
            });
        }
        validate_runtime_files(&self.prepare_layout()?)?;
        if let Some(exit) = process.try_exit()? {
            live.process = None;
            live.accounts_validated_for_process = false;
            inner.last_exit = Some(exit);
            return Ok(CpaHealth::Crashed {
                exit,
                restart_count: inner.restart_count,
            });
        }
        if self
            .control
            .probe_ready(
                live.address,
                &live.secrets,
                &live.artifact.version().to_string(),
                self.spec.control_timeout,
            )
            .is_err()
        {
            live.last_health = None;
            return Ok(CpaHealth::Unhealthy {
                pid: process.pid(),
                restart_count: inner.restart_count,
            });
        }
        live.last_health = Some(Instant::now());
        self.health_invalidated.store(false, Ordering::Release);
        Ok(ready_health(live, inner.restart_count))
    }

    pub(crate) fn ensure_ready_locked(
        &self,
        inner: &mut RuntimeInner,
        expected: Option<&BorrowedSubscriptionEvidence>,
    ) -> Result<(), CpaLifecycleError> {
        crate::request_context::check()?;
        if inner.live.as_ref().is_some_and(|live| live.cleanup_only) {
            if expected.is_none()
                && !self
                    .admission
                    .snapshot()
                    .passive_source_is_admitted(self.is_managed_oauth())
            {
                return Err(CpaLifecycleError::StaleSourceManagement);
            }
            self.start_expected_locked(inner, expected)?;
            return Ok(());
        }
        let live = inner.live.as_mut().ok_or(CpaLifecycleError::NotStarted)?;
        validate_runtime_files(&self.prepare_layout()?)?;
        if let Some(process) = live.process.as_mut() {
            if let Some(exit) = process.try_exit()? {
                live.process = None;
                live.accounts_validated_for_process = false;
                live.last_health = None;
                self.health_invalidated.store(true, Ordering::Release);
                inner.last_exit = Some(exit);
            } else {
                if !self.health_invalidated.load(Ordering::Acquire)
                    && live
                        .last_health
                        .is_some_and(|sample| sample.elapsed() < Duration::from_secs(5))
                {
                    return Ok(());
                }
                live.last_health = None;
                self.control
                    .probe_ready(
                        live.address,
                        &live.secrets,
                        &live.artifact.version().to_string(),
                        self.spec.control_timeout,
                    )
                    .map_err(map_control_error)?;
                crate::request_context::check()?;
                live.last_health = Some(Instant::now());
                self.health_invalidated.store(false, Ordering::Release);
                return Ok(());
            }
        }
        // Native initial discovery may recover without a saved projection. A
        // managed OAuth writer needs either saved Enabled authority or explicit
        // Check evidence; pending/passive reads never start its refresh worker.
        if expected.is_none()
            && !self
                .admission
                .snapshot()
                .passive_source_is_admitted(self.is_managed_oauth())
        {
            return Err(CpaLifecycleError::StaleSourceManagement);
        }
        let exit = inner.last_exit.unwrap_or(CpaExit { code: None });
        let now = Instant::now();
        while inner
            .crashes
            .front()
            .is_some_and(|instant| now.duration_since(*instant) > self.spec.restart_policy.window)
        {
            inner.crashes.pop_front();
        }
        inner.crashes.push_back(now);
        if inner.crashes.len() > usize::from(self.spec.restart_policy.max_restarts) {
            return Err(CpaLifecycleError::CrashLoop(exit));
        }
        let exponent = u32::try_from(inner.crashes.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let multiplier = 2_u32.checked_pow(exponent).unwrap_or(u32::MAX);
        let backoff = self
            .spec
            .restart_policy
            .base_backoff
            .saturating_mul(multiplier)
            .min(self.spec.restart_policy.max_backoff);
        if !backoff.is_zero() {
            crate::request_context::sleep(backoff)?;
        }
        let verified = self.locator.locate()?;
        ensure_supported_artifact(&verified)?;
        let generation = inner.restart_count;
        let live = inner.live.as_mut().ok_or(CpaLifecycleError::NotStarted)?;
        live.accounts_validated_for_process = false;
        if verified.version() != live.artifact.version()
            || verified.sha256_hex() != live.artifact.sha256_hex()
        {
            return Err(CpaLifecycleError::ArtifactChanged);
        }
        let layout = self.prepare_layout()?;
        live.auth_lease
            .as_mut()
            .ok_or(CpaLifecycleError::OwnerState)?
            .refresh_subscription(expected, None)?;
        let spawn_started = Instant::now();
        self.emit_stage(
            CpaStageKind::ProcessSpawn,
            CpaStageOutcome::Entered,
            0,
            generation,
        );
        let process = match self.spawn_process(&verified, &layout, live.address, &live.secrets) {
            Ok(process) => {
                self.emit_stage(
                    CpaStageKind::ProcessSpawn,
                    CpaStageOutcome::Completed,
                    spawn_started.elapsed().as_millis() as u64,
                    generation,
                );
                process
            }
            Err(error) => {
                self.emit_stage(
                    CpaStageKind::ProcessSpawn,
                    CpaStageOutcome::Failed {
                        code: spawn_failure_code(&error),
                    },
                    spawn_started.elapsed().as_millis() as u64,
                    generation,
                );
                return Err(error);
            }
        };
        let running_record = live
            .owner_record
            .clone()
            .running(process.pid(), live.address)
            .map_err(|_| CpaLifecycleError::OwnerState)?;
        // Track the child before any further fallible I/O. Cancellation may
        // revoke publication, but it must never lose the sole cleanup owner.
        live.process = Some(process);
        live.owner_record = running_record.clone();
        let ready = (|| {
            live.lease
                .write(&running_record)
                .map_err(|_| CpaLifecycleError::OwnerState)?;
            self.wait_ready(
                live.process
                    .as_mut()
                    .ok_or(CpaLifecycleError::OwnerState)?
                    .as_mut(),
                &verified,
                live.address,
                &live.secrets,
                generation,
            )?;
            validate_runtime_files(&layout)?;
            self.commit_lifecycle(|| {
                live.artifact = verified;
                live.adopted = false;
                live.last_health = Some(Instant::now());
                inner.restart_count = inner.restart_count.saturating_add(1);
                self.health_invalidated.store(false, Ordering::Release);
            })
        })();
        if let Err(error) = ready {
            live.last_health = None;
            crate::request_context::cleanup(self.spec.shutdown_timeout, || {
                if let Some(process) = live.process.as_mut()
                    && process.shutdown(self.spec.shutdown_timeout).is_ok()
                {
                    live.process = None;
                }
            });
            return Err(error);
        }
        Ok(())
    }

    fn shutdown_live_process(&self, live: &mut LiveRuntime) -> Result<CpaExit, CpaLifecycleError> {
        let Some(process) = live.process.as_mut() else {
            return Ok(CpaExit { code: None });
        };
        if let Some(exit) = process.try_exit()? {
            return Ok(exit);
        }
        if live.adopted {
            self.control
                .probe_ready(
                    live.address,
                    &live.secrets,
                    &live.artifact.version().to_string(),
                    self.spec.control_timeout,
                )
                .map_err(map_control_error)?;
        }
        process
            .shutdown(crate::request_context::remaining(
                self.spec.shutdown_timeout,
            )?)
            .map_err(Into::into)
    }
}

impl CpaRegisteredSourcePort for ManagedCpaRuntime {
    fn begin_routing_batch(&self) -> Result<CpaRoutingBatch<'_>, CpaLifecycleError> {
        CpaRoutingBatch::begin(self)
    }

    fn discover_registered_sources(&self) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        ManagedCpaRuntime::discover_registered_sources(self)
    }
}

impl Drop for ManagedCpaRuntime {
    fn drop(&mut self) {
        self.epochs.advance_runtime();
        if let Some(mut live) = self.inner.get_mut().live.take()
            && self.shutdown_live_process(&mut live).is_ok()
        {
            let _ = live.lease.release();
        }
    }
}

fn account_epoch_facts(
    accounts: &[AccountSnapshotRecord],
) -> Vec<(crate::CpaAccountKind, String, u64, bool, Vec<String>)> {
    accounts
        .iter()
        .map(|account| {
            (
                account.account_kind,
                account.account_digest.clone(),
                account.generation,
                account.active,
                account.observed_model_ids.iter().cloned().collect(),
            )
        })
        .collect()
}

impl CpaSupervisorPort for ManagedCpaRuntime {
    fn materialize_account(
        &self,
        connector_id: &str,
        endpoint_profile_id: &str,
    ) -> Result<CpaAccountMaterializationV1, CpaSupervisorError> {
        let mut matches = self
            .discover_materializations(None)
            .map_err(|_| CpaSupervisorError::Unavailable)?
            .into_iter()
            .filter(|value| {
                value.connector_id == connector_id
                    && value.endpoint_profile_id == endpoint_profile_id
            });
        let first = matches.next().ok_or(CpaSupervisorError::ActionRequired)?;
        if matches.next().is_some() {
            return Err(CpaSupervisorError::ActionRequired);
        }
        Ok(first)
    }
}

fn reserve_loopback() -> Result<SocketAddr, CpaLifecycleError> {
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(CpaLifecycleError::LoopbackBind)?;
    let address = listener
        .local_addr()
        .map_err(CpaLifecycleError::LoopbackBind)?;
    if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) || address.port() == 0 {
        return Err(CpaLifecycleError::LoopbackBind(std::io::Error::other(
            "non-loopback reservation",
        )));
    }
    drop(listener);
    Ok(address)
}

fn ensure_supported_artifact(artifact: &VerifiedCpaBinary) -> Result<(), CpaLifecycleError> {
    if artifact.version().to_string() != MANAGED_CPA_ARTIFACT_VERSION {
        return Err(CpaLifecycleError::UnsupportedArtifactVersion);
    }
    Ok(())
}

/// Why locating or validating the trusted artifact failed. A refusal by the bridge contract
/// is a rejection; anything else is reported as the artifact being unavailable.
fn artifact_failure_code(error: &CpaLifecycleError) -> CpaFailureCode {
    match error {
        CpaLifecycleError::UnsupportedArtifactVersion
        | CpaLifecycleError::InvalidBinding
        | CpaLifecycleError::ArtifactChanged
        | CpaLifecycleError::Artifact(_) => CpaFailureCode::ArtifactRejected,
        _ => CpaFailureCode::ArtifactUnavailable,
    }
}

/// Why the trusted artifact could not be located: the manifest refusing a version or digest
/// is a rejection, everything else means the verified artifact was not available.
fn locate_failure_code(error: &CpaArtifactError) -> CpaFailureCode {
    match error {
        CpaArtifactError::VersionMismatch { .. }
        | CpaArtifactError::DigestMismatch
        | CpaArtifactError::InvalidDigest => CpaFailureCode::ArtifactRejected,
        _ => CpaFailureCode::ArtifactUnavailable,
    }
}

/// Why a spawn, adopt or ready-wait step failed. Causes that are not part of this vocabulary
/// stay `Unknown` instead of being attributed to a step that did not fail.
fn spawn_failure_code(error: &CpaLifecycleError) -> CpaFailureCode {
    match error {
        CpaLifecycleError::AlreadyOwned => CpaFailureCode::AlreadyOwned,
        CpaLifecycleError::UntrustedOrphan | CpaLifecycleError::ArtifactChanged => {
            CpaFailureCode::ArtifactRejected
        }
        CpaLifecycleError::StartupTimeout => CpaFailureCode::ReadyTimeout,
        CpaLifecycleError::ExitedDuringStartup(_)
        | CpaLifecycleError::CrashLoop(_)
        | CpaLifecycleError::UnsafeControlResponse
        | CpaLifecycleError::ControlUnavailable => CpaFailureCode::ReadyRejected,
        CpaLifecycleError::Process(_)
        | CpaLifecycleError::Config(_)
        | CpaLifecycleError::LoopbackBind(_) => CpaFailureCode::SpawnFailed,
        _ => CpaFailureCode::Unknown,
    }
}

/// Why the bounded ready wait failed.
fn ready_failure_code(error: &CpaLifecycleError) -> CpaFailureCode {
    match error {
        CpaLifecycleError::StartupTimeout => CpaFailureCode::ReadyTimeout,
        CpaLifecycleError::ExitedDuringStartup(_)
        | CpaLifecycleError::CrashLoop(_)
        | CpaLifecycleError::UnsafeControlResponse
        | CpaLifecycleError::ControlUnavailable => CpaFailureCode::ReadyRejected,
        _ => CpaFailureCode::Unknown,
    }
}

fn validate_runtime_files(layout: &InstanceLayout) -> Result<(), CpaLifecycleError> {
    validate_private_file(&layout.config_path)?;
    validate_private_file(&layout.capability_path)?;
    if layout.accounts_path.exists() {
        validate_private_file(&layout.accounts_path)?;
    }
    Ok(())
}

fn ready_health(live: &LiveRuntime, restart_count: u64) -> CpaHealth {
    CpaHealth::Ready {
        pid: live.process.as_ref().map_or(0, |process| process.pid()),
        address: live.address,
        restart_count,
        adopted: live.adopted,
    }
}

fn map_control_error(error: AccountDiscoveryError) -> CpaLifecycleError {
    match error {
        AccountDiscoveryError::AuthenticationRequired => {
            CpaLifecycleError::ManagedOAuthAuthenticationRequired
        }
        AccountDiscoveryError::AccountDisappeared => {
            CpaLifecycleError::BorrowedCodexAuthUnavailable
        }
        AccountDiscoveryError::SecretBearingResponse
        | AccountDiscoveryError::NonSubscriptionAccount
        | AccountDiscoveryError::RunningVersionMismatch => CpaLifecycleError::UnsafeControlResponse,
        _ => CpaLifecycleError::ControlUnavailable,
    }
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
