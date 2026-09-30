#!/usr/bin/env python3
"""Validate deterministic model-metadata inference rules and provenance."""

from __future__ import annotations

import re
from datetime import datetime
from typing import Callable

import close_model_metadata


COMPLETENESS_STATES = {"complete", "missing", "not-applicable", "partial"}
CAPABILITY_STATES = {
    "supported",
    "unsupported",
    "unknown",
    "conditional",
    "not-applicable",
}
TOKEN_LIMIT_STATES = {
    "exact",
    "shared-total-budget",
    "entitlement-dependent",
    "runtime-required",
    "unknown",
    "conflict",
    "not-applicable",
}
METADATA_FACT_STATES = {
    "conflict",
    "entitlement-dependent",
    "known",
    "not-applicable",
    "runtime-required",
    "shared-total-budget",
    "unknown",
}
COST_HINT_STATES = {"not-recorded", "recorded"}
MODEL_CLOSABLE_FIELDS = (
    "capability_hints.reasoning",
    "capability_hints.streaming",
    "capability_hints.tool",
    "capability_hints.vision",
    "context_tokens",
    "max_output_tokens",
    "input_modalities",
    "lifecycle",
    "reasoning_rendering_hints",
    "execution_fit",
)
PROVIDER_FIELD_NAMES = (
    "aliases",
    "authentication_candidates",
    "base_url_candidates",
    "default_model_ids",
    "description_candidates",
    "discovery_modes",
    "environment_variable_names",
    "models_url_candidates",
    "protocol_candidates",
    "signup_url_candidates",
    "unsupported_api_styles",
    "unsupported_transport_locators",
)
EXECUTION_FIT_STATES = {
    "native-text-representable",
    "unsupported",
    "not-applicable",
}
REQUIRED_SEMANTICS_VOCABULARIES = {
    "capability_state": CAPABILITY_STATES,
    "cost_hint_state": COST_HINT_STATES,
    "execution_fit_state": EXECUTION_FIT_STATES,
    "metadata_completeness_state": COMPLETENESS_STATES,
    "metadata_fact_state": METADATA_FACT_STATES,
    "token_limit_state": TOKEN_LIMIT_STATES,
}
RULES_SCHEMA = "hiroute.model-metadata-inference-rules/v1"
PROVIDER_COMPLETENESS_DOMAINS = {
    "authentication",
    "discovery",
    "endpoint",
    "identity",
}
MODEL_COMPLETENESS_DOMAINS = {
    "capabilities",
    "cost",
    "identity",
    "lifecycle",
    "limits",
    "modalities",
    "reasoning_rendering",
}


