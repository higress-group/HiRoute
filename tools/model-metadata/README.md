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
