#!/usr/bin/env python3
"""Offline generator for the one current client-bundled model catalog."""
import argparse
import copy
import datetime
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
CURRENT_PRODUCT_RELEASE = "mvp-current"

def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()

def digest(value):
    return "sha256:" + hashlib.sha256(canonical(value)).hexdigest()

def pinned_json(relative_path, expected_bytes_digest):
    raw = (ROOT / relative_path).read_bytes()
    if "sha256:" + hashlib.sha256(raw).hexdigest() != expected_bytes_digest:
        raise ValueError(f"pinned source bytes changed: {relative_path}")
    return json.loads(raw)

def optional(value, key):
    return value.get(key)

def project_token_limit(value, state=None):
    resolved = state or ("known" if value is not None else "unknown")
    return {"state": resolved.replace("-", "_"), "value": value}

def project_state(value):
    """The maintained catalog spells multi-word states in kebab case; Rust enums do not."""
    return value if value is None else value.replace("-", "_")

def project_cost_hint(value):
    """Keep source shape but make every numeric leaf display-only and byte-stable."""
    if isinstance(value, dict):
        return {key: project_cost_hint(item) for key, item in value.items()}
    if isinstance(value, list):
        return [project_cost_hint(item) for item in value]
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return copy.deepcopy(value)
    if isinstance(value, (int, float)):
        return str(value)
    raise ValueError(f"unsupported cost hint value: {type(value).__name__}")

def project_metadata_catalog(catalog, client_runs):
    source_digest = digest(catalog)
    if client_runs["catalog_digest_compared"] != source_digest:
        raise ValueError("client discovery runs do not bind the pinned metadata catalog")
    if client_runs["as_of"] != catalog["as_of"]:
        raise ValueError("metadata catalog and client discovery runs use different snapshots")

    evidence_sources = [copy.deepcopy(value) for value in catalog["evidence_sources"]]
    runs = [{
        "source_key": value["profile_key"],
        "profile_digest": value["profile_digest"],
        "repository_url": value["repository_url"],
        "commit": value["commit"],
        "tree": value["tree"],
        "result_digest": value["result_digest"],
    } for value in client_runs["runs"]]
    access_products = []
    for value in catalog["access_products"]:
        access_products.append({
            "product_key": value["product_key"],
            "provider": value["provider"],
            "product": value["product"],
            "product_kind": value["product_kind"],
            "region_scope": value["region_scope"],
            "billing": value["billing"],
            "restriction": value["restriction"],
            "credential": value["credential"],
            "interfaces": [{
                "interface_key": interface["interface_key"],
                "protocol": interface["protocol"],
                "base_url": optional(interface, "base_url"),
                "base_url_template": optional(interface, "base_url_template"),
                "request_path": optional(interface, "request_path"),
                "request_path_template": optional(interface, "request_path_template"),
            } for interface in value["interfaces"]],
            "documented_upstream_model_ids": value.get("documented_upstream_model_ids", []),
            "quota_facts": value.get("quota_facts", []),
            "evidence_refs": value["evidence_refs"],
            "disposition": value["disposition"],
        })
    canonical_models = []
    for value in catalog["models"]:
        canonical_models.append({
            "model_key": value["model_key"],
            "publisher": value["publisher"],
            "display_name": value["display_name"],
            "canonical_identity": value["canonical_identity"],
            "upstream_ids": value["upstream_ids"],
            "upstream_id_state": optional(value, "upstream_id_state"),
            "context_tokens": project_token_limit(
                optional(value, "context_tokens"), optional(value, "context_state")
            ),
            "context_variants": value.get("context_variants", []),
            "max_input_tokens": optional(value, "max_input_tokens"),
            "max_output_tokens": project_token_limit(
                optional(value, "max_output_tokens"), optional(value, "max_output_state")
            ),
            "max_output_scope": optional(value, "max_output_scope"),
            "max_output_note": optional(value, "max_output_note"),
            "modalities": value["modalities"],
            "capabilities": value["capabilities"],
            "reasoning": {
                "kind": value["reasoning"]["kind"],
                "profiles": value["reasoning"].get("profiles", []),
                "default": optional(value["reasoning"], "default"),
                "note": optional(value["reasoning"], "note"),
            },
            "identity_stability": value["identity_stability"],
            "lifecycle": value["lifecycle"],
            "source_stability_note": value["source_stability_note"],
            "evidence_refs": value["evidence_refs"],
            "data_note": optional(value, "data_note"),
        })
    endpoint_bindings = [{
        "binding_key": value["binding_key"],
        "product_key": value["product_key"],
        "model_key": value["model_key"],
        "upstream_model_id": value["upstream_model_id"],
        "upstream_identity_role": optional(value, "upstream_identity_role"),
        "lifecycle": optional(value, "lifecycle"),
        "replaced_by_upstream_id": optional(value, "replaced_by_upstream_id"),
        "interface_candidates": value["interface_candidates"],
        "availability": value["availability"],
        "protocol_qualification": value["protocol_qualification"],
        "evidence_refs": value["evidence_refs"],
    } for value in catalog["endpoint_bindings"]]
    dynamic_routes = [copy.deepcopy(value) for value in catalog["dynamic_routes"]]
    provider_records = []
    for value in catalog["provider_metadata_records"]:
        provider_records.append({key: copy.deepcopy(value.get(key, [])) for key in (
            "provider_record_key", "provenance_refs", "provider_id", "display_name",
            "description_candidates", "aliases", "base_url_candidates", "protocol_candidates",
            "unsupported_api_styles", "unsupported_transport_locators",
            "authentication_candidates", "environment_variable_names", "default_model_ids",
            "discovery_modes", "models_url_candidates", "signup_url_candidates",
            "metadata_completeness", "usable_for",
        )})
        field_provenance = value.get("field_provenance", {})
        if field_provenance:
            provider_records[-1]["field_provenance"] = copy.deepcopy(field_provenance)
        completeness = value.get("metadata_completeness")
        if completeness is not None:
            provider_records[-1]["metadata_completeness"] = {
                domain: project_state(state) for domain, state in completeness.items()
            }
    model_records = []
    for value in catalog["model_metadata_records"]:
        if "execution_fit" not in value or "cost_hint_state" not in value:
            raise ValueError(
                f"metadata record is not closed to a determinate outcome: {value['model_record_key']}"
            )
        record = {key: copy.deepcopy(value[key]) for key in (
            "model_record_key", "provider_record_key", "provenance_refs", "provider_id",
            "upstream_model_id", "display_name", "context_tokens", "max_output_tokens",
            "input_modalities", "capability_hints", "reasoning_rendering_hints",
            "lifecycle", "status_candidates", "replacement_upstream_ids", "roles",
            "normalized_model_matches", "metadata_completeness", "usable_for",
            "execution_fit", "cost_hint_state",
        )}
        for limit in ("context_tokens", "max_output_tokens"):
            record[limit]["state"] = project_state(record[limit]["state"])
        record["capability_hints"] = {
            name: project_state(state)
            for name, state in record["capability_hints"].items()
        }
        record["metadata_completeness"] = {
            domain: project_state(state)
            for domain, state in record["metadata_completeness"].items()
        }
        record["execution_fit"] = {
            key: project_state(state) if key == "state" else state
            for key, state in record["execution_fit"].items()
        }
        record["cost_hint_state"] = project_state(record["cost_hint_state"])
        record["cost_hints"] = project_cost_hint(value.get("cost_hints", []))
        rendering_state = value.get("reasoning_rendering_state")
        if rendering_state is not None:
            record["reasoning_rendering_state"] = project_state(rendering_state)
        field_provenance = value.get("field_provenance", {})
        if field_provenance:
            record["field_provenance"] = copy.deepcopy(field_provenance)
        model_records.append(record)

    projection = {
        "schema": "hiroute.model-metadata-catalog/v1",
        "as_of": catalog["as_of"],
        "source_catalog_digest": source_digest,
        "evidence_sources": evidence_sources,
        "client_runs": runs,
        "access_products": access_products,
        "canonical_models": canonical_models,
        "endpoint_bindings": endpoint_bindings,
        "dynamic_routes": dynamic_routes,
        "inference_rules": [{
            "rule_key": rule["rule_key"],
            "record_kind": rule["record_kind"],
            "basis": rule["basis"],
            "reason": rule["reason"],
            "evidence_refs": rule["evidence_refs"],
            "collected_on": rule["collected_on"],
        } for rule in catalog["inference_rules"]],
        "provider_records": provider_records,
        "model_records": model_records,
    }
    for field, key in (
        ("evidence_sources", "source_key"), ("client_runs", "source_key"),
        ("access_products", "product_key"), ("canonical_models", "model_key"),
        ("endpoint_bindings", "binding_key"), ("dynamic_routes", "route_key"),
        ("inference_rules", "rule_key"),
        ("provider_records", "provider_record_key"), ("model_records", "model_record_key"),
    ):
        projection[field].sort(key=lambda value: value[key])
    return projection

