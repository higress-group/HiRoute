use std::time::{Duration, Instant};

use thiserror::Error;

use crate::attempt_outcome::{AttemptFailure, FailureStateScope};
use crate::ports::{
    CasOutcome, ExecutionScope, ProbeLeaseOutcome, RuntimeHealth, RuntimeStateEntry,
    RuntimeStateError, RuntimeStateKey, RuntimeStateStore, ScopeError,
};

pub(crate) fn native_endpoint_state_key(
    stable_binding_id: &str,
    profile_digest: &str,
) -> Option<String> {
    let digest = profile_digest.strip_prefix("sha256:")?.get(..32)?;
    Some(format!("{stable_binding_id}/{digest}"))
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeCooldownPolicy {
    pub credential_quota: Duration,
    pub binding: Duration,
}

impl Default for RuntimeCooldownPolicy {
    fn default() -> Self {
        Self {
            credential_quota: Duration::from_secs(60),
            binding: Duration::from_secs(15),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StatePermit {
    generation: u64,
    probe: bool,
    transient_backoff_step: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermitAvailability {
    Permit(StatePermit),
    CoolingDown { until: Instant },
    ProbeBusy,
    Disabled,
}

#[derive(Clone, Copy)]
pub(crate) struct AttemptStatePermits<'a> {
    binding: (&'a RuntimeStateKey, StatePermit),
    credential: Option<(&'a RuntimeStateKey, StatePermit)>,
}

pub(crate) struct OwnedAttemptStatePermits {
    binding_key: RuntimeStateKey,
    binding_permit: StatePermit,
    credential_key: Option<RuntimeStateKey>,
    credential_permit: Option<StatePermit>,
}

impl OwnedAttemptStatePermits {
    pub(crate) fn new(
        binding_key: RuntimeStateKey,
        binding_permit: StatePermit,
        credential_key: RuntimeStateKey,
        credential_permit: StatePermit,
    ) -> Self {
        Self {
            binding_key,
            binding_permit,
            credential_key: Some(credential_key),
            credential_permit: Some(credential_permit),
        }
    }

    pub(crate) fn without_credential(
        binding_key: RuntimeStateKey,
        binding_permit: StatePermit,
    ) -> Self {
        Self {
            binding_key,
            binding_permit,
            credential_key: None,
            credential_permit: None,
        }
    }

    pub(crate) fn borrowed(&self) -> AttemptStatePermits<'_> {
        AttemptStatePermits {
            binding: (&self.binding_key, self.binding_permit),
            credential: self.credential_key.as_ref().zip(self.credential_permit),
        }
    }
}

#[cfg(test)]
pub(crate) async fn acquire_target_permit(
    store: &dyn RuntimeStateStore,
    key: &RuntimeStateKey,
    scope: &ExecutionScope,
) -> Result<Option<StatePermit>, StateAccessError> {
    Ok(
        match acquire_target_permit_status(store, key, scope).await? {
            PermitAvailability::Permit(permit) => Some(permit),
            PermitAvailability::CoolingDown { .. }
            | PermitAvailability::ProbeBusy
            | PermitAvailability::Disabled => None,
        },
    )
}

pub(crate) async fn acquire_target_permit_status(
    store: &dyn RuntimeStateStore,
    key: &RuntimeStateKey,
    scope: &ExecutionScope,
) -> Result<PermitAvailability, StateAccessError> {
    let entry = scoped_read(store, key, scope).await?;
    match entry.health {
        RuntimeHealth::Active => Ok(PermitAvailability::Permit(StatePermit {
            generation: entry.generation,
            probe: false,
            transient_backoff_step: entry.transient_backoff_step,
        })),
        RuntimeHealth::Disabled => Ok(PermitAvailability::Disabled),
        RuntimeHealth::CoolingDown { until } if until > Instant::now() => {
            Ok(PermitAvailability::CoolingDown { until })
        }
        RuntimeHealth::CoolingDown { .. } => {
            let now = Instant::now();
            let remaining = scope
                .deadline()
                .checked_duration_since(now)
                .ok_or(StateAccessError::Scope(ScopeError::Deadline))?;
            // A probe owns the exact target for the full bounded Attempt. LLM
            // latency routinely exceeds a few seconds; a shorter fixed lease
            // would let another request probe the same target and would make a
            // valid slow response fail its precommit state confirmation.
            let lease_duration = remaining;
            if lease_duration.is_zero() {
                return Err(StateAccessError::Scope(ScopeError::Deadline));
            }
            let outcome = scope
                .run(store.acquire_probe_lease(key, entry.generation, now, lease_duration, scope))
                .await??;
            match outcome {
                ProbeLeaseOutcome::Acquired { generation } => {
                    Ok(PermitAvailability::Permit(StatePermit {
                        generation,
                        probe: true,
                        transient_backoff_step: entry.transient_backoff_step,
                    }))
                }
                ProbeLeaseOutcome::Busy => Ok(PermitAvailability::ProbeBusy),
                ProbeLeaseOutcome::Conflict => Err(StateAccessError::Conflict),
            }
        }
    }
}

pub(crate) async fn record_failure(
    store: &dyn RuntimeStateStore,
    permits: AttemptStatePermits<'_>,
    failure: &AttemptFailure,
    policy: &RuntimeCooldownPolicy,
    scope: &ExecutionScope,
) -> Result<(), StateAccessError> {
    record_failure_at(store, permits, failure, policy, scope, Instant::now()).await
}

pub(crate) async fn record_failure_at(
    store: &dyn RuntimeStateStore,
    permits: AttemptStatePermits<'_>,
    failure: &AttemptFailure,
    policy: &RuntimeCooldownPolicy,
    scope: &ExecutionScope,
    now: Instant,
) -> Result<(), StateAccessError> {
    let Some(state_scope) = failure.state_scope() else {
        return Ok(());
    };
    let (key, permit) = match state_scope {
        FailureStateScope::Credential => permits.credential.unwrap_or(permits.binding),
        FailureStateScope::Binding => permits.binding,
    };
    let (health, transient_backoff_step) = if failure.disables_credential() {
        (RuntimeHealth::Disabled, 0)
    } else {
        let (duration, next_step) = match (state_scope, failure.retry_after) {
            (_, Some(duration)) => (duration, 0),
            (FailureStateScope::Binding, None) if uses_transient_backoff(failure) => {
                let step = permit.transient_backoff_step.saturating_add(1).min(5);
                (transient_backoff_delay(step), step)
            }
            (FailureStateScope::Binding, None) => (policy.binding, 0),
            (FailureStateScope::Credential, None) => (policy.credential_quota, 0),
        };
        (
            RuntimeHealth::CoolingDown {
                until: now
                    .checked_add(duration)
                    .ok_or(StateAccessError::InvalidTransition)?,
            },
            next_step,
        )
    };
    let outcome = scope
        .run(
            store.compare_and_swap(
                key,
                permit.generation,
                RuntimeStateEntry {
                    generation: permit
                        .generation
                        .checked_add(1)
                        .ok_or(StateAccessError::InvalidTransition)?,
                    health,
                    probe_lease_until: None,
                    transient_backoff_step,
                },
                scope,
            ),
        )
        .await??;
    match outcome {
        CasOutcome::Applied { generation } if generation == permit.generation.saturating_add(1) => {
            Ok(())
        }
        CasOutcome::Applied { .. } => Err(StateAccessError::Conflict),
        CasOutcome::Conflict => Err(StateAccessError::Conflict),
    }
}

pub(crate) async fn validate_permit(
    store: &dyn RuntimeStateStore,
    key: &RuntimeStateKey,
    permit: StatePermit,
    scope: &ExecutionScope,
) -> Result<(), StateAccessError> {
    let current = scoped_read(store, key, scope).await?;
    if current.generation != permit.generation
        || (permit.probe
            && current
                .probe_lease_until
                .is_none_or(|until| until <= Instant::now()))
        || (!permit.probe
            && (current.health != RuntimeHealth::Active || current.probe_lease_until.is_some()))
    {
        return Err(StateAccessError::Conflict);
    }
    Ok(())
}

pub(crate) async fn validate_attempt_permits(
    store: &dyn RuntimeStateStore,
    permits: AttemptStatePermits<'_>,
    scope: &ExecutionScope,
) -> Result<(), StateAccessError> {
    validate_permit(store, permits.binding.0, permits.binding.1, scope).await?;
    if let Some((key, permit)) = permits.credential {
        validate_permit(store, key, permit, scope).await?;
    }
    Ok(())
}

pub(crate) async fn confirm_success(
    store: &dyn RuntimeStateStore,
    permits: AttemptStatePermits<'_>,
    scope: &ExecutionScope,
) -> Result<(), StateAccessError> {
    for (key, permit) in std::iter::once(permits.binding).chain(permits.credential) {
        validate_permit(store, key, permit, scope).await?;
        if !permit.probe {
            continue;
        }
        let outcome = scope
            .run(
                store.compare_and_swap(
                    key,
                    permit.generation,
                    RuntimeStateEntry {
                        generation: permit
                            .generation
                            .checked_add(1)
                            .ok_or(StateAccessError::InvalidTransition)?,
                        health: RuntimeHealth::Active,
                        probe_lease_until: None,
                        transient_backoff_step: 0,
                    },
                    scope,
                ),
            )
            .await??;
        match outcome {
            CasOutcome::Applied { generation }
                if generation == permit.generation.saturating_add(1) => {}
            CasOutcome::Applied { .. } | CasOutcome::Conflict => {
                return Err(StateAccessError::Conflict);
            }
        }
    }
    Ok(())
}

fn uses_transient_backoff(failure: &AttemptFailure) -> bool {
    use crate::attempt_outcome::{AttemptFailureClass, PreOutputStreamClass};

    matches!(
        failure.class,
        AttemptFailureClass::Transient
            | AttemptFailureClass::Timeout(_)
            | AttemptFailureClass::Disconnect
            | AttemptFailureClass::PreOutputStream(PreOutputStreamClass::Transient)
    )
}

fn transient_backoff_delay(step: u8) -> Duration {
    Duration::from_secs(match step {
        0 | 1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        _ => 30,
    })
}

async fn scoped_read(
    store: &dyn RuntimeStateStore,
    key: &RuntimeStateKey,
    scope: &ExecutionScope,
) -> Result<RuntimeStateEntry, StateAccessError> {
    Ok(scope.run(store.read(key, scope)).await??)
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum StateAccessError {
    #[error("runtime state operation exceeded the shared request scope")]
    Scope(#[from] ScopeError),
    #[error("runtime state authority failed")]
    Store(#[from] RuntimeStateError),
    #[error("runtime state generation changed during a required transition")]
    Conflict,
    #[error("runtime state transition is invalid")]
    InvalidTransition,
}
