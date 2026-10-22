use arc_swap::ArcSwapOption;
use hiroute_domain::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_PRICE_RULES_PER_TARGET: usize = 64;
pub const MAX_PRICE_TARGETS: usize = 16_384;
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPriceRuleV2 {
    pub rule_ref: PriceFactRefV1,
    pub rates: TokenRatesV1,
    pub schedule: Option<PriceScheduleV1>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceIndexEntryV1 {
    pub target: PriceTargetV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding_id: Option<String>,
    pub actual_offer_ref: Option<String>,
    pub reference_model_offer_ref: Option<PriceReferenceOfferV1>,
    pub catalog_rules: Vec<CatalogPriceRuleV2>,
    pub legacy_overrides: Vec<PriceOverrideV1>,
    pub source_override: Option<SourcePriceOverrideV1>,
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ExecutionPriceKey {
    workspace_id: WorkspaceId,
    source_id: String,
    source_identity_digest: CanonicalDigest,
    binding_id: String,
    currency: String,
    valuation_kind: PriceValuationKindV1,
}
impl ExecutionPriceKey {
    fn new(target: &PriceTargetV1, binding: &str) -> Self {
        Self {
            workspace_id: target.workspace_id.clone(),
            source_id: target.source_id.clone(),
            source_identity_digest: target.source_identity_digest.clone(),
            binding_id: binding.into(),
            currency: target.currency.clone(),
            valuation_kind: target.valuation_kind,
        }
    }
}

/// Immutable and indexed, constructed by the control writer from verified catalog/mapping facts.
#[derive(Debug)]
pub struct PriceSnapshot {
    pub(super) generation: PriceGenerationRefV1,
    pub(super) entries: BTreeMap<PriceTargetV1, PriceIndexEntryV1>,
    execution_targets: BTreeMap<ExecutionPriceKey, Option<PriceTargetV1>>,
}
impl PriceSnapshot {
    pub fn build(
        configuration_revision: u64,
        mut catalog_refs: Vec<PriceFactRefV1>,
        entries: Vec<PriceIndexEntryV1>,
    ) -> Result<Self, ComputeContractError> {
        if entries.len() > MAX_PRICE_TARGETS || catalog_refs.len() > 64 {
            return Err(ComputeContractError::InvalidPrice);
        }
        let mut references = std::collections::BTreeSet::new();
        for reference in &catalog_refs {
            reference.validate()?;
            if !references.insert((&reference.id, reference.revision)) {
                return Err(ComputeContractError::DuplicateIdentity);
            }
        }
        catalog_refs.sort_by(|a, b| (&a.id, a.revision).cmp(&(&b.id, b.revision)));
        let mut index = BTreeMap::new();
        let mut execution_targets: BTreeMap<ExecutionPriceKey, Option<PriceTargetV1>> =
            BTreeMap::new();
        for entry in entries {
            super::select::validate_entry(&entry)?;
            if let Some(binding) = &entry.execution_binding_id {
                if binding.is_empty()
                    || binding.len() > 256
                    || binding.chars().any(char::is_control)
                {
                    return Err(ComputeContractError::InvalidPrice);
                }
                let execution = ExecutionPriceKey::new(&entry.target, binding);
                execution_targets
                    .entry(execution)
                    .and_modify(|target| {
                        if target.as_ref() != Some(&entry.target) {
                            *target = None;
                        }
                    })
                    .or_insert_with(|| Some(entry.target.clone()));
            }
            if index.insert(entry.target.clone(), entry).is_some() {
                return Err(ComputeContractError::DuplicateIdentity);
            }
        }
        // Sorted target entries close over source mappings as well as price settings.
        let digest = CanonicalDigest::of(&(
            configuration_revision,
            &catalog_refs,
            index.values().collect::<Vec<_>>(),
        ))
        .map_err(|_| ComputeContractError::InvalidPrice)?;
        let generation = PriceGenerationRefV1 {
            id: format!("price/{}", &digest.as_str()[7..]),
            digest,
            configuration_revision,
            catalog_refs,
        };
        Ok(Self {
            generation,
            entries: index,
            execution_targets,
        })
    }
    pub fn generation_ref(&self) -> &PriceGenerationRefV1 {
        &self.generation
    }
}

#[derive(Clone, Debug)]
pub enum PriceSnapshotHandle {
    Available(Arc<PriceSnapshot>),
    Unavailable(PriceUnknownReasonV1),
}
impl PriceSnapshotHandle {
    pub fn generation_ref(&self) -> Option<&PriceGenerationRefV1> {
        match self {
            Self::Available(s) => Some(&s.generation),
            Self::Unavailable(_) => None,
        }
    }
    /// The caller supplies the verified binding from its pinned publication. Its
    /// compiled execution ID can predate a credential/source revision, while the
    /// stable source/model binding still owns the current price. Never infer this
    /// association from model names; retain the executed identity in the quote.
    pub fn freeze_execution_price(
        &self,
        target: &PriceTargetV1,
        binding: &str,
        at: i64,
        billing: PriceBillingContextV1,
    ) -> Result<FrozenPriceQuoteV1, ComputeContractError> {
        if let Self::Available(snapshot) = self
            && let Some(mapped) = snapshot
                .execution_targets
                .get(&ExecutionPriceKey::new(target, binding))
        {
            let Some(mapped) = mapped else {
                let mut quote = Self::Unavailable(PriceUnknownReasonV1::UnverifiedOfferMapping)
                    .freeze_price(target, at, billing)?;
                quote.generation_ref = Some(snapshot.generation.clone());
                quote.quote_digest = quote.computed_digest()?;
                return Ok(quote);
            };
            let mut quote = self.freeze_price(mapped, at, billing)?;
            quote.exact_target = target.clone();
            quote.quote_digest = quote.computed_digest()?;
            return Ok(quote);
        }
        self.freeze_price(target, at, billing)
    }
    /// Only bounded in-memory lookup, schedule evaluation and quote serialization.
    /// Retain this handle for the whole LogicalRequest, including every retry/relay Attempt.
    pub fn freeze_price(
        &self,
        target: &PriceTargetV1,
        attempt_execution_at: i64,
        billing_context: PriceBillingContextV1,
    ) -> Result<FrozenPriceQuoteV1, ComputeContractError> {
        target.validate()?;
        if let PriceBillingContextV1::Unsupported { condition } = &billing_context
            && (condition.is_empty()
                || condition.len() > 128
                || condition.chars().any(char::is_control))
        {
            return Err(ComputeContractError::InvalidPrice);
        }
        let (generation_ref, entry, missing) = match self {
            Self::Available(s) => (
                Some(s.generation.clone()),
                s.entries.get(target),
                PriceUnknownReasonV1::TargetNotInSnapshot,
            ),
            Self::Unavailable(reason) => (None, None, *reason),
        };
        let mut quote = FrozenPriceQuoteV1 {
            generation_ref,
            exact_target: target.clone(),
            actual_source_ref: target.source_id.clone(),
            actual_offer_ref: entry.and_then(|e| e.actual_offer_ref.clone()),
            reference_model_offer_ref: entry.and_then(|e| e.reference_model_offer_ref.clone()),
            valuation_kind: target.valuation_kind,
            currency: target.currency.clone(),
            unit: PriceUnitV1::MicrosPerMillionTokens,
            rates: TokenRatesV1::unknown(missing),
            origin: PriceOriginV1::Unknown,
            applied_override_refs: vec![],
            selected_rule_refs: vec![],
            attempt_execution_at,
            applied_schedule: None,
            billing_context,
            unknown_reasons: vec![],
            quote_digest: CanonicalDigest::of_bytes(b"pending"),
        };
        if attempt_execution_at < 0 {
            quote.rates = TokenRatesV1::unknown(PriceUnknownReasonV1::InvalidExecutionTime);
        } else if !matches!(quote.billing_context, PriceBillingContextV1::StandardTokens) {
            quote.rates = TokenRatesV1::unknown(PriceUnknownReasonV1::UnsupportedBillingCondition);
        } else if let Some(entry) = entry {
            super::select::fill_quote(entry, &mut quote)?;
        }
        quote.unknown_reasons = quote.rates.unknown_reasons();
        quote.quote_digest = quote.computed_digest()?;
        Ok(quote)
    }
}

#[derive(Default)]
pub struct PriceSnapshotSlot {
    current: ArcSwapOption<PriceSnapshot>,
}
impl PriceSnapshotSlot {
    /// Withdraw only the current index. Existing LogicalRequests keep their captured handle.
    pub fn invalidate_current(&self) {
        self.current.store(None);
    }
    pub fn capture_current_price_snapshot(&self) -> PriceSnapshotHandle {
        self.current.load_full().map_or(
            PriceSnapshotHandle::Unavailable(PriceUnknownReasonV1::SnapshotUnavailable),
            PriceSnapshotHandle::Available,
        )
    }
    /// Existing control writer/recovery must serialize this with commit and Operation completion.
    /// CAS additionally prevents stale concurrent installers from overwriting another generation.
    pub fn install(
        &self,
        snapshot: PriceSnapshot,
        expected: Option<&PriceGenerationRefV1>,
    ) -> Result<(), ComputeContractError> {
        let old = self.current.load_full();
        if old.as_ref().map(|s| &s.generation) != expected
            || old.as_ref().is_some_and(|s| {
                s.generation.configuration_revision > snapshot.generation.configuration_revision
            })
        {
            return Err(ComputeContractError::GenerationConflict);
        }
        let previous = self
            .current
            .compare_and_swap(&old, Some(Arc::new(snapshot)));
        let unchanged = match (&*previous, &old) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if unchanged {
            Ok(())
        } else {
            Err(ComputeContractError::GenerationConflict)
        }
    }
}
