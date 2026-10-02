//! Bounded upgrade phases share the already inherited daemon readiness pipe.
use super::*;
use hiroute_host_runtime::StorageUpgradePhase;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    schema: String,
    phase: StorageUpgradePhase,
}
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(super) fn read_ready(
    reader: &mut impl Read,
    cancelled: &AtomicBool,
    diagnostics: &NativeDiagnostics,
) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let mut budget = DAEMON_READY_TIMEOUT;
    let mut previous = None;
    loop {
        let remaining = budget
            .checked_sub(started.elapsed())
            .ok_or("PROTECTED_CHANNEL_TIMEOUT")?;
        let bytes = read_frame(reader, 4096, remaining, Some(cancelled))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "DAEMON_READY_INVALID")?;
        if value["schema"] != "hiroute.daemon-upgrade-progress/v1" {
            return Ok(bytes);
        }
        let progress: Progress =
            serde_json::from_value(value).map_err(|_| "DAEMON_READY_INVALID")?;
        let index = match progress.phase {
            StorageUpgradePhase::SourceCheck => 0,
            StorageUpgradePhase::Backup => 1,
            StorageUpgradePhase::Conversion => 2,
            StorageUpgradePhase::Validation => 3,
            StorageUpgradePhase::ServiceRecovery => 4,
        };
        if progress.schema != "hiroute.daemon-upgrade-progress/v1"
            || previous.is_some_and(|last| index <= last)
        {
            return Err("DAEMON_READY_INVALID".into());
        }
        // At most five strictly advancing phases. The overall deadline never resets, so a
        // child cannot keep the host waiting indefinitely by repeating progress frames.
        previous = Some(index);
        budget = UPGRADE_TIMEOUT;
        diagnostics.report_upgrade(progress.phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_progress_is_bounded_and_ready_is_still_the_final_authority() {
        let state = std::sync::Arc::new(std::sync::Mutex::new(None));
        let diagnostics = NativeDiagnostics::disabled().with_upgrade_progress(state.clone());
        let mut pipe = std::io::Cursor::new(concat!(
            "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"source_check\"}\n",
            "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"backup\"}\n",
            "{\"schema\":\"hiroute.daemon-ready/v1\"}\n"
        ));
        let ready = read_ready(&mut pipe, &AtomicBool::new(false), &diagnostics).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&ready).unwrap()["schema"],
            "hiroute.daemon-ready/v1"
        );
        assert_eq!(*state.lock().unwrap(), Some(StorageUpgradePhase::Backup));
        for invalid in [
            concat!(
                "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"backup\"}\n",
                "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"backup\"}\n"
            ),
            "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"unknown\"}\n",
            "{\"schema\":\"hiroute.daemon-upgrade-progress/v1\",\"phase\":\"backup\",\"path\":\"untrusted\"}\n",
        ] {
            assert_eq!(
                read_ready(
                    &mut std::io::Cursor::new(invalid),
                    &AtomicBool::new(false),
                    &diagnostics
                )
                .unwrap_err(),
                "DAEMON_READY_INVALID"
            );
        }
    }
}
