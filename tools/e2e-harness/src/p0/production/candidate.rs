//! Current-checkout identity policy. The sealed v1 loader/verifier stays unchanged.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{attestation, contract::ProductionBundle, types::*};
use crate::p0::canonical::{canonical_json_digest, sha256_hex};
use crate::p0::types::{CorpusCase, ProviderBody};

pub(super) const RESULT_V2: &str = "hiroute.e2e.production-result/v2";
pub(super) const LAUNCHER_V2: &str = "hiroute.e2e.production-launcher/v2";
const ATTESTATION_V3: &str = "hiroute.e2e.sut-build-attestation/v3";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateBuildReceipt {
    recipe: String,
    attestation: SutBuildAttestation,
}

fn build_environment() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| {
            [
                "CARGO_",
                "RUST",
                "CC_",
                "CXX_",
                "CMAKE_",
                "PKG_CONFIG_",
                "OPENSSL_",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix))
                || matches!(
                    key.as_str(),
                    "PATH"
                        | "CC"
                        | "CXX"
                        | "CFLAGS"
                        | "CXXFLAGS"
                        | "CPPFLAGS"
                        | "LDFLAGS"
                        | "HOME"
                        | "SDKROOT"
                )
        })
        .collect()
}

fn cargo_config_digest() -> Result<String, ProductionError> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .ok_or_else(|| ProductionError::Provenance("Cargo home unavailable".into()))?;
    let mut inputs = Vec::new();
    for name in ["config.toml", "config"] {
        let path = home.join(name);
        inputs.push((
            name,
            if path.exists() {
                Some(sha256_hex(&fs::read(path)?))
            } else {
                None
            },
        ));
    }
    Ok(canonical_json_digest(&json!(inputs)))
}

// No Deserialize or public constructor: an input document is not source evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CandidateIdentity {
    root: PathBuf,
    pub(super) revision: String,
    tree: String,
    inputs: String,
}

impl CandidateIdentity {
    fn capture(root: &Path) -> Result<Self, ProductionError> {
        let root = root.canonicalize()?;
        let top = attestation::git_text(&root, &["rev-parse", "--show-toplevel"], "checkout")?;
        if Path::new(&top).canonicalize()? != root {
            return Err(ProductionError::Provenance(
                "not the repository root".into(),
            ));
        }
        let status = attestation::git_output(
            &root,
            ["status", "--porcelain", "--untracked-files=all"],
            "candidate status",
        )?;
        if !status.stdout.is_empty() {
            return Err(ProductionError::Provenance(
                "candidate source is not committed and clean".into(),
            ));
        }
        let revision = attestation::git_text(&root, &["rev-parse", "HEAD"], "candidate revision")?;
        validate_revision(&revision)?;
        let tree = attestation::git_text(&root, &["rev-parse", "HEAD^{tree}"], "candidate tree")?;
        let listing =
            attestation::git_output(&root, ["ls-tree", "-r", "HEAD"], "complete build inputs")?;
        Ok(Self {
            root,
            revision,
            tree,
            inputs: sha256_hex(&listing.stdout),
        })
    }

