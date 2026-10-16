use hiroute_domain::{
    CanonicalDigest, ChangeSpecV1, FrozenPriceQuoteV1, PriceGenerationRefV1, PriceModelIdentityV1,
    PriceOriginV1, PriceTargetV1, PriceValuationKindV1, RevisionSetV1, SourcePriceSettingV1,
    TokenRateV1,
};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PriceTargetLocatorV1 {
    Binding {
        binding_id: String,
    },
    SourceModel {
        source_id: String,
        model_identity: PriceModelIdentityV1,
    },
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPriceOverrideChangeV2 {
    pub target_locator: PriceTargetLocatorV1,
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
    pub action: SourcePriceSettingV1,
    pub expected_source_revision: u64,
    pub expected_binding_revision: Option<u64>,
    pub expected_override_revision: u64,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceOverridePreviewV2 {
    pub normalized_target: PriceTargetV1,
    pub before: FrozenPriceQuoteV1,
    pub after: FrozenPriceQuoteV1,
    pub field_changes: Vec<PriceFieldChangeV1>,
    pub spec: ChangeSpecV1,
    pub change_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceRateFieldV1 {
    InputUncached,
    Output,
    CacheRead,
    CacheWrite,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PriceFieldChangeV1 {
    Rate {
        field: PriceRateFieldV1,
        before: TokenRateV1,
        after: TokenRateV1,
    },
    Origin {
        before: PriceOriginV1,
        after: PriceOriginV1,
    },
    Setting {
        before: Option<SourcePriceSettingV1>,
        after: SourcePriceSettingV1,
    },
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPriceOverrideChangeV2 {
    pub spec: ChangeSpecV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub idempotency_key: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectivePriceTargetQueryV2 {
    pub query_id: String,
    pub target_locator: PriceTargetLocatorV1,
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetEffectivePricesV2 {
    pub targets: Vec<EffectivePriceTargetQueryV2>,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectivePriceResultItemV2 {
    pub query_id: String,
    pub quote: FrozenPriceQuoteV1,
    /// Exact optimistic-concurrency inputs for a source-specific price edit. Ordinary
    /// subscription usage prices are not editable; explicit API-equivalent reference prices are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_context: Option<PriceEditContextV1>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceEditContextV1 {
    pub target_locator: PriceTargetLocatorV1,
    pub expected_source_revision: u64,
    pub expected_binding_revision: u64,
    /// Zero is the server-observed absence of an override, not a client default.
    pub expected_override_revision: u64,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectivePricesResultV2 {
    pub pending_activation: bool,
    pub generation_ref: Option<PriceGenerationRefV1>,
    pub evaluated_at: i64,
    pub items: Vec<EffectivePriceResultItemV2>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceApplyOperationV2 {
    pub operation_id: String,
    pub accepted_digest: CanonicalDigest,
    pub state: String,
    pub override_revision: u64,
    pub effective_price_generation: Option<PriceGenerationRefV1>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn price_edit_context_has_an_exact_closed_wire_shape() {
        let context = PriceEditContextV1 {
            target_locator: PriceTargetLocatorV1::Binding {
                binding_id: "binding/one".into(),
            },
            expected_source_revision: 4,
            expected_binding_revision: 7,
            expected_override_revision: 0,
        };
        let value = serde_json::to_value(&context).unwrap();
        assert_eq!(
            value,
            json!({
                "target_locator": {"kind": "binding", "binding_id": "binding/one"},
                "expected_source_revision": 4,
                "expected_binding_revision": 7,
                "expected_override_revision": 0
            })
        );
        let mut unknown = value;
        unknown["generation"] = json!(99);
        assert!(serde_json::from_value::<PriceEditContextV1>(unknown).is_err());
    }
}