def keyed(values, field):
    result = {}
    for value in values:
        identity = value[field]
        if identity in result:
            raise ValueError(f"duplicate {field}: {identity}")
        result[identity] = value
    return result


# The one client-bundled subscription surface a Codex inventory entry can be promoted onto. Every
# field below is already present in the current registry, so a promotion adds model
# identities to an existing endpoint contract instead of inventing a new transport.
CODEX_SUBSCRIPTION_PROVIDER_ID = "openai-codex"
CODEX_SUBSCRIPTION_OFFER_ID = "offer.codex.subscription"
CODEX_SUBSCRIPTION_CAPABILITY_FIELDS = {
    "connector_id": "connector.cpa.codex",
    "connector_revision": 1,
    "endpoint_profile_id": "endpoint.cpa.codex",
    "endpoint_profile_revision": 1,
    "protocol_endpoint_id": "endpoint.cpa.codex.responses",
    "upstream_protocol": "responses",
    "required_adapter_ref": "adapter.openai-responses.v1",
    "required_adapter_revision": 1,
}
CODEX_SUBSCRIPTION_OFFER_FIELDS = {
    "offer_id": CODEX_SUBSCRIPTION_OFFER_ID,
    "endpoint_profile_id": "endpoint.cpa.codex",
    "endpoint_profile_revision": 1,
    "service_offering_id": "codex-subscription",
    "entitlement_id": "codex-subscription",
    "usage_scope": "account",
    "region_id": "global",
    "billing_class": "subscription",
}


def codex_subscription_intent_facts(upstream_id, model, metadata_catalog):
    """Re-derive the promoted facts from the closed provider-scoped metadata record.

    A promotion is only allowed when the maintained record is determinate, active, and
    agrees with the model definition bundled into the client; the returned facts are what
    the capability digest commits to.
    """
    records = [
        record for record in metadata_catalog["model_metadata_records"]
        if record["provider_id"] == CODEX_SUBSCRIPTION_PROVIDER_ID
        and record["upstream_model_id"] == upstream_id
    ]
    if len(records) != 1:
        raise ValueError(f"no unique Codex subscription metadata record for {upstream_id}")
    record = records[0]
    if record.get("execution_fit", {}).get("state") != "native-text-representable":
        raise ValueError(f"metadata execution fit forbids a text promotion: {upstream_id}")
    if record["lifecycle"] != "active":
        raise ValueError(f"metadata lifecycle is not active: {upstream_id}")
    limits = {}
    for field in ("context_tokens", "max_output_tokens"):
        fact = record[field]
        if fact.get("state") != "known":
            raise ValueError(f"metadata {field} is not a known limit: {upstream_id}")
        limits[field] = fact["value"]
    if limits["context_tokens"] != model["capabilities"]["context_tokens"] \
            or limits["max_output_tokens"] != model["capabilities"]["max_output_tokens"]:
        raise ValueError(f"intent limits disagree with the closed metadata: {upstream_id}")
    hints = record["capability_hints"]
    for capability, field in (("tool", "tool"), ("streaming", "streaming"), ("vision", "vision")):
        state = hints[capability]
        if state not in ("supported", "unsupported"):
            raise ValueError(f"metadata {capability} is not determinate: {upstream_id}")
        if model["capabilities"][field] != (state == "supported"):
            raise ValueError(f"intent {field} disagrees with the closed metadata: {upstream_id}")
    efforts = record["reasoning_rendering_hints"].get("supported_reasoning_efforts", [])
    if not efforts:
        raise ValueError(f"metadata has no renderable reasoning profiles: {upstream_id}")
    return {
        "model_record_key": record["model_record_key"],
        "upstream_model_id": upstream_id,
        "context_tokens": limits["context_tokens"],
        "max_output_tokens": limits["max_output_tokens"],
        "tool": model["capabilities"]["tool"],
        "vision": model["capabilities"]["vision"],
        "streaming": model["capabilities"]["streaming"],
        "supported_reasoning_efforts": sorted(set(efforts)),
        "provenance_refs": [dict(reference) for reference in record["provenance_refs"]],
    }


