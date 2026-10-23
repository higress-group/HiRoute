//! Compile only conditions representable by the existing bounded price evaluator.
use super::*;
use hiroute_domain::PriceScheduleV1;

type Compiled = (TokenRatesV1, Option<PriceScheduleV1>);
pub(super) fn compile(price: &DocumentedModelPriceV1) -> Option<Vec<Compiled>> {
    if price.price_state != "exact" {
        return None;
    }
    let mut base = None;
    let mut scheduled = Vec::new();
    for schedule in &price.schedules {
        if schedule.application != "whole-request" {
            return None;
        }
        match schedule.condition.get("kind")?.as_str()? {
            "unconditional" if schedule.condition.len() == 1 => {
                if base.replace(rates(schedule)).is_some() {
                    return None;
                }
            }
            "time-window" => {
                for window in schedule.compiled_time_windows().ok()?? {
                    scheduled.push((rates(schedule), Some(window)));
                }
            }
            "time-window-complement" if schedule.condition.len() == 3 => {
                let key = schedule.condition.get("complement_of")?.as_str()?;
                let target = price.schedules.iter().find(|s| s.schedule_key == key)?;
                let windows = target.compiled_time_windows().ok()??;
                let first = windows.first()?;
                if schedule.condition.get("timezone")?.as_str()? != first.timezone_id {
                    return None;
                }
                for window in complement(&windows)? {
                    scheduled.push((rates(schedule), Some(window)));
                }
            }
            _ => return None,
        }
    }
    if scheduled.len() >= hiroute_application::prices::MAX_PRICE_RULES_PER_TARGET {
        return None;
    }
    let mut result = vec![(
        base.unwrap_or_else(|| {
            TokenRatesV1::unknown(PriceUnknownReasonV1::UnsupportedBillingCondition)
        }),
        None,
    )];
    result.extend(scheduled);
    Some(result)
}

fn complement(windows: &[PriceScheduleV1]) -> Option<Vec<PriceScheduleV1>> {
    let template = windows.first()?;
    let mut result = Vec::new();
    for day in 1..=7 {
        let mut occupied = [false; 1440];
        let previous = if day == 1 { 7 } else { day - 1 };
        for window in windows {
            let begin = usize::from(window.start_minute);
            let end = usize::from(window.end_minute);
            if begin < end {
                if window.weekdays.contains(&day) {
                    occupied[begin..end].fill(true);
                }
            } else {
                if window.weekdays.contains(&day) {
                    occupied[begin..].fill(true);
                }
                if window.weekdays.contains(&previous) {
                    occupied[..end].fill(true);
                }
            }
        }
        let mut start = None;
        for minute in 0u16..=1440 {
            let inside = minute < 1440 && occupied[usize::from(minute)];
            if minute < 1440 && !inside {
                start.get_or_insert(minute);
            } else if let Some(begin) = start.take() {
                let ranges = if begin == 0 && minute == 1440 {
                    vec![(0, 720), (720, 1440)]
                } else {
                    vec![(begin, minute)]
                };
                for (begin, end) in ranges {
                    let mut window = template.clone();
                    window.weekdays = std::collections::BTreeSet::from([day]);
                    window.start_minute = begin;
                    window.end_minute = end % 1440;
                    result.push(window);
                }
            }
        }
    }
    Some(result)
}
