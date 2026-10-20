#![forbid(unsafe_code)]

//! Explicit, offline ReleaseFacts compiler. No build script invokes this crate.

pub mod feed;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use hiroute_domain::{
    CanonicalDigest, ComputeContractError, ConnectorRegistryBundleV1, RELEASE_FACTS_SCHEMA_V2,
    RELEASE_FACTS_TOOL_VERSION_V2, ReleaseFactsManifestV2, ReleaseModelDataBundleV2,
};
use hiroute_integrations::builtin_agent_profiles_artifact;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const RELEASE_FACTS_COMPILER_INPUT_SCHEMA_V2: &str = "hiroute.release-facts-compiler-input/v2";
pub const CONNECTOR_REGISTRY_FILE: &str = "connector-registry.json";
pub const MODEL_DATA_FILE: &str = "model-data.json";
pub const RELEASE_FACTS_MANIFEST_FILE: &str = "manifest.json";
pub const AGENT_PROFILES_FILE: &str = "agent-profiles.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFactsCompilerInputV2 {
    pub schema: String,
    pub tool_version: String,
    pub catalog_id: String,
    pub product_release: String,
    pub sequence: u64,
    pub connector_registry: ConnectorRegistryBundleV1,
    pub model_data: ReleaseModelDataBundleV2,
}

