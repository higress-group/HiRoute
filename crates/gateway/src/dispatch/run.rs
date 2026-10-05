//! Narrow daemon-to-Gateway authority for one exact delegated Worker run.
//!
//! Gateway owns compilation and request binding.  The daemon owns the token verifier, current
//! run safety projection, and the exact version it retained at admission.  Neither side receives
//! a general grant or a way to mutate the other side's authority.

use std::sync::Arc;

use hiroute_domain::WorkspaceId;
use thiserror::Error;

use crate::server::publication::RunPublicationHandle;
use crate::server::request_plan::IngressProtocol;
use crate::server::request_plan::VerifiedRunObservationContext;

use super::super::publication::{CompiledGrant, PublishedGatewayPublication};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRequestLocator {
    workspace_id: WorkspaceId,
    task_id: String,
    run_id: String,
    lease_id: String,
    exact_plan_reference: String,
    observation: RunObservationMetadata,
}

/// Exact Product identities retained when a delegated run is admitted. These
/// values carry no authority; Gateway may use them only after the run bearer,
/// current lease safety, protocol and alias have all been verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunObservationMetadata {
    plan_id: String,
    plan_revision: u64,
    publication_ref: String,
    harness_id: String,
    native_session_id: Option<String>,
    parent_context_ref: Option<String>,
    continued_from_run_id: Option<String>,
}

impl RunObservationMetadata {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plan_id: impl Into<String>,
        plan_revision: u64,
        publication_ref: impl Into<String>,
        harness_id: impl Into<String>,
        native_session_id: Option<String>,
        parent_context_ref: Option<String>,
        continued_from_run_id: Option<String>,
    ) -> Result<Self, RunRequestAuthorityError> {
        let value = Self {
            plan_id: plan_id.into(),
            plan_revision,
            publication_ref: publication_ref.into(),
            harness_id: harness_id.into(),
            native_session_id,
            parent_context_ref,
            continued_from_run_id,
        };
        if value.plan_revision == 0
            || !valid_observation_reference(&value.plan_id)
            || !valid_observation_reference(&value.publication_ref)
            || !matches!(
                value.harness_id.as_str(),
                "codex" | "claude" | "qoder" | "pi"
            )
            || [
                &value.native_session_id,
                &value.parent_context_ref,
                &value.continued_from_run_id,
            ]
            .into_iter()
            .flatten()
            .any(|value| !valid_observation_reference(value))
        {
            return Err(RunRequestAuthorityError::Denied);
        }
        Ok(value)
    }
}

impl RunRequestLocator {
    pub fn new(
        workspace_id: WorkspaceId,
        task_id: impl Into<String>,
        run_id: impl Into<String>,
        lease_id: impl Into<String>,
        exact_plan_reference: impl Into<String>,
        observation: RunObservationMetadata,
    ) -> Result<Self, RunRequestAuthorityError> {
        let value = Self {
            workspace_id,
            task_id: task_id.into(),
            run_id: run_id.into(),
            lease_id: lease_id.into(),
            exact_plan_reference: exact_plan_reference.into(),
            observation,
        };
        if [
            &value.task_id,
            &value.run_id,
            &value.lease_id,
            &value.exact_plan_reference,
        ]
        .iter()
        .any(|value| !valid_reference(value))
        {
            return Err(RunRequestAuthorityError::Denied);
        }
        Ok(value)
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub fn exact_plan_reference(&self) -> &str {
        &self.exact_plan_reference
    }

    fn observation_context(&self) -> VerifiedRunObservationContext {
        VerifiedRunObservationContext {
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            plan_id: self.observation.plan_id.clone(),
            plan_revision: self.observation.plan_revision,
            publication_ref: self.observation.publication_ref.clone(),
            harness_id: self.observation.harness_id.clone(),
            native_session_id: self.observation.native_session_id.clone(),
            parent_context_ref: self.observation.parent_context_ref.clone(),
            continued_from_run_id: self.observation.continued_from_run_id.clone(),
        }
    }
}

/// Daemon-owned request-time verification.  It receives the raw bearer only inside the request
/// path and is responsible for checking the high-entropy run credential plus current revocation
/// and lease state without consulting an ambient publication.
pub trait RunRequestSafetyPort: Send + Sync {
    fn authorize_request(
        &self,
        authorization: &str,
        locator: &RunRequestLocator,
        model_alias: &str,
        protocol: IngressProtocol,
    ) -> Result<(), RunRequestAuthorityError>;
}

/// The daemon resolves a run token to an immutable exact-plan handle.  Gateway never imports the
/// daemon, SQLite, or a current-publication lookup to satisfy this contract.
pub trait RunRequestAuthorityPort: Send + Sync {
    fn authenticate_run(
        &self,
        authorization: &str,
        protocol: IngressProtocol,
    ) -> Result<VerifiedRunRequestAuthority, RunRequestAuthorityError>;
}

#[derive(Clone)]
pub struct VerifiedRunRequestAuthority {
    locator: RunRequestLocator,
    publication: Arc<RunPublicationHandle>,
    safety: Arc<dyn RunRequestSafetyPort>,
    network_allowed: bool,
}

impl VerifiedRunRequestAuthority {
    pub fn new(
        locator: RunRequestLocator,
        publication: Arc<RunPublicationHandle>,
        safety: Arc<dyn RunRequestSafetyPort>,
        network_allowed: bool,
    ) -> Result<Self, RunRequestAuthorityError> {
        if publication.model_alias().is_empty() || locator.workspace_id().as_str().is_empty() {
            return Err(RunRequestAuthorityError::Denied);
        }
        Ok(Self {
            locator,
            publication,
            safety,
            network_allowed,
        })
    }

