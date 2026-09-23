//! Hosted search is not a client-executed function. Keep its native controls typed.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlCitationV1 {
    #[serde(rename = "type")]
    pub kind: UrlCitationKind,
    pub start_index: u32,
    pub end_index: u32,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UrlCitationKind {
    #[serde(rename = "url_citation")]
    UrlCitation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchToolV1 {
    #[serde(rename = "type")]
    pub kind: WebSearchToolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_web_access: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WebSearchToolKind {
    #[serde(rename = "web_search")]
    WebSearch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchCallV1 {
    pub id: String,
    pub status: WebSearchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<WebSearchAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchStatus {
    InProgress,
    Searching,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WebSearchAction {
    Search {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queries: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sources: Option<Vec<WebSearchSource>>,
    },
    OpenPage {
        url: String,
    },
    Find {
        url: String,
        pattern: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchSource {
    #[serde(rename = "type")]
    pub kind: WebSearchSourceKind,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WebSearchSourceKind {
    #[serde(rename = "url")]
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchPhase {
    Added,
    InProgress,
    Searching,
    Completed,
    Done,
}

impl WebSearchCallV1 {
    pub fn wire_value(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("typed search call serialization");
        value["type"] = "web_search_call".into();
        value
    }

    pub fn from_wire(value: &serde_json::Value) -> Result<Self, super::ModelIrError> {
        let mut value = value.clone();
        let object = value
            .as_object_mut()
            .ok_or(super::ModelIrError::InvalidField("web_search_call"))?;
        if object
            .remove("type")
            .as_ref()
            .and_then(serde_json::Value::as_str)
            != Some("web_search_call")
        {
            return Err(super::ModelIrError::InvalidField("web_search_call.type"));
        }
        let item: Self = serde_json::from_value(value)
            .map_err(|_| super::ModelIrError::InvalidField("web_search_call"))?;
        if item.id.is_empty() || item.id.chars().any(char::is_control) {
            return Err(super::ModelIrError::InvalidField("web_search_call.id"));
        }
        Ok(item)
    }
}
