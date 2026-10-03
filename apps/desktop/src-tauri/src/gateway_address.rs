//! Desktop ownership checks around the shared single-listener host contract.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) use hiroute_host_runtime::GatewayPortModeV1;
pub(crate) use hiroute_host_runtime::{GatewayListenerConfigV1, GatewayListenerDesiredV1};
use hiroute_host_runtime::{GatewayListenerReservation, GatewayListenerStore};

pub(crate) fn path(root: &Path) -> PathBuf {
    GatewayListenerStore::new(root).path()
}

pub(crate) fn status(root: &Path) -> Result<GatewayListenerConfigV1, String> {
    GatewayListenerStore::new(root)
        .load_or_default()
        .map_err(|error| error.to_string())
}

pub(crate) fn configure(
    root: &Path,
    desired: GatewayListenerDesiredV1,
) -> Result<GatewayListenerConfigV1, String> {
    GatewayListenerStore::new(root)
        .configure(desired)
        .map_err(|error| error.to_string())
}

pub(crate) fn recover(root: &Path) -> Result<GatewayListenerConfigV1, String> {
    GatewayListenerStore::new(root)
        .recover()
        .map_err(|error| error.to_string())
}

pub(crate) fn mark_applied(root: &Path, listen: SocketAddr) -> Result<(), String> {
    GatewayListenerStore::new(root)
        .mark_applied(listen)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The reserved listener is held until just before the child spawns. The shared store selected
/// and persisted the exact automatic port before returning it.
#[derive(Debug)]
pub(crate) struct Reservation {
    pub listen: SocketAddr,
    inner: GatewayListenerReservation,
}

impl Reservation {
    pub(crate) fn release(self) {
        debug_assert_eq!(self.inner.listen, self.listen);
        let _ = self.inner.release();
    }
}

/// A publication may move only when an explicit listener operation owns the new desired value.
/// There is no reader for the former gateway-address schema.
pub(crate) fn reserve(
    root: &Path,
    last_served: Option<SocketAddr>,
    publication_installed: bool,
) -> Result<Reservation, String> {
    let store = GatewayListenerStore::new(root);
    let before = store.load().map_err(|error| error.to_string())?;
    if publication_installed && before.is_none() {
        return Err("GATEWAY_LISTENER_CONFIG_MISSING".into());
    }
    let inner = store.reserve().map_err(|error| error.to_string())?;
    let listen = inner.listen;
    if publication_installed {
        let served = last_served.ok_or("GATEWAY_LISTENER_APPLIED_MISSING")?;
        let applied = inner
            .config
            .applied_address()
            .map_err(|error| error.to_string())?
            .ok_or("GATEWAY_LISTENER_APPLIED_MISSING")?;
        if applied != served {
            return Err("GATEWAY_LISTENER_APPLIED_MISMATCH".into());
        }
        if listen != served
            && !inner.config.operation.as_ref().is_some_and(|operation| {
                operation.active()
                    && operation.desired.selected_address().ok().flatten() == Some(listen)
            })
        {
            return Err("GATEWAY_LISTENER_CHANGE_NOT_AUTHORIZED".into());
        }
    }
    Ok(Reservation { listen, inner })
}

/// Once a submitted/restarting listener change reaches spawn, any early return must leave an
/// honest failed operation. A successful ready frame disarms the guard after marking applied.
pub(crate) struct ActivationGuard {
    root: PathBuf,
    armed: bool,
}

impl ActivationGuard {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            armed: true,
        }
    }

    pub(crate) fn succeeded(&mut self, listen: SocketAddr) -> Result<(), String> {
        mark_applied(&self.root, listen)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ActivationGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = GatewayListenerStore::new(&self.root).mark_failed("GATEWAY_START_FAILED");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn first_establishment_persists_and_ready_advances_applied() {
        let directory = root();
        let reservation = reserve(directory.path(), None, false).unwrap();
        let listen = reservation.listen;
        assert_ne!(listen.port(), 0);
        reservation.release();
        assert!(status(directory.path()).unwrap().applied.is_none());
        mark_applied(directory.path(), listen).unwrap();
        assert_eq!(
            status(directory.path()).unwrap().applied_address().unwrap(),
            Some(listen)
        );
    }

    #[test]
    fn a_publication_requires_current_applied_evidence_or_an_explicit_change() {
        let directory = root();
        let first = reserve(directory.path(), None, false).unwrap();
        let served = first.listen;
        first.release();
        mark_applied(directory.path(), served).unwrap();
        assert_eq!(
            reserve(directory.path(), Some(served), true)
                .unwrap()
                .listen,
            served
        );

        let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let changed_port = blocker.local_addr().unwrap().port();
        drop(blocker);
        configure(
            directory.path(),
            GatewayListenerDesiredV1::fixed("127.0.0.1", changed_port),
        )
        .unwrap();
        assert_eq!(
            reserve(directory.path(), Some(served), true)
                .unwrap()
                .listen
                .port(),
            changed_port
        );
    }

    #[test]
    fn publication_does_not_migrate_the_old_or_missing_listener_record() {
        let directory = root();
        let served: SocketAddr = "127.0.0.1:45837".parse().unwrap();
        assert_eq!(
            reserve(directory.path(), Some(served), true).unwrap_err(),
            "GATEWAY_LISTENER_CONFIG_MISSING"
        );
        std::fs::write(
            directory.path().join("gateway-address.json"),
            br#"{"address":"127.0.0.1","port":45837}"#,
        )
        .unwrap();
        assert_eq!(
            reserve(directory.path(), Some(served), true).unwrap_err(),
            "GATEWAY_LISTENER_CONFIG_MISSING"
        );
    }

    #[test]
    fn activation_failure_preserves_old_applied_value_and_records_failure() {
        let directory = root();
        let first = reserve(directory.path(), None, false).unwrap();
        let served = first.listen;
        first.release();
        mark_applied(directory.path(), served).unwrap();
        configure(
            directory.path(),
            GatewayListenerDesiredV1::automatic("0.0.0.0"),
        )
        .unwrap();
        {
            let reservation = reserve(directory.path(), Some(served), true).unwrap();
            reservation.release();
            let _guard = ActivationGuard::new(directory.path());
        }
        let failed = status(directory.path()).unwrap();
        assert_eq!(failed.applied_address().unwrap(), Some(served));
        assert_eq!(
            failed.operation.unwrap().state,
            hiroute_host_runtime::GatewayListenerOperationStateV1::Failed
        );
    }
}
