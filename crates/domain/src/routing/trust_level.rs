//! Presentation presets over the existing probability threshold. These do not
//! change routing policy, competence floors, permissions or tool approval.
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustLevelV1 {
    pub id: String,
    pub threshold_millis: u16,
    pub label: TrustLevelLabelV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustLevelLabelV1 {
    pub en: String,
    pub zh: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    schema: String,
    levels: Vec<TrustLevelV1>,
}
static LEVELS: LazyLock<Vec<TrustLevelV1>> = LazyLock::new(|| {
    let table: Table = serde_json::from_str(include_str!(
        "../../../../contracts/routing/trust-levels.v1.json"
    ))
    .expect("bundled trust level table");
    assert_eq!(table.schema, "hiroute.trust-levels/v1");
    assert!(
        table
            .levels
            .windows(2)
            .all(|pair| pair[0].threshold_millis > pair[1].threshold_millis)
    );
    table.levels
});
impl TrustLevelV1 {
    pub fn all() -> &'static [Self] {
        &LEVELS
    }
    pub fn from_threshold(threshold_millis: u16) -> Option<&'static Self> {
        Self::all()
            .iter()
            .find(|level| level.threshold_millis == threshold_millis)
    }
    /// The nearest strictly lower threshold, including custom thresholds.
    pub fn step_up(threshold_millis: u16) -> Option<&'static Self> {
        Self::all()
            .iter()
            .find(|level| level.threshold_millis < threshold_millis)
    }
    /// The nearest strictly higher threshold, including custom thresholds.
    pub fn step_down(threshold_millis: u16) -> Option<&'static Self> {
        Self::all()
            .iter()
            .rev()
            .find(|level| level.threshold_millis > threshold_millis)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_thresholds_keep_their_value_and_step_to_adjacent_levels() {
        assert!(TrustLevelV1::from_threshold(720).is_none());
        assert_eq!(TrustLevelV1::step_up(720).unwrap().threshold_millis, 650);
        assert_eq!(TrustLevelV1::step_down(720).unwrap().threshold_millis, 800);
        assert!(TrustLevelV1::step_up(400).is_none());
        assert!(TrustLevelV1::step_down(950).is_none());
        assert_eq!(TrustLevelV1::from_threshold(800).unwrap().id, "familiar");
    }
}
