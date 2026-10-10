//! Real-process lifecycle regressions with synthetic external OAuth credentials.
#![cfg(unix)]
use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hiroute_integrations::TrustedReleaseCatalog;
use sha2::{Digest, Sha256};

use crate::{
    CpaHealth, CpaProfileBinding, CpaRegisteredSourcePort, CpaRuntimeSpec,
    MANAGED_CPA_ARTIFACT_VERSION, ManagedCpaRuntimeSet, PinnedCpaArtifact, PinnedCpaBinaryLocator,
    RestartPolicy,
};

#[path = "selection_tests.rs"]
mod selection_tests;

struct Fixture {
    root: tempfile::TempDir,
    templates: Vec<Arc<ManagedCpaRuntime>>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = include_str!("cpa_oauth_fixture.py")
            .replace("__HIR_CPA_VERSION__", MANAGED_CPA_ARTIFACT_VERSION);
        let binary = root.path().join("cpa-fixture");
        fs::write(&binary, source.as_bytes()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let locator = Arc::new(PinnedCpaBinaryLocator::new(PinnedCpaArtifact::new(
            root.path(),
            "cpa-fixture",
            MANAGED_CPA_ARTIFACT_VERSION.parse().unwrap(),
            format!("{:x}", Sha256::digest(source.as_bytes())),
        )));
        const MANIFEST: &[u8] =
            include_bytes!("../../../../assets/release-facts/current/bundle/manifest.json");
        let catalog = Arc::new(
            TrustedReleaseCatalog::load_bundled_release_facts(
                MANIFEST,
                MANIFEST,
                include_bytes!(
                    "../../../../assets/release-facts/current/bundle/connector-registry.json"
                ),
                include_bytes!("../../../../assets/release-facts/current/bundle/model-data.json"),
            )
            .unwrap(),
        );
        let templates = [CpaAccountKind::Codex, CpaAccountKind::Claude]
            .into_iter()
            .map(|kind| {
                let provider = kind.stock_provider();
                Arc::new(
                    ManagedCpaRuntime::new(
                        CpaRuntimeSpec {
                            instance_id: format!("template-{provider}"),
                            state_root: root.path().join("cpa/state"),
                            auth_dir: root.path().join(format!("cpa/template-{provider}-auth")),
                            borrowed_codex_auth: None,
                            borrowed_claude_auth: None,
                            managed_oauth: Some(kind),
                            bindings: vec![CpaProfileBinding {
                                account_kind: kind,
                                connector_id: format!("connector.cpa.{provider}"),
                                connection_option_id: format!("{provider}.subscription.global.v1"),
                                endpoint_profile_id: format!("endpoint.cpa.{provider}"),
                            }],
                            startup_timeout: Duration::from_secs(3),
                            control_timeout: Duration::from_secs(1),
                            shutdown_timeout: Duration::from_secs(1),
                            restart_policy: RestartPolicy::default(),
                        },
                        catalog.clone(),
                        locator.clone(),
                    )
                    .unwrap(),
                )
            })
            .collect();
        Self { root, templates }
    }

    fn registry(&self) -> ManagedLoginRegistry {
        ManagedLoginRegistry::load(self.root.path().join("cpa/managed-logins"), &self.templates)
            .unwrap()
    }

    fn spawns(&self) -> usize {
        fs::read_to_string(self.root.path().join("spawns.jsonl"))
            .map(|text| text.lines().count())
            .unwrap_or_default()
    }

    fn session_dir(&self, id: &str) -> PathBuf {
        self.root.path().join("cpa/managed-logins").join(id)
    }

    fn callback_value(&self, session: &CpaLoginSession, url: &str) -> String {
        let url = reqwest::Url::parse(url).unwrap();
        let state = url.query_pairs().find(|(key, _)| key == "state").unwrap().1;
        match session.kind {
            CpaAccountKind::Codex => format!(
                "http://localhost:1455/auth/callback?state={state}&code=fixture-authorization-code"
            ),
            CpaAccountKind::Claude => format!("fixture-authorization-code#{state}"),
        }
    }

    fn callback(&self, registry: &ManagedLoginRegistry, session: &CpaLoginSession, url: &str) {
        let value = self.callback_value(session, url);
        registry.callback(&session.login_ref, &value).unwrap();
    }

