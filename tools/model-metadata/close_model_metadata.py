#!/usr/bin/env python3
"""Deterministic determinate-outcome closure for provider-scoped metadata records.

The closure consumes the maintained catalog plus the reviewed inference-rule table and
replaces every indeterminate fact field with a determinate, machine-readable outcome:

- searched or client-observed facts stay as they are and are implicitly `observed`;
- a conservative, family/product-anchored inference is written together with an explicit
  `field_provenance` entry naming the rule, so it can never be read as provider-verified;
- explicitly unsupported and not-applicable outcomes are written as distinct values, never
  as `unknown`.

The pass is idempotent: re-running it on a closed catalog changes nothing, because every
already-closed field keeps its recorded basis and every determinate field is left alone.
"""

from __future__ import annotations

import argparse
import copy
import json
import math
import re
import sys
from pathlib import Path
from typing import Any

CATALOG_RELATIVE_PATH = Path("assets/model-data/current/inputs/metadata-catalog.json")
RULES_RELATIVE_PATH = Path(
    "tools/model-metadata/inference-rules.json"
)
CLOSABLE_MODEL_FIELDS = (
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
# cost_hint_state is derived from the recorded cost hints; it is never rule-assigned.
PROVIDER_DOMAINS = ("authentication", "discovery", "endpoint", "identity")


class ClosureError(RuntimeError):
    pass


def find_repository_root() -> Path:
    for parent in Path(__file__).resolve().parents:
        if (parent / CATALOG_RELATIVE_PATH).is_file():
            return parent
    raise ClosureError("cannot locate repository root containing the model metadata catalog")


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ClosureError(f"cannot read valid JSON from {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise ClosureError(f"expected a JSON object in {path}")
    return value


def provider_id_from_record_key(record_key: str) -> str:
    return record_key.rsplit("/", 1)[-1]


def normalize_upstream_id(upstream_model_id: str) -> str:
    """Reduce provider-scoped spellings to one comparable family token.

    Only lexical normalization is applied: vendor `org/` prefixes, digest-style `:tag`
    suffixes, `-free` routing markers, and the `5p2`-style compact patch spelling. The
    original ID always stays untouched on the record.
    """
    value = upstream_model_id.strip().lower()
    value = value.rsplit("/", 1)[-1]
    if ":" in value:
        value = value.split(":", 1)[0]
    value = re.sub(r"^us\.(anthropic|amazon|meta)\.", "", value)
    value = re.sub(r"^[a-z0-9_]+\.(?=(gpt|v\d|o\d|claude|gemini))", "", value)
    value = re.sub(r"-free$", "", value)
    value = re.sub(r"(\d)p(\d)", r"\1.\2", value)
    return value


def capability_state(value: Any) -> bool:
    return value in ("supported", "unsupported", "conditional", "not-applicable")


def token_fact(value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    state = value.get("state")
    if state == "known":
        return isinstance(value.get("value"), int) and not isinstance(value.get("value"), bool) and value["value"] > 0
    if state == "conflict":
        return isinstance(value.get("candidates"), list) and len(value["candidates"]) >= 2
    return state in ("unknown", "runtime-required", "shared-total-budget", "entitlement-dependent", "not-applicable")


def get_field(record: dict[str, Any], field_path: str) -> Any:
    if "." in field_path:
        container, key = field_path.split(".", 1)
        return record.get(container, {}).get(key)
    return record.get(field_path)


def canonical_assignment(value: Any) -> Any:
    """Normalize a rule assignment the same way observed client fields are normalized."""
    if isinstance(value, list):
        if all(isinstance(item, str) for item in value):
            return sorted(set(value))
        if all(isinstance(item, dict) for item in value):
            by_encoding = {
                json.dumps(item, ensure_ascii=False, sort_keys=True, separators=(",", ":")): item
                for item in value
            }
            return [by_encoding[key] for key in sorted(by_encoding)]
        return copy.deepcopy(value)
    if isinstance(value, dict):
        return {key: canonical_assignment(item) for key, item in value.items()}
    return value


def has_usable_cost_hint(value: Any) -> bool:
    if isinstance(value, bool) or value is None:
        return False
    if isinstance(value, (int, float)):
        return math.isfinite(value) and value >= 0
    if isinstance(value, list):
        return any(has_usable_cost_hint(item) for item in value)
    if isinstance(value, dict):
        return any(has_usable_cost_hint(item) for item in value.values())
    return False


def set_field(record: dict[str, Any], field_path: str, value: Any) -> None:
    if "." in field_path:
        container, key = field_path.split(".", 1)
        record.setdefault(container, {})[key] = canonical_assignment(value)
    else:
        record[field_path] = canonical_assignment(value)


def reset_model_field(record: dict[str, Any], field_path: str) -> None:
    """Return a rule-closed field to its indeterminate form before re-closing it."""
    if field_path == "input_modalities":
        record[field_path] = []
    elif field_path in ("context_tokens", "max_output_tokens"):
        record[field_path] = {"state": "unknown", "value": None}
    elif field_path == "lifecycle":
        record[field_path] = "unknown"
    elif field_path == "reasoning_rendering_hints":
        record[field_path] = {
            "reasoning_effort_maps": [],
            "supported_reasoning_efforts": [],
            "thinking_level_maps": [],
        }
        record.pop("reasoning_rendering_state", None)
    elif field_path == "execution_fit":
        record.pop("execution_fit", None)
    elif "." in field_path:
        container, key = field_path.split(".", 1)
        record.setdefault(container, {})[key] = "unknown"
    else:
        record[field_path] = "unknown"


def rule_still_applies(
    entry: Any,
    rules_by_key: dict[str, dict[str, Any]],
    *,
    record_kind: str,
    record: dict[str, Any],
    normalized_id: str,
    domain: str | None = None,
) -> bool:
    if not isinstance(entry, dict) or entry.get("basis") != "inferred":
        return False
    rule = rules_by_key.get(entry.get("rule_key"))
    if rule is None or rule.get("record_kind", "model") != record_kind:
        return False
    if not rule_matches(rule, record, normalized_id):
        return False
    if domain is not None:
        return rule.get("assignments", {}).get("domain") == domain
    return True


def assignment_matches(
    record: dict[str, Any], field_path: str, assignments: dict[str, Any]
) -> bool:
    """Check that a stored value is exactly what the recorded rule assigns."""
    if field_path not in assignments:
        return False
    expected = assignments[field_path]
    if field_path == "reasoning_rendering_hints" and expected == {"state": "not-applicable"}:
        rendering = record.get("reasoning_rendering_hints", {})
        return record.get("reasoning_rendering_state") == "not-applicable" and not any(
            isinstance(rendering, dict) and rendering.get(key)
            for key in (
                "reasoning_effort_maps",
                "supported_reasoning_efforts",
                "thinking_level_maps",
            )
        )
    return get_field(record, field_path) == canonical_assignment(expected)


def domain_matches(
    record: dict[str, Any], domain: str, assignments: dict[str, Any]
) -> bool:
    if assignments.get("domain") != domain:
        return False
    if record.get("metadata_completeness", {}).get(domain) != assignments.get(
        "completeness", "complete"
    ):
        return False
    return all(
        record.get(key) == canonical_assignment(value)
        for key, value in assignments.items()
        if key not in ("domain", "completeness")
    )


def field_is_indeterminate(record: dict[str, Any], field_path: str) -> bool:
    value = get_field(record, field_path)
    if field_path == "input_modalities":
        return not value
    if field_path in ("context_tokens", "max_output_tokens"):
        if not isinstance(value, dict) or value.get("state") == "unknown":
            return True
        if value.get("state") == "conflict" and not value.get("candidates"):
            return True
        return False
    if field_path == "lifecycle":
        return value in (None, "unknown")
    if field_path == "reasoning_rendering_hints":
        if not isinstance(value, dict):
            return True
        if value.get("state") == "not-applicable":
            return False
        return not any(value.get(key) for key in (
            "reasoning_effort_maps", "supported_reasoning_efforts", "thinking_level_maps",
        ))
    if field_path == "execution_fit":
        return not isinstance(value, dict) or not value.get("state")
    return not capability_state(value)


def assignment_applies(record: dict[str, Any], field_path: str) -> bool:
    """Guard rules that must not silently contradict an observed fact."""
    if field_path == "reasoning_rendering_hints":
        return True
    if field_path == "execution_fit":
        return True
    if field_path.startswith("capability_hints."):
        return True
    return True


def rule_matches(rule: dict[str, Any], record: dict[str, Any], normalized_id: str) -> bool:
    match = rule.get("match", {})
    pattern = match.get("upstream_model_id_pattern")
    if pattern is not None and not re.search(pattern, normalized_id):
        return False
    provider_pattern = match.get("provider_id_pattern")
    if provider_pattern is not None and not re.search(provider_pattern, record.get("provider_id", "")):
        return False
    return True


def close_model_record(
    record: dict[str, Any],
    rules: list[dict[str, Any]],
    rules_by_key: dict[str, dict[str, Any]],
    errors: list[str],
) -> dict[str, Any]:
    provenance = record.get("field_provenance", {})
    if not isinstance(provenance, dict):
        provenance = {}
    normalized_id = normalize_upstream_id(record.get("upstream_model_id", ""))
    for field_path in CLOSABLE_MODEL_FIELDS:
        value = get_field(record, field_path)
        if value is not None:
            set_field(record, field_path, value)
    for field_path in list(provenance):
        entry = provenance[field_path]
        if not isinstance(entry, dict) or entry.get("basis") != "inferred" or not entry.get("rule_key"):
            errors.append(
                f"{record.get('model_record_key')}: malformed field provenance for {field_path}"
            )
            continue
        if field_path not in CLOSABLE_MODEL_FIELDS:
            errors.append(
                f"{record.get('model_record_key')}: field provenance for a non-closable field {field_path}"
            )
            continue
        if rule_still_applies(
            entry,
            rules_by_key,
            record_kind="model",
            record=record,
            normalized_id=normalized_id,
        ) and assignment_matches(
            record,
            field_path,
            rules_by_key[entry["rule_key"]]["assignments"],
        ):
            continue
        # The recorded rule no longer applies, or the stored value drifted from the
        # rule assignment, so the inferred value is discarded and the field is
        # re-closed from the current rule table.
        del provenance[field_path]
        reset_model_field(record, field_path)
    for field_path in CLOSABLE_MODEL_FIELDS:
        if field_path in provenance:
            entry = provenance[field_path]
            if not isinstance(entry, dict) or entry.get("basis") != "inferred" or not entry.get("rule_key"):
                errors.append(
                    f"{record.get('model_record_key')}: malformed field provenance for {field_path}"
                )
            continue
        if not field_is_indeterminate(record, field_path):
            continue
        applied = None
        for rule in rules:
            if field_path not in rule.get("assignments", {}):
                continue
            if not rule_matches(rule, record, normalized_id):
                continue
            if not assignment_applies(record, field_path):
                continue
            applied = rule
            break
        if applied is None:
            errors.append(
                f"{record.get('model_record_key')}: no inference rule closes {field_path} "
                f"(normalized id {normalized_id})"
            )
            continue
        set_field(record, field_path, applied["assignments"][field_path])
        provenance[field_path] = {"basis": "inferred", "rule_key": applied["rule_key"]}
    if provenance:
        record["field_provenance"] = dict(sorted(provenance.items()))
    elif "field_provenance" in record:
        del record["field_provenance"]
    if isinstance(record.get("reasoning_rendering_hints"), dict) and "state" in record["reasoning_rendering_hints"]:
        state = record["reasoning_rendering_hints"]["state"]
        if state == "not-applicable":
            record["reasoning_rendering_state"] = "not-applicable"
        record["reasoning_rendering_hints"] = {
            "reasoning_effort_maps": [],
            "supported_reasoning_efforts": [],
            "thinking_level_maps": [],
        }
    # This is derived state, not a write-once default. Recompute it on every closure pass so a
    # later source refresh that adds or removes cost hints cannot leave stale metadata behind.
    record["cost_hint_state"] = (
        "recorded" if has_usable_cost_hint(record.get("cost_hints", [])) else "not-recorded"
    )
    return record


def close_provider_record(
    record: dict[str, Any],
    rules: list[dict[str, Any]],
    rules_by_key: dict[str, dict[str, Any]],
    errors: list[str],
) -> dict[str, Any]:
    provenance = record.get("field_provenance", {})
    if not isinstance(provenance, dict):
        provenance = {}
    provider_id = record.get("provider_id", "")
    for domain in list(provenance):
        entry = provenance[domain]
        if not isinstance(entry, dict) or entry.get("basis") != "inferred" or not entry.get("rule_key"):
            errors.append(
                f"{record.get('provider_record_key')}: malformed field provenance for {domain}"
            )
            continue
        if domain not in PROVIDER_DOMAINS:
            errors.append(
                f"{record.get('provider_record_key')}: field provenance for a non-closable domain {domain}"
            )
            continue
        if rule_still_applies(
            entry,
            rules_by_key,
            record_kind="provider",
            record=record,
            normalized_id="",
            domain=domain,
        ) and domain_matches(
            record, domain, rules_by_key[entry["rule_key"]]["assignments"]
        ):
            continue
        stale_rule = rules_by_key.get(entry.get("rule_key"))
        if stale_rule is not None:
            for key, value in stale_rule.get("assignments", {}).items():
                if key not in ("domain", "completeness") and record.get(key) == value:
                    record[key] = []
        record.setdefault("metadata_completeness", {})[domain] = "missing"
        del provenance[domain]
    for rule in rules:
        pattern = rule.get("match", {}).get("provider_id_pattern")
        if pattern is None or not re.search(pattern, provider_id):
            continue
        assignments = rule.get("assignments", {})
        domain = assignments.get("domain")
        if domain not in PROVIDER_DOMAINS:
            continue
        if domain in provenance:
            continue
        current = record.get("metadata_completeness", {}).get(domain)
        if current not in ("missing", "partial"):
            continue
        for key, value in assignments.items():
            if key in ("domain", "completeness"):
                continue
            record[key] = canonical_assignment(value)
        record.setdefault("metadata_completeness", {})[domain] = assignments.get(
            "completeness", "complete"
        )
        provenance[domain] = {"basis": "inferred", "rule_key": rule["rule_key"]}
    if provenance:
        record["field_provenance"] = dict(sorted(provenance.items()))
    elif "field_provenance" in record:
        del record["field_provenance"]
    record["usable_for"] = derive_provider_scenarios(record)
    return record


def derive_provider_scenarios(record: dict[str, Any]) -> list[str]:
    completeness = record.get("metadata_completeness", {})
    scenarios = ["provider-directory"]
    if completeness.get("endpoint") == "complete":
        scenarios.append("custom-api-endpoint-prefill")
    elif record.get("unsupported_api_styles") or record.get("unsupported_transport_locators"):
        scenarios.append("adapter-planning")
    if completeness.get("authentication") != "missing":
        scenarios.append("credential-setup-hint")
    if completeness.get("discovery") != "missing":
        scenarios.append("model-discovery-setup")
    return sorted(scenarios)


def close(catalog: dict[str, Any], rules_document: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(catalog)
    model_rules = rules_document["model_rules"]
    provider_rules = rules_document["provider_rules"]
    rules_by_key = {
        rule["rule_key"]: rule for rule in [*model_rules, *provider_rules]
    }
    errors: list[str] = []

    evidence_by_key = {
        source["source_key"]: source for source in result["evidence_sources"]
    }
    for addition in rules_document.get("evidence_source_additions", []):
        existing = evidence_by_key.get(addition["source_key"])
        if existing is not None:
            if existing != addition:
                errors.append(f"evidence source {addition['source_key']} conflicts with the catalog")
            continue
        result["evidence_sources"].append(copy.deepcopy(addition))
        evidence_by_key[addition["source_key"]] = addition

    provider_records = {
        record["provider_record_key"]: record for record in result["provider_metadata_records"]
    }
    for addition in rules_document.get("record_additions", []):
        provider_key = addition["provider_record_key"]
        if provider_key not in provider_records:
            errors.append(f"record addition references missing provider {provider_key}")
            continue
        if any(
            record["model_record_key"] == addition["model_record_key"]
            for record in result["model_metadata_records"]
        ):
            continue
        record = {
            "model_record_key": addition["model_record_key"],
            "provider_record_key": provider_key,
            "provenance_refs": copy.deepcopy(addition["provenance_refs"]),
            "provider_id": provider_records[provider_key]["provider_id"],
            "upstream_model_id": addition["upstream_model_id"],
            "display_name": addition["display_name"],
            "context_tokens": copy.deepcopy(addition["context_tokens"]),
            "max_output_tokens": copy.deepcopy(addition["max_output_tokens"]),
            "input_modalities": copy.deepcopy(addition["input_modalities"]),
            "capability_hints": copy.deepcopy(addition["capability_hints"]),
            "reasoning_rendering_hints": copy.deepcopy(addition["reasoning_rendering_hints"]),
            "cost_hints": [],
            "lifecycle": addition["lifecycle"],
            "status_candidates": copy.deepcopy(addition.get("status_candidates", [])),
            "replacement_upstream_ids": [],
            "roles": copy.deepcopy(addition.get("roles", [])),
            "normalized_model_matches": copy.deepcopy(addition.get("normalized_model_matches", [])),
            "metadata_completeness": copy.deepcopy(addition["metadata_completeness"]),
            "usable_for": copy.deepcopy(addition["usable_for"]),
            "source_assertions": copy.deepcopy(addition.get("source_assertions", [])),
            "source_locations": copy.deepcopy(addition["source_locations"]),
            "execution_fit": copy.deepcopy(addition["execution_fit"]),
            "cost_hint_state": "not-recorded",
        }
        result["model_metadata_records"].append(record)

    for record in result["model_metadata_records"]:
        close_model_record(record, model_rules, rules_by_key, errors)
        record["metadata_completeness"] = derive_model_completeness(record)
        record["usable_for"] = derive_model_scenarios(record)
    for record in result["provider_metadata_records"]:
        close_provider_record(record, provider_rules, rules_by_key, errors)
    result["model_metadata_records"].sort(key=lambda value: value["model_record_key"])
    result["provider_metadata_records"].sort(key=lambda value: value["provider_record_key"])
    result["evidence_sources"].sort(key=lambda value: value["source_key"])
    result["inference_rules"] = sorted(
        (
            {
                "rule_key": rule["rule_key"],
                "record_kind": rule.get("record_kind", "model"),
                "match": rule["match"],
                "assignments": rule["assignments"],
                "basis": "inferred",
                "reason": rule["reason"],
                "evidence_refs": sorted(set(rule["evidence_refs"])),
                "collected_on": rule.get("collected_on", rules_document["as_of"]),
            }
            for rule in [*model_rules, *provider_rules]
        ),
        key=lambda value: value["rule_key"],
    )
    semantics = result.setdefault("semantics", {})
    for vocabulary, values in rules_document.get("semantics_updates", {}).items():
        if not values or values != sorted(set(values)):
            errors.append(f"semantics vocabulary {vocabulary} must be a sorted unique list")
            continue
        semantics[vocabulary] = list(values)
    completeness_states = rules_document.get("semantics_updates", {}).get(
        "metadata_completeness_state"
    )
    if completeness_states:
        result.setdefault("metadata_usage_policy", {})["completeness_states"] = list(
            completeness_states
        )
    encoding_rules = semantics.setdefault("encoding_rules", [])
    for rule in rules_document.get("encoding_rules_additions", []):
        if rule not in encoding_rules:
            encoding_rules.append(rule)
    if errors:
        raise ClosureError("\n".join(errors))
    return result


def derive_model_completeness(record: dict[str, Any]) -> dict[str, str]:
    capabilities = record["capability_hints"]
    capability_values = list(capabilities.values())
    known_capabilities = sum(value != "unknown" for value in capability_values)
    capabilities_state = "missing"
    if known_capabilities:
        capabilities_state = "complete" if (
            known_capabilities == len(capability_values)
            and "conditional" not in capability_values
        ) else "partial"
    facts = [record.get("context_tokens", {}), record.get("max_output_tokens", {})]
    present_limits = sum(fact.get("state") != "unknown" for fact in facts)
    limits = "missing"
    if present_limits:
        determinate_limits = all(
            fact.get("state") in ("known", "not-applicable") for fact in facts
        )
        limits = "complete" if present_limits == 2 and determinate_limits else "partial"
    rendering = record.get("reasoning_rendering_hints", {})
    has_rendering = any(
        isinstance(value, list) and value for value in rendering.values()
    )
    if has_rendering:
        reasoning_rendering = "complete"
    elif record.get("reasoning_rendering_state") == "not-applicable":
        reasoning_rendering = "not-applicable"
    elif capabilities.get("reasoning") in ("supported", "conditional"):
        reasoning_rendering = "partial"
    else:
        reasoning_rendering = "missing"
    has_cost = has_usable_cost_hint(record.get("cost_hints", []))
    return {
        "capabilities": capabilities_state,
        "cost": "partial" if has_cost else "missing",
        "identity": "complete",
        "lifecycle": "complete" if record.get("lifecycle") not in (None, "unknown") else "missing",
        "limits": limits,
        "modalities": "complete" if record.get("input_modalities") else "missing",
        "reasoning_rendering": reasoning_rendering,
    }


def derive_model_scenarios(record: dict[str, Any]) -> list[str]:
    completeness = record["metadata_completeness"]
    scenarios = ["custom-api-model-prefill", "model-directory"]
    context = record.get("context_tokens", {})
    output = record.get("max_output_tokens", {})
    capabilities = record.get("capability_hints", {})
    if context.get("state") == "known":
        scenarios.append("context-limit-prefill")
    if output.get("state") == "known":
        scenarios.append("output-limit-prefill")
    if completeness["limits"] == "complete":
        scenarios.append("request-limit-prefill")
    if capabilities.get("reasoning") in ("supported", "unsupported", "conditional"):
        scenarios.append("reasoning-capability-prefill")
    if capabilities.get("vision") in ("supported", "unsupported", "conditional"):
        scenarios.append("vision-capability-prefill")
    if completeness["reasoning_rendering"] == "complete":
        scenarios.append("reasoning-rendering-hint")
    if completeness["lifecycle"] != "missing":
        scenarios.append("lifecycle-warning")
    if completeness["cost"] != "missing":
        scenarios.append("cost-hint-display")
    return sorted(scenarios)


def main() -> int:
    root = find_repository_root()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=root / CATALOG_RELATIVE_PATH)
    parser.add_argument("--rules", type=Path, default=root / RULES_RELATIVE_PATH)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        catalog = load_json(args.catalog)
        rules = load_json(args.rules)
        closed = close(catalog, rules)
    except ClosureError as exc:
        print(json.dumps({"result": "red", "errors": str(exc).splitlines()}, ensure_ascii=False, indent=2))
        return 1
    missing_rules = sorted(
        {
            rule_source["rule_key"]
            for record in closed["model_metadata_records"]
            for rule_source in record.get("field_provenance", {}).values()
        }
        - {rule["rule_key"] for rule in closed["inference_rules"]}
    )
    if missing_rules:
        print(json.dumps({"result": "red", "errors": [f"unknown rule refs: {missing_rules}"]}, indent=2))
        return 1
    payload = json.dumps(closed, ensure_ascii=False, indent=2) + "\n"
    if args.check:
        current = args.catalog.read_text(encoding="utf-8")
        if current != payload:
            print(json.dumps({"result": "red", "errors": ["catalog is not a closure fixed point"]}, indent=2))
            return 1
        print(json.dumps({"result": "green", "check": True}))
        return 0
    args.catalog.write_text(payload, encoding="utf-8")
    print(json.dumps({
        "result": "green",
        "model_records": len(closed["model_metadata_records"]),
        "provider_records": len(closed["provider_metadata_records"]),
        "inference_rules": len(closed["inference_rules"]),
    }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
