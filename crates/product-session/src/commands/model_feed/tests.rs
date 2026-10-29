//! The real Client Core exchange waits on a remote refresh while unrelated reads proceed.
use super::*;
use crate::authority::HostAuthority;
use hiroute_client_core::{Client, LocalEndpoint};
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, OnceLock, atomic::AtomicU64},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::{Mutex, Notify},
};
use zeroize::Zeroizing;

struct Authority;
impl HostAuthority for Authority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        _: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        Err("UNEXPECTED_MUTATION".into())
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        Err("UNEXPECTED_MUTATION".into())
    }
}

#[tokio::test]
async fn slow_model_feed_refresh_does_not_hold_session_mutex_or_block_local_queries() {
    refresh_with_concurrent_reads(false).await;
}
#[tokio::test]
async fn model_feed_response_from_replaced_session_is_rejected() {
    refresh_with_concurrent_reads(true).await;
}
async fn refresh_with_concurrent_reads(replace_session: bool) {
    let root = tempfile::Builder::new()
        .prefix("hrfeed-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let runtime = root.path().join("run");
    std::fs::create_dir_all(runtime.join("hiroute")).unwrap();
    std::fs::set_permissions(
        runtime.join("hiroute"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let endpoint = LocalEndpoint::for_child(&runtime, std::process::id());
    let listener = UnixListener::bind(endpoint.path()).unwrap();
    std::fs::set_permissions(endpoint.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let client = Client::new("feed-concurrency-test", endpoint);
    let sessions = Arc::new(Mutex::new(Some(Session::new(client, Arc::new(Authority)))));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (started_server, release_server) = (started.clone(), release.clone());
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (started, release) = (started_server.clone(), release_server.clone());
            tokio::spawn(async move {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
                let response = serde_json::to_vec(&negotiate_hello(&hello).unwrap()).unwrap();
                stream.get_mut().write_all(&response).await.unwrap();
                stream.get_mut().write_all(b"\n").await.unwrap();
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                let request: LocalControlWireRequestV2 = serde_json::from_str(&line).unwrap();
                if request.operation_id == "RefreshModelFeed" {
                    started.notify_one();
                    release.notified().await;
                }
                let response = json!({"schema_version":LOCAL_CONTROL_SCHEMA_V2,"request_id":request.request_id,
                    "status":"succeeded","data":{"operation":request.operation_id},"operation":null,
                    "warnings":[],"next_actions":[],"error":null});
                stream
                    .get_mut()
                    .write_all(&serde_json::to_vec(&response).unwrap())
                    .await
                    .unwrap();
                stream.get_mut().write_all(b"\n").await.unwrap();
            });
        }
    });
    let error = OnceLock::new();
    let epoch = AtomicU64::new(1);
    let diagnostics = hiroute_diagnostics::context::DiagnosticHandle::noop();
    let state = CommandState(&sessions, &error, &epoch, &diagnostics);
    let broker = crate::web_confirmation::ConfirmationBroker::default();
    let context = ClientContext {
        id: "feed-test",
        confirmations: &broker,
        active: &|| true,
        visible: &|| true,
        notify: None,
    };
    let input = ClientEmptyRequestV1::default();
    let refresh = query::<serde_json::Value, _>(&context, &state, "RefreshModelFeed", &input);
    let concurrent = async {
        started.notified().await;
        let reads = async {
            for operation in [
                "GetPlanEditorOptions",
                "GetEffectivePrices",
                "GetAgentConnectionSnapshot",
            ] {
                let value: serde_json::Value =
                    query(&context, &state, operation, &input).await.unwrap();
                assert_eq!(value["operation"], operation);
            }
        };
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), reads).await;
        if replace_session {
            let mut guard = sessions.lock().await;
            let client = guard.as_ref().unwrap().client.clone();
            *guard = Some(Session::new(client, Arc::new(Authority)));
        }
        release.notify_one();
        assert!(
            outcome.is_ok(),
            "slow feed I/O blocked the shared product session"
        );
    };
    let (result, _) = tokio::join!(refresh, concurrent);
    if replace_session {
        assert!(
            result.is_err(),
            "a response from revoked session authority was returned"
        );
    } else {
        assert_eq!(result.unwrap()["operation"], "RefreshModelFeed");
    }
    server.abort();
}
