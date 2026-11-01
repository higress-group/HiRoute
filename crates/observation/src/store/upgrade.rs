//! Read-only format admission. Known old observations are reset by startup;
//! historical rows and objects are never converted here.
#[cfg(test)]
mod tests;
use crate::writer::ObservationStoreError as Error;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn version(root: &Path) -> Result<Option<String>, Error> {
    if !root.join("activity.db").exists() {
        return Ok(None);
    }
    hiroute_platform::private_fs::open_private_file(&root.join("activity.db"))
        .map_err(|_| Error::Corrupt)?;
    let db =
        Connection::open_with_flags(root.join("activity.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::Corrupt)?;
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='observation_meta' AND type='table')", [], |r|r.get(0)).map_err(|_|Error::Corrupt)?;
    if !exists {
        return Err(Error::Corrupt);
    }
    db.query_row(
        "SELECT value FROM observation_meta WHERE key='schema_version'",
        [],
        |r| r.get(0),
    )
    .optional()
    .map_err(|_| Error::Corrupt)?
    .map(Some)
    .ok_or(Error::Corrupt)
}

fn paths(root: &Path) -> Result<(PathBuf, PathBuf, PathBuf), Error> {
    let parent = root.parent().ok_or(Error::Io)?;
    let name = root
        .file_name()
        .ok_or(Error::Io)?
        .to_str()
        .ok_or(Error::Io)?;
    Ok((
        parent.join(format!("{name}.pre-v5-source")),
        parent.join(format!("{name}.v5-target")),
        parent.join(format!("{name}.upgrade.json")),
    ))
}

pub(crate) fn required(root: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(root) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err(Error::Corrupt),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(Error::Io),
        _ => {}
    }
    let (old, target, journal) = paths(root)?;
    if journal.exists() || target.exists() {
        // An earlier converter may have retained real target data. Preserve all
        // generations for whole-group recovery rather than guessing a reset.
        return Err(Error::Corrupt);
    }
    if old.exists() && !root.exists() {
        return Err(Error::Corrupt);
    }
    match version(root)?.as_deref() {
        None => {
            if root.exists() {
                for entry in fs::read_dir(root).map_err(|_| Error::Io)? {
                    let entry = entry.map_err(|_| Error::Io)?;
                    if entry.file_name() == ".DS_Store" {
                        continue;
                    }
                    if entry.file_name() != "conversation-content"
                        || !entry.file_type().map_err(|_| Error::Io)?.is_dir()
                    {
                        return Err(Error::Corrupt);
                    }
                    for child in fs::read_dir(entry.path()).map_err(|_| Error::Io)? {
                        let child = child.map_err(|_| Error::Io)?;
                        if !matches!(child.file_name().to_str(), Some("staging" | "blobs"))
                            || !child.file_type().map_err(|_| Error::Io)?.is_dir()
                            || fs::read_dir(child.path())
                                .map_err(|_| Error::Io)?
                                .next()
                                .is_some()
                        {
                            return Err(Error::Corrupt);
                        }
                    }
                }
            }
            Ok(false)
        }
        Some("6") => {
            let db = Connection::open_with_flags(
                root.join("activity.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|_| Error::Corrupt)?;
            super::schema::validate_current(&db)?;
            Ok(false)
        }
        Some("4") => {
            validate_legacy_tables(root)?;
            Ok(true)
        }
        Some("1" | "2" | "3" | "5") => Ok(true),
        _ => Err(Error::Corrupt),
    }
}

fn validate_legacy_tables(root: &Path) -> Result<(), Error> {
    let db =
        Connection::open_with_flags(root.join("activity.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::Corrupt)?;
    let count:u64=db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN('logical_requests','observation_checkpoints','content_blobs_v2','content_instances_v2')",[],|r|r.get(0)).map_err(|_|Error::Corrupt)?;
    if count != 4 {
        return Err(Error::Corrupt);
    }
    Ok(())
}

pub(crate) fn prepare(root: &Path) -> Result<(), Error> {
    if required(root)? {
        return Err(Error::Corrupt);
    }
    Ok(())
}
