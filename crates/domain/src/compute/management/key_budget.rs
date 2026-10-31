//! Current Key spending policy. Balances are local estimates, never provider credit balances.
use super::*;
use jiff::{Span, Timestamp, civil::DateTime, tz::TimeZone};
use std::collections::BTreeMap;

pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V6: &str = "hiroute.compute-management-change/v6";
pub const COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V6: &str = "hiroute.compute-management-mutation/v6";
pub const MAX_KEY_BUDGET_MICROS: u64 = 1_000_000_000_000;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyBudgetPeriodV1 {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyBudgetRenewalModeV1 {
    Reset,
    Accumulate,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetRenewalV1 {
    pub period: KeyBudgetPeriodV1,
    pub mode: KeyBudgetRenewalModeV1,
    /// Original local calendar anchor; month-end clamping never changes this anchor.
    pub anchor_local: String,
    pub timezone: String,
    pub balance_cap_micros: Option<u64>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetPolicyV1 {
    pub currency: String,
    pub amount_micros: u64,
    pub renewal: Option<KeyBudgetRenewalV1>,
}

impl KeyBudgetPolicyV1 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        if self.currency.len() != 3
            || !self.currency.bytes().all(|b| b.is_ascii_uppercase())
            || self.amount_micros == 0
            || self.amount_micros > MAX_KEY_BUDGET_MICROS
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        if let Some(renewal) = &self.renewal {
            renewal.anchor()?;
            match (renewal.mode, renewal.balance_cap_micros) {
                (KeyBudgetRenewalModeV1::Reset, None) => {}
                (KeyBudgetRenewalModeV1::Accumulate, Some(cap))
                    if cap >= self.amount_micros && cap <= MAX_KEY_BUDGET_MICROS => {}
                _ => return Err(ComputeManagementErrorV2::InvalidMutation),
            }
        }
        Ok(())
    }
}

impl KeyBudgetRenewalV1 {
    fn anchor(&self) -> Result<jiff::Zoned, ComputeManagementErrorV2> {
        let invalid = |_| ComputeManagementErrorV2::InvalidMutation;
        if self.anchor_local.len() > 32 || self.timezone.len() > 100 {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        let local: DateTime = self.anchor_local.parse().map_err(invalid)?;
        local
            .to_zoned(TimeZone::get(&self.timezone).map_err(invalid)?)
            .map_err(invalid)
    }

    /// Index zero is the first renewal. Every event is computed from the original anchor.
    pub fn event_at(&self, index: i64) -> Result<i64, ComputeManagementErrorV2> {
        if !(0..=4_000_000).contains(&index) {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        let span = match self.period {
            KeyBudgetPeriodV1::Daily => Span::new().days(index),
            KeyBudgetPeriodV1::Weekly => Span::new().weeks(index),
            KeyBudgetPeriodV1::Monthly => Span::new().months(index),
            KeyBudgetPeriodV1::Yearly => Span::new().years(index),
        };
        self.anchor()?
            .checked_add(span)
            .map(|z| z.timestamp().as_millisecond())
            .map_err(|_| ComputeManagementErrorV2::InvalidMutation)
    }

    /// Number of elapsed renewal events. Work is bounded independently of offline duration.
    pub fn elapsed(&self, now_ms: i64) -> Result<i64, ComputeManagementErrorV2> {
        let invalid = |_| ComputeManagementErrorV2::InvalidMutation;
        let anchor = self.anchor()?;
        if now_ms < anchor.timestamp().as_millisecond() {
            return Ok(0);
        }
        let now = Timestamp::from_millisecond(now_ms)
            .map_err(invalid)?
            .to_zoned(anchor.time_zone().clone());
        let days = || {
            anchor
                .date()
                .until(now.date())
                .map(|span| i64::from(span.get_days()))
                .map_err(invalid)
        };
        let estimate = match self.period {
            KeyBudgetPeriodV1::Daily => days()?,
            KeyBudgetPeriodV1::Weekly => days()? / 7,
            KeyBudgetPeriodV1::Monthly => {
                (i64::from(now.year()) - i64::from(anchor.year())) * 12 + i64::from(now.month())
                    - i64::from(anchor.month())
            }
            KeyBudgetPeriodV1::Yearly => i64::from(now.year()) - i64::from(anchor.year()),
        };
        let mut count = estimate.max(0);
        if self.event_at(count)? <= now_ms {
            count += 1;
        }
        if count > 0 && self.event_at(count - 1)? > now_ms {
            count -= 1;
        }
        Ok(count)
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetDefinitionV1 {
    pub budget_id: String,
    pub revision: u64,
    pub display_name: String,
    pub policy: KeyBudgetPolicyV1,
}
impl KeyBudgetDefinitionV1 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        validate_identifier(&self.budget_id)?;
        if self.revision == 0
            || self.display_name.trim().is_empty()
            || self.display_name.len() > 256
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        self.policy.validate()
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyBudgetSelectorV1 {
    Saved { key_id: String },
    Input { candidate_ref: String },
}
impl KeyBudgetSelectorV1 {
    pub fn resolve(&self, source_id: &str) -> Result<String, ComputeManagementErrorV2> {
        match self {
            Self::Saved { key_id } => {
                validate_identifier(key_id)?;
                Ok(key_id.clone())
            }
            Self::Input { candidate_ref } => crate::managed_key_id(source_id, candidate_ref)
                .map_err(|_| ComputeManagementErrorV2::InvalidMutation),
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyBudgetAssignmentV1 {
    Unlimited,
    Share {
        budget_id: String,
        expected_revision: u64,
    },
    Configure {
        budget_id: Option<String>,
        expected_revision: u64,
        display_name: String,
        policy: KeyBudgetPolicyV1,
        initial_remaining_micros: Option<u64>,
    },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetEditV1 {
    pub key: KeyBudgetSelectorV1,
    pub assignment: KeyBudgetAssignmentV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetViewV1 {
    pub definition: KeyBudgetDefinitionV1,
    pub balance_micros: i64,
    pub reserved_micros: u64,
    pub available_micros: u64,
    pub spent_micros: u64,
    pub lifetime_spent_micros: u64,
    pub next_renewal_at_ms: Option<i64>,
    pub key_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetMutationV1 {
    pub bindings_before: BTreeMap<String, String>,
    pub bindings_after: BTreeMap<String, String>,
    pub definitions_before: Vec<KeyBudgetDefinitionV1>,
    pub definitions_after: Vec<KeyBudgetDefinitionV1>,
    pub initial_remaining: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementMutationV6 {
    pub schema: String,
    pub source_mutation: Box<ComputeManagementMutation>,
    pub budgets: KeyBudgetMutationV1,
    pub change_spec_digest: CanonicalDigest,
}

impl ComputeManagementMutationV6 {
    pub fn validate(&self, spec: &ChangeSpecV1) -> Result<(), ComputeManagementErrorV2> {
        if self.schema != COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V6
            || matches!(&*self.source_mutation, ComputeManagementMutation::V6(_))
            || spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V6)
            || CanonicalDigest::of(spec).map_err(|_| ComputeManagementErrorV2::InvalidDigest)?
                != self.change_spec_digest
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        let mut original = spec.clone();
        original.desired_state = spec
            .desired_state
            .get("change")
            .cloned()
            .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
        self.source_mutation.validate_for_spec(&original)?;
        let source = self
            .source_mutation
            .desired()
            .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
        if matches!(
            source.provenance,
            ComputeManagementProvenanceV2::ConnectorOwned { .. }
        ) {
            return Err(ComputeManagementErrorV2::InvalidProvenance);
        }
        self.validate_edits(spec, source)?;
        for key in self.budgets.bindings_after.keys() {
            if !source
                .credentials
                .iter()
                .any(|credential| &credential.key_id == key)
            {
                return Err(ComputeManagementErrorV2::InvalidMutation);
            }
        }
        for definition in self
            .budgets
            .definitions_before
            .iter()
            .chain(&self.budgets.definitions_after)
        {
            definition.validate()?;
        }
        for (id, remaining) in &self.budgets.initial_remaining {
            if self
                .budgets
                .definitions_before
                .iter()
                .any(|d| &d.budget_id == id)
                || self
                    .budgets
                    .definitions_after
                    .iter()
                    .find(|d| &d.budget_id == id)
                    .is_none_or(|d| *remaining > d.policy.amount_micros)
            {
                return Err(ComputeManagementErrorV2::InvalidMutation);
            }
        }
        Ok(())
    }
    fn validate_edits(
        &self,
        spec: &ChangeSpecV1,
        source: &ComputeManagementSourceV2,
    ) -> Result<(), ComputeManagementErrorV2> {
        let bad = ComputeManagementErrorV2::InvalidMutation;
        let edits: Vec<KeyBudgetEditV1> = serde_json::from_value(
            spec.desired_state
                .get("key_budget_edits")
                .cloned()
                .ok_or(bad)?,
        )
        .map_err(|_| bad)?;
        if edits.is_empty() || edits.len() > 256 {
            return Err(bad);
        }
        let before: BTreeMap<_, _> = self
            .budgets
            .definitions_before
            .iter()
            .map(|v| (v.budget_id.clone(), v))
            .collect();
        let after: BTreeMap<_, _> = self
            .budgets
            .definitions_after
            .iter()
            .map(|v| (v.budget_id.clone(), v))
            .collect();
        if before.len() != self.budgets.definitions_before.len()
            || after.len() != self.budgets.definitions_after.len()
        {
            return Err(bad);
        }
        if self.budgets.bindings_before.keys().any(|key| {
            self.source_mutation
                .expected()
                .is_none_or(|s| !s.credentials.iter().any(|v| &v.key_id == key))
        }) {
            return Err(bad);
        }
        let mut bindings = self.budgets.bindings_before.clone();
        bindings.retain(|key, _| source.credentials.iter().any(|v| &v.key_id == key));
        let mut keys = std::collections::BTreeSet::new();
        let mut mentioned = std::collections::BTreeSet::new();
        let mut configured = std::collections::BTreeSet::new();
        let mut initial = BTreeMap::new();
        for edit in &edits {
            let key = edit.key.resolve(&source.source_id)?;
            if !keys.insert(key.clone()) || !source.credentials.iter().any(|v| v.key_id == key) {
                return Err(bad);
            }
            match &edit.assignment {
                KeyBudgetAssignmentV1::Unlimited => {
                    bindings.remove(&key);
                }
                KeyBudgetAssignmentV1::Share {
                    budget_id,
                    expected_revision,
                } => {
                    if before
                        .get(budget_id)
                        .is_none_or(|v| v.revision != *expected_revision)
                        || !after.contains_key(budget_id)
                    {
                        return Err(bad);
                    }
                    mentioned.insert(budget_id.clone());
                    bindings.insert(key, budget_id.clone());
                }
                KeyBudgetAssignmentV1::Configure {
                    budget_id,
                    expected_revision,
                    display_name,
                    policy,
                    initial_remaining_micros,
                } => {
                    let id = if let Some(id) = budget_id {
                        id.clone()
                    } else {
                        let digest = CanonicalDigest::of(&(
                            "hiroute.key-budget/v1",
                            &source.source_id,
                            &key,
                            &spec.desired_state["change"],
                        ))
                        .map_err(|_| bad)?;
                        format!("budget/{}", &digest.as_str()[7..39])
                    };
                    if !configured.insert(id.clone()) {
                        return Err(bad);
                    }
                    let value = after.get(&id).ok_or(bad)?;
                    if value.policy != *policy
                        || value.display_name != *display_name
                        || value.revision != expected_revision.checked_add(1).ok_or(bad)?
                    {
                        return Err(bad);
                    }
                    if let Some(previous) = before.get(&id) {
                        if budget_id.is_none()
                            || previous.revision != *expected_revision
                            || previous.policy.currency != policy.currency
                            || initial_remaining_micros.is_some()
                            || self.budgets.bindings_before.get(&key) != Some(&id)
                        {
                            return Err(bad);
                        }
                    } else {
                        if budget_id.is_some() || *expected_revision != 0 {
                            return Err(bad);
                        }
                        initial.insert(
                            id.clone(),
                            initial_remaining_micros.unwrap_or(policy.amount_micros),
                        );
                    }
                    mentioned.insert(id.clone());
                    bindings.insert(key, id);
                }
            }
        }
        if bindings != self.budgets.bindings_after
            || initial != self.budgets.initial_remaining
            || after.keys().any(|id| !mentioned.contains(id))
            || before.keys().any(|id| !mentioned.contains(id))
        {
            return Err(bad);
        }
        for (id, old) in before {
            if !configured.contains(&id) && after.get(&id) != Some(&old) {
                return Err(bad);
            }
        }
        Ok(())
    }
}
