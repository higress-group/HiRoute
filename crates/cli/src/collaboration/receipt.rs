//! Immutable, credential-free submission binding. User retries reuse the exact Preview/Apply
//! bytes; a crash or corrupt record never authorizes reconstructing a different request.
use std::io::Read;
#[cfg(not(windows))]
use std::io::Write;
use std::path::{Path, PathBuf};

use hiroute_application_api::{
    AgentAccessTokenIntentV1, AgentFacetIntent, AgentSettingsApplyV2, CanonicalDigest, ErrorCode,
};
use serde::{Deserialize, Serialize};

use super::{Intent, Selection};

const SCHEMA: &str = "hiroute.agent-collaboration-cli-receipt/v1";

pub(super) fn root(state: &Path, endpoint: &Path) -> Result<PathBuf, ErrorCode> {
    let digest =
        CanonicalDigest::of(&(SCHEMA, endpoint)).map_err(|_| ErrorCode::DaemonUnavailable)?;
    Ok(state
        .join("cli-collaboration")
        .join(digest.as_str().trim_start_matches("sha256:")))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: String,
    instance: CanonicalDigest,
    selection: Selection,
    apply: AgentSettingsApplyV2,
}

fn instance(root: &Path) -> Result<CanonicalDigest, ErrorCode> {
    CanonicalDigest::of(&(SCHEMA, root)).map_err(|_| ErrorCode::DaemonUnavailable)
}
fn path(root: &Path, key: &str) -> Result<PathBuf, ErrorCode> {
    let digest = CanonicalDigest::of(&(SCHEMA, key)).map_err(|_| ErrorCode::DaemonUnavailable)?;
    Ok(root.join(format!(
        "{}.json",
        digest.as_str().trim_start_matches("sha256:")
    )))
}

pub(super) fn load(
    root: &Path,
    key: &str,
    selection: &Selection,
) -> Result<Option<AgentSettingsApplyV2>, ErrorCode> {
    let path = path(root, key)?;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ErrorCode::DaemonUnavailable),
        Ok(_) => {}
    }
    private(root.parent().ok_or(ErrorCode::DaemonUnavailable)?, true)?;
    private(root, true)?;
    private(&path, false)?;
    let mut bytes = Vec::new();
    #[cfg(windows)]
    let file = hiroute_platform::windows::security::open_owner_only(&path, false);
    #[cfg(not(windows))]
    let file = std::fs::File::open(&path);
    file.and_then(|file| file.take(65_537).read_to_end(&mut bytes))
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    if bytes.len() > 65_536 {
        return Err(ErrorCode::DaemonUnavailable);
    }
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|_| ErrorCode::DaemonUnavailable)?;
    if record.schema != SCHEMA
        || record.instance != instance(root)?
        || record.apply.idempotency_key != key
    {
        return Err(ErrorCode::DaemonUnavailable);
    }
    if &record.selection != selection {
        return Err(ErrorCode::IdempotencyKeyReused);
    }
    let spec = &record.apply.spec;
    let valid_intent = match (&selection.intent, &spec.collaboration) {
        (Intent::Configure { trigger_mode }, AgentFacetIntent::Configure { settings }) => {
            trigger_mode == &settings.trigger_mode
        }
        (Intent::Disable, AgentFacetIntent::Restore { restore_point_ref }) => {
            !restore_point_ref.is_empty()
        }
        _ => false,
    };
    if !valid_intent
        || spec.schema_version != hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2
        || !super::bounded(&spec.context_id, 256)
        || !matches!(spec.model, AgentFacetIntent::Keep)
        || !matches!(spec.access_token, AgentAccessTokenIntentV1::Keep)
        || spec.restore_native_model.is_some()
        || spec.restore_managed_changes
        || record.apply.login_item.is_some()
    {
        return Err(ErrorCode::DaemonUnavailable);
    }
    Ok(Some(record.apply))
}

pub(super) fn freeze(
    root: &Path,
    selection: &Selection,
    apply: AgentSettingsApplyV2,
) -> Result<AgentSettingsApplyV2, ErrorCode> {
    let namespace = root.parent().ok_or(ErrorCode::DaemonUnavailable)?;
    create_private_directory(namespace)?;
    create_private_directory(root)?;
    let destination = path(root, &apply.idempotency_key)?;
    let record = Record {
        schema: SCHEMA.into(),
        instance: instance(root)?,
        selection: selection.clone(),
        apply,
    };
    #[cfg(windows)]
    {
        let mut bytes = serde_json::to_vec(&record).map_err(|_| ErrorCode::DaemonUnavailable)?;
        bytes.push(b'\n');
        hiroute_platform::durable::write_private_new(&destination, &bytes)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        load(root, &record.apply.idempotency_key, selection)?.ok_or(ErrorCode::DaemonUnavailable)
    }
    #[cfg(not(windows))]
    {
        let temporary = root.join(format!(".{}", super::random_key()?));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
            serde_json::to_writer(&mut file, &record).map_err(|_| ErrorCode::DaemonUnavailable)?;
            file.write_all(b"\n")
                .and_then(|_| file.sync_all())
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
            // The first writer wins. A simultaneous same-intent retry must load that writer's
            // frozen request instead of submitting its own newer Preview digest.
            std::fs::hard_link(&temporary, &destination)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
            #[cfg(unix)]
            std::fs::File::open(root)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
            load(root, &record.apply.idempotency_key, selection)?
                .ok_or(ErrorCode::DaemonUnavailable)
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }
}

fn create_private_directory(path: &Path) -> Result<(), ErrorCode> {
    create_directory_tree(path)?;
    private(path, true)
}

fn create_directory_tree(path: &Path) -> Result<(), ErrorCode> {
    #[cfg(windows)]
    {
        let directory = hiroute_platform::windows::security::ensure_private_directory(path)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        hiroute_platform::windows::security::verify_owner_only_handle(&directory, true)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        sync_directory_entry(path)
    }
    #[cfg(not(windows))]
    {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                // Another first writer may have just created it but not synced its parent yet.
                return sync_directory_entry(path);
            }
            Ok(_) => return Err(ErrorCode::DaemonUnavailable),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ErrorCode::DaemonUnavailable),
        }
        let parent = path.parent().ok_or(ErrorCode::DaemonUnavailable)?;
        create_directory_tree(parent)?;
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(path)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        private(path, true)?;
        // A new namespace must itself survive interruption, not only its eventual receipt file.
        // Persist every newly created directory entry before allowing a network submission.
        sync_directory_entry(path)
    }
}

fn sync_directory_entry(path: &Path) -> Result<(), ErrorCode> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
        }
    }
    #[cfg(windows)]
    {
        hiroute_platform::durable::directory_barrier(path)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        if let Some(parent) = path.parent() {
            hiroute_platform::durable::directory_barrier(parent)
                .map_err(|_| ErrorCode::DaemonUnavailable)?;
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = path;
    Ok(())
}

fn private(path: &Path, directory: bool) -> Result<(), ErrorCode> {
    #[cfg(windows)]
    {
        let file = hiroute_platform::windows::security::open_owner_only(path, directory)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        if !directory
            && hiroute_platform::windows::security::file_link_count(&file)
                .map_err(|_| ErrorCode::DaemonUnavailable)?
                != 1
        {
            return Err(ErrorCode::DaemonUnavailable);
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ErrorCode::DaemonUnavailable)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(ErrorCode::DaemonUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::Uid::effective().as_raw() || metadata.mode() & 0o077 != 0
        {
            return Err(ErrorCode::DaemonUnavailable);
        }
    }
    Ok(())
}
