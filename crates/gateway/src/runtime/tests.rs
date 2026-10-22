use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::attempt_outcome::{
    AttemptFailure, AttemptFailureClass, AttemptTimeoutPhase, PreOutputStreamClass,
};
use crate::ports::{
    CasOutcome, ExecutionScope, InMemoryRuntimeStateStore, ProbeLeaseOutcome, RuntimeHealth,
    RuntimeStateEntry, RuntimeStateError, RuntimeStateKey, RuntimeStateStore,
};

use super::state::{
    OwnedAttemptStatePermits, RuntimeCooldownPolicy, acquire_target_permit, confirm_success,
    record_failure, record_failure_at, validate_permit,
};

#[derive(Default)]
struct IndependentCasStore {
    entries: Mutex<BTreeMap<RuntimeStateKey, RuntimeStateEntry>>,
}

#[async_trait]
impl RuntimeStateStore for IndependentCasStore {
    async fn read(
        &self,
        key: &RuntimeStateKey,
        _scope: &ExecutionScope,
    ) -> Result<RuntimeStateEntry, RuntimeStateError> {
        Ok(self.entries.lock().get(key).cloned().unwrap_or_default())
    }

    async fn compare_and_swap(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        next: RuntimeStateEntry,
        _scope: &ExecutionScope,
    ) -> Result<CasOutcome, RuntimeStateError> {
        let required_generation = expected_generation
            .checked_add(1)
            .ok_or(RuntimeStateError::Rejected)?;
        let mut entries = self.entries.lock();
        let current = entries.get(key).cloned().unwrap_or_default();
        if current.generation != expected_generation || next.generation != required_generation {
            return Ok(CasOutcome::Conflict);
        }
        entries.insert(key.clone(), next);
        Ok(CasOutcome::Applied {
            generation: required_generation,
        })
    }

    async fn acquire_probe_lease(
        &self,
        _key: &RuntimeStateKey,
        _expected_generation: u64,
        _now: Instant,
        _lease_duration: Duration,
        _scope: &ExecutionScope,
    ) -> Result<ProbeLeaseOutcome, RuntimeStateError> {
        Err(RuntimeStateError::Unavailable)
    }
}

fn scope() -> ExecutionScope {
    ExecutionScope::new(
        Instant::now() + Duration::from_secs(2),
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn runtime_state_independent_adapter_allows_only_one_same_generation_writer() {
    let store = Arc::new(IndependentCasStore::default());
    let key = RuntimeStateKey::binding("binding-a");
    let disabled = RuntimeStateEntry {
        generation: 1,
        health: RuntimeHealth::Disabled,
        probe_lease_until: None,
        transient_backoff_step: 0,
    };
    let cooling = RuntimeStateEntry {
        generation: 1,
        health: RuntimeHealth::CoolingDown {
            until: Instant::now() + Duration::from_secs(30),
        },
        probe_lease_until: None,
        transient_backoff_step: 0,
    };
    let left = {
        let store = Arc::clone(&store);
        let key = key.clone();
        let scope = scope();
        tokio::spawn(async move { store.compare_and_swap(&key, 0, disabled, &scope).await })
    };
    let right = {
        let store = Arc::clone(&store);
        let key = key.clone();
        let scope = scope();
        tokio::spawn(async move { store.compare_and_swap(&key, 0, cooling, &scope).await })
    };
    let outcomes = [left.await.unwrap().unwrap(), right.await.unwrap().unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, CasOutcome::Applied { generation: 1 }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CasOutcome::Conflict)
            .count(),
        1
    );
    assert_eq!(store.read(&key, &scope()).await.unwrap().generation, 1);
}

#[tokio::test]
async fn runtime_state_in_memory_adapter_rejects_non_incrementing_next_generation() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("binding-a");
    let outcome = store
        .compare_and_swap(
            &key,
            0,
            RuntimeStateEntry {
                generation: 0,
                health: RuntimeHealth::Disabled,
                probe_lease_until: None,
                transient_backoff_step: 0,
            },
            &scope(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, CasOutcome::Conflict);
    assert_eq!(store.entry(&key), RuntimeStateEntry::default());
}

#[tokio::test]
async fn expired_cooldown_probe_lease_covers_the_bounded_attempt() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("slow-model-binding");
    store.insert(
        key.clone(),
        RuntimeStateEntry {
            generation: 7,
            health: RuntimeHealth::CoolingDown {
                until: Instant::now() - Duration::from_secs(1),
            },
            probe_lease_until: None,
            transient_backoff_step: 0,
        },
    );
    let started = Instant::now();
    let deadline = started + Duration::from_secs(30);
    let scope = ExecutionScope::new(deadline, CancellationToken::new());

    let permit = acquire_target_permit(&store, &key, &scope)
        .await
        .unwrap()
        .expect("expired cooldown must elect one probe");
    let lease_until = store
        .entry(&key)
        .probe_lease_until
        .expect("probe lease must be persisted");

    assert!(lease_until > started + Duration::from_secs(25));
    assert!(lease_until <= deadline);
    validate_permit(&store, &key, permit, &scope)
        .await
        .expect("a slow but in-deadline attempt must keep its probe authority");
}