    fn check_unchanged(&self) -> Result<(), ProductionError> {
        if &Self::capture(&self.root)? != self {
            return Err(ProductionError::Provenance(
                "candidate changed during execution".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn resolve_sut(&self) -> Result<ResolvedSut, ProductionError> {
        self.check_unchanged()?;
        let target = self.root.join("target");
        if std::fs::symlink_metadata(&target)?.file_type().is_symlink() {
            return Err(ProductionError::Provenance(
                "target must be checkout-private".into(),
            ));
        }
        let toolchain = attestation::resolve_toolchain(&self.root)?;
        let recipe = canonical_json_digest(&json!({
            "source_revision": self.revision, "source_tree": self.tree,
            "source_inputs": self.inputs, "package": "hiroute-gateway",
            "binary": "hirouted", "profile": "dev", "features": ["all"],
            "toolchain": toolchain.digest, "cargo": toolchain.cargo,
            "cargo_sha256": sha256_hex(&fs::read(&toolchain.cargo)?),
            "rustc_sha256": sha256_hex(&fs::read(toolchain.cargo.with_file_name("rustc"))?),
            "wrapper_sha256": toolchain.rustc_wrapper.as_ref()
                .map(|path| fs::read(path).map(|bytes| sha256_hex(&bytes))).transpose()?,
            "cargo_config_sha256": cargo_config_digest()?,
            "environment": build_environment(),
            "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        }));
        let smoke = target.join("smoke");
        let cache = smoke.join("builds");
        for directory in [&smoke, &cache] {
            if !directory.exists() {
                fs::DirBuilder::new().mode(0o700).create(directory)?;
            }
            let metadata = fs::symlink_metadata(directory)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.permissions().mode() & 0o077 != 0
            {
                return Err(ProductionError::Provenance(
                    "unsafe smoke build cache".into(),
                ));
            }
        }
        let key_dir = cache.join(recipe.trim_start_matches("sha256:"));
        if !key_dir.exists() {
            fs::DirBuilder::new().mode(0o700).create(&key_dir)?;
        }
        let key_metadata = fs::symlink_metadata(&key_dir)?;
        if key_metadata.file_type().is_symlink()
            || !key_metadata.is_dir()
            || key_metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ProductionError::Provenance(
                "unsafe Gateway build cache".into(),
            ));
        }
        let directory = key_dir.join("hiroute-gateway");
        let executable = directory.join("hirouted");
        let receipt = directory.join("receipt.json");
        if directory.exists() {
            let metadata = fs::symlink_metadata(&directory)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.permissions().mode() & 0o077 != 0
            {
                return Err(ProductionError::Provenance(
                    "unsafe Gateway build cache".into(),
                ));
            }
        }
        if receipt.exists() {
            if fs::symlink_metadata(&receipt)?.file_type().is_symlink() {
                return Err(ProductionError::Provenance(
                    "unsafe Gateway build receipt".into(),
                ));
            }
            let saved: CandidateBuildReceipt = serde_json::from_slice(&fs::read(receipt)?)?;
            if saved.recipe != recipe
                || saved.attestation.executable_path != path_text(&executable)?
            {
                return Err(ProductionError::Provenance(
                    "Gateway build recipe changed".into(),
                ));
            }
            let sut = ResolvedSut {
                canonical_path: executable,
                executable_sha256: saved.attestation.executable_sha256.clone(),
                source_revision: self.revision.clone(),
                build_attestation: saved.attestation,
            };
            self.verify_live_sut(&sut)?;
            if sut.build_attestation.toolchain_digest != toolchain.digest {
                return Err(ProductionError::Provenance(
                    "Gateway toolchain changed".into(),
                ));
            }
            return Ok(sut);
        }
        if directory.exists() {
            return Err(ProductionError::Provenance(
                "incomplete Gateway build cache".into(),
            ));
        }
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        if std::env::var_os("HIROUTE_SMOKE_REQUIRE_PREPARED").is_some() {
            return Err(ProductionError::Provenance(
                "prepared Gateway build missing".into(),
            ));
        }
        let nonce = attestation::random_hex()?;
        let command = attestation::current_build_command();
        let artifact = attestation::rebuild(&self.root, &toolchain, &command)?;
        if !artifact.starts_with(target.canonicalize()?) {
            return Err(ProductionError::Provenance(
                "Cargo artifact escapes private target".into(),
            ));
        }
        attestation::require_executable(&artifact)?;
        let original_digest = sha256_hex(&fs::read(&artifact)?);
        fs::copy(&artifact, &executable)?;
        let digest = sha256_hex(&fs::read(&executable)?);
        if digest != original_digest {
            return Err(ProductionError::Provenance(
                "Gateway artifact changed while staging".into(),
            ));
        }
        let build = SutBuildAttestation {
            schema_version: ATTESTATION_V3.into(),
            source_revision: self.revision.clone(),
            sealed_source_tree: self.tree.clone(),
            build_input_digest: self.inputs.clone(),
            source_checkout: path_text(&self.root)?,
            cargo_package: "hiroute-gateway".into(),
            cargo_binary: "hirouted".into(),
            cargo_profile: "dev".into(),
            enabled_features: vec!["all".into()],
            target_triple: toolchain.target_triple,
            cargo_version: toolchain.cargo_version,
            rustc_version: toolchain.rustc_version,
            rustc_wrapper: toolchain
                .rustc_wrapper
                .as_deref()
                .map(path_text)
                .transpose()?,
            rustc_wrapper_version: toolchain.rustc_wrapper_version,
            toolchain_digest: toolchain.digest,
            build_nonce: nonce,
            build_command: command,
            executable_path: path_text(&executable)?,
            executable_sha256: digest.clone(),
        };
        let sut = ResolvedSut {
            canonical_path: executable,
            executable_sha256: digest,
            source_revision: self.revision.clone(),
            build_attestation: build,
        };
        self.verify_live_sut(&sut)?;
        let saved = CandidateBuildReceipt {
            recipe,
            attestation: sut.build_attestation.clone(),
        };
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&receipt)?;
        output.write_all(&serde_json::to_vec_pretty(&saved)?)?;
        output.sync_all()?;
        Ok(sut)
    }

    pub(super) fn verify_attestation(
        &self,
        evidence: &SutBuildAttestation,
    ) -> Result<(), ProductionError> {
        if evidence.source_checkout != path_text(&self.root)? {
            return Err(ProductionError::Provenance(
                "attestation checkout differs".into(),
            ));
        }
        attestation::verify_identity(
            evidence,
            ATTESTATION_V3,
            &self.revision,
            &self.tree,
            &self.inputs,
        )
    }

    pub(super) fn verify_live_sut(&self, sut: &ResolvedSut) -> Result<(), ProductionError> {
        self.check_unchanged()?;
        self.verify_attestation(&sut.build_attestation)?;
        verify_private_executable(&self.root, &sut.canonical_path)?;
        if sut.source_revision != self.revision
            || path_text(&sut.canonical_path)? != sut.build_attestation.executable_path
            || sut.executable_sha256 != sut.build_attestation.executable_sha256
            || sha256_hex(&std::fs::read(&sut.canonical_path)?) != sut.executable_sha256
        {
            return Err(ProductionError::Provenance("SUT artifact changed".into()));
        }
        Ok(())
    }
}

fn verify_private_executable(root: &Path, executable: &Path) -> Result<(), ProductionError> {
    let target = root.join("target");
    let target_metadata = fs::symlink_metadata(&target)?;
    let artifact_metadata = fs::symlink_metadata(executable)?;
    if target_metadata.file_type().is_symlink()
        || !artifact_metadata.file_type().is_file()
        || !executable
            .canonicalize()?
            .starts_with(target.canonicalize()?)
    {
        return Err(ProductionError::Provenance(
            "Gateway artifact must be a regular file inside the private target".into(),
        ));
    }
    Ok(())
}

impl ProductionBundle {
    pub(super) fn result_schema(&self) -> &'static str {
        if self.candidate.is_some() {
            RESULT_V2
        } else {
            RESULT_SCHEMA
        }
    }
    pub(super) fn launcher_schema(&self) -> &'static str {
        if self.candidate.is_some() {
            LAUNCHER_V2
        } else {
            LAUNCHER_SCHEMA
        }
    }

    pub(super) fn expected_client_for(
        &self,
        case: &CorpusCase,
        sealed_expected: Value,
    ) -> Result<Value, ProductionError> {
        if self.candidate.is_none() {
            return Ok(sealed_expected);
        }
        let provider = case.providers.first().ok_or_else(|| {
            ProductionError::Contract("current production Provider script missing".into())
        })?;
        let ProviderBody::Json { value } = &provider.response.body else {
            return Err(ProductionError::Contract(
                "current non-stream production response must be JSON".into(),
            ));
        };
        let native_model = provider
            .expected_request
            .body
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProductionError::Contract(
                    "current production Provider request model is missing".into(),
                )
            })?;
        let alias = case
            .ingress
            .body
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProductionError::Contract("current production ingress model is missing".into())
            })?;
        let mut body = value.clone();
        let response_model = body
            .get_mut("model")
            .filter(|model| model.as_str() == Some(native_model))
            .ok_or_else(|| {
                ProductionError::Contract(
                    "current production Provider response model is not exact".into(),
                )
            })?;
        *response_model = Value::String(alias.into());
        Ok(json!({
            "body": body,
            "content_type": provider.response.content_type,
            "status": provider.response.status,
            "transport": "http1",
        }))
    }

    fn current(root: &Path) -> Result<Self, ProductionError> {
        let identity = CandidateIdentity::capture(root)?;
        let e2e = identity.root.join("e2e");
        // Load and verify every existing sealed fixture, schema and checkpoint first.
        let mut bundle = Self::load(
            &e2e.join("scenarios/p0-gateway.json"),
            None,
            &e2e.join("schema"),
        )?;
        let profile: Value =
            serde_json::from_slice(&std::fs::read(e2e.join("profiles/gateway-current.json"))?)?;
        if profile
            != json!({"schema_version":"hiroute.e2e.production-profile/v2",
            "name":"gateway-current", "identity_source":"verified_checkout",
            "completion_policy":"green_only", "scenario":"gateway.responses.controlled"})
        {
            return Err(ProductionError::Contract(
                "current profile is not exact".into(),
            ));
        }
        for current in [
            "schema/current-production-launcher.schema.json",
            "schema/current-production-collector.schema.json",
            "schema/current-gateway-result.schema.json",
        ] {
            let value = serde_json::from_slice(&std::fs::read(e2e.join(current))?)?;
            bundle.schemas.insert(current.into(), value);
        }
        bundle.manifest.contract_digest = canonical_json_digest(&json!({
            "schema":"hiroute.e2e.current-contract/v2", "sealed_contract":bundle.manifest.contract_digest,
            "profile":profile, "schemas":bundle.schema_digests(), "source_tree":identity.tree,
            "aggregate_port_digest": CURRENT_AGGREGATE_PORT_DIGEST,
        }));
        bundle.profile.sut_source_revision = identity.revision.clone();
        // Current same-protocol delivery preserves Responses shorthand. Keep
        // the sealed historical oracle unchanged, but assert the actual native
        // input contract for the current candidate (not canonical array shape).
        bundle.fixture.case.providers[0].expected_request.body["input"] =
            bundle.fixture.case.ingress.body["input"].clone();
        bundle.candidate = Some(identity);
        Ok(bundle)
    }
}

