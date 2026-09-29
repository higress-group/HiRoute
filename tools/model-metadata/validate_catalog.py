#!/usr/bin/env python3
"""Validate HiRoute's sole current model-metadata source and evidence registry."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import date, datetime, timedelta
from decimal import Decimal, InvalidOperation
from pathlib import Path
from urllib.parse import urlparse

sys.path.insert(0, str(Path(__file__).resolve().parent))
import close_model_metadata  # noqa: E402
import validate_inference_rules as inference_validation  # noqa: E402


CATALOG_RELATIVE_PATH = Path("assets/model-data/current/inputs/metadata-catalog.json")
REGISTRY_RELATIVE_PATH = Path(
    "tools/model-metadata/source-registry.json"
)
DISCOVERY_RUNS_RELATIVE_PATH = Path(
    "assets/model-data/current/inputs/client-discovery-runs.json"
)
CLIENT_PROFILES_RELATIVE_PATH = Path(
    "tools/model-metadata/client-source-profiles.json"
)
RULES_RELATIVE_PATH = Path(
    "tools/model-metadata/inference-rules.json"
)
COMPLETENESS_STATES = inference_validation.COMPLETENESS_STATES
CAPABILITY_STATES = inference_validation.CAPABILITY_STATES
TOKEN_LIMIT_STATES = inference_validation.TOKEN_LIMIT_STATES
METADATA_FACT_STATES = inference_validation.METADATA_FACT_STATES
COST_HINT_STATES = inference_validation.COST_HINT_STATES
MODEL_CLOSABLE_FIELDS = inference_validation.MODEL_CLOSABLE_FIELDS
PROVIDER_FIELD_NAMES = inference_validation.PROVIDER_FIELD_NAMES
EXECUTION_FIT_STATES = inference_validation.EXECUTION_FIT_STATES
REQUIRED_SEMANTICS_VOCABULARIES = inference_validation.REQUIRED_SEMANTICS_VOCABULARIES
PROVIDER_COMPLETENESS_DOMAINS = inference_validation.PROVIDER_COMPLETENESS_DOMAINS
MODEL_COMPLETENESS_DOMAINS = inference_validation.MODEL_COMPLETENESS_DOMAINS
USAGE_SCENARIOS = {
    "adapter-planning",
    "context-limit-prefill",
    "cost-hint-display",
    "credential-setup-hint",
    "custom-api-endpoint-prefill",
    "custom-api-model-prefill",
    "lifecycle-warning",
    "model-directory",
    "model-discovery-setup",
    "output-limit-prefill",
    "provider-directory",
    "reasoning-capability-prefill",
    "reasoning-rendering-hint",
    "request-limit-prefill",
    "vision-capability-prefill",
}
EXPECTED_SCENARIO_REQUIREMENTS = {
    "adapter-planning": {
        "record_kind": "provider",
        "minimum": {"endpoint": "partial"},
        "requires_adapter_signal": True,
    },
    "context-limit-prefill": {
        "record_kind": "model",
        "minimum": {"limits": "partial"},
        "requires_known_field": "context_tokens",
    },
    "cost-hint-display": {"record_kind": "model", "minimum": {"cost": "partial"}},
    "credential-setup-hint": {
        "record_kind": "provider",
        "minimum": {"authentication": "partial"},
    },
    "custom-api-endpoint-prefill": {
        "record_kind": "provider",
        "minimum": {"endpoint": "complete"},
    },
    "custom-api-model-prefill": {
        "record_kind": "model",
        "minimum": {"identity": "partial"},
    },
    "lifecycle-warning": {"record_kind": "model", "minimum": {"lifecycle": "partial"}},
    "model-directory": {"record_kind": "model", "minimum": {"identity": "partial"}},
    "model-discovery-setup": {
        "record_kind": "provider",
        "minimum": {"discovery": "partial"},
    },
    "output-limit-prefill": {
        "record_kind": "model",
        "minimum": {"limits": "partial"},
        "requires_known_field": "max_output_tokens",
    },
    "provider-directory": {
        "record_kind": "provider",
        "minimum": {"identity": "partial"},
    },
    "reasoning-capability-prefill": {
        "record_kind": "model",
        "minimum": {"capabilities": "partial"},
        "requires_known_field": "reasoning",
    },
    "reasoning-rendering-hint": {
        "record_kind": "model",
        "minimum": {"reasoning_rendering": "complete"},
    },
    "request-limit-prefill": {
        "record_kind": "model",
        "minimum": {"limits": "complete"},
    },
    "vision-capability-prefill": {
        "record_kind": "model",
        "minimum": {"capabilities": "partial"},
        "requires_known_field": "vision",
    },
}


def find_repository_root() -> Path:
    for parent in Path(__file__).resolve().parents:
        if (parent / CATALOG_RELATIVE_PATH).is_file():
            return parent
    raise RuntimeError("cannot locate repository root containing the model metadata catalog")


def load_json(path: Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"cannot read valid JSON from {path}: {exc}") from exc


def canonical_digest(value: object) -> str:
    payload = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def is_sorted_unique(values: list[str]) -> bool:
    return values == sorted(set(values))


def is_sorted_unique_json(values: list[object]) -> bool:
    encoded = [
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        for value in values
    ]
    return encoded == sorted(set(encoded))


def is_sha256(value: object) -> bool:
    return bool(
        isinstance(value, str)
        and re.fullmatch(r"sha256:[0-9a-f]{64}", value)
    )


def assertion_scalars(record: dict[str, object], *fields: str) -> set[str]:
    values: set[str] = set()
    assertions = record.get("source_assertions", [])
    if not isinstance(assertions, list):
        return values
    for assertion in assertions:
        if not isinstance(assertion, dict):
            continue
        for field in fields:
            value = assertion.get(field)
            if isinstance(value, str) and value:
                values.add(value)
    return values


def expected_provider_completeness(record: dict[str, object]) -> dict[str, str]:
    auth = record.get("authentication_candidates", [])
    env_vars = record.get("environment_variable_names", [])
    discovery = record.get("discovery_modes", [])
    models_urls = record.get("models_url_candidates", [])
    defaults = record.get("default_model_ids", [])
    base_urls = record.get("base_url_candidates", [])
    protocols = record.get("protocol_candidates", [])
    unsupported_api = record.get("unsupported_api_styles", [])
    unsupported_transport = record.get("unsupported_transport_locators", [])
    explicit_identity_count = len(assertion_scalars(record, "display_name", "name"))
    authentication = "missing"
    if isinstance(auth, list) and auth:
        authentication = "complete" if len(auth) == 1 else "partial"
    elif isinstance(env_vars, list) and env_vars:
        authentication = "partial"
    discovery_state = "missing"
    if isinstance(discovery, list) and discovery and isinstance(models_urls, list) and models_urls:
        discovery_state = "complete"
    elif any(isinstance(value, list) and value for value in (discovery, models_urls, defaults)):
        discovery_state = "partial"
    endpoint_present = any(
        isinstance(value, list) and value
        for value in (base_urls, protocols, unsupported_api, unsupported_transport)
    )
    endpoint = "missing"
    if endpoint_present:
        endpoint = "complete" if (
            isinstance(base_urls, list)
            and len(base_urls) == 1
            and isinstance(protocols, list)
            and len(protocols) == 1
            and not unsupported_api
            and not unsupported_transport
        ) else "partial"
    return {
        "authentication": authentication,
        "discovery": discovery_state,
        "endpoint": endpoint,
        "identity": "complete" if explicit_identity_count == 1 else "partial",
    }


def expected_provider_scenarios(record: dict[str, object], completeness: dict[str, str]) -> list[str]:
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


def expected_model_completeness(record: dict[str, object]) -> dict[str, str]:
    capabilities = record.get("capability_hints", {})
    capability_values = list(capabilities.values()) if isinstance(capabilities, dict) else []
    known_capabilities = sum(value != "unknown" for value in capability_values)
    capability_state = "missing"
    if known_capabilities:
        capability_state = "complete" if (
            known_capabilities == len(capability_values)
            and "conditional" not in capability_values
        ) else "partial"
    context = record.get("context_tokens", {})
    output = record.get("max_output_tokens", {})
    facts = [value if isinstance(value, dict) else {} for value in (context, output)]
    present_limits = sum(value.get("state") != "unknown" for value in facts)
    limits = "missing"
    if present_limits:
        determinate = all(value.get("state") in ("known", "not-applicable") for value in facts)
        limits = "complete" if present_limits == 2 and determinate else "partial"
    rendering = record.get("reasoning_rendering_hints", {})
    rendering_values = rendering.values() if isinstance(rendering, dict) else []
    has_rendering = any(isinstance(value, list) and value for value in rendering_values)
    reasoning = capabilities.get("reasoning") if isinstance(capabilities, dict) else None
    if has_rendering:
        reasoning_rendering = "complete"
    elif record.get("reasoning_rendering_state") == "not-applicable":
        reasoning_rendering = "not-applicable"
    elif reasoning in ("supported", "conditional"):
        reasoning_rendering = "partial"
    else:
        reasoning_rendering = "missing"
    lifecycle = record.get("lifecycle")
    cost_hints = record.get("cost_hints", [])
    has_usable_cost = isinstance(cost_hints, list) and close_model_metadata.has_usable_cost_hint(
        cost_hints
    )
    return {
        "capabilities": capability_state,
        "cost": "partial" if has_usable_cost else "missing",
        "identity": "complete",
        "lifecycle": "complete" if lifecycle != "unknown" else "missing",
        "limits": limits,
        "modalities": "complete" if record.get("input_modalities") else "missing",
        "reasoning_rendering": reasoning_rendering,
    }


def expected_model_scenarios(record: dict[str, object], completeness: dict[str, str]) -> list[str]:
    scenarios = ["custom-api-model-prefill", "model-directory"]
    context = record.get("context_tokens", {})
    output = record.get("max_output_tokens", {})
    capabilities = record.get("capability_hints", {})
    if isinstance(context, dict) and context.get("state") == "known":
        scenarios.append("context-limit-prefill")
    if isinstance(output, dict) and output.get("state") == "known":
        scenarios.append("output-limit-prefill")
    if completeness["limits"] == "complete":
        scenarios.append("request-limit-prefill")
    if isinstance(capabilities, dict) and capabilities.get("reasoning") in (
        "supported", "unsupported", "conditional",
    ):
        scenarios.append("reasoning-capability-prefill")
    if isinstance(capabilities, dict) and capabilities.get("vision") in (
        "supported", "unsupported", "conditional",
    ):
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
    parser = argparse.ArgumentParser()
    parser.add_argument("--catalog", type=Path, default=root / CATALOG_RELATIVE_PATH)
    parser.add_argument(
        "--source-registry", type=Path, default=root / REGISTRY_RELATIVE_PATH
    )
    parser.add_argument(
        "--client-discovery-runs",
        type=Path,
        default=root / DISCOVERY_RUNS_RELATIVE_PATH,
    )
    parser.add_argument(
        "--client-source-profiles",
        type=Path,
        default=root / CLIENT_PROFILES_RELATIVE_PATH,
    )
    parser.add_argument(
        "--inference-rules",
        type=Path,
        default=root / RULES_RELATIVE_PATH,
    )
    parser.add_argument(
        "--today",
        type=lambda value: datetime.strptime(value, "%Y-%m-%d").date(),
        default=date.today(),
        help="date used for freshness reporting (YYYY-MM-DD)",
    )
    args = parser.parse_args()

    try:
        catalog = load_json(args.catalog)
        registry = load_json(args.source_registry)
        discovery_runs = load_json(args.client_discovery_runs)
        client_profiles = load_json(args.client_source_profiles)
        rules_document = load_json(args.inference_rules)
    except RuntimeError as exc:
        print(json.dumps({"result": "red", "errors": [str(exc)]}))
        return 1

    errors: list[str] = []
    stale_sources: list[dict[str, str | int]] = []

    typed_values = (catalog, registry, discovery_runs, client_profiles, rules_document)
    if not all(isinstance(value, dict) for value in typed_values):
        errors.append("catalog, registries, runs, profiles, and rules must be JSON objects")
        catalog = catalog if isinstance(catalog, dict) else {}
        registry = registry if isinstance(registry, dict) else {}
        discovery_runs = discovery_runs if isinstance(discovery_runs, dict) else {}
        client_profiles = client_profiles if isinstance(client_profiles, dict) else {}
        rules_document = rules_document if isinstance(rules_document, dict) else {}

    catalog_digest = canonical_digest(catalog)

    expected_scope = {
        "catalog_generation": "current",
        "delivery": "client-bundled",
        "future_refresh_transport": "https",
        "includes_compatibility_contract": False,
        "includes_legacy_projection": False,
        "runtime_projection_source": True,
    }
    if catalog.get("artifact_kind") != "hiroute.model-metadata-current-source":
        errors.append("unexpected catalog artifact_kind")
    if catalog.get("scope") != expected_scope:
        errors.append("catalog scope must remain the exact single-current-source scope")
    if registry.get("artifact_kind") != "hiroute.model-metadata-source-registry":
        errors.append("unexpected source registry artifact_kind")
    if discovery_runs.get("artifact_kind") != "hiroute.model-metadata-client-discovery-runs":
        errors.append("unexpected client discovery runs artifact_kind")
    if client_profiles.get("artifact_kind") != "hiroute.model-metadata-client-source-profiles":
        errors.append("unexpected client source profiles artifact_kind")
    profile_records = client_profiles.get("profiles", [])
    if not isinstance(profile_records, list):
        errors.append("client source profiles must be an array")
        profile_records = []
    profile_keys = [record.get("profile_key") for record in profile_records]
    if not all(isinstance(value, str) and value for value in profile_keys) or not is_sorted_unique(profile_keys):
        errors.append("client source profiles must be sorted and unique by profile_key")
    profiles_by_key = {record.get("profile_key"): record for record in profile_records}
    for profile_key, profile in profiles_by_key.items():
        if profile.get("evidence_eligible") is not False:
            errors.append(f"client source profile {profile_key} must be discovery-only")
        repository_urls = profile.get("expected_repository_urls", [])
        if not isinstance(repository_urls, list) or not repository_urls or repository_urls != sorted(set(repository_urls)):
            errors.append(f"client source profile {profile_key} needs sorted repository URLs")
        elif not all(isinstance(url, str) and url.startswith("https://") for url in repository_urls):
            errors.append(f"client source profile {profile_key} has invalid repository URL")
    if discovery_runs.get("catalog_digest_compared") != catalog_digest:
        errors.append("client discovery runs must target the current canonical catalog digest")
    discovery_policy = discovery_runs.get("policy", {})
    if not isinstance(discovery_policy, dict):
        errors.append("client discovery runs need a policy object")
    else:
        if discovery_policy.get("candidate_outputs_materialized") is not True or discovery_policy.get(
            "metadata_records_materialized"
        ) is not True:
            errors.append("client discovery runs must record materialized metadata records")
        if discovery_policy.get("scenario_eligibility_basis") != "field-completeness":
            errors.append("client discovery scenario eligibility must use field completeness")
        if discovery_policy.get("provenance_controls_scenario_eligibility") is not False:
            errors.append("client discovery provenance must not gate scenario eligibility")
    run_records = discovery_runs.get("runs", [])
    if not isinstance(run_records, list):
        errors.append("client discovery runs must be an array")
        run_records = []
    run_keys = [record.get("profile_key") for record in run_records]
    if not all(isinstance(value, str) and value for value in run_keys) or not is_sorted_unique(run_keys):
        errors.append("client discovery runs must be sorted and unique by profile_key")
    for run in run_records:
        profile_key = run.get("profile_key")
        registered_profile = profiles_by_key.get(profile_key)
        if registered_profile is None:
            errors.append(f"client discovery run {profile_key} has no registered profile")
        elif run.get("profile_digest") != canonical_digest(registered_profile):
            errors.append(f"client discovery run {profile_key} profile digest is stale")
        if not isinstance(run.get("commit"), str) or not re.fullmatch(r"[0-9a-f]{40}", run["commit"]):
            errors.append(f"client discovery run {profile_key} has invalid commit")
        if not isinstance(run.get("tree"), str) or not re.fullmatch(r"[0-9a-f]{40,64}", run["tree"]):
            errors.append(f"client discovery run {profile_key} has invalid tree")
        for field in ("profile_digest", "result_digest"):
            if not is_sha256(run.get(field)):
                errors.append(f"client discovery run {profile_key} has invalid {field}")
        if run.get("determinism_check") != "byte-identical-two-runs":
            errors.append(f"client discovery run {profile_key} lacks deterministic rerun evidence")
        if not isinstance(run.get("scanned_blob_count"), int) or run["scanned_blob_count"] <= 0:
            errors.append(f"client discovery run {profile_key} has invalid scanned blob count")
    runs_by_key = {record.get("profile_key"): record for record in run_records}
    resolved_changes = discovery_runs.get("resolved_catalog_changes", [])
    if not isinstance(resolved_changes, list):
        errors.append("resolved catalog changes must be an array")
    else:
        change_keys = [record.get("change_key") for record in resolved_changes]
        if not all(isinstance(value, str) and value for value in change_keys) or not is_sorted_unique(change_keys):
            errors.append("resolved catalog changes must be sorted and unique by change_key")

    authority_records = registry.get("authorities", [])
    if not isinstance(authority_records, list):
        errors.append("source registry authorities must be an array")
        authority_records = []
    authority_keys = [item.get("authority_key") for item in authority_records]
    if not all(isinstance(value, str) and value for value in authority_keys):
        errors.append("every source authority needs a non-empty authority_key")
    elif not is_sorted_unique(authority_keys):
        errors.append("source authorities must be sorted and unique by authority_key")

    prefix_records: list[tuple[str, str, int]] = []
    for authority in authority_records:
        refresh_days = authority.get(
            "refresh_days", registry.get("default_refresh_days", 30)
        )
        if not isinstance(refresh_days, int) or refresh_days <= 0:
            errors.append(f"invalid refresh_days for {authority.get('authority_key')}")
            continue
        prefixes = authority.get("allowed_https_prefixes", [])
        if not isinstance(prefixes, list) or not prefixes:
            errors.append(f"authority {authority.get('authority_key')} has no prefixes")
            continue
        if prefixes != sorted(set(prefixes)):
            errors.append(
                f"authority {authority.get('authority_key')} prefixes are not sorted/unique"
            )
        for prefix in prefixes:
            if not isinstance(prefix, str) or not prefix.startswith("https://"):
                errors.append(f"invalid HTTPS prefix for {authority.get('authority_key')}")
                continue
            prefix_records.append((prefix, authority["authority_key"], refresh_days))

    semantics = catalog.get("semantics", {})
    capability_states = set(semantics.get("capability_state", []))
    token_states = set(semantics.get("token_limit_state", []))
    reasoning_kinds = set(semantics.get("reasoning_kind", []))
    stability_states = set(semantics.get("identity_stability", []))
    lifecycle_states = set(semantics.get("lifecycle", []))
    dispositions = set(semantics.get("disposition", []))
    decision_states = set(semantics.get("data_quality_status", []))
    pricing_components = set(semantics.get("pricing_rate_component", []))
    pricing_conditions = set(semantics.get("pricing_condition_kind", []))
    pricing_applications = set(semantics.get("pricing_application", []))
    upstream_identity_roles = set(semantics.get("upstream_identity_role", []))
    upstream_id_states = set(semantics.get("upstream_id_state", []))
    metadata_fact_states = set(semantics.get("metadata_fact_state", []))
    metadata_completeness_states = set(semantics.get("metadata_completeness_state", []))
    metadata_provenance_kinds = set(semantics.get("metadata_provenance_kind", []))
    metadata_usage_scenarios = set(semantics.get("metadata_usage_scenario", []))

    collections = {
        "evidence_sources": "source_key",
        "access_products": "product_key",
        "models": "model_key",
        "endpoint_bindings": "binding_key",
        "dynamic_routes": "route_key",
        "pricing_facts": "pricing_key",
        "provider_metadata_records": "provider_record_key",
        "model_metadata_records": "model_record_key",
        "data_quality_decisions": "decision_key",
    }
    for collection_name, key_name in collections.items():
        records = catalog.get(collection_name, [])
        if not isinstance(records, list):
            errors.append(f"{collection_name} must be an array")
            continue
        keys = [item.get(key_name) for item in records]
        if not all(isinstance(value, str) and value for value in keys):
            errors.append(f"every {collection_name} record needs {key_name}")
        elif not is_sorted_unique(keys):
            errors.append(f"{collection_name} must be sorted and unique by {key_name}")

    sources = {
        item.get("source_key"): item for item in catalog.get("evidence_sources", [])
    }
    for source_key, source in sources.items():
        locator = source.get("locator")
        if not isinstance(locator, str) or not locator.startswith("https://"):
            errors.append(f"{source_key} has an invalid locator")
            continue
        matches = [record for record in prefix_records if locator.startswith(record[0])]
        if not matches:
            errors.append(f"{source_key} locator is not authorized by source registry")
            continue
        parsed_authority = urlparse(locator).netloc
        if source.get("authority") != parsed_authority:
            errors.append(f"{source_key} authority does not match locator")
        digest_state = source.get("digest_state")
        digest = source.get("bytes_digest")
        if digest_state == "captured":
            if not isinstance(digest, str) or len(digest) != 71:
                errors.append(f"{source_key} captured digest is not sha256:<64 hex>")
            elif not digest.startswith("sha256:") or any(
                char not in "0123456789abcdef" for char in digest[7:]
            ):
                errors.append(f"{source_key} captured digest is malformed")
        elif digest_state == "not-captured":
            if digest is not None:
                errors.append(f"{source_key} not-captured digest must be null")
        else:
            errors.append(f"{source_key} has invalid digest_state")
        try:
            collected_on = datetime.strptime(source.get("collected_on", ""), "%Y-%m-%d").date()
        except (TypeError, ValueError):
            errors.append(f"{source_key} has invalid collected_on")
            continue
        refresh_days = min(record[2] for record in matches)
        if collected_on + timedelta(days=refresh_days) < args.today:
            stale_sources.append(
                {
                    "source_key": source_key,
                    "collected_on": collected_on.isoformat(),
                    "refresh_days": refresh_days,
                }
            )

    def check_evidence_refs(owner: str, refs: object) -> None:
        if not isinstance(refs, list) or not refs:
            errors.append(f"{owner} needs at least one evidence_ref")
            return
        if refs != sorted(set(refs)):
            errors.append(f"{owner} evidence_refs must be sorted and unique")
        missing = set(refs) - set(sources)
        if missing:
            errors.append(f"{owner} references missing sources: {sorted(missing)}")

    catalog_rules = inference_validation.validate_rule_catalog(
        catalog,
        rules_document,
        sources,
        catalog_digest,
        canonical_digest,
        is_sorted_unique,
        errors,
    )

    products = {
        item.get("product_key"): item for item in catalog.get("access_products", [])
    }
    interfaces: dict[str, str] = {}
    for product_key, product in products.items():
        check_evidence_refs(f"product {product_key}", product.get("evidence_refs"))
        if not set(product.get("disposition", [])) <= dispositions:
            errors.append(f"product {product_key} has an invalid disposition")
        for interface in product.get("interfaces", []):
            interface_key = interface.get("interface_key")
            if not isinstance(interface_key, str) or not interface_key.startswith(
                f"{product_key}/"
            ):
                errors.append(f"product {product_key} has an invalid interface_key")
            elif interface_key in interfaces:
                errors.append(f"duplicate interface_key {interface_key}")
            else:
                interfaces[interface_key] = product_key

    models = {item.get("model_key"): item for item in catalog.get("models", [])}
    upstream_owners: dict[str, str] = {}
    for model_key, model in models.items():
        check_evidence_refs(f"model {model_key}", model.get("evidence_refs"))
        if model.get("identity_stability") not in stability_states:
            errors.append(f"model {model_key} has invalid identity_stability")
        if model.get("lifecycle") not in lifecycle_states:
            errors.append(f"model {model_key} has invalid lifecycle")
        if "upstream_id_state" in model and model.get("upstream_id_state") not in upstream_id_states:
            errors.append(f"model {model_key} has invalid upstream_id_state")
        for field in ("modalities", "capabilities"):
            values = model.get(field, {})
            if not isinstance(values, dict) or not set(values.values()) <= capability_states:
                errors.append(f"model {model_key} has invalid {field}")
        reasoning = model.get("reasoning", {})
        if reasoning.get("kind") not in reasoning_kinds:
            errors.append(f"model {model_key} has invalid reasoning kind")
        for field in ("context_tokens", "max_output_tokens"):
            value = model.get(field)
            state = model.get(field.replace("_tokens", "_state"))
            if isinstance(value, bool) or (
                value is not None and (not isinstance(value, int) or value <= 0)
            ):
                errors.append(f"model {model_key} has invalid {field}")
            if value is None and state not in token_states:
                errors.append(f"model {model_key} null {field} needs an explicit state")
            if state is not None and state not in token_states:
                errors.append(f"model {model_key} has invalid {field} state")
        upstream_ids = model.get("upstream_ids", [])
        if upstream_ids != sorted(set(upstream_ids), key=str.casefold):
            errors.append(f"model {model_key} upstream_ids must be sorted and unique")
        for upstream_id in upstream_ids:
            folded = upstream_id.casefold()
            previous = upstream_owners.setdefault(folded, model_key)
            if previous != model_key:
                errors.append(
                    f"upstream model ID {upstream_id} belongs to both {previous} and {model_key}"
                )

    if metadata_fact_states != METADATA_FACT_STATES:
        errors.append("metadata fact states must cover the determinate token-fact vocabulary")
    if metadata_completeness_states != COMPLETENESS_STATES:
        errors.append("metadata completeness states must include the determinate outcomes")
    if capability_states != CAPABILITY_STATES:
        errors.append("capability states must include the determinate outcomes")
    if token_states != TOKEN_LIMIT_STATES:
        errors.append("token limit states must include the determinate outcomes")
    if metadata_provenance_kinds != {"client-discovery-run", "evidence-source"}:
        errors.append("metadata provenance kinds are incomplete")
    if metadata_usage_scenarios != USAGE_SCENARIOS:
        errors.append("metadata usage scenario vocabulary is incomplete")
    semantics_updates = rules_document.get("semantics_updates", {})
    if not isinstance(semantics_updates, dict):
        errors.append("inference rules need a semantics_updates object")
        semantics_updates = {}
    for vocabulary, required_states in REQUIRED_SEMANTICS_VOCABULARIES.items():
        declared = semantics_updates.get(vocabulary)
        if declared != sorted(required_states):
            errors.append(f"inference rules must declare the exact {vocabulary} vocabulary")
        elif semantics.get(vocabulary) != declared:
            errors.append(f"catalog semantics {vocabulary} must match the inference-rule vocabulary")
    for vocabulary in set(semantics_updates) - set(REQUIRED_SEMANTICS_VOCABULARIES):
        errors.append(f"unsupported inference-rule vocabulary {vocabulary}")
    encoding_additions = rules_document.get("encoding_rules_additions", [])
    if not isinstance(encoding_additions, list) or not encoding_additions:
        errors.append("inference rules need encoding-rule additions")
    else:
        for rule in encoding_additions:
            if not isinstance(rule, str) or rule not in semantics.get("encoding_rules", []):
                errors.append("catalog encoding rules must include every closure policy rule")

    usage_policy = catalog.get("metadata_usage_policy", {})
    if not isinstance(usage_policy, dict):
        errors.append("metadata usage policy must be an object")
    else:
        if usage_policy.get("eligibility_basis") != "field-completeness":
            errors.append("metadata usage policy must use field completeness")
        if usage_policy.get("provenance_controls_scenario_eligibility") is not False:
            errors.append("metadata provenance must not control scenario eligibility")
        if usage_policy.get("runtime_execution_requires_qualification") is not True:
            errors.append("metadata usage policy must preserve runtime qualification")
        completeness_domains = usage_policy.get("completeness_domains", {})
        if completeness_domains != {
            "model": sorted(MODEL_COMPLETENESS_DOMAINS),
            "provider": sorted(PROVIDER_COMPLETENESS_DOMAINS),
        }:
            errors.append("metadata usage policy completeness domains are stale")
        if usage_policy.get("completeness_states") != sorted(COMPLETENESS_STATES):
            errors.append("metadata usage policy completeness states are stale")
        requirements = usage_policy.get("scenario_requirements", {})
        if not isinstance(requirements, dict) or set(requirements) != USAGE_SCENARIOS:
            errors.append("metadata usage policy scenario requirements are incomplete")
        else:
            if requirements != EXPECTED_SCENARIO_REQUIREMENTS:
                errors.append("metadata usage policy scenario requirements are stale")
            for scenario, requirement in requirements.items():
                if not isinstance(requirement, dict) or requirement.get("record_kind") not in {
                    "model", "provider"
                }:
                    errors.append(f"scenario {scenario} has invalid record kind")
                    continue
                domains = (
                    MODEL_COMPLETENESS_DOMAINS
                    if requirement["record_kind"] == "model"
                    else PROVIDER_COMPLETENESS_DOMAINS
                )
                minimum = requirement.get("minimum", {})
                if not isinstance(minimum, dict) or not minimum or not set(minimum) <= domains:
                    errors.append(f"scenario {scenario} has invalid completeness requirement")
                elif not set(minimum.values()) <= COMPLETENESS_STATES:
                    errors.append(f"scenario {scenario} has invalid minimum state")

    provider_records = {
        item.get("provider_record_key"): item
        for item in catalog.get("provider_metadata_records", [])
    }
    list_fields = PROVIDER_FIELD_NAMES

    def check_metadata_provenance(label: str, record: dict[str, object]) -> None:
        refs = record.get("provenance_refs", [])
        if not isinstance(refs, list) or not refs or not is_sorted_unique_json(refs):
            errors.append(f"{label} provenance refs are invalid")
            return
        for ref in refs:
            if not isinstance(ref, dict) or set(ref) != {"kind", "source_key"}:
                errors.append(f"{label} has malformed provenance ref")
                continue
            kind = ref.get("kind")
            source_key = ref.get("source_key")
            if kind not in metadata_provenance_kinds:
                errors.append(f"{label} has invalid provenance kind")
            elif kind == "client-discovery-run" and source_key not in runs_by_key:
                errors.append(f"{label} has missing discovery run")
            elif kind == "evidence-source" and source_key not in sources:
                errors.append(f"{label} has missing evidence source")

    def check_source_locations(label: str, record: dict[str, object]) -> None:
        locations = record.get("source_locations", [])
        if (
            not isinstance(locations, list)
            or not locations
            or not is_sorted_unique_json(locations)
        ):
            errors.append(f"{label} source locations are invalid")
            return
        for location in locations:
            if not isinstance(location, dict):
                errors.append(f"{label} has a malformed source location")
            elif set(location) == {"source_key"}:
                if location["source_key"] not in sources:
                    errors.append(f"{label} cites a missing evidence source location")
            elif not set(location) <= {"git_object_id", "line", "path", "pointer"}:
                errors.append(f"{label} has an unrecognized source location shape")

    for record_key, record in provider_records.items():
        check_metadata_provenance(f"provider metadata {record_key}", record)
        provider_id = record.get("provider_id")
        if not isinstance(provider_id, str) or not provider_id:
            errors.append(f"provider metadata {record_key} has invalid provider_id")
        if not isinstance(record.get("display_name"), str) or not record.get("display_name"):
            errors.append(f"provider metadata {record_key} has invalid display_name")
        for field in list_fields:
            values = record.get(field, [])
            if not isinstance(values, list) or not all(
                isinstance(value, str) and value for value in values
            ) or not is_sorted_unique(values):
                errors.append(f"provider metadata {record_key} has invalid {field}")
        for base_url in record.get("base_url_candidates", []):
            parsed = urlparse(base_url)
            if parsed.scheme not in {"http", "https"} or not parsed.netloc or parsed.username or parsed.password:
                errors.append(f"provider metadata {record_key} has unsafe base URL")
        protocols = record.get("protocol_candidates", [])
        if not set(protocols) <= {"chat_completions", "messages", "responses"}:
            errors.append(f"provider metadata {record_key} has unsupported protocol candidate")
        completeness = record.get("metadata_completeness", {})
        if not isinstance(completeness, dict) or set(completeness) != PROVIDER_COMPLETENESS_DOMAINS:
            errors.append(f"provider metadata {record_key} completeness domains are invalid")
            completeness = {}
        elif not set(completeness.values()) <= COMPLETENESS_STATES:
            errors.append(f"provider metadata {record_key} has an invalid completeness state")
        inference_validation.check_closure_provenance(
            f"provider metadata {record_key}",
            record,
            allowed_fields=tuple(sorted(PROVIDER_COMPLETENESS_DOMAINS)),
            record_kind="provider",
            normalized_id="",
            domains=True,
            catalog_rules=catalog_rules,
            errors=errors,
        )
        expected_completeness = expected_provider_completeness(record)
        for domain, actual in completeness.items():
            if actual == expected_completeness[domain]:
                continue
            if (record.get("field_provenance") or {}).get(domain) is not None:
                continue
            errors.append(
                f"provider metadata {record_key} {domain} completeness is not derived "
                "from fields or an inference rule"
            )
        expected_scenarios = expected_provider_scenarios(record, completeness)
        if record.get("usable_for") != expected_scenarios:
            errors.append(f"provider metadata {record_key} scenarios are not derived from completeness")
        assertions = record.get("source_assertions", [])
        if not isinstance(assertions, list) or not is_sorted_unique_json(assertions):
            errors.append(f"provider metadata {record_key} source assertions are not deterministic")
        check_source_locations(f"provider metadata {record_key}", record)

    for record in catalog.get("model_metadata_records", []):
        record_key = record.get("model_record_key")
        provider_record = provider_records.get(record.get("provider_record_key"))
        if provider_record is None:
            errors.append(f"model metadata {record_key} has missing provider metadata")
            continue
        check_metadata_provenance(f"model metadata {record_key}", record)
        if record.get("provider_id") != provider_record.get("provider_id"):
            errors.append(f"model metadata {record_key} crosses provider identities")
        for field in ("upstream_model_id", "display_name"):
            if not isinstance(record.get(field), str) or not record.get(field):
                errors.append(f"model metadata {record_key} has invalid {field}")
        for field in ("context_tokens", "max_output_tokens"):
            fact = record.get(field, {})
            if not isinstance(fact, dict) or fact.get("state") not in metadata_fact_states:
                errors.append(f"model metadata {record_key} has invalid {field} state")
                continue
            if fact.get("state") == "unknown":
                errors.append(f"model metadata {record_key} has an indeterminate {field}")
            value = fact.get("value")
            if fact.get("state") == "known":
                if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
                    errors.append(f"model metadata {record_key} has invalid known {field}")
            elif value is not None:
                errors.append(f"model metadata {record_key} non-known {field} must be null")
            candidates = fact.get("candidates", [])
            if candidates and (
                not isinstance(candidates, list)
                or not all(isinstance(item, int) and not isinstance(item, bool) and item > 0 for item in candidates)
                or candidates != sorted(set(candidates))
            ):
                errors.append(f"model metadata {record_key} has invalid {field} candidates")
            if fact.get("state") == "conflict" and len(candidates) < 2:
                errors.append(f"model metadata {record_key} conflicting {field} needs candidates")
        capability_hints = record.get("capability_hints", {})
        if not isinstance(capability_hints, dict) or set(capability_hints) != {
            "reasoning", "streaming", "tool", "vision"
        } or not set(capability_hints.values()) <= capability_states:
            errors.append(f"model metadata {record_key} has invalid capability hints")
        elif "unknown" in capability_hints.values():
            errors.append(f"model metadata {record_key} has an indeterminate capability hint")
        if record.get("lifecycle") not in lifecycle_states:
            errors.append(f"model metadata {record_key} has invalid lifecycle")
        elif record.get("lifecycle") == "unknown":
            errors.append(f"model metadata {record_key} has an indeterminate lifecycle")
        for field in (
            "normalized_model_matches",
            "input_modalities",
            "replacement_upstream_ids",
            "roles",
            "status_candidates",
        ):
            values = record.get(field, [])
            if not isinstance(values, list) or not all(
                isinstance(value, str) and value for value in values
            ) or not is_sorted_unique(values):
                errors.append(f"model metadata {record_key} has invalid {field}")
        rendering = record.get("reasoning_rendering_hints", {})
        rendering_state = record.get("reasoning_rendering_state")
        if not isinstance(rendering, dict) or set(rendering) != {
            "reasoning_effort_maps", "supported_reasoning_efforts", "thinking_level_maps"
        }:
            errors.append(f"model metadata {record_key} has invalid reasoning rendering hints")
            rendering = {}
        else:
            for field in ("reasoning_effort_maps", "thinking_level_maps"):
                values = rendering.get(field, [])
                if not isinstance(values, list) or not all(isinstance(value, dict) for value in values) or not is_sorted_unique_json(values):
                    errors.append(f"model metadata {record_key} has invalid {field}")
            efforts = rendering.get("supported_reasoning_efforts", [])
            if not isinstance(efforts, list) or not all(isinstance(value, str) and value for value in efforts) or not is_sorted_unique(efforts):
                errors.append(f"model metadata {record_key} has invalid supported reasoning efforts")
            has_rendering = any(rendering.get(field) for field in rendering)
            if rendering_state is None:
                if not has_rendering:
                    errors.append(
                        f"model metadata {record_key} has an indeterminate reasoning rendering outcome"
                    )
            elif rendering_state != "not-applicable":
                errors.append(f"model metadata {record_key} has an invalid reasoning rendering state")
            elif has_rendering:
                errors.append(
                    f"model metadata {record_key} declares not-applicable rendering with hints"
                )
        execution_fit = record.get("execution_fit")
        if not isinstance(execution_fit, dict) or execution_fit.get("state") not in EXECUTION_FIT_STATES:
            errors.append(f"model metadata {record_key} has an invalid execution fit")
        else:
            if set(execution_fit) - {"state", "reason"}:
                errors.append(f"model metadata {record_key} has extra execution fit fields")
            reason = execution_fit.get("reason")
            if execution_fit["state"] == "native-text-representable":
                if "text" not in record.get("input_modalities", []):
                    errors.append(
                        f"model metadata {record_key} is text-representable without text input"
                    )
            elif not isinstance(reason, str) or not reason:
                errors.append(f"model metadata {record_key} needs an execution fit reason")
        cost_hints = record.get("cost_hints", [])
        if not isinstance(cost_hints, list) or not all(isinstance(value, dict) for value in cost_hints) or not is_sorted_unique_json(cost_hints):
            errors.append(f"model metadata {record_key} has invalid cost hints")
        has_usable_cost = close_model_metadata.has_usable_cost_hint(cost_hints)
        expected_cost_state = "recorded" if has_usable_cost else "not-recorded"
        if record.get("cost_hint_state") != expected_cost_state:
            errors.append(f"model metadata {record_key} cost hint state is not derived from hints")
        missing_matches = set(record.get("normalized_model_matches", [])) - set(models)
        if missing_matches:
            errors.append(f"model metadata {record_key} references missing canonical models")
        inference_validation.check_closure_provenance(
            f"model metadata {record_key}",
            record,
            allowed_fields=MODEL_CLOSABLE_FIELDS,
            record_kind="model",
            normalized_id=close_model_metadata.normalize_upstream_id(
                record.get("upstream_model_id", "")
            ),
            domains=False,
            catalog_rules=catalog_rules,
            errors=errors,
        )
        expected_completeness = expected_model_completeness(record)
        if record.get("metadata_completeness") != expected_completeness:
            errors.append(f"model metadata {record_key} completeness is not derived from fields")
        expected_scenarios = expected_model_scenarios(record, expected_completeness)
        if record.get("usable_for") != expected_scenarios:
            errors.append(f"model metadata {record_key} scenarios are not derived from completeness")
        assertions = record.get("source_assertions", [])
        if not isinstance(assertions, list) or not is_sorted_unique_json(assertions):
            errors.append(f"model metadata {record_key} source assertions are not deterministic")
        check_source_locations(f"model metadata {record_key}", record)

    all_metadata_records = [
        *provider_records.values(),
        *catalog.get("model_metadata_records", []),
    ]
    expected_materialization = {
        "model_metadata_record_count": len(catalog.get("model_metadata_records", [])),
        "provider_metadata_record_count": len(provider_records),
        "scenario_eligible_record_counts": {
            scenario: sum(scenario in record.get("usable_for", []) for record in all_metadata_records)
            for scenario in sorted(USAGE_SCENARIOS)
        },
        "unique_upstream_model_id_count": len(
            {
                record.get("upstream_model_id")
                for record in catalog.get("model_metadata_records", [])
                if isinstance(record.get("upstream_model_id"), str)
            }
        ),
    }
    if discovery_runs.get("materialization") != expected_materialization:
        errors.append("client discovery materialization summary is stale")

    for binding in catalog.get("endpoint_bindings", []):
        binding_key = binding.get("binding_key")
        product_key = binding.get("product_key")
        model_key = binding.get("model_key")
        if product_key not in products or model_key not in models:
            errors.append(f"binding {binding_key} has missing product/model")
            continue
        candidate_interfaces = binding.get("interface_candidates", [])
        if candidate_interfaces != sorted(set(candidate_interfaces)):
            errors.append(f"binding {binding_key} interface candidates are not sorted/unique")
        if any(interfaces.get(key) != product_key for key in candidate_interfaces):
            errors.append(f"binding {binding_key} references another product's interface")
        overrides = binding.get("capability_overrides", {})
        if not isinstance(overrides, dict) or set(overrides) - {
            "context_tokens", "max_output_tokens", "tool", "vision", "streaming"
        }:
            errors.append(f"binding {binding_key} has invalid capability overrides")
        else:
            for key, value in overrides.items():
                if key in ("context_tokens", "max_output_tokens") and (
                    isinstance(value, bool) or not isinstance(value, int) or value <= 0
                ):
                    errors.append(f"binding {binding_key} has invalid {key} override")
                if key in ("tool", "vision", "streaming") and not isinstance(value, bool):
                    errors.append(f"binding {binding_key} has invalid {key} override")
        if binding.get("upstream_model_id", "").casefold() not in {
            value.casefold() for value in models[model_key].get("upstream_ids", [])
        }:
            errors.append(f"binding {binding_key} upstream ID does not belong to model")
        if "upstream_identity_role" in binding and binding.get(
            "upstream_identity_role"
        ) not in upstream_identity_roles:
            errors.append(f"binding {binding_key} has invalid upstream identity role")
        if "lifecycle" in binding and binding.get("lifecycle") not in lifecycle_states:
            errors.append(f"binding {binding_key} has invalid lifecycle")
        replacement = binding.get("replaced_by_upstream_id")
        if replacement is not None and (
            not isinstance(replacement, str)
            or replacement.casefold()
            not in {value.casefold() for value in models[model_key].get("upstream_ids", [])}
        ):
            errors.append(f"binding {binding_key} has invalid replacement upstream ID")
        check_evidence_refs(f"binding {binding_key}", binding.get("evidence_refs"))

    for route in catalog.get("dynamic_routes", []):
        route_key = route.get("route_key")
        if route.get("product_key") not in products:
            errors.append(f"dynamic route {route_key} has missing product")
        if route.get("rated_model_identity") is not None:
            errors.append(f"dynamic route {route_key} must not have a rated model identity")
        if not route.get("require_actual_response_model") or not route.get(
            "require_actual_response_provider"
        ):
            errors.append(f"dynamic route {route_key} must require actual attribution")
        check_evidence_refs(f"dynamic route {route_key}", route.get("evidence_refs"))

    for pricing in catalog.get("pricing_facts", []):
        pricing_key = pricing.get("pricing_key")
        product_key = pricing.get("product_key")
        model_key = pricing.get("model_key")
        upstream_model_id = pricing.get("upstream_model_id")
        if product_key not in products or model_key not in models:
            errors.append(f"pricing {pricing_key} has missing product/model")
            continue
        if not isinstance(upstream_model_id, str) or upstream_model_id.casefold() not in {
            value.casefold() for value in models[model_key].get("upstream_ids", [])
        }:
            errors.append(f"pricing {pricing_key} upstream ID does not belong to model")
        if pricing.get("price_state") not in set(semantics.get("fact_state", [])):
            errors.append(f"pricing {pricing_key} has invalid price_state")
        currency = pricing.get("currency")
        if not isinstance(currency, str) or not re.fullmatch(r"[A-Z]{3}", currency):
            errors.append(f"pricing {pricing_key} has invalid currency")
        if pricing.get("unit") != "per-1m-tokens":
            errors.append(f"pricing {pricing_key} has unsupported unit")
        schedules = pricing.get("schedules", [])
        if not isinstance(schedules, list) or not schedules:
            errors.append(f"pricing {pricing_key} needs schedules")
            schedules = []
        schedule_keys = [schedule.get("schedule_key") for schedule in schedules]
        if not all(isinstance(value, str) and value for value in schedule_keys) or not is_sorted_unique(
            schedule_keys
        ):
            errors.append(f"pricing {pricing_key} schedules must be sorted and unique")
        for schedule in schedules:
            schedule_key = schedule.get("schedule_key")
            condition = schedule.get("condition", {})
            if not isinstance(condition, dict) or condition.get("kind") not in pricing_conditions:
                errors.append(f"pricing {pricing_key}/{schedule_key} has invalid condition")
            if schedule.get("application") not in pricing_applications:
                errors.append(f"pricing {pricing_key}/{schedule_key} has invalid application")
            rates = schedule.get("rates")
            if not isinstance(rates, dict) or not rates:
                errors.append(f"pricing {pricing_key}/{schedule_key} needs rates")
                continue
            if not set(rates) <= pricing_components:
                errors.append(f"pricing {pricing_key}/{schedule_key} has invalid rate components")
            for component, rate in rates.items():
                if not isinstance(rate, str):
                    errors.append(f"pricing {pricing_key}/{schedule_key}/{component} rate must be a decimal string")
                    continue
                try:
                    parsed_rate = Decimal(rate)
                except InvalidOperation:
                    errors.append(f"pricing {pricing_key}/{schedule_key}/{component} rate is invalid")
                    continue
                if not parsed_rate.is_finite() or parsed_rate < 0:
                    errors.append(f"pricing {pricing_key}/{schedule_key}/{component} rate must be finite and non-negative")
        check_evidence_refs(f"pricing {pricing_key}", pricing.get("evidence_refs"))

    for decision in catalog.get("data_quality_decisions", []):
        if decision.get("status") not in decision_states:
            errors.append(
                f"decision {decision.get('decision_key')} has an invalid status"
            )

    result = {
        "result": "red" if errors else "green",
        "catalog": str(args.catalog),
        "canonical_digest": catalog_digest,
        "counts": {name: len(catalog.get(name, [])) for name in collections},
        "client_discovery_run_count": len(run_records),
        "stale_sources": stale_sources,
        "errors": errors,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