impl ReleaseFactsCompilerInputV2 {
    pub fn validate(&self) -> Result<(), ReleaseFactsCompilerError> {
        if self.schema != RELEASE_FACTS_COMPILER_INPUT_SCHEMA_V2
            || self.tool_version != RELEASE_FACTS_TOOL_VERSION_V2
            || self.product_release != self.connector_registry.product_release
            || self.product_release != self.model_data.data.product_release
            || self.sequence == 0
            || !valid_release_id(&self.product_release)
            || !valid_release_id(&self.catalog_id)
        {
            return Err(ReleaseFactsCompilerError::InvalidInput);
        }
        self.model_data
            .cross_reference_digest(&self.connector_registry)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledReleaseFacts {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl CompiledReleaseFacts {
    pub fn digests(&self) -> BTreeMap<String, CanonicalDigest> {
        self.files
            .iter()
            .map(|(name, bytes)| (name.clone(), CanonicalDigest::of_bytes(bytes)))
            .collect()
    }
}

pub fn compile_release_facts(
    input: &ReleaseFactsCompilerInputV2,
) -> Result<CompiledReleaseFacts, ReleaseFactsCompilerError> {
    input.validate()?;
    let mut normalized = input.clone();
    normalize_v2(&mut normalized)?;
    normalized.validate()?;

    let registry_bytes = canonical_sorted_json_line(&normalized.connector_registry)?;
    let model_data_bytes = canonical_sorted_json_line(&normalized.model_data)?;
    let cross_reference_digest = normalized
        .model_data
        .cross_reference_digest(&normalized.connector_registry)?;
    compile_normalized_release_facts(
        normalized.product_release,
        normalized.sequence,
        normalized.catalog_id,
        registry_bytes,
        model_data_bytes,
        cross_reference_digest,
        MODEL_DATA_FILE,
    )
}

#[allow(clippy::too_many_arguments)]
fn compile_normalized_release_facts(
    product_release: String,
    sequence: u64,
    catalog_id: String,
    registry_bytes: Vec<u8>,
    model_data_bytes: Vec<u8>,
    cross_reference_digest: CanonicalDigest,
    model_data_file: &str,
) -> Result<CompiledReleaseFacts, ReleaseFactsCompilerError> {
    let manifest = ReleaseFactsManifestV2 {
        schema: RELEASE_FACTS_SCHEMA_V2.to_owned(),
        tool_version: RELEASE_FACTS_TOOL_VERSION_V2.to_owned(),
        catalog_id,
        product_release,
        sequence,
        connector_registry_digest: CanonicalDigest::of_bytes(&registry_bytes),
        model_data_digest: CanonicalDigest::of_bytes(&model_data_bytes),
        cross_reference_digest,
    };
    manifest.validate_shape()?;
    let manifest_bytes = canonical_json_line(&manifest)?;

    let profiles = builtin_agent_profiles_artifact();
    profiles.validate()?;
    let profile_bytes = canonical_json_line(&profiles)?;
    Ok(CompiledReleaseFacts {
        files: BTreeMap::from([
            (AGENT_PROFILES_FILE.to_owned(), profile_bytes),
            (CONNECTOR_REGISTRY_FILE.to_owned(), registry_bytes),
            (model_data_file.to_owned(), model_data_bytes),
            (RELEASE_FACTS_MANIFEST_FILE.to_owned(), manifest_bytes),
        ]),
    })
}

pub fn read_compiler_input(
    path: &Path,
) -> Result<ReleaseFactsCompilerInputV2, ReleaseFactsCompilerError> {
    let bytes = fs::read(path).map_err(|source| ReleaseFactsCompilerError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(ReleaseFactsCompilerError::Json)
}

pub fn generate(
    input_path: &Path,
    output_directory: &Path,
) -> Result<CompiledReleaseFacts, ReleaseFactsCompilerError> {
    let input = read_compiler_input(input_path)?;
    let compiled = compile_release_facts(&input)?;
    fs::create_dir_all(output_directory).map_err(|source| ReleaseFactsCompilerError::Io {
        path: output_directory.to_owned(),
        source,
    })?;
    let actual_names = output_names(output_directory)?;
    let expected_names = compiled.files.keys().cloned().collect::<BTreeSet<_>>();
    if !actual_names.is_subset(&expected_names) {
        return Err(ReleaseFactsCompilerError::OutputSetMismatch {
            expected: expected_names,
            actual: actual_names,
        });
    }
    for (name, bytes) in &compiled.files {
        let path = output_directory.join(name);
        fs::write(&path, bytes).map_err(|source| ReleaseFactsCompilerError::Io { path, source })?;
    }
    Ok(compiled)
}

pub fn check(
    input_path: &Path,
    output_directory: &Path,
) -> Result<CompiledReleaseFacts, ReleaseFactsCompilerError> {
    let input = read_compiler_input(input_path)?;
    let compiled = compile_release_facts(&input)?;
    let actual_names = output_names(output_directory)?;
    let expected_names = compiled.files.keys().cloned().collect::<BTreeSet<_>>();
    if actual_names != expected_names {
        return Err(ReleaseFactsCompilerError::OutputSetMismatch {
            expected: expected_names,
            actual: actual_names,
        });
    }
    for (name, expected) in &compiled.files {
        let path = output_directory.join(name);
        let actual =
            fs::read(&path).map_err(|source| ReleaseFactsCompilerError::Io { path, source })?;
        if &actual != expected {
            return Err(ReleaseFactsCompilerError::OutputBytesMismatch(name.clone()));
        }
    }
    Ok(compiled)
}

fn output_names(output_directory: &Path) -> Result<BTreeSet<String>, ReleaseFactsCompilerError> {
    fs::read_dir(output_directory)
        .map_err(|source| ReleaseFactsCompilerError::Io {
            path: output_directory.to_owned(),
            source,
        })?
        .map(|entry| {
            entry
                .map_err(|source| ReleaseFactsCompilerError::Io {
                    path: output_directory.to_owned(),
                    source,
                })
                .and_then(|entry| {
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| ReleaseFactsCompilerError::UnexpectedOutputPath)
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()
}

fn canonical_json_line<T: Serialize>(value: &T) -> Result<Vec<u8>, ReleaseFactsCompilerError> {
    let mut bytes = serde_json::to_vec(value).map_err(ReleaseFactsCompilerError::Json)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_sorted_json_line<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, ReleaseFactsCompilerError> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            serde_json::Value::Object(values) => {
                let mut entries = values.into_iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key, sorted(value)))
                        .collect(),
                )
            }
            other => other,
        }
    }

    let value = serde_json::to_value(value).map_err(ReleaseFactsCompilerError::Json)?;
    canonical_json_line(&sorted(value))
}

fn normalize_v2(input: &mut ReleaseFactsCompilerInputV2) -> Result<(), ReleaseFactsCompilerError> {
    normalize_registry(&mut input.connector_registry);
    normalize_model_data(&mut input.model_data.data);
    input
        .model_data
        .rating_snapshot
        .models
        .sort_by(|left, right| {
            left.model_configuration_id
                .cmp(&right.model_configuration_id)
        });
    input
        .model_data
        .rating_snapshot
        .records
        .sort_by(|left, right| {
            (&left.model_configuration_id, &left.native_configuration)
                .cmp(&(&right.model_configuration_id, &right.native_configuration))
        });
    input.model_data.rating_snapshot.model_catalog_digest =
        CanonicalDigest::of(&input.model_data.data.models)
            .map_err(|_| ReleaseFactsCompilerError::InvalidInput)?;
    input.model_data.rating_snapshot.digest = input.model_data.rating_snapshot.computed_digest()?;
    Ok(())
}

fn normalize_registry(registry: &mut ConnectorRegistryBundleV1) {
    registry
        .connectors
        .sort_by(|left, right| left.connector_id.cmp(&right.connector_id));
    for connector in &mut registry.connectors {
        connector.endpoint_profile_refs.sort();
        connector.required_secret_slots.sort();
    }
    registry
        .endpoint_profiles
        .sort_by(|left, right| left.endpoint_profile_id.cmp(&right.endpoint_profile_id));
    for profile in &mut registry.endpoint_profiles {
        profile
            .protocol_endpoints
            .sort_by(|left, right| left.protocol_endpoint_id.cmp(&right.protocol_endpoint_id));
    }
    registry
        .connection_options
        .sort_by(|left, right| left.connection_option_id.cmp(&right.connection_option_id));
}

fn normalize_model_data(model_data: &mut hiroute_domain::ModelDataBundleV1) {
    model_data.models.sort_by(|left, right| {
        left.model_configuration_id
            .cmp(&right.model_configuration_id)
    });
    model_data
        .model_endpoint_capabilities
        .sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
    model_data.ratings.sort_by(|left, right| {
        left.model_configuration_id
            .cmp(&right.model_configuration_id)
    });
    model_data
        .offers
        .sort_by(|left, right| left.offer_id.cmp(&right.offer_id));
    for offer in &mut model_data.offers {
        offer.model_configuration_ids.sort();
    }
    model_data
        .free_offers
        .sort_by(|left, right| left.free_offer_id.cmp(&right.free_offer_id));
    for offer in &mut model_data.free_offers {
        offer.model_configuration_ids.sort();
    }
    model_data
        .price_rates
        .sort_by(|left, right| left.price_rate_id.cmp(&right.price_rate_id));
}

fn valid_release_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("//")
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

#[derive(Debug, Error)]
pub enum ReleaseFactsCompilerError {
    #[error("ReleaseFacts compiler input is invalid")]
    InvalidInput,
    #[error(transparent)]
    Contract(#[from] ComputeContractError),
    #[error("cannot encode or parse ReleaseFacts JSON: {0}")]
    Json(serde_json::Error),
    #[error("ReleaseFacts compiler input schema is unsupported")]
    UnsupportedInputSchema,
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ReleaseFacts output contains a non-Unicode path")]
    UnexpectedOutputPath,
    #[error("ReleaseFacts output file set differs: expected {expected:?}, actual {actual:?}")]
    OutputSetMismatch {
        expected: BTreeSet<String>,
        actual: BTreeSet<String>,
    },
    #[error("ReleaseFacts output bytes differ for {0}")]
    OutputBytesMismatch(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn current_input_path() -> PathBuf {
        root().join("assets/release-facts/current/compiler-input.json")
    }

    fn current_bundle_path() -> PathBuf {
        root().join("assets/release-facts/current/bundle")
    }

    #[test]
    fn release_facts_compile_is_byte_deterministic() {
        let input = read_compiler_input(&current_input_path()).unwrap();
        let first = compile_release_facts(&input).unwrap();
        let second = compile_release_facts(&input).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.files.len(), 4);
        assert!(first.files[CONNECTOR_REGISTRY_FILE].ends_with(b"\n"));
        assert!(first.files[MODEL_DATA_FILE].ends_with(b"\n"));
    }

    #[test]
    fn checked_in_current_bundle_is_exact_compiler_output() {
        let input = read_compiler_input(&current_input_path()).unwrap();
        let compiled = compile_release_facts(&input).unwrap();
        let artifacts = current_bundle_path();
        assert_eq!(
            output_names(&artifacts).unwrap(),
            compiled.files.keys().cloned().collect()
        );
        for (name, expected) in compiled.files {
            assert_eq!(fs::read(artifacts.join(name)).unwrap(), expected);
        }
    }

    #[test]
    fn release_facts_compiler_rejects_model_missing_from_rating_snapshot() {
        let mut input = read_compiler_input(&current_input_path()).unwrap();
        let omitted_model_id = input.model_data.data.models[0]
            .model_configuration_id
            .clone();
        input
            .model_data
            .rating_snapshot
            .models
            .retain(|model| model.model_configuration_id != omitted_model_id);
        input.model_data.rating_snapshot.digest =
            input.model_data.rating_snapshot.computed_digest().unwrap();
        assert!(
            input
                .model_data
                .data
                .models
                .iter()
                .any(|model| model.model_configuration_id == omitted_model_id)
        );
        assert!(matches!(
            compile_release_facts(&input),
            Err(ReleaseFactsCompilerError::Contract(
                ComputeContractError::CrossReference
            ))
        ));
    }

    #[test]
    fn release_facts_check_detects_extra_and_changed_files() {
        let directory = tempfile::tempdir().unwrap();
        let input_path = directory.path().join("input.json");
        fs::copy(current_input_path(), &input_path).unwrap();
        let output = directory.path().join("output");
        generate(&input_path, &output).unwrap();
        check(&input_path, &output).unwrap();
        fs::write(output.join("extra"), b"unexpected").unwrap();
        assert!(matches!(
            check(&input_path, &output),
            Err(ReleaseFactsCompilerError::OutputSetMismatch { .. })
        ));
    }
}
