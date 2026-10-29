//! JSON Schema positions are owned here, shared by negotiation and Replay storage.
//! A business property/const/default/example named `encrypted` is not an annotation.
use serde_json::Value;

use super::ModelIrError;

#[derive(Clone, Copy)]
pub(crate) enum SchemaChildren {
    Map,
    Array,
    Single,
}

pub(crate) fn schema_children(key: &str) -> Option<SchemaChildren> {
    match key {
        "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
            Some(SchemaChildren::Map)
        }
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => Some(SchemaChildren::Array),
        "items"
        | "contains"
        | "additionalProperties"
        | "additionalItems"
        | "contentSchema"
        | "unevaluatedProperties"
        | "unevaluatedItems"
        | "not"
        | "if"
        | "then"
        | "else"
        | "propertyNames" => Some(SchemaChildren::Single),
        _ => None,
    }
}

pub(super) fn project_schema(value: &mut Value) -> Result<bool, ModelIrError> {
    if value.is_boolean() {
        return Ok(false);
    }
    let schema = value
        .as_object_mut()
        .ok_or(ModelIrError::InvalidField("function.parameters schema"))?;
    if schema.get("encrypted").is_some_and(|v| !v.is_boolean()) {
        return Err(ModelIrError::InvalidField("schema.encrypted"));
    }
    let mut negotiated = schema.remove("encrypted").is_some();
    for (key, value) in schema.iter_mut() {
        match schema_children(key) {
            Some(SchemaChildren::Map) => {
                for child in value
                    .as_object_mut()
                    .ok_or(ModelIrError::InvalidField("schema map"))?
                    .values_mut()
                {
                    negotiated |= project_schema(child)?;
                }
            }
            Some(SchemaChildren::Array) => {
                for child in value
                    .as_array_mut()
                    .ok_or(ModelIrError::InvalidField("schema array"))?
                {
                    negotiated |= project_schema(child)?;
                }
            }
            Some(SchemaChildren::Single) => {
                if let Some(children) = value.as_array_mut() {
                    for child in children {
                        negotiated |= project_schema(child)?;
                    }
                } else {
                    negotiated |= project_schema(value)?;
                }
            }
            None => {}
        }
    }
    Ok(negotiated)
}
