//! The digest covers complete default impact; pagination only bounds its presentation.
use super::*;
use hiroute_domain::{ComputeManagementRepositoryPort, PriceValuationKindV1, SourcePriceSettingV1};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    digest: CanonicalDigest,
    offset: usize,
}

fn prices(
    facts: &hiroute_application::prices::PriceControlFactsV1,
    source: &str,
    model: &str,
) -> BTreeMap<String, Value> {
    facts.entries.iter().filter(|e| e.target.source_id == source && match &e.target.model_identity {
        hiroute_domain::PriceModelIdentityV1::CatalogModel(id) | hiroute_domain::PriceModelIdentityV1::LocalModel(id) => id == model,
    }).map(|e| {
        let value = match e.source_override.as_ref().map(|o| &o.setting) {
            Some(SourcePriceSettingV1::Set {rates}) => json!({"manual":rates}),
            _ => json!({"rules":e.catalog_rules.iter().map(|r| (&r.rates,&r.schedule)).collect::<Vec<_>>(),"legacy":e.legacy_overrides}),
        };
        (format!("{}:{}", e.target.currency, match e.target.valuation_kind {PriceValuationKindV1::UsageEstimate => "usage_estimate", PriceValuationKindV1::ApiEquivalent => "api_equivalent"}),value)
    }).collect()
}

