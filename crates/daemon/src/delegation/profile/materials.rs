//! Explicit run materials and a task-owned native storage reference. No launcher cleanup here.
use super::*;
use hiroute_domain::delegation::{
    DELEGATION_NATIVE_ROOT_MARKER_FILE_V1, DelegationNativeFilesystemIdentityV1,
    DelegationNativeRootMarkerV1, DelegationNativeRootV1, valid_native_history_path,
};
use hiroute_domain::{CanonicalDigest, WorkspaceId};
use std::fs;
use std::path::Component;

/// Paths relative to a NEW private_root. The launcher creates/protects only this collection
/// and its root. It must reject existing roots, traversal, links and unknown ownership.
/// Do not derive additional directories or cleanup targets by inspecting env/session_meta.
pub struct RunMaterials {
    pub directories: Vec<PathBuf>,
    pub files: Vec<RunMaterialFile>,
}

/// Already rendered by 20; no template substitutions, executable permissions or secret logs.
pub struct RunMaterialFile {
    pub relative_path: PathBuf,
    pub contents: Zeroizing<Vec<u8>>,
}

/// Prepared by 20, never created or removed by 18. No run token is stored by this helper.
/// Re-derive using the persisted task/workspace/Harness identity, never Plan or run identity.
pub struct TaskSessionRoot {
    path: PathBuf,
    harness: WorkerHarnessV1,
    workspace_root_identity: String,
}

pub enum SessionRootUse<'a> {
    New,
    /// The existing native-cache/body inventory supplies exact required relative files.
    /// This existence check does not replace current content authorization/retention checks.
    Continue {
        required_history: &'a [PathBuf],
    },
}

impl TaskSessionRoot {
    pub fn prepare(
        session_base: &Path,
        workspace: &WorkspaceId,
        workspace_root_identity: &str,
        task_id: &str,
        harness: WorkerHarnessV1,
        usage: SessionRootUse<'_>,
    ) -> Result<Self, DelegationErrorV1> {
        use hiroute_domain::delegation::valid_delegation_id;
        if WorkspaceId::parse(workspace.as_str()).is_err()
            || !valid_delegation_id(task_id)
            || !valid_delegation_id(workspace_root_identity)
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let base = checked_directory(session_base)?;
        let digest = CanonicalDigest::of(&json!([
            "delegation-native-root-v1",
            workspace,
            workspace_root_identity,
            task_id,
            harness
        ]))
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        let path = base.join(digest.as_str().trim_start_matches("sha256:"));
        match usage {
            SessionRootUse::New => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                // No create_dir_all/reuse/chmod/remove of an unknown existing task directory.
                builder
                    .create(&path)
                    .map_err(|_| DelegationErrorV1::Conflict)?;
            }
            SessionRootUse::Continue { required_history } => {
                checked_directory(&path).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
                if required_history.is_empty() || required_history.len() > 256 {
                    return Err(DelegationErrorV1::ResumeUnavailable);
                }
                for relative in required_history {
                    check_history(&path, relative)?;
                }
            }
        }
        let actual = checked_directory(&path)?;
        if actual.parent() != Some(base.as_path()) {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        Ok(Self {
            path: actual,
            harness,
            workspace_root_identity: workspace_root_identity.to_owned(),
        })
    }

