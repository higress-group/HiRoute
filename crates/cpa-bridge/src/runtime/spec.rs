use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use hiroute_domain::{AuthenticationKind, BillingClass, ConnectionOrigin, ConnectorRuntimeKind};
use hiroute_integrations::TrustedReleaseCatalog;

use crate::CpaProfileBinding;
use crate::borrowed_codex::BorrowedCodexAuthSpec;
use crate::errors::CpaLifecycleError;

const MAX_BOUND: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct CpaRuntimeSpec {
    pub instance_id: String,
    pub state_root: PathBuf,
    pub auth_dir: PathBuf,
    /// Owner-controlled Codex CLI token source borrowed as an access-only lease.
    ///
    /// A Codex binding requires this source. The runtime never imports its refresh token.
    pub borrowed_codex_auth: Option<BorrowedCodexAuthSpec>,
    pub borrowed_claude_auth: Option<crate::BorrowedClaudeAuthSpec>,
    pub bindings: Vec<CpaProfileBinding>,
    pub startup_timeout: Duration,
    pub control_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub restart_policy: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartPolicy {
    pub max_restarts: u8,
    pub window: Duration,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window: Duration::from_secs(60),
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        }
    }
}

pub(super) fn validate_spec(
    spec: &CpaRuntimeSpec,
    catalog: &TrustedReleaseCatalog,
) -> Result<(), CpaLifecycleError> {
    if !valid_id(&spec.instance_id)
        || !spec.state_root.is_absolute()
        || !spec.auth_dir.is_absolute()
        || spec.bindings.is_empty()
        || spec.bindings.len() > 2
        || spec.startup_timeout.is_zero()
        || spec.startup_timeout > MAX_BOUND
        || spec.control_timeout.is_zero()
        || spec.control_timeout > MAX_BOUND
        || spec.shutdown_timeout.is_zero()
        || spec.shutdown_timeout > MAX_BOUND
        || spec.restart_policy.max_restarts == 0
        || spec.restart_policy.window.is_zero()
        || spec.restart_policy.window > MAX_BOUND
        || spec.restart_policy.max_backoff > MAX_BOUND
        || spec.restart_policy.base_backoff > spec.restart_policy.max_backoff
    {
        return Err(CpaLifecycleError::InvalidSpec);
    }
    if spec
        .borrowed_codex_auth
        .as_ref()
        .is_some_and(|source| !source.source_path().is_absolute())
    {
        return Err(CpaLifecycleError::InvalidSpec);
    }
    if spec
        .borrowed_claude_auth
        .as_ref()
        .is_some_and(|source| !source.source_path().is_absolute())
        || (spec.borrowed_codex_auth.is_some() && spec.borrowed_claude_auth.is_some())
    {
        return Err(CpaLifecycleError::InvalidSpec);
    }
    let mut kinds = BTreeSet::new();
    for binding in &spec.bindings {
        if !kinds.insert(binding.account_kind) {
            return Err(CpaLifecycleError::InvalidBinding);
        }
        let resolved = catalog
            .resolve_connection_option(&binding.connection_option_id)
            .map_err(|_| CpaLifecycleError::InvalidBinding)?;
        if resolved.connector.connector_id != binding.connector_id
            || resolved.connector.runtime_kind != ConnectorRuntimeKind::CpaBridge
            || resolved.connector.authentication != AuthenticationKind::ConnectorOwnedOpaque
            || resolved.option.origin != ConnectionOrigin::AgentSubscription
            || resolved.option.billing_class != BillingClass::Subscription
            || resolved.endpoint_profile.endpoint_profile_id != binding.endpoint_profile_id
            || !resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .any(|endpoint| endpoint.protocol == binding.account_kind.required_protocol())
        {
            return Err(CpaLifecycleError::InvalidBinding);
        }
    }
    if (spec.borrowed_claude_auth.is_some() && !kinds.contains(&crate::CpaAccountKind::Claude))
        || kinds.contains(&crate::CpaAccountKind::Codex) != spec.borrowed_codex_auth.is_some()
    {
        return Err(CpaLifecycleError::InvalidSpec);
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
