//! Exercise real bundled validation with synthetic descriptive growth, not new model facts.
use hiroute_domain::{
    CanonicalDigest, ConnectorRegistryBundleV1, MAX_RELEASE_BUNDLE_BYTES,
    MAX_RELEASE_MODEL_DATA_BYTES, OfficialModelSnapshotV1, ReleaseFactsManifestV2,
    ReleaseModelDataBundleV2,
};
use hiroute_integrations::{ReleaseVerificationError, TrustedReleaseCatalog};

const MANIFEST: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
const REGISTRY: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json");
const MODEL_DATA: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/model-data.json");
const OFFICIAL_DATA: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/official-model-data.json");

struct BundledFixture {
    manifest: Vec<u8>,
    model_data: Vec<u8>,
    official_data: Vec<u8>,
    records: usize,
}

impl BundledFixture {
    fn expanded() -> Self {
        let mut data: ReleaseModelDataBundleV2 = serde_json::from_slice(MODEL_DATA).unwrap();
        let originals = data.metadata_catalog.model_records.clone();
        data.metadata_catalog.source_catalog_digest =
            CanonicalDigest::of_bytes(b"synthetic-bundled-size-regression");
        let mut bytes = serde_json::to_vec(&data).unwrap().len();
        for index in 0..100_000 {
            let mut record = originals[index % originals.len()].clone();
            record.model_record_key = format!("synthetic/{index:06}/{}", record.model_record_key);
            let added = serde_json::to_vec(&record).unwrap().len() + 1;
            if bytes + added > MAX_RELEASE_MODEL_DATA_BYTES {
                break;
            }
            data.metadata_catalog.model_records.push(record);
            bytes += added;
        }
        data.metadata_catalog
            .model_records
            .sort_by(|left, right| left.model_record_key.cmp(&right.model_record_key));
        let mut model_data = serde_json::to_vec(&data).unwrap();
        assert_eq!(model_data.len(), bytes);
        assert!(MAX_RELEASE_MODEL_DATA_BYTES - model_data.len() < 16 * 1024);
        // Valid JSON whitespace tests the inclusive byte guard exactly; almost all growth
        // comes from complete records with retained provenance and reference closure.
        model_data.resize(MAX_RELEASE_MODEL_DATA_BYTES, b' ');
        let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(REGISTRY).unwrap();
        let mut manifest: ReleaseFactsManifestV2 = serde_json::from_slice(MANIFEST).unwrap();
        manifest.model_data_digest = CanonicalDigest::of_bytes(&model_data);
        manifest.cross_reference_digest = data.cross_reference_digest(&registry).unwrap();
        let mut snapshot: OfficialModelSnapshotV1 = serde_json::from_slice(OFFICIAL_DATA).unwrap();
        snapshot.data.metadata_catalog = data.metadata_catalog;
        snapshot.data_digest = CanonicalDigest::of(&snapshot.data).unwrap();
        Self {
            manifest: serde_json::to_vec(&manifest).unwrap(),
            model_data,
            records: snapshot.data.metadata_catalog.model_records.len(),
            official_data: serde_json::to_vec(&snapshot).unwrap(),
        }
    }

    fn load(&self) -> TrustedReleaseCatalog {
        // The same validation and official-snapshot attachment used by the daemon's
        // production loader; synthetic bytes replace only the compiled-in test inputs.
        TrustedReleaseCatalog::load_bundled_release_facts(
            &self.manifest,
            &self.manifest,
            REGISTRY,
            &self.model_data,
        )
        .unwrap()
        .with_embedded_official_data(serde_json::from_slice(&self.official_data).unwrap())
        .unwrap()
    }
}

#[test]
fn bundled_model_data_accepts_eight_mib_with_complete_metadata() {
    let fixture = BundledFixture::expanded();
    assert!(fixture.model_data.len() > MAX_RELEASE_BUNDLE_BYTES);
    let catalog = fixture.load();
    assert_eq!(
        catalog.model_metadata().model_records.len(),
        fixture.records
    );
    assert_eq!(
        catalog
            .official_snapshot()
            .data
            .metadata_catalog
            .model_records
            .len(),
        fixture.records
    );
    assert_eq!(
        catalog.registry(),
        &serde_json::from_slice::<ConnectorRegistryBundleV1>(REGISTRY).unwrap()
    );
    assert_eq!(
        catalog.model_data().models,
        serde_json::from_slice::<ReleaseModelDataBundleV2>(MODEL_DATA)
            .unwrap()
            .data
            .models
    );

    let mut oversized = fixture.model_data.clone();
    oversized.push(b' ');
    assert!(matches!(
        TrustedReleaseCatalog::load_release_facts(&fixture.manifest, REGISTRY, &oversized),
        Err(ReleaseVerificationError::BundleTooLarge)
    ));
    // The enlarged budget must not bypass binding the exact model bytes to the manifest.
    let mut changed = fixture.model_data.clone();
    *changed.last_mut().unwrap() = b'\n';
    assert!(matches!(
        TrustedReleaseCatalog::load_release_facts(&fixture.manifest, REGISTRY, &changed),
        Err(ReleaseVerificationError::DigestMismatch)
    ));
}

#[test]
fn registry_retains_its_independent_two_mib_boundary() {
    let mut registry = REGISTRY.to_vec();
    registry.resize(MAX_RELEASE_BUNDLE_BYTES, b' ');
    let mut manifest: ReleaseFactsManifestV2 = serde_json::from_slice(MANIFEST).unwrap();
    manifest.connector_registry_digest = CanonicalDigest::of_bytes(&registry);
    let manifest = serde_json::to_vec(&manifest).unwrap();
    TrustedReleaseCatalog::load_bundled_release_facts(&manifest, &manifest, &registry, MODEL_DATA)
        .unwrap();
    registry.push(b' ');
    assert!(matches!(
        TrustedReleaseCatalog::load_release_facts(&manifest, &registry, MODEL_DATA),
        Err(ReleaseVerificationError::BundleTooLarge)
    ));
}

#[test]
#[ignore = "opt-in component benchmark; use --release --ignored --nocapture --test-threads=1"]
fn bundled_model_data_initialization_timing() {
    let expanded = BundledFixture::expanded();
    let baseline = BundledFixture {
        manifest: MANIFEST.to_vec(),
        model_data: MODEL_DATA.to_vec(),
        official_data: OFFICIAL_DATA.to_vec(),
        records: serde_json::from_slice::<ReleaseModelDataBundleV2>(MODEL_DATA)
            .unwrap()
            .metadata_catalog
            .model_records
            .len(),
    };
    for (label, fixture) in [("current", baseline), ("synthetic_8mib", expanded)] {
        for _ in 0..3 {
            std::hint::black_box(fixture.load());
        }
        let mut samples = Vec::new();
        for _ in 0..31 {
            let start = std::time::Instant::now();
            let catalog = std::hint::black_box(fixture.load());
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            drop(catalog);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{label}: model_bytes={} records={} rounds=31 warmups=3 median_ms={:.3} p95_ms={:.3}",
            fixture.model_data.len(),
            fixture.records,
            samples[15],
            samples[29]
        );
    }
}
