# Decision models, routing and competence

[Code map](README.md) · [Architecture](architecture.md) ·
[Gateway map](../../crates/gateway/README.md) · [Decision API v1](../../decision-extensions/api/README.md)

Built-in decision models and custom extensions supply judgments. HiRoute owns
routing policy and execution. Difficulty routing uses one task scope and regular/primary
groups; custom routing first selects a task category, then its regular/optional
primary group. Tool selection is [protocol documentation only](../../decision-extensions/api/decision-design.md),
with no current runtime or product entry.

## Start with a user path

| Capability | Production owner | Representative evidence |
| --- | --- | --- |
| Discover CLI presets, securely connect, edit, test or delete a decision model/custom extension | [CLI journey](../../crates/cli/src/quickstart/decision.rs), [shared presets](../../assets/decision-services/providers.v1.json), [terminal credential input](../../crates/cli/src/protected_prompt.rs), [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [shared product transaction](../../crates/product-session/src/session/decision_services.rs), [store](../../crates/local-storage/src/control/decision_services.rs) | [Desktop shell scenarios](../../apps/desktop/tests/v3/browser/product-shell-scenarios.mjs), [real CLI lifecycle](../../crates/daemon/tests/support/decision_services_product.py), [installed secure onboarding](../../crates/daemon/tests/support/quickstart_decision.py) |
| Create a connection while editing a route | [DesktopApp](../../apps/desktop/src/product/DesktopApp.tsx), [selector](../../apps/desktop/src/features/decision-services/DecisionSelector.tsx) | Preserve route edits and select the exact saved revision without implicit publication |
| Configure categories, degree and competence | [Plan editor](../../apps/desktop/src/plan-editor.tsx), [branch editor](../../apps/desktop/src/features/decision-services/BranchRoutingEditor.tsx), [judgment controls](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx), [domain](../../crates/domain/src/routing/decision.rs) | Existing model picker, collapsed advanced settings, complete overrides and publish validation |
| Edit native default/custom assessments | [Assessment intent and released-input adapter](../../crates/domain/src/routing/assessment.rs), [frozen protocol](../../crates/domain/src/routing/system_one_c.rs), [judgment fields](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx) | [Real CLI next-publication default/custom and restart](../../crates/daemon/tests/support/c_protocol_product.py), [native Session publication/reopen](../../apps/desktop/src-tauri/src/editor_publication_tests.rs), [Desktop interaction](../../apps/desktop/tests/v3/browser/c-protocol-scenarios.mjs) |
| Explicitly test connectivity or applicability | [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [CLI applicability contract](../../crates/application-api/src/classifier_diagnostic_c.rs), [Application](../../crates/application/src/classifier_c.rs), [production diagnostic](../../crates/gateway/src/core_runtime/classification/c_capability.rs) | [CLI request boundary](../../crates/cli/src/control_plane.rs), [focused Windows case](../../crates/daemon/tests/windows_headless.rs): the connection page has one connectivity test; CLI/acceptance owns the separate nineteen-question diagnostic; no implicit provider call |
| Freeze connection, definition and rubric | [Authoring snapshot](../../crates/daemon/src/control/runtime/plan_content_snapshot.rs), [Application compiler](../../crates/application/src/compiler), [Gateway compiler](../../crates/gateway/src/publication/compiler.rs) | [Decision publication contracts](../../crates/gateway/src/publication/tests/decision_contracts.rs), [real publication/restart](../../crates/daemon/tests/publication_process.rs) |
| Judge the current task and preceding actual stage | [Call lifetime](../../crates/gateway/src/core_runtime/classification.rs), [safe transport diagnostics](../../crates/gateway/src/core_runtime/classification/diagnostic.rs), [System One adapter](../../crates/gateway/src/core_runtime/classification/system_one.rs), [custom codec](../../crates/gateway/src/core_runtime/classification/protocol.rs) | [Provider mapping](../../crates/gateway/src/core_runtime/classification/system_one_tests.rs), [protocol/replay tests](../../crates/gateway/src/core_runtime/classification/protocol_tests.rs) |
| Select a group and execute candidates | [Group policy](../../crates/gateway/src/core_runtime/classification/group_policy.rs), [planner](../../crates/gateway/src/planner.rs), [history](../../crates/gateway/src/agent_turn_history/store.rs) | [Threshold/score isolation](../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs), [group boundaries](../../crates/gateway/src/planning/branch_tests.rs), [multi-turn listener journey](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Inspect actual execution and competence | [Quality query](../../crates/observation/src/query_v2/plan_quality.rs), [PlanQuality](../../apps/desktop/src/features/PlanQuality.tsx), [stage detail](../../apps/desktop/src/features/PlanQualityStage.tsx) | [Real C execution and retained query](../../crates/daemon/tests/support/c_protocol_product.py), [stage summaries](../../crates/observation/src/store/tests/plan_quality/summary.rs), [UI identities](../../apps/desktop/tests/plan-quality-state.test.mjs), [C counts and coverage UI](../../apps/desktop/tests/v3/browser/c-protocol-scenarios.mjs) |

## Frozen configuration

The [publication projection](../../crates/domain/src/publication/projection.rs)
keeps branch/reasoning execution keys distinct from source binding identities.
[Credential admission](../../crates/gateway/src/core_runtime/credentials.rs) and
attempt leases use the frozen source binding to check current Native membership.

[Decision domain types](../../crates/domain/src/routing/decision.rs) own immutable
saved connection revisions and judgment settings; [definition types](../../crates/domain/src/routing/decision_definition.rs)
own categorical/ordinal questions. Plans freeze the selected connection, prompts,
thresholds and rubric. Editing a connection or draft does not reinterpret a
published plan or historical stage. Branch overrides are complete copies; a
category without primary candidates needs no degree question.

Desktop calls connections decision models or custom extensions; CLI retains
`decision services list/apply/test`. Writes reuse Operations, protected input and
existing Secret storage. Custom HTTP belongs to a saved connection, not inline
`rest` authoring. Current development v1 replaces the earlier decision shape;
the custom-extension v1 shape has no old-format negotiation. New native publications freeze historical problem applicability in the existing System One
wire envelope. Earlier frozen publications keep their original execution semantics.

Editor input/readback has no native protocol or background selection. Effective
native authoring records the current internal protocol, with a fixed eight-turn
background. Compiled execution freezes actual fixed/effective content and digest.
Legacy omission remains frozen execution support only and is registered for
supported old publications and retained [Operation validation](../../crates/domain/src/operation/routing_content.rs), which checks the original editor against already sealed settings without replaying completed effects. Default assessments resolve to the current default at
the next edit/publication; custom text stays exact. The released-input adapter
recognizes exact legacy default instructions and all criteria. Existing
publications are never rewritten; assessment inheritance and thresholds keep
their existing owners. Desktop has one connection test with the current request
example; CLI/acceptance retains the explicit maximum-shape diagnostic.

## Provider and routing policy

Bailian, OpenRouter Jev, TypeSafe and compatible connections share the typed
`model/state/questions` adapter. Endpoint/model/limits are connection settings.
See [System One mapping](../../decision-extensions/api/system-one-design.md) for
provider-specific configuration and [custom v1](../../decision-extensions/api/README.md)
for the extension boundary. Add a separate adapter only for an actual incompatible
wire protocol; a provider name alone does not justify one.

The adapter binds each question to a category, degree or historical assessment,
then reduces only the selected path. The extension owns provider integration and
context trimming; HiRoute owns probability thresholds, competence protection and
candidate selection. Trimming inside the assessment target marks that assessment
partial; loss of an earlier prefix does not by itself invalidate the scored stage.

[C allocation](../../crates/gateway/src/core_runtime/classification/c_request.rs)
partitions prior context, actual assessed stage and latest user. It reserves the
third question before target admission, removes optional background first and
never refills history after a partial target frees question bytes. Already-bounded
interrupted/unknown turns stay in the continuous background suffix; execution
status and target-score eligibility are separate. The shared immutable
[native category v1 template](../../crates/domain/src/routing/fixtures/native_category_v1.json)
feeds native compilation; transport consumes the frozen decision.
Freeze records default/custom assessment intent for every scope before hashing. Legacy native wording
has [its own current definition](../../crates/domain/src/routing/legacy_category.rs), independent of the C fixture. C applicability
only gates an otherwise eligible low-score protection opportunity. Invalid or
unavailable applicability retains that opportunity conservatively; category,
degree, rubric validity, inheritance and relay retain their separate owners.

`group_policy.rs` uses the current simple probability and applicable complete
assessment for the same category, plan revision and rubric. Each new user turn
runs a fresh decision: it neither preserves an old upgrade cursor nor blindly
returns to regular. Missing/partial scores remain unrated. Recognized tool
continuations reuse the frozen decision only while history and authority permit.
[ContextHold](../../crates/gateway/src/context_hold/store.rs) binds exact route,
version, protocol, authorization and session; unrelated plan publication does not
invalidate that route, and each request still reauthorizes. Message-prefix continuity is
separate from system-instruction validity: [history entry](../../crates/gateway/src/context_hold/store.rs)
distinguishes continuing, rewritten and unavailable checkpoints. [Turn history](../../crates/gateway/src/agent_turn_history/store.rs)
records the opening cause on completion; [timeline projection](../../crates/observation/src/query_v2/routing_timeline/projection.rs)
prefers it over legacy ledger inference. [Boundary regressions](../../crates/observation/src/store/tests/plan_quality/timeline_boundaries.rs)
cover persistence, unknown evidence and old canonical bytes.

The planner always prefers a valid held candidate within the newly selected group,
including an in-group backup; retired follow-up fields survive only at serialized
compatibility boundaries. It reuses eligibility, protocol projection and bounded relay. Regular
candidates may relay to the same category's primary group; primary exhaustion
fails. Duplicate candidate identities are tried once. Same-turn continuations can
retain an actual primary availability relay, but that fact neither rewrites the
original decision nor survives a new user turn.

## Observation and public consumers

[Execution receipts](../../crates/observation/src/receipt/plan_quality.rs) and
[quality queries](../../crates/observation/src/query_v2/plan_quality.rs) own evidence
identity: actual category/group/candidate position, model/reasoning, plan revision
and frozen rubric. Opening selection probability/reason is distinct from a later
competence score. Summary and drill-down use the same identity; missing or partial
assessments do not enter averages.

[All-decision C summaries](../../crates/observation/src/query_v2/routing_observations.rs)
read the existing route events and safe-fact projection, independently of stage
pages and score/model filters. The existing observation query v2 opts in with `include_routing_observations`;
omission preserves released fields, authorization and cursor bindings. The [Application reader](../../crates/application/src/observation_query/control/v2.rs) retains the released identity for existing queries under current Local Control admission.
[Product presentation](../../apps/desktop/src/features/RoutingObservations.tsx)
shows counts, denominators, cohorts and coverage; it does not infer accuracy.
Every current classifier decision records its protocol. Known non-C decisions stay
separate, so mixed current scopes retain rates; authenticated supported pre-applicability records also count as known non-C. The existing bounded projection refresh retains format/routing/time evidence from verified carriers; unproven records and projection gaps remain partial. Plan identity alone never proves a protocol. Content-only deletion retains safe evidence; full fact retention/deletion removes it.

The [model row projection](../../apps/desktop/src/features/plan-quality-state.ts)
associates configured candidates with retained execution by declared position and
reasoning, not display name or a freshly materialized model ID. The shared stage
view reads recorded execution facts even if the current plan has changed.
The `observation plan-quality samples` CLI exposes the same query; its
[command registry](../../crates/application-api/src/commands.rs), generated manifests
and [CLI entry tests](../../crates/cli/src/lib.rs) must remain aligned.

| Published surface | Canonical owner / check |
| --- | --- |
| Custom extension protocol and samples | [API guide](../../decision-extensions/api/README.md), [examples](../../decision-extensions/api/decision-examples.json), [generator](../../scripts/decision-contracts.py), [OpenAPI](../../decision-extensions/api/decision.openapi.json) |
| Official optional Jev service | [Extension](../../decision-extensions/extensions/jev-decider), with the same v1 contract |
| Website decision pages and downloads | [Renderer](../../apps/website/src/lib/decision-docs.mjs), [content preparation](../../apps/website/scripts/prepare-content.mjs), [website checks](../../apps/website/tests/decision-docs.test.mjs) |
| UI/CLI instructions | [Website guides](../../apps/website/content/guides), [standalone CLI](../standalone-cli.md) |
| Routing cost explanations and FAQ | [Bilingual articles](../../news/README.md), [homepage](../../apps/website/src/components/HomePage.astro), [frozen-evidence cache calculation](../../experiments/cache_cost_analysis.py) |
| Illustrations and native screenshots | [Component capture](../../apps/desktop/decision-docs.tsx), [fixture checks](../../apps/desktop/tests/decision-docs-data.test.mjs), [asset provenance](../../decision-extensions/assets/README.md) |

Documentation fixtures and native windows with constructed observation records
illustrate the product. Neither capture method proves live-provider behavior or
answer quality; acceptance records must identify the actual revision and scenario.
