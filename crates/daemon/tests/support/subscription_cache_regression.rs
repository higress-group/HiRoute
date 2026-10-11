//! Production Local Control regressions; the CPA fixture never calls a provider.
use super::*;
use hiroute_application_api::ComputeCandidateFactStateV2;

fn cache_fixture() -> (tempfile::TempDir, TcpListener, ProductDaemon) {
    let directory = tempfile::tempdir().unwrap();
    // tempfile inherits umask for directories. A group-writable ancestor makes the
    // production diagnostics writer reject this otherwise isolated fixture root.
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    configure_product_root(directory.path());
    let (binary, sha256) = install_subscription_fixture(directory.path());
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let daemon = ProductDaemon::start_with_cpa(
        directory.path(),
        proxy.local_addr().unwrap(),
        &binary,
        &sha256,
    );
    (directory, proxy, daemon)
}

async fn check_current(
    daemon: &mut ProductDaemon,
    label: &str,
) -> ComputeSubscriptionCheckResultV2 {
    let listed = succeeded(
        daemon
            .client
            .compute_subscriptions(&format!("{label}-list"))
            .await
            .unwrap(),
    );
    assert_eq!(listed.candidates.len(), 1);
    let pending = &listed.candidates[0];
    assert_eq!(
        pending.fact_state,
        ComputeCandidateFactStateV2::PendingApproval
    );
    assert!(pending.validation.is_none());
    // Repeated metadata listing must not create a new candidate by itself.
    let relisted = succeeded(
        daemon
            .client
            .compute_subscriptions(&format!("{label}-relist"))
            .await
            .unwrap(),
    );
    assert_eq!(relisted.candidates[0].candidate, pending.candidate);
    let preview = succeeded(
        daemon
            .client
            .preview_subscription_check(&format!("{label}-preview"), pending.candidate.clone())
            .await
            .unwrap(),
    );
    let grant = daemon.register(
        APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
        preview.accept_digest.clone(),
        preview.expected_revisions.clone(),
    );
    let applied = daemon
        .client
        .apply_subscription_check(
            &format!("{label}-apply"),
            ComputeConnectionApplyRequestV1 {
                spec: preview.spec,
                accept_digest: preview.accept_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: format!("{label}-check"),
            },
            grant,
        )
        .await
        .unwrap();
    assert_eq!(applied.status, MachineStatus::Accepted, "{applied:?}");
    let operation = applied.data.unwrap();
    assert_eq!(operation.state, "succeeded");
    let checked = succeeded(
        daemon
            .client
            .subscription_check_result(&format!("{label}-result"), operation.operation_id)
            .await
            .unwrap(),
    );
    assert_eq!(checked.status, ComputeSubscriptionCheckStatusV2::Verified);
    let relisted = succeeded(
        daemon
            .client
            .compute_subscriptions(&format!("{label}-checked-list"))
            .await
            .unwrap(),
    );
    assert_eq!(
        relisted.candidates[0].candidate,
        checked.checked_candidate.as_ref().unwrap().candidate
    );
    assert_eq!(relisted.candidates[0].validation, checked.validation);
    checked
}

async fn management_snapshot(daemon: &ProductDaemon, label: &str) -> ComputeManagementSnapshotV2 {
    succeeded(
        daemon
            .client
            .compute_management_snapshot(label, ComputeManagementQueryV2::default())
            .await
            .unwrap(),
    )
}

async fn save_checked(
    daemon: &ProductDaemon,
    checked: &ComputeSubscriptionCheckResultV2,
    intent: ComputeManagementIntentV2,
    label: &str,
) -> ComputeManagementSnapshotV2 {
    let current = management_snapshot(daemon, &format!("{label}-before")).await;
    let candidate = checked.checked_candidate.as_ref().unwrap();
    let preview = succeeded(
        daemon
            .client
            .preview_compute_save(
                &format!("{label}-preview"),
                ComputeManagementChangeV2 {
                    schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                    subject: ComputeManagementSubjectV2::Candidate {
                        candidate: candidate.candidate.clone(),
                    },
                    expected_revisions: current.revisions,
                    selected_model_refs: vec![candidate.models[0].model_ref.clone()],
                    intent,
                    key_edits: Vec::new(),
                    validation: checked.validation.clone(),
                },
            )
            .await
            .unwrap(),
    );
    let applied = daemon
        .client
        .apply_compute_save(&format!("{label}-apply"), apply_request(&preview, label))
        .await
        .unwrap();
    assert_eq!(applied.status, MachineStatus::Accepted, "{applied:?}");
    assert_eq!(applied.data.unwrap().state, "succeeded");
    management_snapshot(daemon, &format!("{label}-after")).await
}

