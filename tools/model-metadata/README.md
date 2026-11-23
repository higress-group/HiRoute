# Model metadata validation

This directory contains the deterministic rules and validators used by the public model
catalog. The runtime catalog remains under `assets/model-data/current`; these tools verify
its source registry, closure rules, provenance and generated projection without network or
wall-clock input.

```sh
python3 tools/model-metadata/close_model_metadata.py --check
python3 tools/model-metadata/validate_catalog.py
python3 assets/model-data/current/generate.py --check
python3 tools/model-metadata/test_codex_subscription_projection.py
```

An inference rule may specify its own `collected_on` date. Without it, the table's
`as_of` applies; refreshing one rule must not make unrelated evidence appear fresh.
The validator checks the date and the generated provenance against the same rule.

Client-repository discovery and editorial maintenance workflows are not required to build or
validate the published catalog.

Native decision windows are `decision_constraints` on exact endpoint bindings in
the same source catalog. Each constraint names a decision interface, a positive
unsigned 32-bit request limit, an optional independent question limit, its header
condition, evidence references and `documented`, `inferred` or `maintainer_policy`
basis. Decision-only canonical identities leave ordinary generation limits
not applicable; these records do not create inference capabilities or offers.
Rounded-value normalization and product-selected defaults retain an explanatory
note instead of claiming measured endpoint capacity. Unknown bindings receive the
runtime product fallback; never add a fabricated catalog fact for that fallback.

The decision resolver uses exact canonical HTTPS URL, wire protocol and callable
model identity. Business-space URL templates accept one WorkspaceId DNS label on
the documented official suffix. Provider names and unlisted aliases do not match.
The TypeSafe header destination remains an installed typed transport policy;
remote metadata cannot authorize a new header destination. A saved resolved
snapshot retains its catalog identity and is not refreshed at publication/runtime.

Native API capability defaults match endpoint bindings to the selected exact wire
interface. Represent protocol-specific capabilities with separate bindings and
explicit overrides; an unrelated interface does not contribute or conflict, while
an unmatched or withdrawn explicit binding cannot revive provider-prefill hints.
When a canonical model records an independent `max_input_tokens`, the Native API
resolver uses it for the input budget before falling back to its context value.
The canonical context window remains the provider's total window; saved custom
values and frozen publications retain their existing authority.

A saved registered API source can retain several native interfaces. Shared
source-level tool availability includes its qualified saved interfaces, while
candidate materialization resolves each emitted wire profile's own tool support
before compiling request/response fidelity. Manual tool values retain precedence
across the saved interfaces; metadata aggregation never authorizes another target.

Generated model JSON remains one JSON document, with deterministic whitespace after
commas outside strings once a line reaches 64 KiB. This keeps complete large
catalogs readable by line-oriented review and credential scanners. Strings and
canonical semantic digests are unchanged; byte digests and bundle manifests bind
the exact emitted bytes. Python preparation and the Rust producer use the same
format. Existing artifact-size and structural validation still apply.

GPT-6.2 Astra and Sol have separate Responses and Chat Completions bindings. Tools
require Responses. Their documented total context, input limit and output limit
are independent. The [OpenAI model index](https://developers.openai.com/api/docs/models#flagship-models)
lists `low`, `medium`, `high`, `xhigh`, and `max` for both models. The catalog records
those profiles with an unknown model-specific default. Both APIs reject `none`.
The Codex subscription projection uses the same ordered profiles after the scoped
native model record confirms them. Its authenticated directory keeps the client
window, client default and the additional automatic-delegation `ultra` mode in
source assertions. The executable subscription uses the five wire-level profiles.
CPA still verifies each account's model membership and the subscription Offer
retains its own billing and entitlement.

Their prices preserve the whole-request 272,000-input-token boundary, Standard,
Batch and Fast tiers, and separate 30-minute and Responses-only 12-hour writes.
Cache writes are total rates, not input surcharges. Ultrafast's dated promotion,
background completion windows, regional uplifts and tool charges are outside these
schedules. Unsupported conditions remain unknown in the installed price evaluator.

The [GPT-6.2 migration guide](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6.2-astra)
requires callers to review unsupported sampling/log-probability fields and replace
`prompt_cache_retention` with `prompt_cache_options.ttl`. Native Gateway projection
preserves caller fields; it does not perform this model-specific migration.
Top-level `instructions` follow `input` on GPT-6.2; reusable instructions belong in
an initial developer message. Prefix edits, model downgrades and end-user or
organization changes can cause the provider to drop reasoning items. Preserving
the native wire does not guarantee provider-side reasoning reuse across routing
changes. See [prefix preservation](https://developers.openai.com/api/docs/guides/keep-prefix-unchanged)
and [cache billing](https://developers.openai.com/api/docs/guides/prompt-caching).
