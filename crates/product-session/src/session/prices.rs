//! Price-only native entry. WebView supplies intent; native owns confirmation and authority.
use super::*;
use hiroute_domain::{PriceUnknownReasonV1, SourcePriceSettingV1, TokenRateV1, TokenRatesV1};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceEditInput {
    pub target_locator: PriceTargetLocatorV1,
    pub currency: String,
    pub valuation_kind: hiroute_domain::PriceValuationKindV1,
    pub expected_source_revision: u64,
    pub expected_binding_revision: Option<u64>,
    pub expected_override_revision: u64,
    pub action: DecimalPriceAction,
    pub language: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecimalPriceAction {
    Set {
        input_uncached: String,
        output: String,
        cache_read: Option<String>,
        cache_write: Option<String>,
    },
    FollowCatalog,
}
impl PriceEditInput {
    fn change(&self) -> Result<PreviewPriceOverrideChangeV2, DesktopFailure> {
        let required = |s: &str| {
            TokenRateV1::parse_decimal(s).map_err(|_| DesktopFailure::from("INVALID_PRICE_DECIMAL"))
        };
        let optional = |s: &Option<String>| match s.as_deref() {
            None | Some("") => Ok(TokenRateV1::unknown(
                PriceUnknownReasonV1::CacheRateNotCollected,
            )),
            Some(s) => required(s),
        };
        Ok(PreviewPriceOverrideChangeV2 {
            target_locator: self.target_locator.clone(),
            currency: self.currency.clone(),
            valuation_kind: self.valuation_kind,
            expected_source_revision: self.expected_source_revision,
            expected_binding_revision: self.expected_binding_revision,
            expected_override_revision: self.expected_override_revision,
            action: match &self.action {
                DecimalPriceAction::FollowCatalog => SourcePriceSettingV1::FollowCatalog,
                DecimalPriceAction::Set {
                    input_uncached,
                    output,
                    cache_read,
                    cache_write,
                } => SourcePriceSettingV1::Set {
                    rates: TokenRatesV1 {
                        input_uncached: required(input_uncached)?,
                        output: required(output)?,
                        cache_read: optional(cache_read)?,
                        cache_write: optional(cache_write)?,
                    },
                },
            },
        })
    }
}
// Not serializable: neither an approval boolean nor an Apply payload crosses WebView IPC.
pub struct PriceConfirmation {
    permit: ConfirmationPermit,
    english: bool,
    preview: PriceOverridePreviewV2,
    request: ApplyPriceOverrideChangeV2,
    intent: IntentEvidence,
}
impl PriceConfirmation {
    pub fn revision(&self) -> u64 {
        self.request.expected_revisions.target
    }
    pub fn english(&self) -> bool {
        self.english
    }
    pub fn message(&self) -> String {
        let rate = |value: &TokenRateV1| match value {
            TokenRateV1::Known {
                micros_per_million_tokens: n,
            } => {
                format!("{}.{:06}", n / 1_000_000, n % 1_000_000)
            }
            TokenRateV1::Unknown { .. } => if self.english { "Unknown" } else { "未知" }.into(),
        };
        let before = &self.preview.before.rates;
        let after = &self.preview.after.rates;
        let labels = if self.english {
            [
                "Source",
                "Currency per million tokens",
                "Uncached input",
                "Output",
                "Cache read",
                "Cache write",
                "Digest",
            ]
        } else {
            [
                "来源",
                "币种 / 每百万 token",
                "普通输入",
                "输出",
                "缓存读取",
                "缓存写入",
                "变更摘要",
            ]
        };
        format!(
            "{}: {}\n{}: {}\n{}: {} → {}\n{}: {} → {}\n{}: {} → {}\n{}: {} → {}\n{}: {}",
            labels[0],
            self.preview.normalized_target.source_id,
            labels[1],
            self.preview.normalized_target.currency,
            labels[2],
            rate(&before.input_uncached),
            rate(&after.input_uncached),
            labels[3],
            rate(&before.output),
            rate(&after.output),
            labels[4],
            rate(&before.cache_read),
            rate(&after.cache_read),
            labels[5],
            rate(&before.cache_write),
            rate(&after.cache_write),
            labels[6],
            self.request.accept_digest
        )
    }
}
impl Session {
    pub async fn preview_price(
        &mut self,
        input: PriceEditInput,
    ) -> Result<PriceConfirmation, DesktopFailure> {
        if self.confirmation.is_open() {
            return Err("CONFIRMATION_ALREADY_OPEN".into());
        }
        if !matches!(input.language.as_str(), "zh" | "en") {
            return Err("INVALID_LANGUAGE".into());
        }
        let change = input.change()?;
        let intent = IntentEvidence {
            schema: "hiroute.desktop-price-intent/v1".into(),
            digest: CanonicalDigest::of(&change).map_err(|_| "REQUEST_INVALID")?,
        };
        let retry = self.retry_key(&intent).await?;
        let preview: PriceOverridePreviewV2 =
            query(&self.client, "PreviewPriceOverrideChange", &change).await?;
        let request = ApplyPriceOverrideChangeV2 {
            spec: preview.spec.clone(),
            accept_digest: preview.change_digest.clone(),
            expected_revisions: preview.expected_revisions.clone(),
            idempotency_key: retry.unwrap_or(crate::random_id()?),
        };
        Ok(PriceConfirmation {
            permit: self.confirmation.begin()?,
            english: input.language == "en",
            preview,
            request,
            intent,
        })
    }
    pub async fn finish_price_confirmation(
        &mut self,
        context: PriceConfirmation,
        accepted: bool,
    ) -> Result<MutationOutcome, DesktopFailure> {
        if !self.confirmation.finish(&context.permit, accepted)? {
            return Ok(MutationOutcome {
                state: "cancelled_before_apply".into(),
                operation: None,
            });
        }
        let hint = SubmittedOperation {
            // Historical field name; holds the stable, non-secret target reference only.
            plan_id: context
                .preview
                .normalized_target
                .digest()
                .map_err(|_| "REQUEST_INVALID")?
                .to_string(),
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: "ApplyPriceOverrideChange".into(),
            idempotency_key: context.request.idempotency_key.clone(),
            accepted_digest: context.request.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(context.intent),
            latest_edit_not_applied: false,
        };

        self.hint = Some(hint);
        let response = self
            .client
            .call_wire(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: crate::random_id()?,
                operation_id: "ApplyPriceOverrideChange".into(),
                payload: serde_json::to_value(context.request).map_err(|_| "REQUEST_INVALID")?,
                protected_grant: None,
            })
            .await;
        self.reconcile_apply_response(response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_intent_preserves_full_u64_and_rejects_webview_authority() {
        let mut value = serde_json::json!({
            "target_locator": {"kind":"binding", "binding_id":"binding/test"},
            "currency":"USD", "valuation_kind":"usage_estimate",
            "expected_source_revision":1, "expected_binding_revision":1, "expected_override_revision":0,
            "language":"en", "action":{"kind":"set", "input_uncached":"18446744073709.551615", "output":"0", "cache_read":null, "cache_write":null}
        });
        let input: PriceEditInput = serde_json::from_value(value.clone()).unwrap();
        let SourcePriceSettingV1::Set { rates } = input.change().unwrap().action else {
            panic!("manual price")
        };
        assert_eq!(rates.input_uncached, TokenRateV1::known(u64::MAX));
        assert!(matches!(rates.cache_read, TokenRateV1::Unknown { .. }));
        for field in [
            "confirmed",
            "protected_grant",
            "capability",
            "spec",
            "accepted_digest",
        ] {
            let mut forged = value.clone();
            forged[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<PriceEditInput>(forged).is_err());
        }
        value["action"]["input_uncached"] = serde_json::json!("18446744073709.551616");
        assert!(
            serde_json::from_value::<PriceEditInput>(value)
                .unwrap()
                .change()
                .is_err()
        );
    }
}

#[derive(Serialize)]
pub struct PriceDisplayResult {
    pub result: EffectivePricesResultV2,
    pub display_rates: Vec<[Option<String>; 4]>,
}
impl PriceDisplayResult {
    pub fn new(result: EffectivePricesResultV2) -> Self {
        let display_rates = result
            .items
            .iter()
            .map(|item| {
                let r = &item.quote.rates;
                [&r.input_uncached, &r.output, &r.cache_read, &r.cache_write].map(|rate| match rate
                {
                    TokenRateV1::Known {
                        micros_per_million_tokens: n,
                    } => Some(format!("{}.{:06}", n / 1_000_000, n % 1_000_000)),
                    TokenRateV1::Unknown { .. } => None,
                })
            })
            .collect();
        Self {
            result,
            display_rates,
        }
    }
}
