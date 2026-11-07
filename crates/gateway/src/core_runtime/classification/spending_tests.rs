//! Real exchange lifecycle with a narrow spending-authority test double. The
//! durable ledger and exact official price resolver have separate regressions.
use super::*;
use crate::server::composition::{CredentialResolver, PortError, ProductionPorts};
use crate::server::publication::GatewayPublicationInstaller;
use hiroute_domain::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Spending {
    error: u8,
    captures: AtomicUsize,
    reservations: AtomicUsize,
    settlements: Mutex<Vec<KeySpendSettlementV1>>,
}
impl KeySpendAuthorityV1 for Spending {
    fn currency(&self, _: &str) -> PortResult<Option<String>> {
        Ok(Some("USD".into()))
    }
    fn reserve(
        &self,
        _: &str,
        _: u64,
        estimate: Option<&KeySpendEstimateV1>,
    ) -> Result<Option<KeySpendReservationV1>, KeyBudgetGateErrorV1> {
        self.reservations.fetch_add(1, Ordering::SeqCst);
        match self.error {
            1 => return Err(KeyBudgetGateErrorV1::Exhausted),
            2 => {
                return Err(hiroute_domain::PortError::new(
                    PortErrorCode::Unavailable,
                    "fixture.budget",
                )
                .into());
            }
            _ => {}
        }
        assert!(
            estimate.is_some(),
            "captured decision estimate was dropped before reservation"
        );
        Ok(Some(KeySpendReservationV1 {
            reservation_id: "reservation/decision".into(),
            budget_id: "budget/one".into(),
            amount_micros: 100,
            budget_epoch: 0,
        }))
    }
    fn mark_sent(&self, _: &str) -> PortResult<()> {
        Ok(())
    }
    fn settle(&self, _: &str, value: &KeySpendSettlementV1) -> PortResult<()> {
        self.settlements.lock().unwrap().push(value.clone());
        Ok(())
    }
}

struct Credentials(Arc<Spending>);
#[async_trait::async_trait]
impl CredentialResolver for Credentials {
    fn acquire(&self, _: &str) -> Result<crate::server::composition::CredentialLease, PortError> {
        Err(PortError::Rejected)
    }
    fn decision_spend_estimates(
        &self,
        _: &DecisionCredentialBindingV1,
        input_bound: u64,
    ) -> std::collections::BTreeMap<String, KeySpendEstimateV1> {
        self.0.captures.fetch_add(1, Ordering::SeqCst);
        assert!(input_bound > 0);
        // The lifecycle test needs only the opaque captured estimate. Real quote
        // validation, tier arithmetic and persistence are exercised by their owners.
        std::collections::BTreeMap::from([(
            "USD".into(),
            KeySpendEstimateV1 {
                pricing: ExecutionPricingEvidenceV1 {
                    usage_semantics: Default::default(),
                    schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
                    request_generation: None,
                    captured_at_ms: 0,
                    attempt_execution_at_ms: 0,
                    quote: None,
                    reference_quote: None,
                    unknown_reason: Some(PriceUnknownReasonV1::PriceNotCollected),
                },
                input_bound,
                output_bound: 0,
                input_price_tiers: vec![],
            },
        )])
    }
    fn admit_decision_credentials(
        &self,
        _: &DecisionCredentialBindingV1,
    ) -> Result<DecisionCredentialAdmissionV1, PortError> {
        Ok(DecisionCredentialAdmissionV1 {
            state: MaterializationState::Ready,
            keys: vec![DecisionCredentialKeyV1 {
                key_id: "key/one".into(),
                generation: 1,
            }],
            credential_state_revision: CanonicalDigest::of_bytes(b"admitted"),
        })
    }
    async fn lease_decision_credential(
        &self,
        _: &DecisionCredentialLeaseRequestV1,
        _: &ExecutionScope,
    ) -> Result<Option<CredentialLease>, PortError> {
        Ok(Some(
            CredentialLease::new("key/one", "key/one", 1, "Bearer fixture")
                .unwrap()
                .with_spend_authority(Some(self.0.clone())),
        ))
    }
}