    pub fn locator(&self) -> &RunRequestLocator {
        &self.locator
    }

    pub(crate) fn network_allowed(&self) -> bool {
        self.network_allowed
    }

    pub(crate) fn observation_context(&self) -> VerifiedRunObservationContext {
        self.locator.observation_context()
    }

    pub(crate) fn begin(
        &self,
        authorization: &str,
        protocol: IngressProtocol,
    ) -> Result<(Arc<PublishedGatewayPublication>, CompiledGrant), RunRequestAuthorityError> {
        if protocol != self.publication.protocol() {
            return Err(RunRequestAuthorityError::Denied);
        }
        self.safety.authorize_request(
            authorization,
            &self.locator,
            self.publication.model_alias(),
            protocol,
        )?;
        let grant = self
            .publication
            .authenticate(authorization)
            .ok_or(RunRequestAuthorityError::Denied)?;
        if grant.protocol != protocol
            || grant.routes.len() != 1
            || !matches!(grant.routes.get(self.publication.model_alias()),
                Some(crate::server::publication::CompiledGrantRoute::Plan { alias })
                    if alias.as_ref() == self.publication.model_alias())
        {
            return Err(RunRequestAuthorityError::Denied);
        }
        Ok((self.publication.publication(), grant))
    }

    pub(crate) fn authorize_alias(
        &self,
        alias: &str,
        protocol: IngressProtocol,
    ) -> Result<(), RunRequestAuthorityError> {
        if protocol != self.publication.protocol() || alias != self.publication.model_alias() {
            return Err(RunRequestAuthorityError::Denied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RunRequestAuthorityError {
    #[error("delegated run request is denied")]
    Denied,
    #[error("delegated run authority is unavailable")]
    Unavailable,
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

fn valid_observation_reference(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hiroute_domain::CanonicalDigest;

    use super::*;
    use crate::server::dispatch::GatewayRequestAuthority;
    use crate::server::publication::{
        AliasPlanV1, GatewayPublicationInstaller, GatewayPublicationSnapshotV3, GrantV1,
        exact_test_candidate, token_sha256,
    };

    const TOKEN: &str = "hr_run_model_test-token";
    static DIRECTORY_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn empty_global() -> Arc<GatewayPublicationInstaller> {
        let path = std::env::temp_dir().join(format!(
            "hiroute-run-authority-{}-{}",
            std::process::id(),
            DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        Arc::new(GatewayPublicationInstaller::open(path).unwrap())
    }

    struct Safety(AtomicUsize);
    impl RunRequestSafetyPort for Safety {
        fn authorize_request(
            &self,
            authorization: &str,
            locator: &RunRequestLocator,
            model_alias: &str,
            protocol: IngressProtocol,
        ) -> Result<(), RunRequestAuthorityError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            if authorization == format!("Bearer {TOKEN}")
                && locator.run_id() == "run/one"
                && model_alias == "worker-model"
                && protocol == IngressProtocol::Responses
            {
                Ok(())
            } else {
                Err(RunRequestAuthorityError::Denied)
            }
        }
    }

    struct Authority(VerifiedRunRequestAuthority);
    impl RunRequestAuthorityPort for Authority {
        fn authenticate_run(
            &self,
            _: &str,
            _: IngressProtocol,
        ) -> Result<VerifiedRunRequestAuthority, RunRequestAuthorityError> {
            Ok(self.0.clone())
        }
    }

    fn observation_metadata(harness: &str) -> RunObservationMetadata {
        RunObservationMetadata::new(
            "plan/one",
            7,
            "publication/3/digest/sha256:abc",
            harness,
            Some("native/session-one".into()),
            Some("task/root".into()),
            Some("run/zero".into()),
        )
        .unwrap()
    }

    fn verified(safety: Arc<Safety>) -> VerifiedRunRequestAuthority {
        verified_for_harness(safety, "codex")
    }

    fn verified_for_harness(safety: Arc<Safety>, harness: &str) -> VerifiedRunRequestAuthority {
        let snapshot = GatewayPublicationSnapshotV3::seal(
            "workspace",
            "delegation-run/run-one",
            1,
            1,
            "renderer",
            vec![AliasPlanV1 {
                served_model_id: "worker-model".into(),
                purpose: "worker".into(),
                agent_plan_revision: 7,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 1_000,
                max_attempts: 1,
                routing: None,
                candidates: vec![exact_test_candidate(1, IngressProtocol::Responses, None)],
            }],
            vec![GrantV1 {
                grant_id: "delegation-run/run-one".into(),
                generation: 1,
                bearer_token_sha256: token_sha256(TOKEN),
                protocol: IngressProtocol::Responses,
                routes: [crate::server::publication::test_plan_route(
                    "worker-model",
                    7,
                )]
                .into(),
            }],
        )
        .unwrap();
        let publication = RunPublicationHandle::compile_exact(
            snapshot,
            "worker-model",
            IngressProtocol::Responses,
            &CanonicalDigest::of_bytes(TOKEN.as_bytes()),
        )
        .unwrap();
        VerifiedRunRequestAuthority::new(
            RunRequestLocator::new(
                WorkspaceId::parse("workspace").unwrap(),
                "task/one",
                "run/one",
                "lease/one",
                "plan/one/revision/7/digest/sha256:abc",
                observation_metadata(harness),
            )
            .unwrap(),
            publication,
            safety,
            false,
        )
        .unwrap()
    }

    #[test]
    fn unknown_native_harness_cannot_enter_verified_run_observation() {
        for harness in ["unknown", "", "qoder_cli", "Qoder"] {
            assert!(matches!(
                RunObservationMetadata::new(
                    "plan/one",
                    7,
                    "publication/one",
                    harness,
                    None,
                    None,
                    None
                ),
                Err(RunRequestAuthorityError::Denied)
            ));
        }
    }

    #[test]
    fn search_permission_is_derived_only_from_verified_run_network_access() {
        for allowed in [false, true] {
            let mut run = verified(Arc::new(Safety(AtomicUsize::new(0))));
            run.network_allowed = allowed;
            let authority = GatewayRequestAuthority::new(empty_global())
                .with_run_request_authority(Arc::new(Authority(run)));
            let request = authority
                .begin(
                    IngressProtocol::Responses,
                    Some("Bearer hr_run_model_test-token"),
                )
                .unwrap()
                .authorize_alias("worker-model", std::time::Instant::now())
                .unwrap();
            assert_eq!(request.web_search_allowed(), allowed);
        }
    }

    #[test]
    fn verified_run_emits_exact_relation_without_transport_supplied_identity() {
        use std::sync::mpsc;
        use std::time::Duration;

        use hiroute_domain::RunObservationLink;

        use crate::server::core_runtime::observation::{
            GatewayObservation, GatewayObservationSinks, ObservationAck, ObservationNack,
            ObservationRecord, ObservationRecordSink, OtelContentPolicy, accounted_acknowledgement,
        };

        struct Capture(mpsc::Sender<Vec<u8>>);
        impl ObservationRecordSink for Capture {
            fn deliver(
                &self,
                record: &ObservationRecord,
            ) -> Result<ObservationAck, ObservationNack> {
                self.0.send(record.payload().to_vec()).unwrap();
                Ok(accounted_acknowledgement(record))
            }
        }

        for harness in ["codex", "claude", "qoder"] {
            let authority = GatewayRequestAuthority::new(empty_global())
                .with_run_request_authority(Arc::new(Authority(verified_for_harness(
                    Arc::new(Safety(AtomicUsize::new(0))),
                    harness,
                ))));
            let request = authority
                .begin(IngressProtocol::Responses, Some(&format!("Bearer {TOKEN}")))
                .unwrap()
                .authorize_alias("worker-model", std::time::Instant::now())
                .unwrap();
            let (sender, receiver) = mpsc::channel();
            let mut sinks = GatewayObservationSinks::discard();
            sinks.run_relation = Arc::new(Capture(sender));
            let observation = GatewayObservation::with_sinks_and_policy_and_workspace_key(
                true,
                64 * 1024,
                sinks,
                OtelContentPolicy::Disabled,
                [7_u8; 32],
            );
            let _request_observation = observation.begin_request_with_content(
                &request,
                IngressProtocol::Responses,
                &crate::context_hold::ContextIdentityFacts::request_scoped(),
                true,
            );
            let link: RunObservationLink =
                serde_json::from_slice(&receiver.recv_timeout(Duration::from_secs(1)).unwrap())
                    .unwrap();
            assert_eq!(link.workspace_id.as_str(), "workspace");
            assert_eq!(link.task_id, "task/one");
            assert_eq!(link.run_id, "run/one");
            assert_eq!(link.plan_id, "plan/one");
            assert_eq!(link.plan_revision, "7");
            assert_eq!(link.publication_ref, "publication/3/digest/sha256:abc");
            assert_eq!(link.harness_id, harness);
            assert_eq!(link.protocol_kind, "responses");
            assert_eq!(
                link.native_session_id.as_deref(),
                Some("native/session-one")
            );
            assert_eq!(link.parent_context_ref.as_deref(), Some("task/root"));
            assert_eq!(link.continued_from_run_id.as_deref(), Some("run/zero"));
            assert!(!link.producer_epoch.is_empty());
            assert!(link.source_event_id.starts_with("relation-event-"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn production_listener_denies_injected_search_before_credential_resolution() {
        use crate::server::composition::{
            CredentialLease, CredentialResolver, PortError, ProductionPorts,
        };
        use std::io::{Read, Write};
        use std::os::unix::fs::DirBuilderExt;
        const CHILD: &str = "HIROUTE_SEARCH_DENIAL_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            struct OwnedPrivateDirectory(std::path::PathBuf);
            impl Drop for OwnedPrivateDirectory {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let root = OwnedPrivateDirectory(std::env::temp_dir().join(format!(
                "hiroute-search-denial-{}-{}",
                std::process::id(),
                DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            )));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root.0)
                .unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "server::dispatch::run::tests::production_listener_denies_injected_search_before_credential_resolution", "--nocapture"])
                .env(CHILD, "1").env("HIROUTE_REPLAY_ROOT", root.0.join("replay"))
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        struct Credentials(AtomicUsize);
        #[async_trait::async_trait]
        impl CredentialResolver for Credentials {
            fn acquire(&self, _: &str) -> Result<CredentialLease, PortError> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(PortError::Rejected)
            }
            async fn lease_exact(
                &self,
                _: crate::ports::CredentialLeaseRequest<'_>,
                _: &crate::ports::ExecutionScope,
            ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(PortError::Rejected)
            }
        }
        let credentials = Arc::new(Credentials(AtomicUsize::new(0)));
        let mut ports = ProductionPorts::fail_closed(empty_global());
        ports.credentials = credentials.clone();
        let runtime = crate::server::core_runtime::ProductionGatewayRuntime::compose(ports)
            .with_run_request_authority(Arc::new(Authority(verified(Arc::new(Safety(
                AtomicUsize::new(0),
            ))))));
        let address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut gateway = crate::server::GatewayLauncher::from_runtime(address, Arc::new(runtime))
            .unwrap()
            .start_managed()
            .unwrap();
        let body = r#"{"model":"worker-model","input":"injected search","tools":[{"type":"web_search","external_web_access":true}]}"#;
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(stream, "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        stream
            .take(32 * 1024)
            .read_to_string(&mut response)
            .unwrap();
        gateway.shutdown();
        gateway.join(std::time::Duration::from_secs(5)).unwrap();
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(response.contains("WORKER_NETWORK_DENIED"), "{response}");
        assert_eq!(credentials.0.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn exact_run_token_uses_its_own_pinned_publication_without_a_global_publication() {
        let safety = Arc::new(Safety(AtomicUsize::new(0)));
        let authority = GatewayRequestAuthority::new(empty_global())
            .with_run_request_authority(Arc::new(Authority(verified(Arc::clone(&safety)))));

        let request = authority
            .begin(
                IngressProtocol::Responses,
                Some("Bearer hr_run_model_test-token"),
            )
            .unwrap()
            .authorize_alias("worker-model", std::time::Instant::now())
            .unwrap();
        assert_eq!(request.served_model_id(), "worker-model");
        assert_eq!(safety.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn run_authority_never_falls_back_to_a_different_alias_or_token() {
        let authority = GatewayRequestAuthority::new(empty_global()).with_run_request_authority(
            Arc::new(Authority(verified(Arc::new(Safety(AtomicUsize::new(0)))))),
        );

        let wrong_token = authority.begin(
            IngressProtocol::Responses,
            Some("Bearer hr_run_model_other-token"),
        );
        assert!(matches!(
            wrong_token,
            Err(super::super::DispatchError::Unauthorized)
        ));
        let wrong_alias = authority
            .begin(
                IngressProtocol::Responses,
                Some("Bearer hr_run_model_test-token"),
            )
            .unwrap()
            .authorize_alias("ordinary-model", std::time::Instant::now());
        assert!(matches!(
            wrong_alias,
            Err(super::super::DispatchError::Unauthorized)
        ));
    }
}
