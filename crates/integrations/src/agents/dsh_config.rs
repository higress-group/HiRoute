//! Bounded, passive DSH public patch documents. No YAML expressions or plugins are executed.
use super::pi_sources::clear_native_value;
use super::{QoderNativeError, qoder_error};
use serde_json::{Value, json};
use zeroize::Zeroizing;

pub(super) struct Patch {
    rows: Vec<Value>,
    bytes: Zeroizing<Vec<u8>>,
    spans: Vec<std::ops::Range<usize>>,
}
impl Drop for Patch {
    fn drop(&mut self) {
        self.rows.iter_mut().for_each(clear_native_value);
    }
}
impl Patch {
    pub fn parse(bytes: Option<&[u8]>) -> Result<Self, QoderNativeError> {
        let bytes = bytes.unwrap_or(b"[]\n");
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(qoder_error("DSH patch bound"));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| qoder_error("DSH patch encoding"))?;
        // serde_yaml normalizes unknown secondary tags (e.g. !!js) to scalars.
        // Refuse the executable public extensions before any such normalization.
        if text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| l.contains("!!js") || l.contains("!include"))
        {
            return Err(qoder_error("DSH static patch required"));
        }
        let value: Value = if text.trim().is_empty() {
            json!([])
        } else {
            let yaml: serde_yaml::Value =
                serde_yaml::from_str(text).map_err(|_| qoder_error("DSH static patch required"))?;
            fn static_value(value: &serde_yaml::Value, depth: usize) -> bool {
                if depth > 32 {
                    return false;
                }
                match value {
                    serde_yaml::Value::Tagged(_) => false,
                    serde_yaml::Value::Sequence(values) => {
                        values.iter().all(|v| static_value(v, depth + 1))
                    }
                    serde_yaml::Value::Mapping(values) => values.iter().all(|(k, v)| {
                        k.as_str().is_some_and(|k| k != "<<") && static_value(v, depth + 1)
                    }),
                    _ => true,
                }
            }
            if !static_value(&yaml, 0) {
                return Err(qoder_error("DSH static patch required"));
            }
            serde_json::to_value(yaml).map_err(|_| qoder_error("DSH static patch required"))?
        };
        let rows = value
            .as_array()
            .ok_or_else(|| qoder_error("DSH patch sequence"))?
            .clone();
        let mut offset = 0;
        let mut starts = Vec::new();
        for line in text.split_inclusive('\n') {
            if line.starts_with("- ") || line.trim_end() == "-" {
                starts.push(offset);
            }
            offset += line.len();
        }
        let spans = starts
            .iter()
            .enumerate()
            .map(|(i, start)| *start..starts.get(i + 1).copied().unwrap_or(bytes.len()))
            .collect();
        let mut ids = std::collections::BTreeSet::new();
        for row in &rows {
            if !row.is_object() {
                return Err(qoder_error("DSH patch row"));
            }
            if let Some(id) = row.get("id").and_then(Value::as_str)
                && !ids.insert(id)
            {
                return Err(qoder_error("DSH duplicate patch row"));
            }
        }
        Ok(Self {
            rows,
            bytes: Zeroizing::new(bytes.to_vec()),
            spans,
        })
    }
    pub fn config(&self, id: &str) -> Result<Option<&Value>, QoderNativeError> {
        let row = self.rows.iter().find(|r| r["id"] == id);
        if row.is_some_and(|r| {
            r.get("remove").is_some() || r.get("disabled").is_some_and(|v| v != false)
        }) {
            return Err(qoder_error("DSH disabled target"));
        }
        Ok(row.and_then(|r| r.get("config")))
    }
    pub fn credentials_config(&self) -> Result<Option<&Value>, QoderNativeError> {
        // Nested insert/remove operations can replace the module. Do not infer
        // the default store from a composition this passive reader cannot prove.
        if self
            .rows
            .iter()
            .filter(|r| !bundled_routing_insert(r))
            .any(|r| {
                r["id"].as_str().is_none()
                    || r.as_object().expect("validated row").keys().any(|k| {
                        !matches!(k.as_str(), "id" | "name" | "config" | "disabled" | "remove")
                    })
            })
        {
            return Err(qoder_error("DSH static credentials composition required"));
        }
        if let Some(row) = self.rows.iter().find(|r| r["id"] == "credentials")
            && row
                .get("name")
                .is_some_and(|v| v != "@deepseek-ai/dsh-credentials-local")
        {
            return Err(qoder_error("DSH standard credentials module required"));
        }
        self.config("credentials")
    }
    pub fn providers(&self) -> Result<Value, QoderNativeError> {
        let providers = self
            .config("llm-pi-ai")?
            .and_then(|c| c.get("providers"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !providers.is_object() {
            return Err(qoder_error("DSH providers object"));
        }
        Ok(providers)
    }
    pub fn model_root(&self) -> Result<Value, QoderNativeError> {
        let mut value = json!({"providers":self.providers()?});
        if let Some(default) = self.config("agent-default-model")?
            && let (Some(provider), Some(model)) =
                (default["provider"].as_str(), default["model"].as_str())
        {
            value["model"] = json!({"name":format!("{provider}/{model}")});
        }
        Ok(value)
    }
    pub fn edit_routing_adapter(
        &self,
        uri: &str,
        original: Option<&Self>,
    ) -> Result<Zeroizing<Vec<u8>>, QoderNativeError> {
        let id = "hiroute-routing";
        let declared = json!({"insert":[{"id":id,"name":uri}]});
        let index = self.rows.iter().position(|r| {
            r["insert"]
                .as_array()
                .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == id))
        });
        if index.is_some_and(|i| self.rows[i] != declared) {
            return Err(qoder_error("DSH routing adapter conflict"));
        }
        let desired = match original {
            None => Some(declared.clone()),
            Some(original) => original
                .rows
                .iter()
                .find(|r| {
                    r["insert"]
                        .as_array()
                        .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == id))
                })
                .cloned(),
        };
        if desired.as_ref().is_some_and(|value| value != &declared) {
            return Err(qoder_error("DSH routing adapter original"));
        }
        if self.rows.len() != self.spans.len() {
            return Err(qoder_error("DSH block patch required"));
        }
        let mut output = Zeroizing::new(Vec::new());
        let rendered = desired
            .map(|row| {
                serde_json::to_string_pretty(&row)
                    .map(|s| format!("- {}\n", s.replace('\n', "\n  ")))
            })
            .transpose()
            .map_err(|_| qoder_error("DSH patch encoding"))?;
        if let Some(index) = index {
            let span = &self.spans[index];
            let fragment = Self::parse(Some(&self.bytes[span.clone()]))?;
            if fragment.rows.as_slice() != std::slice::from_ref(&self.rows[index]) {
                return Err(qoder_error("DSH adapter span"));
            }
            output.extend_from_slice(&self.bytes[..span.start]);
            if let Some(rendered) = rendered {
                output.extend_from_slice(rendered.as_bytes());
            }
            output.extend_from_slice(&self.bytes[span.end..]);
        } else {
            output.extend_from_slice(&self.bytes);
            if let Some(rendered) = rendered {
                if self.rows.is_empty() {
                    output.clear();
                }
                if !output.is_empty() && !output.ends_with(b"\n") {
                    output.push(b'\n');
                }
                output.extend_from_slice(rendered.as_bytes());
            }
        }
        Self::parse(Some(&output))?;
        Ok(output)
    }
    pub fn edit_provider(
        &self,
        id: &str,
        provider: Option<&Value>,
    ) -> Result<Zeroizing<Vec<u8>>, QoderNativeError> {
        // Static flow documents are readable, but only block sequences have safe edit spans.
        // An empty sequence has no foreign rows to preserve.
        if self.spans.len() != self.rows.len() {
            return Err(qoder_error("DSH block patch required"));
        }
        if let Some(first) = self.spans.first() {
            let prefix = std::str::from_utf8(&self.bytes[..first.start])
                .map_err(|_| qoder_error("DSH patch encoding"))?;
            if prefix.lines().any(|line| {
                let line = line.trim();
                !line.is_empty() && !line.starts_with('#') && line != "---"
            }) {
                return Err(qoder_error("DSH block patch required"));
            }
            // A dash inside a multiline string is not a top-level row. Prove each span
            // independently represents the corresponding semantic row before editing it.
            for (span, row) in self.spans.iter().zip(&self.rows) {
                let fragment = Self::parse(Some(&self.bytes[span.clone()]))?;
                if fragment.rows.as_slice() != std::slice::from_ref(row) {
                    return Err(qoder_error("DSH block patch required"));
                }
            }
        }
        let index = self.rows.iter().position(|r| r["id"] == "llm-pi-ai");
        let mut row = index
            .map(|i| self.rows[i].clone())
            .unwrap_or_else(|| json!({"id":"llm-pi-ai","config":{"providers":{}}}));
        if row.get("config").is_none() {
            row["config"] = json!({});
        }
        if !row["config"].is_object() {
            return Err(qoder_error("DSH provider config"));
        }
        let mut providers = self.providers()?;
        let map = providers.as_object_mut().expect("validated providers");
        match provider {
            Some(value) => {
                map.insert(id.into(), value.clone());
            }
            None => {
                map.remove(id);
            }
        }
        row["config"]["providers"] = providers;
        // JSON is a public YAML subset; only the affected row is reformatted.
        let rendered =
            serde_json::to_string_pretty(&row).map_err(|_| qoder_error("DSH patch encoding"))?;
        let rendered = format!("- {}\n", rendered.replace('\n', "\n  "));
        let mut output = Zeroizing::new(Vec::new());
        if let Some(index) = index {
            let span = &self.spans[index];
            output.extend_from_slice(&self.bytes[..span.start]);
            output.extend_from_slice(rendered.as_bytes());
            output.extend_from_slice(&self.bytes[span.end..]);
        } else {
            if !self.rows.is_empty() || self.bytes.contains(&b'#') {
                // Preserve leading comments, omitting the empty sequence syntax.
                for line in self.bytes.split_inclusive(|b| *b == b'\n') {
                    if line.starts_with(b"#") || line.iter().all(u8::is_ascii_whitespace) {
                        output.extend_from_slice(line);
                    }
                }
                if !self.rows.is_empty() {
                    output = Zeroizing::new(self.bytes.to_vec());
                }
            }
            if !output.is_empty() && !output.ends_with(b"\n") {
                output.push(b'\n');
            }
            output.extend_from_slice(rendered.as_bytes());
        }
        Self::parse(Some(&output))?;
        Ok(output)
    }
}

