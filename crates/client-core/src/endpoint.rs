use crate::FailureCode;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct LocalEndpoint {
    pub(crate) path: PathBuf,
    pub(crate) expected_pid: Option<u32>,
}

impl LocalEndpoint {
    pub fn from_runtime_root(root: impl AsRef<Path>) -> Self {
        Self {
            path: root.as_ref().join("hiroute/control.sock"),
            expected_pid: None,
        }
    }

    /// The native launcher must obtain this PID from its own child handle, not a WebView.
    pub fn for_child(root: impl AsRef<Path>, child_pid: u32) -> Self {
        Self {
            expected_pid: Some(child_pid),
            ..Self::from_runtime_root(root)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    pub(crate) fn validate_path(&self) -> Result<(), FailureCode> {
        use std::os::unix::fs::FileTypeExt;
        let parent = self.path.parent().ok_or(FailureCode::LocatorUnavailable)?;
        let directory =
            std::fs::symlink_metadata(parent).map_err(|_| FailureCode::LocatorUnavailable)?;
        let socket =
            std::fs::symlink_metadata(&self.path).map_err(|_| FailureCode::LocatorUnavailable)?;
        if !directory.is_dir() || !socket.file_type().is_socket() {
            return Err(FailureCode::PeerRejected);
        }
        let absolute = std::path::absolute(parent).map_err(|_| FailureCode::LocatorUnavailable)?;
        for ancestor in absolute.ancestors() {
            let metadata =
                std::fs::symlink_metadata(ancestor).map_err(|_| FailureCode::LocatorUnavailable)?;
            if metadata.file_type().is_symlink() {
                // macOS exposes these immutable system aliases. Other ancestor symlinks would
                // allow a caller to retarget the private directory after endpoint selection.
                let allowed = [("/tmp", "/private/tmp"), ("/var", "/private/var")]
                    .iter()
                    .any(|(alias, target)| {
                        ancestor == Path::new(alias)
                            && std::fs::canonicalize(ancestor).ok().as_deref()
                                == Some(Path::new(target))
                    });
                if !allowed {
                    return Err(FailureCode::PeerRejected);
                }
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn validate_peer(&self, stream: &tokio::net::UnixStream) -> Result<(), FailureCode> {
        if let Some(expected) = self.expected_pid {
            #[cfg(target_os = "macos")]
            let pid = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::LocalPeerPid)
                .map_err(|_| FailureCode::PeerRejected)? as u32;
            #[cfg(any(target_os = "linux", target_os = "android"))]
            let pid = stream
                .peer_cred()
                .map_err(|_| FailureCode::PeerRejected)?
                .pid()
                .ok_or(FailureCode::PeerRejected)? as u32;
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
            let pid = {
                return Err(FailureCode::PeerRejected);
            };
            if pid != expected {
                return Err(FailureCode::PeerRejected);
            }
        }
        Ok(())
    }
}
