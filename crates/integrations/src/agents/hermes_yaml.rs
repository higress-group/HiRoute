//! Static Hermes mappings and bounded member edits. Unowned byte ranges remain untouched.
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::ops::Range;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Hermes native configuration is unavailable: {stage}")]
pub struct HermesNativeError {
    pub stage: &'static str,
}
pub(super) fn hermes_error(stage: &'static str) -> HermesNativeError {
    HermesNativeError { stage }
}

pub(super) struct HermesDocument {
    pub value: Value,
    bytes: Zeroizing<Vec<u8>>,
}
impl Drop for HermesDocument {
    fn drop(&mut self) {
        super::pi_sources::clear_native_value(&mut self.value);
    }
}

impl HermesDocument {
    pub fn parse(bytes: &[u8]) -> Result<Self, HermesNativeError> {
        if bytes.len() > 2 * 1024 * 1024 || bytes.contains(&0) {
            return Err(hermes_error("document bound"));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| hermes_error("encoding"))?;
        // Aliases can silently rewrite an unowned member when a referenced owned mapping is
        // edited. Refuse them even though a YAML decoder could resolve them successfully.
        let mut literal_indent = None;
        for line in text.lines() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if literal_indent.is_some_and(|level| indent > level || trimmed.is_empty()) {
                continue;
            }
            literal_indent = None;
            if mapping_colon(line)
                .is_some_and(|colon| line[colon + 1..].trim_start().starts_with(['|', '>']))
            {
                literal_indent = Some(indent);
            }

            if trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with("---") || trimmed.starts_with("...") {
                return Err(hermes_error("single document required"));
            }
            let mut quote = None;
            let mut previous = b' ';
            for byte in line.bytes() {
                match (quote, byte) {
                    (None, b'#') => break,
                    (None, b'\'' | b'"') => quote = Some(byte),
                    (Some(q), b) if q == b && previous != b'\\' => quote = None,
                    (None, b'&' | b'*' | b'!')
                        if previous.is_ascii_whitespace()
                            || matches!(previous, b':' | b'[' | b',' | b'{') =>
                    {
                        return Err(hermes_error("static mapping required"));
                    }
                    _ => {}
                }
                previous = byte;
            }
        }
        let yaml: serde_yaml::Value = if text.trim().is_empty() {
            serde_yaml::Value::Mapping(Default::default())
        } else {
            serde_yaml::from_str(text).map_err(|_| hermes_error("unambiguous mapping required"))?
        };
        fn static_value(v: &serde_yaml::Value, depth: usize) -> bool {
            if depth > 32 {
                return false;
            }
            match v {
                serde_yaml::Value::Tagged(_) => false,
                serde_yaml::Value::Mapping(m) => m.iter().all(|(k, v)| {
                    k.as_str().is_some_and(|k| k != "<<") && static_value(v, depth + 1)
                }),
                serde_yaml::Value::Sequence(a) => a.iter().all(|v| static_value(v, depth + 1)),
                _ => true,
            }
        }
        if !static_value(&yaml, 0) {
            return Err(hermes_error("static mapping required"));
        }
        let value = serde_json::to_value(yaml).map_err(|_| hermes_error("mapping values"))?;
        if !value.is_object() {
            return Err(hermes_error("profile mapping required"));
        }
        Ok(Self {
            value,
            bytes: Zeroizing::new(bytes.to_vec()),
        })
    }

    /// A single top-level mapping is the edit unit; only the specified descendant members
    /// are changed. Scalar/flow mappings are readable but cannot provide safe member spans.
    pub fn edit(
        &self,
        changes: &BTreeMap<Vec<String>, Option<Value>>,
    ) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
        let mut bytes = self.bytes.clone();
        for (path, value) in changes {
            if path.is_empty()
                || path
                    .iter()
                    .any(|k| k.is_empty() || k.chars().any(char::is_control))
            {
                return Err(hermes_error("owned path"));
            }
            let current = Self::parse(&bytes)?;
            bytes = current.edit_one(path, value.as_ref())?;
        }
        // Every rendered edit must preserve the requested semantic result.
        let next = Self::parse(&bytes)?;
        for (path, value) in changes {
            if at(&next.value, path) != value.as_ref() {
                return Err(hermes_error("member edit mismatch"));
            }
        }
        Ok(bytes)
    }

    fn edit_one(
        &self,
        path: &[String],
        value: Option<&Value>,
    ) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
        let text = std::str::from_utf8(&self.bytes).map_err(|_| hermes_error("encoding"))?;
        let spans = mapping_spans(text, 0, text.len(), 0)?;
        self.edit_in(text, &spans, path, value, 0, 0..text.len())
    }

    fn edit_in(
        &self,
        text: &str,
        spans: &BTreeMap<String, MemberSpan>,
        path: &[String],
        value: Option<&Value>,
        indent: usize,
        parent: Range<usize>,
    ) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
        let key = &path[0];
        if path.len() == 1 {
            let mut result = Zeroizing::new(self.bytes.to_vec());
            let replacement = match value {
                Some(v) => render_member(key, v, indent)?,
                None => String::new(),
            };
            if let Some(span) = spans.get(key) {
                result.splice(span.range.clone(), replacement.bytes());
            } else if value.is_some() {
                let mut insert = replacement;
                if parent.end > 0 && !text[..parent.end].ends_with('\n') {
                    insert.insert(0, '\n');
                }
                result.splice(parent.end..parent.end, insert.bytes());
            }
            return Ok(result);
        }
        if let Some(span) = spans.get(key) {
            if span.empty_map {
                if value.is_none() {
                    return Ok(self.bytes.clone());
                }
                let mut nested = value.cloned().expect("checked value");
                for name in path[1..].iter().rev() {
                    nested = Value::Object(Map::from_iter([(name.clone(), nested)]));
                }
                return self.edit_in(text, spans, &path[..1], Some(&nested), indent, parent);
            }
            if !span.block {
                return Err(hermes_error("block mapping required for edit"));
            }
            let body = span.body..span.range.end;
            let next_indent = text[body.clone()]
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .map(|l| l.len() - l.trim_start_matches(' ').len())
                .filter(|n| *n > indent)
                .min()
                .unwrap_or(indent + 2);
            let nested = mapping_spans(text, body.start, body.end, next_indent)?;
            return self.edit_in(text, &nested, &path[1..], value, next_indent, body);
        }
        if value.is_none() {
            return Ok(self.bytes.clone());
        }
        let mut nested = value.cloned().expect("checked value");
        for name in path[1..].iter().rev() {
            nested = Value::Object(Map::from_iter([(name.clone(), nested)]));
        }
        self.edit_in(text, spans, &path[..1], Some(&nested), indent, parent)
    }
}

