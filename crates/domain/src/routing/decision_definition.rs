//! The current v1 model-decision vocabulary. Execution policy is kept out of the wire.
use serde::{Deserialize, Serialize};

use super::DegreePolicyV1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrdinalLevelV1 {
    pub id: String,
    pub criterion: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename = "ordinal", deny_unknown_fields)]
pub struct OrdinalDefinitionV1 {
    pub instructions: String,
    pub levels: Vec<OrdinalLevelV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryOptionV1 {
    pub id: String,
    pub criterion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refinement: Option<OrdinalDefinitionV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionDefinitionV1 {
    Ordinal {
        instructions: String,
        levels: Vec<OrdinalLevelV1>,
    },
    Categorical {
        instructions: String,
        options: Vec<CategoryOptionV1>,
    },
}

impl From<OrdinalDefinitionV1> for DecisionDefinitionV1 {
    fn from(value: OrdinalDefinitionV1) -> Self {
        Self::Ordinal {
            instructions: value.instructions,
            levels: value.levels,
        }
    }
}

impl DegreePolicyV1 {
    pub fn definition(&self) -> OrdinalDefinitionV1 {
        OrdinalDefinitionV1 {
            instructions: self.instructions.clone(),
            levels: vec![
                OrdinalLevelV1 {
                    id: "simple".into(),
                    criterion: self.simple.clone(),
                },
                OrdinalLevelV1 {
                    id: "complex".into(),
                    criterion: self.complex.clone(),
                },
            ],
        }
    }
}
