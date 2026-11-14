//! Stop-only recovery for managed logins and retired CPA artifacts.
use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) const PREVIOUS_CPA_VERSION: &str = "8.0.48-hiroute.1";

pub(super) fn is_previous_artifact(record: &OwnerRecord) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        record.binary_version == PREVIOUS_CPA_VERSION
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = record;
        false
    }
}

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
            let exit = if is_previous_artifact(&stale) {
                self.stop_previous_artifact(&layout, &stale)?
            } else {
                let artifact = self.locator.locate()?;
                ensure_supported_artifact(&artifact)?;
                if stale.binary_version != artifact.version().to_string()
                    || stale.binary_sha256_hex != artifact.sha256_hex()
                {
                    return Err(CpaLifecycleError::UntrustedOrphan);
                }
                let (_, mut process) = self.authenticate_orphan(&layout, &stale)?;
                match process.try_exit()? {
                    Some(exit) => exit,
                    None => process.shutdown(self.spec.shutdown_timeout)?,
                }
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
                    .restore_stale(&stale, self.spec.shutdown_timeout)
                    .map_err(|_| CpaLifecycleError::OwnerState)?;
                Err(error)
            }
        }
    }

    pub(super) fn authenticate_orphan(
        &self,
        layout: &InstanceLayout,
        record: &OwnerRecord,
    ) -> Result<(InstanceSecrets, Box<dyn CpaProcessHandle>), CpaLifecycleError> {
        if !record.address.ip().is_loopback() {
            return Err(CpaLifecycleError::UntrustedOrphan);
        }
        validate_runtime_files(layout)?;
        let secrets = InstanceSecrets::read(&layout.capability_path)?;
        let deadline = Instant::now() + self.spec.control_timeout;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if is_previous_artifact(record) {
            // Resolve the child in its private launch directory before sending
            // the management bearer to the listener. Keep that identity pinned.
            let mut process =
                crate::process::attach_recovered_process(self.backend.as_ref(), record.cpa_pid)?;
            if process.pid() != record.cpa_pid || !process.has_work_dir(&layout.work_dir)? {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            self.control
                .probe_predecessor(
                    record.address,
                    &secrets,
                    &record.binary_version,
                    record.cpa_pid,
                    deadline.saturating_duration_since(Instant::now()),
                )
                .map_err(|error| self.map_control_error(error))?;
            if process.try_exit()?.is_some() {
                return Err(CpaLifecycleError::UntrustedOrphan);
            }
            return Ok((secrets, process));
        }
        let probe = || {
            let timeout = deadline.saturating_duration_since(Instant::now());
            self.control
                .probe_ready(record.address, &secrets, &record.binary_version, timeout)
        };
        probe().map_err(|error| self.map_control_error(error))?;
        let process =
            crate::process::attach_recovered_process(self.backend.as_ref(), record.cpa_pid)?;
        probe().map_err(|error| self.map_control_error(error))?;
        if process.pid() != record.cpa_pid {
            return Err(CpaLifecycleError::UntrustedOrphan);
        }
        Ok((secrets, process))
    }

    pub(super) fn stop_previous_artifact(
        &self,
        layout: &InstanceLayout,
        record: &OwnerRecord,
    ) -> Result<CpaExit, CpaLifecycleError> {
        // The old signed bytes may have been moved or removed by the installer.
        // The private capability and the old endpoint's own version and PID bind
        // the live process for stopping. Only the current artifact can serve.
        let mut process = match self.authenticate_orphan(layout, record) {
            Ok((_, process)) => process,
            // The predecessor may have exited while the capability was being checked.
            Err(_) if !self.backend.pid_is_running(record.cpa_pid)? => {
                return Ok(CpaExit { code: None });
            }
            Err(error) => return Err(error),
        };
        crate::request_context::cleanup(self.spec.shutdown_timeout, || {
            let exit = match process.try_exit()? {
                Some(exit) => exit,
                None => process.shutdown(self.spec.shutdown_timeout)?,
            };
            if process.try_exit()?.is_none() {
                return Err(crate::process::CpaProcessError::ShutdownTimeout.into());
            }
            Ok(exit)
        })
    }

    pub(super) fn retire_previous_before_start(
        &self,
        layout: &InstanceLayout,
        stale: &OwnerRecord,
        lease: &OwnerLease,
        fresh: &OwnerRecord,
    ) -> Result<(), CpaLifecycleError> {
        let result = (|| {
            // Hold the auth directory without creating a new lease or writing new
            // capabilities while the predecessor can still access its credentials.
            let _auth = ManagedAuthLease::acquire_expected(&layout.auth_dir, None, None)?;
            self.stop_previous_artifact(layout, stale)?;
            lease
                .write(fresh)
                .map_err(|_| CpaLifecycleError::OwnerState)
        })();
        if result.is_err() {
            lease
                .restore_stale(stale, self.spec.shutdown_timeout)
                .map_err(|_| CpaLifecycleError::OwnerState)?;
        }
        result
    }
}