pub(super) fn at<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, k| v.get(k))
}

struct MemberSpan {
    range: Range<usize>,
    body: usize,
    block: bool,
    empty_map: bool,
}
fn mapping_spans(
    text: &str,
    start: usize,
    end: usize,
    indent: usize,
) -> Result<BTreeMap<String, MemberSpan>, HermesNativeError> {
    let mut members: Vec<(String, usize, usize, bool, bool, usize)> = Vec::new();
    let mut offset = start;
    let mut scalar_indent = None;
    for line in text[start..end].split_inclusive('\n') {
        let trimmed = line.trim();
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if line.starts_with('\t')
            || (spaces < indent && !trimmed.is_empty() && !trimmed.starts_with('#'))
        {
            return Err(hermes_error("block indentation"));
        }
        if scalar_indent.is_some_and(|level| spaces > level || trimmed.is_empty()) {
            if let Some(member) = members.last_mut() {
                member.5 = offset + line.len();
            }
            offset += line.len();
            continue;
        }
        scalar_indent = None;
        if spaces == indent && !trimmed.is_empty() && !trimmed.starts_with('#') {
            let colon = mapping_colon(trimmed).ok_or_else(|| hermes_error("mapping member"))?;
            let (raw, rest) = (&trimmed[..colon], &trimmed[colon + 1..]);
            let key = if raw.starts_with('\'') || raw.starts_with('"') {
                serde_yaml::from_str::<String>(raw).map_err(|_| hermes_error("member key"))?
            } else {
                raw.to_owned()
            };
            if key.is_empty() {
                return Err(hermes_error("member key"));
            }
            let rhs = rest.trim();
            if rhs.starts_with('|') || rhs.starts_with('>') {
                scalar_indent = Some(spaces);
            }
            members.push((
                key,
                offset,
                offset + line.len(),
                rhs.is_empty() || rhs.starts_with('#'),
                rhs.split('#').next().is_some_and(|v| v.trim() == "{}"),
                offset + line.len(),
            ));
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && let Some(member) = members.last_mut()
        {
            member.5 = offset + line.len();
        }
        offset += line.len();
    }
    let mut result = BTreeMap::new();
    for (i, (key, begin, body, block, empty_map, content_end)) in members.iter().enumerate() {
        let stop = members.get(i + 1).map_or(end, |x| x.1).min(*content_end);
        if result
            .insert(
                key.clone(),
                MemberSpan {
                    range: *begin..stop,
                    body: *body,
                    block: *block,
                    empty_map: *empty_map,
                },
            )
            .is_some()
        {
            return Err(hermes_error("duplicate member"));
        }
    }
    Ok(result)
}

fn mapping_colon(line: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in line.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, byte) {
            (Some(b'"'), b'\\') => escaped = true,
            (Some(q), b) if q == b => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b':') => return Some(index),
            (None, b'#') => return None,
            _ => {}
        }
    }
    None
}

