# Decision models, routing and competence

[Code map](README.md) · [Architecture](architecture.md) ·
[Gateway map](../../crates/gateway/README.md) · [Decision API v1](../../decision-extensions/api/README.md)

Built-in decision models and custom extensions supply judgments. HiRoute owns
routing policy and execution. Smart saving uses one task scope and economy/primary
groups; custom routing first selects a task category, then its regular/optional
primary group. Tool selection is [protocol documentation only](../../decision-extensions/api/decision-design.md),
with no current runtime or product entry.

## Start with a user path

| Capability | Production owner | Representative evidence |
| --- | --- | --- |
| Connect, edit, test or delete a decision model/custom extension | [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [native transaction](../../apps/desktop/src-tauri/src/session/decision_services.rs), [store](../../crates/local-storage/src/control/decision_services.rs) | [Desktop shell scenarios](../../apps/desktop/tests/v3/browser/product-shell-scenarios.mjs), [real CLI lifecycle](../../crates/daemon/tests/support/decision_services_product.py) |
| Create a connection while editing a route | [DesktopApp](../../apps/desktop/src/product/DesktopApp.tsx), [selector](../../apps/desktop/src/features/decision-services/DecisionSelector.tsx) | Preserve route edits and select the exact saved revision without implicit publication |
| Configure categories, degree and competence | [Plan editor](../../apps/desktop/src/plan-editor.tsx), [branch editor](../../apps/desktop/src/features/decision-services/BranchRoutingEditor.tsx), [judgment controls](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx), [domain](../../crates/domain/src/routing/decision.rs) | Existing model picker, collapsed advanced settings, complete overrides and publish validation |
| Freeze connection, definition and rubric | [Authoring snapshot](../../crates/daemon/src/control/runtime/plan_content_snapshot.rs), [Application compiler](../../crates/application/src/compiler), [Gateway compiler](../../crates/gateway/src/publication/compiler.rs) | [Decision publication contracts](../../crates/gateway/src/publication/tests/decision_contracts.rs), [real publication/restart](../../crates/daemon/tests/publication_process.rs) |
| Judge the current task and preceding actual stage | [Call lifetime](../../crates/gateway/src/core_runtime/classification.rs), [safe transport diagnostics](../../crates/gateway/src/core_runtime/classification/diagnostic.rs), [System One adapter](../../crates/gateway/src/core_runtime/classification/system_one.rs), [custom codec](../../crates/gateway/src/core_runtime/classification/protocol.rs) | [Provider mapping](../../crates/gateway/src/core_runtime/classification/system_one_tests.rs), [protocol/replay tests](../../crates/gateway/src/core_runtime/classification/protocol_tests.rs) |
| Select a group and execute candidates | [Group policy](../../crates/gateway/src/core_runtime/classification/group_policy.rs), [planner](../../crates/gateway/src/planner.rs), [history](../../crates/gateway/src/agent_turn_history/store.rs) | [Threshold/score isolation](../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs), [group boundaries](../../crates/gateway/src/planning/branch_tests.rs), [multi-turn listener journey](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Inspect actual execution and competence | [Quality query](../../crates/observation/src/query_v2/plan_quality.rs), [PlanQuality](../../apps/desktop/src/features/PlanQuality.tsx), [stage detail](../../apps/desktop/src/features/PlanQualityStage.tsx) | [Stage summaries](../../crates/observation/src/store/tests/plan_quality/summary.rs), [UI identities](../../apps/desktop/tests/plan-quality-state.test.mjs) |

## Frozen configuration

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
there is no additional protocol version or old decision-format recovery reader.

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

`group_policy.rs` uses the current simple probability and applicable complete
assessment for the same category, plan revision and rubric. Each new user turn
runs a fresh decision: it neither preserves an old upgrade cursor nor blindly
returns to regular. Missing/partial scores remain unrated. Recognized tool
continuations reuse the frozen decision only while history and authority permit.
[ContextHold](../../crates/gateway/src/context_hold/store.rs) binds exact route,
version, protocol, authorization and session; unrelated plan publication does not
invalidate that route, and each request still reauthorizes.

The planner reuses eligibility, protocol projection and bounded relay. Regular
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
| Illustrations and native screenshots | [Component capture](../../apps/desktop/decision-docs.tsx), [fixture checks](../../apps/desktop/tests/decision-docs-data.test.mjs), [asset provenance](../../decision-extensions/assets/README.md) |

Documentation fixtures and native windows with constructed observation records
illustrate the product. Neither capture method proves live-provider behavior or
answer quality; acceptance records must identify the actual revision and scenario.
