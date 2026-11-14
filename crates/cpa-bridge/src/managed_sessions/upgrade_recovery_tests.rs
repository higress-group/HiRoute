use super::*;
use crate::artifact::VerifiedCpaBinary;
use crate::config::{InstanceSecrets, SecretText, render_managed_config};
use crate::owner::{OwnerClaim, OwnerLease, OwnerRecord};
use crate::runtime::recovery::PREVIOUS_CPA_VERSION;
use crate::{CpaLaunch, CpaProcessBackend, StdCpaProcessBackend};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::process::ExitStatusExt as _;
use std::process::Child;

const CRASH_OWNER_CHILD: &str = "HIROUTE_CPA_UPGRADE_CRASH_OWNER";
const DURABLE_SELECTION: &str = "upgrade-selection.json";
const PREVIOUS_BINARY: &str = "previous-cpa";

#[derive(Serialize, Deserialize)]
struct SavedSubscription {
    login: String,
    account: String,
    credential_id: String,
    address: SocketAddr,
    pid: u32,
}

#[test]
fn managed_subscription_recovers_after_crash_with_previous_cpa_running() {
    if let Some(root) = std::env::var_os(CRASH_OWNER_CHILD) {
        prepare_crash(Path::new(&root));
        return;
    }

    let fixture = Fixture::new();
    let mut owner = CrashOwner::start(fixture.root.path());
    let saved = owner.wait_for_saved(fixture.root.path());
    let login = fixture.session_dir(&saved.login);
    let auth = login.join("auth/credential.json");
    let auth_before = fs::read(&auth).unwrap();
    owner.child.kill().unwrap();
    assert_eq!(owner.child.wait().unwrap().signal(), Some(9));
    assert!(
        old_listener_is_live(saved.address),
        "old CPA did not survive"
    );

    let login_record = fs::read(login.join("login.json")).unwrap();
    let owner_path = login
        .join("runtime")
        .join(&saved.login)
        .join("owner.lock/owner.json");
    let ownership_before = fs::read(&owner_path).unwrap();
    let old: serde_json::Value = serde_json::from_slice(&ownership_before).unwrap();
    assert_eq!(old["binary_version"], PREVIOUS_CPA_VERSION);
    assert_eq!(old["cpa_pid"], saved.pid);

    // A response naming another PID must leave the process and private state alone.
    let mismatch = fixture.root.path().join("wrong-process-id");
    fs::write(&mismatch, b"").unwrap();
    let rejected = ManagedCpaRuntimeSet::new(fixture.templates.clone()).unwrap();
    assert!(rejected.login_session(&saved.login).is_none());
    assert!(old_listener_is_live(saved.address));
    assert_eq!(fs::read(&owner_path).unwrap(), ownership_before);
    assert_eq!(fs::read(&auth).unwrap(), auth_before);
    fs::remove_file(mismatch).unwrap();

    // Recover with only the current install bytes available.
    fs::remove_file(fixture.root.path().join(PREVIOUS_BINARY)).unwrap();
    let spawns_before = fixture.spawns();
    let recovered = ManagedCpaRuntimeSet::new(fixture.templates.clone()).unwrap();
    assert!(
        !old_listener_is_live(saved.address),
        "predecessor not retired"
    );
    assert!(!owner_path.exists());
    assert_eq!(fixture.spawns(), spawns_before);
    assert_eq!(fs::read(&auth).unwrap(), auth_before);
    assert_eq!(fs::read(login.join("login.json")).unwrap(), login_record);
    let session = recovered.login_session(&saved.login).unwrap();
    assert_eq!(session.state, CpaLoginState::Authorized);
    assert_eq!(session.account_ref.as_deref(), Some(saved.account.as_str()));

    project(&recovered, &session, 1, CpaSourceManagementState::Enabled).unwrap();
    let restored = target(&selected(&recovered, &session));
    assert_eq!(
        restored.credential_ref().credential_id(),
        saved.credential_id
    );
    assert_eq!(restored.upstream_model_id(), MODEL);
    assert_eq!(fixture.spawns(), spawns_before + 1);
    let running: serde_json::Value =
        serde_json::from_slice(&fs::read(owner_path).unwrap()).unwrap();
    assert_eq!(running["binary_version"], MANAGED_CPA_ARTIFACT_VERSION);
    assert_ne!(running["cpa_pid"], saved.pid);
    lease(&recovered, &restored)
        .unwrap()
        .unwrap()
        .apply_authorization(&mut http::HeaderMap::new())
        .unwrap();
    recovered.shutdown().unwrap();
}

