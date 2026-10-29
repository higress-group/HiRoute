//! Bounded test-only evidence for executable metadata drift. Nothing here authorizes Save.
use hiroute_domain::CanonicalDigest;
use serde_json::{Value, json};
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const CONTENT_LIMIT: u64 = 1024 * 1024;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;

// Keep ctime: the diagnostic must expose it, never normalize the production guard away.
fn strict_stat(meta: &Metadata) -> Value {
    json!({
        "dev":meta.dev(), "ino":meta.ino(), "uid":meta.uid(), "gid":meta.gid(),
        "mode":meta.mode(), "nlink":meta.nlink(), "len":meta.len(),
        "mtime":(meta.mtime(),meta.mtime_nsec()), "ctime":(meta.ctime(),meta.ctime_nsec()),
    })
}

fn content(file: &File, length: u64) -> Value {
    if length > CONTENT_LIMIT {
        return json!({"state":"limit"});
    }
    let mut bytes = Vec::new();
    match file.take(CONTENT_LIMIT + 1).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() as u64 > CONTENT_LIMIT => json!({"state":"limit"}),
        Ok(_) if bytes.len() as u64 != length => json!({"state":"changed"}),
        Ok(_) => json!({"state":"complete", "digest":CanonicalDigest::of_bytes(&bytes)}),
        Err(_) => json!({"state":"unavailable"}),
    }
}

pub(super) fn snapshot(path: &Path) -> Value {
    snapshot_with(path, || {})
}

fn snapshot_with(path: &Path, after_read: impl FnOnce()) -> Value {
    let Ok(file) = hiroute_platform::private_fs::open_regular_file(path) else {
        return json!({"state":"unavailable"});
    };
    let Ok(before) = file.metadata() else {
        return json!({"state":"unavailable"});
    };
    let body = content(&file, before.len());
    #[cfg(target_os = "macos")]
    let extended = macos::extended(&file);
    #[cfg(not(target_os = "macos"))]
    let extended = json!({"state":"not_sampled"});
    after_read();
    let after = file.metadata().ok().map(|meta| strict_stat(&meta));
    let named = std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| strict_stat(&meta));
    let before = strict_stat(&before);
    let stable = after.as_ref() == Some(&before) && named.as_ref() == Some(&before);
    json!({
        "state":if stable { "stable" } else { "changed" },
        "stat":before, "after":after, "named":named,
        "content":body, "extended":extended,
    })
}
