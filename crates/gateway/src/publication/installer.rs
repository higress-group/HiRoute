use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hiroute_gateway_core::core::publication::adapter::AtomicPublicationSlot;
use hiroute_gateway_core::core::publication::{
    ActivePublication, InstallError, PrepareOutcome, PreparedPublication, PublicationInstaller,
};
use thiserror::Error;

use super::compiler::{CompiledAlias, CompiledGrant, compile};
use super::{
    GatewayPublicationSnapshotV3, PublicationSchemaError, constant_time_digest_eq, token_sha256,
};

const CORE_INSTALL_DEADLINE: Duration = Duration::from_secs(5);
const DURABLE_PUBLICATION_SCHEMA: &str = "hiroute.gateway.durable-publication/v1";
const CACHE_SCHEMA: &str = "hiroute.gateway.publication-cache/v1";
const CACHE_COMPILER: &str = "hiroute.gateway-cache-compiler/v1";
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationCache {
    schema_version: String,
    compiler_revision: String,
    source_publication_digest: String,
    snapshot_sha256: String,
    snapshot: GatewayPublicationSnapshotV3,
}
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DurablePublicationState {
    schema_version: String,
    snapshot: GatewayPublicationSnapshotV3,
    // Earlier writes in the current on-disk envelope contain this redundant history. Product
    // publications, not grant-derived Gateway aliases, own immutable Plan revisions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    agent_plan_revisions: Vec<DurableAgentPlanRevision>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DurableAgentPlanRevision {
    served_model_id: String,
    agent_plan_revision: u64,
    semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationFailpoint {
    None,
    BeforeDurableLkg,
    AfterRenameBeforeDirectoryDurability,
    AfterDurableLkg,
}

#[derive(Debug)]
pub struct PublishedGatewayPublication {
    snapshot: Arc<GatewayPublicationSnapshotV3>,
    pub(crate) aliases: BTreeMap<Arc<str>, CompiledAlias>,
    pub(crate) grants: Arc<[CompiledGrant]>,
    pub(crate) core: Arc<ActivePublication>,
}

impl PublishedGatewayPublication {
    /// Builds an isolated exact-run publication after its compiler envelope has been installed
    /// into a private core instance.  Unlike `GatewayPublicationInstaller::publish`, this never
    /// writes an LKG or swaps the aggregate request root.
    pub(crate) fn from_ephemeral(
        snapshot: Arc<GatewayPublicationSnapshotV3>,
        aliases: BTreeMap<Arc<str>, CompiledAlias>,
        grants: Arc<[CompiledGrant]>,
        core: Arc<ActivePublication>,
    ) -> Self {
        Self {
            snapshot,
            aliases,
            grants,
            core,
        }
    }

    pub fn workspace_id(&self) -> &str {
        &self.snapshot.workspace_id
    }

    pub fn authority_id(&self) -> &str {
        &self.snapshot.authority_id
    }

    pub fn authority_epoch(&self) -> u64 {
        self.snapshot.authority_epoch
    }

    pub fn publication_revision(&self) -> u64 {
        self.snapshot.publication_revision
    }

    pub fn payload_digest(&self) -> &str {
        &self.snapshot.payload_digest
    }

    pub fn catalog_renderer_revision(&self) -> &str {
        &self.snapshot.catalog_renderer_revision
    }

    /// Reattaches observation-only price identities derived by the daemon
    /// from one verified Product publication. Execution core/config Arcs stay
    /// unchanged; aggregate identity must match exactly.
    pub fn with_verified_request_pricing(
        self: &Arc<Self>,
        source: &GatewayPublicationSnapshotV3,
    ) -> Result<Arc<Self>, PublicationInstallError> {
        source.validate()?;
        if source.workspace_id != self.snapshot.workspace_id
            || source.authority_id != self.snapshot.authority_id
            || source.authority_epoch != self.snapshot.authority_epoch
            || source.publication_revision != self.snapshot.publication_revision
            || source.payload_digest != self.snapshot.payload_digest
        {
            return Err(PublicationInstallError::InvalidDurableState);
        }
        let mut aliases = self.aliases.clone();
        for (alias_id, compiled) in &mut aliases {
            let source_alias = source
                .aliases
                .iter()
                .find(|alias| alias.served_model_id == alias_id.as_ref())
                .ok_or(PublicationInstallError::InvalidDurableState)?;
            compiled.execution.pricing_bindings =
                super::compiler::compile_pricing_bindings(source_alias)?;
        }
        Ok(Arc::new(Self {
            snapshot: Arc::clone(&self.snapshot),
            aliases,
            grants: Arc::clone(&self.grants),
            core: Arc::clone(&self.core),
        }))
    }

    pub fn agent_plan_revision(&self, alias: &str) -> Option<u64> {
        self.aliases.get(alias).map(|plan| plan.agent_plan_revision)
    }

    pub(crate) fn authenticate_bearer(&self, header: &str) -> Option<CompiledGrant> {
        let token = header.strip_prefix("Bearer ")?;
        if token.is_empty() || token.len() > 4_096 {
            return None;
        }
        let verifier = token_sha256(token);
        let mut matched = None;
        for grant in self.grants.iter() {
            if constant_time_digest_eq(&verifier, &grant.bearer_token_sha256) {
                matched = Some(grant.clone());
            }
        }
        matched
    }
}

#[derive(Debug)]
pub enum GatewayPrepareOutcome {
    Prepared(PreparedGatewayPublication),
    Duplicate(Arc<PublishedGatewayPublication>),
}

#[derive(Debug)]
pub struct PreparedGatewayPublication {
    snapshot: Option<Arc<GatewayPublicationSnapshotV3>>,
    aliases: Option<BTreeMap<Arc<str>, CompiledAlias>>,
    grants: Option<Arc<[CompiledGrant]>>,
    core: Option<PreparedPublication>,
    core_installer: Arc<PublicationInstaller>,
    pending: Arc<AtomicBool>,
}

impl Drop for PreparedGatewayPublication {
    fn drop(&mut self) {
        if let Some(core) = self.core.take() {
            let _ = self.core_installer.abandon_prepared(core);
        }
        self.pending.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub struct GatewayPublicationInstaller {
    lkg_path: PathBuf,
    core: Arc<PublicationInstaller>,
    active: AtomicPublicationSlot<PublishedGatewayPublication>,
    pending: Arc<AtomicBool>,
    durability_uncertain: AtomicBool,
    publish_gate: Mutex<()>,
}

impl GatewayPublicationInstaller {
    /// Product startup resolves its authoritative publication and installation decision from
    /// control.db. A cache, including a stale or corrupt cache, never grants initial admission.
    pub fn open_for_product_authority(
        path: impl AsRef<Path>,
    ) -> Result<Self, PublicationInstallError> {
        let path = path.as_ref();
        if path.file_name().is_none() {
            return Err(PublicationInstallError::InvalidLkgPath);
        }
        Ok(Self::empty(path.to_path_buf()))
    }

    fn empty(path: PathBuf) -> Self {
        Self {
            lkg_path: path,
            core: Arc::new(PublicationInstaller::new()),
            active: AtomicPublicationSlot::empty(),
            pending: Arc::new(AtomicBool::new(false)),
            durability_uncertain: AtomicBool::new(false),
            publish_gate: Mutex::new(()),
        }
    }
    /// Opens a durable aggregate feed. A missing LKG is a valid unavailable
    /// starting state; an existing but invalid LKG fails closed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PublicationInstallError> {
        let path = path.as_ref().to_path_buf();
        let restored = if path.exists() {
            let bytes = std::fs::read(&path).map_err(PublicationInstallError::Io)?;
            Some(decode_durable_state(&bytes)?)
        } else {
            None
        };
        let installer = Self::empty(path);
        if let Some(snapshot) = restored {
            let prepared = match installer.prepare(snapshot)? {
                GatewayPrepareOutcome::Prepared(prepared) => prepared,
                GatewayPrepareOutcome::Duplicate(_) => unreachable!("fresh core cannot duplicate"),
            };
            installer.publish_restored(prepared)?;
        }
        Ok(installer)
    }

    pub fn active(&self) -> Option<Arc<PublishedGatewayPublication>> {
        self.active.load()
    }

    pub fn durability_uncertain(&self) -> bool {
        self.durability_uncertain.load(Ordering::Acquire)
    }

    fn enter_durability_uncertain(&self) {
        self.durability_uncertain.store(true, Ordering::Release);
        // Existing request-owned Arcs remain valid, but no new request may pin
        // the old root after disk rename and live state have diverged.
        self.active.clear();
    }

    pub fn prepare(
        &self,
        snapshot: GatewayPublicationSnapshotV3,
    ) -> Result<GatewayPrepareOutcome, PublicationInstallError> {
        if self.durability_uncertain.load(Ordering::Acquire) {
            return Err(PublicationInstallError::InstallerRequiresRestart);
        }
        if self
            .pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PublicationInstallError::PrepareBusy);
        }
        let result = self.prepare_owned(snapshot);
        if result.is_err() || matches!(result, Ok(GatewayPrepareOutcome::Duplicate(_))) {
            self.pending.store(false, Ordering::Release);
        }
        result
    }

    fn prepare_owned(
        &self,
        snapshot: GatewayPublicationSnapshotV3,
    ) -> Result<GatewayPrepareOutcome, PublicationInstallError> {
        snapshot.validate()?;
        let snapshot = Arc::new(snapshot);
        let compiled = compile(&snapshot)?;
        let outcome = self
            .core
            .prepare_uncancelled(compiled.envelope, Instant::now() + CORE_INSTALL_DEADLINE)?;
        match outcome {
            PrepareOutcome::Duplicate(_) => self
                .active()
                .map(GatewayPrepareOutcome::Duplicate)
                .ok_or(PublicationInstallError::CoreDuplicateWithoutAggregate),
            PrepareOutcome::Prepared(core) => Ok(GatewayPrepareOutcome::Prepared(
                PreparedGatewayPublication {
                    snapshot: Some(snapshot),
                    aliases: Some(compiled.aliases),
                    grants: Some(compiled.grants),
                    core: Some(core),
                    core_installer: Arc::clone(&self.core),
                    pending: Arc::clone(&self.pending),
                },
            )),
        }
    }

    pub fn publish(
        &self,
        prepared: PreparedGatewayPublication,
    ) -> Result<Arc<PublishedGatewayPublication>, PublicationInstallError> {
        self.publish_with_failpoint(prepared, PublicationFailpoint::None)
    }

    pub fn publish_with_failpoint(
        &self,
        mut prepared: PreparedGatewayPublication,
        failpoint: PublicationFailpoint,
    ) -> Result<Arc<PublishedGatewayPublication>, PublicationInstallError> {
        if !Arc::ptr_eq(&self.core, &prepared.core_installer) {
            return Err(PublicationInstallError::ForeignPreparedPublication);
        }
        if self.durability_uncertain.load(Ordering::Acquire) {
            return Err(PublicationInstallError::InstallerRequiresRestart);
        }
        let _gate = self
            .publish_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoint == PublicationFailpoint::BeforeDurableLkg {
            return Err(PublicationInstallError::InjectedBeforeDurableLkg);
        }
        let snapshot = prepared
            .snapshot
            .as_ref()
            .ok_or(PublicationInstallError::ConsumedPrepared)?;
        self.persist(snapshot, failpoint)?;
        if failpoint == PublicationFailpoint::AfterDurableLkg {
            self.enter_durability_uncertain();
            return Err(PublicationInstallError::CrashAfterDurableLkg);
        }
        match self.finish_publish(&mut prepared) {
            Ok(publication) => Ok(publication),
            Err(error) => {
                self.enter_durability_uncertain();
                Err(PublicationInstallError::LiveInstallAfterDurable(
                    error.to_string(),
                ))
            }
        }
    }

    fn publish_restored(
        &self,
        mut prepared: PreparedGatewayPublication,
    ) -> Result<Arc<PublishedGatewayPublication>, PublicationInstallError> {
        let _gate = self
            .publish_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.finish_publish(&mut prepared)
    }

    fn finish_publish(
        &self,
        prepared: &mut PreparedGatewayPublication,
    ) -> Result<Arc<PublishedGatewayPublication>, PublicationInstallError> {
        let core = self.core.publish_uncancelled(
            prepared
                .core
                .take()
                .ok_or(PublicationInstallError::ConsumedPrepared)?,
            Instant::now() + CORE_INSTALL_DEADLINE,
        )?;
        let published = Arc::new(PublishedGatewayPublication {
            snapshot: prepared
                .snapshot
                .take()
                .ok_or(PublicationInstallError::ConsumedPrepared)?,
            aliases: prepared
                .aliases
                .take()
                .ok_or(PublicationInstallError::ConsumedPrepared)?,
            grants: prepared
                .grants
                .take()
                .ok_or(PublicationInstallError::ConsumedPrepared)?,
            core,
        });
        // This is the sole externally visible linearization point. Requests
        // never consult the core installer independently.
        self.active.store(Arc::clone(&published));
        Ok(published)
    }

    fn persist(
        &self,
        snapshot: &GatewayPublicationSnapshotV3,
        failpoint: PublicationFailpoint,
    ) -> Result<(), PublicationInstallError> {
        let parent = self
            .lkg_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(PublicationInstallError::Io)?;
        let file_name = self
            .lkg_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(PublicationInstallError::InvalidLkgPath)?;
        let temporary = parent.join(format!(".{file_name}.prepare"));
        // Price identities are reconstructed from the verified Product record
        // after restart; the Gateway LKG stores only execution semantics.
        let mut durable_snapshot = snapshot.clone();
        durable_snapshot.clear_pricing_identities();
        let durable = encode_durable_state(&durable_snapshot)?;
        let mut bytes = serde_json::to_vec(&durable).map_err(PublicationInstallError::Json)?;
        bytes.push(b'\n');
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(PublicationInstallError::Io)?;
        file.write_all(&bytes)
            .map_err(PublicationInstallError::Io)?;
        file.sync_all().map_err(PublicationInstallError::Io)?;
        std::fs::rename(&temporary, &self.lkg_path).map_err(PublicationInstallError::Io)?;
        if failpoint == PublicationFailpoint::AfterRenameBeforeDirectoryDurability {
            self.enter_durability_uncertain();
            return Err(PublicationInstallError::InjectedAfterRenameBeforeDirectoryDurability);
        }
        if let Err(source) = File::open(parent).and_then(|directory| directory.sync_all()) {
            self.enter_durability_uncertain();
            return Err(PublicationInstallError::DirectoryDurabilityUncertain(
                source,
            ));
        }
        Ok(())
    }
}

