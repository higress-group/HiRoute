//! Source resolution and stateless Preview. Facts are read under the daemon's existing writer.
use super::*;
use crate::control::ControlReadError;
use hiroute_application_api::*;
use hiroute_domain::*;

pub struct PriceControlFactsV1 {
    pub configuration_revision: u64,
    pub catalog_model_ids: std::collections::BTreeSet<String>,
    pub revisions: RevisionSetV1,
    pub source_bindings: Vec<PriceSourceBinding>,
    pub entries: Vec<PriceIndexEntryV1>,
    pub catalog_refs: Vec<PriceFactRefV1>,
    pub effective: PriceSnapshotHandle,
    pub evaluated_at: i64,
}

/// Minimal trusted source/model identity needed by price resolution.
///
/// The daemon normalizes both current management sources and pre-migration projections into this
/// view, so the price controller does not use legacy projection rows as source-existence truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceSourceBinding {
    pub source_id: String,
    pub source_revision: u64,
    pub source_identity_digest: CanonicalDigest,
    pub source_origin: SourceOrigin,
    pub connection_option_id: Option<String>,
    pub binding_id: String,
    pub binding_revision: u64,
    pub model_configuration_id: String,
    pub execution_model_configuration_id: Option<String>,
    pub billing_class: BillingClass,
    pub offer_ref: Option<String>,
}
pub trait SourcePriceControlPort: Send + Sync {
    fn price_control_facts(&self) -> Result<PriceControlFactsV1, ControlReadError>;
}

pub fn resolve_price_target(
    facts: &PriceControlFactsV1,
    locator: &PriceTargetLocatorV1,
    currency: &str,
    valuation_kind: PriceValuationKindV1,
) -> Result<(PriceTargetV1, PriceSourceBinding, u64), ErrorCode> {
    let matches = facts
        .source_bindings
        .iter()
        .filter(|binding| match locator {
            PriceTargetLocatorV1::Binding { binding_id } => &binding.binding_id == binding_id,
            PriceTargetLocatorV1::SourceModel {
                source_id,
                model_identity,
            } => {
                binding.source_id == *source_id
                    && match model_identity {
                        PriceModelIdentityV1::CatalogModel(id)
                        | PriceModelIdentityV1::LocalModel(id) => {
                            binding.model_configuration_id == *id
                        }
                    }
            }
        })
        .collect::<Vec<_>>();
    let [binding] = matches.as_slice() else {
        return Err(ErrorCode::ResourceNotFound);
    };
    let model_identity = if facts
        .catalog_model_ids
        .contains(&binding.model_configuration_id)
    {
        PriceModelIdentityV1::CatalogModel(binding.model_configuration_id.clone())
    } else {
        PriceModelIdentityV1::LocalModel(binding.model_configuration_id.clone())
    };
    if let PriceTargetLocatorV1::SourceModel {
        model_identity: requested,
        ..
    } = locator
        && *requested != model_identity
    {
        return Err(ErrorCode::InvalidArguments);
    }
    let target = PriceTargetV1 {
        workspace_id: WorkspaceId::default(),
        source_id: binding.source_id.clone(),
        source_identity_digest: binding.source_identity_digest.clone(),
        model_identity,
        currency: currency.into(),
        valuation_kind,
    };
    target.validate().map_err(|_| ErrorCode::InvalidArguments)?;
    Ok((target, (*binding).clone(), binding.source_revision))
}

