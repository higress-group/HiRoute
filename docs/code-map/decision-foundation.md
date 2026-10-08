# Decision models, routing and competence

[Architecture](architecture.md) · [Gateway map](../../crates/gateway/README.md) ·
[Decision protocol](../../decision-extensions/api/decision-design.md) ·
[Routing guide](../smart-saving-model-classification.md)

The current development v1 separates task categories, ordinal degree and actual model groups.
Use fresh data: no old decision-format reader, migration, negotiation or additional protocol version.
Tool selection remains [future protocol documentation](../../decision-extensions/api/decision-design.md),
with no runtime or product entry. The paths and assertions below identify current
implementation boundaries. Contract tests establish routing and observation behavior;
provider judgment quality requires separate evaluation on representative tasks.

## Start with a user path

| Capability | Production owner | Representative assertion |
| --- | --- | --- |
| Connect, edit, test or delete a decision model/custom extension | [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [native transaction](../../apps/desktop/src-tauri/src/session/decision_services.rs), [store](../../crates/local-storage/src/control/decision_services.rs) | [Shell journeys](../../apps/desktop/tests/v3/browser/product-shell-scenarios.mjs), [real CLI lifecycle](../../crates/daemon/tests/support/decision_services_product.py) |
| Add a connection from an unfinished route | [DesktopApp](../../apps/desktop/src/product/DesktopApp.tsx), [DecisionSelector](../../apps/desktop/src/features/decision-services/DecisionSelector.tsx) | Retain edits, select exact saved revision, do not publish implicitly |
| Configure smart saving or independent task categories | [Plan editor](../../apps/desktop/src/plan-editor.tsx), [BranchRoutingEditor](../../apps/desktop/src/features/decision-services/BranchRoutingEditor.tsx), [JudgmentSettings](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx), [domain](../../crates/domain/src/routing/decision.rs) | Collapsed advanced fields, existing model picker/effort dialog, complete override copy/reset, unfinished draft versus publish validation |
| Freeze definition, provider and rubric | [Application compiler](../../crates/application/src/compiler), [gateway compiler](../../crates/gateway/src/publication/compiler.rs), [definition DTO](../../crates/domain/src/routing/decision_definition.rs) | [Publication/restart contract](../../crates/gateway/src/publication/tests/grant_projection.rs), typed current fixture producer |
| Judge this task and the preceding actual stage once | [Lifetime/transport](../../crates/gateway/src/core_runtime/classification.rs), [System One adapter](../../crates/gateway/src/core_runtime/classification/system_one.rs), [custom v1 codec](../../crates/gateway/src/core_runtime/classification/protocol.rs) | [Typed provider mapping](../../crates/gateway/src/core_runtime/classification/system_one_tests.rs), [strict protocol and Replay](../../crates/gateway/src/core_runtime/classification/protocol_tests.rs) |
| Choose a group on each new turn and execute its candidates | [Group policy](../../crates/gateway/src/core_runtime/classification/group_policy.rs), [planner](../../crates/gateway/src/planner.rs), [history](../../crates/gateway/src/agent_turn_history/store.rs) | [Threshold/score isolation](../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs), [group boundaries](../../crates/gateway/src/planning/branch_tests.rs), [real listener multi-turn journey](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Inspect competence and actual execution | [Receipt](../../crates/observation/src/receipt), [quality query](../../crates/observation/src/query_v2), [PlanQuality](../../apps/desktop/src/features/PlanQuality.tsx), [stage detail](../../apps/desktop/src/features/PlanQualityStage.tsx) | [Query aggregation](../../crates/observation/src/store/tests/plan_quality/summary.rs), [UI identities](../../apps/desktop/tests/plan-quality-state.test.mjs), listener facts |

## Shared contracts

`DecisionServiceV1` is an immutable saved connection revision. Desktop calls it a decision model
or custom extension; the existing CLI remains `decision services list/apply/test`.
Writes and credentials reuse existing Operations, protected input and Secret storage. A plan freezes
the chosen connection; editing it does not silently alter published plans. Provider/origin changes clear
authentication. Disclosures and protocol views retain edits. Save-and-test tests the exact saved revision.
No separate Secret page, decision daemon or provider plugin framework is introduced.
The [authoring snapshot](../../crates/daemon/src/control/runtime/plan_content_snapshot.rs) reads the
selected immutable connection by ID/revision from storage; preview and the existing apply reproduction
both verify its full content before freezing it. Historical saved revisions remain selectable. There is
no inline `rest` authoring mode; custom HTTP transport belongs to a saved `DecisionConnectionV1::Custom`.

`JudgmentSettingsV1` contains degree and competence prompts/thresholds. Smart saving has one task scope
and two execution groups: economy/primary. `BranchRoutingV1` is only for custom categories; each has
regular candidates and optional primary candidates. Missing primary means no degree question, while
competence remains observable. A branch either follows plan judgment or stores a complete independent
copy. Published settings and their rubric digest are frozen; draft edits do not reinterpret old evidence.
`smart_saving` is reserved for the built-in task scope and rejected as a custom category ID at authoring,
materialization and Gateway publication. Prompt text has no per-field character quota: the editor,
Domain and canonical OpenAPI preserve complete conditions and standards; existing transport budgets
and provider limits remain owned by their adapters.

`DecisionDefinitionV1` describes ordinal or categorical questions. Gateway compiles one System One
call: a category Choice when needed, an independent degree Score for each dual-group category, and
an optional historical competence Score. Request-local question bindings reduce only the selected path.
Degree uses probabilities; competence divides the provider's raw 0..2 score by two. Unselected malformed
answers do not poison a valid result. Current input is preserved; only whole old history turns can be
removed for the adapter's byte bound, and target evidence loss marks assessment partial.
The history store distinguishes an uncaptured earlier prefix from loss inside the current
scoring stage; only the latter invalidates that stage's complete competence evidence.

The custom v1 request has exactly `decision`, `latest_user`, `visible_conversation`, `history_partial`,
and `assessment_target`. The [OpenAPI](../../decision-extensions/api/decision.openapi.json), Desktop
examples and [official Jev extension](../../decision-extensions/extensions/jev-decider) share this boundary.
`python3 scripts/decision-contracts.py` produces schema/examples from the current canonical cases.
The extension owns provider integration and trimming, but must obey the supplied definition and rubric;
HiRoute owns probability thresholds, protection, candidate selection and actual attribution.

`group_policy.rs` uses this call's simple probability and fresh applicable assessment; no persistent upgrade
cursor exists. A new user message always decides again. Regular requires P(simple) at/above the configured
threshold and no complete low score for the same category, plan revision and rubric. Complex, unavailable
degree or applicable low competence selects primary. Missing or partial scores cannot preserve an old
protection. A new turn can still choose primary; it never automatically resets to regular. Tool continuations
and same-turn replay inherit only while message history continues and the frozen decision remains reusable.
A history rebuild or changed plan/authority can require a fresh decision. Candidate hold applies only within the newly selected group.
The [ContextHold scope](../../crates/gateway/src/context_hold/store.rs) pins the current authorization,
exact route/version, protocol and session, not the aggregate publication revision. Publishing an unrelated
plan preserves continuation; every request still reauthorizes against the current publication and validates
the held binding/profile. The listener journey covers unrelated publication, new user input, changed plan
revision and changed authorization as separate boundaries.

Planner reuses existing eligibility, protocol projection and relay. Regular candidates continue to same-category
primary after exhaustion; primary exhaustion fails. Neither a previous successful model nor opaque provider
state can revive regular or another category after primary selection. Duplicate candidate identities are tried
once. Actual group and candidate position come from the frozen attempt, not list membership.
The active turn's accepted executions also retain primary relay for tool continuations,
even if instruction changes invalidate exact-candidate hold. The derived continuation
records `availability_relay`; it never rewrites the original decision or survives a new user turn.

Observation reuses route/execution facts and the existing stage projection. Category, actual group, candidate
position, model/profile, frozen floor and rubric form precise evidence identity. Stage opening selection records
probability/threshold and reason separately from its eventual score. A later score may update competence but
cannot rewrite an earlier low-score trigger. Summary/drill filters use the same identity; missing/partial scores
stay unrated. Session layout and evidence navigation reuse existing components.
The [model row projection](../../apps/desktop/src/features/plan-quality-state.ts) associates
current published candidates by revision/category/group/declared position and reasoning.
Editor-option model IDs are current source materializations and can differ from retained
execution IDs after source edits. This display association keeps every observed model/profile
summary and its exact drill-down identity; it does not merge statistics by model name.
The shared [stage view](../../apps/desktop/src/features/PlanQualityStage.tsx) shows the
recorded actual group and one-based candidate position even for historical, same-group
or selection-missing stages; the current candidate list is not its source of truth.
The released `observation plan-quality samples` CLI uses this same query and returns
actual `branch_execution.group`/`candidate_index`; public help and schema discovery are
covered by the [CLI entry tests](../../crates/cli/src/lib.rs). Keep the command's
[registry lifecycle](../../crates/application-api/src/commands.rs) and generated manifests
aligned with the handler; an internal codec alone does not expose a public command.
The [website renderer](../../apps/website/src/lib/decision-docs.mjs) embeds the six bilingual
decision READMEs through a closed link map. Its [content preparation](../../apps/website/scripts/prepare-content.mjs)
copies the canonical OpenAPI and examples as downloads. Keep those links and artifacts aligned
when removing extension files; [website contract tests](../../apps/website/tests/decision-docs.test.mjs)
cover rendering and exact download bytes separately from native protocol acceptance.
The [session transcript](../../apps/desktop/src/features/Sessions.tsx) attaches request model labels only
to delivered responses. Replayed input has no inferred model attribution. Reading follows catalog cursors
past pages of hidden reasoning/control metadata without fetching their bodies, preserving evidence access.
Evidence links retain an exact request through refresh/retry and identify its time/model even when
the body begins with replayed history. Desktop keeps the plan editor mounted so the explicit return
to model performance preserves its version, period and stage filter; full-session navigation is explicit.

## Provider boundary

Bailian Token Plan (`decision-model-preview`), Bailian workspace, OpenRouter Jev, TypeSafe and compatible
connections share the typed `model/state/questions` codec. Endpoint, model and limits remain connection
settings. Token Plan has one entry without account-edition branching. See [System One mapping and official
references](../../decision-extensions/api/system-one-design.md). Add a small adapter only if a future provider
actually has an incompatible protocol. Synthetic checks establish contract behavior; real acceptance must
name its exact revision, provider and scenario.

## Public documentation and illustrations

The six bilingual Markdown files under [decision-extensions](../../decision-extensions/README.md)
are rendered directly by [the website decision renderer](../../apps/website/src/lib/decision-docs.mjs).
UI/CLI journeys live in [website guides](../../apps/website/content/guides), with complete terminal
examples in [standalone CLI](../standalone-cli.md). Keep navigation labels distinct from the stable URLs.
The [canonical examples](../../decision-extensions/api/decision-examples.json) own the protocol samples;
`decision-contracts.py` projects current cases into OpenAPI while keeping future tool selection documentation-only.
[Website tests](../../apps/website/tests/decision-docs.test.mjs) compare rendered API JSON with that source.

[Documentation capture](../../apps/desktop/decision-docs.tsx) mounts current product components with
synthetic read-only data. Its [fixture checks](../../apps/desktop/tests/decision-docs-data.test.mjs) and
[browser capture](../../apps/desktop/tests/decision-docs-capture.mjs) validate illustrations, not native or
provider acceptance. Homepage competence images instead come from complete native Desktop windows
with isolated constructed observation records. [Asset provenance](../../decision-extensions/assets/README.md)
records both capture methods; browser captures must not overwrite the native images.
