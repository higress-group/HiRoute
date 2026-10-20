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
    let minimum = env!("CARGO_PKG_VERSION").to_owned();
    let date = "2026-10-18".to_owned();
    FeedPayload {
        schema: MODEL_FEED_SCHEMA.into(),
        sequence,
        issued_at: "2026-10-18T00:00:00Z".into(),
        source_revision: "1".repeat(40),
        featured: vec![],
        records: vec![
            FeedRecord::Model {
                min_client_version: minimum.clone(),
                included_on: date.clone(),
                record: Box::new(FeedModel {
                    definition,
                    native_reasoning: Some(reasoning),
                    canonical,
                }),
            },
            FeedRecord::ModelEndpointCapability {
                min_client_version: minimum.clone(),
                included_on: date.clone(),
                record: capability,
            },
            FeedRecord::Offer {
                min_client_version: minimum,
                included_on: date,
                record: offer,
            },
        ],
    }
}
