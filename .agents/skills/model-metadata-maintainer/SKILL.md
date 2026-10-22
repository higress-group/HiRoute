---
name: model-metadata-maintainer
description: "Refresh, extend, or audit HiRoute's single current model-metadata source with field-completeness-derived usage scenarios and traceable sources. Use for model/provider metadata, evidence freshness, generated current artifacts, or pinned client-repository scans; not for runtime contract changes."
---

# Model Metadata Maintainer

Maintain `assets/model-data/current/inputs/metadata-catalog.json` as HiRoute's sole current model-metadata source. The generated files in `assets/model-data/current/`, `assets/release-facts/current/` and `assets/model-feed/payload.json` are deterministic projections of that source, never separately maintained catalogs. Do not create edition selectors, compatibility catalogs, or parallel metadata products.

## Load only what the task needs

- For every catalog update or audit, read [references/data-contract.md](references/data-contract.md).
- When selecting, adding, or refreshing evidence, also read [source-registry.json](../../../tools/model-metadata/source-registry.json).
- For a client-repository scan, read [references/client-discovery.md](references/client-discovery.md) and [client-source-profiles.json](../../../tools/model-metadata/client-source-profiles.json).
- Run `tools/model-metadata/validate_catalog.py` after every data or source-registry change.

## Update workflow

1. Identify the requested scope: full refresh, provider/product refresh, model refresh, new source, or one data-quality decision.
2. Inspect the current catalog entities and source registry before browsing. Reuse a canonical identity only when publisher identity is genuinely the same; never reuse a value merely because an old HiRoute record had it.
3. Classify metadata by what its fields can support, never by a source-confidence tier. Exact-commit Hermes Agent or OpenClaw assertions may populate the same provider/model metadata-record schema used by future sources. Provenance affects audit, refresh, conflict resolution, and identity normalization; it does not suppress a `usable_for` scenario whose required fields are present.
4. Record facts at their real scope:
   - model facts on `models`;
   - product, region, billing, credential, and interface facts on `access_products`;
   - product/model/upstream/protocol facts on `endpoint_bindings`;
   - non-stable routers on `dynamic_routes`.
5. Preserve uncertainty explicitly. Do not replace `unknown`, `conflict`, `conditional`, or `runtime-required` with a guessed boolean or number. A provider-scoped field may only stop being `unknown` through step 8's reviewed, labeled closure — never by writing an unlabeled value into the record.
6. Add or update normalized `evidence_sources`, then reference them by `source_key`. When authenticated bytes are captured, store a SHA-256 digest; never store credentials, tokens, headers, account IDs, or response bodies containing secrets.
7. Update `as_of` only for facts actually reviewed. Do not imply that untouched providers were refreshed.
8. Close the catalog: `tools/model-metadata/close_model_metadata.py` rewrites every provider-scoped record so no closable field stays `unknown` — it either keeps an observed assertion, assigns an inferred value from `tools/model-metadata/inference-rules.json` (recording `{basis: inferred, rule_key}`), or leaves an explicit `unsupported`/`not-applicable`. When a new or changed fact needs a rule, add or amend the rule there instead of hand-writing a value; a rule that stops matching is discarded and re-closed, never left stale.
9. Run the validator and review the semantic diff. Report changed facts, added/retired identities, conflicts, runtime-required items, inference-closed fields (with their rule keys), and source freshness separately.
10. When client records or consumer coverage change, refresh `assets/model-data/current/inputs/client-discovery-runs.json`, then regenerate `assets/model-data/current/` and `assets/release-facts/current/`. Keep review findings in the change discussion; do not commit one-off reports, uncertainty dumps, or projection notes.
11. Before regenerating a semantic correction, withdrawal or addition, increase `assets/model-data/current/inputs/official-data-revision.json`; the same data revision must never identify different semantic content. Regenerate and commit the reviewed inputs and shared release bundle first, then build `assets/model-feed/payload.json` with `cargo run -p hiroute-release-facts -- feed build`. This two-stage commit sequence gives the publication a reproducible, squash-stable bundle-tree `source_revision`. The producer compares the embedded snapshot with HEAD and, when supplied, the PR/push `--base`, even while the payload is JSON null. Maintain editorial announcements in `featured.json`; model facts still come only from the catalog. When exact publication bytes change, increase `publication.json` sequence and update its UTC issue time, then regenerate and check again. Announcement-only changes do not require a data revision increase. Before the first recorded feed-capable release, publication is skipped, but embedded semantic identity remains checked. See [feed publication](../../../assets/model-feed/README.md) for HTTPS trust and deployment boundaries.

## Scanning a client repository

Use an exact-commit scan when a maintained client such as Hermes Agent or OpenClaw can reveal candidate model IDs, provider products, endpoints, protocol quirks, lifecycle hints, or official source locators.

