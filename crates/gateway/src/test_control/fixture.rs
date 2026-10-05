//! Validated Native HTTPS fixtures available only with E2E test control.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hiroute_domain::{
    CanonicalDigest, ConnectorRuntimeKind, GatewayCandidateProtocolProfileV1,
    GatewayOperationalTargetV1,
};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde::Serialize;

use super::super::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
use super::super::publication::{
    AliasPlanV1, CandidateBindingV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use super::super::request_plan::IngressProtocol;

pub const E2E_DIAL_CONFIG_ENV: &str = "HIROUTE_E2E_DIAL_CONFIG";
pub const E2E_DIAL_CONFIG_FILE: &str = "gateway-e2e-dial.json";

pub fn sealed_native_candidate(
    local_id: u32,
    stable_target_key: &str,
    credential_refs: &[String],
    authority: &str,
    native_model: &str,
    protocol_pairs: &[(IngressProtocol, IngressProtocol)],
) -> CandidateBindingV1 {
    assert!(local_id > 0);
    assert!(!stable_target_key.is_empty());
    assert!(!credential_refs.is_empty());
    assert!(!protocol_pairs.is_empty());
    let request_path = protocol_pairs[0].1.path();
    assert!(
        protocol_pairs
            .iter()
            .all(|(_, upstream)| upstream.path() == request_path),
        "one sealed operational target has exactly one request path"
    );
    let protocol_profiles = protocol_pairs
        .iter()
        .map(|(ingress, upstream)| {
            let mut profile = CandidateProtocolProfile::exact_portable_path(
                *ingress,
                *upstream,
                native_model,
                fixed_reasoning("fixed"),
            );
            profile.connector.connector_id = "builtin-native-e2e".into();
            profile.connector.connector_revision = "1".into();
            serde_json::from_value::<GatewayCandidateProtocolProfileV1>(
                serde_json::to_value(profile).expect("Gateway profile serializes"),
            )
            .expect("Gateway and Product profile DTOs are exact")
        })
        .collect::<Vec<_>>();
    let endpoint = format!("https://{authority}{request_path}");
    let operational_target = GatewayOperationalTargetV1::RegisteredHttps {
        uri: endpoint.clone(),
    };
    let candidate = CandidateBindingV1 {
        local_id,
        stable_target_key: stable_target_key.into(),
        adapter_id: "adapter.native-e2e@1".into(),
        credential_refs: credential_refs.to_vec(),
        credential_destination_ref: format!("connection-option/{stable_target_key}"),
        upstream_model_id: native_model.into(),
        native_transport_model: native_model.into(),
        endpoint,
        connector_runtime: ConnectorRuntimeKind::BuiltinNative,
        operational_target_digest: CanonicalDigest::of(&operational_target).unwrap(),
        operational_target,
        protocol_profile_digest: CanonicalDigest::of(&protocol_profiles).unwrap(),
        protocol_profiles,
        pricing_identity: None,
    };
    let snapshot = GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "fixture-validation",
        1,
        1,
        "fixture-validation/1",
        vec![AliasPlanV1 {
            served_model_id: "fixture-validation".into(),
            purpose: "validate one exact E2E candidate".into(),
            agent_plan_revision: 1,
            protocols: protocol_pairs.iter().map(|(ingress, _)| *ingress).collect(),
            overall_timeout_ms: 1_000,
            max_attempts: 1,
            routing: None,
            candidates: vec![candidate.clone()],
        }],
        protocol_pairs
            .iter()
            .enumerate()
            .map(|(index, (protocol, _))| GrantV1 {
                route_protocols: Default::default(),
                grant_id: format!("fixture-validation-{index}"),
                generation: 1,
                bearer_token_sha256: token_sha256(&format!("fixture-validation-{index}")),
                protocol: *protocol,
                routes: [(
                    "fixture-validation".into(),
                    crate::server::publication::ModelRouteV2::Plan {
                        plan_id: "legacy/fixture-validation".into(),
                        alias: "fixture-validation".into(),
                        revision: 1,
                        semantic_digest: CanonicalDigest::of_bytes(b"fixture-validation"),
                    },
                )]
                .into(),
            })
            .collect(),
    )
    .expect("E2E candidate is production-valid");
    assert_eq!(snapshot.aliases[0].candidates[0], candidate);
    candidate
}

