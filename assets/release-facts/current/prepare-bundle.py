#!/usr/bin/env python3
"""Prepare the deterministic current ReleaseFacts bundle shipped with the client."""

import argparse
import datetime
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
BUNDLE = HERE / "bundle"
PRODUCT_RELEASE = "mvp-current"
SEQUENCE = 1
CATALOG_ID = "client-bundled/current"


def compact(value, *, sort_keys=True):
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=sort_keys,
    ).encode()


def digest_bytes(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def digest_value(value):
    return digest_bytes(compact(value))


def native_configuration_key(record):
    configuration = record["native_configuration"]
    rank = {"fixed": 0, "toggle": 1, "profile": 2, "budget": 3}
    kind = configuration["kind"]
    detail = {
        "fixed": configuration.get("profile"),
        "toggle": configuration.get("enabled"),
        "profile": configuration.get("profile"),
        "budget": configuration.get("tokens"),
    }[kind]
    return (
        record["model_configuration_id"],
        rank[kind],
        detail,
    )


def normalize_registry(registry):
    registry["connectors"].sort(key=lambda value: value["connector_id"])
    for connector in registry["connectors"]:
        connector["endpoint_profile_refs"].sort()
        connector["required_secret_slots"].sort()
    registry["endpoint_profiles"].sort(
        key=lambda value: value["endpoint_profile_id"]
    )
    for profile in registry["endpoint_profiles"]:
        profile["protocol_endpoints"].sort(
            key=lambda value: value["protocol_endpoint_id"]
        )
    registry["connection_options"].sort(
        key=lambda value: value["connection_option_id"]
    )


def normalize_model_data(model_data):
    data = model_data["data"]
    data["models"].sort(key=lambda value: value["model_configuration_id"])
    data["model_endpoint_capabilities"].sort(
        key=lambda value: value["capability_id"]
    )
    data["ratings"].sort(key=lambda value: value["model_configuration_id"])
    data["offers"].sort(key=lambda value: value["offer_id"])
    for offer in data["offers"]:
        offer["model_configuration_ids"].sort()
    data["free_offers"].sort(key=lambda value: value["free_offer_id"])
    for offer in data["free_offers"]:
        offer["model_configuration_ids"].sort()
    data["price_rates"].sort(key=lambda value: value["price_rate_id"])
    snapshot = model_data["rating_snapshot"]
    snapshot["models"].sort(key=lambda value: value["model_configuration_id"])
    snapshot["records"].sort(key=native_configuration_key)
    snapshot["model_catalog_digest"] = digest_value(data["models"])
    snapshot["digest"] = digest_value(
        [
            snapshot["schema"],
            snapshot["version"],
            snapshot["scale_version"],
            snapshot["model_catalog_digest"],
            snapshot["models"],
            snapshot["records"],
        ]
    )
    metadata = model_data["metadata_catalog"]
    for field, key in (
        ("evidence_sources", "source_key"),
        ("client_runs", "source_key"),
        ("access_products", "product_key"),
        ("canonical_models", "model_key"),
        ("endpoint_bindings", "binding_key"),
        ("dynamic_routes", "route_key"),
        ("provider_records", "provider_record_key"),
        ("model_records", "model_record_key"),
    ):
        metadata[field].sort(key=lambda value: value[key])


def apply_runtime_projection(registry, model_data, projection):
    metadata_digest = model_data["metadata_catalog"]["source_catalog_digest"]
    if projection["schema"] != "hiroute.runtime-metadata-projection/v1" \
            or projection["source_catalog_digest"] != metadata_digest:
        raise ValueError("runtime projection does not bind the current metadata catalog")
    endpoint_qualified_options = {
        value["connection_option_id"]
        for value in projection["attempts"]
        if value["outcome"] == "endpoint_qualified"
    }
    projected_options = {
        value["connection_option"]["connection_option_id"]
        for value in projection["registrations"]
    }
    if projected_options != endpoint_qualified_options:
        raise ValueError("runtime registrations do not match endpoint-qualified attempts")
    connectors = {value["connector_id"]: value for value in registry["connectors"]}
    profiles = {value["endpoint_profile_id"]: value for value in registry["endpoint_profiles"]}
    options = {value["connection_option_id"]: value for value in registry["connection_options"]}
    for registration in projection["registrations"]:
        profile = registration["endpoint_profile"]
        option = registration["connection_option"]
        connector_id = registration["connector_id"]
        if profile["connector_id"] != connector_id or option["connector_id"] != connector_id:
            raise ValueError("runtime registration crosses connector identity")
        connector = connectors.get(connector_id)
        if connector is None:
            suffix = connector_id.removeprefix("connector.")
            connector = {
                "connector_id": connector_id,
                "revision": 1,
                "runtime_kind": "builtin_native",
                "implementation_ref": registration["implementation_ref"],
                "implementation_revision": 1,
                "accepted_origins": ["native_api"],
                "authentication": "provider_api_key",
                "required_secret_slots": ["provider_api_key"],
                "endpoint_profile_refs": [],
                "catalog_adapter_ref": f"catalog.{suffix}",
                "catalog_adapter_revision": 1,
                "error_classifier_ref": f"errors.{suffix}",
                "error_classifier_revision": 1,
                "usage_decoder_ref": f"usage.{suffix}",
                "usage_decoder_revision": 1,
                "cache_policy_ref": f"cache.{suffix}",
                "cache_policy_revision": 1,
            }
            registry["connectors"].append(connector)
            connectors[connector_id] = connector
        elif connector["runtime_kind"] != "builtin_native" \
                or connector["authentication"] != "provider_api_key" \
                or connector["implementation_ref"] != registration["implementation_ref"]:
            raise ValueError(f"runtime registration conflicts with connector: {connector_id}")
        if profile["endpoint_profile_id"] not in connector["endpoint_profile_refs"]:
            connector["endpoint_profile_refs"].append(profile["endpoint_profile_id"])
        current_profile = profiles.get(profile["endpoint_profile_id"])
        if current_profile is None:
            registry["endpoint_profiles"].append(profile)
        else:
            registry["endpoint_profiles"][registry["endpoint_profiles"].index(current_profile)] = profile
        profiles[profile["endpoint_profile_id"]] = profile
        current_option = options.get(option["connection_option_id"])
        if current_option is None:
            registry["connection_options"].append(option)
        else:
            registry["connection_options"][registry["connection_options"].index(current_option)] = option
        options[option["connection_option_id"]] = option


def converge_preserved_registered_facts(registry, projection):
    if projection["preserved_connection_options"] != ["bailian.payg.cn.v1"]:
        raise ValueError("unexpected preserved registered option set")
    option = next(
        (value for value in registry["connection_options"]
         if value["connection_option_id"] == "bailian.payg.cn.v1"),
        None,
    )
    profile = next(
        (value for value in registry["endpoint_profiles"]
         if value["endpoint_profile_id"] == "endpoint.bailian.payg.cn.v1"),
        None,
    )
    if option is None or profile is None \
            or option["connector_id"] != "connector.bailian.p0" \
            or profile["connector_id"] != "connector.bailian.p0":
        raise ValueError("preserved Bailian PAYG identity changed")
    chat = next(
        (value for value in profile["protocol_endpoints"]
         if value["protocol_endpoint_id"] == "endpoint.bailian.payg.cn.v1.chat"),
        None,
    )
    if chat is None \
            or chat["protocol"] != "chat_completions" \
            or chat["base_url"] != "https://dashscope.aliyuncs.com" \
            or chat["request_path"] != "/compatible-mode/v1/chat/completions" \
            or chat["adapter_ref"] != "adapter.openai-chat.v1" \
            or chat["adapter_revision"] != 1:
        raise ValueError("preserved Bailian PAYG endpoint changed")
    # These were formerly hard-coded by the sole legacy registered check. Moving them into the
    # same client-bundled endpoint record converges authority without qualifying an incomplete binding.
    chat["inventory_path"] = "/api/v1/models"
    chat["authentication_semantics"] = {"kind": "bearer"}
    chat.pop("required_headers", None)
    profile["inventory_strategy"] = "remote_models"
    profile["inventory_protocol_endpoint_id"] = chat["protocol_endpoint_id"]


def converge_scanned_zhipu_facts(registry, metadata):
    """Preserve Messages and prefer the documented native Responses transport."""
    option = next(
        (value for value in registry["connection_options"]
         if value["connection_option_id"] == "zhipu.coding-plan.cn.v1"),
        None,
    )
    profile = next(
        (value for value in registry["endpoint_profiles"]
         if value["endpoint_profile_id"] == "endpoint.zhipu.coding-plan.cn.v1"),
        None,
    )
    if option is None or profile is None \
            or option["connector_id"] != "connector.zhipu.p0" \
            or profile["connector_id"] != "connector.zhipu.p0":
        raise ValueError("scanned Zhipu Coding Plan identity changed")
    messages = next(
        (value for value in profile["protocol_endpoints"]
         if value["protocol_endpoint_id"]
         == "endpoint.zhipu.coding-plan.cn.v1.messages"),
        None,
    )
    if messages is None \
            or messages["protocol"] != "messages" \
            or messages["base_url"] != "https://open.bigmodel.cn" \
            or messages["request_path"] != "/api/anthropic/v1/messages" \
            or messages["adapter_ref"] != "adapter.anthropic-messages.v1" \
            or messages["adapter_revision"] != 1:
        raise ValueError("scanned Zhipu Coding Plan Messages endpoint changed")
    # The metadata catalog's source-024/source-025 record the Coding Plan endpoints and
    # plan credential. The supported Claude configuration uses ANTHROPIC_AUTH_TOKEN, whose
    # transport is Authorization Bearer, plus the standard version header for Messages.
    messages["authentication_semantics"] = {"kind": "bearer"}
    messages["required_headers"] = [["anthropic-version", "2023-06-01"]]
    product = next(
        value for value in metadata["access_products"]
        if value["product_key"] == "zhipu-coding-cn"
    )
    interface = next(
        value for value in product["interfaces"]
        if value["interface_key"] == "zhipu-coding-cn/openai-responses"
    )
    source = next(
        value for value in metadata["evidence_sources"]
        if value["source_key"] == "source-125"
    )
    if {key: interface[key] for key in (
        "interface_key", "protocol", "base_url", "request_path"
    )} != {
        "interface_key": "zhipu-coding-cn/openai-responses",
        "protocol": "openai-responses",
        "base_url": "https://open.bigmodel.cn/api/v1",
        "request_path": "/responses",
    } or source["locator"] != "https://docs.bigmodel.cn/cn/coding-plan/tool/codex" \
            or source["source_key"] not in product["evidence_refs"]:
        raise ValueError("Zhipu Coding Plan Responses source is not qualified")
    responses = {
        "protocol_endpoint_id": "endpoint.zhipu.coding-plan.cn.v1.responses",
        "protocol": "responses",
        "base_url": "https://open.bigmodel.cn",
        "request_path": "/api/v1/responses",
        "adapter_ref": "adapter.openai-responses.v1",
        "adapter_revision": 1,
        "stable_preference": 0,
        "authentication_semantics": {"kind": "bearer"},
    }
    if any(value["protocol_endpoint_id"] == responses["protocol_endpoint_id"]
           for value in profile["protocol_endpoints"]):
        raise ValueError("preserved Zhipu registry unexpectedly contains Responses")
    profile["protocol_endpoints"].append(responses)
    for endpoint in profile["protocol_endpoints"]:
        if endpoint is not responses:
            endpoint["stable_preference"] += 1
    profile["verification_evidence"] = digest_value([
        profile["verification_evidence"], product["evidence_refs"], source["locator"]
    ])
    profile["last_verified_at"] = int(datetime.datetime.strptime(
        source["collected_on"], "%Y-%m-%d"
    ).replace(tzinfo=datetime.timezone.utc).timestamp())


def compiler_input():
    registry = json.loads(
        (ROOT / "assets/connector-registry/current/registry-seed.json").read_bytes()
    )
    model_data = json.loads(
        (ROOT / "assets/model-data/current/model-data.json").read_bytes()
    )
    projection = json.loads(
        (ROOT / "assets/model-data/current/runtime-projection.json").read_bytes()
    )
    registry["registry_version"] = PRODUCT_RELEASE
    registry["product_release"] = PRODUCT_RELEASE
    model_data["data"]["product_release"] = PRODUCT_RELEASE
    model_data["data"]["connector_registry_version"] = PRODUCT_RELEASE
    apply_runtime_projection(registry, model_data, projection)
    converge_preserved_registered_facts(registry, projection)
    converge_scanned_zhipu_facts(registry, model_data["metadata_catalog"])
    templates = {value["connection_option_id"]: value for value in projection["connection_templates"]}
    for option in registry["connection_options"]:
        template = templates[option["connection_option_id"]]
        option["display_name"] = template["name"]["en"]
        profile = next(value for value in registry["endpoint_profiles"]
                       if value["endpoint_profile_id"] == option["endpoint_profile_id"])
        for defaults in template["endpoint_defaults"]:
            endpoint = next(value for value in profile["protocol_endpoints"]
                            if value["protocol_endpoint_id"] == defaults["protocol_endpoint_id"])
            endpoint["authentication_semantics"] = defaults["authentication_semantics"]
            if endpoint["protocol"] == "messages":
                endpoint["required_headers"] = [["anthropic-version", "2023-06-01"]]
    normalize_registry(registry)
    normalize_model_data(model_data)
    return {
        "schema": "hiroute.release-facts-compiler-input/v2",
        "tool_version": "hiroute-release-facts/2",
        "catalog_id": CATALOG_ID,
        "product_release": PRODUCT_RELEASE,
        "sequence": SEQUENCE,
        "connector_registry": registry,
        "model_data": model_data,
    }


def outputs():
    value = compiler_input()
    profiles = json.loads(
        (ROOT / "assets/agent-profiles/current/profile-seed.json").read_bytes()
    )
    profiles["tool_version"] = "hiroute-release-facts/2"
    registry_bytes = compact(value["connector_registry"]) + b"\n"
    model_data_bytes = compact(value["model_data"]) + b"\n"
    cross_reference_digest = digest_value(
        [
            "hiroute.release-facts-cross-reference/v2",
            value["connector_registry"],
            value["model_data"],
        ]
    )
    manifest = {
        "schema": "hiroute.release-facts/v2",
        "tool_version": "hiroute-release-facts/2",
        "catalog_id": CATALOG_ID,
        "product_release": PRODUCT_RELEASE,
        "sequence": SEQUENCE,
        "connector_registry_digest": digest_bytes(registry_bytes),
        "model_data_digest": digest_bytes(model_data_bytes),
        "cross_reference_digest": cross_reference_digest,
    }
    return {
        HERE / "compiler-input.json": (
            json.dumps(value, ensure_ascii=False, indent=2) + "\n"
        ).encode(),
        BUNDLE / "agent-profiles.json": compact(profiles, sort_keys=False) + b"\n",
        BUNDLE / "connector-registry.json": registry_bytes,
        BUNDLE / "model-data.json": model_data_bytes,
        BUNDLE / "manifest.json": compact(manifest, sort_keys=False) + b"\n",
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    changed = []
    for path, content in outputs().items():
        if path.exists() and path.read_bytes() == content:
            continue
        changed.append(str(path.relative_to(ROOT)))
        if not arguments.check:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
    print(json.dumps({"check": arguments.check, "changed_files": changed}))
    if arguments.check and changed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
