//! Resume only the exact outstanding reply, then transfer ownership to a fresh
//! Desktop session. No daemon stop, admission pause, new pipe or business replay.
use super::*;

impl Resident {
    pub(crate) fn native_connection(&mut self) -> hiroute_product_session::NativeConnectionStatus {
        use hiroute_product_session::NativeConnectionStatus as Status;
        if self.has_authority() {
            return Status::Ready;
        }
        let Some(owned) = self.owned.as_mut() else {
            return Status::NotOwned;
        };
        if owned.shutdown.is_some()
            && owned.pending_ack.is_some()
            && matches!(owned.child.try_wait(), Ok(None))
        {
            Status::RecoveryRequired
        } else {
            Status::RestartRequired
        }
    }

    #[cfg(any(test, all(target_os = "macos", feature = "desktop-runtime")))]
    pub(crate) fn recover_authority(&mut self) -> Result<Self, String> {
        if self.has_authority() {
            return Err("AUTHORITY_RECOVERY_NOT_NEEDED".into());
        }
        let lock = self
            ._lock
            .try_clone()
            .map_err(|_| "HOST_LOCK_UNAVAILABLE")?;
        let owned = self.owned.as_mut().ok_or("AUTHORITY_RECOVERY_NOT_OWNED")?;
        if owned.shutdown.is_none() || !matches!(owned.child.try_wait(), Ok(None)) {
            return Err("AUTHORITY_RECOVERY_UNAVAILABLE".into());
        }
        owned.resume_ack().map_err(|error| {
            if error == "PROTECTED_CHANNEL_TIMEOUT" {
                "AUTHORITY_RECOVERY_PENDING"
            } else {
                "AUTHORITY_RECOVERY_UNAVAILABLE"
            }
            .to_owned()
        })?;
        // No old session may retain an abandoned or uncertain protected input.
        for candidate_ref in owned.manual_input_candidates.clone() {
            owned.release_input(&candidate_ref)?;
        }
        owned.authority_healthy = true;
        // Captured old NativeAuthority instances keep this empty Resident. They
        // cannot mint fresh grants even though the daemon and its PID are unchanged.
        Ok(Self {
            client: self.client.clone(),
            owned: self.owned.take(),
            _lock: lock,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_never_takes_authority_from_an_external_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut resident = Resident {
            client: Client::new(
                "hiroute-desktop",
                LocalEndpoint::from_runtime_root(root.path()),
            ),
            owned: None,
            _lock: acquire_host_lock(root.path()).unwrap(),
        };
        assert_eq!(
            resident.native_connection(),
            hiroute_product_session::NativeConnectionStatus::NotOwned
        );
        assert_eq!(
            resident.recover_authority().err().unwrap(),
            "AUTHORITY_RECOVERY_NOT_OWNED"
        );
    }

    #[test]
    fn an_unframed_or_closed_channel_has_no_in_place_recovery() {
        let root = tempfile::tempdir().unwrap();
        let mut resident = super::super::tests::recovery_resident(root.path(), false);
        resident.owned.as_mut().unwrap().authority_healthy = false;
        let pid = resident.owned.as_ref().unwrap().child.id();
        assert_eq!(
            resident.native_connection(),
            hiroute_product_session::NativeConnectionStatus::RestartRequired
        );
        assert_eq!(
            resident.recover_authority().err().unwrap(),
            "AUTHORITY_RECOVERY_UNAVAILABLE"
        );
        assert_eq!(resident.owned.as_ref().unwrap().child.id(), pid);
        assert!(
            resident
                .owned
                .as_mut()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert!(resident.owned.as_ref().unwrap().shutdown.is_some());
    }
}