def codex_subscription_intent_capability(identity, reference, model, metadata_catalog):
    intent = reference.get("catalog_intent", {})
    capabilities = intent.get("endpoint_capabilities", [])
    if len(capabilities) != 1:
        raise ValueError(f"Codex subscription intent needs exactly one capability: {identity}")
    capability = copy.deepcopy(capabilities[0])
    if capability.get("model_configuration_id") != identity:
        raise ValueError("catalog capability crosses reference model identity")
    for key, expected in CODEX_SUBSCRIPTION_CAPABILITY_FIELDS.items():
        if key not in capability or capability[key] != expected:
            raise ValueError(f"catalog capability deviates at {key}: {identity}")
    required = {
        "capability_id", "revision", "model_configuration_id", "connector_id",
        "connector_revision", "endpoint_profile_id", "endpoint_profile_revision",
        "protocol_endpoint_id", "upstream_protocol", "upstream_model_id",
        "required_adapter_ref", "required_adapter_revision", "evidence_digest",
    }
    if set(capability) != required:
        raise ValueError(f"catalog capability shape is not the bundled contract: {identity}")
    facts = codex_subscription_intent_facts(capability["upstream_model_id"], model, metadata_catalog)
    native = reference["native_capability"]
    if native["kind"] != "discrete" or native["parameter"] != "reasoning_effort":
        raise ValueError(f"Codex subscription promotion needs the reasoning_effort contract: {identity}")
    if sorted(set(native["profiles"])) != facts["supported_reasoning_efforts"]:
        raise ValueError(f"native profiles are not the maintained rendering set: {identity}")
    capability["evidence_digest"] = digest(
        ["hiroute.codex-subscription-intent/v1", "endpoint-capability",
         capability["capability_id"], facts]
    )
    return capability


def codex_subscription_intent_offer(identity, reference):
    intent = reference.get("catalog_intent", {})
    offers = intent.get("offers", [])
    if len(offers) != 1:
        raise ValueError(f"Codex subscription intent needs exactly one offer: {identity}")
    offer = copy.deepcopy(offers[0])
    if offer.get("offer_id") != CODEX_SUBSCRIPTION_OFFER_ID:
        raise ValueError(f"catalog offer is not the bundled Codex subscription: {identity}")
    if offer.get("model_configuration_ids") != [identity]:
        raise ValueError("catalog offer must join exactly the reference model identity")
    for key, expected in CODEX_SUBSCRIPTION_OFFER_FIELDS.items():
        if key not in offer or offer[key] != expected:
            raise ValueError(f"catalog offer deviates at {key}: {identity}")
    required = {
        "offer_id", "revision", "endpoint_profile_id", "endpoint_profile_revision",
        "service_offering_id", "entitlement_id", "usage_scope", "region_id",
        "model_configuration_ids", "billing_class", "evidence_digest",
    }
    if set(offer) != required:
        raise ValueError(f"catalog offer shape is not the bundled contract: {identity}")
    offer["evidence_digest"] = digest(
        ["hiroute.codex-subscription-intent/v1", "offer", offer["offer_id"],
         sorted(offer["model_configuration_ids"])]
    )
    return offer


def merge_catalog_intent_offer(offers, offer):
    existing = next((value for value in offers if value["offer_id"] == offer["offer_id"]), None)
    if existing is None:
        offers.append(copy.deepcopy(offer))
        return
    for field in (
        "endpoint_profile_id", "endpoint_profile_revision", "service_offering_id",
        "entitlement_id", "usage_scope", "region_id", "billing_class",
    ):
        if existing[field] != offer[field]:
            raise ValueError(f"catalog intent offer conflicts with the preserved offer: {offer['offer_id']}")
    existing["model_configuration_ids"] = sorted(set(
        existing["model_configuration_ids"] + offer["model_configuration_ids"]
    ))
    existing["evidence_digest"] = digest([
        existing["evidence_digest"], offer["evidence_digest"]
    ])

def joined_request_path(interface):
    base = interface.get("base_url")
    if not base or interface.get("base_url_template"):
        raise ValueError(f"runtime interface needs an exact base URL: {interface['interface_key']}")
    parsed = urlsplit(base)
    if parsed.scheme != "https" or not parsed.netloc or parsed.query or parsed.fragment:
        raise ValueError(f"invalid runtime base URL: {interface['interface_key']}")
    request = interface.get("request_path")
    if not request or interface.get("request_path_template"):
        raise ValueError(f"runtime interface needs an exact request path: {interface['interface_key']}")
    return f"{parsed.scheme}://{parsed.netloc}", parsed.path.rstrip("/") + request

def runtime_model_definition(model, model_configuration_id, publisher_id):
    context_state = model.get("context_state", "known" if model.get("context_tokens") else "unknown")
    output_state = model.get("max_output_state", "known" if model.get("max_output_tokens") else "unknown")
    if context_state != "known" or output_state != "known":
        raise ValueError(f"runtime model has conditional or unknown limits: {model['model_key']}")
    capability_states = model["capabilities"]
    tool = capability_states.get("tool", "unknown")
    streaming = capability_states.get("streaming", "unknown")
    vision = capability_states.get("vision", model["modalities"].get("image_input", "unknown"))
    if any(value not in ("supported", "unsupported") for value in (tool, streaming, vision)):
        raise ValueError(f"runtime model has unknown boolean capability: {model['model_key']}")
    return {
        "model_configuration_id": model_configuration_id,
        "revision": 1,
        "display_name": model["display_name"],
        "publisher_id": publisher_id,
        "capabilities": {
            "tool": tool == "supported",
            "vision": vision == "supported",
            "streaming": streaming == "supported",
            "context_tokens": model["context_tokens"],
            "max_output_tokens": model["max_output_tokens"],
        },
    }

def reasoning_configurations(capability):
    kind = capability["kind"]
    if kind == "fixed":
        return [{"kind": "fixed", "profile": capability["profile"]}]
    if kind == "toggle":
        return [{"kind": "toggle", "enabled": value} for value in (False, True)]
    if kind == "discrete":
        return [{"kind": "profile", "profile": value} for value in capability["profiles"]]
    if kind == "budget":
        return [{"kind": "budget", "tokens": value} for value in range(
            capability["minimum_tokens"], capability["maximum_tokens"] + 1,
            capability["step_tokens"]
        )]
    raise ValueError(f"unsupported native reasoning kind: {kind}")

