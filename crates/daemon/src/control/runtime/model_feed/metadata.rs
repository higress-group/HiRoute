//! Complete descriptive reads. Pagination never holds the feed lock or performs network I/O.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const RESPONSE_BUDGET: usize = 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    snapshot: ModelMetadataSnapshotRefV1,
    kind: Option<String>,
    search: Option<String>,
    after: (String, String),
}

fn capture(adapter: &LocalControlAdapter) -> Result<Arc<ReleaseCatalogSnapshot>, ErrorCode> {
    adapter
        .current_catalog()
        .ok_or(ErrorCode::ChangePreviewStale)
}

fn identity(catalog: &ReleaseCatalogSnapshot) -> ModelMetadataSnapshotRefV1 {
    let selected = catalog.official_snapshot();
    ModelMetadataSnapshotRefV1 {
        generation: catalog.generation,
        data_revision: selected.data_revision,
        data_digest: selected.data_digest,
    }
}

fn records(catalog: &ReleaseCatalogSnapshot) -> Result<Vec<ModelMetadataEntryV1>, ErrorCode> {
    let snapshot = catalog.official_snapshot();
    let mut body = serde_json::to_value(&snapshot.data).map_err(|_| ErrorCode::Internal)?;
    let mut result = Vec::new();
    let origin = if catalog.official_data_from_feed() {
        "website_cache"
    } else {
        "embedded"
    };
    let mut add = |kind: &str, field: &mut Value, key: &str| -> Result<(), ErrorCode> {
        let Some(values) = field.as_array_mut() else {
            return Err(ErrorCode::Internal);
        };
        for value in values.drain(..) {
            let id = value
                .get(key)
                .and_then(Value::as_str)
                .ok_or(ErrorCode::Internal)?;
            let id = if kind == "rating" {
                format!(
                    "{id}/{}",
                    CanonicalDigest::of(&value["native_configuration"])
                        .map_err(|_| ErrorCode::Internal)?
                )
            } else {
                id.to_owned()
            };
            let supported = kind == "model"
                && catalog
                    .model_data()
                    .model_endpoint_capabilities
                    .iter()
                    .any(|c| {
                        c.model_configuration_id == id
                            && catalog.model_data().offers.iter().any(|o| {
                                o.endpoint_profile_id == c.endpoint_profile_id
                                    && o.model_configuration_ids.contains(&id)
                            })
                    });
            result.push(ModelMetadataEntryV1 {
                kind: kind.into(),
                key: id,
                origin: origin.into(),
                qualification: if supported {
                    "locally_supported"
                } else {
                    "descriptive_only"
                }
                .into(),
                value,
            });
        }
        Ok(())
    };
    for (kind, field, key) in [
        ("model", "models", "model_configuration_id"),
        ("capability", "model_endpoint_capabilities", "capability_id"),
        ("offer", "offers", "offer_id"),
        ("legacy_price", "price_rates", "price_rate_id"),
        ("price", "pricing_facts", "pricing_key"),
        (
            "connection_hint",
            "connection_hints",
            "connection_option_id",
        ),
    ] {
        add(kind, &mut body[field], key)?;
    }
    add(
        "rating",
        &mut body["rating_snapshot"]["records"],
        "model_configuration_id",
    )?;
    add(
        "native_reasoning",
        &mut body["rating_snapshot"]["models"],
        "model_configuration_id",
    )?;
    for (kind, field, key) in [
        ("evidence_source", "evidence_sources", "source_key"),
        ("client_run", "client_runs", "source_key"),
        ("access_product", "access_products", "product_key"),
        ("canonical_model", "canonical_models", "model_key"),
        ("endpoint_binding", "endpoint_bindings", "binding_key"),
        ("dynamic_route", "dynamic_routes", "route_key"),
        ("inference_rule", "inference_rules", "rule_key"),
        ("provider", "provider_records", "provider_record_key"),
        ("model_metadata", "model_records", "model_record_key"),
    ] {
        add(kind, &mut body["metadata_catalog"][field], key)?;
    }
    result.sort_by(|a, b| (&a.kind, &a.key).cmp(&(&b.kind, &b.key)));
    Ok(result)
}

pub(super) fn list(
    adapter: &LocalControlAdapter,
    request: ListModelMetadataV1,
) -> Result<ModelMetadataPageV1, ErrorCode> {
    page(capture(adapter)?, request)
}