/// Private run artifact. Public smoke summaries retain only its verified digest and scope.
#[derive(Serialize)]
pub struct CandidateRunReport {
    pub schema: &'static str,
    pub request_run_id: String,
    pub tool_sha256: String,
    pub build_ms: u128,
    pub execution_ms: u128,
    pub report: ProductionRunReport,
}

/// Build evidence only. No provider, Gateway runtime or scenario is started.
pub fn prepare_current_candidate(root: &Path) -> Result<Value, ProductionError> {
    let bundle = ProductionBundle::current(root)?;
    let sut = bundle.resolve_sut()?;
    let identity = bundle
        .candidate
        .as_ref()
        .expect("current bundle has identity");
    identity.verify_live_sut(&sut)?;
    Ok(
        json!({"schema":"hiroute.e2e.current-preparation/v1", "prepared":true,
        "source_revision":identity.revision, "executable_sha256":sut.executable_sha256}),
    )
}

pub async fn run_current_candidate(
    root: &Path,
    request_run_id: &str,
    timeout: Duration,
) -> Result<CandidateRunReport, ProductionError> {
    if request_run_id.len() != 64
        || !request_run_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(ProductionError::Contract(
            "invalid smoke run identity".into(),
        ));
    }
    if timeout.is_zero() || timeout > Duration::from_secs(300) {
        return Err(ProductionError::Contract(
            "invalid current scenario timeout".into(),
        ));
    }
    let bundle = ProductionBundle::current(root)?;
    let build_started = Instant::now();
    let sut = bundle.resolve_sut()?;
    let build_ms = build_started.elapsed().as_millis();
    let identity = bundle
        .candidate
        .as_ref()
        .expect("current bundle has identity");
    let tool = std::env::current_exe()?.canonicalize()?;
    if !tool.starts_with(root.canonicalize()?.join("target")) {
        return Err(ProductionError::Provenance(
            "runner is outside candidate target".into(),
        ));
    }
    let tool_sha256 = sha256_hex(&std::fs::read(&tool)?);
    let started = Instant::now();
    let run = tokio::time::timeout(
        timeout + Duration::from_secs(10),
        super::runtime::run(
            &bundle,
            ProductionRunOptions {
                sut: sut.clone(),
                timeout,
            },
        ),
    )
    .await
    .map_err(|_| ProductionError::Process("current scenario deadline exceeded".into()))??;
    let execution_ms = started.elapsed().as_millis();
    identity.verify_live_sut(&sut)?;
    Ok(CandidateRunReport {
        schema: "hiroute.e2e.current-run/v2",
        request_run_id: request_run_id.into(),
        tool_sha256,
        build_ms,
        execution_ms,
        report: run.0,
    })
}