def build_runtime_projection(catalog):
    products = keyed(catalog["access_products"], "product_key")
    models = keyed(catalog["models"], "model_key")
    evidence = keyed(catalog["evidence_sources"], "source_key")
    endpoint_bindings = catalog["endpoint_bindings"]
    protocol = {
        "openai-chat": ("chat_completions", "adapter.openai-chat.v1"),
        "openai-responses": ("responses", "adapter.openai-responses.v1"),
        "anthropic-messages": ("messages", "adapter.anthropic-messages.v1"),
    }
    registrations = [
        {
            "connector_id": "connector.openai.p0", "implementation_ref": "builtin/openai",
            "product_key": "openai-platform-global", "endpoint_profile_id": "endpoint.openai.platform.global.v1",
            "provider_platform_id": "openai", "service_offering_id": "platform-api", "entitlement_id": "api-key",
            "usage_scope": "account", "region_id": "global", "logical_endpoint_group": "openai-platform",
            "connection_option_id": "openai.platform.global.v1", "display_name": "OpenAI Platform API",
            "billing_class": "paid", "inventory_interface": "openai-platform-global/openai-responses",
            "inventory_path": "/v1/models", "authentication": {"kind": "bearer"},
            "interfaces": [
                ("openai-platform-global/openai-responses", "responses", 0),
                ("openai-platform-global/openai-chat", "chat", 1),
            ],
            "models": [
                ("gpt-5-3-codex", "model.openai.gpt-5.3-codex", "gpt-5.3-codex", "publisher.openai",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["low", "medium", "high", "xhigh"]}, None),
                ("gpt-5-6-sol", "model.openai.gpt-5.6-sol", "gpt-5.6-sol", "publisher.openai",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["none", "low", "medium", "high", "xhigh", "max"]}, None),
                ("gpt-6-astra", "model.openai.gpt-6-astra", "gpt-6-astra", "publisher.openai",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["low", "medium", "high", "xhigh", "max"]}, None),
            ],
        },
        {
            "connector_id": "connector.anthropic.p0", "implementation_ref": "builtin/anthropic",
            "product_key": "anthropic-platform", "endpoint_profile_id": "endpoint.anthropic.platform.global.v1",
            "provider_platform_id": "anthropic", "service_offering_id": "claude-api", "entitlement_id": "api-key",
            "usage_scope": "workspace", "region_id": "global", "logical_endpoint_group": "anthropic-platform",
            "connection_option_id": "anthropic.platform.global.v1", "display_name": "Anthropic Claude API",
            "billing_class": "paid", "inventory_interface": "anthropic-platform/anthropic-messages",
            "inventory_path": "/v1/models", "authentication": {"kind": "api_key_header", "header": "x-api-key"},
            "interfaces": [("anthropic-platform/anthropic-messages", "messages", 0)],
            "models": [
                ("claude-opus-5", "model.anthropic.claude-opus-5", "claude-opus-5", "publisher.anthropic",
                    {"kind": "discrete", "parameter": "output_config.effort", "profiles": ["low", "medium", "high", "xhigh", "max"]},
                    "claude_adaptive_effort_messages"),
            ],
        },
        {
            "connector_id": "connector.google-gemini.p0", "implementation_ref": "builtin/google-gemini-openai",
            "product_key": "gemini-api", "endpoint_profile_id": "endpoint.google.gemini-api.global.v1",
            "provider_platform_id": "google", "service_offering_id": "gemini-api-paid", "entitlement_id": "api-key",
            "usage_scope": "project", "region_id": "global", "logical_endpoint_group": "google-gemini-api",
            "connection_option_id": "google.gemini-api-paid.global.v1", "display_name": "Google Gemini API Paid Tier",
            "billing_class": "paid", "inventory_interface": "gemini-api/openai-chat",
            "inventory_path": "/v1beta/openai/models", "authentication": {"kind": "bearer"},
            "interfaces": [("gemini-api/openai-chat", "chat", 0)],
            "models": [
                ("gemini-3-1-pro-preview", "model.google.gemini-3.1-pro-preview", "gemini-3.1-pro-preview", "publisher.google",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["low", "medium", "high"]}, None),
                ("gemini-3-8-flash", "model.google.gemini-3.8-flash", "gemini-3.8-flash", "publisher.google",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["low", "medium", "high"]}, None),
            ],
        },
        {
            "connector_id": "connector.deepseek.p0", "implementation_ref": "builtin/deepseek",
            "product_key": "deepseek-platform", "endpoint_profile_id": "endpoint.deepseek.official.global.v1",
            "provider_platform_id": "deepseek", "service_offering_id": "official-api", "entitlement_id": "api-key",
            "usage_scope": "account", "region_id": "global", "logical_endpoint_group": "deepseek-official",
            "connection_option_id": "deepseek.official.global.v1", "display_name": "DeepSeek Official API",
            "billing_class": "paid", "inventory_interface": "deepseek-platform/openai-responses",
            "inventory_path": "/models", "authentication": {"kind": "bearer"},
            "interfaces": [
                ("deepseek-platform/openai-responses", "responses", 0),
                ("deepseek-platform/openai-chat", "chat", 1),
                ("deepseek-platform/anthropic-messages", "messages", 2),
            ],
            "models": [
                ("deepseek-v4-1-flash", "model.deepseek.v4-1-flash", "deepseek-flash", "publisher.deepseek",
                    {"kind": "fixed", "profile": "provider-default"}, None),
                ("deepseek-v4-pro-0813", "model.deepseek.v4-pro-0813", "deepseek-v4-pro", "publisher.deepseek",
                    {"kind": "fixed", "profile": "provider-default"}, None),
            ],
        },
        {
            "connector_id": "connector.bailian.p0", "implementation_ref": "builtin/bailian",
            "product_key": "bailian-payg-cn", "endpoint_profile_id": "endpoint.bailian.payg.cn.v1",
            "provider_platform_id": "bailian", "service_offering_id": "model-studio", "entitlement_id": "payg-api-key",
            "usage_scope": "workspace", "region_id": "cn", "logical_endpoint_group": "bailian-payg",
            "connection_option_id": "bailian.payg.cn.v1", "display_name": "Bailian Pay As You Go China",
            "billing_class": "paid", "inventory_interface": "bailian-payg-cn/openai-chat",
            "inventory_path": "/api/v1/models", "authentication": {"kind": "bearer"},
            "interfaces": [
                ("bailian-payg-cn/openai-chat", "chat", 0),
                ("bailian-payg-cn/openai-responses", "responses", 1),
                ("bailian-payg-cn/anthropic-messages", "messages", 2),
            ],
            "models": [
                ("qwen-3-8-max", "model.bailian.qwen3.8-max", "qwen3.8-max", "publisher.alibaba-qwen",
                    {"kind": "discrete", "parameter": "reasoning_effort", "profiles": ["low", "medium", "xhigh"]}, None),
            ],
        },
    ]
    for suffix, product_key, billing in [("general", "zai-general-global", "paid"), ("coding-plan", "zai-devpack-global", "subscription")]:
        interfaces = [(f"{product_key}/openai-chat", "chat", 0)]
        if suffix == "coding-plan":
            interfaces.append((f"{product_key}/anthropic-messages", "messages", 1))
        registrations.append({
            "connector_id": "connector.zai.p0", "implementation_ref": "builtin/zai",
            "product_key": product_key, "endpoint_profile_id": f"endpoint.zai.{suffix}.global.v1",
            "provider_platform_id": "zai", "service_offering_id": suffix, "entitlement_id": "api-key",
            "usage_scope": "account", "region_id": "global", "logical_endpoint_group": f"zai-{suffix}",
            "connection_option_id": f"zai.{suffix}.global.v1", "display_name": f"Z.AI {suffix}",
            "billing_class": billing, "inventory_interface": interfaces[0][0], "inventory_path": None,
            "authentication": {"kind": "bearer"}, "interfaces": interfaces, "models": [],
        })
    output_registrations, output_models, capabilities, offers = [], [], [], []
    attempts, gaps = [], []
    qualified_bindings = {}
    preserved_options = {"bailian.payg.cn.v1"}
    for registration in registrations:
        model_attempts = []
        qualification_failures = []
        for model_key, _, upstream_id, _, _, _ in registration["models"]:
            matches = [value for value in endpoint_bindings
                if value["product_key"] == registration["product_key"]
                and value["model_key"] == model_key
                and value["upstream_model_id"] == upstream_id
                and registration["inventory_interface"] in value["interface_candidates"]]
            if len(matches) != 1:
                model_attempts.append({
                    "model_key": model_key,
                    "upstream_model_id": upstream_id,
                    "outcome": "gap",
                    "reason": "missing unique product/model/interface endpoint binding",
                })
                qualification_failures.append("missing exact endpoint binding")
                continue
            binding = matches[0]
            availability = binding["availability"]["state"]
            protocol_qualification = binding["protocol_qualification"]
            if availability != "available" or protocol_qualification != "qualified":
                reason = (f"binding is availability={availability}, "
                    f"protocol_qualification={protocol_qualification}")
                model_attempts.append({
                    "model_key": model_key,
                    "upstream_model_id": upstream_id,
                    "binding_key": binding["binding_key"],
                    "outcome": "gap",
                    "reason": reason,
                })
                qualification_failures.append(reason)
                continue
            qualified_bindings[(model_key, upstream_id)] = binding
            model_attempts.append({
                "model_key": model_key,
                "upstream_model_id": upstream_id,
                "binding_key": binding["binding_key"],
                "outcome": "qualified",
            })
        preserved = registration["connection_option_id"] in preserved_options
        attempts.append({
            "connection_option_id": registration["connection_option_id"],
            "product_key": registration["product_key"],
            "outcome": "preserved" if preserved else "endpoint_qualified",
            "models": model_attempts,
        })
        if qualification_failures:
            gaps.append({
                "scope": f"{registration['connection_option_id']}/known-model-projection",
                "reason": ("known model not promoted; authenticated inventory remains eligible "
                    "for the conservative runtime fallback: "
                    + "; ".join(sorted(set(qualification_failures)))),
            })
        if preserved:
            continue
        product = products[registration["product_key"]]
        interfaces = keyed(product["interfaces"], "interface_key")
        profile_endpoints = []
        endpoint_by_interface = {}
        for interface_key, suffix, preference in registration["interfaces"]:
            interface = interfaces[interface_key]
            if interface["protocol"] not in protocol:
                raise ValueError(f"no runtime adapter for {interface['protocol']}")
            upstream_protocol, adapter = protocol[interface["protocol"]]
            base_url, request_path = joined_request_path(interface)
            endpoint_id = f"{registration['endpoint_profile_id']}.{suffix}"
            endpoint = {
                "protocol_endpoint_id": endpoint_id,
                "protocol": upstream_protocol,
                "base_url": base_url,
                "request_path": request_path,
                "adapter_ref": adapter,
                "adapter_revision": 1,
                "stable_preference": preference,
                "authentication_semantics": registration["authentication"],
            }
            if upstream_protocol == "messages":
                endpoint["required_headers"] = [["anthropic-version", "2023-06-01"]]
            if interface_key == registration["inventory_interface"] and registration["inventory_path"]:
                endpoint["inventory_path"] = registration["inventory_path"]
            profile_endpoints.append(endpoint)
            endpoint_by_interface[interface_key] = endpoint
        inventory = endpoint_by_interface[registration["inventory_interface"]]
        evidence_refs = sorted(set(product["evidence_refs"]))
        evidence_digest = digest([evidence_ref for evidence_ref in evidence_refs])
        verified_at = int(datetime.datetime.strptime(catalog["as_of"], "%Y-%m-%d").replace(
            tzinfo=datetime.timezone.utc).timestamp())
        profile = {
            "endpoint_profile_id": registration["endpoint_profile_id"], "revision": 1,
            "connector_id": registration["connector_id"], "connector_revision": 1,
            "provider_platform_id": registration["provider_platform_id"],
            "service_offering_id": registration["service_offering_id"],
            "entitlement_id": registration["entitlement_id"], "usage_scope": registration["usage_scope"],
            "region_id": registration["region_id"], "logical_endpoint_group": registration["logical_endpoint_group"],
            "protocol_endpoints": profile_endpoints, "inventory_strategy": "remote_models" if registration["inventory_path"] else "bundled_catalog",
            **({"inventory_protocol_endpoint_id": inventory["protocol_endpoint_id"]} if registration["inventory_path"] else {}),
            "verification_evidence": evidence_digest, "last_verified_at": verified_at,
        }
        option = {
            "connection_option_id": registration["connection_option_id"],
            "display_name": registration["display_name"], "origin": "native_api",
            "connector_id": registration["connector_id"], "connector_revision": 1,
            "endpoint_profile_id": registration["endpoint_profile_id"], "endpoint_profile_revision": 1,
            "billing_class": registration["billing_class"],
        }
        output_registrations.append({
            "connector_id": registration["connector_id"],
            "implementation_ref": registration["implementation_ref"],
            "endpoint_profile": profile, "connection_option": option,
        })
        model_ids = []
        for model_key, configuration_id, upstream_id, publisher_id, native, convention in registration["models"]:
            source_model = models[model_key]
            binding = qualified_bindings.get((model_key, upstream_id))
            if binding is None:
                continue
            product_authorities = {
                evidence[reference]["authority"] for reference in product["evidence_refs"]
            }
            model_authorities = {
                evidence[reference]["authority"] for reference in source_model["evidence_refs"]
            }
            # Provenance is used only to fence identity here. Executability is gated separately
            # by the explicit endpoint, authentication, adapter, model ID, limits, and capability
            # checks below; an authority label alone never promotes a record.
            if product_authorities.isdisjoint(model_authorities):
                raise ValueError(f"runtime product/model identity is not source-coherent: {model_key}")
            if upstream_id not in source_model["upstream_ids"]:
                raise ValueError(f"runtime upstream ID is not source-scoped: {upstream_id}")
            if native["kind"] == "discrete" and not set(native["profiles"]).issubset(
                set(source_model["reasoning"].get("profiles", []))
            ):
                raise ValueError(f"runtime reasoning profiles exceed source metadata: {model_key}")
            definition = runtime_model_definition(source_model, configuration_id, publisher_id)
            native_model = {"model_configuration_id": configuration_id, "capability": native}
            if convention:
                native_model["native_render_convention"] = convention
            output_models.append({
                "source_model_key": model_key, "model": definition, "native_reasoning": native_model,
                "sources": [evidence[ref]["locator"] for ref in source_model["evidence_refs"]],
            })
            capability_evidence = digest([product["product_key"], product["evidence_refs"], model_key,
                source_model["evidence_refs"], binding["binding_key"], binding["evidence_refs"],
                registration["inventory_interface"]])
            capabilities.append({
                "capability_id": f"cap.runtime.{configuration_id.removeprefix('model.')}.{registration['connection_option_id']}",
                "revision": 1, "model_configuration_id": configuration_id,
                "connector_id": registration["connector_id"], "connector_revision": 1,
                "endpoint_profile_id": registration["endpoint_profile_id"], "endpoint_profile_revision": 1,
                "protocol_endpoint_id": inventory["protocol_endpoint_id"],
                "upstream_protocol": inventory["protocol"], "upstream_model_id": upstream_id,
                "required_adapter_ref": inventory["adapter_ref"], "required_adapter_revision": 1,
                "evidence_digest": capability_evidence,
            })
            model_ids.append(configuration_id)
        if model_ids:
            offers.append({
                "offer_id": f"offer.{registration['connection_option_id'].removesuffix('.v1')}", "revision": 1,
                "endpoint_profile_id": registration["endpoint_profile_id"], "endpoint_profile_revision": 1,
                "service_offering_id": registration["service_offering_id"],
                "entitlement_id": registration["entitlement_id"], "usage_scope": registration["usage_scope"],
                "region_id": registration["region_id"], "model_configuration_ids": model_ids,
                "billing_class": registration["billing_class"], "evidence_digest": evidence_digest,
            })
    gaps.extend([
        {"scope": "bailian-coding-cn", "reason": "no complete current model/product binding with executable limits"},
        {"scope": "bailian-token-personal-cn-beijing", "reason": "no complete current model/product binding with executable limits"},
        {"scope": "bailian-token-team-cn-beijing", "reason": "no complete current model/product binding with executable limits"},
        {"scope": "kimi-code-subscription", "reason": "model output limit remains entitlement-dependent"},
        {"scope": "kimi-platform-cn", "reason": "current documented model output limit is unknown"},
        {"scope": "kimi-platform-global", "reason": "current documented model output limit is unknown or entitlement-dependent"},
        {"scope": "zhipu-coding-cn", "reason": "no authenticated remote model-directory contract in the maintained facts"},
        {"scope": "zhipu-general-cn", "reason": "no authenticated remote model-directory contract in the maintained facts"},
        {"scope": "cloud-auth-transports", "reason": "OAuth, ADC, AWS SDK, and workload identity require dedicated credential adapters"},
    ])
    templates = catalog["connection_templates"]
    template_ids = [value["connection_option_id"] for value in templates]
    if len(set(template_ids)) != len(template_ids):
        raise ValueError("duplicate connection template")
    for template in templates:
        if not template["provider_id"] or any(not template[field].get(lang, "").strip()
                for field in ("name", "description") for lang in ("zh", "en")):
            raise ValueError("connection template needs readable bilingual identity")
        if not template["documentation_url"].startswith("https://"):
            raise ValueError("connection template needs a source URL")
        for endpoint in template["endpoint_defaults"]:
            authentication = endpoint["authentication_semantics"]
            if authentication["kind"] not in ("none", "bearer", "api_key_header"):
                raise ValueError("unsupported template authentication")
            if authentication["kind"] == "api_key_header" and not authentication.get("header"):
                raise ValueError("template authentication needs an explicit header")
    return {
        "schema": "hiroute.runtime-metadata-projection/v1",
        "connection_templates": catalog["connection_templates"],
        "source_catalog_digest": digest(catalog),
        "attempts": sorted(attempts, key=lambda value: value["connection_option_id"]),
        "preserved_connection_options": sorted(preserved_options),
        "registrations": sorted(output_registrations, key=lambda value: value["connection_option"]["connection_option_id"]),
        "models": sorted(output_models, key=lambda value: value["model"]["model_configuration_id"]),
        "model_endpoint_capabilities": sorted(capabilities, key=lambda value: value["capability_id"]),
        "offers": sorted(offers, key=lambda value: value["offer_id"]),
        "gaps": gaps,
    }

