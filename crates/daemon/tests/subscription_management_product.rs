#![cfg(all(unix, feature = "integration-test-hooks"))]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hiroute_application_api::{
    APPLY_SUBSCRIPTION_CHECK_OPERATION_V2, ApplyResultV1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2,
    ComputeConnectionAccessKindV1, ComputeConnectionApplyRequestV1, ComputeManagementChangeV2,
    ComputeManagementIntentV2, ComputeManagementQueryV2, ComputeManagementRuntimeReadStateV2,
    ComputeManagementSnapshotV2, ComputeManagementSubjectV2, ComputeModelAvailabilityReasonV1,
    ComputeModelAvailabilityV1, ComputeModelMembershipV2, ComputeSaveDispositionV2,
    ComputeSavePreviewV2, ComputeSaveResultV2, ComputeSubscriptionCheckResultV2,
    ComputeSubscriptionCheckStatusV2, ErrorCode, MachineEnvelopeV2, MachineStatus,
};
use hiroute_domain::{
    BillingClass, ComputeManagementRepositoryPort, MaterializationState, WorkspaceId,
};
use hiroute_local_storage::LocalStorageSet;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

#[path = "support/discovered_model_product_support.rs"]
mod product_support;
use product_support::{ProductDaemon, configure_product_root};

const ACCESS_SENTINEL: &str = "subscription-product-access-token-must-stay-protected";
const FAKE_CPA: &str = r#"#!/usr/bin/python3
import http.server
import json
import os
import re
import sys
import urllib.parse

VERSION = "8.0.4-hiroute.2"
MANAGED_NAME = "hiroute-managed-codex.json"
MODEL_ID = "gpt-5.3-codex-spark"
ADDED_MODEL_ID = "gpt-5.5"

if "--help" in sys.argv:
    print("CLIProxyAPI Version: " + VERSION)
    raise SystemExit(0)

config_path = sys.argv[sys.argv.index("--config") + 1]
text = open(config_path, "r", encoding="utf-8").read()
assert "--local-password-stdin" in sys.argv
assert len(sys.stdin.read()) >= 32

def scalar(name):
    match = re.search(r"^" + re.escape(name) + r":\s*['\"]?([^'\"\n]+)", text, re.M)
    if not match:
        raise RuntimeError("missing config value " + name)
    return match.group(1).strip()

port = int(scalar("port"))
auth_dir = scalar("auth-dir")

def managed():
    with open(os.path.join(auth_dir, MANAGED_NAME), "r", encoding="utf-8") as stream:
        return json.load(stream)

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def reply(self, value, management=False):
        body = json.dumps(value, separators=(",", ":")).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        if management:
            self.send_header("x-cpa-version", "v" + VERSION)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/healthz":
            self.reply({"status": "ok"})
            return
        if parsed.path == "/v0/management/auth-files":
            name = urllib.parse.parse_qs(parsed.query).get("name", [""])[0]
            if name != MANAGED_NAME:
                self.reply({"files": []}, True)
                return
            current = managed()
            self.reply({"files": [{
                "id": MANAGED_NAME,
                "auth_index": current.get("account_id", ""),
                "name": MANAGED_NAME,
                "provider": "codex",
                "status": "active",
                "disabled": False,
                "unavailable": False,
                "runtime_only": False,
                "source": "file",
                "account_type": "oauth",
                "request_retry": current.get("request_retry", 0)
            }]}, True)
            return
        if parsed.path == "/v0/management/auth-files/models":
            current = managed()
            prefix = current.get("prefix", "")
            revision = current.get("last_refresh", "")
            models = [] if revision == "rights-restricted" else [MODEL_ID]
            if revision in ["token-rotated", "rights-restricted", "rights-restored"]:
                models.append(ADDED_MODEL_ID)
            self.reply({"models": [{"id": prefix + "/" + model} for model in models]}, True)
            return
        if parsed.path == "/v1/models":
            current = managed()
            prefix = current.get("prefix", "")
            revision = current.get("last_refresh", "")
            models = [] if revision == "rights-restricted" else [MODEL_ID]
            if revision in ["token-rotated", "rights-restricted", "rights-restored"]:
                models.append(ADDED_MODEL_ID)
            self.reply({"data": [{"id": prefix + "/" + model} for model in models]})
            return
        self.send_error(404)

    def do_PATCH(self):
        if self.path != "/v0/management/auth-files/fields":
            self.send_error(404)
            return
        length = int(self.headers.get("content-length", "0"))
        update = json.loads(self.rfile.read(length))
        current = managed()
        current["prefix"] = update["prefix"]
        current["request_retry"] = update["request_retry"]
        current["disable_cooling"] = update["disable_cooling"]
        path = os.path.join(auth_dir, MANAGED_NAME)
        temporary = path + ".fake-cpa-update"
        with open(temporary, "w", encoding="utf-8") as stream:
            json.dump(current, stream, separators=(",", ":"))
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
        self.reply({}, True)

    def log_message(self, _format, *_args):
        pass