1. Resolve and record a full 40-character commit. Never scan a moving branch, tag, or working-tree view.
2. Run `scripts/scan_client_repo.py` with a registered profile, repository path, exact commit, catalog, and explicit `--as-of` date. The script reads Git objects without importing or executing target-repository code.
3. Treat the result as a reproducible provenance input, not as a second metadata catalog. Materialize it into unified provider-scoped records with `scripts/materialize_client_metadata.py`; do not assign an attestation/confidence tier.
4. Derive every completeness domain and `usable_for` scenario mechanically. A model ID alone supports directory and model prefill; a single Base URL plus supported protocol supports endpoint prefill; known token fields support the corresponding limit prefill; capability, rendering, lifecycle, and cost fields unlock only their matching scenarios.
5. Normalize a record to a publisher-level identity only when identity resolution is sufficient. `normalized_model_matches` is a navigation hint and does not gate use of the provider-scoped record.
6. Preserve raw assertions, exact Git locations, conflicts, and unknowns. A default/fallback remains a default hint; a cost object remains a cost hint unless its currency, unit, product, and schedule are separately modeled as a pricing fact.
7. Treat reasoning support, supported-effort lists, and effort maps as prefill hints unless the record also establishes the exact endpoint/protocol parameter path and render convention required by the target runtime contract. Do not infer streaming or tool fidelity from generic protocol compatibility.

Example:

```bash
python3 .agents/skills/model-metadata-maintainer/scripts/scan_client_repo.py \
  --profile openclaw \
  --repo /absolute/path/to/openclaw \
  --commit <40-character-commit> \
  --catalog assets/model-data/current/inputs/metadata-catalog.json \
  --as-of YYYY-MM-DD \
  --output /tmp/openclaw-model-discovery.json
```

Materialize one or more pinned scans into the same current catalog:

```bash
python3 .agents/skills/model-metadata-maintainer/scripts/materialize_client_metadata.py \
  --catalog assets/model-data/current/inputs/metadata-catalog.json \
  --scan /tmp/hermes-model-discovery.json \
  --scan /tmp/openclaw-model-discovery.json \
  --output assets/model-data/current/inputs/metadata-catalog.json
```

## Adding a source

Update `tools/model-metadata/source-registry.json` when a new authoritative domain or documentation family is needed.

- Confirm the domain is controlled by the provider, product owner, or an authenticated service endpoint.
- Add a stable `authority_key`, allowed HTTPS prefixes, evidence scope, and refresh interval.
- Do not broaden an existing prefix merely to make validation pass.
- Add the concrete page or endpoint to catalog `evidence_sources`; the registry authorizes a source family but does not itself prove a field.
- If ownership is unclear, keep the locator as a private discovery note outside the catalog and report it as unresolved source authority.

For a client source tree, update `tools/model-metadata/client-source-profiles.json` instead. Pin the allowed repository identity, choose the narrow parser and path set, set `evidence_eligible=false`, and test it against a full commit. Adding a discovery profile never authorizes that repository as provider evidence.

## Merge and conflict rules

- Newer evidence does not automatically outrank stronger evidence. Prefer authenticated endpoint metadata for account availability, and provider documentation for public identity and contract claims.
- When current sources disagree, preserve `conflict` with every provenance ref. Conflict reduces the affected completeness domain; it does not lower unrelated domains or the whole record.
- A rolling alias and an immutable revision are distinct identities or bindings according to the provider contract; do not silently rewrite one into the other.
- A default token value is not a hard limit. Shared input/output budgets use `shared-total-budget`; entitlement-specific limits use `entitlement-dependent`.
- A documented product/model association remains `conditional` until region, account, entitlement, and protocol qualification are resolved.
- Retired endpoints remain historical facts only when the task requires history; they never become executable bindings.

## Integration boundary

This skill owns the current metadata source and its existing deterministic generators. A metadata refresh may update source facts and regenerate the checked-in current model data and client bundle, but it does not authorize changes to runtime loaders, compiler contracts, persistence, or consumers. Such changes remain with the selected issue-spec integration owner. Never add legacy readers, compatibility projections, edition selectors, or dual-write paths.

## Validation

From the repository root:

```bash
python3 tools/model-metadata/close_model_metadata.py --check
python3 tools/model-metadata/validate_catalog.py
python3 assets/model-data/current/generate.py
python3 assets/model-data/current/generate.py --check
python3 assets/release-facts/current/prepare-bundle.py
python3 assets/release-facts/current/prepare-bundle.py --check
cargo run -p hiroute-release-facts -- feed build
cargo run -p hiroute-release-facts -- feed build --check
node apps/website/scripts/model-feed.mjs check
```

`close_model_metadata.py` owns the closure rules and writes the source catalog in place; `--check` fails when the catalog is not a fixed point. Run it before validation, and run it twice when it changed a record: the second run must produce no diff. Then run the current generators and feed builder twice; their second run must produce no diff. The feed check uses shared Rust admission against supported release bundles; Node checks only complement it. Pass the intended PR/push baseline to `feed build --check --base BASE` so committed semantic changes are checked as well as local regeneration. A compatible recorded client release precedes the first publication; HTTPS and the production website pipeline establish trust without a signing-key lifecycle. The validator checks entity keys, references, state vocabularies, source authorization, token-null semantics, endpoint relationships, field-derived completeness and `usable_for`, provenance, the inference-rule table and per-field provenance, the closure fixed point, dynamic-route attribution, and a deterministic canonical digest. Structural failures are red; stale sources are reported separately and require refresh or an explicit runtime/excluded disposition.

For a changed discovery profile or scanner, scan its target commit twice and compare the outputs byte-for-byte. Also run the scanner's self-tests:

```bash
python3 .agents/skills/model-metadata-maintainer/scripts/test_scan_client_repo.py
python3 .agents/skills/model-metadata-maintainer/scripts/test_materialize_client_metadata.py
```