def validate_rule_catalog(
    catalog: dict[str, object],
    rules_document: dict[str, object],
    sources: dict[object, object],
    catalog_digest: str,
    canonical_digest: Callable[[object], str],
    is_sorted_unique: Callable[[list[str]], bool],
    errors: list[str],
) -> dict[object, dict[str, object]]:
    """Validate the authored rule table, embedded rules, and closure fixed point."""
    catalog_rule_records = catalog.get("inference_rules", [])
    if not isinstance(catalog_rule_records, list):
        errors.append("catalog inference rules must be an array")
        catalog_rule_records = []
    catalog_rule_keys = [
        record.get("rule_key") if isinstance(record, dict) else None
        for record in catalog_rule_records
    ]
    if not all(
        isinstance(value, str) and value for value in catalog_rule_keys
    ) or not is_sorted_unique(catalog_rule_keys):
        errors.append("catalog inference rules must be sorted and unique by rule_key")
    catalog_rules = {
        record.get("rule_key"): record
        for record in catalog_rule_records
        if isinstance(record, dict)
    }

    if rules_document.get("schema") != RULES_SCHEMA:
        errors.append("unexpected inference rules schema")
    if not isinstance(rules_document.get("policy"), str) or not rules_document["policy"]:
        errors.append("inference rules need a policy statement")
    try:
        datetime.strptime(rules_document.get("as_of", ""), "%Y-%m-%d")
    except (TypeError, ValueError):
        errors.append("inference rules need a valid as_of date")

    def check_rule_table(kind: str, records: object) -> None:
        if not isinstance(records, list) or not records:
            errors.append(f"inference rules need {kind} rules")
            return
        for rule in records:
            rule_key = rule.get("rule_key") if isinstance(rule, dict) else None
            required_fields = {
                "assignments",
                "evidence_refs",
                "match",
                "reason",
                "record_kind",
                "rule_key",
            }
            if (
                not isinstance(rule, dict)
                or not required_fields <= set(rule)
                or set(rule) - required_fields - {"collected_on"}
            ):
                errors.append(f"malformed {kind} inference rule {rule_key}")
                continue
            collected_on = rule.get("collected_on", rules_document.get("as_of"))
            try:
                datetime.strptime(collected_on, "%Y-%m-%d")
            except (TypeError, ValueError):
                errors.append(f"inference rule {rule_key} has invalid collected_on")
            if rule.get("record_kind") != kind:
                errors.append(f"inference rule {rule_key} has a mismatched record kind")
            match = rule.get("match", {})
            if (
                not isinstance(match, dict)
                or not match
                or not set(match)
                <= {"upstream_model_id_pattern", "provider_id_pattern"}
            ):
                errors.append(f"inference rule {rule_key} has an invalid match")
            else:
                for clause, pattern in match.items():
                    if not isinstance(pattern, str) or not pattern:
                        errors.append(f"inference rule {rule_key} has an invalid {clause}")
                        continue
                    try:
                        re.compile(pattern)
                    except re.error:
                        errors.append(
                            f"inference rule {rule_key} has an uncompilable {clause}"
                        )
            assignments = rule.get("assignments", {})
            if not isinstance(assignments, dict) or not assignments:
                errors.append(f"inference rule {rule_key} has no assignments")
            elif kind == "model" and not set(assignments) <= set(MODEL_CLOSABLE_FIELDS):
                errors.append(
                    f"inference rule {rule_key} assigns outside closable model fields"
                )
            elif kind == "provider":
                if not set(assignments) <= (
                    {"domain", "completeness"} | set(PROVIDER_FIELD_NAMES)
                ):
                    errors.append(
                        f"inference rule {rule_key} assigns outside provider domains"
                    )
                if assignments.get("domain") not in PROVIDER_COMPLETENESS_DOMAINS:
                    errors.append(
                        f"inference rule {rule_key} has an invalid provider domain"
                    )
                if (
                    "completeness" in assignments
                    and assignments["completeness"] not in COMPLETENESS_STATES
                ):
                    errors.append(
                        f"inference rule {rule_key} has an invalid completeness state"
                    )
            if not isinstance(rule.get("reason"), str) or not rule["reason"]:
                errors.append(f"inference rule {rule_key} needs a reason")
            refs = rule.get("evidence_refs", [])
            if not isinstance(refs, list) or not refs or refs != sorted(set(refs)):
                errors.append(
                    f"inference rule {rule_key} evidence refs must be sorted and unique"
                )
            else:
                missing = set(refs) - set(sources)
                if missing:
                    errors.append(
                        f"inference rule {rule_key} cites missing sources: {sorted(missing)}"
                    )
            embedded = catalog_rules.get(rule_key)
            if embedded is None:
                errors.append(f"inference rule {rule_key} is missing from the catalog")
                continue
            if embedded != {
                "rule_key": rule_key,
                "record_kind": kind,
                "match": rule["match"],
                "assignments": rule["assignments"],
                "basis": "inferred",
                "reason": rule["reason"],
                "evidence_refs": sorted(set(rule["evidence_refs"])),
                "collected_on": collected_on,
            }:
                errors.append(
                    f"catalog inference rule {rule_key} does not match the rule table"
                )

    check_rule_table("model", rules_document.get("model_rules"))
    check_rule_table("provider", rules_document.get("provider_rules"))
    declared_rule_keys = {
        rule.get("rule_key")
        for rule in [
            *(rules_document.get("model_rules") or []),
            *(rules_document.get("provider_rules") or []),
        ]
        if isinstance(rule, dict)
    }
    undeclared = sorted(set(catalog_rules) - declared_rule_keys)
    if undeclared:
        errors.append(
            f"catalog inference rules are not declared by the rule table: {undeclared}"
        )

    try:
        closed_catalog = close_model_metadata.close(catalog, rules_document)
    except close_model_metadata.ClosureError as exc:
        errors.append("metadata closure failed: " + "; ".join(str(exc).splitlines()))
        closed_catalog = None
    if closed_catalog is not None and canonical_digest(closed_catalog) != catalog_digest:
        errors.append("catalog is not a fixed point of the deterministic metadata closure")
    return catalog_rules