fn page(
    catalog: Arc<ReleaseCatalogSnapshot>,
    request: ListModelMetadataV1,
) -> Result<ModelMetadataPageV1, ErrorCode> {
    if request.kind.as_ref().is_some_and(|s| s.len() > 64)
        || request.search.as_ref().is_some_and(|s| s.len() > 256)
        || request.cursor.as_ref().is_some_and(|s| s.len() > 4096)
    {
        return Err(ErrorCode::InvalidArguments);
    }
    let size = request.page_size.unwrap_or(50);
    if size == 0 || size > 100 {
        return Err(ErrorCode::InvalidArguments);
    }
    let snapshot = identity(&catalog);
    let search = request.search.map(|s| s.to_lowercase());
    let after = if let Some(cursor) = &request.cursor {
        let cursor: Cursor =
            serde_json::from_str(cursor).map_err(|_| ErrorCode::InvalidArguments)?;
        if cursor.snapshot != snapshot {
            return Err(ErrorCode::ChangePreviewStale);
        }
        if cursor.kind != request.kind || cursor.search != search {
            return Err(ErrorCode::InvalidArguments);
        }
        Some(cursor.after)
    } else {
        None
    };
    let all: Vec<_> = records(&catalog)?
        .into_iter()
        .filter(|r| {
            request.kind.as_ref().is_none_or(|kind| &r.kind == kind)
                && search.as_ref().is_none_or(|s| {
                    r.key.to_lowercase().contains(s)
                        || r.value.to_string().to_lowercase().contains(s)
                })
        })
        .collect();
    let start = if let Some(after) = after {
        all.iter()
            .position(|r| (&r.kind, &r.key) == (&after.0, &after.1))
            .ok_or(ErrorCode::InvalidArguments)?
            + 1
    } else {
        0
    };
    let total = all.len();
    let mut bytes = 4096;
    let mut page = Vec::new();
    for entry in all.into_iter().skip(start).take(size as usize) {
        let count = serde_json::to_vec(&entry)
            .map_err(|_| ErrorCode::Internal)?
            .len();
        if bytes + count > RESPONSE_BUDGET {
            break;
        }
        bytes += count;
        page.push(entry);
    }
    let complete = start + page.len() == total;
    let next_cursor = if complete {
        None
    } else {
        let last = page.last().ok_or(ErrorCode::Internal)?;
        Some(
            serde_json::to_string(&Cursor {
                snapshot: snapshot.clone(),
                kind: request.kind,
                search,
                after: (last.kind.clone(), last.key.clone()),
            })
            .map_err(|_| ErrorCode::Internal)?,
        )
    };
    let response = ModelMetadataPageV1 {
        snapshot,
        records: page,
        total,
        next_cursor,
        complete,
    };
    if serde_json::to_vec(&response)
        .map_err(|_| ErrorCode::Internal)?
        .len()
        > RESPONSE_BUDGET
    {
        return Err(ErrorCode::Internal);
    }
    Ok(response)
}

