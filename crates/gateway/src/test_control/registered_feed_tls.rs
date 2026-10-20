use hiroute_gateway_core::core::execution_plan::{CaPolicy, TransportScheme};
use hiroute_gateway_core::test_support::plain_target;
use hiroute_gateway_core::transport::pingora::PingoraConnectorAdapter;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
type TestError = Box<dyn std::error::Error + Send + Sync>;

struct TlsRoot(std::path::PathBuf);
impl TlsRoot {
    fn new() -> Result<Self, TestError> {
        use std::os::unix::fs::DirBuilderExt;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("hiroute-feed-tls-{}-{nonce}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TlsRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ProviderProcess(Child);

impl Drop for ProviderProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn registered_feed_openssl_fixture_accepts_production_tls_connector() -> Result<(), TestError>
{
    let root = TlsRoot::new()?;
    let support = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../daemon/tests/support")
        .canonicalize()?;
    let script = r#"
import json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from model_feed_product import FeedUpstream
upstream = FeedUpstream(Path(sys.argv[2]))
try:
    print(json.dumps({'port': upstream.server.server_port}), flush=True)
    sys.stdin.read()
finally:
    upstream.close()
"#;
    let mut provider = ProviderProcess(
        Command::new("python3")
            .args(["-B", "-u", "-c", script])
            .arg(support)
            .arg(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let mut ready = String::new();
    BufReader::new(provider.0.stdout.take().ok_or("provider stdout missing")?)
        .read_line(&mut ready)?;
    let port = serde_json::from_str::<serde_json::Value>(&ready)?["port"]
        .as_u64()
        .ok_or("provider port missing")?;
    let address = format!("127.0.0.1:{port}").parse()?;
    let mut target = plain_target(address, 74);
    target.scheme = TransportScheme::Https;
    target.authority = Arc::from("dashscope.aliyuncs.com");
    target.sni = Some(Arc::from("dashscope.aliyuncs.com"));
    target.ca = CaPolicy::Pem(Arc::from(std::fs::read(root.path().join("ca.pem"))?));
    target.alpn = Arc::from([Arc::from("h2"), Arc::from("http/1.1")]);
    target = target.with_derived_connection_fingerprint();
    let connector = PingoraConnectorAdapter::new();
    let connected =
        tokio::time::timeout(Duration::from_secs(5), connector.connect(&target, address)).await?;
    assert!(
        connected.is_ok(),
        "registered feed fixture TLS handshake failed before HTTP: {:?}",
        connected.err()
    );
    Ok(())
}