    fn authorize(&self, registry: &ManagedLoginRegistry, kind: CpaAccountKind) -> CpaLoginSession {
        let (session, url) = registry.start(&self.templates, kind).unwrap();
        self.callback(registry, &session, &url);
        let session = registry.status(&session.login_ref).unwrap();
        assert_eq!(session.state, CpaLoginState::Authorized);
        session
    }
}

#[test]
fn callback_version_mismatch_fails_closed_and_never_resubmits_the_code() {
    let fixture = Fixture::new();
    fs::write(fixture.root.path().join("callback-version-mismatch"), b"").unwrap();
    let registry = fixture.registry();
    let (session, url) = registry
        .start(&fixture.templates, CpaAccountKind::Claude)
        .unwrap();
    let input = fixture.callback_value(&session, &url);
    assert!(matches!(
        registry.callback(&session.login_ref, &input),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
    assert!(matches!(
        registry.callback(&session.login_ref, &input),
        Err(CpaLifecycleError::InvalidOAuthSession)
    ));
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("callbacks.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1,
        "uncertain callback acknowledgement resent a one-use authorization code"
    );
    registry.cancel(&session.login_ref, false).unwrap();
}

#[test]
fn missing_management_version_is_rejected_for_auth_url_and_status() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let omit_url = fixture.root.path().join("missing-auth-url-version");
    fs::write(&omit_url, b"").unwrap();
    assert!(matches!(
        registry.start(&fixture.templates, CpaAccountKind::Codex),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
    fs::remove_file(omit_url).unwrap();
    let (session, _) = registry
        .start(&fixture.templates, CpaAccountKind::Codex)
        .unwrap();
    let omit_status = fixture.root.path().join("missing-status-version");
    fs::write(&omit_status, b"").unwrap();
    assert!(matches!(
        registry.status(&session.login_ref),
        Err(CpaLifecycleError::UnsafeControlResponse)
    ));
    fs::remove_file(omit_status).unwrap();
    registry.cancel(&session.login_ref, false).unwrap();
}

fn runtime(registry: &ManagedLoginRegistry, id: &str) -> Arc<ManagedCpaRuntime> {
    registry.sessions.lock().get(id).unwrap().runtime.clone()
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "owned fixture did not reach its bounded state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn kill_owned_pending(runtime: &ManagedCpaRuntime) {
    let pid = match runtime.health().unwrap() {
        CpaHealth::Ready { pid, .. } | CpaHealth::Unhealthy { pid, .. } => pid,
        _ => panic!("pending fixture must be running before crash injection"),
    };
    // Pending login deliberately has suspended execution. Its observation can
    // be unhealthy while the separately owned OAuth process is still alive.
    assert_ne!(pid, 0);
    assert!(
        Command::new("/bin/kill")
            .args(["-0", "--", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success(),
        "owned pending fixture was not alive before crash injection"
    );
    assert!(
        Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    wait_until(|| matches!(runtime.health(), Ok(CpaHealth::Crashed { .. })));
}

#[test]
fn authorized_random_login_refs_reload_without_starting_or_granting_routing() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let mut refs = Vec::new();
    for kind in [CpaAccountKind::Codex, CpaAccountKind::Claude] {
        let session = fixture.authorize(&registry, kind);
        assert!(valid_login_ref(&session.login_ref));
        assert_eq!(session.login_ref.len(), 49);
        let managed = registry.for_candidate(&session.candidate_ref()).unwrap();
        assert!(matches!(managed.health(), Ok(CpaHealth::Stopped { .. })));
        let path = fixture.session_dir(&session.login_ref).join("login.json");
        let manifest = fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&manifest).contains("fixture-managed-"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        refs.push(session);
    }
    assert_ne!(refs[0].login_ref, refs[1].login_ref);
    let before = fixture.spawns();
    drop(registry);
    let reopened = fixture.registry();
    for expected in refs {
        let actual = reopened.session(&expected.login_ref).unwrap();
        assert_eq!(actual.account_ref, expected.account_ref);
        assert_eq!(actual.candidate_ref(), expected.candidate_ref());
        assert!(matches!(
            runtime(&reopened, &actual.login_ref).health(),
            Ok(CpaHealth::Stopped { .. })
        ));
    }
    let set = ManagedCpaRuntimeSet::new(fixture.templates.clone()).unwrap();
    assert!(set.discover_registered_sources().unwrap().is_empty());
    assert_eq!(
        fixture.spawns(),
        before,
        "unsaved authorization recovery must not start a refresh writer"
    );
}

#[test]
fn cancel_stops_late_exchange_before_deleting_its_exact_store() {
    let fixture = Fixture::new();
    fs::write(fixture.root.path().join("delay-write"), "0.35").unwrap();
    let registry = fixture.registry();
    let (session, url) = registry
        .start(&fixture.templates, CpaAccountKind::Codex)
        .unwrap();
    fixture.callback(&registry, &session, &url);
    let directory = fixture.session_dir(&session.login_ref);
    wait_until(|| directory.join("exchange-started").exists());
    assert_eq!(
        registry.cancel(&session.login_ref, false).unwrap().state,
        CpaLoginState::Cancelled
    );
    std::thread::sleep(Duration::from_millis(400));
    assert!(!directory.join("auth").exists());
    assert!(
        !directory.join("exchange-finished").exists(),
        "cancelled child wrote a credential late"
    );
    assert!(matches!(
        registry.callback(&session.login_ref, "fixture-authorization-code"),
        Err(CpaLifecycleError::StaleSourceManagement)
    ));
    assert!(registry.for_candidate(&session.candidate_ref()).is_none());
    let (next, _) = registry
        .start(&fixture.templates, CpaAccountKind::Codex)
        .unwrap();
    assert_ne!(next.login_ref, session.login_ref);
    registry.cancel(&next.login_ref, false).unwrap();
}

#[test]
fn crashed_pending_status_callback_cancel_never_spawn_a_replacement() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let (session, url) = registry
        .start(&fixture.templates, CpaAccountKind::Codex)
        .unwrap();
    kill_owned_pending(&runtime(&registry, &session.login_ref));
    let before = fixture.spawns();
    assert!(registry.status(&session.login_ref).is_err());
    let state = reqwest::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    assert!(registry.callback(&session.login_ref,
        &format!("http://localhost:1455/auth/callback?state={state}&code=fixture-authorization-code")).is_err());
    assert_eq!(
        registry.cancel(&session.login_ref, false).unwrap().state,
        CpaLoginState::Cancelled
    );
    assert_eq!(
        fixture.spawns(),
        before,
        "an expired or cancelled pending login has no restart authority"
    );
}

#[test]
fn failed_stop_preserves_credentials_and_manifest_until_exact_cleanup_succeeds() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let session = fixture.authorize(&registry, CpaAccountKind::Codex);
    let managed = runtime(&registry, &session.login_ref);
    managed.start().unwrap();
    let lock = managed
        .spec
        .state_root
        .join(&managed.spec.instance_id)
        .join("owner.lock");
    let blocker = lock.join("fixture-blocks-release");
    fs::write(&blocker, b"owned stop failure injection").unwrap();
    let directory = fixture.session_dir(&session.login_ref);
    let auth_before = fs::read(directory.join("auth/credential.json")).unwrap();
    let manifest_before = fs::read(directory.join("login.json")).unwrap();
    assert!(registry.cancel(&session.login_ref, true).is_err());
    assert_eq!(
        registry.session(&session.login_ref).unwrap().state,
        CpaLoginState::Authorized
    );
    assert_eq!(
        fs::read(directory.join("auth/credential.json")).unwrap(),
        auth_before
    );
    assert_eq!(
        fs::read(directory.join("login.json")).unwrap(),
        manifest_before
    );
    fs::remove_file(blocker).unwrap();
    assert_eq!(
        registry.cancel(&session.login_ref, true).unwrap().state,
        CpaLoginState::Forgotten
    );
    assert!(!directory.join("auth").exists());
    assert!(registry.for_candidate(&session.candidate_ref()).is_none());
}

#[test]
fn damaged_manifest_does_not_poison_a_healthy_provider_or_delete_evidence() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let damaged = fixture.authorize(&registry, CpaAccountKind::Codex);
    let healthy = fixture.authorize(&registry, CpaAccountKind::Claude);
    let manifest = fixture.session_dir(&damaged.login_ref).join("login.json");
    private_atomic_write(&manifest, b"{damaged fixture manifest").unwrap();
    let before = fixture.spawns();
    drop(registry);
    let reopened = fixture.registry();
    assert!(reopened.session(&damaged.login_ref).is_none());
    assert!(reopened.for_candidate(&damaged.candidate_ref()).is_none());
    assert_eq!(
        reopened.session(&healthy.login_ref).unwrap().state,
        CpaLoginState::Authorized
    );
    assert_eq!(fs::read(manifest).unwrap(), b"{damaged fixture manifest");
    assert!(
        fixture
            .session_dir(&damaged.login_ref)
            .join("auth/credential.json")
            .is_file()
    );
    assert_eq!(fixture.spawns(), before);
}

#[test]
fn partial_write_failure_recovers_same_binding_without_mutating_login_manifest() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let session = fixture.authorize(&registry, CpaAccountKind::Codex);
    let managed = runtime(&registry, &session.login_ref);
    let directory = fixture.session_dir(&session.login_ref);
    let token_file = directory.join("auth/credential.json");
    let before = managed.managed_credentials().unwrap();
    let manifest = fs::read(directory.join("login.json")).unwrap();
    let mut tokens: serde_json::Value =
        serde_json::from_slice(&fs::read(&token_file).unwrap()).unwrap();
    fs::write(&token_file, b"{").unwrap();
    assert!(managed.managed_credentials().is_err());
    tokens["access_token"] = "fixture-rotated-access".into();
    tokens["refresh_token"] = "fixture-rotated-refresh".into();
    private_atomic_write(&token_file, &serde_json::to_vec(&tokens).unwrap()).unwrap();
    assert_eq!(managed.managed_credentials().unwrap(), before);
    assert_eq!(fs::read(directory.join("login.json")).unwrap(), manifest);
    assert_eq!(
        registry.session(&session.login_ref).unwrap().account_ref,
        session.account_ref
    );
    assert_eq!(fixture.spawns(), 1);
}

#[test]
fn housekeeping_finishes_disconnected_callback_and_expires_idle_pending_login() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let (session, url) = registry
        .start(&fixture.templates, CpaAccountKind::Claude)
        .unwrap();
    fixture.callback(&registry, &session, &url);
    registry.maintain().unwrap();
    assert_eq!(
        registry.session(&session.login_ref).unwrap().state,
        CpaLoginState::Authorized
    );
    assert!(matches!(
        runtime(&registry, &session.login_ref).health(),
        Ok(CpaHealth::Stopped { .. })
    ));
    let (pending, _) = registry
        .start(&fixture.templates, CpaAccountKind::Claude)
        .unwrap();
    {
        let mut sessions = registry.sessions.lock();
        let record = &mut sessions.get_mut(&pending.login_ref).unwrap().record;
        record.created_at = now().unwrap() - 2;
        record.expires_at = now().unwrap() - 1;
        registry.save(record).unwrap();
    }
    let before = fixture.spawns();
    registry.maintain().unwrap();
    assert_eq!(
        registry.session(&pending.login_ref).unwrap().state,
        CpaLoginState::Expired
    );
    assert!(
        !fixture
            .session_dir(&pending.login_ref)
            .join("auth")
            .exists()
    );
    assert_eq!(fixture.spawns(), before);
}

#[test]
fn housekeeping_failure_does_not_block_an_independent_provider_cleanup() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let (broken, _) = registry
        .start(&fixture.templates, CpaAccountKind::Codex)
        .unwrap();
    let (healthy, url) = registry
        .start(&fixture.templates, CpaAccountKind::Claude)
        .unwrap();
    fixture.callback(&registry, &healthy, &url);
    kill_owned_pending(&runtime(&registry, &broken.login_ref));
    let before = fixture.spawns();
    assert!(registry.maintain().is_err());
    assert_eq!(
        registry.session(&healthy.login_ref).unwrap().state,
        CpaLoginState::Authorized
    );
    assert!(matches!(
        runtime(&registry, &healthy.login_ref).health(),
        Ok(CpaHealth::Stopped { .. })
    ));
    assert_eq!(fixture.spawns(), before);
    registry.cancel(&broken.login_ref, false).unwrap();
}
