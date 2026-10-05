//! Current production initialization. Legacy sealed inputs remain in runtime.rs.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use super::types::ProductionError;
use crate::gateway_fixture::{
    TestTlsListener, read_complete_http_request, sealed_native_candidate, write_dial_config,
};
use crate::p0::privacy::private_write;
use hiroute_gateway::server::publication::{AliasPlanV1, GatewayPublicationSnapshotV3, GrantV1};
use hiroute_gateway::server::request_plan::IngressProtocol;
use sha2::{Digest, Sha256};

const CURRENT_PUBLICATION_SCHEMA: &str = "hiroute.gateway.publication-snapshot/v3";

/// TLS terminates only at the controlled upstream. HTTP bytes pass unchanged to the
/// existing NativeProvider, which independently records the actual request and reply.
pub(super) struct CurrentUpstream {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<std::io::Result<()>>>,
}
impl CurrentUpstream {
    pub(super) fn start(
        root: &Path,
        ledger: SocketAddr,
        client_digest: &str,
    ) -> Result<(Self, String), ProductionError> {
        let listener = TestTlsListener::bind("oracle-native-provider.invalid")
            .map_err(|_| ProductionError::Process("controlled TLS initialization failed".into()))?;
        listener.set_nonblocking(true)?;
        let inputs = root.join("inputs");
        let config = write_dial_config(&inputs, &[&listener]).map_err(|_| {
            ProductionError::Process("controlled dial map initialization failed".into())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o600))?;
        }
        let snapshot = GatewayPublicationSnapshotV3::seal(
            "workspace-oracle",
            "oracle-authority",
            1,
            22_012,
            "current-production-oracle/v2",
            vec![AliasPlanV1 {
                served_model_id: "oracle-smoke".into(),
                purpose: "controlled production smoke".into(),
                agent_plan_revision: 22_012,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 5_000,
                max_attempts: 1,
                routing: None,
                candidates: vec![sealed_native_candidate(
                    1,
                    "oracle-native-provider",
                    &["oracle-credential".into()],
                    listener.authority(),
                    "oracle-native-provider",
                    &[(IngressProtocol::Responses, IngressProtocol::Responses)],
                )],
            }],
            vec![GrantV1 {
                route_protocols: Default::default(),
                grant_id: "oracle-grant".into(),
                generation: 1,
                bearer_token_sha256: client_digest.into(),
                protocol: IngressProtocol::Responses,
                routes: [(
                    "oracle-smoke".into(),
                    serde_json::from_value(serde_json::json!({
                        "kind": "plan",
                        "plan_id": "legacy/oracle-smoke",
                        "alias": "oracle-smoke",
                        "revision": 22_012,
                        "semantic_digest": format!("sha256:{:x}", Sha256::digest(b"oracle-smoke")),
                    }))?,
                )]
                .into(),
            }],
        )
        .map_err(|_| {
            ProductionError::Contract("current publication initialization failed".into())
        })?;
        if snapshot.schema_version != CURRENT_PUBLICATION_SCHEMA {
            return Err(ProductionError::Contract(
                "current publication schema drifted".into(),
            ));
        }
        // Replace only the just-created private initial input; never a product operation.
        std::fs::remove_file(inputs.join("publication.json"))?;
        private_write(
            &inputs.join("publication.json"),
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while !cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut tls, _)) => {
                        tls.set_nonblocking(false)?;
                        let request = read_complete_http_request(&mut tls, Duration::from_secs(5))?;
                        if request.len() > 4 * 1024 * 1024 {
                            return Err(std::io::Error::other("controlled request too large"));
                        }
                        let mut upstream =
                            TcpStream::connect_timeout(&ledger, Duration::from_secs(5))?;
                        upstream.set_read_timeout(Some(Duration::from_secs(5)))?;
                        upstream.set_write_timeout(Some(Duration::from_secs(5)))?;
                        upstream.write_all(&request)?;
                        let mut response = Vec::new();
                        upstream
                            .take(4 * 1024 * 1024 + 1)
                            .read_to_end(&mut response)?;
                        if response.len() > 4 * 1024 * 1024 {
                            return Err(std::io::Error::other("controlled response too large"));
                        }
                        tls.write_all(&response)?;
                        tls.flush()?;
                        tls.finish()?;
                        return Ok(());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(std::io::Error::other(
                "controlled TLS request not completed",
            ))
        });
        Ok((
            Self {
                stop,
                worker: Some(worker),
            },
            snapshot.payload_digest,
        ))
    }
    pub(super) fn finish(&mut self) -> Result<(), ProductionError> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker
            .take()
            .expect("one TLS worker")
            .join()
            .map_err(|_| ProductionError::Process("controlled TLS worker panicked".into()))?
            .map_err(|_| ProductionError::Process("controlled TLS request failed".into()))
    }
}
impl Drop for CurrentUpstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