fn encode_durable_state(
    snapshot: &GatewayPublicationSnapshotV3,
) -> Result<PublicationCache, PublicationInstallError> {
    let bytes = serde_json::to_string(snapshot).map_err(PublicationInstallError::Json)?;
    Ok(PublicationCache {
        schema_version: CACHE_SCHEMA.into(),
        compiler_revision: CACHE_COMPILER.into(),
        source_publication_digest: snapshot.payload_digest.clone(),
        snapshot_sha256: token_sha256(&bytes),
        snapshot: snapshot.clone(),
    })
}

fn decode_durable_state(
    bytes: &[u8],
) -> Result<GatewayPublicationSnapshotV3, PublicationInstallError> {
    if let Ok(cache) = serde_json::from_slice::<PublicationCache>(bytes) {
        let snapshot =
            serde_json::to_string(&cache.snapshot).map_err(PublicationInstallError::Json)?;
        if cache.schema_version != CACHE_SCHEMA
            || cache.compiler_revision != CACHE_COMPILER
            || cache.source_publication_digest != cache.snapshot.payload_digest
            || cache.snapshot_sha256 != token_sha256(&snapshot)
        {
            return Err(PublicationInstallError::InvalidDurableState);
        }
        cache.snapshot.validate()?;
        return Ok(cache.snapshot);
    }
    let durable: DurablePublicationState =
        serde_json::from_slice(bytes).map_err(PublicationInstallError::Json)?;
    if durable.schema_version != DURABLE_PUBLICATION_SCHEMA {
        return Err(PublicationInstallError::InvalidDurableState);
    }
    durable.snapshot.validate()?;
    Ok(durable.snapshot)
}