def add_documented_zhipu_responses_capability(projection, catalog, data):
    """Keep the old Messages binding while enabling qualified new Coding Plan sources."""
    product = next(value for value in catalog["access_products"]
        if value["product_key"] == "zhipu-coding-cn")
    interface = next(value for value in product["interfaces"]
        if value["interface_key"] == "zhipu-coding-cn/openai-responses")
    binding = next(value for value in catalog["endpoint_bindings"]
        if value["binding_key"] == "zhipu-coding-cn/glm-5-3-global/glm-5-3")
    source = next(value for value in catalog["evidence_sources"]
        if value["source_key"] == "source-125")
    if (interface["protocol"] != "openai-responses"
        or joined_request_path(interface) != ("https://open.bigmodel.cn", "/api/v1/responses")
        or binding["product_key"] != product["product_key"]
        or binding["upstream_model_id"] != "glm-5.3"
        or interface["interface_key"] not in binding["interface_candidates"]
        or binding["protocol_qualification"] != "runtime-required"
        or source["locator"] != "https://docs.bigmodel.cn/cn/coding-plan/tool/codex"
        or source["source_key"] not in product["evidence_refs"]
        or source["source_key"] not in binding["evidence_refs"]):
        raise ValueError("the documented Zhipu Coding Plan Responses binding changed")
    model = next(value for value in data["models"]
        if value["model_configuration_id"] == "model.zhipu.glm-5.3")
    messages = next(value for value in data["model_endpoint_capabilities"]
        if value["capability_id"] == "cap.zhipu.glm-5.3.coding-plan.messages")
    if (not model["capabilities"]["tool"] or not model["capabilities"]["streaming"]
        or messages["endpoint_profile_id"] != "endpoint.zhipu.coding-plan.cn.v1"
        or messages["upstream_protocol"] != "messages"
        or messages["upstream_model_id"] != binding["upstream_model_id"]):
        raise ValueError("the preserved Zhipu model and Messages capability changed")
    responses = copy.deepcopy(messages)
    responses.update({
        "capability_id": "cap.zhipu.glm-5.3.coding-plan.responses",
        "protocol_endpoint_id": "endpoint.zhipu.coding-plan.cn.v1.responses",
        "upstream_protocol": "responses",
        "required_adapter_ref": "adapter.openai-responses.v1",
        "evidence_digest": digest([product["evidence_refs"], binding["evidence_refs"],
            source["locator"], messages["evidence_digest"]]),
    })
    projection["model_endpoint_capabilities"].append(responses)
    projection["model_endpoint_capabilities"].sort(key=lambda value: value["capability_id"])

