//! Native files and a controlled profile reader exercise identity lookup behavior.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Barrier, Condvar, Mutex as StdMutex, mpsc};
use std::thread;
use std::time::Instant;

#[derive(Default)]
struct ProfileGate {
    state: StdMutex<(usize, bool)>,
    changed: Condvar,
}

impl ProfileGate {
    fn block(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while state.0 == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "controlled profile request did not start"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

struct ReleaseProfileOnDrop(Arc<ProfileGate>);

impl Drop for ReleaseProfileOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Default)]
struct ControlledReader {
    calls: AtomicUsize,
    fail: AtomicBool,
    fail_tokens: StdMutex<BTreeSet<String>>,
    accounts: StdMutex<BTreeMap<String, String>>,
    blocked_token: Option<String>,
    gate: Option<Arc<ProfileGate>>,
}

impl ClaudeProfileReader for ControlledReader {
    fn fetch(
        &self,
        token: &str,
        _proxy: &crate::proxy_environment::ProxyEnvironment,
    ) -> Result<String, CpaLifecycleError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.blocked_token.as_deref() == Some(token) {
            self.gate.as_ref().unwrap().block();
        }
        if self.fail.load(Ordering::SeqCst) || self.fail_tokens.lock().unwrap().contains(token) {
            return Err(CpaLifecycleError::BorrowedClaudeAuthUnavailable);
        }
        Ok(self
            .accounts
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .unwrap_or_else(|| "controlled-account-one".into()))
    }
}

