//! A task owns its context descriptor and continuation binding, never the borrowed client root.
//! Native transcripts remain opaque. Only Claude's exact file is inspected for flush/readiness;
//! Codex/Qoder availability is established by exact session/load, not a shared-history scan.
use super::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{Read, Write};
use unicode_normalization::UnicodeNormalization;

const CONTEXT_FILE: &str = "borrowed-native-context.json";
const SESSION_FILE: &str = "native-session-binding.json";
const METADATA_LIMIT: u64 = 32 * 1024;

/// The original context selected for a task. These paths convey no cleanup ownership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorrowedNativeContext {
    pub home: PathBuf,
    pub config_root: PathBuf,
    pub workspace: PathBuf,
}

#[derive(Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextDescriptor {
    version: u32,
    harness: WorkerHarnessV1,
    context: BorrowedNativeContext,
}

#[derive(Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBinding {
    version: u32,
    context_digest: CanonicalDigest,
    native_session_id: String,
}

impl TaskSessionRoot {
    /// Bind a new task once. Continuing an old private task never implicitly migrates it.
    pub fn bind_borrowed_context(
        &self,
        home: &Path,
        config_root: &Path,
        workspace: &Path,
    ) -> Result<(), DelegationErrorV1> {
        if !self.newly_created {
            return Err(DelegationErrorV1::Conflict);
        }
        let context = BorrowedNativeContext {
            home: canonical_context_path(home, false)?,
            config_root: canonical_context_path(config_root, true)?,
            workspace: canonical_context_path(workspace, false)?,
        };
        if context.config_root.starts_with(&self.path) {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        write_owned_json(
            &self.path,
            CONTEXT_FILE,
            &ContextDescriptor {
                version: 1,
                harness: self.harness,
                context,
            },
        )
    }

    /// Only Codex/Claude accept a missing descriptor as the registered legacy private-root
    /// format. Malformed, replaced or mismatched metadata never selects a recovery reader.
    pub fn borrowed_context(&self) -> Result<Option<BorrowedNativeContext>, DelegationErrorV1> {
        read_context(&self.path, self.harness).map(|value| value.map(|value| value.context))
    }

    /// Match the task's persisted ACP identity before attempting Continue. For borrowed Codex/Qoder,
    /// this proves only the retained binding: session/load must still confirm native history.
    pub fn verify_native_session(&self, native_session_id: &str) -> Result<(), DelegationErrorV1> {
        match read_context(&self.path, self.harness)? {
            Some(context) => {
                let binding = read_binding(&self.path, &context)?;
                if binding.native_session_id != native_session_id {
                    return Err(DelegationErrorV1::ResumeUnavailable);
                }
                verify_borrowed_history(&context, native_session_id)
            }
            None => legacy_native_history(&self.path, self.harness, native_session_id).map(|_| ()),
        }
    }

    /// Locate an actual transcript for lifecycle observation. A Codex/Qoder borrowed binding
    /// is deliberately not returned as a transcript; its contents say nothing about disk flush.
    pub(crate) fn native_transcript_path(
        root: &Path,
        harness: WorkerHarnessV1,
        native_session_id: &str,
    ) -> Result<PathBuf, DelegationErrorV1> {
        if let Some(context) = read_context(root, harness)? {
            return claude_transcript(&context, native_session_id);
        }
        legacy_native_history(root, harness, native_session_id)?
            .pop()
            .map(|relative| root.join(relative))
            .ok_or(DelegationErrorV1::ResumeUnavailable)
    }
}

pub(super) fn read_context(
    root: &Path,
    harness: WorkerHarnessV1,
) -> Result<Option<ContextDescriptor>, DelegationErrorV1> {
    checked_directory(root).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    match fs::symlink_metadata(root.join(CONTEXT_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return match fs::symlink_metadata(root.join(SESSION_FILE)) {
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && harness != WorkerHarnessV1::QoderCli =>
                {
                    Ok(None)
                }
                _ => Err(DelegationErrorV1::ResumeUnavailable),
            };
        }
        Err(_) => return Err(DelegationErrorV1::ResumeUnavailable),
        Ok(_) => {}
    }
    let descriptor: ContextDescriptor = read_owned_json(root, CONTEXT_FILE)?;
    if descriptor.version != 1 || descriptor.harness != harness {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let context = &descriptor.context;
    for (path, missing) in [
        (&context.home, false),
        (&context.config_root, true),
        (&context.workspace, false),
    ] {
        if canonical_context_path(path, missing)
            .map_err(|_| DelegationErrorV1::ResumeUnavailable)?
            != *path
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
    }
    if context.config_root.starts_with(root) {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    Ok(Some(descriptor))
}

pub(super) fn retain_borrowed_session(
    root: &Path,
    harness: WorkerHarnessV1,
    native_session_id: &str,
) -> Result<Vec<String>, DelegationErrorV1> {
    let context = read_context(root, harness)?.ok_or(DelegationErrorV1::ResumeUnavailable)?;
    verify_borrowed_history(&context, native_session_id)?;
    write_owned_json(
        root,
        SESSION_FILE,
        &SessionBinding {
            version: 1,
            context_digest: context_digest(&context)?,
            native_session_id: native_session_id.to_owned(),
        },
    )?;
    Ok(vec![SESSION_FILE.to_owned()])
}

pub(super) fn verify_continuation_materials(
    root: &Path,
    harness: WorkerHarnessV1,
    required: &[PathBuf],
) -> Result<(), DelegationErrorV1> {
    if let Some(context) = read_context(root, harness)? {
        if required != [PathBuf::from(SESSION_FILE)] {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let binding = read_binding(root, &context)?;
        verify_borrowed_history(&context, &binding.native_session_id)?;
    }
    Ok(())
}

fn read_binding(
    root: &Path,
    context: &ContextDescriptor,
) -> Result<SessionBinding, DelegationErrorV1> {
    let binding: SessionBinding = read_owned_json(root, SESSION_FILE)?;
    if binding.version != 1 || binding.context_digest != context_digest(context)? {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    validate_session_id(&binding.native_session_id)?;
    Ok(binding)
}

fn context_digest(context: &ContextDescriptor) -> Result<CanonicalDigest, DelegationErrorV1> {
    CanonicalDigest::of(context).map_err(|_| DelegationErrorV1::ResumeUnavailable)
}

fn verify_borrowed_history(
    context: &ContextDescriptor,
    native_session_id: &str,
) -> Result<(), DelegationErrorV1> {
    validate_session_id(native_session_id)?;
    if context.harness == WorkerHarnessV1::ClaudeCode {
        claude_transcript(context, native_session_id)?;
    }
    Ok(())
}

fn claude_transcript(
    descriptor: &ContextDescriptor,
    native_session_id: &str,
) -> Result<PathBuf, DelegationErrorV1> {
    validate_session_id(native_session_id)?;
    if descriptor.harness != WorkerHarnessV1::ClaudeCode {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let context = &descriptor.context;
    let relative = PathBuf::from("projects")
        .join(claude_workspace_key(&context.workspace)?)
        .join(format!("{native_session_id}.jsonl"));
    // This checks only the exact path's components, never siblings or the shared root's mode.
    check_history(&context.config_root, &relative)?;
    Ok(context.config_root.join(relative))
}

fn validate_session_id(id: &str) -> Result<(), DelegationErrorV1> {
    if id.is_empty()
        || id.len() > 256
        || id
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
    {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    Ok(())
}

/// Claude Code 2.1.231 and SDK 0.3.215 encode NFC cwd using JavaScript UTF-16 code units.
/// The 200-character key limit and signed 32-bit hash also apply to non-BMP workspace names.
fn claude_workspace_key(workspace: &Path) -> Result<String, DelegationErrorV1> {
    let text = workspace
        .to_str()
        .ok_or(DelegationErrorV1::ResumeUnavailable)?;
    let normalized: String = text.nfc().collect();
    let mut hash = 0_i32;
    let mut key = String::new();
    for unit in normalized.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(i32::from(unit));
        key.push(if unit <= 127 && (unit as u8).is_ascii_alphanumeric() {
            char::from(unit as u8)
        } else {
            '-'
        });
    }
    if key.len() > 200 {
        key.truncate(200);
        key.push('-');
        key.push_str(&base36(hash.unsigned_abs()));
    }
    Ok(key)
}

fn base36(mut value: u32) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from(
            b"0123456789abcdefghijklmnopqrstuvwxyz"[(value % 36) as usize],
        ));
        value /= 36;
        if value == 0 {
            return digits.into_iter().rev().collect();
        }
    }
}

fn canonical_context_path(path: &Path, allow_missing: bool) -> Result<PathBuf, DelegationErrorV1> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|path| path.contains(['\0', '\r', '\n']))
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    match fs::canonicalize(path) {
        Ok(path) if path.is_dir() => Ok(path),
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            // Clients may create a default config root on their first run. Resolve its existing
            // prefix without creating, chmodding, copying or adopting anything in the user root.
            let parent = path.parent().ok_or(DelegationErrorV1::InvalidArguments)?;
            let name = path
                .file_name()
                .ok_or(DelegationErrorV1::InvalidArguments)?;
            Ok(canonical_context_path(parent, true)?.join(name))
        }
        _ => Err(DelegationErrorV1::ResumeUnavailable),
    }
}

