//! The current price evaluator supports finite, fixed UTC daily windows.
use super::*;
use crate::PriceScheduleV1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    kind: String,
    timezone: String,
    days: Vec<String>,
    windows: Vec<Window>,
    effective_from: i64,
    effective_until: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    start: String,
    end: String,
}

impl DocumentedPriceScheduleV1 {
    /// None is descriptive-only; malformed supported conditions reject the publication.
    pub fn compiled_time_windows(
        &self,
    ) -> Result<Option<Vec<PriceScheduleV1>>, ComputeContractError> {
        if self.condition.get("kind").and_then(Value::as_str) != Some("time-window") {
            return Ok(None);
        }
        let keys = [
            "kind",
            "timezone",
            "days",
            "windows",
            "effective_from",
            "effective_until",
        ];
        if self.condition.keys().any(|k| !keys.contains(&k.as_str())) {
            return Ok(None);
        }
        if !self.condition.contains_key("effective_from")
            && !self.condition.contains_key("effective_until")
        {
            return Ok(None);
        }
        let condition: Condition = serde_json::from_value(
            serde_json::to_value(&self.condition)
                .map_err(|_| ComputeContractError::InvalidModelData)?,
        )
        .map_err(|_| ComputeContractError::InvalidModelData)?;
        if condition.kind != "time-window"
            || condition.effective_from >= condition.effective_until
            || condition.days.is_empty()
            || condition.days.len() > 7
            || condition.windows.is_empty()
            || condition.windows.len() > 32
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        let names = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        let weekdays = condition
            .days
            .iter()
            .map(|day| {
                names
                    .iter()
                    .position(|name| name == day)
                    .map(|index| index as u8 + 1)
                    .ok_or(ComputeContractError::InvalidModelData)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if weekdays.len() != condition.days.len() {
            return Err(ComputeContractError::InvalidModelData);
        }
        let mut result = Vec::new();
        for window in condition.windows {
            let start = minute(&window.start)?;
            let end = minute(&window.end)?;
            if start == end {
                return Err(ComputeContractError::InvalidModelData);
            }
            result.push(PriceScheduleV1 {
                timezone_id: condition.timezone.clone(),
                timezone_data_revision: "fixed-utc/v1".into(),
                schedule_compile_version: "documented-time/v1".into(),
                effective_from: condition.effective_from,
                effective_until: condition.effective_until,
                utc_offset_minutes: 0,
                weekdays: weekdays.clone(),
                start_minute: start,
                end_minute: end,
            });
        }
        if !matches!(condition.timezone.as_str(), "UTC" | "Etc/UTC") {
            return Ok(None);
        }
        Ok(Some(result))
    }
}
fn minute(value: &str) -> Result<u16, ComputeContractError> {
    if value.len() != 5 || value.as_bytes()[2] != b':' {
        return Err(ComputeContractError::InvalidModelData);
    }
    let hour = value.get(..2).and_then(|s| s.parse::<u16>().ok());
    let minute = value.get(3..).and_then(|s| s.parse::<u16>().ok());
    match (hour, minute) {
        (Some(h), Some(m)) if h < 24 && m < 60 => Ok(h * 60 + m),
        _ => Err(ComputeContractError::InvalidModelData),
    }
}