fn write_source(path: &Path, token: &str) {
    private_atomic_write(
        path,
        &serde_json::to_vec(&serde_json::json!({
            "claudeAiOauth": {
                "accessToken": token,
                "refreshToken": "TEST_REFRESH_MUST_REMAIN_NATIVE",
                "expiresAt": u64::MAX,
                "scopes": ["user:profile", "user:inference"]
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

fn source(root: &Path, token: &str, reader: &Arc<ControlledReader>) -> BorrowedClaudeAuthSpec {
    ensure_private_dir(root).unwrap();
    let path = root.join(".credentials.json");
    write_source(&path, token);
    let mut spec = BorrowedClaudeAuthSpec::new(ClaudeSubscriptionLocation::File(path));
    spec.profile_reader = reader.clone();
    spec
}

#[test]
fn concurrent_identical_revision_fetches_one_profile_on_success_and_failure() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let gate = Arc::new(ProfileGate::default());
        let _release = ReleaseProfileOnDrop(Arc::clone(&gate));
        let reader = Arc::new(ControlledReader {
            fail: AtomicBool::new(fail),
            blocked_token: Some("shared-access".into()),
            gate: Some(Arc::clone(&gate)),
            ..ControlledReader::default()
        });
        let spec = source(root.path(), "shared-access", &reader);
        let original = fs::read(spec.source_path()).unwrap();
        let barrier = Arc::new(Barrier::new(9));
        let tasks = (0..8)
            .map(|_| {
                let spec = spec.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    spec.inspect()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        gate.wait_entered();
        gate.release();
        for task in tasks {
            assert_eq!(task.join().unwrap().is_err(), fail);
        }
        assert_eq!(
            reader.calls.load(Ordering::SeqCst),
            1,
            "same-revision callers duplicated remote account lookup"
        );
        assert_eq!(fs::read(spec.source_path()).unwrap(), original);
    }
}

#[test]
fn failed_profile_is_backed_off_but_new_access_revision_is_immediately_eligible() {
    let root = tempfile::tempdir().unwrap();
    let reader = Arc::new(ControlledReader::default());
    reader.fail.store(true, Ordering::SeqCst);
    let spec = source(root.path(), "failed-access", &reader);
    for _ in 0..16 {
        assert!(spec.inspect().is_err());
    }
    assert_eq!(
        reader.calls.load(Ordering::SeqCst),
        1,
        "failure caused one remote lookup per request"
    );
    write_source(spec.source_path(), "rotated-access");
    assert!(spec.inspect().is_err());
    assert_eq!(
        reader.calls.load(Ordering::SeqCst),
        2,
        "new revision inherited an older token's failure delay"
    );
    reader.fail.store(false, Ordering::SeqCst);
    assert!(
        spec.inspect().is_err(),
        "same revision retried before the failure interval elapsed"
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
    // This is the documented retry interval, not a scheduling/order assumption.
    thread::sleep(Duration::from_millis(2_100));
    let recovered = spec.inspect().unwrap();
    assert_eq!(reader.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        spec.inspect().unwrap().account_ref(),
        recovered.account_ref()
    );
    assert_eq!(reader.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn slow_old_revision_does_not_hold_up_new_identity_or_overwrite_its_result() {
    for fail_old in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let gate = Arc::new(ProfileGate::default());
        let _release = ReleaseProfileOnDrop(Arc::clone(&gate));
        let reader = Arc::new(ControlledReader {
            blocked_token: Some("old-slow-access".into()),
            gate: Some(Arc::clone(&gate)),
            ..ControlledReader::default()
        });
        if fail_old {
            reader
                .fail_tokens
                .lock()
                .unwrap()
                .insert("old-slow-access".into());
        }
        reader
            .accounts
            .lock()
            .unwrap()
            .insert("new-access".into(), "controlled-account-two".into());
        let spec = source(root.path(), "old-slow-access", &reader);
        let old_spec = spec.clone();
        let old = thread::spawn(move || old_spec.inspect());
        gate.wait_entered();
        write_source(spec.source_path(), "new-access");
        let new_spec = spec.clone();
        let (completed, completion) = mpsc::channel();
        let new = thread::spawn(move || {
            let result = new_spec.inspect().map(|evidence| evidence.account_ref());
            completed.send(result).unwrap();
        });
        let early = completion.recv_timeout(Duration::from_millis(600));
        gate.release();
        let _ = old.join().unwrap();
        new.join().unwrap();
        let current_account = early
            .expect("new access revision queued behind old remote profile IO")
            .unwrap();
        assert_eq!(
            spec.inspect().unwrap().account_ref(),
            current_account,
            "late old profile replaced the current account proof"
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn a_terminated_waiter_does_not_cancel_another_callers_shared_profile() {
    for cancel in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let gate = Arc::new(ProfileGate::default());
        let _release = ReleaseProfileOnDrop(Arc::clone(&gate));
        let reader = Arc::new(ControlledReader {
            blocked_token: Some("shared-access".into()),
            gate: Some(Arc::clone(&gate)),
            ..ControlledReader::default()
        });
        let spec = source(root.path(), "shared-access", &reader);
        let owner_spec = spec.clone();
        let owner = thread::spawn(move || owner_spec.inspect());
        gate.wait_entered();
        let context = crate::CpaRequestContext::new(
            Instant::now()
                + if cancel {
                    Duration::from_secs(3)
                } else {
                    Duration::from_millis(120)
                },
        );
        let waiter_context = context.clone();
        let waiter_spec = spec.clone();
        let (completed, completion) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = waiter_context.run(|| waiter_spec.inspect());
            completed.send(result.is_err()).unwrap();
        });
        if cancel {
            context.cancel();
        }
        let terminated = completion.recv_timeout(Duration::from_millis(600));
        gate.release();
        let proof = owner.join().unwrap().unwrap();
        waiter.join().unwrap();
        assert_eq!(
            terminated.ok(),
            Some(true),
            "terminated identity waiter remained attached to slow profile IO"
        );
        assert_eq!(spec.inspect().unwrap().account_ref(), proof.account_ref());
        assert_eq!(
            reader.calls.load(Ordering::SeqCst),
            1,
            "waiter termination revoked another caller's shared account proof"
        );
    }
}

#[test]
fn initiating_owner_cancellation_lets_a_valid_waiter_take_over_without_failure_backoff() {
    struct FirstOwnerReader {
        calls: AtomicUsize,
        gate: Arc<ProfileGate>,
        takeover: Arc<ProfileGate>,
    }
    impl ClaudeProfileReader for FirstOwnerReader {
        fn fetch(
            &self,
            _token: &str,
            _proxy: &crate::proxy_environment::ProxyEnvironment,
        ) -> Result<String, CpaLifecycleError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.gate.block();
                crate::request_context::check()?;
            } else {
                self.takeover.block();
            }
            Ok("controlled-account-one".into())
        }
    }

    let root = tempfile::tempdir().unwrap();
    ensure_private_dir(root.path()).unwrap();
    let path = root.path().join(".credentials.json");
    write_source(&path, "shared-access");
    let gate = Arc::new(ProfileGate::default());
    let _release = ReleaseProfileOnDrop(Arc::clone(&gate));
    let takeover = Arc::new(ProfileGate::default());
    let _release_takeover = ReleaseProfileOnDrop(Arc::clone(&takeover));
    let reader = Arc::new(FirstOwnerReader {
        calls: AtomicUsize::new(0),
        gate: Arc::clone(&gate),
        takeover: Arc::clone(&takeover),
    });
    let mut spec = BorrowedClaudeAuthSpec::new(ClaudeSubscriptionLocation::File(path));
    spec.profile_reader = reader.clone();
    let context = crate::CpaRequestContext::new(Instant::now() + Duration::from_secs(3));
    let owner_context = context.clone();
    let owner_spec = spec.clone();
    let owner = thread::spawn(move || owner_context.run(|| owner_spec.inspect()));
    gate.wait_entered();
    let (completed, completion) = mpsc::channel();
    let ready = Arc::new(Barrier::new(9));
    let waiters = (0..8)
        .map(|_| {
            let waiter_spec = spec.clone();
            let completed = completed.clone();
            let ready = ready.clone();
            thread::spawn(move || {
                ready.wait();
                completed.send(waiter_spec.inspect().is_ok()).unwrap();
            })
        })
        .collect::<Vec<_>>();
    ready.wait();
    context.cancel();
    gate.release();
    takeover.wait_entered();
    assert!(owner.join().unwrap().is_err());
    takeover.release();
    for _ in 0..8 {
        assert_eq!(
            completion.recv_timeout(Duration::from_millis(600)).ok(),
            Some(true),
            "a cancelled initiating owner imposed its failure/backoff on a valid caller"
        );
    }
    for waiter in waiters {
        waiter.join().unwrap();
    }
    assert!(spec.inspect().is_ok());
    assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn profile_cache_key_keeps_canonical_native_sources_distinct() {
    let root = tempfile::tempdir().unwrap();
    let reader = Arc::new(ControlledReader::default());
    let first = source(root.path(), "shared-token", &reader);
    let first_evidence = first.inspect().unwrap();
    let other_path = root.path().join("other-credentials.json");
    write_source(&other_path, "shared-token");
    let mut other = first.clone();
    other.location = ClaudeSubscriptionLocation::File(other_path);
    let other_evidence = other.inspect().unwrap();
    assert_eq!(first_evidence.account_ref(), other_evidence.account_ref());
    assert_ne!(
        first_evidence.binding_evidence_digest(),
        other_evidence.binding_evidence_digest()
    );
    assert_eq!(
        reader.calls.load(Ordering::SeqCst),
        2,
        "different source reused an unrelated source's cache entry"
    );
    first.inspect().unwrap();
    other.inspect().unwrap();
    assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn retained_profile_results_are_bounded_without_rejecting_new_revisions() {
    let root = tempfile::tempdir().unwrap();
    let reader = Arc::new(ControlledReader::default());
    let spec = source(root.path(), "access-0", &reader);
    for index in 0..12 {
        write_source(spec.source_path(), &format!("access-{index}"));
        spec.inspect().unwrap();
    }
    assert_eq!(reader.calls.load(Ordering::SeqCst), 12);
    for index in 0..12 {
        write_source(spec.source_path(), &format!("access-{index}"));
        spec.inspect().unwrap();
    }
    assert!(
        reader.calls.load(Ordering::SeqCst) >= 16,
        "more than eight past revisions remained cached"
    );
}