fn read_owned_json<T: DeserializeOwned>(root: &Path, name: &str) -> Result<T, DelegationErrorV1> {
    let path = root.join(name);
    let path_metadata =
        fs::symlink_metadata(&path).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    if linked(&path_metadata) || !path_metadata.is_file() || path_metadata.len() > METADATA_LIMIT {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let file = open_marker(&path)?;
    let metadata = file
        .metadata()
        .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    if !metadata.is_file() || metadata.len() > METADATA_LIMIT {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
    }
    let mut bytes = Vec::new();
    file.take(METADATA_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    serde_json::from_slice(&bytes).map_err(|_| DelegationErrorV1::ResumeUnavailable)
}

fn write_owned_json<T: Serialize + DeserializeOwned + PartialEq>(
    root: &Path,
    name: &str,
    value: &T,
) -> Result<(), DelegationErrorV1> {
    checked_directory(root)?;
    let bytes = serde_json::to_vec(value).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(root.join(name)) {
        Ok(mut file) => {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
            fs::File::open(root)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| DelegationErrorV1::StorageUnavailable)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_owned_json::<T>(root, name)? == *value {
                Ok(())
            } else {
                Err(DelegationErrorV1::Conflict)
            }
        }
        Err(_) => Err(DelegationErrorV1::StorageUnavailable),
    }
}

#[cfg(test)]
#[path = "native_history_tests.rs"]
mod tests;
