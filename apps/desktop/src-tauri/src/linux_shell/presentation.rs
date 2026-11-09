use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{ErrorKind, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Language {
    #[default]
    System,
    En,
    Zh,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Presentation {
    theme: Theme,
    language: Language,
}

const UNAVAILABLE: &str = "SHELL_PRESENTATION_UNAVAILABLE";

fn path() -> Result<PathBuf, &'static str> {
    let home = super::runtime::layout()?.home;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    Ok(config.join("hiroute-desktop/presentation.json"))
}

fn directory(path: &Path) -> Result<bool, &'static str> {
    let parent = path.parent().ok_or(UNAVAILABLE)?;
    let metadata = match parent.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(UNAVAILABLE),
    };
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
        || parent.canonicalize().ok().as_deref() != Some(parent)
    {
        return Err(UNAVAILABLE);
    }
    Ok(true)
}

fn read(path: &Path) -> Result<Presentation, &'static str> {
    if !directory(path)? {
        return Ok(Presentation::default());
    }
    let file = match hiroute_platform::private_fs::open_private_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Presentation::default()),
        Err(_) => return Err(UNAVAILABLE),
    };
    let meta = file.metadata().map_err(|_| UNAVAILABLE)?;
    if meta.uid() != nix::unistd::Uid::effective().as_raw()
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
    {
        return Err(UNAVAILABLE);
    }
    let mut bytes = Vec::new();
    file.take(513)
        .read_to_end(&mut bytes)
        .map_err(|_| UNAVAILABLE)?;
    if bytes.len() > 512 {
        return Err(UNAVAILABLE);
    }
    serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE)
}

pub(super) fn get() -> Result<Presentation, &'static str> {
    read(&path()?)
}

pub(super) fn set(value: Presentation) -> Result<(), &'static str> {
    let path = path()?;
    if !directory(&path)? {
        let parent = path.parent().ok_or(UNAVAILABLE)?;
        fs::create_dir_all(parent.parent().ok_or(UNAVAILABLE)?).map_err(|_| UNAVAILABLE)?;
        hiroute_platform::private_fs::create_directory(parent).map_err(|_| UNAVAILABLE)?;
    }
    // Refuse a foreign or unreadable existing record before replacing it.
    read(&path)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| UNAVAILABLE)?;
    hiroute_platform::durable::write_private_atomic(&path, &bytes).map_err(|_| UNAVAILABLE)
}