fn render_member(key: &str, value: &Value, indent: usize) -> Result<String, HermesNativeError> {
    // JSON scalar/flow values are valid YAML and avoid guessing quoting conventions. Mapping
    // descendants remain block mappings so the next scoped edit has observable member spans.
    let prefix = " ".repeat(indent);
    let key = serde_json::to_string(key).map_err(|_| hermes_error("render key"))?;
    if let Some(map) = value.as_object().filter(|m| !m.is_empty()) {
        let mut out = format!("{prefix}{key}:\n");
        for (name, value) in map {
            out.push_str(&render_member(name, value, indent + 2)?);
        }
        Ok(out)
    } else {
        Ok(format!(
            "{prefix}{key}: {}\n",
            serde_json::to_string(value).map_err(|_| hermes_error("render value"))?
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unowned_literal_markers_quoted_keys_and_following_comments_survive_member_removal() {
        let before = b"model:\n  api_key: native-secret\n# belongs to the following member\n\"unrelated:key\": |\n  *literal !literal &literal\n  # literal content\n";
        let doc = HermesDocument::parse(before).unwrap();
        let after = doc
            .edit(&BTreeMap::from([(
                vec!["model".into(), "api_key".into()],
                None,
            )]))
            .unwrap();
        assert!(std::str::from_utf8(&after).unwrap().contains("# belongs to the following member\n\"unrelated:key\": |\n  *literal !literal &literal\n  # literal content\n"));
        assert!(
            HermesDocument::parse(&after).unwrap().value["model"]
                .get("api_key")
                .is_none()
        );
    }
    #[test]
    fn bounded_members_preserve_foreign_yaml_and_reject_ambiguous_composition() {
        let bytes=b"# profile\nmodel:\n  default: old\n  provider: native\n  unrelated: keep\nauxiliary:\n  title: native # keep exact bytes\n";
        let doc = HermesDocument::parse(bytes).unwrap();
        let next = doc
            .edit(&BTreeMap::from([
                (
                    vec!["model".into(), "default".into()],
                    Some(json!("plan alias")),
                ),
                (
                    vec!["providers".into(), "hiroute-main".into()],
                    Some(json!({"api":"http://localhost/v1"})),
                ),
            ]))
            .unwrap();
        assert!(
            std::str::from_utf8(&next)
                .unwrap()
                .contains("auxiliary:\n  title: native # keep exact bytes\n")
        );
        assert_eq!(
            HermesDocument::parse(&next).unwrap().value["model"]["unrelated"],
            "keep"
        );
        assert!(HermesDocument::parse(b"model: &owned\n  default: old\nother: *owned\n").is_err());
        assert!(
            HermesDocument::parse(b"model:\n  default: a\n  default: b\n")
                .and_then(|d| d.edit(&BTreeMap::from([(
                    vec!["model".into(), "default".into()],
                    Some(json!("new"))
                )])))
                .is_err()
        );
    }
}