    pub(super) fn check_binding(
        &self,
        harness: WorkerHarnessV1,
        root: &str,
    ) -> Result<(), DelegationErrorV1> {
        if self.harness != harness || self.workspace_root_identity != root {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Publish a root marker before the runtime store records this generation as ready.  The
    /// marker is evidence for the already-derived task root only; it never authorizes adoption of
    /// an existing directory.
    pub(crate) fn create_ownership_marker(
        &self,
        session_base: &Path,
        root: &DelegationNativeRootV1,
    ) -> Result<DelegationNativeFilesystemIdentityV1, DelegationErrorV1> {
        self.check_root_record(session_base, root, false)?;
        let marker = serde_json::to_vec(&root.ownership_marker())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let path = self.path.join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        use std::io::Write;
        file.write_all(&marker)
            .and_then(|()| file.sync_all())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let marker_metadata = file
            .metadata()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        fs::File::open(&self.path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        filesystem_identity(session_base, &self.path, &marker_metadata)
    }

    pub(crate) fn verify_ownership(
        &self,
        session_base: &Path,
        root: &DelegationNativeRootV1,
    ) -> Result<DelegationNativeFilesystemIdentityV1, DelegationErrorV1> {
        self.check_root_record(session_base, root, true)?;
        let marker_path = self.path.join(DELEGATION_NATIVE_ROOT_MARKER_FILE_V1);
        let path_metadata =
            fs::symlink_metadata(&marker_path).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if linked(&path_metadata) || !path_metadata.is_file() || path_metadata.len() > 4096 {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let file = open_marker(&marker_path)?;
        let metadata = file
            .metadata()
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if linked(&metadata) || !metadata.is_file() || metadata.len() > 4096 {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
        }
        use std::io::Read;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if bytes.len() > 4096 {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let actual: DelegationNativeRootMarkerV1 =
            serde_json::from_slice(&bytes).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if actual != root.ownership_marker() {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        filesystem_identity(session_base, &self.path, &metadata)
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)
    }

    fn check_root_record(
        &self,
        session_base: &Path,
        root: &DelegationNativeRootV1,
        continuing: bool,
    ) -> Result<(), DelegationErrorV1> {
        root.validate()?;
        self.check_binding(root.harness, &root.workspace_root_identity)?;
        let base = checked_directory(session_base)?;
        if self.path.parent() != Some(base.as_path())
            || self.path.file_name().and_then(|name| name.to_str())
                != Some(root.relative_root.as_str())
            || (continuing
                && root.managed_base_path.as_deref() != Some(base.to_string_lossy().as_ref()))
        {
            return Err(if continuing {
                DelegationErrorV1::ResumeUnavailable
            } else {
                DelegationErrorV1::Conflict
            });
        }
        Ok(())
    }

    /// The adapter checks account/read before applying its per-session CODEX_CONFIG.
    /// Bootstrap the same private provider at app-server startup, without persisting a token.
    pub(super) fn prepare_codex_provider(
        &self,
        gateway: std::net::SocketAddr,
        catalog_path: Option<&Path>,
    ) -> Result<(), DelegationErrorV1> {
        use std::io::{Read, Write};
        checked_directory(&self.path)?;
        let catalog_setting = catalog_path.map_or(Ok(String::new()), |path| {
            let path = path
                .to_str()
                .filter(|value| !value.contains(['\n', '\r', '\0']))
                .ok_or(DelegationErrorV1::InvalidArguments)?;
            Ok::<_, DelegationErrorV1>(format!(
                "model_catalog_json = {}\n",
                serde_json::to_string(path).map_err(|_| DelegationErrorV1::InvalidArguments)?
            ))
        })?;
        let contents = format!(
            "model_provider = \"hiroute\"\n{catalog_setting}[model_providers.hiroute]\nname = \"HiRoute managed run\"\nbase_url = \"http://{gateway}/v1\"\nwire_api = \"responses\"\nenv_key = \"HIROUTE_RUN_TOKEN\"\nrequires_openai_auth = false\n"
        );
        let path = self.path.join("config.toml");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => file
                .write_all(contents.as_bytes())
                .map_err(|_| DelegationErrorV1::StorageUnavailable),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                if linked(&metadata) || !metadata.is_file() {
                    return Err(DelegationErrorV1::PermissionDenied);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.uid() != nix::unistd::geteuid().as_raw()
                        || metadata.mode() & 0o077 != 0
                    {
                        return Err(DelegationErrorV1::PermissionDenied);
                    }
                }
                let mut actual = String::new();
                fs::File::open(path)
                    .and_then(|file| file.take(4097).read_to_string(&mut actual))
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                if actual == contents {
                    Ok(())
                } else {
                    Err(DelegationErrorV1::Conflict)
                }
            }
            Err(_) => Err(DelegationErrorV1::StorageUnavailable),
        }
    }

    /// The task root is a private Codex target. Its catalog contains only the frozen Plan alias;
    /// no user's native catalog or cache is copied into it. A Continue must see identical bytes.
    pub(super) fn prepare_codex_catalog(
        &self,
        alias: &str,
        contents: &[u8],
    ) -> Result<PathBuf, DelegationErrorV1> {
        use std::io::{Read, Write};
        checked_directory(&self.path)?;
        if contents.is_empty() || contents.len() > 1024 * 1024 {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        let value: Value = serde_json::from_slice(contents)
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        hiroute_integrations::agents::CodexCatalogSelection::for_current_adapter(value.clone())
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        if value["models"]
            .as_array()
            .is_none_or(|models| models.len() != 1 || models[0]["slug"].as_str() != Some(alias))
        {
            return Err(DelegationErrorV1::Conflict);
        }
        let path = self.path.join("worker-model-catalog.json");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(contents)
                    .and_then(|()| file.sync_all())
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                fs::File::open(&self.path)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                if linked(&metadata) || !metadata.is_file() || metadata.len() > 1024 * 1024 {
                    return Err(DelegationErrorV1::PermissionDenied);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.uid() != nix::unistd::geteuid().as_raw()
                        || metadata.mode() & 0o077 != 0
                        || metadata.nlink() != 1
                    {
                        return Err(DelegationErrorV1::PermissionDenied);
                    }
                }
                let mut actual = Vec::new();
                open_marker(&path)?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut actual)
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
                if actual != contents {
                    return Err(DelegationErrorV1::Conflict);
                }
            }
            Err(_) => return Err(DelegationErrorV1::StorageUnavailable),
        }
        Ok(path)
    }
}

/// Resolve the one adapter-owned append-only transcript for an exact native session. The
/// relative result is suitable both for continuation retention and for the lifecycle's bounded
/// post-turn flush observation; neither caller may infer a path from an unverified identifier.
pub(crate) fn native_history(
    root: &Path,
    harness: WorkerHarnessV1,
    native_session_id: &str,
) -> Result<Vec<String>, DelegationErrorV1> {
    if native_session_id.is_empty()
        || native_session_id.len() > 256
        || native_session_id
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let root = fs::canonicalize(root).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    let metadata = fs::symlink_metadata(&root).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
    }
    let durable_root = root.join(match harness {
        WorkerHarnessV1::CodexCli => "sessions",
        WorkerHarnessV1::ClaudeCode => "projects",
    });
    let metadata =
        fs::symlink_metadata(&durable_root).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let codex_suffix = format!("-{native_session_id}.jsonl");
    let claude_name = format!("{native_session_id}.jsonl");
    let mut pending = vec![durable_root];
    let mut paths = Vec::new();
    let mut visited = 0_usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|_| DelegationErrorV1::ResumeUnavailable)? {
            let entry = entry.map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
            visited = visited
                .checked_add(1)
                .ok_or(DelegationErrorV1::ResumeUnavailable)?;
            if visited > 512 {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
            if metadata.file_type().is_symlink() {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(DelegationErrorV1::ResumeUnavailable)?;
            let matches_session = match harness {
                WorkerHarnessV1::CodexCli => file_name.ends_with(&codex_suffix),
                WorkerHarnessV1::ClaudeCode => file_name == claude_name,
            };
            if !matches_session {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .ok()
                .and_then(Path::to_str)
                .filter(|value| valid_native_history_path(value))
                .ok_or(DelegationErrorV1::ResumeUnavailable)?;
            paths.push(relative.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    if paths.len() != 1 {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    Ok(paths)
}

pub(super) fn checked_run_root(
    private_root: &Path,
    session_root: &TaskSessionRoot,
) -> Result<PathBuf, DelegationErrorV1> {
    let name = private_root
        .file_name()
        .ok_or(DelegationErrorV1::InvalidArguments)?;
    if !private_root.is_absolute() || !portable_relative(Path::new(name)) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let parent = checked_directory(
        private_root
            .parent()
            .ok_or(DelegationErrorV1::InvalidArguments)?,
    )?;
    let session = checked_directory(session_root.path())?;
    // Actual filesystem-resolved component ancestry, not textual prefix comparisons. A new
    // root cannot be an ancestor of an existing session directory without itself existing.
    let path = parent.join(name);
    if parent.starts_with(&session) || session.starts_with(&path) {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        _ => Err(DelegationErrorV1::Conflict),
    }
}

fn check_history(root: &Path, relative: &Path) -> Result<(), DelegationErrorV1> {
    if !portable_relative(relative) {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let mut path = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        path.push(component.as_os_str());
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        if linked(&metadata)
            || (index + 1 < components.len() && !metadata.is_dir())
            || (index + 1 == components.len() && !metadata.is_file())
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
    }
    // Existing body/native-cache ownership and deletion gates remain authoritative. Do not
    // import/read history here or infer a missing session from an arbitrary newest file.
    Ok(())
}

fn portable_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|c| match c {
            Component::Normal(s) => s
                .to_str()
                .is_some_and(|s| !s.contains(['\\', ':', '\0', '\r', '\n'])),
            _ => false,
        })
}

fn checked_directory(path: &Path) -> Result<PathBuf, DelegationErrorV1> {
    if !path.is_absolute() {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    if linked(&metadata) || !metadata.is_dir() {
        return Err(DelegationErrorV1::PermissionDenied);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(DelegationErrorV1::PermissionDenied);
        }
    }
    // As with the existing non-Unix managed directory checks, Windows roots must be supplied
    // from the already owner-controlled daemon store. This helper is not a new ACL provider.
    fs::canonicalize(path).map_err(|_| DelegationErrorV1::CapabilityUnavailable)
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        } // reparse point
    }
    metadata.file_type().is_symlink()
}

fn open_marker(path: &Path) -> Result<fs::File, DelegationErrorV1> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    options
        .open(path)
        .map_err(|_| DelegationErrorV1::ResumeUnavailable)
}

fn filesystem_identity(
    base: &Path,
    root: &Path,
    marker: &fs::Metadata,
) -> Result<DelegationNativeFilesystemIdentityV1, DelegationErrorV1> {
    let base = fs::metadata(base).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    let root = fs::metadata(root).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return Ok(DelegationNativeFilesystemIdentityV1 {
            scheme: "unix-dev-inode-v1".into(),
            base_device: base.dev(),
            base_inode: base.ino(),
            root_device: root.dev(),
            root_inode: root.ino(),
            marker_device: marker.dev(),
            marker_inode: marker.ino(),
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Ok(DelegationNativeFilesystemIdentityV1 {
            scheme: "windows-volume-file-index-v1".into(),
            base_device: u64::from(base.volume_serial_number().unwrap_or(0)),
            base_inode: base.file_index().unwrap_or(0),
            root_device: u64::from(root.volume_serial_number().unwrap_or(0)),
            root_inode: root.file_index().unwrap_or(0),
            marker_device: u64::from(marker.volume_serial_number().unwrap_or(0)),
            marker_inode: marker.file_index().unwrap_or(0),
        });
    }
    #[allow(unreachable_code)]
    Err(DelegationErrorV1::CapabilityUnavailable)
}
