//! Private handoff from the standalone CLI to its independently launched service.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const SCHEMA: &str = "hiroute.standalone-proxy-environment/v1";
const MAX_BYTES: u64 = 16 * 1024;
const KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceProxyEnvironment {
    schema: String,
    variables: BTreeMap<String, String>,
}

impl std::fmt::Debug for ServiceProxyEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceProxyEnvironment([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("STANDALONE_PROXY_ENVIRONMENT_INVALID")]
pub struct ServiceProxyError;

impl ServiceProxyEnvironment {
    pub fn capture(
        values: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ServiceProxyError> {
        let mut variables = BTreeMap::new();
        for (key, value) in values {
            if let Some(key) = key.to_str().filter(|key| KEYS.contains(key)) {
                variables.insert(
                    key.to_owned(),
                    value.into_string().map_err(|_| ServiceProxyError)?,
                );
            }
        }
        let result = Self {
            schema: SCHEMA.into(),
            variables,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn variables(&self) -> impl Iterator<Item = (OsString, OsString)> + '_ {
        self.variables
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
    }

    fn validate(&self) -> Result<(), ServiceProxyError> {
        if self.schema != SCHEMA
            || self.variables.iter().any(|(key, value)| {
                !KEYS.contains(&key.as_str()) || value.contains(['\0', '\r', '\n'])
            })
            || serde_json::to_vec(self)
                .map_err(|_| ServiceProxyError)?
                .len() as u64
                > MAX_BYTES
        {
            return Err(ServiceProxyError);
        }
        Ok(())
    }

    /// This lives beside the fixed installation marker, independent of shell XDG overrides.
    pub fn path(home: &Path) -> PathBuf {
        home.join(".local/share/hiroute/service/proxy-environment.json")
    }

    pub fn store(&self, home: &Path) -> Result<(), ServiceProxyError> {
        self.validate()?;
        let path = Self::path(home);
        validate_directories(home, true)?;
        if let Some(file) = open_private(&path)? {
            drop(file);
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| ServiceProxyError)?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let temporary = path.with_file_name(format!(".proxy-{suffix}.tmp"));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
            }
            let mut file = options.open(&temporary).map_err(|_| ServiceProxyError)?;
            file.write_all(&serde_json::to_vec(self).map_err(|_| ServiceProxyError)?)
                .and_then(|()| file.sync_all())
                .map_err(|_| ServiceProxyError)?;
            fs::rename(&temporary, &path).map_err(|_| ServiceProxyError)
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    pub fn load(home: &Path) -> Result<Option<Self>, ServiceProxyError> {
        if !validate_directories(home, false)? {
            return Ok(None);
        }
        let Some(file) = open_private(&Self::path(home))? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceProxyError)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(ServiceProxyError);
        }
        let result: Self = serde_json::from_slice(&bytes).map_err(|_| ServiceProxyError)?;
        result.validate()?;
        Ok(Some(result))
    }
}

fn validate_directories(home: &Path, create: bool) -> Result<bool, ServiceProxyError> {
    if !home.is_absolute() {
        return Err(ServiceProxyError);
    }
    let mut path = home.to_path_buf();
    for component in ["", ".local", "share", "hiroute", "service"] {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(ServiceProxyError);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    if metadata.uid() != nix::unistd::geteuid().as_raw()
                        || metadata.permissions().mode() & 0o022 != 0
                    {
                        return Err(ServiceProxyError);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(&path).map_err(|_| ServiceProxyError)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(ServiceProxyError),
        }
    }
    Ok(true)
}

fn open_private(path: &Path) -> Result<Option<File>, ServiceProxyError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ServiceProxyError),
    };
    let metadata = file.metadata().map_err(|_| ServiceProxyError)?;
    if !metadata.is_file() {
        return Err(ServiceProxyError);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ServiceProxyError);
        }
    }
    Ok(Some(file))
}

#[cfg(test)]
#[path = "service_proxy_tests.rs"]
mod tests;
