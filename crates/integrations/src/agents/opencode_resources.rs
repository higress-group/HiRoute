//! Bounded native Markdown model references for the existing settings dependency check.
//! These borrowed resources are never rewritten or interpreted as an executable config.
use super::{AgentFilesystemScanError as Error, OpenCodeNativeContext};
use hiroute_domain::CanonicalDigest;
use std::path::PathBuf;

pub(super) struct ModelReferences {
    pub models: Vec<String>,
    pub documents: Vec<(PathBuf, CanonicalDigest)>,
}

pub(super) fn model_references(native: &OpenCodeNativeContext) -> Result<ModelReferences, Error> {
    let mut result = ModelReferences {
        models: Vec::new(),
        documents: Vec::new(),
    };
    let mut entries_left = 256usize;
    let mut bytes_left = 4 * 1024 * 1024usize;
    for root in native
        .resource_roots(None)
        .map_err(|_| Error::InvalidConfig)?
    {
        for (name, recursive) in [
            ("agent", true),
            ("agents", true),
            ("mode", false),
            ("modes", false),
        ] {
            let mut pending = vec![(root.join(name), 0)];
            while let Some((directory, depth)) = pending.pop() {
                if depth > 16 {
                    return Err(Error::ConfigTooLarge);
                }
                let metadata = match std::fs::symlink_metadata(&directory) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(Error::InvalidConfig);
                }
                #[cfg(windows)]
                let _directory =
                    hiroute_platform::windows::security::open_identity(&directory, true)?;
                for entry in std::fs::read_dir(&directory)? {
                    entries_left = entries_left.checked_sub(1).ok_or(Error::ConfigTooLarge)?;
                    let entry = entry?;
                    let path = entry.path();
                    let kind = entry.file_type()?;
                    if kind.is_symlink() {
                        return Err(Error::SymlinkConfig);
                    }
                    if kind.is_dir() {
                        if recursive {
                            pending.push((path, depth + 1));
                        }
                        continue;
                    }
                    if path
                        .extension()
                        .is_none_or(|extension| !extension.eq_ignore_ascii_case("md"))
                    {
                        continue;
                    }
                    let (bytes, _) = super::filesystem_config::read_system_config_bytes(&path)?
                        .ok_or(Error::SourceChanged)?;
                    bytes_left = bytes_left
                        .checked_sub(bytes.len())
                        .ok_or(Error::ConfigTooLarge)?;
                    if let Some(model) = model_reference(&bytes)? {
                        result.models.push(model);
                    }
                    result
                        .documents
                        .push((path, CanonicalDigest::of_bytes(&bytes)));
                }
            }
        }
    }
    result.documents.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn model_reference(bytes: &[u8]) -> Result<Option<String>, Error> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::InvalidConfig)?
        .trim_start_matches('\u{feff}');
    if !text.starts_with("---") {
        return Ok(None);
    }
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(Error::InvalidConfig);
    }
    let mut header = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        header.push_str(line);
    }
    if !closed {
        return Err(Error::InvalidConfig);
    }
    if header.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_yaml::Value =
        serde_yaml::from_str(&header).map_err(|_| Error::InvalidConfig)?;
    if value.is_null() {
        return Ok(None);
    }
    let value = value.as_mapping().ok_or(Error::InvalidConfig)?;
    // Native gray-matter also accepts YAML merges and a permissive fallback. Keep
    // ambiguous variants unavailable for removal, without implementing its parser.
    if value
        .keys()
        .any(|key| key.as_str().is_none_or(|key| key == "<<"))
    {
        return Err(Error::InvalidConfig);
    }
    value
        .get(serde_yaml::Value::String("model".into()))
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or(Error::InvalidConfig)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_frontmatter_models_are_static_or_explicitly_unknown() {
        assert_eq!(model_reference(b"plain prompt").unwrap(), None);
        for text in [
            "---\nmodel: owned/route\n---\nprompt",
            "\u{feff}---\r\nmodel: 'owned/route'\r\n---\r\nprompt",
        ] {
            assert_eq!(
                model_reference(text.as_bytes()).unwrap().as_deref(),
                Some("owned/route")
            );
        }
        for text in [
            "---\nmodel: [owned/route]\n---\n",
            "---\n<<: {model: owned/route}\n---\n",
            "---\nmodel: owned/route\n",
            "---yaml\nmodel: owned/route\n---\n",
        ] {
            assert!(model_reference(text.as_bytes()).is_err(), "{text}");
        }
    }
}
