"""Exact Claude subscription capability joins from the sole maintained catalog.

These are static model facts. Account inventory, Check, model selection and Save
remain independent runtime gates, including for a documented convenience alias.
"""
import copy


PRODUCT = "claude-code-subscription"
INTERFACE = f"{PRODUCT}/anthropic-messages"
ENDPOINT = "endpoint.cpa.claude"


def native_projection(model, binding):
    projection = binding.get("reasoning_projection", {})
    reasoning = model["reasoning"]
    if projection.get("kind") == "adaptive-effort":
        if projection != {
            "kind": "adaptive-effort",
            "parameter": "output_config.effort",
            "thinking_type": "adaptive",
        } or reasoning["kind"] not in ("adaptive", "adaptive-discrete", "adaptive-fixed-on"):
            raise ValueError(f"unsupported Claude adaptive projection: {binding['binding_key']}")
        profiles = reasoning.get("profiles", [])
        if (not profiles or len(profiles) != len(set(profiles))
                or any(p not in ("low", "medium", "high", "xhigh", "max") for p in profiles)):
            raise ValueError(f"Claude efforts lack exact model evidence: {model['model_key']}")
        return {
            "capability": {"kind": "discrete", "parameter": "output_config.effort",
                           "profiles": list(profiles)},
            "native_render_convention": "claude_adaptive_effort_messages",
        }
    if projection.get("kind") == "toggle-minimum-budget":
        expected = {
            "kind": "toggle-minimum-budget", "parameter": "enable_thinking",
            "enabled_budget_tokens": 1024, "scope": "supported-subset",
        }
        if ({key: value for key, value in projection.items() if key != "note"} != expected
                or not projection.get("note") or reasoning["kind"] != "manual-budget"
                or reasoning.get("minimum_budget_tokens") != 1024
                or reasoning.get("output_constraint") != "budget_tokens < max_tokens"):
            raise ValueError(f"unsupported Claude manual thinking subset: {binding['binding_key']}")
        # The existing Messages toggle renderer owns enabled + budget_tokens=1024.
        # It rejects a smaller output cap; this is not an inferred native default.
        return {"capability": {"kind": "toggle", "parameter": "enable_thinking"}}
    raise ValueError(f"missing Claude reasoning projection: {binding['binding_key']}")


def append_claude_subscription(catalog, output_models, capabilities, offers,
                              model_definition, digest):
    products = {value["product_key"]: value for value in catalog["access_products"]}
    product = products[PRODUCT]
    bindings = [value for value in catalog["endpoint_bindings"]
                if value["product_key"] == PRODUCT]
    if not bindings:
        return
    if (product["provider"] != "Anthropic"
            or product["product_kind"] != "developer-subscription"
            or product["billing"] != "shared-subscription-usage"):
        raise ValueError("Claude subscription capabilities cross product identity")
    interfaces = [value for value in product["interfaces"] if value["interface_key"] == INTERFACE]
    if interfaces != [{"interface_key": INTERFACE, "protocol": "anthropic-messages",
                       "base_url": "https://api.anthropic.com", "request_path": "/v1/messages"}]:
        raise ValueError("Claude subscription interface is not the registered Messages contract")
    models = {value["model_key"]: value for value in catalog["models"]}
    sources = {value["source_key"]: value for value in catalog["evidence_sources"]}
    projected_models = {}
    seen_upstream = set()
    evidence_digests = []
    for binding in sorted(bindings, key=lambda value: value["binding_key"]):
        model = models[binding["model_key"]]
        upstream = binding["upstream_model_id"]
        if (model["publisher"] != "Anthropic" or model["lifecycle"] != "active"
                or binding.get("lifecycle") != "active"
                or upstream not in model["upstream_ids"] or upstream in seen_upstream
                or binding["interface_candidates"] != [INTERFACE]
                or binding["availability"]["state"] != "conditional"
                or binding["protocol_qualification"] != "runtime-required"
                or binding.get("capability_overrides")):
            raise ValueError(f"Claude subscription binding is not exact and conditional: {upstream}")
        seen_upstream.add(upstream)
        for entity in (product, model, binding):
            if not entity["evidence_refs"]:
                raise ValueError(f"Claude subscription evidence is missing: {upstream}")
            for reference in entity["evidence_refs"]:
                if sources[reference]["authority"] not in (
                    "platform.claude.com", "support.claude.com", "code.claude.com",
                ):
                    raise ValueError(f"Claude subscription evidence crosses publisher: {upstream}")
        identity = f"model.anthropic.{model['model_key']}"
        definition = model_definition(model, identity, "publisher.anthropic")
        native = {"model_configuration_id": identity, **native_projection(model, binding)}
        projected = {
            "source_model_key": model["model_key"], "model": definition,
            "native_reasoning": native,
            "sources": [sources[ref]["locator"] for ref in sorted(set(
                model["evidence_refs"] + binding["evidence_refs"]))],
        }
        previous = projected_models.get(identity)
        if previous is not None and previous != projected:
            raise ValueError(f"Claude aliases disagree on model facts: {identity}")
        projected_models[identity] = projected
        evidence = digest(["hiroute.claude-subscription-capability/v1", product, model, binding,
                           [sources[ref] for ref in binding["evidence_refs"]]])
        evidence_digests.append(evidence)
        capabilities.append({
            "capability_id": f"cap.cpa.claude.{upstream}.messages", "revision": 1,
            "model_configuration_id": identity, "connector_id": "connector.cpa.claude",
            "connector_revision": 1, "endpoint_profile_id": ENDPOINT,
            "endpoint_profile_revision": 1, "protocol_endpoint_id": f"{ENDPOINT}.messages",
            "upstream_protocol": "messages", "upstream_model_id": upstream,
            "required_adapter_ref": "adapter.anthropic-messages.v1",
            "required_adapter_revision": 1, "evidence_digest": evidence,
        })
    output_models.extend(copy.deepcopy(projected_models[key]) for key in sorted(projected_models))
    offers.append({
        "offer_id": "offer.claude.subscription", "revision": 1,
        "endpoint_profile_id": ENDPOINT, "endpoint_profile_revision": 1,
        "service_offering_id": "claude-subscription", "entitlement_id": "claude-subscription",
        "usage_scope": "account", "region_id": "global",
        "model_configuration_ids": sorted(projected_models), "billing_class": "subscription",
        "evidence_digest": digest(["hiroute.claude-subscription-offer/v1", sorted(evidence_digests)]),
    })
