# Current model metadata

Run `python3 assets/model-data/current/generate.py`; `--check` rejects stale output. This directory
is the single maintained MVP model-data source. The generator pins the complete source metadata and
exact client-discovery runs under `inputs/` by byte digest and emits `model-data.json`, `runtime-projection.json`, and
`coverage.json`; no network or wall-clock input participates.

Schema and endpoint identifiers may retain a numeric wire-format revision. Those suffixes do not
identify model-metadata editions: the only maintained and runtime-loaded catalog is `current`.

The catalog contains source-scoped Provider and model records alongside access products,
normalized publisher models, endpoint bindings, and dynamic-route attribution. Each maintained
provider-scoped field closes to an observed fact, a traceable conservative inference, or explicit
unsupported/not-applicable. Cost hints remain display-only and cannot become runtime price or free
facts. Source, product, region, protocol, account, and upstream model ID scopes remain distinct, so
same-name models from different Providers never share endpoints, credentials, prices, or
entitlements.

The generated runtime projection promotes only records with a complete executable endpoint and
adapter contract. Models newly observed from an already-qualified account/Endpoint may use the
separately marked conservative runtime fallback; that does not create canonical identity, rating,
price, free eligibility, or cross-Provider facts. Known image-only and internal-agent records stay
outside the native text execution slice.

Claude subscription capabilities use explicit product/model/upstream-ID bindings in the same
source catalog. `subscription_projection.py` translates their `reasoning_projection` into existing
runtime contracts; it does not infer family members or account access. Check must still observe
the exact ID in the selected account's inventory before these static facts can qualify a saved
binding. Haiku 4.5 retains its manual-budget source fact and exposes only the explicit toggle
subset: disabled, or enabled with 1024 thinking tokens and an output budget greater than 1024.

Ratings are an independent slice. Existing measured/estimated records retain their exact evidence;
missing ratings stay unknown rather than being filled from metadata inference.
