//! Recovery uses distinct inherited maintenance pipes, never the uncertain grant/ACK stream.
use super::*;
use hiroute_diagnostics::event::{ProtectedExchangeKind, ProtectedExchangePhase};
use hiroute_host_runtime::{LauncherUpgradeRequest, LauncherUpgradeStatus, UpgradeAction};

pub(super) struct MaintenanceChannel {
    request: File,
    response: File,
    healthy: bool,
}

impl MaintenanceChannel {
    pub(super) fn new(request: File, response: File) -> Result<Self, String> {
        nonblocking(&request)?;
        nonblocking(&response)?;
        Ok(Self {
            request,
            response,
            healthy: true,
        })
    }

    fn status(
        &mut self,
        action: UpgradeAction,
        diagnostics: Option<&DiagnosticsPort>,
    ) -> Result<LauncherUpgradeStatus, String> {
        if !self.healthy {
            return Err("AUTHORITY_RECOVERY_UNAVAILABLE".into());
        }
        let id = crate::random_id()?;
        let mut bytes = serde_json::to_vec(&LauncherUpgradeRequest {
            schema: "hiroute.launcher-upgrade/v1".into(),
            registration_id: id.clone(),
            action,
        })
        .map_err(|_| "UPGRADE_FRAME_INVALID")?;
        bytes.push(b'\n');
        let started = Instant::now();
        let mut phase = ProtectedExchangePhase::RequestWrite;
        let result = (|| {
            write_frame(&mut self.request, &bytes)?;
            phase = ProtectedExchangePhase::AcknowledgementRead;
            let bytes = read_frame(&mut self.response, 4096, Duration::from_secs(10), None)?;
            let status: LauncherUpgradeStatus =
                serde_json::from_slice(&bytes).map_err(|_| "UPGRADE_ACK_INVALID")?;
            if status.schema != "hiroute.launcher-upgrade-status/v1" || status.registration_id != id
            {
                return Err("REGISTRATION_ACK_MISMATCH".into());
            }
            Ok(status)
        })();
        if result.is_err() {
            self.healthy = false;
        }
        channel::emit_exchange(
            diagnostics,
            ProtectedExchangeKind::Recovery,
            phase,
            &id,
            started,
            result.as_ref().err(),
            result.is_err(),
        );
        result
    }
}

impl Resident {
    pub(crate) fn native_connection(&mut self) -> hiroute_product_session::NativeConnectionStatus {
        use hiroute_product_session::NativeConnectionStatus as Status;
        if self.has_authority() {
            return Status::Ready;
        }
        let Some(owned) = self.owned.as_mut() else {
            return Status::NotOwned;
        };
        if matches!(owned.child.try_wait(), Ok(Some(_)))
            || owned.shutdown.is_none()
            || owned
                .maintenance
                .as_ref()
                .is_some_and(|channel| channel.healthy)
        {
            Status::RecoveryRequired
        } else {
            Status::RestartRequired
        }
    }

    #[cfg(any(test, all(target_os = "macos", feature = "desktop-runtime")))]
    pub(crate) fn recover_authority(
        &mut self,
        root: &Path,
        daemon: &Path,
        diagnostics: &NativeDiagnostics,
    ) -> Result<Self, String> {
        if self.has_authority() {
            return Err("AUTHORITY_RECOVERY_NOT_NEEDED".into());
        }
        let owned = self.owned.as_mut().ok_or("AUTHORITY_RECOVERY_NOT_OWNED")?;
        let exited = owned
            .child
            .try_wait()
            .map_err(|_| "UPGRADE_STOP_FAILED")?
            .is_some();
        if !exited && owned.shutdown.is_some() {
            let maintenance = owned
                .maintenance
                .as_mut()
                .ok_or("AUTHORITY_RECOVERY_UNAVAILABLE")?;
            let status = maintenance.status(UpgradeAction::Prepare, owned.diagnostics.as_ref())?;
            if !status.drained() {
                // Never stop live work, and restore ordinary admission before returning.
                maintenance.status(UpgradeAction::Cancel, owned.diagnostics.as_ref())?;
                return Err("AUTHORITY_RECOVERY_WORK_ACTIVE".into());
            }
        }
        drop(owned.shutdown.take());
        let deadline = Instant::now() + Duration::from_secs(45);
        while owned
            .child
            .try_wait()
            .map_err(|_| "UPGRADE_STOP_FAILED")?
            .is_none()
        {
            if Instant::now() >= deadline {
                return Err("UPGRADE_STOP_PENDING".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // The old Resident remains permanently unusable for any captured old-session callback.
        // The new ProductSession receives a distinct NativeAuthority and fresh descriptors.
        self.replace_stopped(|lock| {
            Self::open_resident(
                root,
                daemon,
                &AtomicBool::new(false),
                diagnostics,
                Some(lock),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintenance_requires_its_own_exact_ack_and_cannot_reuse_a_failed_channel() {
        let (mut requests, request_writer) = pipe().unwrap();
        let (reply_reader, mut replies) = pipe().unwrap();
        nonblocking(&requests).unwrap();
        let mut channel = MaintenanceChannel::new(request_writer, reply_reader).unwrap();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let request = read_frame(&mut requests, 4096, Duration::from_secs(2), None).unwrap();
                let request: LauncherUpgradeRequest = serde_json::from_slice(&request).unwrap();
                let reply = format!("{{\"schema\":\"hiroute.launcher-upgrade-status/v1\",\"registration_id\":\"conflict\",\"paused\":true,\"active_calls\":0,\"active_tasks\":0}}\n");
                write_frame(&mut replies, reply.as_bytes()).unwrap();
                assert!(!request.registration_id.is_empty());
                assert_eq!(read_frame(&mut requests, 4096, Duration::from_millis(100), None).unwrap_err(), "PROTECTED_CHANNEL_TIMEOUT");
            });
            assert!(channel.status(UpgradeAction::Prepare, None).is_err());
            assert_eq!(
                channel.status(UpgradeAction::Cancel, None).err().as_deref(),
                Some("AUTHORITY_RECOVERY_UNAVAILABLE")
            );
        });
    }
}
