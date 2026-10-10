//! Stop-only recovery for disabled and unfinished independent logins.
//! This path cannot spawn CPA or start its refresh/discovery workers.
use super::*;

impl ManagedCpaRuntime {
    /// Stops an existing managed OAuth process without ever launching a replacement.
    /// Missing ownership is a no-op; ambiguous ownership preserves the private store.
    pub fn stop_existing_managed_process(&self) -> Result<(), CpaLifecycleError> {
        let kind = self
            .spec
            .managed_oauth
            .ok_or(CpaLifecycleError::InvalidSpec)?;
        self.suspend_subscription_execution();
        let mut inner = self.lock_lifecycle()?;
        if let Some(live) = inner.live.as_mut() {
            let exit = self.shutdown_live_process(live)?;
            live.lease
                .release()
                .map_err(|_| CpaLifecycleError::OwnerState)?;
            inner.live.take();
            inner.last_exit = Some(exit);
            inner.oauth_state = None;
            inner.oauth_callback_submitted = true;
            self.epochs.advance_runtime();
            return Ok(());
        }

        let lock_dir = self
            .spec
            .state_root
            .join(&self.spec.instance_id)
            .join("owner.lock");
        match std::fs::symlink_metadata(&lock_dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            _ => return Err(CpaLifecycleError::OwnerState),
        }
        let stale =
            crate::owner::read_record(&lock_dir).map_err(|_| CpaLifecycleError::OwnerState)?;
        if self.backend.pid_is_running(stale.owner_pid)? {
            return Err(CpaLifecycleError::AlreadyOwned);
        }
        // A crash between spawn and recording its PID leaves no child identity to
        // authenticate. Do not infer that the private store is safe to remove.
        if stale.cpa_pid == 0 {
            return Err(CpaLifecycleError::OwnerState);
        }
        let nonce = SecretText::generate()?.expose().to_owned();
        let transferred = stale
            .clone()
            .transfer(std::process::id(), nonce.clone())
            .map_err(|_| CpaLifecycleError::OwnerState)?;
        let lease = OwnerLease::reclaim_stale(&lock_dir, &stale, &transferred, nonce)
            .map_err(|_| CpaLifecycleError::OwnerState)?;
        let result = (|| {
            let layout = self.prepare_layout()?;
            // Only the lock is acquired. A stopped or half-written credential is
            // neither read nor refreshed by this recovery operation.
            let _auth_lease = ManagedAuthLease::acquire_subscription(
                &layout.auth_dir,
                None,
                None,
                Some(kind),
                None,
            )?;
            if !self.backend.pid_is_running(stale.cpa_pid)? {
                lease.release().map_err(|_| CpaLifecycleError::OwnerState)?;
                return Ok(None);
            }
            let artifact = self.locator.locate()?;
            ensure_supported_artifact(&artifact)?;
            if stale.binary_version != artifact.version().to_string()
                || stale.binary_sha256_hex != artifact.sha256_hex()
                || !stale.address.ip().is_loopback()
            {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            let secrets = InstanceSecrets::read(&layout.capability_path)?;
            validate_runtime_files(&layout)?;
            self.control
                .probe_ready(
                    stale.address,
                    &secrets,
                    &stale.binary_version,
                    self.spec.control_timeout,
                )
                .map_err(|error| self.map_control_error(error))?;
            let mut process = self.backend.attach_authenticated(stale.cpa_pid)?;
            self.control
                .probe_ready(
                    stale.address,
                    &secrets,
                    &stale.binary_version,
                    self.spec.control_timeout,
                )
                .map_err(|error| self.map_control_error(error))?;
            if process.pid() != stale.cpa_pid {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            let exit = match process.try_exit()? {
                Some(exit) => exit,
                None => process.shutdown(self.spec.shutdown_timeout)?,
            };
            // Proxy-policy differences deliberately do not enter start_fresh: only
            // an explicitly enabled saved source may later start a new process.
            lease.release().map_err(|_| CpaLifecycleError::OwnerState)?;
            Ok(Some(exit))
        })();
        match result {
            Ok(exit) => {
                inner.last_exit = exit;
                inner.oauth_state = None;
                inner.oauth_callback_submitted = true;
                self.epochs.advance_runtime();
                Ok(())
            }
            Err(error) => {
                lease
                    .restore_stale(&stale)
                    .map_err(|_| CpaLifecycleError::OwnerState)?;
                Err(error)
            }
        }
    }
}
