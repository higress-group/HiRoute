use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Flags {
    values: BTreeMap<String, Vec<String>>,
    switches: BTreeSet<String>,
}
impl Flags {
    pub(super) fn parse(
        options: &[String],
        single: &[&str],
        repeated: &[&str],
        switches: &[&str],
    ) -> Result<Self> {
        let mut result = Self {
            values: BTreeMap::new(),
            switches: BTreeSet::new(),
        };
        let mut index = 0;
        while index < options.len() {
            let flag = &options[index];
            if switches.contains(&flag.as_str()) {
                if !result.switches.insert(flag.clone()) {
                    return Err(error(ErrorCode::InvalidArguments));
                }
                index += 1;
                continue;
            }
            if !single.contains(&flag.as_str()) && !repeated.contains(&flag.as_str()) {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let value = options
                .get(index + 1)
                .filter(|value| {
                    crate::request_input::valid_positional(value)
                        && value.len() <= 4096
                        && !value.contains(char::is_control)
                })
                .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
            let values = result.values.entry(flag.clone()).or_default();
            if !values.is_empty() && single.contains(&flag.as_str()) || values.len() >= 128 {
                return Err(error(ErrorCode::InvalidArguments));
            }
            values.push(value.clone());
            index += 2;
        }
        Ok(result)
    }
    pub(super) fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.first())
            .map(String::as_str)
    }
    pub(super) fn all(&self, name: &str) -> &[String] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }
    pub(super) fn has(&self, name: &str) -> bool {
        self.switches.contains(name) || self.values.contains_key(name)
    }
    pub(super) fn required(&self, name: &str) -> Result<&str> {
        self.get(name).ok_or_else(|| choices(&[name], json!({})))
    }
}

pub(super) fn submission(
    options: &[String],
) -> Result<(Vec<String>, Option<String>, Option<String>)> {
    let mut remaining = Vec::new();
    let mut path = None;
    let mut key = None;
    let mut index = 0;
    while index < options.len() {
        let slot = match options[index].as_str() {
            "--apply-file" => Some(&mut path),
            "--idempotency-key" => Some(&mut key),
            _ => None,
        };
        if let Some(slot) = slot {
            if slot.is_some() {
                return Err(error(ErrorCode::InvalidArguments));
            }
            let value = options
                .get(index + 1)
                .filter(|v| {
                    crate::request_input::valid_positional(v)
                        && v.len() <= 4096
                        && !v.contains(char::is_control)
                })
                .ok_or_else(|| error(ErrorCode::InvalidArguments))?;
            *slot = Some(value.clone());
            index += 2;
        } else {
            remaining.push(options[index].clone());
            index += 1;
        }
    }
    if key.as_ref().is_some_and(|key| key.len() > 200) || key.is_some() && path.is_none() {
        return Err(error(ErrorCode::InvalidArguments));
    }
    Ok((remaining, path, key))
}
