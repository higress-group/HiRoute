//! The digest covers the complete impact; pagination only bounds its presentation.
use super::*;
use hiroute_domain::{ComputeManagementRepositoryPort, SourcePriceSettingV1};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    digest: CanonicalDigest,
    offset: usize,
}

fn price_signature(
    facts: &hiroute_application::prices::PriceControlFactsV1,
    source_id: &str,
    model: &str,
) -> Value {
    let mut entries = facts.entries.iter().filter(|e| e.target.source_id == source_id && match &e.target.model_identity {
        hiroute_domain::PriceModelIdentityV1::CatalogModel(id) | hiroute_domain::PriceModelIdentityV1::LocalModel(id) => id == model,
    }).map(|e| {
        let rates = if let Some(setting) = e.source_override.as_ref().map(|v| &v.setting)
            && let SourcePriceSettingV1::Set {rates} = setting {json!({"manual": rates})}
        else {json!({"rules": e.catalog_rules.iter().map(|r| (&r.rates, &r.schedule)).collect::<Vec<_>>(), "legacy":e.legacy_overrides})};
        (e.target.currency.clone(), e.target.valuation_kind, rates)
    }).collect::<Vec<_>>();
    entries.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    json!(entries)
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
        catalog: (*feed.embedded_catalog()).clone(),
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
    let mut preference_dependencies = Vec::new();
    for source in &management.sources {
        let preferences = stores
            .control()
            .source_default_preferences(&source.source_id)
            .map_err(|_| ErrorCode::DaemonUnavailable)?;
        let current = super::super::model_connections::defaults::project_source(
            &catalog,
            source,
            &preferences,
        );
        let after = super::super::model_connections::defaults::project_source(
            &fallback,
            source,
            &preferences,
        );
        preference_dependencies.push((source.source_id.clone(), source.revision, preferences));
        for (old, new) in current.models.iter().zip(&after.models) {
            let Some(id) = old.catalog_configuration_id.as_ref() else {
                continue;
            };
            let mut changes = BTreeMap::<String, Value>::new();
            macro_rules! compare {($field:ident) => {if old.capabilities.$field != new.capabilities.$field {changes.insert(stringify!($field).into(),json!({"current":old.capabilities.$field.value,"fallback":new.capabilities.$field.value}));}}}
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
            let old_prices = price_signature(&current_prices, &source.source_id, id);
            let new_prices = price_signature(&fallback_prices, &source.source_id, id);
            if old_prices != new_prices {
                changes.insert(
                    "prices".into(),
                    json!({"current":old_prices,"fallback":new_prices}),
                );
            }
            if !changes.is_empty() {
                let published_revisions = versions
                    .iter()
                    .filter(|v| candidates(v).any(|c| c.binding_id == old.binding_id))
                    .map(|v| v.reference.clone())
                    .collect();
                all.push(ModelFeedDefaultImpactV2 {
                    source_id: source.source_id.clone(),
                    source_name: source.display_name.clone(),
                    binding_id: old.binding_id.clone(),
                    model_configuration_id: id.clone(),
                    changes,
                    published_revisions,
                });
            }
        }
    }
    all.sort_by(|a, b| (&a.source_id, &a.binding_id).cmp(&(&b.source_id, &b.binding_id)));
    let digest = CanonicalDigest::of(&(
        &before,
        &management.revisions,
        &preference_dependencies,
        &all,
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
            .len();
        if bytes + size * 3 > 1024 * 1024 {
            break;
        }
        bytes += size * 3;
        impact.push(record.clone());
    }
    if offset < all.len() && impact.is_empty() {
        return Err(ErrorCode::Internal);
    }
    let next = offset + impact.len();
    let complete = next == all.len();
    let mut sources: Vec<ModelFeedAffectedSourceV1> = Vec::new();
    let mut published_revisions = Vec::new();
    for record in &impact {
        if !sources.iter().any(|s| s.source_id == record.source_id) {
            sources.push(ModelFeedAffectedSourceV1 {
                source_id: record.source_id.clone(),
                display_name: record.source_name.clone(),
                model_configuration_ids: vec![],
                published_revisions: vec![],
            });
        }
        let source = sources
            .iter_mut()
            .find(|s| s.source_id == record.source_id)
            .ok_or(ErrorCode::Internal)?;
        source
            .model_configuration_ids
            .push(record.model_configuration_id.clone());
        for revision in &record.published_revisions {
            if !source.published_revisions.contains(revision) {
                source.published_revisions.push(revision.clone());
            }
            if !published_revisions.contains(revision) {
                published_revisions.push(revision.clone());
            }
        }
    }
    let mut unique_published = Vec::new();
    for reference in all.iter().flat_map(|r| &r.published_revisions) {
        if !unique_published.contains(reference) {
            unique_published.push(reference.clone());
        }
    }
    let all_published = unique_published.len();
    Ok((
        ModelFeedRemovalPreviewV2 {
            sources,
            published_revisions,
            impact,
            total: all.len(),
            published_plan_count: all_published,
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
