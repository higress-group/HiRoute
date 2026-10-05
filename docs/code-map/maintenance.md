# Engineering cleanup ledger

[Code map](README.md)

This ledger records responsibility and test concerns; it does not authorize a
broad rewrite. Test churn alone does not establish redundancy or code decay.

## Review causes, not counts

- Build current-state fixtures through the current producer and deserialize them
  through real consumers. Keep one legal setup per boundary; see the
  [testing guide](testing.md).
- Keep historical fixtures only for supported recovery contracts. The
  [compatibility registry](../../contracts/compatibility-support.v1.json) records
  their owner, reason and removal condition.
- Preserve assessment attribution, history restoration, authorization and cleanup
  regressions even when implementation changes require updating their fixtures.
  A stable product assertion matters more than a stable test line count.

## First batch and remaining work

| Area | First foundation batch | Next bounded step / exit criterion |
| --- | --- | --- |
| Agent UI ownership | Group current ecosystem forms, state and request construction under one feature; make dispatch explicit. Pi now reuses Qoder's additional-route editor and native settings transaction | Add native differences in the typed leaf; preserve independent model/Skill configure, edit and restore scenarios without new verification controls |
| Settings facts | Pi separates common transaction facts from typed Codex, Claude and additional-provider facts | Keep capability/catalog requirements in the matching leaf; changing one native ecosystem must not add irrelevant requirements to another |
| Saved native sources | Pi acceptance now follows saved discovery through eligibility, source-local authority, Plan publication and ordinary native startup | Keep a real saved-source route witness when extending discovery; scan visibility or successful save alone does not prove usability |
| Native reasoning wire | Domain owns current field DTOs, protocol paths and binary toggle semantics; daemon and Gateway reuse them | Extend the existing materializer → compiler → Gateway assertion for new wire shapes; retain accepted parameter aliases and custom Bool profile compatibility |
| Worker native context | [Native user-context reuse and skill inheritance](worker-context.md) | For each new ecosystem, verify native context and per-run overrides; preserve exact continuation, configuration non-pollution, ownership and retention |
| Decision coupling | Separate existing Jev pure policy from HTTP hosting; give Gateway name resolution a narrow owner | [Separate provider transport from routing/tool policies](decision-foundation.md); future tasks own built-in Jev, provider configuration, natural-language branches and ContextHold tool selection |
| False-green checks | Bind schema convergence to the authoritative production source; select required browser scenarios by stable identity | Gate tests must reject missing or changed production facts even when a fixture still contains the expected token |
| Test discovery and layout | Add a product map; replace selected dynamic source-shape checks with behavioral coverage | Consolidate only after recording preserved unique assertions and the layer that owns each one |
| Product runtime binding | Pi reuses native model, collaboration, context, cancellation and compaction journeys across ecosystems | Let shared fixtures accept an explicit production runtime directory: Desktop uses `run/`, while the existing headless fixture uses `runtime/`. Preserve socket ownership/readiness checks and independent receipt assertions; do not encode a second native resource HOME or copy the journey |
| Large control assembly | Document `LocalControlAdapter` and runtime owners | Extract one cohesive port/consumer during a real feature; avoid a facade-wide rewrite |
| Worker and observation | Correct stale local ownership docs | Review embedded fixtures, shared process harnesses and constant-only assertions independently; preserve lease, cleanup and content-visibility fault cases |
| Recovery and compatibility | Document current Configure/Edit, ordinary Disable and legacy-tail distinctions | Keep recent recovery code stable; change only with a concrete failing lifecycle scenario |

For the next feature, compare how many responsibilities it touches, whether state
has one owner, how much unrelated code the agent must read, and whether tests change
because the product contract changed or because internal structure moved. Record
examples in the feature handoff. Prefer a small concrete improvement over a new
global score, a file-size target or another layer of generic abstractions.