def check_closure_provenance(
    label: str,
    record: dict[str, object],
    *,
    allowed_fields: tuple[str, ...],
    record_kind: str,
    normalized_id: str,
    domains: bool,
    catalog_rules: dict[object, dict[str, object]],
    errors: list[str],
) -> dict[str, object]:
    """Check that inferred field values remain traceable to matching rules."""
    provenance = record.get("field_provenance")
    if provenance is None:
        return {}
    if not isinstance(provenance, dict) or not provenance:
        errors.append(f"{label} field provenance must be a non-empty object")
        return {}
    if list(provenance) != sorted(provenance):
        errors.append(f"{label} field provenance must be sorted by field")
    for field, entry in provenance.items():
        if field not in allowed_fields:
            errors.append(f"{label} has field provenance for a non-closable field {field}")
            continue
        if (
            not isinstance(entry, dict)
            or set(entry) != {"basis", "rule_key"}
            or entry.get("basis") != "inferred"
        ):
            errors.append(f"{label} has malformed field provenance for {field}")
            continue
        rule = catalog_rules.get(entry.get("rule_key"))
        if rule is None:
            errors.append(
                f"{label} provenance references unknown rule {entry.get('rule_key')}"
            )
            continue
        if rule.get("record_kind") != record_kind:
            errors.append(f"{label} provenance references a {rule.get('record_kind')} rule")
            continue
        if not close_model_metadata.rule_matches(rule, record, normalized_id):
            errors.append(f"{label} provenance rule {rule['rule_key']} does not match the record")
            continue
        assignments = rule.get("assignments", {})
        if domains:
            if assignments.get("domain") != field:
                errors.append(f"{label} provenance rule {rule['rule_key']} assigns another domain")
                continue
            declared = assignments.get("completeness", "complete")
            if record.get("metadata_completeness", {}).get(field) != declared:
                errors.append(
                    f"{label} {field} completeness does not match rule {rule['rule_key']}"
                )
            for key, value in assignments.items():
                if key in ("domain", "completeness"):
                    continue
                if record.get(key) != close_model_metadata.canonical_assignment(value):
                    errors.append(
                        f"{label} rule {rule['rule_key']} assigns a different {key}"
                    )
            continue
        if field not in assignments:
            errors.append(f"{label} provenance rule {rule['rule_key']} does not assign {field}")
            continue
        expected = assignments[field]
        if field == "reasoning_rendering_hints" and expected == {
            "state": "not-applicable"
        }:
            rendering = record.get("reasoning_rendering_hints", {})
            if record.get("reasoning_rendering_state") != "not-applicable" or any(
                isinstance(rendering, dict) and rendering.get(key)
                for key in (
                    "reasoning_effort_maps",
                    "supported_reasoning_efforts",
                    "thinking_level_maps",
                )
            ):
                errors.append(
                    f"{label} reasoning rendering does not match rule {rule['rule_key']}"
                )
        elif close_model_metadata.get_field(
            record, field
        ) != close_model_metadata.canonical_assignment(expected):
            errors.append(
                f"{label} {field} does not match rule {rule['rule_key']} assignment"
            )
    return provenance