fn retain_failed_saved_edit_diagnostics(root: &Path) -> std::io::Result<PathBuf> {
    use hiroute_diagnostics::DiagnosticEvent;
    use hiroute_diagnostics::publication::PublicationStage;
    use hiroute_diagnostics::record::DiagnosticRecordV1;

    let logs = root.join("storage/diagnostics/daemon");
    // The writer is asynchronous. Wait only for its already-emitted admission event;
    // this never resubmits the failed product action.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let failed_admission = fs::read(logs.join("current.jsonl"))
            .unwrap_or_default()
            .split(|byte| *byte == b'\n')
            .filter_map(|line| DiagnosticRecordV1::parse_line(line).ok())
            .any(|record| {
                matches!(record.event, DiagnosticEvent::PublicationTiming(timing)
                    if timing.stage == PublicationStage::Admission && !timing.ok)
            });
        if failed_admission || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let evidence = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/product-e2e-evidence")
        .join(format!(
            "subscription-cache-retained-edit-{}",
            std::process::id()
        ));
    fs::create_dir_all(&evidence)?;
    fs::set_permissions(&evidence, fs::Permissions::from_mode(0o700))?;
    let mut files = Vec::new();
    let mut current_boot = None;
    let mut level_applied = None;
    let mut failed_admission = false;
    let mut paths = fs::read_dir(logs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            continue;
        }
        let bytes = fs::read(&path)?;
        let mut safe_jsonl = Vec::new();
        let mut records = 0;
        let mut bad_lines = 0;
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let Ok(record) = DiagnosticRecordV1::parse_line(line) else {
                bad_lines += 1;
                continue;
            };
            if path.file_name().is_some_and(|name| name == "current.jsonl") {
                current_boot = Some(record.boot_id);
                if let DiagnosticEvent::LevelApplied(level) = &record.event {
                    level_applied = Some(serde_json::to_value(level).unwrap());
                }
                if let DiagnosticEvent::PublicationTiming(timing) = &record.event {
                    failed_admission |= timing.stage == PublicationStage::Admission && !timing.ok;
                }
            }
            // Re-encode the closed product schema. No test root, configuration, raw
            // process output or unvalidated line is copied into retained evidence.
            safe_jsonl.extend(record.encode_jsonl().unwrap());
            records += 1;
        }
        let destination = evidence.join(path.file_name().unwrap());
        fs::write(&destination, safe_jsonl)?;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600))?;
        files.push(
            serde_json::json!({"file": destination, "records": records, "bad_lines": bad_lines}),
        );
    }
    let report = serde_json::json!({
        "schema": "hiroute.subscription-cache-diagnostic/v1",
        "scenario": "unconsumed_check_expires_when_same_saved_source_revision_changes",
        "scenario_result": "red",
        "failed_request": "revision-disable-apply",
        "evidence_directory": evidence,
        "current_boot": current_boot,
        "level_applied": level_applied,
        "failed_admission_observed": failed_admission,
        "port_context": "not_emitted_by_product_diagnostics",
        "files": files,
    });
    let report_path = evidence.join("result.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap())?;
    fs::set_permissions(&report_path, fs::Permissions::from_mode(0o600))?;
    Ok(report_path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checked_candidate_cache_supports_ready_disabled_ready_without_restart() {
    let (_directory, _proxy, mut daemon) = cache_fixture();
    let mut previous_source: Option<hiroute_application_api::ComputeManagedSourceViewV2> = None;
    let mut previous_checked_revision = 0;
    for (round, intent, state) in [
        (
            "initial-ready",
            ComputeManagementIntentV2::SaveReady,
            MaterializationState::Ready,
        ),
        (
            "second-disabled",
            ComputeManagementIntentV2::SaveDisabled,
            MaterializationState::Disabled,
        ),
        (
            "third-ready",
            ComputeManagementIntentV2::SaveReady,
            MaterializationState::Ready,
        ),
    ] {
        let checked = check_current(&mut daemon, round).await;
        let candidate = checked.checked_candidate.as_ref().unwrap();
        assert!(candidate.candidate.candidate_revision > previous_checked_revision);
        previous_checked_revision = candidate.candidate.candidate_revision;
        let snapshot = save_checked(&daemon, &checked, intent, round).await;
        assert_eq!(snapshot.sources.len(), 1);
        let source = &snapshot.sources[0];
        assert_eq!(source.state, state);
        if let Some(previous) = &previous_source {
            assert_eq!(source.source_id, previous.source_id);
            assert_eq!(source.revision, previous.revision + 1);
            assert_eq!(source.models[0].model_ref, previous.models[0].model_ref);
            assert_eq!(source.models[0].binding_id, previous.models[0].binding_id);
        }
        previous_source = Some(source.clone());
        let relisted = succeeded(
            daemon
                .client
                .compute_subscriptions(&format!("{round}-consumed-list"))
                .await
                .unwrap(),
        );
        let pending = &relisted.candidates[0];
        assert_eq!(
            pending.fact_state,
            ComputeCandidateFactStateV2::PendingApproval
        );
        assert!(pending.models.is_empty());
        assert!(pending.validation.is_none());
        assert_eq!(
            pending.existing_source_id.as_deref(),
            Some(source.source_id.as_str())
        );
        assert_eq!(
            pending.candidate.candidate_revision,
            previous_checked_revision + 1
        );
    }
    daemon.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unconsumed_check_expires_when_same_saved_source_revision_changes() {
    let (directory, _proxy, mut daemon) = cache_fixture();
    let initial = check_current(&mut daemon, "revision-initial").await;
    let snapshot = save_checked(
        &daemon,
        &initial,
        ComputeManagementIntentV2::SaveReady,
        "revision-save",
    )
    .await;
    let source = &snapshot.sources[0];
    let checked = check_current(&mut daemon, "revision-unsaved").await;
    assert!(checked.save_operation.is_none());
    let before_edit = management_snapshot(&daemon, "revision-before-independent-edit").await;
    assert_eq!(before_edit.sources[0].source_id, source.source_id);
    assert_eq!(before_edit.sources[0].revision, source.revision);

    // A separate legal edit uses the retained saved validation, leaving the new check
    // unconsumed. The check advanced global revisions, so edit against the fresh snapshot.
    // The source ID and native evidence stay unchanged, but its revision moves.
    let preview = succeeded(
        daemon
            .client
            .preview_compute_save(
                "revision-disable-preview",
                ComputeManagementChangeV2 {
                    schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                    subject: ComputeManagementSubjectV2::SavedSource {
                        source_id: source.source_id.clone(),
                    },
                    expected_revisions: before_edit.revisions,
                    selected_model_refs: source
                        .models
                        .iter()
                        .map(|model| model.model_ref.clone())
                        .collect(),
                    intent: ComputeManagementIntentV2::SaveDisabled,
                    key_edits: Vec::new(),
                    validation: initial.validation,
                },
            )
            .await
            .unwrap(),
    );
    let disabled = daemon
        .client
        .apply_compute_save(
            "revision-disable-apply",
            apply_request(&preview, "revision-disable"),
        )
        .await
        .unwrap();
    if disabled.status != MachineStatus::Accepted {
        match retain_failed_saved_edit_diagnostics(directory.path()) {
            Ok(path) => eprintln!("safe_product_diagnostics={}", path.display()),
            Err(error) => eprintln!(
                "safe_product_diagnostics=unavailable kind={:?}",
                error.kind()
            ),
        }
    }
    assert_eq!(disabled.status, MachineStatus::Accepted, "{disabled:?}");
    assert_eq!(disabled.data.unwrap().state, "succeeded");
    let current = management_snapshot(&daemon, "revision-current").await;
    assert_eq!(current.sources[0].source_id, source.source_id);
    assert_eq!(current.sources[0].revision, source.revision + 1);

    // The save boundary must reject the stale receipt even before a list refreshes cache.
    let candidate = checked.checked_candidate.as_ref().unwrap();
    let stale_save = daemon
        .client
        .preview_compute_save(
            "revision-stale-save",
            ComputeManagementChangeV2 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: candidate.candidate.clone(),
                },
                expected_revisions: current.revisions,
                selected_model_refs: vec![candidate.models[0].model_ref.clone()],
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: checked.validation.clone(),
            },
        )
        .await
        .unwrap();
    assert_error(&stale_save, ErrorCode::RevisionConflict);
    let status = succeeded(
        daemon
            .client
            .subscription_check_result(
                "revision-stale-status",
                checked.approval_operation.operation_id,
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        status.status,
        ComputeSubscriptionCheckStatusV2::SourceChanged
    );
    let fresh = check_current(&mut daemon, "revision-fresh").await;
    assert!(
        fresh
            .checked_candidate
            .as_ref()
            .unwrap()
            .candidate
            .candidate_revision
            > candidate.candidate.candidate_revision
    );
    daemon.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_checked_candidate_does_not_hide_same_source_lineage_replacement() {
    let (directory, _proxy, mut daemon) = cache_fixture();
    let initial = check_current(&mut daemon, "lineage-initial").await;
    let saved = save_checked(
        &daemon,
        &initial,
        ComputeManagementIntentV2::SaveReady,
        "lineage-save",
    )
    .await;
    let checked = check_current(&mut daemon, "lineage-unsaved").await;
    let source_id = &saved.sources[0].source_id;
    // Fault-inject a changed durable association while retaining ID/revision. There is
    // intentionally no public operation that rewrites a saved lineage in place.
    let mut source = saved_source_json(directory.path(), source_id);
    let lineage = hiroute_domain::CanonicalDigest::of_bytes(b"changed-cache-lineage");
    source["lineage_digest"] = serde_json::to_value(&lineage).unwrap();
    let connection = Connection::open(directory.path().join("storage/live/control.db")).unwrap();
    connection.execute(
        "UPDATE compute_management_sources SET lineage_digest=?1, source_json=?2 WHERE source_id=?3",
        params![lineage.as_str(), serde_json::to_string(&source).unwrap(), source_id],
    ).unwrap();
    drop(connection);
    let listed = succeeded(
        daemon
            .client
            .compute_subscriptions("lineage-after-replacement")
            .await
            .unwrap(),
    );
    let candidate = &listed.candidates[0];
    assert_eq!(
        candidate.fact_state,
        ComputeCandidateFactStateV2::PendingApproval
    );
    assert!(candidate.validation.is_none());
    assert!(
        candidate.candidate.candidate_revision
            > checked
                .checked_candidate
                .unwrap()
                .candidate
                .candidate_revision
    );
    daemon.stop();
}