#[derive(Clone)]
pub struct TestTlsListener {
    listener: Arc<TcpListener>,
    config: Arc<ServerConfig>,
    authority: Arc<str>,
    ca_pem: Arc<str>,
}

impl TestTlsListener {
    pub fn bind(authority: impl Into<Arc<str>>) -> Result<Self, Box<dyn std::error::Error>> {
        let authority = authority.into();
        let certified = generate_simple_self_signed(vec![authority.to_string()])?;
        let certificate: CertificateDer<'static> = certified.cert.der().clone();
        let private_key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            listener: Arc::new(TcpListener::bind("127.0.0.1:0")?),
            config: Arc::new(config),
            authority,
            ca_pem: Arc::from(certified.cert.pem()),
        })
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn set_nonblocking(&self, value: bool) -> std::io::Result<()> {
        self.listener.set_nonblocking(value)
    }

    pub fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            listener: Arc::new(self.listener.try_clone()?),
            config: Arc::clone(&self.config),
            authority: Arc::clone(&self.authority),
            ca_pem: Arc::clone(&self.ca_pem),
        })
    }

    pub fn accept(&self) -> std::io::Result<(TestTlsStream, SocketAddr)> {
        let (stream, address) = self.listener.accept()?;
        let connection =
            ServerConnection::new(Arc::clone(&self.config)).map_err(std::io::Error::other)?;
        Ok((TestTlsStream(StreamOwned::new(connection, stream)), address))
    }

    fn dial_target(&self) -> std::io::Result<DialConfigTarget> {
        Ok(DialConfigTarget {
            authority: self.authority.to_string(),
            address: self.local_addr()?,
            ca_pem: self.ca_pem.to_string(),
        })
    }
}

pub struct TestTlsStream(StreamOwned<ServerConnection, TcpStream>);

impl TestTlsStream {
    pub fn set_nonblocking(&self, value: bool) -> std::io::Result<()> {
        self.0.sock.set_nonblocking(value)
    }

    pub fn set_read_timeout(&self, value: Option<std::time::Duration>) -> std::io::Result<()> {
        self.0.sock.set_read_timeout(value)
    }

    /// Flushes application data and emits TLS `close_notify` before closing the write half.
    pub fn finish(&mut self) -> std::io::Result<()> {
        self.flush()?;
        self.0.conn.send_close_notify();
        while self.0.conn.wants_write() {
            self.0.conn.write_tls(&mut self.0.sock)?;
        }
        self.0.sock.shutdown(Shutdown::Write)
    }
}

impl Read for TestTlsStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for TestTlsStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[derive(Serialize)]
struct DialConfig<'a> {
    schema_version: &'static str,
    targets: &'a [DialConfigTarget],
    dns_failures: &'a [&'a str],
}

#[derive(Serialize)]
struct DialConfigTarget {
    authority: String,
    address: SocketAddr,
    ca_pem: String,
}

pub fn write_dial_config(
    directory: &Path,
    providers: &[&TestTlsListener],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    write_dial_config_with_dns_failures(directory, providers, &[])
}

pub fn write_dial_config_with_dns_failures(
    directory: &Path,
    providers: &[&TestTlsListener],
    dns_failures: &[&str],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let targets = providers
        .iter()
        .map(|provider| provider.dial_target())
        .collect::<Result<Vec<_>, _>>()?;
    let path = directory.join(E2E_DIAL_CONFIG_FILE);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&DialConfig {
            schema_version: "hiroute.gateway.e2e-dial-map/v1",
            targets: &targets,
            dns_failures,
        })?,
    )?;
    Ok(path)
}