#[tokio::test]
async fn credential_scoped_failure_without_a_credential_cools_only_the_binding() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("no-credential-binding");
    let request_scope = scope();
    let permit = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
    record_failure(
        &store,
        permits.borrowed(),
        &AttemptFailure {
            class: AttemptFailureClass::Quota,
            status: Some(429),
            retry_after: Some(Duration::from_secs(5)),
        },
        &RuntimeCooldownPolicy::default(),
        &request_scope,
    )
    .await
    .unwrap();

    assert!(matches!(
        store.entry(&key).health,
        RuntimeHealth::CoolingDown { .. }
    ));
    assert_eq!(store.entry(&key).transient_backoff_step, 0);
}

#[tokio::test]
async fn binding_transient_backoff_saturates_and_trusted_retry_after_resets_it() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("binding-backoff");
    let request_scope = scope();
    let failure = AttemptFailure {
        class: AttemptFailureClass::Transient,
        status: Some(503),
        retry_after: None,
    };

    for (expected_step, expected_seconds) in [(1, 2), (2, 4), (3, 8), (4, 16), (5, 30), (5, 30)] {
        let permit = acquire_target_permit(&store, &key, &request_scope)
            .await
            .unwrap()
            .unwrap();
        let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
        let now = Instant::now();
        record_failure_at(
            &store,
            permits.borrowed(),
            &failure,
            &RuntimeCooldownPolicy::default(),
            &request_scope,
            now,
        )
        .await
        .unwrap();
        let mut state = store.entry(&key);
        assert_eq!(state.transient_backoff_step, expected_step);
        assert_eq!(
            state.health,
            RuntimeHealth::CoolingDown {
                until: now + Duration::from_secs(expected_seconds)
            }
        );
        state.health = RuntimeHealth::CoolingDown {
            until: Instant::now() - Duration::from_millis(1),
        };
        state.probe_lease_until = None;
        store.insert(key.clone(), state);
    }

    let permit = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
    let now = Instant::now();
    record_failure_at(
        &store,
        permits.borrowed(),
        &AttemptFailure {
            class: AttemptFailureClass::Transient,
            status: Some(503),
            retry_after: Some(Duration::from_secs(77)),
        },
        &RuntimeCooldownPolicy::default(),
        &request_scope,
        now,
    )
    .await
    .unwrap();
    let state = store.entry(&key);
    assert_eq!(state.transient_backoff_step, 0);
    assert_eq!(
        state.health,
        RuntimeHealth::CoolingDown {
            until: now + Duration::from_secs(77)
        }
    );
}

#[tokio::test]
async fn stale_binding_failure_cannot_increment_the_same_backoff_state_twice() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("binding-concurrent-backoff");
    let request_scope = scope();
    let left = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let right = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let failure = AttemptFailure {
        class: AttemptFailureClass::Transient,
        status: Some(503),
        retry_after: None,
    };
    let now = Instant::now();
    let left = OwnedAttemptStatePermits::without_credential(key.clone(), left);
    record_failure_at(
        &store,
        left.borrowed(),
        &failure,
        &RuntimeCooldownPolicy::default(),
        &request_scope,
        now,
    )
    .await
    .unwrap();
    let right = OwnedAttemptStatePermits::without_credential(key.clone(), right);
    assert!(
        record_failure_at(
            &store,
            right.borrowed(),
            &failure,
            &RuntimeCooldownPolicy::default(),
            &request_scope,
            now,
        )
        .await
        .is_err()
    );
    let current = store.entry(&key);
    assert_eq!(current.generation, 1);
    assert_eq!(current.transient_backoff_step, 1);
}