pub(super) fn preview(
    adapter: &LocalControlAdapter,
    query: ModelFeedRemovalQueryV2,
) -> Result<(ModelFeedRemovalPreviewV2, CanonicalDigest), ErrorCode> {
    let page_size = query.page_size.unwrap_or(25);
    if !(1..=100).contains(&page_size) || query.cursor.as_ref().is_some_and(|s| s.len() > 1024) {
        return Err(ErrorCode::InvalidArguments);
    }
    let feed = adapter
        .model_feed
        .as_ref()
        .ok_or(ErrorCode::DaemonUnavailable)?;
    let before = feed.removal_digest().map_err(feed_error)?;
    let catalog = feed.current();
    let fallback = ReleaseCatalogSnapshot {
        catalog: feed.embedded_catalog().as_ref().clone(),
        generation: catalog
            .generation
            .checked_add(1)
            .ok_or(ErrorCode::Internal)?,
        accepted_sequence: catalog.accepted_sequence,
    };
    let status = feed.status().map_err(feed_error)?;
    let stores = adapter
        .stores_lock()
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let management = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let versions = stores
        .control()
        .published_plan_versions(&WorkspaceId::default())
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let current_prices = adapter
        .price_facts_for(&stores, None, &catalog)
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let fallback_prices = adapter
        .price_facts_for(&stores, None, &fallback)
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let mut all = Vec::new();
    let mut preferences = Vec::new();
    let mut all_publications = Vec::new();
    let mut source_publications: BTreeMap<String, Vec<hiroute_domain::PlanExecutionRef>> =
        BTreeMap::new();
    for source in &management.sources {
        let preference = stores
            .control()
            .source_default_preferences(&source.source_id)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        let current = super::super::model_connections::defaults::project_source(
            &catalog,
            source,
            &preference,
        );
        let after = super::super::model_connections::defaults::project_source(
            &fallback,
            source,
            &preference,
        );
        preferences.push((source.source_id.clone(), source.revision, preference));
        for (old, new) in current.models.iter().zip(&after.models) {
            let Some(id) = old.catalog_configuration_id.as_ref() else {
                continue;
            };
            let mut changes = BTreeMap::new();
            macro_rules! compare {($field:ident) => {if old.capabilities.$field != new.capabilities.$field {changes.insert(stringify!($field).to_owned(),json!({"current":old.capabilities.$field.value,"fallback":new.capabilities.$field.value}));}}}
            compare!(tool);
            compare!(vision);
            compare!(streaming);
            compare!(context_tokens);
            compare!(max_output_tokens);
            compare!(native_reasoning);
            let old_available = catalog.model_data().model(id).is_some();
            let new_available = fallback.model_data().model(id).is_some();
            if old_available != new_available {
                changes.insert(
                    "availability".into(),
                    json!({"current":old_available,"fallback":new_available}),
                );
            }
            let old_prices = prices(&current_prices, &source.source_id, id);
            let new_prices = prices(&fallback_prices, &source.source_id, id);
            for scope in old_prices
                .keys()
                .chain(new_prices.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                if old_prices.get(scope) != new_prices.get(scope) {
                    changes.insert(
                        format!("prices:{scope}"),
                        json!({"current":old_prices.get(scope),"fallback":new_prices.get(scope)}),
                    );
                }
            }
            if changes.is_empty() {
                continue;
            }
            let published = versions
                .iter()
                .filter(|v| candidates(v).any(|c| c.binding_id == old.binding_id))
                .map(|v| v.reference.clone())
                .collect::<Vec<_>>();
            let source_refs = source_publications
                .entry(source.source_id.clone())
                .or_default();
            for reference in &published {
                if !all_publications.contains(reference) {
                    all_publications.push(reference.clone());
                }
                if !source_refs.contains(reference) {
                    source_refs.push(reference.clone());
                }
            }
            // One capability or price/currency scope per row, independent of publication history size.
            for (field, change) in changes {
                all.push(ModelFeedDefaultImpactV2 {
                    source_id: source.source_id.clone(),
                    source_name: source.display_name.clone(),
                    binding_id: old.binding_id.clone(),
                    model_configuration_id: id.clone(),
                    changes: BTreeMap::from([(field, change)]),
                    published_revision_count: published.len(),
                });
            }
        }
    }
    all.sort_by(|a, b| {
        (&a.source_id, &a.binding_id, a.changes.keys().next()).cmp(&(
            &b.source_id,
            &b.binding_id,
            b.changes.keys().next(),
        ))
    });
    let digest = CanonicalDigest::of(&(
        &before,
        &management.revisions,
        &preferences,
        &all,
        &all_publications,
    ))
    .map_err(|_| ErrorCode::Internal)?;
    drop(stores);
    if before != feed.removal_digest().map_err(feed_error)? {
        return Err(ErrorCode::ChangePreviewStale);
    }
    let offset = match query.cursor {
        Some(cursor) => {
            let cursor: Cursor =
                serde_json::from_str(&cursor).map_err(|_| ErrorCode::InvalidArguments)?;
            if cursor.digest != digest {
                return Err(ErrorCode::ChangePreviewStale);
            }
            cursor.offset
        }
        None => 0,
    };
    if offset > all.len() {
        return Err(ErrorCode::InvalidArguments);
    }
    let mut impact = Vec::new();
    let mut bytes = 4096;
    for record in all.iter().skip(offset).take(page_size as usize) {
        let size = serde_json::to_vec(record)
            .map_err(|_| ErrorCode::Internal)?
            .len()
            + record.source_id.len()
            + record.source_name.len()
            + record.model_configuration_id.len()
            + 256;
        if bytes + size > 1024 * 1024 {
            break;
        }
        bytes += size;
        impact.push(record.clone());
    }
    if offset < all.len() && impact.is_empty() {
        return Err(ErrorCode::Internal);
    }
    let next = offset + impact.len();
    let complete = next == all.len();
    let mut sources: Vec<ModelFeedAffectedSourceV2> = Vec::new();
    for record in &impact {
        if !sources.iter().any(|s| s.source_id == record.source_id) {
            sources.push(ModelFeedAffectedSourceV2 {
                source_id: record.source_id.clone(),
                display_name: record.source_name.clone(),
                model_configuration_ids: vec![],
                published_revision_count: source_publications
                    .get(&record.source_id)
                    .map_or(0, Vec::len),
            });
        }
        let source = sources
            .iter_mut()
            .find(|s| s.source_id == record.source_id)
            .ok_or(ErrorCode::Internal)?;
        if !source
            .model_configuration_ids
            .contains(&record.model_configuration_id)
        {
            source
                .model_configuration_ids
                .push(record.model_configuration_id.clone());
        }
    }
    Ok((
        ModelFeedRemovalPreviewV2 {
            sources,
            impact,
            total: all.len(),
            published_revision_count: all_publications.len(),
            next_cursor: if complete {
                None
            } else {
                Some(
                    serde_json::to_string(&Cursor {
                        digest: digest.clone(),
                        offset: next,
                    })
                    .map_err(|_| ErrorCode::Internal)?,
                )
            },
            complete,
            retained_sequence: status.highest_accepted_sequence,
            retrieval_enabled: status.retrieval_enabled,
            digest,
        },
        before,
    ))
}
