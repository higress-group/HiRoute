//! Account for simultaneous installation allocations on their actual volumes.
use super::*;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
#[error(
    "UPGRADE_INSUFFICIENT_SPACE: the volume at {location} needs at least {required_bytes} free bytes; {available_bytes} are available. Keep history and recovery packages, free unrelated files, then retry. / 此位置所在磁盘空间不足；请保留历史和恢复包，释放无关文件后再试。",
    location = .volume.display()
)]
pub struct ReplacementSpaceFailure {
    pub volume: PathBuf,
    pub required_bytes: u64,
    pub available_bytes: u64,
}

fn existing(path: &std::path::Path) -> io::Result<PathBuf> {
    let mut path = path;
    loop {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                security::open_identity(path, true)?;
                return Ok(path.to_owned());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                path = path.parent().ok_or_else(invalid)?;
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn preflight_desktop_space(
    package: &Package,
    shell_state: &std::path::Path,
    shell_bytes: u64,
    recovery_copy_bytes: u64,
    storage: Option<&crate::UpgradePreflight>,
) -> io::Result<()> {
    let layout = layout()?;
    let payload: u64 = package.files.values().map(|file| file.size).sum();
    let mut allocations = vec![
        (
            shell_state.to_owned(),
            payload
                .saturating_add(shell_bytes)
                .saturating_add(recovery_copy_bytes)
                .saturating_add(64 * 1024 * 1024),
        ),
        (
            layout.home.join(".local/lib/hiroute"),
            payload.saturating_add(32 * 1024 * 1024),
        ),
    ];
    if let Some(report) = storage
        && report.required_bytes > 0
    {
        allocations.push((layout.storage_root(), report.required_bytes));
    }
    let mut volumes: BTreeMap<u32, (PathBuf, u64)> = BTreeMap::new();
    for (path, bytes) in allocations {
        let path = existing(&path)?;
        let file = security::open_identity(&path, true)?;
        let volume = security::file_identity(&file)?.0;
        let entry = volumes.entry(volume).or_insert((path, 0));
        entry.1 = entry.1.saturating_add(bytes);
    }
    for (_, (volume, required_bytes)) in volumes {
        let available_bytes = fs2::available_space(&volume)?;
        if available_bytes < required_bytes {
            return Err(io::Error::other(ReplacementSpaceFailure {
                volume,
                required_bytes,
                available_bytes,
            }));
        }
    }
    Ok(())
}