fn runtime(
    root: &std::path::Path,
    endpoint: &str,
    spending: Arc<Spending>,
) -> (ProductionGatewayRuntime, ComplexityClassifierModeV1) {
    let lineage = CanonicalDigest::of_bytes(b"source/one");
    let service: DecisionServiceV2 = serde_json::from_value(serde_json::json!({
        "schema":"hiroute.decision-service/v2", "id":"decision/spending", "revision":1, "name":"Spending",
        "connection":{"kind":"native","provider":"openai","wire_protocol":"openai_decisions","model":"gpt-6-luna","endpoint":endpoint,"timeout_ms":3000,
            "authentication":{"kind":"shared_source","source_id":"source/one","source_lineage_digest":lineage}}
    })).unwrap();
    let source: ComputeManagementSourceV2 = serde_json::from_value(serde_json::json!({
        "schema":"hiroute.compute-management-source/v4","source_id":"source/one","revision":1,"lineage_digest":lineage,"display_name":"Source",
        "provenance":{"kind":"user_configured","configuration_revision":1,"evidence_digest":lineage},
        "target":{"scheme":"http","authority":"127.0.0.1","port":1,"request_path":"/v1/responses","upstream_protocol":"responses","protocol_profile_id":"adapter.openai-responses.v1","protocol_profile_revision":1},
        "authentication":{"kind":"bearer"},"state":"ready","models":[],"last_candidate_ref":"candidate/one","last_candidate_revision":1
    })).unwrap();
    let network =
        NativeNetworkRegistryV1::new(NativeProxyEnvironmentV1::from_variables("test".into(), []));
    network
        .initialize([NativeNetworkRegistryEntryV1::for_source(
            &source,
            NativeSourceNetworkRecordV1::initial(&source).unwrap(),
        )
        .unwrap()])
        .unwrap();
    let mut ports = ProductionPorts::fail_closed(Arc::new(
        GatewayPublicationInstaller::open(root.join("publication.json")).unwrap(),
    ));
    ports.credentials = Arc::new(Credentials(spending));
    ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
    ports.native_network = Some(Arc::new(network));
    (
        ProductionGatewayRuntime::compose(ports),
        ComplexityClassifierModeV1::DecisionServiceV2 {
            service: Box::new(service),
        },
    )
}

fn root(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hiroute-decision-spend-{}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

fn server(kind: u8) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/decisions", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let length: usize = std::str::from_utf8(&request)
            .unwrap()
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let answers: Vec<_> = request["questions"].as_array().unwrap().iter().map(|q| {
            if q["type"] == "choice" {
                serde_json::json!({"name":q["name"],"type":"choice","choice":q["choices"][0]["value"],"probabilities":q["choices"].as_array().unwrap().iter().enumerate().map(|(i,c)| serde_json::json!({"value":c["value"],"probability":if i==0 {1.0} else {0.0}})).collect::<Vec<_>>()})
            } else {
                serde_json::json!({"name":q["name"],"type":"score","score":0.0,"probabilities":q["levels"].as_array().unwrap().iter().enumerate().map(|(i,l)| serde_json::json!({"value":i,"label":l["label"],"probability":if i==0 {1.0} else {0.0}})).collect::<Vec<_>>()})
            }
        }).collect();
        let mut response = serde_json::json!({"answers":answers});
        if kind != 2 {
            response["usage"] = serde_json::json!({"input_tokens":100,"output_tokens":0,"input_tokens_details":{"cached_tokens":30,"cache_write_tokens":20}});
        }
        if kind == 1 {
            response["answers"] = serde_json::Value::Null;
        }
        let bytes = serde_json::to_vec(&response).unwrap();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    (endpoint, thread)
}

#[tokio::test(flavor = "current_thread")]
async fn decision_exchange_settles_usage_even_when_answers_are_invalid_and_keeps_missing_usage_pending()
 {
    for kind in 0..3 {
        let root = root(&format!("response-{kind}"));
        let spending = Arc::new(Spending::default());
        let (endpoint, thread) = server(kind);
        let (runtime, mode) = runtime(&root, &endpoint, spending.clone());
        let result = runtime.test_classifier_decision(&mode).await;
        if kind == 1 {
            assert_eq!(
                result.unwrap_err(),
                ClassifierDiagnosticError::InvalidOutput
            );
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
        thread.join().unwrap();
        assert_eq!(spending.captures.load(Ordering::SeqCst), 1);
        assert_eq!(spending.reservations.load(Ordering::SeqCst), 1);
        let values = spending.settlements.lock().unwrap();
        assert_eq!(values.len(), 1);
        if kind == 2 {
            assert_eq!(values[0], KeySpendSettlementV1::Unknown);
        } else {
            assert!(
                matches!(&values[0], KeySpendSettlementV1::Usage { usage } if usage.input_tokens == Some(100) && usage.cache_read_tokens == Some(30))
            );
        }
        drop(values);
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn decision_budget_errors_are_local_and_proven_unsent_attempts_release_their_reservation() {
    for error in 0..3 {
        let root = root(&format!("failure-{error}"));
        let spending = Arc::new(Spending {
            error,
            ..Default::default()
        });
        let (runtime, mode) = runtime(&root, "http://127.0.0.1:1/v1/decisions", spending.clone());
        let result = runtime.test_classifier_decision(&mode).await.unwrap_err();
        if error == 0 {
            assert_eq!(
                *spending.settlements.lock().unwrap(),
                [KeySpendSettlementV1::Unsent]
            );
        } else {
            assert_eq!(
                result.detail().kind,
                if error == 1 {
                    FailureKind::BudgetExhausted
                } else {
                    FailureKind::BudgetUnavailable
                }
            );
            assert!(spending.settlements.lock().unwrap().is_empty());
        }
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}