#[tokio::test]
async fn overload_uses_default_and_successful_probe_resets_transient_step() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("binding-overload");
    let request_scope = scope();
    store.insert(
        key.clone(),
        RuntimeStateEntry {
            generation: 4,
            health: RuntimeHealth::CoolingDown {
                until: Instant::now() - Duration::from_millis(1),
            },
            probe_lease_until: None,
            transient_backoff_step: 5,
        },
    );
    let permit = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
    let now = Instant::now();
    record_failure_at(
        &store,
        permits.borrowed(),
        &AttemptFailure {
            class: AttemptFailureClass::BindingOverload,
            status: Some(529),
            retry_after: None,
        },
        &RuntimeCooldownPolicy::default(),
        &request_scope,
        now,
    )
    .await
    .unwrap();
    let mut state = store.entry(&key);
    assert_eq!(state.transient_backoff_step, 0);
    assert_eq!(
        state.health,
        RuntimeHealth::CoolingDown {
            until: now + Duration::from_secs(15)
        }
    );

    state.health = RuntimeHealth::CoolingDown {
        until: Instant::now() - Duration::from_millis(1),
    };
    state.transient_backoff_step = 4;
    store.insert(key.clone(), state);
    let permit = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
    confirm_success(&store, permits.borrowed(), &request_scope)
        .await
        .unwrap();
    let active = store.entry(&key);
    assert_eq!(active.health, RuntimeHealth::Active);
    assert_eq!(active.transient_backoff_step, 0);
}

#[tokio::test]
async fn agent_grant_soft_failures_leave_shared_state_and_inflight_main_permits_unchanged() {
    for class in [
        AttemptFailureClass::Transient,
        AttemptFailureClass::BindingOverload,
        AttemptFailureClass::Timeout(AttemptTimeoutPhase::FirstByte),
        AttemptFailureClass::Disconnect,
        AttemptFailureClass::PreOutputStream(PreOutputStreamClass::Transient),
        AttemptFailureClass::PreOutputStream(PreOutputStreamClass::BindingOverload),
    ] {
        let store = InMemoryRuntimeStateStore::default();
        let key = RuntimeStateKey::binding("shared-agent-binding");
        let request_scope = scope();
        let main = acquire_target_permit(&store, &key, &request_scope)
            .await
            .unwrap()
            .unwrap();
        let auxiliary = acquire_target_permit(&store, &key, &request_scope)
            .await
            .unwrap()
            .unwrap();
        let permits = OwnedAttemptStatePermits::without_credential(key.clone(), auxiliary);
        record_failure(
            &store,
            permits.borrowed(),
            &AttemptFailure {
                class,
                status: Some(503),
                retry_after: Some(Duration::from_secs(77)),
            },
            &RuntimeCooldownPolicy::for_gateway_model_execution(),
            &request_scope,
        )
        .await
        .unwrap();

        assert_eq!(store.entry(&key), RuntimeStateEntry::default(), "{class:?}");
        let main = OwnedAttemptStatePermits::without_credential(key.clone(), main);
        confirm_success(&store, main.borrowed(), &request_scope)
            .await
            .expect("an independent soft failure must not invalidate the in-flight main");
        assert!(
            acquire_target_permit(&store, &key, &request_scope)
                .await
                .unwrap()
                .is_some(),
            "{class:?} must not suppress the next request"
        );
    }
}

#[tokio::test]
async fn agent_grant_failed_probe_releases_lease_without_changing_prior_health_or_step() {
    for (class, status) in [
        (AttemptFailureClass::Transient, 503),
        (AttemptFailureClass::PermanentClient, 400),
    ] {
        let store = InMemoryRuntimeStateStore::default();
        let key = RuntimeStateKey::binding("expired-agent-binding");
        let credential =
            RuntimeStateKey::credential("expired-agent-binding", "agent-credential", "key-a", 1);
        let expired_until = Instant::now() - Duration::from_secs(1);
        store.insert(
            key.clone(),
            RuntimeStateEntry {
                generation: 7,
                health: RuntimeHealth::CoolingDown {
                    until: expired_until,
                },
                probe_lease_until: None,
                transient_backoff_step: 3,
            },
        );
        store.insert(
            credential.clone(),
            RuntimeStateEntry {
                generation: 11,
                health: RuntimeHealth::CoolingDown {
                    until: expired_until,
                },
                probe_lease_until: None,
                transient_backoff_step: 0,
            },
        );
        let request_scope = scope();
        let permit = acquire_target_permit(&store, &key, &request_scope)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.entry(&key).generation, 8);
        assert!(store.entry(&key).probe_lease_until.is_some());
        let credential_permit = acquire_target_permit(&store, &credential, &request_scope)
            .await
            .unwrap()
            .unwrap();
        let permits = OwnedAttemptStatePermits::new(
            key.clone(),
            permit,
            credential.clone(),
            credential_permit,
        );
        record_failure(
            &store,
            permits.borrowed(),
            &AttemptFailure {
                class,
                status: Some(status),
                retry_after: None,
            },
            &RuntimeCooldownPolicy::for_gateway_model_execution(),
            &request_scope,
        )
        .await
        .unwrap();

        assert_eq!(
            store.entry(&key),
            RuntimeStateEntry {
                generation: 9,
                health: RuntimeHealth::CoolingDown {
                    until: expired_until,
                },
                probe_lease_until: None,
                transient_backoff_step: 3,
            }
        );
        assert_eq!(
            store.entry(&credential),
            RuntimeStateEntry {
                generation: 13,
                health: RuntimeHealth::CoolingDown {
                    until: expired_until,
                },
                probe_lease_until: None,
                transient_backoff_step: 0,
            }
        );
        assert!(
            acquire_target_permit(&store, &key, &request_scope)
                .await
                .unwrap()
                .is_some(),
            "the next request can probe immediately without treating the failure as a recovery"
        );
        assert!(
            acquire_target_permit(&store, &credential, &request_scope)
                .await
                .unwrap()
                .is_some(),
            "the soft failure must not leave an expired-quota probe busy"
        );
    }
}