http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#;

fn succeeded<T: std::fmt::Debug>(envelope: MachineEnvelopeV2<T>) -> T {
    assert_eq!(envelope.status, MachineStatus::Succeeded, "{envelope:?}");
    assert!(envelope.error.is_none(), "{envelope:?}");
    envelope.data.expect("successful response has data")
}

fn assert_error<T: std::fmt::Debug>(envelope: &MachineEnvelopeV2<T>, code: ErrorCode) {
    assert_eq!(envelope.status, code.status(), "{envelope:?}");
    assert_eq!(
        envelope.error.as_ref().map(|error| error.code),
        Some(code),
        "{envelope:?}"
    );
    assert!(envelope.data.is_none(), "{envelope:?}");
}

fn install_subscription_fixture(root: &Path) -> (PathBuf, String) {
    let auth = root.join("home/.codex/auth.json");
    fs::create_dir_all(auth.parent().unwrap()).unwrap();
    fs::set_permissions(auth.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    write_subscription_auth(&auth, "subscription-product-account", "fixture");
    let binary = root.join("cpa-artifact/cliproxyapi");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, FAKE_CPA.as_bytes()).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(FAKE_CPA.as_bytes()));
    (binary, sha256)
}

fn write_subscription_auth(path: &Path, account_id: &str, revision: &str) {
    fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "OPENAI_API_KEY": null,
            "auth_mode": "chatgpt",
            "last_refresh": revision,
            "tokens": {
                "access_token": format!("{ACCESS_SENTINEL}-{revision}"),
                "id_token": format!("fixture.id.{revision}"),
                "refresh_token": "fixture-refresh-never-materialized",
                "account_id": account_id
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn saved_source_json(root: &Path, source_id: &str) -> serde_json::Value {
    let connection = Connection::open(root.join("storage/live/control.db")).unwrap();
    let encoded: String = connection
        .query_row(
            "SELECT source_json FROM compute_management_sources WHERE source_id=?1",
            [source_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&encoded).unwrap()
}

fn apply_request(preview: &ComputeSavePreviewV2, key: &str) -> ComputeConnectionApplyRequestV1 {
    ComputeConnectionApplyRequestV1 {
        spec: preview.spec.clone(),
        accept_digest: preview.accept_digest.clone(),
        expected_revisions: preview.expected_revisions.clone(),
        idempotency_key: key.into(),
    }
}

fn assert_subscription_snapshot(
    snapshot: &ComputeManagementSnapshotV2,
    expected_availability: ComputeModelAvailabilityV1,
    expected_reason: Option<ComputeModelAvailabilityReasonV1>,
) {
    assert_eq!(
        snapshot.runtime_state,
        ComputeManagementRuntimeReadStateV2::Complete
    );
    assert_eq!(snapshot.sources.len(), 1);
    let source = &snapshot.sources[0];
    assert_eq!(source.state, MaterializationState::Ready);
    assert_eq!(
        source.connection_identity.access_kind,
        ComputeConnectionAccessKindV1::Subscription
    );
    assert_eq!(
        source.connection_identity.connection_option_id.as_deref(),
        Some("codex.subscription.global.v1")
    );
    assert_eq!(
        source.connection_identity.product_label.as_deref(),
        Some("OpenAI · Codex subscription")
    );
    assert!(source.keys.is_empty());
    assert_eq!(source.models.len(), 1);
    let model = &source.models[0];
    assert_eq!(model.upstream_model_id, "gpt-5.3-codex-spark");
    assert_eq!(
        model.catalog_configuration_id.as_deref(),
        Some("model.openai.gpt-5.3-codex-spark")
    );
    assert_eq!(model.membership, ComputeModelMembershipV2::Observed);
    assert_eq!(model.presentation.billing_class, BillingClass::Subscription);
    assert!(model.presentation.price_contexts.is_empty());
    assert!(model.presentation.evaluated_at_ms > 0);
    assert_eq!(model.presentation.availability, expected_availability);
    assert_eq!(model.presentation.reason_code, expected_reason);
    assert_eq!(
        source.ready_model_count,
        u32::from(expected_availability == ComputeModelAvailabilityV1::Available)
    );
    let public = serde_json::to_string(snapshot).unwrap();
    assert!(!public.contains(ACCESS_SENTINEL));
    assert!(!public.contains("account/cpa/"));
}

fn corrupt_saved_display_name(root: &Path, source_id: &str) {
    let connection = Connection::open(root.join("storage/live/control.db")).unwrap();
    let encoded: String = connection
        .query_row(
            "SELECT source_json FROM compute_management_sources WHERE source_id=?1",
            [source_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut source: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    source["display_name"] = serde_json::Value::String("mismatched saved source".into());
    connection
        .execute(
            "UPDATE compute_management_sources SET source_json=?1 WHERE source_id=?2",
            params![serde_json::to_string(&source).unwrap(), source_id],
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hirouted_client_core_subscription_save_snapshot_and_restart_are_closed() {
    let directory = tempfile::tempdir().unwrap();
    configure_product_root(directory.path());
    let (binary, sha256) = install_subscription_fixture(directory.path());
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_address = proxy.local_addr().unwrap();
    let mut daemon =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);

    let candidates = succeeded(
        daemon
            .client
            .compute_subscriptions("subscription-candidates")
            .await
            .unwrap(),
    );
    assert_eq!(candidates.candidates.len(), 1);
    let pending = candidates.candidates[0].candidate.clone();
    assert!(candidates.candidates[0].models.is_empty());
    assert_eq!(candidates.candidates[0].existing_source_id, None);
    let check_preview = succeeded(
        daemon
            .client
            .preview_subscription_check("subscription-preview", pending)
            .await
            .unwrap(),
    );
    let check_grant = daemon.register(
        APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
        check_preview.accept_digest.clone(),
        check_preview.expected_revisions.clone(),
    );
    let checked_apply = daemon
        .client
        .apply_subscription_check(
            "subscription-apply",
            ComputeConnectionApplyRequestV1 {
                spec: check_preview.spec,
                accept_digest: check_preview.accept_digest,
                expected_revisions: check_preview.expected_revisions,
                idempotency_key: "subscription-product-check".into(),
            },
            check_grant,
        )
        .await
        .unwrap();
    assert_eq!(
        checked_apply.status,
        MachineStatus::Accepted,
        "{checked_apply:?}"
    );
    let checked_apply: ApplyResultV1 = checked_apply.data.clone().unwrap();
    assert_eq!(checked_apply.state, "succeeded");
    let checked: ComputeSubscriptionCheckResultV2 = succeeded(
        daemon
            .client
            .subscription_check_result("subscription-result", checked_apply.operation_id.clone())
            .await
            .unwrap(),
    );
    assert_eq!(checked.status, ComputeSubscriptionCheckStatusV2::Verified);
    let validation = checked.validation.clone().unwrap();
    let checked_candidate = checked.checked_candidate.clone().unwrap();
    assert_eq!(checked_candidate.models.len(), 1);
    assert!(checked_candidate.models[0].selectable);
    assert_eq!(
        checked_candidate.models[0].upstream_model_id,
        "gpt-5.3-codex-spark"
    );

    // A checked but unsaved candidate keeps serving its checked revision with no association.
    let rescanned_before_save = succeeded(
        daemon
            .client
            .compute_subscriptions("subscription-rescan-before-save")
            .await
            .unwrap(),
    );
    assert_eq!(rescanned_before_save.candidates.len(), 1);
    assert_eq!(rescanned_before_save.candidates[0].existing_source_id, None);
    assert_eq!(
        rescanned_before_save.candidates[0].candidate,
        checked_candidate.candidate
    );

    let before_save = succeeded(
        daemon
            .client
            .compute_management_snapshot(
                "verified-before-save",
                ComputeManagementQueryV2::default(),
            )
            .await
            .unwrap(),
    );
    assert!(before_save.sources.is_empty());
    let save_preview = succeeded(
        daemon
            .client
            .preview_compute_save(
                "subscription-save-preview",
                ComputeManagementChangeV2 {
                    schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                    subject: ComputeManagementSubjectV2::Candidate {
                        candidate: checked_candidate.candidate.clone(),
                    },
                    expected_revisions: before_save.revisions,
                    selected_model_refs: vec![checked_candidate.models[0].model_ref.clone()],
                    intent: ComputeManagementIntentV2::SaveReady,
                    key_edits: Vec::new(),
                    validation: Some(validation.clone()),
                },
            )
            .await
            .unwrap(),
    );
    let saved_apply = daemon
        .client
        .apply_compute_save(
            "subscription-save-apply",
            apply_request(&save_preview, "subscription-product-save"),
        )
        .await
        .unwrap();
    assert_eq!(
        saved_apply.status,
        MachineStatus::Accepted,
        "{saved_apply:?}"
    );
    let saved_apply: ApplyResultV1 = saved_apply.data.clone().unwrap();
    assert_eq!(saved_apply.state, "succeeded");
    let save_operation = saved_apply.operation_id.clone();
    let saved_result: ComputeSaveResultV2 = succeeded(
        daemon
            .client
            .compute_save_result("subscription-save-result", save_operation)
            .await
            .unwrap(),
    );
    assert_eq!(saved_result.disposition, ComputeSaveDispositionV2::Saved);
    let source_id = saved_result.source_id.unwrap();
    assert!(source_id.starts_with("source/managed-"));

    // The same daemon session must reflect the saved association without a restart: the candidate
    // advances to a new pending revision that carries the saved source and no reusable validation.
    let rescanned_after_save = succeeded(
        daemon
            .client
            .compute_subscriptions("subscription-rescan-after-save")
            .await
            .unwrap(),
    );
    assert_eq!(rescanned_after_save.candidates.len(), 1);
    let rescanned_candidate = rescanned_after_save.candidates[0].clone();
    assert_eq!(
        rescanned_candidate.existing_source_id.as_deref(),
        Some(source_id.as_str())
    );
    assert!(rescanned_candidate.models.is_empty());
    assert!(rescanned_candidate.validation.is_none());
    assert_eq!(
        rescanned_candidate.candidate.candidate_revision,
        checked_candidate.candidate.candidate_revision + 1
    );

    let saved_snapshot = succeeded(
        daemon
            .client
            .compute_management_snapshot(
                "subscription-saved-snapshot",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(&saved_snapshot, ComputeModelAvailabilityV1::Available, None);

    // The consumed save validation cannot be replayed for another save.
    let reused = daemon
        .client
        .preview_compute_save(
            "subscription-save-reuse",
            ComputeManagementChangeV2 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: checked_candidate.candidate.clone(),
                },
                expected_revisions: saved_snapshot.revisions.clone(),
                selected_model_refs: vec![checked_candidate.models[0].model_ref.clone()],
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: Some(validation),
            },
        )
        .await
        .unwrap();
    assert_error(&reused, ErrorCode::RevisionConflict);

    // Re-authorization re-runs the protected check on the rebuilt pending revision and keeps the
    // saved association in the resulting checked candidate.
    let recheck_preview = succeeded(
        daemon
            .client
            .preview_subscription_check(
                "subscription-reauthorize-preview",
                rescanned_candidate.candidate.clone(),
            )
            .await
            .unwrap(),
    );
    let recheck_grant = daemon.register(
        APPLY_SUBSCRIPTION_CHECK_OPERATION_V2,
        recheck_preview.accept_digest.clone(),
        recheck_preview.expected_revisions.clone(),
    );
    let rechecked_apply = daemon
        .client
        .apply_subscription_check(
            "subscription-reauthorize-apply",
            ComputeConnectionApplyRequestV1 {
                spec: recheck_preview.spec,
                accept_digest: recheck_preview.accept_digest,
                expected_revisions: recheck_preview.expected_revisions,
                idempotency_key: "subscription-product-reauthorize".into(),
            },
            recheck_grant,
        )
        .await
        .unwrap();
    assert_eq!(
        rechecked_apply.status,
        MachineStatus::Accepted,
        "{rechecked_apply:?}"
    );
    let rechecked_apply: ApplyResultV1 = rechecked_apply.data.clone().unwrap();
    assert_eq!(rechecked_apply.state, "succeeded");
    let rechecked: ComputeSubscriptionCheckResultV2 = succeeded(
        daemon
            .client
            .subscription_check_result(
                "subscription-reauthorize-result",
                rechecked_apply.operation_id.clone(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(rechecked.status, ComputeSubscriptionCheckStatusV2::Verified);
    let rechecked_candidate = rechecked.checked_candidate.clone().unwrap();
    assert_eq!(
        rechecked_candidate.existing_source_id.as_deref(),
        Some(source_id.as_str())
    );
    assert_eq!(
        rechecked_candidate.candidate.candidate_revision,
        rescanned_candidate.candidate.candidate_revision + 1
    );
    let rescanned_after_recheck = succeeded(
        daemon
            .client
            .compute_subscriptions("subscription-rescan-after-recheck")
            .await
            .unwrap(),
    );
    assert_eq!(
        rescanned_after_recheck.candidates[0].candidate,
        rechecked_candidate.candidate
    );
    assert_eq!(
        rescanned_after_recheck.candidates[0]
            .existing_source_id
            .as_deref(),
        Some(source_id.as_str())
    );

    daemon.stop();

    let stores =
        LocalStorageSet::open_for_daemon_startup(directory.path().join("storage")).unwrap();
    assert!(
        stores
            .control()
            .compute_projection_rows()
            .unwrap()
            .is_empty(),
        "the saved management source must still be unpublished"
    );
    assert_eq!(
        stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .sources
            .len(),
        1
    );
    drop(stores);

    let restarted =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);
    let restarted_snapshot = succeeded(
        restarted
            .client
            .compute_management_snapshot(
                "subscription-restarted-snapshot",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(
        &restarted_snapshot,
        ComputeModelAvailabilityV1::Available,
        None,
    );

    // The restarted daemon serves the same candidate identity and association as the session that
    // saved, so the association never depended on a restart to become visible.
    let rescanned_after_restart = succeeded(
        restarted
            .client
            .compute_subscriptions("subscription-rescan-after-restart")
            .await
            .unwrap(),
    );
    assert_eq!(rescanned_after_restart.candidates.len(), 1);
    assert_eq!(
        rescanned_after_restart.candidates[0].candidate,
        rechecked_candidate.candidate
    );
    assert_eq!(
        rescanned_after_restart.candidates[0]
            .existing_source_id
            .as_deref(),
        Some(source_id.as_str())
    );
    assert!(!rescanned_after_restart.candidates[0].models.is_empty());

    // A same-account token rotation is owned entirely by CPA. The subscription source, audit
    // identity, selected model, binding and authored references remain byte-for-byte stable; a
    // newly visible catalog model is not added implicitly.
    let auth_path = directory.path().join("home/.codex/auth.json");
    let before_rotation = restarted_snapshot.sources[0].clone();
    let before_durable = saved_source_json(directory.path(), &source_id);
    write_subscription_auth(&auth_path, "subscription-product-account", "token-rotated");
    let rotated_snapshot = succeeded(
        restarted
            .client
            .compute_management_snapshot(
                "subscription-token-rotated",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(
        &rotated_snapshot,
        ComputeModelAvailabilityV1::Available,
        None,
    );
    let rotated_source = rotated_snapshot.sources[0].clone();
    assert_eq!(rotated_source.source_id, before_rotation.source_id);
    assert_eq!(rotated_source.revision, before_rotation.revision);
    assert_eq!(rotated_source.models.len(), 1);
    assert_eq!(
        rotated_source.models[0].model_ref,
        before_rotation.models[0].model_ref
    );
    assert_eq!(
        rotated_source.models[0].binding_id,
        before_rotation.models[0].binding_id
    );
    assert_eq!(
        rotated_source.models[0].upstream_model_id,
        "gpt-5.3-codex-spark"
    );
    let after_durable = saved_source_json(directory.path(), &source_id);
    assert_eq!(after_durable, before_durable);
    assert!(
        before_durable["provenance"]
            .get("authorization_generation")
            .is_none()
    );
    assert!(
        after_durable["provenance"]
            .get("authorization_generation")
            .is_none()
    );

    // A later rights change for the same account keeps the exact saved member and binding but
    // removes execution eligibility. A newly visible model is still not added. Restoring rights
    // makes the retained member executable again without route edits.
    write_subscription_auth(
        &auth_path,
        "subscription-product-account",
        "rights-restricted",
    );
    let restricted_deadline = Instant::now() + Duration::from_secs(10);
    let restricted = loop {
        let snapshot = succeeded(
            restarted
                .client
                .compute_management_snapshot(
                    "subscription-rights-restricted",
                    ComputeManagementQueryV2 {
                        source_id: Some(source_id.clone()),
                    },
                )
                .await
                .unwrap(),
        );
        if snapshot.sources[0].models[0].presentation.reason_code
            == Some(ComputeModelAvailabilityReasonV1::ModelNotAllowed)
        {
            break snapshot.sources[0].clone();
        }
        assert!(
            Instant::now() < restricted_deadline,
            "subscription rights restriction did not converge: {snapshot:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(restricted.source_id, rotated_source.source_id);
    assert_eq!(restricted.revision, rotated_source.revision);
    assert_eq!(restricted.models.len(), 1);
    assert_eq!(
        restricted.models[0].model_ref,
        rotated_source.models[0].model_ref
    );
    assert_eq!(
        restricted.models[0].binding_id,
        rotated_source.models[0].binding_id
    );
    assert_eq!(
        restricted.models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::ModelNotAllowed)
    );
    assert_eq!(restricted.ready_model_count, 0);

    write_subscription_auth(
        &auth_path,
        "subscription-product-account",
        "rights-restored",
    );
    let rights_restored_deadline = Instant::now() + Duration::from_secs(10);
    let rights_restored = loop {
        let snapshot = succeeded(
            restarted
                .client
                .compute_management_snapshot(
                    "subscription-rights-restored",
                    ComputeManagementQueryV2 {
                        source_id: Some(source_id.clone()),
                    },
                )
                .await
                .unwrap(),
        );
        if snapshot.sources[0].models[0].presentation.availability
            == ComputeModelAvailabilityV1::Available
        {
            break snapshot.sources[0].clone();
        }
        assert!(
            Instant::now() < rights_restored_deadline,
            "subscription rights restoration did not converge: {snapshot:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(rights_restored.source_id, rotated_source.source_id);
    assert_eq!(rights_restored.revision, rotated_source.revision);
    assert_eq!(rights_restored.models.len(), 1);
    assert_eq!(
        rights_restored.models[0].binding_id,
        rotated_source.models[0].binding_id
    );

    // Losing the native auth source while the daemon is live immediately moves the subscription
    // out of the executable state. Restoring the stable native context reopens it without a source
    // write or another user save.
    let live_missing_auth_path = directory.path().join("home/.codex/auth.live-missing");
    fs::rename(&auth_path, &live_missing_auth_path).unwrap();
    let missing_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = succeeded(
            restarted
                .client
                .compute_management_snapshot(
                    "subscription-live-auth-missing",
                    ComputeManagementQueryV2 {
                        source_id: Some(source_id.clone()),
                    },
                )
                .await
                .unwrap(),
        );
        if snapshot.sources[0].models[0].presentation.reason_code
            == Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired)
        {
            assert_eq!(snapshot.sources[0].revision, rights_restored.revision);
            break;
        }
        assert!(
            Instant::now() < missing_deadline,
            "live auth removal did not close subscription admission: {snapshot:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    fs::rename(&live_missing_auth_path, &auth_path).unwrap();
    let restored_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = succeeded(
            restarted
                .client
                .compute_management_snapshot(
                    "subscription-live-auth-restored",
                    ComputeManagementQueryV2 {
                        source_id: Some(source_id.clone()),
                    },
                )
                .await
                .unwrap(),
        );
        if snapshot.sources[0].models[0].presentation.availability
            == ComputeModelAvailabilityV1::Available
        {
            assert_eq!(snapshot.sources[0].revision, rights_restored.revision);
            break;
        }
        assert!(
            Instant::now() < restored_deadline,
            "restored committed auth did not reopen subscription admission: {snapshot:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    restarted.stop();

    fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap();
    let runtime_unavailable =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);
    let runtime_snapshot = succeeded(
        runtime_unavailable
            .client
            .compute_management_snapshot(
                "subscription-runtime-unavailable",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(
        &runtime_snapshot,
        ComputeModelAvailabilityV1::Unavailable,
        Some(ComputeModelAvailabilityReasonV1::RuntimeUnavailable),
    );
    runtime_unavailable.stop();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();

    let auth_path = directory.path().join("home/.codex/auth.json");
    let valid_auth = fs::read(&auth_path).unwrap();
    fs::write(&auth_path, br#"{"auth_mode":"chatgpt"}"#).unwrap();
    fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600)).unwrap();
    let invalid_auth =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);
    let invalid_auth_snapshot = succeeded(
        invalid_auth
            .client
            .compute_management_snapshot(
                "subscription-invalid-auth",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(
        &invalid_auth_snapshot,
        ComputeModelAvailabilityV1::NeedsCredentials,
        Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired),
    );
    invalid_auth.stop();

    fs::write(&auth_path, valid_auth).unwrap();
    fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600)).unwrap();
    let unavailable_auth_path = directory.path().join("home/.codex/auth.unavailable");
    fs::rename(&auth_path, &unavailable_auth_path).unwrap();
    let needs_auth =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);
    let auth_snapshot = succeeded(
        needs_auth
            .client
            .compute_management_snapshot(
                "subscription-needs-auth",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id.clone()),
                },
            )
            .await
            .unwrap(),
    );
    assert_subscription_snapshot(
        &auth_snapshot,
        ComputeModelAvailabilityV1::NeedsCredentials,
        Some(ComputeModelAvailabilityReasonV1::AuthenticationRequired),
    );
    needs_auth.stop();

    fs::rename(unavailable_auth_path, auth_path).unwrap();
    corrupt_saved_display_name(directory.path(), &source_id);
    let mismatched =
        ProductDaemon::start_with_cpa(directory.path(), proxy_address, &binary, &sha256);
    let mismatch_snapshot = succeeded(
        mismatched
            .client
            .compute_management_snapshot(
                "subscription-retained-source-mismatch",
                ComputeManagementQueryV2 {
                    source_id: Some(source_id),
                },
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        mismatch_snapshot.runtime_state,
        ComputeManagementRuntimeReadStateV2::Partial
    );
    assert_eq!(mismatch_snapshot.sources.len(), 1);
    let mismatched_source = &mismatch_snapshot.sources[0];
    assert_eq!(
        mismatched_source.connection_identity.access_kind,
        ComputeConnectionAccessKindV1::Unknown
    );
    assert!(
        mismatched_source
            .connection_identity
            .connection_option_id
            .is_none()
    );
    assert!(
        mismatched_source
            .connection_identity
            .product_label
            .is_none()
    );
    assert_eq!(mismatched_source.ready_model_count, 0);
    assert_eq!(
        mismatched_source.models[0].presentation.billing_class,
        BillingClass::Unknown
    );
    assert_eq!(
        mismatched_source.models[0].presentation.availability,
        ComputeModelAvailabilityV1::Unknown
    );
    assert_eq!(
        mismatched_source.models[0].presentation.reason_code,
        Some(ComputeModelAvailabilityReasonV1::FactsUnavailable)
    );
    mismatched.stop();

    assert!(matches!(
        proxy.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}
