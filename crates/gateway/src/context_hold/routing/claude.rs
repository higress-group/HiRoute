//! Claude's native headers describe activation and scope, never human provenance.
use super::{Field, NativeCheckpoint, NativeUserOrigin, RoutingFacts, digest, header_value};
use http::HeaderMap;

pub(super) fn facts(headers: &HeaderMap, usable_identity: bool) -> RoutingFacts {
    let purpose = header_value(headers, "x-claude-code-request-class");
    // The native call class is independent of optional session/prompt enrichment.
    // A known compaction must not enter ordinary history when that enrichment fails.
    if matches!(&purpose, Field::Value(value) if matches!(value.as_str(), "auxiliary" | "compaction"))
    {
        return RoutingFacts::detached();
    }
    if !usable_identity {
        return RoutingFacts::uncertain();
    }
    if matches!(&purpose, Field::Value(value) if value == "subagent")
        && !matches!(
            header_value(headers, "x-claude-code-agent-id"),
            Field::Value(_)
        )
    {
        return RoutingFacts::uncertain();
    }
    let turn = match header_value(headers, "x-claude-code-prompt-id") {
        Field::Value(value) => Some(digest(&value)),
        Field::Absent => None,
        Field::Invalid => return RoutingFacts::uncertain(),
    };
    let compacted = match header_value(headers, "x-claude-code-context-compacted") {
        Field::Absent => false,
        Field::Value(value) if matches!(value.as_str(), "manual" | "auto") => true,
        _ => return RoutingFacts::uncertain(),
    };
    RoutingFacts {
        native: NativeCheckpoint {
            turn,
            compacted,
            user_origin: NativeUserOrigin::Observed,
            ..Default::default()
        },
        ..RoutingFacts::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(fields: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in fields {
            headers.append(*name, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn known_auxiliary_class_survives_unavailable_identity_and_invalid_enrichment() {
        for purpose in ["auxiliary", "compaction"] {
            for usable_identity in [false, true] {
                let facts = facts(
                    &headers(&[
                        ("x-claude-code-request-class", purpose),
                        ("x-claude-code-prompt-id", ""),
                        ("x-claude-code-agent-id", ""),
                        ("x-claude-code-context-compacted", "unsupported"),
                    ]),
                    usable_identity,
                );
                assert!(!facts.track);
                assert_eq!(facts.native.user_origin, NativeUserOrigin::ExistingTurn);
                assert!(facts.native.turn.is_none());
            }
        }
    }

    #[test]
    fn unknown_missing_and_duplicate_classes_cannot_claim_auxiliary_exclusion() {
        for fields in [
            vec![],
            vec![("x-claude-code-request-class", "workflow")],
            vec![("x-claude-code-request-class", "future-kind")],
            vec![("x-claude-code-request-class", "")],
            vec![
                ("x-claude-code-request-class", "auxiliary"),
                ("x-claude-code-request-class", "compaction"),
            ],
        ] {
            for usable_identity in [false, true] {
                let facts = facts(&headers(&fields), usable_identity);
                assert!(facts.track);
                assert_eq!(facts.native.user_origin, NativeUserOrigin::Observed);
                assert!(!facts.identity_conflict);
            }
        }
    }

    #[test]
    fn prompt_is_only_a_routing_activation_even_after_native_compression() {
        for purpose in ["main", "workflow", "future-kind"] {
            for compacted in [None, Some("manual"), Some("auto")] {
                let mut fields = vec![
                    ("x-claude-code-request-class", purpose),
                    ("x-claude-code-prompt-id", "native-prompt"),
                ];
                if let Some(compacted) = compacted {
                    fields.push(("x-claude-code-context-compacted", compacted));
                }
                let facts = facts(&headers(&fields), true);
                assert!(facts.track);
                assert_eq!(facts.native.turn, Some(digest("native-prompt")));
                assert_eq!(facts.native.compacted, compacted.is_some());
                assert_eq!(facts.native.user_origin, NativeUserOrigin::Observed);
                assert!(!facts.native.requires_checkpoint);
                assert!(facts.native.checkpoint.is_none());
            }
        }
    }

    #[test]
    fn subagent_identity_is_never_invented_and_optional_fault_recovers() {
        let mut fields = vec![
            ("x-claude-code-request-class", "subagent"),
            ("x-claude-code-prompt-id", "same-prompt"),
        ];
        assert!(facts(&headers(&fields), true).native.turn.is_none());
        fields.push(("x-claude-code-agent-id", "actual-child"));
        let coherent = facts(&headers(&fields), true);
        assert_eq!(coherent.native.turn, Some(digest("same-prompt")));
        fields.push(("x-claude-code-context-compacted", "invalid"));
        assert!(facts(&headers(&fields), true).track);
        fields.pop();
        assert_eq!(facts(&headers(&fields), true), coherent);
    }
}
