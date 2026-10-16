//! JSON request transport only; command owners retain their typed schema and byte limit.

use std::io::Read;
use std::path::Path;

use hiroute_application_api::ErrorCode;
use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

pub(crate) mod diagnostics;
mod fields;

pub(crate) const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

pub(crate) enum RequestInput<'a> {
    Stdin,
    File(&'a Path),
}

impl<'a> RequestInput<'a> {
    pub(crate) fn parse(options: &'a [String]) -> Result<Self, ErrorCode> {
        match options {
            [flag] if flag == "--request-stdin" => Ok(Self::Stdin),
            [flag, path] if flag == "--request-file" && valid_positional(path) => {
                Ok(Self::File(Path::new(path)))
            }
            _ => Err(ErrorCode::InvalidArguments),
        }
    }

    /// Human/positional codecs may fall back only when no request source was supplied.
    pub(crate) fn optional(options: &'a [String]) -> Result<Option<Self>, ErrorCode> {
        if options
            .iter()
            .any(|option| matches!(option.as_str(), "--request-stdin" | "--request-file"))
        {
            Self::parse(options).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn read(self, limit: u64) -> Result<Value, ErrorCode> {
        match self {
            Self::Stdin => read_json(std::io::stdin(), limit),
            Self::File(path) => {
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOCTTY);
                }
                let file = options
                    .open(path)
                    .map_err(|_| ErrorCode::InvalidArguments)?;
                // A request file is a finite regular file, never a blocking device or pipe.
                if !file
                    .metadata()
                    .map_err(|_| ErrorCode::InvalidArguments)?
                    .is_file()
                {
                    return Err(ErrorCode::InvalidArguments);
                }
                read_json(file, limit)
            }
        }
    }
}

pub(crate) fn read(options: &[String], limit: u64) -> Result<Value, ErrorCode> {
    RequestInput::parse(options)?.read(limit)
}

pub(crate) fn typed<T: DeserializeOwned + Serialize>(
    options: &[String],
    limit: u64,
) -> Result<Value, ErrorCode> {
    let request: T = deserialize(read(options, limit)?)?;
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

pub(crate) fn deserialize<T: DeserializeOwned + Serialize>(value: Value) -> Result<T, ErrorCode> {
    let request = serde_path_to_error::deserialize(value.clone()).map_err(|error| {
        diagnostics::typed(&error);
        ErrorCode::InvalidArguments
    })?;
    fields::validate(&request, &value).map_err(|_| {
        diagnostics::record("unknown_field", "$", "only declared request fields");
        ErrorCode::InvalidArguments
    })?;
    Ok(request)
}

pub(crate) fn valid_positional(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-')
}

pub(crate) fn read_json(reader: impl Read, limit: u64) -> Result<Value, ErrorCode> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.len() as u64 > limit {
        diagnostics::record(
            "input_too_large",
            "$",
            "a request within the documented byte limit",
        );
        return Err(ErrorCode::InvalidArguments);
    }
    let StrictValue(value) = serde_json::from_slice(&bytes).map_err(|error| {
        diagnostics::json(&error);
        ErrorCode::InvalidArguments
    })?;
    Ok(value)
}

// Value's normal deserializer silently keeps the last duplicate member. Reject duplicates
// recursively before command owners choose a schema or deserialize tagged/flattened DTOs.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("one JSON value without duplicate object members")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(StrictValue)
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(StrictValue(value)) = access.next_element()? {
                    values.push(value);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON member"));
                    }
                    let StrictValue(value) = access.next_value()?;
                    values.insert(key, value);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_or_missing_sources_before_reading() {
        for options in [
            vec![],
            vec!["--request-file"],
            vec!["--request-file", ""],
            vec!["--request-file", "--request-stdin"],
            vec!["--request-stdin", "--request-file", "request.json"],
            vec!["--request-stdin", "--request-stdin"],
            vec!["--request-file", "one", "--request-file", "two"],
            vec!["id", "--request-stdin"],
        ] {
            assert!(
                RequestInput::parse(&options.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .is_err()
            );
        }
    }

    #[test]
    fn strict_json_rejects_duplicate_members_trailing_input_and_invalid_utf8() {
        for bytes in [
            br#"{"schema":"v1","schema":"v2"}"#.as_slice(),
            br#"{"nested":[{"id":"first","id":"second"}]}"#,
            br#"{"id":"first","\u0069d":"second"}"#,
            b"{} {}",
            b"",
            b"\xff",
        ] {
            assert_eq!(
                read_json(bytes, MAX_REQUEST_BYTES),
                Err(ErrorCode::InvalidArguments)
            );
        }
        let bytes = br#"{"array":[null,true,-2,3,1.5,"text"]}"#;
        assert_eq!(
            read_json(bytes.as_slice(), bytes.len() as u64).unwrap(),
            serde_json::json!({"array":[null,true,-2,3,1.5,"text"]})
        );
        assert_eq!(
            read_json(bytes.as_slice(), bytes.len() as u64 - 1),
            Err(ErrorCode::InvalidArguments)
        );
    }
}