def compile_candidate():
    maintenance = json.loads((HERE / "maintenance.json").read_text())
    if maintenance.get("schema") != "hiroute.native-rating-maintenance/v1":
        raise ValueError("unsupported current maintenance contract")
    old = pinned_json(maintenance["source_bundle"], maintenance["source_bytes_digest"])
    if old.get("schema") != "hiroute.rating-seed/v1":
        raise ValueError("current rating seed has an unsupported contract")
    metadata_catalog = pinned_json(
        maintenance["metadata_catalog"], maintenance["metadata_catalog_bytes_digest"]
    )
    client_runs = pinned_json(
        maintenance["client_discovery_runs"], maintenance["client_discovery_runs_bytes_digest"]
    )
    metadata = {r["model_configuration_id"]: r for r in old["rating_snapshot"]["ratings"]}
    ratings = {r["model_configuration_id"]: r for r in old["data"]["ratings"]}
    models = sorted(old["native_reasoning"], key=lambda n: n["model_configuration_id"])
    records, coverage = [], []
    unknown = {"state": "unknown", "reason": "rating_not_collected"}
    seen = set()
    for model in models:
        identity, cap = model["model_configuration_id"], model["capability"]
        if identity in seen:
            raise ValueError("duplicate model identity")
        seen.add(identity)
        effort = metadata[identity]["native_effort"]
        config = None
        if effort not in ("default", "provider-default"):
            if cap["kind"] == "fixed" and cap["profile"] == effort:
                config = {"kind": "fixed", "profile": effort}
            elif cap["kind"] == "discrete" and effort in cap["profiles"]:
                config = {"kind": "profile", "profile": effort}
        evidence = maintenance["evidence"].get(identity)
        if config is not None:
            if not evidence or not evidence["identity_url"].startswith("https://"):
                raise ValueError("adopted estimate needs explicit provenance")
            score = ratings[identity]["overall_score_tenths"]
            if not 5 <= score <= 50:
                raise ValueError("score out of scale")
            records.append({"model_configuration_id": identity, "native_configuration": config,
                "overall": {"state": "estimated", "score_tenths": score,
                    "evidence_ref": evidence["id"], "method_revision": maintenance["method_revision"]},
                "coding": unknown, "tool": unknown})
        coverage.append({"model_configuration_id": identity, "native_capability": cap,
            "adopted_configuration": config,
            "overall": "estimated_preserved_explicit_configuration" if config else "unknown_preserved_default_not_resolved",
            "coding": "unknown_no_per_dimension_measurement_provenance",
            "tool": "unknown_no_per_dimension_measurement_provenance",
            "other_configurations": "unknown_not_collected"})
    data = copy.deepcopy(old["data"])
    for reference in maintenance.get("reference_models", []):
        identity = reference["model"]["model_configuration_id"]
        if identity in seen or not reference["sources"]:
            raise ValueError("duplicate or unsourced reference model")
        seen.add(identity)
        data["models"].append(reference["model"])
        native = {"model_configuration_id": identity, "capability": reference["native_capability"]}
        models.append(native)
        cap = native["capability"]
        convention = reference.get("native_render_convention")
        if convention is not None:
            if convention != "claude_adaptive_effort_messages" or cap["kind"] != "discrete" or cap["parameter"] != "output_config.effort":
                raise ValueError("invalid fixed native render convention")
            native["native_render_convention"] = convention
        configurations = reasoning_configurations(cap)
        for config in configurations:
            records.append({"model_configuration_id": identity, "native_configuration": config,
                "overall": unknown, "coding": unknown, "tool": unknown})
        intent = reference.get("catalog_intent", {})
        for capability in intent.get("endpoint_capabilities", []):
            if capability["model_configuration_id"] != identity:
                raise ValueError("catalog capability crosses reference model identity")
            if capability["connector_id"] == CODEX_SUBSCRIPTION_CAPABILITY_FIELDS["connector_id"]:
                capability = codex_subscription_intent_capability(
                    identity, reference, reference["model"], metadata_catalog
                )
            else:
                capability = copy.deepcopy(capability)
            existing_capabilities = {value["capability_id"] for value in data["model_endpoint_capabilities"]}
            if capability["capability_id"] in existing_capabilities:
                raise ValueError(f"catalog intent duplicates capability: {capability['capability_id']}")
            data["model_endpoint_capabilities"].append(capability)
        for offer in intent.get("offers", []):
            if identity not in offer["model_configuration_ids"]:
                raise ValueError("catalog offer omits reference model identity")
            if offer["offer_id"] == CODEX_SUBSCRIPTION_OFFER_ID:
                merge_catalog_intent_offer(
                    data["offers"], codex_subscription_intent_offer(identity, reference)
                )
            else:
                data["offers"].append(copy.deepcopy(offer))
        for rate in intent.get("price_rates", []):
            if rate["model_configuration_id"] != identity:
                raise ValueError("catalog price crosses reference model identity")
            data["price_rates"].append(copy.deepcopy(rate))
        coverage.append({"model_configuration_id": identity, "native_capability": cap,
            "configurations": configurations, "overall": "unknown_no_common_scale_measurement",
            "coding": "unknown_no_common_scale_measurement", "tool": "unknown_no_common_scale_measurement",
            "price": ("catalog_offer_without_flat_price" if intent.get("offers")
                and not intent.get("price_rates") else "unknown_no_verified_offer_mapping"),
            "sources": reference["sources"],
            "identity_note": reference["identity_note"],
            "native_render_convention": convention})
    runtime_projection = build_runtime_projection(metadata_catalog)
    add_documented_zhipu_responses_capability(runtime_projection, metadata_catalog, data)
    native_by_id = {value["model_configuration_id"]: value for value in models}
    model_by_id = {value["model_configuration_id"]: value for value in data["models"]}
    for projected in runtime_projection["models"]:
        model = projected["model"]
        native = projected["native_reasoning"]
        identity = model["model_configuration_id"]
        if identity in model_by_id:
            if model_by_id[identity] != model or native_by_id.get(identity) != native:
                raise ValueError(f"runtime projection conflicts with preserved model: {identity}")
            continue
        if identity in seen:
            raise ValueError(f"runtime projection duplicates model identity: {identity}")
        seen.add(identity)
        model_by_id[identity] = model
        native_by_id[identity] = native
        data["models"].append(copy.deepcopy(model))
        models.append(copy.deepcopy(native))
        configurations = reasoning_configurations(native["capability"])
        for config in configurations:
            records.append({
                "model_configuration_id": identity,
                "native_configuration": config,
                "overall": unknown,
                "coding": unknown,
                "tool": unknown,
            })
        coverage.append({
            "model_configuration_id": identity,
            "native_capability": native["capability"],
            "configurations": configurations,
            "overall": "unknown_no_common_scale_measurement",
            "coding": "unknown_no_common_scale_measurement",
            "tool": "unknown_no_common_scale_measurement",
            "price": "catalog_offer_without_flat_price",
            "sources": projected["sources"],
            "identity_note": f"Derived from client-bundled metadata model {projected['source_model_key']}; endpoint eligibility remains credential-qualified.",
            "native_render_convention": native.get("native_render_convention"),
        })
    existing_capabilities = {value["capability_id"] for value in data["model_endpoint_capabilities"]}
    for capability in runtime_projection["model_endpoint_capabilities"]:
        if capability["capability_id"] in existing_capabilities:
            raise ValueError(f"runtime projection duplicates capability: {capability['capability_id']}")
        existing_capabilities.add(capability["capability_id"])
        data["model_endpoint_capabilities"].append(copy.deepcopy(capability))
    offers_by_id = {value["offer_id"]: value for value in data["offers"]}
    for offer in runtime_projection["offers"]:
        existing = offers_by_id.get(offer["offer_id"])
        if existing is None:
            data["offers"].append(copy.deepcopy(offer))
            offers_by_id[offer["offer_id"]] = data["offers"][-1]
            continue
        for field in (
            "endpoint_profile_id", "endpoint_profile_revision", "service_offering_id",
            "entitlement_id", "usage_scope", "region_id", "billing_class",
        ):
            if existing[field] != offer[field]:
                raise ValueError(f"runtime projection conflicts with preserved offer: {offer['offer_id']}")
        existing["model_configuration_ids"] = sorted(set(
            existing["model_configuration_ids"] + offer["model_configuration_ids"]
        ))
        existing["evidence_digest"] = digest([
            existing["evidence_digest"], offer["evidence_digest"]
        ])
    data["models"].sort(key=lambda m: m["model_configuration_id"])
    data["model_endpoint_capabilities"].sort(key=lambda c: c["capability_id"])
    data["offers"].sort(key=lambda o: o["offer_id"])
    data["price_rates"].sort(key=lambda p: p["price_rate_id"])
    models.sort(key=lambda m: m["model_configuration_id"])
    records.sort(key=lambda r: (r["model_configuration_id"], canonical(r["native_configuration"])))
    coverage.sort(key=lambda r: r["model_configuration_id"])
    data["bundle_version"] = maintenance["snapshot_version"]
    data["product_release"] = CURRENT_PRODUCT_RELEASE
    data["connector_registry_version"] = CURRENT_PRODUCT_RELEASE
    data["models_slice_version"] = maintenance["snapshot_version"]
    data["capability_slice_version"] = maintenance["snapshot_version"]
    data["ratings"] = []
    data["ratings_slice_version"] = maintenance["snapshot_version"]
    data["prices_slice_version"] = maintenance["snapshot_version"]
    data["free_offers_slice_version"] = maintenance["snapshot_version"]
    snapshot = {"schema": "hiroute.rating-snapshot/v2", "version": maintenance["snapshot_version"],
        "scale_version": old["rating_snapshot"]["scale"]["version"],
        "model_catalog_digest": digest(data["models"]), "models": models, "records": records}
    snapshot["digest"] = digest([snapshot[k] for k in
        ("schema", "version", "scale_version", "model_catalog_digest", "models", "records")])
    bundle = {"schema": "hiroute.release-model-data/v2", "data": data,
        "rating_snapshot": snapshot,
        "metadata_catalog": project_metadata_catalog(metadata_catalog, client_runs)}
    payload = canonical(bundle) + b"\n"
    if len(payload) > 2 * 1024 * 1024:
        raise ValueError("candidate exceeds existing release limit")
    return {"model-data.json": payload, "runtime-projection.json": canonical(runtime_projection) + b"\n",
        "coverage.json": canonical({
        "schema": "hiroute.model-coverage/v1", "snapshot_digest": snapshot["digest"],
        "models": coverage, "pending_families": maintenance["pending_families"],
        "release_status": "current_client_bundled"}) + b"\n"}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    changed = 0
    for name, content in compile_candidate().items():
        path = HERE / name
        if path.exists() and path.read_bytes() == content:
            continue
        changed += 1
        if not args.check:
            path.write_bytes(content)
    print(json.dumps({"changed_files": changed, "check": args.check}))
    if args.check and changed:
        raise SystemExit(1)

if __name__ == "__main__":
    main()
