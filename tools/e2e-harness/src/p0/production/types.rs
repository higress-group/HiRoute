use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::p0::canonical::canonical_json_digest;
use crate::p0::types::CorpusCase;

pub const ORACLE_VERSION: &str = "p0-gateway-production-oracle-v1";
pub const PROFILE_SCHEMA: &str = "hiroute.e2e.production-profile/v1";
pub const SCENARIO_SCHEMA: &str = "hiroute.e2e.production-scenario/v1";
pub const FIXTURE_SCHEMA: &str = "hiroute.e2e.production-smoke/v1";
pub const MANIFEST_SCHEMA: &str = "hiroute.e2e.production-manifest/v1";
pub const SUT_BUILD_ATTESTATION_SCHEMA: &str = "hiroute.e2e.sut-build-attestation/v1";
pub const LAUNCHER_SCHEMA: &str = "hiroute.e2e.production-launcher/v1";
pub const READINESS_SCHEMA: &str = "hiroute.e2e.production-readiness/v1";
pub const PRODUCT_READINESS_SCHEMA: &str = "hiroute.gateway.ready/v2";
pub const COLLECTOR_SCHEMA: &str = "hiroute.e2e.production-collector/v1";
pub const RESULT_SCHEMA: &str = "hiroute.e2e.production-result/v1";
pub const CREDENTIAL_MANIFEST_SCHEMA: &str = "hiroute.gateway.credentials/v1";
pub const CREDENTIAL_LEASE_SCHEMA: &str = "hiroute.gateway.credential-leases/v1";
pub const LIFECYCLE_SCHEMA: &str = "hiroute.gateway.lifecycle-fact-envelope/v2";
pub const LEGACY_EXECUTION_SCHEMA: &str = "hiroute.observation.execution-fact-envelope/v2";
pub const LEGACY_PRICED_EXECUTION_SCHEMA: &str = "hiroute.observation.execution-fact-envelope/v3";
pub const CURRENT_EXECUTION_SCHEMA: &str = "hiroute.observation.execution-fact-envelope/v3";
pub const CONTENT_SCHEMA: &str = "hiroute.observation.conversation-content-envelope/v2";
pub const OTEL_SCHEMA: &str = "hiroute.otel.gen-ai-mapping/v1";
pub const LEGACY_AGGREGATE_PORT_DIGEST: &str =
    "sha256:05755b4351c7c7bc3c33fbd2e8406c313b4231e50e28cce1b3e1f57c47f43cfa";
pub const CURRENT_AGGREGATE_PORT_DIGEST: &str =
    "sha256:5d0b1cafa66bfe816867c2fc68dfecccb4b8f7958506d25d19193c51475a34a6";
pub const SEALED_SUT_REVISION: &str = "8af977300795c467894371055f7fbfc9716252fa";
pub const SEALED_SUT_TREE: &str = "5ed527f53fa4e8ba9395b5a89b155a1b67228e6e";
pub const SEALED_SUT_BUILD_INPUT_DIGEST: &str =
    "sha256:203d9b4b4024b458673c8a1f5573e202476d6b69f03902ed160c0737b4f689b4";
pub const LIFECYCLE_SCHEMA_DIGEST: &str =
    "sha256:762143e3ab3121bf40cb7039a73ea0b47a5724c38837ba4152f75fc54e09e316";
pub const LEGACY_EXECUTION_SCHEMA_DIGEST: &str =
    "sha256:499cc87ff8aabf124c6613da86b076c1e34a7890fafe034dd897d50e10cae036";
pub const LEGACY_PRICED_EXECUTION_SCHEMA_DIGEST: &str =
    "sha256:a22fc62f35454021f1df21b2b9f0ce54cde325b3fc7b3aa2e990ab9ecba71292";
pub const CURRENT_EXECUTION_SCHEMA_DIGEST: &str =
    "sha256:f8b98014f3f7dae247ca3e6c5046691cbe43ccbe705f6441dfd3301af4a5af0d";
pub const CONTENT_SCHEMA_DIGEST: &str =
    "sha256:6f17cd772a0fb322d25dd31dc95e34325cbabbd68912b32f5bb9bed526adfe44";
