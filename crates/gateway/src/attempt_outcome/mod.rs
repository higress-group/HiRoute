//! Trusted Provider failure classification used by the exact-target runtime.
//!
//! HTTP status alone is sufficient only where the protocol contract is
//! unambiguous. In particular, a 429 must carry the Connector-classified
//! quota or binding-overload scope; an unclassified 429 can relay without guessing a quarantine scope.

use std::time::Duration;

/// The subset of a Connector error profile that the runtime is allowed to
/// consume. It contains no endpoint, header value, credential, or payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectorErrorProfile {
    pub http_status_typed: bool,
    pub stream_error_typed: bool,
    pub retry_after_typed: bool,
}

impl ConnectorErrorProfile {
    pub const fn exact() -> Self {
        Self {
            http_status_typed: true,
            stream_error_typed: true,
            retry_after_typed: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFailureKind {
    ReasoningHistory,
    Credential,
    Quota,
    BindingOverload,
    Protocol,
    PermanentClient,
    Transient,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptTimeoutPhase {
    Connect,
    RequestWrite,
    FirstByte,
    StreamIdle,
    Overall,
}

/// Sanitized failure facts emitted by the trusted Connector adapter. Raw
/// Provider bodies and headers never cross this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawAttemptFailure {
    Connect,
    Timeout {
        phase: AttemptTimeoutPhase,
    },
    Disconnect,
    Http {
        status: u16,
        kind: Option<ProviderFailureKind>,
        retry_after: Option<Duration>,
    },
    Stream {
        kind: Option<ProviderFailureKind>,
        retry_after: Option<Duration>,
    },
    Protocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreOutputStreamClass {
    Credential,
    Quota,
    BindingOverload,
    Protocol,
    PermanentClient,
    Transient,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptFailureClass {
    ReasoningHistory,
    Credential,
    Quota,
    BindingOverload,
    Protocol,
    PermanentClient,
    Transient,
    Timeout(AttemptTimeoutPhase),
    Disconnect,
    PreOutputStream(PreOutputStreamClass),
    PostCommit,
    Unclassified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptFailure {
    pub class: AttemptFailureClass,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
}

impl AttemptFailure {
    pub fn postcommit() -> Self {
        Self {
            class: AttemptFailureClass::PostCommit,
            status: None,
            retry_after: None,
        }
    }

    /// Whether a failure may select another frozen target while the
    /// downstream response body commit fence is still clear.
    pub fn is_precommit_relayable(&self) -> bool {
        // Cause classification controls quarantine and key rotation, not whether
        // another already-authorized candidate can answer before body delivery.
        !matches!(self.class, AttemptFailureClass::PostCommit)
    }

    /// Credential and quota failures may stay on the same frozen binding and
    /// request the next exact key. Every other relay advances to the next
    /// frozen candidate.
    pub fn permits_next_key(&self) -> bool {
        matches!(
            self.class,
            AttemptFailureClass::Credential
                | AttemptFailureClass::Quota
                | AttemptFailureClass::PreOutputStream(
                    PreOutputStreamClass::Credential | PreOutputStreamClass::Quota
                )
        )
    }

    pub fn state_scope(&self) -> Option<FailureStateScope> {
        match self.class {
            AttemptFailureClass::Credential
            | AttemptFailureClass::Quota
            | AttemptFailureClass::PreOutputStream(
                PreOutputStreamClass::Credential | PreOutputStreamClass::Quota,
            ) => Some(FailureStateScope::Credential),
            AttemptFailureClass::BindingOverload
            | AttemptFailureClass::Transient
            | AttemptFailureClass::Timeout(_)
            | AttemptFailureClass::Disconnect
            | AttemptFailureClass::PreOutputStream(
                PreOutputStreamClass::BindingOverload | PreOutputStreamClass::Transient,
            ) => Some(FailureStateScope::Binding),
            AttemptFailureClass::Protocol
            | AttemptFailureClass::ReasoningHistory
            | AttemptFailureClass::PermanentClient
            | AttemptFailureClass::PreOutputStream(
                PreOutputStreamClass::Protocol
                | PreOutputStreamClass::PermanentClient
                | PreOutputStreamClass::Unclassified,
            )
            | AttemptFailureClass::PostCommit
            | AttemptFailureClass::Unclassified => None,
        }
    }

    pub fn disables_credential(&self) -> bool {
        matches!(
            self.class,
            AttemptFailureClass::Credential
                | AttemptFailureClass::PreOutputStream(PreOutputStreamClass::Credential)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureStateScope {
    Credential,
    Binding,
}

pub fn classify_failure(
    failure: &RawAttemptFailure,
    profile: ConnectorErrorProfile,
) -> AttemptFailure {
    match failure {
        RawAttemptFailure::Connect => simple(AttemptFailureClass::Transient),
        RawAttemptFailure::Timeout { phase } => simple(AttemptFailureClass::Timeout(*phase)),
        RawAttemptFailure::Disconnect => simple(AttemptFailureClass::Disconnect),
        RawAttemptFailure::Protocol => simple(AttemptFailureClass::Protocol),
        RawAttemptFailure::Http {
            status,
            kind,
            retry_after,
        } => classify_http(*status, *kind, *retry_after, profile),
        RawAttemptFailure::Stream { kind, retry_after } => {
            let class = if profile.stream_error_typed {
                kind.map_or(PreOutputStreamClass::Unclassified, stream_class)
            } else {
                PreOutputStreamClass::Unclassified
            };
            AttemptFailure {
                class: AttemptFailureClass::PreOutputStream(class),
                status: None,
                retry_after: profile.retry_after_typed.then_some(*retry_after).flatten(),
            }
        }
    }
}

fn classify_http(
    status: u16,
    kind: Option<ProviderFailureKind>,
    retry_after: Option<Duration>,
    profile: ConnectorErrorProfile,
) -> AttemptFailure {
    if !profile.http_status_typed {
        return AttemptFailure {
            class: AttemptFailureClass::Unclassified,
            status: Some(status),
            retry_after: None,
        };
    }
    let class = match status {
        401 | 403 => AttemptFailureClass::Credential,
        429 => match kind {
            Some(ProviderFailureKind::Quota) => AttemptFailureClass::Quota,
            Some(ProviderFailureKind::BindingOverload) => AttemptFailureClass::BindingOverload,
            _ => AttemptFailureClass::Unclassified,
        },
        400 | 404 | 422 => match kind {
            Some(ProviderFailureKind::ReasoningHistory) if status == 400 => {
                AttemptFailureClass::ReasoningHistory
            }
            Some(ProviderFailureKind::Protocol) => AttemptFailureClass::Protocol,
            Some(ProviderFailureKind::PermanentClient) => AttemptFailureClass::PermanentClient,
            _ => AttemptFailureClass::Unclassified,
        },
        500..=599 => AttemptFailureClass::Transient,
        _ => match kind {
            Some(ProviderFailureKind::ReasoningHistory) => AttemptFailureClass::Unclassified,
            Some(ProviderFailureKind::Protocol) => AttemptFailureClass::Protocol,
            Some(ProviderFailureKind::PermanentClient) => AttemptFailureClass::PermanentClient,
            Some(ProviderFailureKind::Transient) => AttemptFailureClass::Transient,
            Some(ProviderFailureKind::Credential) => AttemptFailureClass::Credential,
            Some(ProviderFailureKind::Quota | ProviderFailureKind::BindingOverload) | None => {
                AttemptFailureClass::Unclassified
            }
        },
    };
    AttemptFailure {
        class,
        status: Some(status),
        retry_after: profile.retry_after_typed.then_some(retry_after).flatten(),
    }
}

fn stream_class(kind: ProviderFailureKind) -> PreOutputStreamClass {
    match kind {
        ProviderFailureKind::ReasoningHistory => PreOutputStreamClass::PermanentClient,
        ProviderFailureKind::Credential => PreOutputStreamClass::Credential,
        ProviderFailureKind::Quota => PreOutputStreamClass::Quota,
        ProviderFailureKind::BindingOverload => PreOutputStreamClass::BindingOverload,
        ProviderFailureKind::Protocol => PreOutputStreamClass::Protocol,
        ProviderFailureKind::PermanentClient => PreOutputStreamClass::PermanentClient,
        ProviderFailureKind::Transient => PreOutputStreamClass::Transient,
    }
}

fn simple(class: AttemptFailureClass) -> AttemptFailure {
    AttemptFailure {
        class,
        status: None,
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_error_classes_keep_quota_and_overload_separate() {
        let profile = ConnectorErrorProfile::exact();
        let quota = classify_failure(
            &RawAttemptFailure::Http {
                status: 429,
                kind: Some(ProviderFailureKind::Quota),
                retry_after: None,
            },
            profile,
        );
        let overload = classify_failure(
            &RawAttemptFailure::Http {
                status: 429,
                kind: Some(ProviderFailureKind::BindingOverload),
                retry_after: None,
            },
            profile,
        );
        assert_eq!(quota.class, AttemptFailureClass::Quota);
        assert_eq!(quota.state_scope(), Some(FailureStateScope::Credential));
        assert_eq!(overload.class, AttemptFailureClass::BindingOverload);
        assert_eq!(overload.state_scope(), Some(FailureStateScope::Binding));
    }

    #[test]
    fn unclassified_429_can_relay_without_guessing_failure_scope() {
        let failure = classify_failure(
            &RawAttemptFailure::Http {
                status: 429,
                kind: None,
                retry_after: None,
            },
            ConnectorErrorProfile::exact(),
        );
        assert_eq!(failure.class, AttemptFailureClass::Unclassified);
        assert!(failure.is_precommit_relayable());
        assert_eq!(failure.state_scope(), None);
    }

    #[test]
    fn upstream_client_errors_can_relay_without_quarantining() {
        for status in [400, 404, 422] {
            let failure = classify_failure(
                &RawAttemptFailure::Http {
                    status,
                    kind: Some(ProviderFailureKind::PermanentClient),
                    retry_after: None,
                },
                ConnectorErrorProfile::exact(),
            );
            assert_eq!(failure.class, AttemptFailureClass::PermanentClient);
            assert!(failure.is_precommit_relayable());
            assert_eq!(failure.state_scope(), None);
        }
    }

    #[test]
    fn ambiguous_and_typed_client_errors_can_relay() {
        let protocol = classify_failure(
            &RawAttemptFailure::Http {
                status: 404,
                kind: Some(ProviderFailureKind::Protocol),
                retry_after: None,
            },
            ConnectorErrorProfile::exact(),
        );
        assert_eq!(protocol.class, AttemptFailureClass::Protocol);
        assert!(protocol.is_precommit_relayable());

        let ambiguous = classify_failure(
            &RawAttemptFailure::Http {
                status: 422,
                kind: None,
                retry_after: None,
            },
            ConnectorErrorProfile::exact(),
        );
        assert_eq!(ambiguous.class, AttemptFailureClass::Unclassified);
        assert!(ambiguous.is_precommit_relayable());
        assert_eq!(ambiguous.state_scope(), None);
    }

    #[test]
    fn credential_statuses_and_transient_failures_keep_distinct_state_scope() {
        let profile = ConnectorErrorProfile::exact();
        for status in [401, 403] {
            let failure = classify_failure(
                &RawAttemptFailure::Http {
                    status,
                    kind: None,
                    retry_after: None,
                },
                profile,
            );
            assert_eq!(failure.class, AttemptFailureClass::Credential);
            assert_eq!(failure.state_scope(), Some(FailureStateScope::Credential));
        }
        let server = classify_failure(
            &RawAttemptFailure::Http {
                status: 503,
                kind: None,
                retry_after: None,
            },
            profile,
        );
        assert_eq!(server.class, AttemptFailureClass::Transient);
        assert_eq!(server.state_scope(), Some(FailureStateScope::Binding));
        let timeout = classify_failure(
            &RawAttemptFailure::Timeout {
                phase: AttemptTimeoutPhase::FirstByte,
            },
            profile,
        );
        assert_eq!(
            timeout.class,
            AttemptFailureClass::Timeout(AttemptTimeoutPhase::FirstByte)
        );
    }

    #[test]
    fn preoutput_stream_error_remains_a_distinct_typed_class() {
        let failure = classify_failure(
            &RawAttemptFailure::Stream {
                kind: Some(ProviderFailureKind::BindingOverload),
                retry_after: Some(Duration::from_secs(2)),
            },
            ConnectorErrorProfile::exact(),
        );
        assert_eq!(
            failure.class,
            AttemptFailureClass::PreOutputStream(PreOutputStreamClass::BindingOverload)
        );
        assert!(failure.is_precommit_relayable());
        assert_eq!(failure.state_scope(), Some(FailureStateScope::Binding));
    }
}
