//! Synthetic new model, closed over real embedded endpoint and offer authority.
use hiroute_integrations::model_feed::*;

pub(crate) const MODEL: &str = "model.feed.regression";

pub(crate) fn publication(sequence: u64) -> FeedPayload {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let original = "model.bailian.qwen3-max-2026-01-23";
    let mut definition = catalog.model_data().model(original).unwrap().clone();
    definition.model_configuration_id = MODEL.into();
    definition.display_name = "Feed regression model".into();
    let mut capability = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .find(|c| c.model_configuration_id == original)
        .unwrap()
        .clone();
    capability.model_configuration_id = MODEL.into();
    capability.capability_id = "capability.feed.regression".into();
    capability.upstream_model_id = "feed-regression".into();
    let mut reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|r| r.model_configuration_id == original)
        .unwrap()
        .clone();
    reasoning.model_configuration_id = MODEL.into();
    let mut canonical = catalog.model_metadata().canonical_models[0].clone();
    canonical.model_key = "feed-regression".into();
    canonical.upstream_ids = vec!["feed-regression".into()];
    let mut offer = catalog
        .model_data()
        .offer("offer.bailian.payg.cn")
        .unwrap()
        .clone();
    offer.model_configuration_ids.push(MODEL.into());
    let mut snapshot = catalog.official_snapshot();
    let data = &mut snapshot.data;
    data.models.push(definition);
    data.models
        .sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
    data.model_endpoint_capabilities.push(capability);
    let offer_id = offer.offer_id.clone();
    *data
        .offers
        .iter_mut()
        .find(|o| o.offer_id == offer_id)
        .unwrap() = offer;
    data.rating_snapshot.models.push(reasoning);
    data.rating_snapshot
        .models
        .sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
    data.rating_snapshot.model_catalog_digest =
        hiroute_domain::CanonicalDigest::of(&data.models).unwrap();
    data.rating_snapshot.digest = data.rating_snapshot.computed_digest().unwrap();
    data.metadata_catalog.canonical_models.push(canonical);
    data.metadata_catalog
        .canonical_models
        .sort_by(|a, b| a.model_key.cmp(&b.model_key));
    snapshot.data_revision = sequence.max(2);
    snapshot.data_digest = hiroute_domain::CanonicalDigest::of(data).unwrap();
    FeedPayload::from_snapshot(
        snapshot,
        sequence,
        "2026-10-18T00:00:00Z".into(),
        "1".repeat(40),
        vec![],
    )
}
