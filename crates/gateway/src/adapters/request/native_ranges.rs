//! Substitute validated current-input roots through the existing RawJson body
//! encoder. The original compact length remains the context admission oracle.
use super::template::{
    PreparedNativeTemplate, ReplacementEncoding, TemplateReplacement, replacement_wire_len,
};
use crate::replay::{
    ReplayStore,
    native_json::{RawJsonIndex, root_ranges},
};

impl PreparedNativeTemplate {
    pub(crate) fn use_native_ranges(&mut self, replay: &ReplayStore, input: &RawJsonIndex) -> bool {
        if self.preserved_native_roots.is_empty() {
            return false;
        }
        let Some(roots) = root_ranges(&self.bytes) else {
            return false;
        };
        let spans: Vec<_> = self
            .preserved_native_roots
            .iter()
            .filter_map(|key| {
                let span = roots.get(key)?;
                let mut covered = self
                    .replacements
                    .iter()
                    .filter(|replacement| {
                        replacement.start >= span.start && replacement.end <= span.end
                    })
                    .peekable();
                if covered.peek().is_none()
                    || covered.any(|replacement| {
                        replacement.encoding == ReplacementEncoding::RawJsonInJsonString
                    })
                {
                    return None;
                }
                Some((span.clone(), input.get(key)?.clone()))
            })
            .collect();
        if spans.is_empty() {
            return false;
        }
        if self.replacements.iter().any(|replacement| {
            spans.iter().any(|(span, _)| {
                replacement.start < span.end
                    && replacement.end > span.start
                    && !(replacement.start >= span.start && replacement.end <= span.end)
            })
        }) {
            return false;
        }
        let retained = self
            .replacements
            .iter()
            .filter(|replacement| {
                !spans
                    .iter()
                    .any(|(span, _)| replacement.start >= span.start && replacement.end <= span.end)
            })
            .count();
        // The existing request-local template charge also bounds this result.
        if retained + spans.len() > self.replacements.len() {
            return false;
        }
        let mut replacements = Vec::with_capacity(self.replacements.len());
        replacements.extend(
            self.replacements
                .iter()
                .filter(|replacement| {
                    !spans.iter().any(|(span, _)| {
                        replacement.start >= span.start && replacement.end <= span.end
                    })
                })
                .cloned(),
        );
        replacements.extend(
            spans
                .into_iter()
                .map(|(span, content)| TemplateReplacement {
                    start: span.start,
                    end: span.end,
                    content,
                    encoding: ReplacementEncoding::RawJson,
                }),
        );
        replacements.sort_by_key(|replacement| replacement.start);
        if replacement_wire_len(self.bytes.len(), &replacements) != Some(self.wire_len)
            || input.register(replay).is_err()
        {
            return false;
        }
        self.replacements = replacements.into();
        true
    }
}