fn prepare_crash(root: &Path) {
    let set = ManagedCpaRuntimeSet::new(Fixture::templates(root)).unwrap();
    let (pending, url) = set.start_login(CpaAccountKind::Codex).unwrap();
    let url = reqwest::Url::parse(&url).unwrap();
    let state = url.query_pairs().find(|(key, _)| key == "state").unwrap().1;
    let callback = format!(
        "http://localhost:1455/auth/callback?state={state}&code=fixture-authorization-code"
    );
    set.submit_login_callback(&pending.login_ref, &callback)
        .unwrap();
    let session = set.login_status(&pending.login_ref).unwrap();
    project(&set, &session, 1, CpaSourceManagementState::Enabled).unwrap();
    let routed = target(&selected(&set, &session));
    set.shutdown().unwrap();

    // Reuse the produced login and auth store. The external predecessor speaks
    // the released private API and is supervised by this disposable owner process.
    let binary = root.join(PREVIOUS_BINARY);
    let source =
        include_str!("cpa_oauth_fixture.py").replace("__HIR_CPA_VERSION__", PREVIOUS_CPA_VERSION);
    fs::write(&binary, &source).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let login = root.join("cpa/managed-logins").join(&session.login_ref);
    let instance = login.join("runtime").join(&session.login_ref);
    let config = instance.join("config.yaml");
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let secrets = InstanceSecrets::generate().unwrap();
    secrets.write(&instance.join("capability.private")).unwrap();
    private_atomic_write(
        &config,
        &render_managed_config(address.port(), &login.join("auth"), &secrets).unwrap(),
    )
    .unwrap();
    let nonce = SecretText::generate().unwrap().expose().to_owned();
    let record = OwnerRecord::claim(
        std::process::id(),
        nonce.clone(),
        PREVIOUS_CPA_VERSION.into(),
        digest.clone(),
    )
    .unwrap();
    let OwnerClaim::Acquired(lease) =
        OwnerLease::acquire(&instance.join("owner.lock"), &record, nonce).unwrap()
    else {
        panic!("fixture retained an unexpected live owner");
    };
    drop(listener);
    let previous = StdCpaProcessBackend
        .spawn(&CpaLaunch {
            binary: VerifiedCpaBinary::fixture(
                binary,
                PREVIOUS_CPA_VERSION.parse().unwrap(),
                digest,
            ),
            config_path: config,
            work_dir: instance.join("work"),
            management_password: secrets.management,
            proxy_environment: crate::proxy_environment::ProxyEnvironment::capture([]),
        })
        .unwrap();
    let pid = previous.pid();
    lease.write(&record.running(pid, address).unwrap()).unwrap();
    wait_until(|| old_listener_is_live(address));
    let saved = SavedSubscription {
        login: session.login_ref,
        account: session.account_ref.unwrap(),
        credential_id: routed.credential_ref().credential_id().to_string(),
        address,
        pid,
    };
    private_atomic_write(
        &root.join(DURABLE_SELECTION),
        &serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    loop {
        std::thread::park();
    }
}

fn old_listener_is_live(address: SocketAddr) -> bool {
    TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok()
}

struct CrashOwner {
    child: Child,
    previous: Option<(u32, SocketAddr)>,
}

impl CrashOwner {
    fn start(root: &Path) -> Self {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime_set::managed_sessions::tests::selection_tests::upgrade_recovery_tests::managed_subscription_recovers_after_crash_with_previous_cpa_running",
                "--nocapture",
            ])
            .env(CRASH_OWNER_CHILD, root)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child,
            previous: None,
        }
    }

    fn wait_for_saved(&mut self, root: &Path) -> SavedSubscription {
        let ready = root.join(DURABLE_SELECTION);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.exists() {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "old owner failed to save"
            );
            assert!(Instant::now() < deadline, "old owner never saved");
            std::thread::sleep(Duration::from_millis(10));
        }
        let saved: SavedSubscription = serde_json::from_slice(&fs::read(ready).unwrap()).unwrap();
        self.previous = Some((saved.pid, saved.address));
        saved
    }
}

impl Drop for CrashOwner {
    fn drop(&mut self) {
        if let Some((pid, address)) = self.previous
            && old_listener_is_live(address)
            && let Ok(mut process) = StdCpaProcessBackend.attach_authenticated(pid)
        {
            let _ = process.shutdown(Duration::from_secs(1));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
