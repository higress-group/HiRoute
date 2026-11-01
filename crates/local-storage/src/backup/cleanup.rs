//! Upgrade recovery material expires only after the complete service is ready.
//! Retire namespaces atomically, then reuse the product maintenance lifecycle.
use super::*;
use hiroute_diagnostics::files::{PrivateDir, VerifiedFile};
use hiroute_host_runtime::RetainedRecoveryPackage;
use hiroute_platform::private_fs::{DirectoryRemoval, directory_identity};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BATCH: usize = 128;
const WORK: Duration = Duration::from_millis(10);
const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Source {
    Backups,
    Package { name: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    source: Source,
    payload_identity: [u64; 2],
    job_identity: [u64; 2],
}

struct Job {
    path: PathBuf,
    record: Record,
    removal: Option<DirectoryRemoval>,
}

struct Package {
    record: RetainedRecoveryPackage,
    handoff: VerifiedFile,
    image: fs::File,
    digest: Sha256,
}

struct State {
    initialized: bool,
    discovery: Option<fs::ReadDir>,
    pending: Option<PathBuf>,
    backups_submitted: bool,
    package: Option<Package>,
    active: Option<Job>,
    due: Instant,
    done: bool,
    #[cfg(test)]
    image: Option<fs::File>,
}

pub struct UpgradeCleanup {
    storage: PathBuf,
    ready: AtomicBool,
    state: Mutex<State>,
}

impl UpgradeCleanup {
    pub fn new(storage: &Path) -> Self {
        Self {
            storage: storage.to_owned(),
            ready: AtomicBool::new(false),
            state: Mutex::new(State {
                initialized: false,
                discovery: None,
                pending: None,
                backups_submitted: false,
                package: None,
                active: None,
                due: Instant::now(),
                done: false,
                #[cfg(test)]
                image: None,
            }),
        }
    }

    /// The caller has restored publication and bound both Gateway and Control.
    /// SQL migration completion alone must never call this method.
    pub fn service_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub fn cycle(&self) -> Result<bool, LocalStorageError> {
        self.cycle_at(Instant::now())
    }

    fn cycle_at(&self, now: Instant) -> Result<bool, LocalStorageError> {
        if !self.ready.load(Ordering::Acquire) {
            return Ok(false);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| LocalStorageError::InvalidData)?;
        if state.done || now < state.due {
            return Ok(state.done);
        }
        let started = Instant::now();
        let result = self.work(&mut state);
        // Filesystem operations can overrun a cooperative budget. Include that
        // cost in the next wait; errors never create an eager catch-up loop.
        let wait = started
            .elapsed()
            .saturating_mul(100)
            .max(if result.is_err() {
                Duration::from_secs(5)
            } else {
                INTERVAL
            });
        state.due = now + wait;
        result.map(|()| state.done)
    }

    fn parent(&self) -> Result<&Path, LocalStorageError> {
        self.storage.parent().ok_or(LocalStorageError::InvalidData)
    }

    fn jobs(&self) -> Result<PathBuf, LocalStorageError> {
        let mut name = self
            .storage
            .file_name()
            .ok_or(LocalStorageError::InvalidData)?
            .to_os_string();
        name.push(".upgrade-cleanup");
        Ok(self.parent()?.join(name))
    }

    fn source(&self, source: &Source) -> Result<PathBuf, LocalStorageError> {
        match source {
            Source::Backups => Ok(crate::upgrade_backup_root(&self.storage)?
                .parent()
                .ok_or(LocalStorageError::InvalidData)?
                .to_owned()),
            Source::Package { name } if package_name(name) => {
                Ok(self.parent()?.join("upgrade-packages").join(name))
            }
            _ => Err(LocalStorageError::InvalidData),
        }
    }

    fn work(&self, state: &mut State) -> Result<(), LocalStorageError> {
        if !state.initialized {
            state.discovery = match fs::read_dir(self.jobs()?) {
                Ok(entries) => Some(entries),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            // Snapshot this handoff once. A later installer may prepare the
            // next generation while the old process is still serving requests.
            state.initialized = true;
            state.package = self.package(state)?;
        }
        if let Some(job) = state.active.as_mut() {
            match self.advance(job) {
                Ok(true) => state.active = None,
                Ok(false) => {}
                Err(error) => {
                    job.removal = None;
                    return Err(error);
                }
            }
            return Ok(());
        }
        if let Some(path) = state.pending.as_ref() {
            // Do not lose a failed admission by advancing the directory cursor.
            // Keep this exact item pending under the same error backoff.
            state.active = self.load(path.clone())?;
            state.pending = None;
            return Ok(());
        }
        if let Some(entries) = state.discovery.as_mut() {
            for _ in 0..16 {
                let Some(entry) = entries.next().transpose()? else {
                    state.discovery = None;
                    return Ok(());
                };
                let name = entry.file_name();
                if name
                    .to_str()
                    .is_some_and(|n| n.starts_with("retire-") && portable(n))
                {
                    state.pending = Some(entry.path());
                    return Ok(());
                }
            }
            return Ok(());
        }
        if !state.backups_submitted {
            state.active = self.submit(Source::Backups)?;
            if state.active.is_none() {
                self.clear_observation_link()?;
            }
            state.backups_submitted = true;
            return Ok(());
        }
        if let Some(package) = state.package.as_mut() {
            let started = Instant::now();
            let mut buffer = [0_u8; 64 * 1024];
            for _ in 0..128 {
                let count = package.image.read(&mut buffer)?;
                if count == 0 {
                    let digest = format!("{:x}", package.digest.clone().finalize());
                    if digest == package.record.target_daemon_sha256 {
                        let directory =
                            PrivateDir::open_existing(self.parent()?).map_err(safety)?;
                        let unchanged = directory
                            .open_read(hiroute_host_runtime::RETAINED_PACKAGE_FILE)
                            .map_err(safety)?
                            .is_some_and(|file| file.identity() == package.handoff.identity());
                        if !unchanged {
                            state.package = None;
                            return Ok(());
                        }
                        let root = self.parent()?.join("upgrade-packages");
                        if package.record.package_root.parent() == Some(root.as_path())
                            && let Some(name) = package
                                .record
                                .package_root
                                .file_name()
                                .and_then(|n| n.to_str())
                                .filter(|n| package_name(n))
                        {
                            // Publish the resumable job before dropping the old
                            // handoff. Never delete caller-supplied package paths.
                            state.active = self.submit(Source::Package {
                                name: name.to_owned(),
                            })?;
                        }
                        directory
                            .remove_verified(
                                hiroute_host_runtime::RETAINED_PACKAGE_FILE,
                                package.handoff.identity(),
                            )
                            .map_err(safety)?;
                        hiroute_platform::durable::directory_barrier(self.parent()?)?;
                    }
                    state.package = None;
                    return Ok(());
                }
                package.digest.update(&buffer[..count]);
                if started.elapsed() >= WORK {
                    break;
                }
            }
            return Ok(());
        }
        state.done = true;
        Ok(())
    }

    fn submit(&self, source: Source) -> Result<Option<Job>, LocalStorageError> {
        let original = self.source(&source)?;
        match fs::symlink_metadata(&original) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let _original = PrivateDir::open_existing(&original).map_err(safety)?;
        let identity = directory_identity(&original)?;
        let jobs = self.jobs()?;
        crate::prepare_owner_directory(&jobs)?;
        let path = jobs.join(format!("retire-{}", random_attempt_id()?));
        hiroute_platform::private_fs::create_directory(&path)?;
        let record = Record {
            schema: 1,
            source,
            payload_identity: identity,
            job_identity: directory_identity(&path)?,
        };
        hiroute_platform::durable::write_private_new(
            &path.join("task.json"),
            &serde_json::to_vec(&record).map_err(|_| LocalStorageError::InvalidData)?,
        )?;
        hiroute_platform::durable::directory_barrier(&path)?;
        hiroute_platform::durable::directory_barrier(&jobs)?;
        Ok(Some(Job {
            path,
            record,
            removal: None,
        }))
    }

    fn load(&self, path: PathBuf) -> Result<Option<Job>, LocalStorageError> {
        let directory = PrivateDir::open_existing(&path).map_err(safety)?;
        let Some(mut file) = directory.open_read("task.json").map_err(safety)? else {
            // Crash before task publication or after its final removal. Only an
            // empty private wrapper can be removed without a persisted claim.
            drop(directory);
            fs::remove_dir(&path)?;
            return Ok(None);
        };
        if file.len() > 2048 {
            return Err(LocalStorageError::InvalidData);
        }
        let record: Record = serde_json::from_slice(&file.read_prefix(2048).map_err(safety)?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        file.recheck().map_err(safety)?;
        if record.schema != 1 || directory_identity(&path)? != record.job_identity {
            return Err(LocalStorageError::InvalidData);
        }
        self.source(&record.source)?;
        Ok(Some(Job {
            path,
            record,
            removal: None,
        }))
    }

    fn advance(&self, job: &mut Job) -> Result<bool, LocalStorageError> {
        let _job = PrivateDir::open_existing(&job.path).map_err(safety)?;
        if directory_identity(&job.path)? != job.record.job_identity {
            return Err(LocalStorageError::Permission);
        }
        let payload = job.path.join("data");
        if !present(&payload)? {
            let original = self.source(&job.record.source)?;
            if present(&original)? && directory_identity(&original)? == job.record.payload_identity
            {
                PrivateDir::open_existing(&original).map_err(safety)?;
                if matches!(job.record.source, Source::Backups) {
                    self.clear_observation_link()?;
                }
                hiroute_platform::durable::rename(&original, &payload)?;
                hiroute_platform::durable::directory_barrier(
                    original.parent().ok_or(LocalStorageError::InvalidData)?,
                )?;
                hiroute_platform::durable::directory_barrier(&job.path)?;
            } else {
                return self.finish(job);
            }
        }
        if job.removal.is_none() {
            job.removal = Some(DirectoryRemoval::open(
                &payload,
                job.record.payload_identity,
            )?);
        }
        if job.removal.as_mut().unwrap().step(BATCH, WORK)?.done {
            job.removal = None;
            return self.finish(job);
        }
        Ok(false)
    }

    fn finish(&self, job: &Job) -> Result<bool, LocalStorageError> {
        // Only our task record may remain. Preserve unexpected additions.
        for entry in fs::read_dir(&job.path)?.take(2) {
            if entry?.file_name() != "task.json" {
                return Err(LocalStorageError::InvalidData);
            }
        }
        let directory = PrivateDir::open_existing(&job.path).map_err(safety)?;
        if let Some(file) = directory.open_read("task.json").map_err(safety)? {
            directory
                .remove_verified("task.json", file.identity())
                .map_err(safety)?;
        }
        drop(directory);
        fs::remove_dir(&job.path)?;
        hiroute_platform::durable::directory_barrier(
            job.path.parent().ok_or(LocalStorageError::InvalidData)?,
        )?;
        Ok(true)
    }

    fn clear_observation_link(&self) -> Result<(), LocalStorageError> {
        let directory = PrivateDir::open_existing(&self.storage).map_err(safety)?;
        if let Some(file) = directory
            .open_read("observation-source-backup.json")
            .map_err(safety)?
        {
            directory
                .remove_verified("observation-source-backup.json", file.identity())
                .map_err(safety)?;
            hiroute_platform::durable::directory_barrier(&self.storage)?;
        }
        Ok(())
    }

    fn package(&self, _state: &mut State) -> Result<Option<Package>, LocalStorageError> {
        let directory = PrivateDir::open_existing(self.parent()?).map_err(safety)?;
        let Some(mut file) = directory
            .open_read(hiroute_host_runtime::RETAINED_PACKAGE_FILE)
            .map_err(safety)?
        else {
            return Ok(None);
        };
        if file.len() > 1024 * 1024 {
            return Err(LocalStorageError::InvalidData);
        }
        let record: RetainedRecoveryPackage =
            serde_json::from_slice(&file.read_prefix(1024 * 1024).map_err(safety)?)
                .map_err(|_| LocalStorageError::InvalidData)?;
        file.recheck().map_err(safety)?;
        if record.schema != "hiroute.retained-recovery-package/v1"
            || record.storage_root != self.storage
            || record.target_daemon_sha256.len() != 64
            || !record
                .target_daemon_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(LocalStorageError::InvalidData);
        }
        #[cfg(test)]
        if let Some(image) = _state.image.take() {
            return Ok(Some(Package {
                record,
                handoff: file,
                image,
                digest: Sha256::new(),
            }));
        }
        #[cfg(target_os = "linux")]
        let image = fs::File::open("/proc/self/exe")?;
        #[cfg(not(target_os = "linux"))]
        let image = hiroute_platform::private_fs::open_regular_file(&std::env::current_exe()?)?;
        Ok(Some(Package {
            record,
            handoff: file,
            image,
            digest: Sha256::new(),
        }))
    }
}

fn safety(_: hiroute_diagnostics::files::FileSafetyError) -> LocalStorageError {
    LocalStorageError::Permission
}
fn present(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
fn portable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn package_name(name: &str) -> bool {
    name.starts_with("old-package-") && portable(name)
}

#[cfg(test)]
mod tests;
