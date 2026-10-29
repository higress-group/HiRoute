use thiserror::Error;

const MAX_ALIAS_BYTES: usize = 128;
// A JSON escape uses at most six wire bytes per decoded ASCII byte.
const MAX_MODEL_TOKEN_BYTES: usize = MAX_ALIAS_BYTES * 6 + 2;
const MAX_KEY_TOKEN_BYTES: usize = 5 * 6 + 2;
const MAX_DEPTH: usize = 128;

/// Locates the first top-level model without retaining or rescanning its prefix.
/// Full JSON and protocol validation still belongs to the canonical decoder.
#[derive(Debug)]
pub struct ModelSelector {
    state: State,
    token: [u8; MAX_MODEL_TOKEN_BYTES],
    token_len: usize,
    token_overflow: bool,
    escaped: bool,
    model_key: bool,
    stack: [u8; MAX_DEPTH],
    depth: usize,
    observed: usize,
    limit: usize,
    selected: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum State {
    Start,
    Key,
    Colon,
    Value,
    AfterValue,
    String(StringKind),
    Nested,
    Primitive,
}

#[derive(Clone, Copy, Debug)]
enum StringKind {
    Key,
    Model,
    Skipped,
    Nested,
}

impl ModelSelector {
    // Includes the fixed state, bounded alias and serde's bounded token scratch.
    pub(crate) const MEMORY_BYTES: usize = 4 * 1024;

    pub fn new(limit: usize) -> Self {
        Self {
            state: State::Start,
            token: [0; MAX_MODEL_TOKEN_BYTES],
            token_len: 0,
            token_overflow: false,
            escaped: false,
            model_key: false,
            stack: [0; MAX_DEPTH],
            depth: 0,
            observed: 0,
            limit,
            selected: None,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<Option<String>, SelectorError> {
        if let Some(model) = &self.selected {
            return Ok(Some(model.clone()));
        }
        for &byte in chunk {
            if self.observed == self.limit {
                return Err(SelectorError::LimitExceeded(self.limit));
            }
            self.observed += 1;
            self.step(byte)?;
            if let Some(model) = &self.selected {
                return Ok(Some(model.clone()));
            }
        }
        if self.observed == self.limit {
            return Err(SelectorError::LimitExceeded(self.limit));
        }
        Ok(None)
    }

    pub fn finish(&self) -> Result<String, SelectorError> {
        self.selected.clone().ok_or(SelectorError::Invalid)
    }

    fn step(&mut self, byte: u8) -> Result<(), SelectorError> {
        match self.state {
            State::String(kind) => return self.string_byte(kind, byte),
            State::Nested => {
                match byte {
                    b'"' => self.start_string(StringKind::Nested),
                    b'{' | b'[' => self.push(byte)?,
                    b'}' | b']' => {
                        let expected = if byte == b'}' { b'{' } else { b'[' };
                        if self.depth == 0 || self.stack[self.depth - 1] != expected {
                            return Err(SelectorError::Invalid);
                        }
                        self.depth -= 1;
                        if self.depth == 0 {
                            self.state = State::AfterValue;
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            State::Primitive => {
                return match byte {
                    b',' => {
                        self.state = State::Key;
                        Ok(())
                    }
                    b'}' => Err(SelectorError::Missing),
                    _ => Ok(()),
                };
            }
            _ if byte.is_ascii_whitespace() => return Ok(()),
            _ => {}
        }
        match (self.state, byte) {
            (State::Start, b'{') => self.state = State::Key,
            (State::Key, b'"') => self.start_string(StringKind::Key),
            (State::Key, b'}') | (State::AfterValue, b'}') => {
                return Err(SelectorError::Missing);
            }
            (State::Colon, b':') => self.state = State::Value,
            (State::Value, b'"') => self.start_string(if self.model_key {
                StringKind::Model
            } else {
                StringKind::Skipped
            }),
            (State::Value, _) if self.model_key => return Err(SelectorError::Invalid),
            (State::Value, b'{' | b'[') => {
                self.push(byte)?;
                self.state = State::Nested;
            }
            (State::Value, b',' | b'}') => return Err(SelectorError::Invalid),
            (State::Value, _) => self.state = State::Primitive,
            (State::AfterValue, b',') => self.state = State::Key,
            _ => return Err(SelectorError::Invalid),
        }
        Ok(())
    }

    fn push(&mut self, byte: u8) -> Result<(), SelectorError> {
        if self.depth == MAX_DEPTH {
            return Err(SelectorError::Invalid);
        }
        self.stack[self.depth] = byte;
        self.depth += 1;
        Ok(())
    }

    fn start_string(&mut self, kind: StringKind) {
        self.state = State::String(kind);
        self.escaped = false;
        self.token_len = 1;
        self.token[0] = b'"';
        self.token_overflow = false;
    }

    fn string_byte(&mut self, kind: StringKind, byte: u8) -> Result<(), SelectorError> {
        if matches!(kind, StringKind::Key | StringKind::Model) {
            let limit = if matches!(kind, StringKind::Key) {
                MAX_KEY_TOKEN_BYTES
            } else {
                MAX_MODEL_TOKEN_BYTES
            };
            if self.token_len < limit {
                self.token[self.token_len] = byte;
                self.token_len += 1;
            } else if matches!(kind, StringKind::Model) {
                return Err(SelectorError::InvalidModel);
            } else {
                // A longer key cannot decode to "model". Do not retain it.
                self.token_overflow = true;
            }
        }
        if self.escaped {
            self.escaped = false;
        } else if byte == b'\\' {
            self.escaped = true;
        } else if byte == b'"' {
            self.state = match kind {
                StringKind::Key => {
                    self.model_key = !self.token_overflow
                        && serde_json::from_slice::<String>(&self.token[..self.token_len])
                            .map_err(|_| SelectorError::Invalid)?
                            == "model";
                    State::Colon
                }
                StringKind::Model => {
                    let model = serde_json::from_slice::<String>(&self.token[..self.token_len])
                        .map_err(|_| SelectorError::Invalid)?;
                    if model.is_empty() || model.len() > MAX_ALIAS_BYTES {
                        return Err(SelectorError::InvalidModel);
                    }
                    self.selected = Some(model);
                    State::AfterValue
                }
                StringKind::Skipped => State::AfterValue,
                StringKind::Nested => State::Nested,
            };
        } else if byte < 0x20 {
            return Err(SelectorError::Invalid);
        }
        Ok(())
    }
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
#[path = "selector_tests.rs"]
mod tests;
