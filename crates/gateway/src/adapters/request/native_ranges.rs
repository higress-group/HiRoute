//! Substitute current native values through the existing RawJson encoder.
//! Every selected span independently keeps its original compact context length.
use super::template::{
    PreparedNativeTemplate, ReplacementEncoding, TemplateReplacement, replacement_wire_len,
};
use crate::replay::{
    ReplayStore,
    native_json::{RawJsonIndex, array_ranges, root_ranges},
};

impl PreparedNativeTemplate {
    pub(crate) fn use_native_ranges(&mut self, replay: &ReplayStore, input: &RawJsonIndex) -> bool {
        if self.preserved_native_roots.is_empty() {
            return false;
        }
        let Some(roots) = root_ranges(&self.bytes) else {
            return false;
        };
        let mut candidates = Vec::new();
        for key in self.preserved_native_roots.iter() {
            let Some(root) = roots.get(key) else {
                continue;
            };
            if key == "messages" {
                let Some(messages) = array_ranges(&self.bytes[root.clone()]) else {
                    continue;
                };
                for (index, span) in messages.into_iter().enumerate() {
                    if let Some(content) = input.message(index) {
                        candidates.push((root.start + span.start..root.start + span.end, content));
                    }
                }
            } else if let Some(content) = input.get(key) {
                candidates.push((root.clone(), content));
            }
        }
        candidates.sort_by_key(|(span, _)| span.start);
        let mut spans = Vec::new();
        for (span, content) in candidates {
            let first = self
                .replacements
                .partition_point(|value| value.end <= span.start);
            let end = self
                .replacements
                .partition_point(|value| value.start < span.end);
            if first >= end {
                continue;
            }
            let covered = &self.replacements[first..end];
            if covered.iter().any(|value| {
                value.start < span.start
                    || value.end > span.end
                    || value.encoding == ReplacementEncoding::RawJsonInJsonString
            }) {
                continue;
            }
            let compact_len = replacement_wire_len(span.end, covered)
                .and_then(|length| length.checked_sub(span.start));
            if compact_len.and_then(|length| u64::try_from(length).ok()) != Some(content.byte_len())
            {
                continue;
            }
            spans.push((span, content, first, end));
        }
        if spans.is_empty() || spans.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
            return false;
        }
        let mut replacements = Vec::with_capacity(self.replacements.len());
        let mut cursor = 0;
        let mut references = Vec::with_capacity(spans.len());
        for (span, content, first, end) in spans {
            replacements.extend(self.replacements[cursor..first].iter().cloned());
            replacements.push(TemplateReplacement {
                start: span.start,
                end: span.end,
                content: content.clone(),
                encoding: ReplacementEncoding::RawJson,
            });
            references.push(content.clone());
            cursor = end;
        }
        replacements.extend(self.replacements[cursor..].iter().cloned());
        if replacements.len() > self.replacements.len()
            || replacement_wire_len(self.bytes.len(), &replacements) != Some(self.wire_len)
            || input.register(replay, &references).is_err()
        {
            return false;
        }
        self.replacements = replacements.into();
        true
    }
}