pub const OTEL_SCHEMA_DIGEST: &str =
    "sha256:d730badfd13c19d1c464c5a0c871a35a3e72a0f9c662d13d1997f066bf762e33";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionProfile {
    pub schema_version: String,
    pub oracle_version: String,
    pub name: String,
    pub completion_policy: String,
    pub launch_mode: String,
    pub sut_binary: String,
    pub sut_source_revision: String,
    pub aggregate_port_digest: String,
}

impl ProductionProfile {
    pub fn resolve_sut(&self) -> Result<ResolvedSut, ProductionError> {
        if self.schema_version != PROFILE_SCHEMA
            || self.oracle_version != ORACLE_VERSION
            || self.name != "gateway-isolated"
            || self.completion_policy != "green_only"
            || self.launch_mode != "production_publication_credentials"
            || self.sut_binary != "${HIROUTE_E2E_SUT_BIN}"
            || self.sut_source_revision != SEALED_SUT_REVISION
            || self.aggregate_port_digest != LEGACY_AGGREGATE_PORT_DIGEST
        {
            return Err(ProductionError::Contract(
                "profile is not the sealed green-only production profile".into(),
            ));
        }
        validate_revision(&self.sut_source_revision)?;
        super::attestation::resolve(self)
    }
}

pub(super) fn validate_revision(value: &str) -> Result<(), ProductionError> {
    if value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ProductionError::Contract(
            "SUT revision must be 40 lowercase hexadecimal characters".into(),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedSut {
    pub canonical_path: PathBuf,
    pub executable_sha256: String,
    pub source_revision: String,
    pub build_attestation: SutBuildAttestation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SutBuildAttestation {
    pub schema_version: String,
    pub source_revision: String,
    pub sealed_source_tree: String,
    pub build_input_digest: String,
    pub source_checkout: String,
    pub cargo_package: String,
    pub cargo_binary: String,
    pub cargo_profile: String,
    pub enabled_features: Vec<String>,
    pub target_triple: String,
    pub cargo_version: String,
    pub rustc_version: String,
    pub rustc_wrapper: Option<String>,
    pub rustc_wrapper_version: Option<String>,
    pub toolchain_digest: String,
    pub build_nonce: String,
    pub build_command: Vec<String>,
    pub executable_path: String,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionScenario {
    pub schema_version: String,
    pub oracle_version: String,
    pub name: String,
    pub contract_manifest: String,
    pub fixture: String,
    pub coverage: Vec<ProductionCoverage>,
    pub checkpoints: Vec<ProductionCheckpoint>,
    pub matrix_extension: MatrixExtension,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionCoverage {
    pub id: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionCheckpoint {
    pub id: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixExtension {
    pub owner: String,
    pub contract: String,
    pub protocol_pairs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionFixture {
    pub schema_version: String,
    pub oracle_version: String,
    pub case: CorpusCase,
    pub expected_client: Value,
    pub expected_observation: ExpectedObservation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedObservation {
    pub lifecycle_terminal: String,
    pub execution_terminal: String,
    pub content_terminals: Vec<ContentTerminal>,
    pub required_execution_facts: Vec<String>,
    pub required_otel_signal: String,
    pub freshness_binding: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentTerminal {
    pub direction: String,
    pub phase: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionManifest {
    pub schema_version: String,
    pub oracle_version: String,
    pub contract_digest: String,
    pub artifacts: Vec<ProductionArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionArtifact {
    pub path: String,
    pub schema: Option<String>,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherRecord {
    pub schema_version: String,
    pub mode: String,
    pub executable_path: String,
    pub executable_sha256: String,
    pub sut_source_revision: String,
    pub build_attestation: SutBuildAttestation,
    pub child_pid: u32,
    pub listen_address: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub publication_digest: String,
    pub credential_manifest_digest: String,
    pub credential_lease_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductReady {
    pub schema_version: String,
    pub status: String,
    pub publication_revision: u64,
    pub publication_digest: String,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessEvidence {
    pub schema_version: String,
    pub child_pid: u32,
    pub listen_address: String,
    pub probe_path: String,
    pub product: ProductReady,
    pub listener_owner_verifier: String,
    pub listener_owner_pid_before_probe: u32,
    pub listener_owner_pid_after_probe: u32,
    pub child_alive_before_probe: bool,
    pub child_alive_after_probe: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEvidence {
    pub name: String,
    pub file_sha256: String,
    pub records_digest: String,
    pub producer_id: String,
    pub producer_epoch: String,
    pub stream_id: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub record_count: usize,
    pub terminal: String,
    pub records: Vec<Value>,
}

impl ChannelEvidence {
    pub fn digest_records(records: &[Value]) -> String {
        canonical_json_digest(&Value::Array(records.to_vec()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectorEvidence {
    pub schema_version: String,
    pub request_id: String,
    pub lifecycle: ChannelEvidence,
    pub execution_fact: ChannelEvidence,
    pub conversation_content: ChannelEvidence,
    pub otel: ChannelEvidence,
    pub independent_streams: bool,
    pub no_gap_or_loss: bool,
    pub content_terminal_independent: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCheck {
    pub id: String,
    pub status: EvidenceStatus,
    pub expected_digest: String,
    pub actual_digest: String,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointResult {
    pub id: String,
    pub status: EvidenceStatus,
    pub evidence: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioState {
    Green,
    Red,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessExitEvidence {
    pub test_process_code: i32,
    pub sut_termination: String,
    pub sut_reaped: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyEvidence {
    pub private_root_mode: String,
    pub observation_root_mode: String,
    pub process_root_mode: String,
    pub sensitive_occurrences: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRunReport {
    pub schema_version: String,
    pub oracle_version: String,
    pub contract_digest: String,
    pub aggregate_port_digest: String,
    pub schema_digests: BTreeMap<String, String>,
    pub evidence_digest: String,
    pub result_payload_digest: String,
    pub process_exit: ProcessExitEvidence,
    pub scenario_state: ScenarioState,
    pub run_nonce: String,
    pub freshness_challenge: String,
    pub launcher: LauncherRecord,
    pub readiness: ReadinessEvidence,
    pub native_provider: Value,
    pub client_output: Value,
    pub observations: CollectorEvidence,
    pub privacy: PrivacyEvidence,
    pub checks: Vec<EvidenceCheck>,
    pub checkpoints: Vec<CheckpointResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct VerifiedProductionRun(pub(crate) ProductionRunReport);

impl std::ops::Deref for VerifiedProductionRun {
    type Target = ProductionRunReport;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl VerifiedProductionRun {
    pub fn as_report(&self) -> &ProductionRunReport {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ProductionRunOptions {
    pub sut: ResolvedSut,
    pub timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum ProductionError {
    #[error("production Oracle contract failed: {0}")]
    Contract(String),
    #[error("production Oracle provenance failed: {0}")]
    Provenance(String),
    #[error("production Oracle readiness failed: {0}")]
    Readiness(String),
    #[error("production Oracle collection failed: {0}")]
    Collector(String),
    #[error("production Oracle result is red")]
    ScenarioRed,
    #[error("production Oracle path is not UTF-8")]
    NonUtf8Path,
    #[error("production Oracle I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("production Oracle JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("production Oracle process failed: {0}")]
    Process(String),
    #[error("production Oracle client failed: {0}")]
    Client(String),
    #[error("production Oracle HTTP failed: {0}")]
    Http(String),
}

impl From<crate::process::ProcessError> for ProductionError {
    fn from(error: crate::process::ProcessError) -> Self {
        Self::Process(error.to_string())
    }
}

impl From<crate::p0::client::ClientError> for ProductionError {
    fn from(error: crate::p0::client::ClientError) -> Self {
        Self::Client(error.to_string())
    }
}

impl From<crate::http::HttpError> for ProductionError {
    fn from(error: crate::http::HttpError) -> Self {
        Self::Http(error.to_string())
    }
}

pub(crate) fn path_text(path: &Path) -> Result<String, ProductionError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or(ProductionError::NonUtf8Path)
}
