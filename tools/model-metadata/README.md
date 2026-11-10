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