pub fn preview_source_price_change(
    facts: &PriceControlFactsV1,
    request: &PreviewPriceOverrideChangeV2,
) -> Result<PriceOverridePreviewV2, ErrorCode> {
    let (target, binding, source_revision) = resolve_price_target(
        facts,
        &request.target_locator,
        &request.currency,
        request.valuation_kind,
    )?;
    if source_revision != request.expected_source_revision
        || request
            .expected_binding_revision
            .is_some_and(|r| r != binding.binding_revision)
    {
        return Err(ErrorCode::RevisionConflict);
    }
    let current = facts
        .entries
        .iter()
        .find(|e| e.target == target)
        .and_then(|e| e.source_override.clone());
    if current.as_ref().map_or(0, |v| v.revision) != request.expected_override_revision {
        return Err(ErrorCode::RevisionConflict);
    }
    let change = SourcePriceChangeV2 {
        schema: SOURCE_PRICE_CHANGE_SCHEMA_V2.into(),
        target: target.clone(),
        binding_id: binding.binding_id,
        expected_binding_revision: binding.binding_revision,
        expected_source_revision: source_revision,
        before: current,
        after: SourcePriceOverrideV1 {
            target: target.clone(),
            revision: request
                .expected_override_revision
                .checked_add(1)
                .ok_or(ErrorCode::InvalidArguments)?,
            setting: request.action.clone(),
        },
    };
    change.validate().map_err(|_| ErrorCode::InvalidArguments)?;
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "prices.override.apply".into(),
        resource_id: Some(target.source_id.clone()),
        desired_state: serde_json::to_value(&change).map_err(|_| ErrorCode::InvalidArguments)?,
    };
    let digest = price_change_digest(&spec, &facts.revisions)?;
    let mut field_changes = vec![PriceFieldChangeV1::Setting {
        before: change.before.as_ref().map(|v| v.setting.clone()),
        after: change.after.setting.clone(),
    }];
    let mut entries = facts.entries.clone();
    match entries.iter_mut().find(|e| e.target == target) {
        Some(entry) => entry.source_override = Some(change.after),
        None => entries.push(PriceIndexEntryV1 {
            target: target.clone(),
            actual_offer_ref: None,
            reference_model_offer_ref: None,
            catalog_rules: vec![],
            legacy_overrides: vec![],
            source_override: Some(change.after),
        }),
    }
    let next = PriceSnapshot::build(
        facts
            .configuration_revision
            .checked_add(1)
            .ok_or(ErrorCode::InvalidArguments)?,
        facts.catalog_refs.clone(),
        entries,
    )
    .map_err(|_| ErrorCode::InvalidArguments)?;
    let after = PriceSnapshotHandle::Available(std::sync::Arc::new(next))
        .freeze_price(
            &target,
            facts.evaluated_at,
            PriceBillingContextV1::StandardTokens,
        )
        .map_err(|_| ErrorCode::InvalidArguments)?;
    let before = facts
        .effective
        .freeze_price(
            &target,
            facts.evaluated_at,
            PriceBillingContextV1::StandardTokens,
        )
        .map_err(|_| ErrorCode::InvalidArguments)?;
    for (field, old, new) in [
        (
            PriceRateFieldV1::InputUncached,
            &before.rates.input_uncached,
            &after.rates.input_uncached,
        ),
        (
            PriceRateFieldV1::Output,
            &before.rates.output,
            &after.rates.output,
        ),
        (
            PriceRateFieldV1::CacheRead,
            &before.rates.cache_read,
            &after.rates.cache_read,
        ),
        (
            PriceRateFieldV1::CacheWrite,
            &before.rates.cache_write,
            &after.rates.cache_write,
        ),
    ] {
        if old != new {
            field_changes.push(PriceFieldChangeV1::Rate {
                field,
                before: old.clone(),
                after: new.clone(),
            });
        }
    }
    if before.origin != after.origin {
        field_changes.push(PriceFieldChangeV1::Origin {
            before: before.origin,
            after: after.origin,
        });
    }
    Ok(PriceOverridePreviewV2 {
        normalized_target: target,
        field_changes,
        before,
        after,
        spec,
        change_digest: digest,
        expected_revisions: facts.revisions.clone(),
    })
}
pub(super) fn price_change_digest(
    spec: &ChangeSpecV1,
    revisions: &RevisionSetV1,
) -> Result<CanonicalDigest, ErrorCode> {
    CanonicalDigest::of(&(SOURCE_PRICE_CHANGE_SCHEMA_V2, spec, revisions))
        .map_err(|_| ErrorCode::InvalidArguments)
}
pub(super) fn request_from_spec(
    spec: &ChangeSpecV1,
) -> Result<PreviewPriceOverrideChangeV2, ErrorCode> {
    let c: SourcePriceChangeV2 = serde_json::from_value(spec.desired_state.clone())
        .map_err(|_| ErrorCode::InvalidArguments)?;
    c.validate().map_err(|_| ErrorCode::InvalidArguments)?;
    Ok(PreviewPriceOverrideChangeV2 {
        target_locator: PriceTargetLocatorV1::SourceModel {
            source_id: c.target.source_id,
            model_identity: c.target.model_identity,
        },
        currency: c.target.currency,
        valuation_kind: c.target.valuation_kind,
        action: c.after.setting,
        expected_source_revision: c.expected_source_revision,
        expected_binding_revision: Some(c.expected_binding_revision),
        expected_override_revision: c.before.map_or(0, |b| b.revision),
    })
}
