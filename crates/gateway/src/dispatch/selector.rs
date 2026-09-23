use serde_json::Value;
use thiserror::Error;

const MAX_ALIAS_BYTES: usize = 128;

/// Bounded incremental extractor for the top-level `model` selector. It stops
/// as soon as that string closes and never materializes unrelated body values.
#[derive(Debug)]
pub struct ModelSelector {
    bytes: Vec<u8>,
    limit: usize,
}

impl ModelSelector {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(4_096)),
            limit,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<Option<String>, SelectorError> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        match extract_top_level_model(&self.bytes)? {
            Extract::Found(model) => return validate_model(model).map(Some),
            Extract::NeedMore => {}
        }
        if chunk.len() > remaining || self.bytes.len() == self.limit {
            return Err(SelectorError::LimitExceeded(self.limit));
        }
        Ok(None)
    }

    pub fn finish(&self) -> Result<String, SelectorError> {
        if let Extract::Found(model) = extract_top_level_model(&self.bytes)? {
            return validate_model(model);
        }
        let value: Value =
            serde_json::from_slice(&self.bytes).map_err(|_| SelectorError::Invalid)?;
        let model = value
            .as_object()
            .and_then(|object| object.get("model"))
            .and_then(Value::as_str)
            .ok_or(SelectorError::Missing)?;
        validate_model(model.to_owned())
    }
}

fn validate_model(model: String) -> Result<String, SelectorError> {
    if model.is_empty() || model.len() > MAX_ALIAS_BYTES {
        Err(SelectorError::InvalidModel)
    } else {
        Ok(model)
    }
}

#[derive(Debug)]
enum Extract {
    Found(String),
    NeedMore,
}

fn extract_top_level_model(bytes: &[u8]) -> Result<Extract, SelectorError> {
    let mut index = skip_whitespace(bytes, 0);
    if bytes.get(index) != Some(&b'{') {
        return if index == bytes.len() {
            Ok(Extract::NeedMore)
        } else {
            Err(SelectorError::Invalid)
        };
    }
    index += 1;
    loop {
        index = skip_whitespace(bytes, index);
        if index == bytes.len() {
            return Ok(Extract::NeedMore);
        }
        if bytes[index] == b'}' {
            return Err(SelectorError::Missing);
        }
        let Some((key, next)) = parse_string(bytes, index)? else {
            return Ok(Extract::NeedMore);
        };
        index = skip_whitespace(bytes, next);
        if index == bytes.len() {
            return Ok(Extract::NeedMore);
        }
        if bytes[index] != b':' {
            return Err(SelectorError::Invalid);
        }
        index = skip_whitespace(bytes, index + 1);
        if index == bytes.len() {
            return Ok(Extract::NeedMore);
        }
        if key == "model" {
            let Some((model, _)) = parse_string(bytes, index)? else {
                return Ok(Extract::NeedMore);
            };
            return Ok(Extract::Found(model));
        }
        let Some(next) = skip_json_value(bytes, index)? else {
            return Ok(Extract::NeedMore);
        };
        index = skip_whitespace(bytes, next);
        match bytes.get(index) {
            Some(b',') => index += 1,
            Some(b'}') => return Err(SelectorError::Missing),
            None => return Ok(Extract::NeedMore),
            _ => return Err(SelectorError::Invalid),
        }
    }
}

fn parse_string(bytes: &[u8], start: usize) -> Result<Option<(String, usize)>, SelectorError> {
    if bytes.get(start) != Some(&b'"') {
        return Err(SelectorError::Invalid);
    }
    let mut index = start + 1;
    let mut escaped = false;
    while let Some(byte) = bytes.get(index).copied() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            let value = serde_json::from_slice::<String>(&bytes[start..=index])
                .map_err(|_| SelectorError::Invalid)?;
            return Ok(Some((value, index + 1)));
        } else if byte < 0x20 {
            return Err(SelectorError::Invalid);
        }
        index += 1;
    }
    Ok(None)
}

fn skip_json_value(bytes: &[u8], start: usize) -> Result<Option<usize>, SelectorError> {
    if start == bytes.len() {
        return Ok(None);
    }
    if bytes[start] == b'"' {
        return parse_string(bytes, start).map(|value| value.map(|(_, next)| next));
    }
    if !matches!(bytes[start], b'{' | b'[') {
        let mut index = start;
        while let Some(byte) = bytes.get(index) {
            if matches!(byte, b',' | b'}') {
                return if skip_whitespace(bytes, start) == index {
                    Err(SelectorError::Invalid)
                } else {
                    Ok(Some(index))
                };
            }
            index += 1;
        }
        return Ok(None);
    }
    let mut stack = vec![bytes[start]];
    let mut index = start + 1;
    let mut in_string = false;
    let mut escaped = false;
    while let Some(byte) = bytes.get(index).copied() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => stack.push(byte),
                b'}' if stack.last() == Some(&b'{') => {
                    stack.pop();
                }
                b']' if stack.last() == Some(&b'[') => {
                    stack.pop();
                }
                b'}' | b']' => return Err(SelectorError::Invalid),
                _ => {}
            }
            if stack.is_empty() {
                return Ok(Some(index + 1));
            }
        }
        index += 1;
    }
    Ok(None)
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SelectorError {
    #[error("request selector JSON is invalid")]
    Invalid,
    #[error("request model selector is missing")]
    Missing,
    #[error("request model selector is invalid")]
    InvalidModel,
    #[error("request model selector exceeded {0} bytes")]
    LimitExceeded(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_stops_before_later_secret_and_ignores_nested_model() {
        let mut selector = ModelSelector::new(64);
        let body = br#"{"metadata":{"model":"wrong"},"model":"plan-fast","secret":"later"}"#;
        assert_eq!(selector.feed(body).unwrap(), Some("plan-fast".into()));
    }

    #[test]
    fn selector_is_incremental_and_bounded() {
        let mut selector = ModelSelector::new(32);
        assert_eq!(selector.feed(br#"{"model":"plan-"#).unwrap(), None);
        assert_eq!(
            selector.feed(br#"fast","input":"#).unwrap(),
            Some("plan-fast".into())
        );
        assert_eq!(
            ModelSelector::new(8).feed(br#"{"input":"too late"}"#),
            Err(SelectorError::LimitExceeded(8))
        );
    }

    #[test]
    fn selector_waits_when_a_model_value_has_not_arrived() {
        let mut selector = ModelSelector::new(64);
        for chunk in [
            br#"{"m"#.as_slice(),
            b"ode",
            br#"l":"#.as_slice(),
            br#""wi"#.as_slice(),
        ] {
            assert_eq!(selector.feed(chunk).unwrap(), None);
        }
        assert_eq!(
            selector.feed(br#"re-protocol"}"#).unwrap(),
            Some("wire-protocol".into())
        );

        assert_eq!(
            ModelSelector::new(32).feed(br#"{"model":   "#).unwrap(),
            None
        );
        assert_eq!(
            ModelSelector::new(32).feed(br#"{"model":1}"#),
            Err(SelectorError::Invalid)
        );
    }
}