#[derive(Debug, Error)]
pub enum PublicationInstallError {
    #[error(transparent)]
    Schema(#[from] PublicationSchemaError),
    #[error(transparent)]
    Plan(#[from] hiroute_gateway_core::core::execution_plan::PlanError),
    #[error(transparent)]
    Core(#[from] InstallError),
    #[error("gateway publication I/O failed: {0}")]
    Io(std::io::Error),
    #[error("gateway publication JSON failed: {0}")]
    Json(serde_json::Error),
    #[error("another publication is already prepared")]
    PrepareBusy,
    #[error("core reported duplicate without an aggregate publication")]
    CoreDuplicateWithoutAggregate,
    #[error("prepared publication was already consumed")]
    ConsumedPrepared,
    #[error("prepared publication belongs to a different installer")]
    ForeignPreparedPublication,
    #[error("gateway publication LKG path is invalid")]
    InvalidLkgPath,
    #[error("gateway durable publication state is invalid")]
    InvalidDurableState,
    #[error("gateway publication endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("gateway publication contains an invalid frozen routing policy")]
    InvalidPlannerPolicy,
    #[error("publication failpoint fired before durable LKG")]
    InjectedBeforeDurableLkg,
    #[error("publication failpoint fired after rename but before directory durability")]
    InjectedAfterRenameBeforeDirectoryDurability,
    #[error("publication rename completed but directory durability is uncertain: {0}")]
    DirectoryDurabilityUncertain(std::io::Error),
    #[error("simulated crash after durable LKG and before live swap")]
    CrashAfterDurableLkg,
    #[error("durable LKG completed but live install did not: {0}")]
    LiveInstallAfterDurable(String),
    #[error("publication installer must restart after a durability-uncertain boundary")]
    InstallerRequiresRestart,
}

impl PublicationInstallError {
    pub fn is_crash_boundary(&self) -> bool {
        matches!(
            self,
            Self::InjectedAfterRenameBeforeDirectoryDurability
                | Self::DirectoryDurabilityUncertain(_)
                | Self::CrashAfterDurableLkg
                | Self::LiveInstallAfterDurable(_)
        )
    }
}