pub(super) fn get(
    adapter: &LocalControlAdapter,
    request: GetModelMetadataV1,
) -> Result<ModelMetadataDetailV1, ErrorCode> {
    detail(capture(adapter)?, request)
}
fn detail(
    catalog: Arc<ReleaseCatalogSnapshot>,
    request: GetModelMetadataV1,
) -> Result<ModelMetadataDetailV1, ErrorCode> {
    // Rating identities include a bounded model ID and the native-configuration digest.
    if request.kind.len() > 64 || request.key.len() > 512 {
        return Err(ErrorCode::InvalidArguments);
    }
    let snapshot = identity(&catalog);
    if request
        .snapshot
        .as_ref()
        .is_some_and(|expected| expected != &snapshot)
    {
        return Err(ErrorCode::ChangePreviewStale);
    }
    let record = records(&catalog)?
        .into_iter()
        .find(|r| r.kind == request.kind && r.key == request.key)
        .ok_or(ErrorCode::InvalidArguments)?;
    Ok(ModelMetadataDetailV1 { snapshot, record })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn current() -> Arc<ReleaseCatalogSnapshot> {
        Arc::new(ReleaseCatalogSnapshot {
            catalog: crate::release_catalog::current_fixture_catalog(),
            generation: 1,
            accepted_sequence: 0,
        })
    }
    #[test]
    fn complete_metadata_pages_have_no_loss_and_old_generation_cursors_conflict() {
        let catalog = current();
        let expected: BTreeSet<_> = records(&catalog)
            .unwrap()
            .into_iter()
            .map(|r| (r.kind, r.key))
            .collect();
        let mut actual = BTreeSet::new();
        let mut cursor = None;
        loop {
            let response = page(
                catalog.clone(),
                ListModelMetadataV1 {
                    page_size: Some(100),
                    cursor: cursor.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(response.total, expected.len());
            for record in response.records {
                assert!(actual.insert((record.kind, record.key)));
            }
            if response.complete {
                assert!(response.next_cursor.is_none());
                break;
            }
            cursor = response.next_cursor;
        }
        assert_eq!(actual, expected);
        let first = page(
            catalog.clone(),
            ListModelMetadataV1 {
                page_size: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let mut next = (*catalog).clone();
        next.generation += 1;
        let result = page(
            Arc::new(next),
            ListModelMetadataV1 {
                cursor: first.next_cursor,
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(ErrorCode::ChangePreviewStale)));
    }
    #[test]
    fn metadata_response_budget_continues_instead_of_truncating() {
        let base = crate::release_catalog::current_fixture_catalog();
        let mut snapshot = base.official_snapshot();
        for price in &mut snapshot.data.pricing_facts {
            for i in 0..12 {
                price.schedules[0]
                    .condition
                    .insert(format!("detail_{i}"), Value::String("x".repeat(16_000)));
            }
        }
        snapshot.data_revision += 1;
        snapshot.data_digest = CanonicalDigest::of(&snapshot.data).unwrap();
        let payload = hiroute_integrations::model_feed::FeedPayload::from_snapshot(
            snapshot,
            1,
            "2026-10-21T00:00:00Z".into(),
            "1".repeat(40),
            vec![],
        );
        let admitted =
            hiroute_integrations::model_feed::admit(&base, &payload, env!("CARGO_PKG_VERSION"))
                .unwrap();
        let catalog = Arc::new(ReleaseCatalogSnapshot {
            catalog: base.with_feed(&admitted).unwrap(),
            generation: 2,
            accepted_sequence: 1,
        });
        let first = page(
            catalog.clone(),
            ListModelMetadataV1 {
                kind: Some("price".into()),
                page_size: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!first.complete);
        assert!(serde_json::to_vec(&first).unwrap().len() <= RESPONSE_BUDGET);
        let second = page(
            catalog,
            ListModelMetadataV1 {
                kind: Some("price".into()),
                cursor: first.next_cursor.clone(),
                page_size: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(second.complete);
        assert_eq!(first.records.len() + second.records.len(), first.total);
    }
    #[test]
    fn model_metadata_detail_accepts_full_compound_rating_identities() {
        let base = crate::release_catalog::current_fixture_catalog();
        let mut snapshot = base.official_snapshot();
        let old = snapshot.data.rating_snapshot.records[0]
            .model_configuration_id
            .clone();
        let long = format!("model.{}", "x".repeat(245));
        for model in &mut snapshot.data.models {
            if model.model_configuration_id == old {
                model.model_configuration_id = long.clone();
            }
        }
        snapshot
            .data
            .models
            .sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
        for cap in &mut snapshot.data.model_endpoint_capabilities {
            if cap.model_configuration_id == old {
                cap.model_configuration_id = long.clone();
            }
        }
        for price in &mut snapshot.data.price_rates {
            if price.model_configuration_id == old {
                price.model_configuration_id = long.clone();
            }
        }
        for offer in &mut snapshot.data.offers {
            for id in &mut offer.model_configuration_ids {
                if id == &old {
                    *id = long.clone();
                }
            }
            offer.model_configuration_ids.sort();
        }
        for native in &mut snapshot.data.rating_snapshot.models {
            if native.model_configuration_id == old {
                native.model_configuration_id = long.clone();
            }
        }
        for rating in &mut snapshot.data.rating_snapshot.records {
            if rating.model_configuration_id == old {
                rating.model_configuration_id = long.clone();
            }
        }
        snapshot
            .data
            .rating_snapshot
            .models
            .sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
        snapshot
            .data
            .rating_snapshot
            .records
            .sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
        snapshot.data.rating_snapshot.model_catalog_digest =
            CanonicalDigest::of(&snapshot.data.models).unwrap();
        snapshot.data.rating_snapshot.digest =
            snapshot.data.rating_snapshot.computed_digest().unwrap();
        snapshot.data_revision += 1;
        snapshot.data_digest = CanonicalDigest::of(&snapshot.data).unwrap();
        let payload = hiroute_integrations::model_feed::FeedPayload::from_snapshot(
            snapshot,
            2,
            "2026-10-21T00:00:00Z".into(),
            "1".repeat(40),
            vec![],
        );
        let admitted =
            hiroute_integrations::model_feed::admit(&base, &payload, env!("CARGO_PKG_VERSION"))
                .unwrap();
        let catalog = Arc::new(ReleaseCatalogSnapshot {
            catalog: base.with_feed(&admitted).unwrap(),
            generation: 2,
            accepted_sequence: 2,
        });
        let row = records(&catalog)
            .unwrap()
            .into_iter()
            .find(|r| r.kind == "rating" && r.key.starts_with(&long))
            .unwrap();
        assert!(row.key.len() > 256);
        let response = detail(
            catalog.clone(),
            GetModelMetadataV1 {
                kind: row.kind.clone(),
                key: row.key.clone(),
                snapshot: Some(identity(&catalog)),
            },
        )
        .unwrap();
        assert_eq!(response.record, row);
    }
}