/// Write private detailed evidence; callers expose a redacted summary separately.
pub fn write_current_report(
    path: &Path,
    report: &CandidateRunReport,
) -> Result<(), ProductionError> {
    crate::p0::privacy::private_write(path, &serde_json::to_vec_pretty(report)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn current_gateway_artifact_rejects_symlink_and_parent_escape() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target/smoke/builds");
        fs::create_dir_all(&target).unwrap();
        let valid = target.join("hirouted");
        fs::write(&valid, b"same executable bytes").unwrap();
        assert!(verify_private_executable(temp.path(), &valid).is_ok());

        let linked = target.join("linked-hirouted");
        symlink(&valid, &linked).unwrap();
        assert!(verify_private_executable(temp.path(), &linked).is_err());

        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("hirouted"), b"same executable bytes").unwrap();
        symlink(&outside, target.join("alias")).unwrap();
        assert!(verify_private_executable(temp.path(), &target.join("alias/hirouted")).is_err());
    }

    #[test]
    fn current_expected_client_is_native_body_with_only_the_model_alias_rewritten() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let e2e = root.join("e2e");
        let mut bundle = ProductionBundle::load(
            &e2e.join("scenarios/p0-gateway.json"),
            None,
            &e2e.join("schema"),
        )
        .unwrap();
        let case = bundle.fixture.case.clone();
        let sealed_expected = bundle.fixture.expected_client.clone();
        assert_eq!(
            bundle
                .expected_client_for(&case, sealed_expected.clone())
                .unwrap(),
            sealed_expected
        );

        bundle.candidate = Some(CandidateIdentity {
            root,
            revision: "0".repeat(40),
            tree: "1".repeat(40),
            inputs: "sha256:test".into(),
        });
        assert_eq!(
            bundle
                .expected_client_for(&case, bundle.fixture.expected_client.clone())
                .unwrap(),
            json!({
                "body": {
                    "id": "oracle-native-response",
                    "model": "oracle-smoke",
                    "status": "completed",
                    "output": [{
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "${RUN_CHALLENGE}"}],
                    }],
                    "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5},
                },
                "content_type": "application/json",
                "status": 200,
                "transport": "http1",
            })
        );
    }

    #[test]
    fn current_identity_requires_a_committed_clean_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .output()
                .unwrap()
        };
        assert!(git(&["init", "--quiet"]).status.success());
        std::fs::write(temp.path().join("input"), "one").unwrap();
        assert!(CandidateIdentity::capture(temp.path()).is_err());
        assert!(git(&["add", "input"]).status.success());
        assert!(
            git(&[
                "-c",
                "user.name=Smoke Test",
                "-c",
                "user.email=smoke@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "input"
            ])
            .status
            .success()
        );
        let identity = CandidateIdentity::capture(temp.path()).unwrap();
        std::fs::write(temp.path().join("input"), "two").unwrap();
        assert!(identity.check_unchanged().is_err());
        std::fs::write(temp.path().join("input"), "one").unwrap();
        std::fs::write(temp.path().join("untracked"), "new build input").unwrap();
        assert!(identity.check_unchanged().is_err());
    }
}