#[tokio::test]
async fn agent_grant_stale_failed_probe_cannot_release_another_generation() {
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("stale-agent-probe");
    store.insert(
        key.clone(),
        RuntimeStateEntry {
            generation: 7,
            health: RuntimeHealth::CoolingDown {
                until: Instant::now() - Duration::from_secs(1),
            },
            probe_lease_until: None,
            transient_backoff_step: 3,
        },
    );
    let request_scope = scope();
    let permit = acquire_target_permit(&store, &key, &request_scope)
        .await
        .unwrap()
        .unwrap();
    let mut newer = store.entry(&key);
    newer.generation += 1;
    newer.probe_lease_until = Some(Instant::now() + Duration::from_secs(30));
    store.insert(key.clone(), newer.clone());
    let permits = OwnedAttemptStatePermits::without_credential(key.clone(), permit);
    assert!(
        record_failure(
            &store,
            permits.borrowed(),
            &AttemptFailure {
                class: AttemptFailureClass::Transient,
                status: Some(503),
                retry_after: None,
            },
            &RuntimeCooldownPolicy::for_gateway_model_execution(),
            &request_scope,
        )
        .await
        .is_err()
    );
    assert_eq!(store.entry(&key), newer);
}

#[tokio::test]
async fn agent_grant_credential_and_quota_failures_still_update_shared_authority() {
    for (class, status) in [
        (AttemptFailureClass::Credential, 401),
        (AttemptFailureClass::Quota, 429),
        (
            AttemptFailureClass::PreOutputStream(PreOutputStreamClass::Credential),
            401,
        ),
        (
            AttemptFailureClass::PreOutputStream(PreOutputStreamClass::Quota),
            429,
        ),
    ] {
        for has_credential in [true, false] {
            let store = InMemoryRuntimeStateStore::default();
            let binding = RuntimeStateKey::binding("agent-binding");
            let credential =
                RuntimeStateKey::credential("agent-binding", "agent-credential", "key-a", 1);
            let request_scope = scope();
            let binding_permit = acquire_target_permit(&store, &binding, &request_scope)
                .await
                .unwrap()
                .unwrap();
            let permits = if has_credential {
                let permit = acquire_target_permit(&store, &credential, &request_scope)
                    .await
                    .unwrap()
                    .unwrap();
                OwnedAttemptStatePermits::new(
                    binding.clone(),
                    binding_permit,
                    credential.clone(),
                    permit,
                )
            } else {
                OwnedAttemptStatePermits::without_credential(binding.clone(), binding_permit)
            };
            record_failure(
                &store,
                permits.borrowed(),
                &AttemptFailure {
                    class,
                    status: Some(status),
                    retry_after: Some(Duration::from_secs(60)),
                },
                &RuntimeCooldownPolicy::for_gateway_model_execution(),
                &request_scope,
            )
            .await
            .unwrap();

            let target = if has_credential {
                &credential
            } else {
                &binding
            };
            let state = store.entry(target);
            assert_eq!(
                state.generation, 1,
                "{class:?}, credential={has_credential}"
            );
            assert_eq!(state.transient_backoff_step, 0);
            if status == 401 {
                assert_eq!(state.health, RuntimeHealth::Disabled);
            } else {
                assert!(matches!(state.health, RuntimeHealth::CoolingDown { .. }));
            }
            if has_credential {
                assert_eq!(store.entry(&binding), RuntimeStateEntry::default());
            }
        }
    }
}
