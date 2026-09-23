use std::time::Duration;

use crate::attempt_outcome::AttemptFailureClass;
use crate::ports::{CasOutcome, InMemoryRuntimeStateStore, ProbeLeaseOutcome, RuntimeHealth};
use crate::runtime::state::acquire_target_permit;
use crate::server::core_runtime::profiles::fixed_reasoning;

use super::*;

#[test]
fn dns_lookup_uses_scheme_default_and_preserves_explicit_port() {
    assert_eq!(
        dns_lookup_target("provider.example", TransportScheme::Https).unwrap(),
        ("provider.example".to_owned(), 443)
    );
    assert_eq!(
        dns_lookup_target("provider.example:8443", TransportScheme::Https).unwrap(),
        ("provider.example".to_owned(), 8443)
    );
    assert_eq!(
        dns_lookup_target("provider.example", TransportScheme::Http).unwrap(),
        ("provider.example".to_owned(), 80)
    );
}

async fn active_permits(store: &dyn RuntimeStateStore) -> super::super::OwnedAttemptStatePermits {
    let scope = ExecutionScope::new(
        Instant::now() + Duration::from_secs(1),
        tokio_util::sync::CancellationToken::new(),
    );
    let binding_key = RuntimeStateKey::binding("mechanical-binding");
    let binding_permit = acquire_target_permit(store, &binding_key, &scope)
        .await
        .unwrap()
        .unwrap();
    let credential_key = RuntimeStateKey::credential("mechanical-binding", "credential", "key", 1);
    let credential_permit = acquire_target_permit(store, &credential_key, &scope)
        .await
        .unwrap()
        .unwrap();
    super::super::OwnedAttemptStatePermits::new(
        binding_key,
        binding_permit,
        credential_key,
        credential_permit,
    )
}

fn production_profile() -> CandidateProtocolProfile {
    let mut profile = CandidateProtocolProfile::exact_portable_path(
        crate::server::request_plan::IngressProtocol::Responses,
        crate::server::request_plan::IngressProtocol::Responses,
        "native-model",
        fixed_reasoning("fixed"),
    );
    profile.connector.connector_id = "builtin-openai".into();
    profile
}

#[tokio::test]
async fn connect_first_byte_and_idle_failures_cool_exact_binding() {
    for (class, timeout, expected_class) in [
        (
            CoreAttemptFailureClass::Connect,
            None,
            AttemptFailureClass::Transient,
        ),
        (
            CoreAttemptFailureClass::Connect,
            Some(AttemptTimeoutKind::Connect),
            AttemptFailureClass::Timeout(crate::attempt_outcome::AttemptTimeoutPhase::Connect),
        ),
        (
            CoreAttemptFailureClass::FirstByte,
            Some(AttemptTimeoutKind::FirstByte),
            AttemptFailureClass::Timeout(crate::attempt_outcome::AttemptTimeoutPhase::FirstByte),
        ),
        (
            CoreAttemptFailureClass::StreamIdle,
            Some(AttemptTimeoutKind::StreamIdle),
            AttemptFailureClass::Timeout(crate::attempt_outcome::AttemptTimeoutPhase::StreamIdle),
        ),
    ] {
        let store = InMemoryRuntimeStateStore::default();
        let permits = active_permits(&store).await;
        let cancellation = tokio_util::sync::CancellationToken::new();
        let authority = super::super::RuntimeStateAuthoritySignal::default();
        let confirmed = confirm_mechanical_state(
            &store,
            &permits,
            &production_profile(),
            &RuntimeCooldownPolicy::default(),
            MechanicalFailureSignal {
                class,
                timeout,
                runtime_state_authority: authority,
            },
            Instant::now() + Duration::from_secs(1),
            &cancellation,
        )
        .await
        .unwrap()
        .expect("mechanical failure is classified");

        assert_eq!(confirmed.1.retryability, RetryabilityFact::Retryable);
        assert_eq!(confirmed.0.class, expected_class);
        let binding = store.entry(&RuntimeStateKey::binding("mechanical-binding"));
        assert_eq!(binding.generation, 1);
        assert!(matches!(binding.health, RuntimeHealth::CoolingDown { .. }));
        assert_eq!(
            store
                .entry(&RuntimeStateKey::credential(
                    "mechanical-binding",
                    "credential",
                    "key",
                    1,
                ))
                .generation,
            0,
            "mechanical failures must not cool the credential"
        );
    }
}

struct FailingCasStore;

#[async_trait]
impl ProductionStore for FailingCasStore {
    async fn read_exact(
        &self,
        _key: &RuntimeStateKey,
        _scope: &ExecutionScope,
    ) -> Result<RuntimeStateEntry, PortError> {
        Ok(RuntimeStateEntry::default())
    }

    async fn compare_and_swap_exact(
        &self,
        _key: &RuntimeStateKey,
        _expected_generation: u64,
        _next: RuntimeStateEntry,
        _scope: &ExecutionScope,
    ) -> Result<CasOutcome, PortError> {
        Err(PortError::Unavailable("external RuntimeStateStore"))
    }

    async fn acquire_probe_lease_exact(
        &self,
        _key: &RuntimeStateKey,
        _expected_generation: u64,
        _now: Instant,
        _lease_duration: Duration,
        _scope: &ExecutionScope,
    ) -> Result<ProbeLeaseOutcome, PortError> {
        Err(PortError::Unavailable("external RuntimeStateStore"))
    }
}

#[tokio::test]
async fn mechanical_failure_state_write_error_is_fail_closed() {
    let store = ProductionStateAdapter {
        inner: Arc::new(FailingCasStore),
    };
    let permits = active_permits(&store).await;
    let cancellation = tokio_util::sync::CancellationToken::new();
    let authority = super::super::RuntimeStateAuthoritySignal::default();

    let error = confirm_mechanical_state(
        &store,
        &permits,
        &production_profile(),
        &RuntimeCooldownPolicy::default(),
        MechanicalFailureSignal {
            class: CoreAttemptFailureClass::Connect,
            timeout: None,
            runtime_state_authority: authority.clone(),
        },
        Instant::now() + Duration::from_secs(1),
        &cancellation,
    )
    .await
    .unwrap_err();

    assert_eq!(error.as_ref(), RUNTIME_STATE_AUTHORITY_FAILED);
    assert!(authority.failed());
}
