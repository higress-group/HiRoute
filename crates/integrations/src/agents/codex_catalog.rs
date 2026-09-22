use std::collections::BTreeSet;

use hiroute_domain::valid_client_model_name;
use serde::Deserialize;
use serde_json::Value;

pub const CODEX_CATALOG_SOURCE_REVISION: &str = "be6e8eac029b183056b7e4402879f15d2c85f61b";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexCatalogError {
    #[error(
        "remove model_context_window and model_auto_compact_token_limit from the effective Codex configuration before using plan windows"
    )]
    ContextOverride,
    #[error("the catalog selection fields are invalid or ambiguous")]
    InvalidCatalog,
    #[error("the native default cannot be resolved")]
    MissingDefault,
    #[error("appending plans would change the native default")]
    DefaultChanged,
    #[error("no priority remains after the original catalog")]
    PriorityExhausted,
    #[error("the plan's common client capabilities are not proven")]
    CapabilityUnproven,
}

pub struct CodexCatalogSelection {
    original: Value,
    entries: Vec<SelectionEntry>,
}

#[derive(Deserialize)]
struct SelectionEntry {
    slug: String,
    priority: i32,
    visibility: Visibility,
    supported_in_api: bool,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Visibility {
    List,
    Hide,
    None,
}

#[derive(Clone, Copy)]
pub struct CodexDefaultPolicy<'a> {
    pub explicit_model: Option<&'a str>,
    pub uses_codex_backend: bool,
    pub allow_provider_model_fallback: bool,
}

impl CodexCatalogSelection {
    /// Parse the selected complete catalog using this adapter's structural contract.
    ///
    /// This intentionally does not authenticate or version-gate a Codex executable. Runtime
    /// behavior belongs to the selected surface check; catalog parsing only protects the
    /// structure and default-preservation semantics needed for a recoverable native edit.
    pub fn for_current_adapter(original: Value) -> Result<Self, CodexCatalogError> {
        let selected = Self::parse(original)?;
        selected.validate_schema()?;
        Ok(selected)
    }

