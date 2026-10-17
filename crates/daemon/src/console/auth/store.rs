use super::*;
use hiroute_platform::{durable, private_fs};
use std::io::Read;

pub(super) const MAX_TOKENS: usize = 128;
const MAX_STORE: u64 = 128 * 1024;
const MAX_AUDIT: usize = 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredToken {
    #[serde(flatten)]
    pub metadata: TokenMetadata,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema: String,
    tokens: Vec<StoredToken>,
}

pub(super) fn prepare(root: &std::path::Path) -> Result<(), AuthError> {
    private_fs::prepare_storage_directory(root).map_err(|_| AuthError::Unavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(root).map_err(|_| AuthError::Unavailable)?;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(AuthError::Unavailable);
        }
    }
    Ok(())
}
fn read(root: &std::path::Path, name: &str, limit: u64) -> Result<Option<Vec<u8>>, AuthError> {
    let mut file = match private_fs::open_private_file(&root.join(name)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AuthError::Unavailable),
    };
    private_fs::verify_single_link(&file).map_err(|_| AuthError::Unavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|_| AuthError::Unavailable)?;
        if metadata.uid() != nix::unistd::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(AuthError::Unavailable);
        }
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AuthError::Unavailable)?;
    if bytes.len() as u64 > limit {
        return Err(AuthError::Unavailable);
    }
    Ok(Some(bytes))
}
pub(super) fn load(root: &std::path::Path) -> Result<Vec<StoredToken>, AuthError> {
    let Some(bytes) = read(root, "tokens.json", MAX_STORE)? else {
        return Ok(vec![]);
    };
    let store: Store = serde_json::from_slice(&bytes).map_err(|_| AuthError::Unavailable)?;
    let mut ids = std::collections::BTreeSet::new();
    if store.schema != "hiroute.console-tokens/v1"
        || store.tokens.len() > MAX_TOKENS
        || store.tokens.iter().any(|t| {
            !super::valid_secret(&t.sha256)
                || !super::valid_secret(&t.metadata.id)
                || !super::valid_name(&t.metadata.name)
                || t.metadata.created == 0
                || t.metadata.expires.is_some_and(|e| e <= t.metadata.created)
                || !ids.insert(&t.metadata.id)
        })
    {
        return Err(AuthError::Unavailable);
    }
    Ok(store.tokens)
}
pub(super) fn save(root: &std::path::Path, tokens: &[StoredToken]) -> Result<(), AuthError> {
    let bytes = serde_json::to_vec(&Store {
        schema: "hiroute.console-tokens/v1".into(),
        tokens: tokens.to_vec(),
    })
    .map_err(|_| AuthError::Unavailable)?;
    durable::write_private_atomic(&root.join("tokens.json"), &bytes)
        .map_err(|_| AuthError::Unavailable)
}
/// Append bytes without rewriting or interpreting earlier records. Rotation retains two
/// bounded generations, all published through the platform's private-file boundary.
pub(super) fn audit(root: &std::path::Path, event: &AuditEvent<'_>) -> Result<(), AuthError> {
    let mut bytes = read(root, "audit.jsonl", MAX_AUDIT as u64)?.unwrap_or_default();
    let mut row = serde_json::to_vec(event).map_err(|_| AuthError::Unavailable)?;
    row.push(b'\n');
    if bytes.len() + row.len() > MAX_AUDIT {
        if let Some(previous) = read(root, "audit.1.jsonl", MAX_AUDIT as u64)? {
            durable::write_private_atomic(&root.join("audit.2.jsonl"), &previous)
                .map_err(|_| AuthError::Unavailable)?;
        }
        durable::write_private_atomic(&root.join("audit.1.jsonl"), &bytes)
            .map_err(|_| AuthError::Unavailable)?;
        bytes.clear();
    }
    bytes.extend_from_slice(&row);
    durable::write_private_atomic(&root.join("audit.jsonl"), &bytes)
        .map_err(|_| AuthError::Unavailable)
}
