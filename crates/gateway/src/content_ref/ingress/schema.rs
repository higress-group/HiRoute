//! Keep only protocol-inspected schema structure; large leaf data stays in Replay.
use super::*;
use crate::server::core_runtime::adapters::{SchemaChildren, schema_children};

impl Compactor<'_> {
    pub(super) fn schema(&mut self, value: &mut Value, depth: usize) -> Result<(), ReplayError> {
        if depth > MAX_JSON_DEPTH {
            return Err(ReplayError::StructureLimit);
        }
        // A literal object in the reserved marker shape is data, never a ref to
        // another range. It has no schema child/encoding fields to inspect.
        if value.as_object().is_some_and(|object| {
            object.len() == 1 && object.contains_key(crate::content_ref::JSON_MARKER_KEY)
        }) {
            return self.externalize_json(value);
        }
        let Some(object) = value.as_object_mut() else {
            // Boolean schemas and invalid shapes remain visible to the adapter.
            return Ok(());
        };
        for (key, value) in object {
            match schema_children(key) {
                Some(SchemaChildren::Map) => {
                    if let Some(children) = value.as_object_mut() {
                        for child in children.values_mut() {
                            self.schema(child, depth + 2)?;
                        }
                    }
                }
                Some(SchemaChildren::Array) => {
                    if let Some(children) = value.as_array_mut() {
                        for child in children {
                            self.schema(child, depth + 2)?;
                        }
                    }
                }
                Some(SchemaChildren::Single) => {
                    if let Some(children) = value.as_array_mut() {
                        for child in children {
                            self.schema(child, depth + 2)?;
                        }
                    } else {
                        self.schema(value, depth + 1)?;
                    }
                }
                None if key == "encrypted" => {}
                None => match value {
                    Value::String(text) => self.externalize_string(text)?,
                    // Default/example/enum/unknown keyword data has no encoding
                    // semantics. Preserve its complete JSON, without visiting
                    // a business key named `parameters`, `encrypted` or `type`.
                    Value::Object(_) | Value::Array(_) => self.externalize_json(value)?,
                    _ => {}
                },
            }
        }
        Ok(())
    }
}