    pub(super) fn parse(original: Value) -> Result<Self, CodexCatalogError> {
        let models = original
            .get("models")
            .and_then(Value::as_array)
            .filter(|models| !models.is_empty())
            .ok_or(CodexCatalogError::InvalidCatalog)?;
        let mut names = BTreeSet::new();
        let entries = models
            .iter()
            .map(|model| {
                let entry: SelectionEntry = serde_json::from_value(model.clone())
                    .map_err(|_| CodexCatalogError::InvalidCatalog)?;
                if !valid_client_model_name(&entry.slug) || !names.insert(entry.slug.clone()) {
                    return Err(CodexCatalogError::InvalidCatalog);
                }
                Ok(entry)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { original, entries })
    }

    pub fn original(&self) -> &Value {
        &self.original
    }

    // Pinned ModelsManager::build_available_models and ModelPreset picker default rules.
    pub fn effective_default<'a>(
        &'a self,
        policy: CodexDefaultPolicy<'a>,
    ) -> Result<&'a str, CodexCatalogError> {
        let mut available = self
            .entries
            .iter()
            .filter(|entry| policy.uses_codex_backend || entry.supported_in_api)
            .collect::<Vec<_>>();
        available.sort_by_key(|entry| entry.priority);
        if let Some(requested) = policy.explicit_model {
            if !valid_client_model_name(requested) {
                return Err(CodexCatalogError::MissingDefault);
            }
            if !policy.allow_provider_model_fallback
                || available.iter().any(|entry| entry.slug == requested)
            {
                return Ok(requested);
            }
        }
        available
            .iter()
            .find(|entry| entry.visibility == Visibility::List)
            .or_else(|| available.first())
            .map(|entry| entry.slug.as_str())
            .ok_or(CodexCatalogError::MissingDefault)
    }

    pub fn require_preserved_default(
        &self,
        updated: &Self,
        policy: CodexDefaultPolicy<'_>,
    ) -> Result<(), CodexCatalogError> {
        if self.effective_default(policy)? != updated.effective_default(policy)? {
            return Err(CodexCatalogError::DefaultChanged);
        }
        Ok(())
    }

    pub fn plan_priorities(&self, count: usize) -> Result<Vec<i32>, CodexCatalogError> {
        let last = self
            .entries
            .iter()
            .map(|entry| entry.priority)
            .max()
            .ok_or(CodexCatalogError::InvalidCatalog)?;
        let count = i32::try_from(count).map_err(|_| CodexCatalogError::PriorityExhausted)?;
        last.checked_add(count)
            .ok_or(CodexCatalogError::PriorityExhausted)?;
        Ok((1..=count).map(|offset| last + offset).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog(models: Value) -> CodexCatalogSelection {
        CodexCatalogSelection::parse(json!({"models": models, "opaque": {"retain": [null, 7]}}))
            .unwrap()
    }

    fn policy() -> CodexDefaultPolicy<'static> {
        CodexDefaultPolicy {
            explicit_model: None,
            uses_codex_backend: false,
            allow_provider_model_fallback: false,
        }
    }

    fn entry(slug: &str, priority: i32, visibility: &str, api: bool) -> Value {
        json!({"slug": slug, "priority": priority, "visibility": visibility, "supported_in_api": api, "unknown": {"original": true}})
    }

    #[test]
    fn preserves_original_json_and_orders_only_selection() {
        let models = json!([
            entry("Later.Model", 9, "list", true),
            entry("First/Model", -1, "list", true)
        ]);
        let source = catalog(models.clone());
        assert_eq!(source.original()["models"], models);
        assert_eq!(source.original()["opaque"]["retain"], json!([null, 7]));
        assert_eq!(source.effective_default(policy()), Ok("First/Model"));
        assert_eq!(source.plan_priorities(2), Ok(vec![10, 11]));
    }

    #[test]
    fn auth_filters_before_picker_default() {
        let source = catalog(json!([
            entry("subscription", 0, "list", false),
            entry("api", 1, "list", true)
        ]));
        assert_eq!(source.effective_default(policy()), Ok("api"));
        assert_eq!(
            source.effective_default(CodexDefaultPolicy {
                uses_codex_backend: true,
                ..policy()
            }),
            Ok("subscription")
        );
    }

    #[test]
    fn visible_model_precedes_hidden_default_but_hidden_is_last_resort() {
        let source = catalog(json!([
            entry("hidden", 0, "hide", true),
            entry("visible", 1, "list", true)
        ]));
        assert_eq!(source.effective_default(policy()), Ok("visible"));
        let source = catalog(json!([
            entry("hidden", 0, "none", true),
            entry("later", 1, "hide", true)
        ]));
        assert_eq!(source.effective_default(policy()), Ok("hidden"));
    }

    #[test]
    fn equal_priority_preserves_original_order() {
        let source = catalog(json!([
            entry("z-first", 0, "list", true),
            entry("a-second", 0, "list", true)
        ]));
        assert_eq!(source.effective_default(policy()), Ok("z-first"));
    }

    #[test]
    fn explicit_default_obeys_provider_fallback_without_writing_model() {
        let source = catalog(json!([entry("api", 1, "list", true)]));
        let explicit = CodexDefaultPolicy {
            explicit_model: Some("external"),
            ..policy()
        };
        assert_eq!(source.effective_default(explicit), Ok("external"));
        assert_eq!(
            source.effective_default(CodexDefaultPolicy {
                allow_provider_model_fallback: true,
                ..explicit
            }),
            Ok("api")
        );
        assert!(source.original().get("model").is_none());
    }

    #[test]
    fn missing_available_model_does_not_invent_a_default() {
        let source = catalog(json!([entry("subscription", 0, "list", false)]));
        assert_eq!(
            source.effective_default(policy()),
            Err(CodexCatalogError::MissingDefault)
        );
    }

    #[test]
    fn appended_visible_plan_must_not_replace_hidden_native_default() {
        let native = entry("hidden", 0, "hide", true);
        let source = catalog(json!([native]));
        let updated = catalog(json!([native, entry("plan", 1, "list", true)]));
        assert_eq!(
            source.require_preserved_default(&updated, policy()),
            Err(CodexCatalogError::DefaultChanged)
        );
        assert_eq!(
            source.require_preserved_default(
                &updated,
                CodexDefaultPolicy {
                    explicit_model: Some("hidden"),
                    ..policy()
                }
            ),
            Ok(())
        );
    }

    #[test]
    fn priority_overflow_is_rejected_without_changing_original() {
        let source = catalog(json!([entry("last", i32::MAX, "list", true)]));
        assert_eq!(
            source.plan_priorities(1),
            Err(CodexCatalogError::PriorityExhausted)
        );
        assert_eq!(source.plan_priorities(0), Ok(vec![]));
    }

    #[test]
    fn duplicate_original_names_are_not_merged() {
        let models =
            json!({"models": [entry("same", 0, "list", true), entry("same", 1, "hide", false)]});
        assert!(matches!(
            CodexCatalogSelection::parse(models),
            Err(CodexCatalogError::InvalidCatalog)
        ));
    }

    #[test]
    fn model_list_projection_is_not_an_original_catalog() {
        assert!(matches!(
            CodexCatalogSelection::parse(json!({"data": [{"id": "api"}]})),
            Err(CodexCatalogError::InvalidCatalog)
        ));
    }

    #[test]
    fn empty_catalog_does_not_invent_metadata() {
        assert!(matches!(
            CodexCatalogSelection::parse(json!({"models": []})),
            Err(CodexCatalogError::InvalidCatalog)
        ));
    }

    #[test]
    fn incomplete_selection_facts_are_rejected() {
        assert!(matches!(
            CodexCatalogSelection::parse(json!({"models": [{"slug": "api", "priority": 0}]})),
            Err(CodexCatalogError::InvalidCatalog)
        ));
    }
}