/// Native composition files are public-readable; credentials keep the private reader.
pub(super) fn read_patch_bytes(
    path: &std::path::Path,
) -> Result<Zeroizing<Vec<u8>>, super::AgentFilesystemScanError> {
    let observed = super::filesystem_config::read_system_config_bytes(path)?;
    Ok(observed.map(|(bytes, _)| bytes).unwrap_or_default())
}

fn bundled_routing_insert(row: &Value) -> bool {
    let Some(entries) = row.get("insert").and_then(Value::as_array) else {
        return false;
    };
    if row.as_object().is_none_or(|v| v.len() != 1) || entries.len() != 1 {
        return false;
    }
    let entry = &entries[0];
    if entry["id"] != "hiroute-routing" || entry.as_object().is_none_or(|v| v.len() != 2) {
        return false;
    }
    let Some(uri) = entry["name"]
        .as_str()
        .and_then(|name| reqwest::Url::parse(name).ok())
    else {
        return false;
    };
    let Ok(path) = uri.to_file_path() else {
        return false;
    };
    path.file_name()
        .is_some_and(|name| name == "hiroute-routing.mjs")
        && super::filesystem_config::read_system_config_bytes(&path)
            .ok()
            .flatten()
            .is_some_and(|(bytes, _)| bytes.as_slice() == super::DSH_ROUTING_ADAPTER.as_bytes())
}
