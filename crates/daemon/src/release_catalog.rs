//! Fail-closed loader for the one complete ReleaseFacts catalog embedded in this daemon build.

use hiroute_integrations::TrustedReleaseCatalog;

const BUNDLED_MANIFEST: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
const BUNDLED_REGISTRY: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json");
const BUNDLED_MODEL_DATA: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/model-data.json");

pub(crate) fn load_production_release_catalog() -> Result<TrustedReleaseCatalog, String> {
    let catalog = TrustedReleaseCatalog::load_bundled_release_facts(
        BUNDLED_MANIFEST,
        BUNDLED_MANIFEST,
        BUNDLED_REGISTRY,
        BUNDLED_MODEL_DATA,
    )
    .map_err(|error| format!("ReleaseFacts verification failed: {error}"))?;
    let snapshot = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/official-model-data.json"
    ))
    .map_err(|_| "invalid compiled official model snapshot")?;
    let catalog = catalog
        .with_embedded_official_data(snapshot)
        .map_err(|error| error.to_string())?;
    #[cfg(feature = "integration-test-hooks")]
    let catalog = crate::model_feed::product_fixture::catalog(catalog)?;
    Ok(catalog)
}

#[cfg(test)]
pub(crate) fn fixture_catalog() -> TrustedReleaseCatalog {
    load_production_release_catalog().unwrap()
}

#[cfg(test)]
pub(crate) fn current_fixture_catalog() -> TrustedReleaseCatalog {
    fixture_catalog()
}

/// The discovery fixtures deliberately consume the same executable endpoint facts as production.
#[cfg(test)]
pub(crate) fn routable_discovery_fixture_catalog() -> TrustedReleaseCatalog {
    fixture_catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_loader_uses_the_exact_embedded_catalog() {
        let catalog = load_production_release_catalog().unwrap();
        assert_eq!(catalog.release_facts_manifest().sequence, 1);
        assert_eq!(
            catalog.current_release_model_data().data.product_release,
            "mvp-current"
        );
        assert_eq!(
            catalog.release_facts_manifest().connector_registry_digest,
            hiroute_domain::CanonicalDigest::of_bytes(BUNDLED_REGISTRY)
        );
        assert_eq!(
            catalog.release_facts_manifest().model_data_digest,
            hiroute_domain::CanonicalDigest::of_bytes(BUNDLED_MODEL_DATA)
        );
        let zhipu = catalog
            .registry()
            .endpoint_profiles
            .iter()
            .find(|profile| profile.endpoint_profile_id == "endpoint.zhipu.coding-plan.cn.v1")
            .unwrap()
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == hiroute_domain::UpstreamProtocol::Messages)
            .unwrap();
        assert_eq!(
            zhipu.authentication_semantics,
            Some(hiroute_domain::GatewayAuthenticationSemanticsV1::Bearer)
        );
        assert_eq!(
            zhipu.required_headers,
            vec![("anthropic-version".into(), "2023-06-01".into())]
        );
    }
}
