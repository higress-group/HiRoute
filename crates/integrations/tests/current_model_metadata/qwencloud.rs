use hiroute_domain::{ConnectorRegistryBundleV1, ReleaseModelDataBundleV2, UpstreamProtocol};
use hiroute_integrations::{TrustedReleaseCatalog, compute::matching_endpoint_metadata};
use serde_json::{json, to_value};

use super::{encoded, fixture};

const PRODUCT: &str = "qwencloud-payg-us-east-1-us-only";
const MODEL: &str = "qwen-4-max";
const UPSTREAM: &str = "qwen4-max";
const HOST: &str = "https://us-east-1.maas.qwencloudapi.com";
const BEIJING_PROFILE: &str = "endpoint.bailian.payg.cn.v1";
const WIRES: [(UpstreamProtocol, &str, &str); 3] = [
    (
        UpstreamProtocol::Messages,
        "anthropic-messages",
        "/apps/anthropic/v1/messages",
    ),
    (
        UpstreamProtocol::ChatCompletions,
        "openai-chat",
        "/compatible-mode/v1/chat/completions",
    ),
    (
        UpstreamProtocol::Responses,
        "openai-responses",
        "/compatible-mode/v1/responses",
    ),
];

fn catalog(
    registry: &ConnectorRegistryBundleV1,
    data: &ReleaseModelDataBundleV2,
) -> TrustedReleaseCatalog {
    let (manifest, registry_bytes, data_bytes) = encoded(registry, data);
    TrustedReleaseCatalog::load_release_facts(&manifest, &registry_bytes, &data_bytes)
        .unwrap()
        .with_embedded_official_data(
            serde_json::from_slice(include_bytes!(
                "../../../../assets/release-facts/current/bundle/official-model-data.json"
            ))
            .unwrap(),
        )
        .unwrap()
}

#[test]
fn qwencloud_virginia_us_only_facts_require_an_explicit_product_lookup() {
    let (registry, data) = fixture();
    let catalog = catalog(&registry, &data);
    let metadata = catalog.model_metadata();
    let product = metadata
        .access_products
        .iter()
        .find(|product| product.product_key == PRODUCT)
        .unwrap();
    assert_eq!(product.provider, "QwenCloud");
    assert_eq!(product.product_kind, "api-payg");
    assert_eq!(product.documented_upstream_model_ids, [UPSTREAM]);
    assert!(product.region_scope.contains("US-only inference"));
    assert!(product.restriction.contains("fixed US deployment scope"));
    assert!(product.restriction.contains("Global workspace"));
    assert!(
        product
            .disposition
            .iter()
            .any(|value| value == "needs-runtime-qualification")
    );
    assert_eq!(product.interfaces.len(), WIRES.len());
    for (_, protocol, path) in WIRES {
        let interface = product
            .interfaces
            .iter()
            .find(|interface| interface.protocol == protocol)
            .unwrap();
        assert_eq!(
            format!(
                "{}{}",
                interface.base_url.as_deref().unwrap(),
                interface.request_path.as_deref().unwrap()
            ),
            format!("{HOST}{path}")
        );
    }

    let snapshot = catalog.official_snapshot();
    let prices: Vec<_> = snapshot
        .data
        .pricing_facts
        .iter()
        .filter(|price| price.model_key == MODEL)
        .collect();
    assert_eq!(
        prices.len(),
        2,
        "only Beijing and Virginia US-only are recorded"
    );
    let price = prices
        .iter()
        .find(|price| price.product_key == PRODUCT)
        .unwrap();
    assert_eq!(price.upstream_model_id, UPSTREAM);
    assert_eq!(price.currency, "USD");
    assert_eq!(price.unit, "per-1m-tokens");
    assert_eq!(price.price_state, "exact");
    assert_eq!(
        to_value(&price.schedules).unwrap(),
        json!([
            {
                "schedule_key": "standard",
                "condition": {
                    "kind": "region", "region": "us-east-1", "deployment_scope": "us-only",
                    "minimum_input_tokens": 1, "maximum_input_tokens": 1_000_000
                },
                "application": "whole-request",
                "rates": {"input": "1.50", "output": "4.50", "cache-read": "0.075"}
            },
            {
                "schedule_key": "standard-cache-write-300s",
                "condition": {
                    "kind": "region", "region": "us-east-1", "deployment_scope": "us-only",
                    "cache_ttl_seconds": 300
                },
                "application": "whole-request",
                "rates": {"cache-write": "1.875"}
            },
            {
                "schedule_key": "standard-cache-write-3600s",
                "condition": {
                    "kind": "region", "region": "us-east-1", "deployment_scope": "us-only",
                    "cache_ttl_seconds": 3600
                },
                "application": "whole-request",
                "rates": {"cache-write": "3"}
            }
        ])
    );
}

#[test]
fn qwencloud_shared_wire_does_not_match_us_only_metadata_without_scope() {
    let (mut registry, data) = fixture();
    let beijing = catalog(&registry, &data);
    let profile = registry
        .endpoint_profiles
        .iter_mut()
        .find(|profile| profile.endpoint_profile_id == BEIJING_PROFILE)
        .unwrap();
    // Emulate an installed Virginia destination. Global and unknown workspaces
    // supply the same wire identity to this matcher.
    for endpoint in &mut profile.protocol_endpoints {
        let (_, _, path) = WIRES
            .iter()
            .find(|wire| wire.0 == endpoint.protocol)
            .unwrap();
        assert_eq!(endpoint.request_path, *path);
        endpoint.base_url = HOST.into();
    }
    let shared_wire = catalog(&registry, &data);
    for (protocol, _, _) in WIRES {
        let capability = shared_wire
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|capability| {
                capability.endpoint_profile_id == BEIJING_PROFILE
                    && capability.upstream_model_id == UPSTREAM
                    && capability.upstream_protocol == protocol
            })
            .unwrap();
        let control = matching_endpoint_metadata(&beijing, capability);
        assert_eq!(control.len(), 1);
        assert_eq!(control[0].product_key, "bailian-payg-cn");
        assert!(
            matching_endpoint_metadata(&shared_wire, capability).is_empty(),
            "the shared Virginia wire must not give {protocol:?} US-only defaults or prices"
        );
    }
}
