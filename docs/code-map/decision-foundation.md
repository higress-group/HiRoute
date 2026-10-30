# Decision models, routing and competence

[Code map](README.md) · [Architecture](architecture.md) ·
[Gateway map](../../crates/gateway/README.md) · [Decision API v1](../../decision-extensions/api/README.md)

Built-in decision models supply judgments for HiRoute routing policy. Custom extensions
select a category/group directly; HiRoute owns candidate execution for both. Difficulty routing uses one task scope and regular/primary
groups; custom routing first selects a task category, then its regular/optional
primary group. Tool selection is [protocol documentation only](../../decision-extensions/api/decision-design.md),
with no current runtime or product entry.

## Start with a user path

| Capability | Production owner | Representative evidence |
| --- | --- | --- |
| Publish or diagnose a versioned native applicability bundle | [Versioned fixed content](../../crates/domain/src/routing/system_one_c.rs), [diagnostic schema](../../crates/application-api/src/generated/native_protocol.rs), [request builder](../../crates/gateway/src/core_runtime/classification/c_request.rs) | [v1/v2 bundle and policy identity](../../crates/domain/src/routing/system_one_c_tests.rs), [v1 Operation recovery](../../crates/application/src/operations/tests/native_v1_recovery.rs), [both-version diagnostic and publication path](../../crates/daemon/tests/support/c_protocol_product.py), [version-separated cohorts](../../crates/observation/src/store/tests/plan_quality/routing_observations.rs) |
| Discover CLI presets, securely connect, edit, test or delete a decision model/custom extension | [CLI journey](../../crates/cli/src/quickstart/decision.rs), [shared presets](../../assets/decision-services/providers.v1.json), [terminal credential input](../../crates/cli/src/protected_prompt.rs), [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [shared product transaction](../../crates/product-session/src/session/decision_services.rs), [store](../../crates/local-storage/src/control/decision_services.rs) | [Desktop shell scenarios](../../apps/desktop/tests/v3/browser/product-shell-scenarios.mjs), [real CLI lifecycle](../../crates/daemon/tests/support/decision_services_product.py), [installed secure onboarding](../../crates/daemon/tests/support/quickstart_decision.py) |
| Create a connection while editing a route | [DesktopApp](../../apps/desktop/src/product/DesktopApp.tsx), [selector](../../apps/desktop/src/features/decision-services/DecisionSelector.tsx) | Preserve route edits and select the exact saved revision without implicit publication |
| Configure categories, degree and competence | [Plan editor](../../apps/desktop/src/plan-editor.tsx), [branch editor](../../apps/desktop/src/features/decision-services/BranchRoutingEditor.tsx), [judgment controls](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx), [domain](../../crates/domain/src/routing/decision.rs) | Existing model picker, collapsed advanced settings, complete overrides and publish validation |
| Edit native default/custom assessments | [Assessment intent and released-input adapter](../../crates/domain/src/routing/assessment.rs), [frozen protocol](../../crates/domain/src/routing/system_one_c.rs), [judgment fields](../../apps/desktop/src/features/decision-services/JudgmentSettings.tsx) | [Real CLI next-publication default/custom and restart](../../crates/daemon/tests/support/c_protocol_product.py), [native Session publication/reopen](../../apps/desktop/src-tauri/src/editor_publication_tests.rs), [Desktop interaction](../../apps/desktop/tests/v3/browser/c-protocol-scenarios.mjs) |
| Explicitly test connectivity or applicability | [Connection UI](../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx), [CLI applicability contract](../../crates/application-api/src/classifier_diagnostic_c.rs), [Application](../../crates/application/src/classifier_c.rs), [production diagnostic](../../crates/gateway/src/core_runtime/classification/c_capability.rs) | [CLI request boundary](../../crates/cli/src/control_plane.rs), [focused Windows case](../../crates/daemon/tests/windows_headless.rs): the connection page has one connectivity test; CLI/acceptance owns the separate nineteen-question diagnostic; no implicit provider call |
| Freeze connection, definition and rubric | [Authoring snapshot](../../crates/daemon/src/control/runtime/plan_content_snapshot.rs), [Application compiler](../../crates/application/src/compiler), [Gateway compiler](../../crates/gateway/src/publication/compiler.rs) | [Decision publication contracts](../../crates/gateway/src/publication/tests/decision_contracts.rs), [real publication/restart](../../crates/daemon/tests/publication_process.rs) |
| Judge the current task and preceding actual stage | [Call lifetime](../../crates/gateway/src/core_runtime/classification.rs), [safe transport diagnostics](../../crates/gateway/src/core_runtime/classification/diagnostic.rs), [System One adapter](../../crates/gateway/src/core_runtime/classification/system_one.rs), [custom codec](../../crates/gateway/src/core_runtime/classification/extension.rs) | [Provider mapping](../../crates/gateway/src/core_runtime/classification/system_one_tests.rs), [protocol/replay tests](../../crates/gateway/src/core_runtime/classification/protocol_tests.rs) |
| Select a group and execute candidates | [Group policy](../../crates/gateway/src/core_runtime/classification/group_policy.rs), [planner](../../crates/gateway/src/planner.rs), [history](../../crates/gateway/src/agent_turn_history/store.rs) | [Threshold/score isolation](../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs), [group boundaries](../../crates/gateway/src/planning/branch_tests.rs), [multi-turn listener journey](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Inspect actual execution and competence | [Quality query](../../crates/observation/src/query_v2/plan_quality.rs), [PlanQuality](../../apps/desktop/src/features/PlanQuality.tsx), [stage detail](../../apps/desktop/src/features/PlanQualityStage.tsx) | [Real C execution and retained query](../../crates/daemon/tests/support/c_protocol_product.py), [stage summaries](../../crates/observation/src/store/tests/plan_quality/summary.rs), [UI identities](../../apps/desktop/tests/plan-quality-state.test.mjs), [C counts and coverage UI](../../apps/desktop/tests/v3/browser/c-protocol-scenarios.mjs) |
| Start a model trial or divide a published route | [TrialQuickStart](../../apps/desktop/src/features/trial/TrialQuickStart.tsx), [Plan editor](../../apps/desktop/src/plan-editor.tsx), [division source and draft](../../apps/desktop/src/features/trust/division-draft.ts), [draft disclosure](../../apps/desktop/src/features/trust/DivisionDraftDialog.tsx) | [Trial scenarios](../../apps/desktop/tests/v3/visual/new-model-trial-active.mjs) distinguish built-in protection from custom group selection; the manual division entry depends on published model groups, not threshold suggestions |

## Frozen configuration

The [execution and association contract](../../issue-spec/specs/request-routing/spec.md#requirement-execution-facts-and-history-association-have-separate-authority)
separates request-owned actual facts, bounded history projections, shared protocol
meaning and native evidence. Use it when changing any of the owners below; a cache
miss, unknown source or missing predecessor must not become a global routing switch.

The [publication projection](../../crates/domain/src/publication/projection.rs)
keeps branch/reasoning execution keys distinct from source binding identities.
[Credential admission](../../crates/gateway/src/core_runtime/credentials.rs) and
attempt leases use the frozen source binding to check current Native membership.

[Decision domain types](../../crates/domain/src/routing/decision.rs) own immutable
saved connection revisions and judgment settings; [definition types](../../crates/domain/src/routing/decision_definition.rs)
own categorical/ordinal questions. Plans freeze the selected connection. Built-in execution freezes prompts, thresholds and rubric; new custom execution omits judgment while source drafts keep parked values. Editing a connection or draft does not reinterpret a
published plan or historical stage. Branch overrides are complete copies; a
category without primary candidates needs no degree question.

Desktop calls connections decision models or custom extensions; CLI retains
`decision services list/apply/test`. Writes reuse Operations, protected input and
existing Secret storage. Custom HTTP belongs to a saved connection, not inline
`rest` authoring. Current development v1 replaces the earlier decision shape;
the custom-extension v1 shape has no old-format negotiation. New native publications freeze historical problem applicability in the existing System One
wire envelope. Earlier built-in frozen publications keep their original execution semantics.

Editor input/readback has no native protocol or background selection. Effective
native authoring records C v2 as the current internal protocol, with a fixed eight-turn
background. Compiled execution freezes actual fixed/effective content and digest;
supported C v1 publications and retained Operations resolve their own original bundle.
Legacy omission remains frozen execution support only and is registered for
supported old publications and retained [Operation validation](../../crates/domain/src/operation/routing_content.rs), which checks the original editor against already sealed settings without replaying completed effects. Default assessments resolve to the current default at
the next edit/publication; custom text stays exact. The released-input adapter
recognizes exact legacy default instructions and all criteria. Existing
publications are never rewritten; assessment inheritance and thresholds keep
their existing owners. Desktop has one connection test with the current request
example; CLI/acceptance retains the explicit maximum-shape diagnostic.
The editable native default adds shared [feedback guidance and its highest criterion](../../assets/decision-services/default-assessment-guidance.json),
including explicit acceptance after correction. Frozen validation compares the
sealed assessment rather than resolving the current default again; changing the
default does not rewrite existing C v1/v2 publications or retained Operations.
The separate [C v2 questions](../../crates/domain/src/routing/fixtures/system_one_c_v2.json)
change only applicability, leaving the fixed C v1 bundle untouched. The [baseline publication fixture](../../crates/domain/src/routing/fixtures/baseline-c-default-publication.json)
checks that sealed stock questions and retained Operations keep their original bytes.
Explicit diagnostics accept either exact schema/bundle pair; observations and cohorts
retain the executed version and content identity.

## Provider and routing policy

Bailian, OpenRouter Jev, TypeSafe and compatible connections share the typed
`model/state/questions` adapter. Endpoint/model/limits are connection settings.
See [System One mapping](../../decision-extensions/api/system-one-design.md) for
provider-specific configuration and [custom v1](../../decision-extensions/api/README.md)
for the extension boundary. Add a separate adapter only for an actual incompatible
wire protocol; a provider name alone does not justify one.

The native adapter binds each question to a category, degree or historical assessment, then reduces only the selected path. Custom extensions receive route shape and safe history, choose category/group directly, and own their policy and context trimming. HiRoute retains candidate selection and relay. The custom codec verifies actual preceding-stage Plan/connection identity locally and discards assessment if the response changes category. Trimming inside the assessment target marks that assessment
partial; loss of an earlier prefix does not by itself invalidate the scored stage.

[C allocation](../../crates/gateway/src/core_runtime/classification/c_request.rs)
partitions prior context, actual assessed stage and latest user. It reserves the
third question before target admission, removes optional background first and
never refills history after a partial target frees question bytes. Already-bounded
interrupted/unknown turns stay in the continuous background suffix; execution
status and target-score eligibility are separate. The shared immutable
[native category v1 template](../../crates/domain/src/routing/fixtures/native_category_v1.json)
feeds native compilation; transport consumes the frozen decision.
The [core termination boundary](../../crates/gateway-core/src/runtime/driver/error.rs)
preserves typed cancellation before and after attempt creation. Product completion
keeps cancellation, accepted executions and captured history separate; a complete
capture is not proof that an interrupted stage completed.
Freeze records default/custom assessment intent for every scope before hashing. Legacy native wording
has [its own current definition](../../crates/domain/src/routing/legacy_category.rs), independent of the C fixture. C applicability
only gates an otherwise eligible low-score protection opportunity. Invalid or
unavailable applicability retains that opportunity conservatively; category,
degree, rubric validity, inheritance and relay retain their separate owners.

For built-in connections, `group_policy.rs` uses the current simple probability and applicable complete
assessment for the same category, plan revision and rubric. Valid simple and complex
base selections both enter the eligible historical gate; suppression returns to that
base, while a prior Primary stage remains an actual Primary trigger. Each new user turn
runs a fresh decision: it neither preserves an old upgrade cursor nor blindly
returns to regular. Missing/partial scores remain unrated. Recognized tool
continuations reuse the frozen decision only while history and authority permit.
[ContextHold](../../crates/gateway/src/context_hold/store.rs) binds exact route,
version, protocol, authorization and session; unrelated plan publication does not
invalidate that route, and each request still reauthorizes.
[Optional history admission](../../crates/gateway/src/core_runtime/decision_history.rs)
derives execution scope from request authority independently of ContextHold.
[Branch resolution](../../crates/gateway/src/agent_turn_history/store/branches.rs)
uses a bounded scope index and the [accepted response boundary](../../crates/gateway/src/agent_turn_history/boundary.rs).
[Opening evidence](../../crates/gateway/src/agent_turn_history/store/opening.rs)
compares the full old request separately from that response edge. Both reused
and independent branches require an actually appended input to establish a new
activation; a native identifier change alone cannot close a tool stage. Its
[regressions](../../crates/gateway/src/agent_turn_history/opening_evidence_tests.rs)
cover interrupted output, concurrent retries, input windows and missing measurements.
They also vary input and identifier independently: repeated IDs cannot hide new
input, while explicit continuation evidence makes current input unavailable;
a new-message cause does not authorize predecessor reuse or repair the old score.
Request-local model setup remains in strict ContextHold, outside execution-prefix
identity. A unique visible response projection can retain actual stage capture
when retained native history is only partially modeled: [ingress coverage](../../crates/gateway/src/model_ir/native_history.rs)
binds unknown prior items in full and checks only the accepted-output suffix.
The [coverage regressions](../../crates/gateway/src/agent_turn_history/boundary/coverage_tests.rs)
separate old unknown content, new unknown output and new User input across protocols.
A unique visible response projection also retains actual stage capture
when plaintext reasoning is omitted, with sticky association uncertainty and no
affinity hint; [projection regressions](../../crates/gateway/src/agent_turn_history/boundary/projection_tests.rs)
cover competing branches, old-prefix changes and frozen membership.
The same boundary owner has a separate full ordered plaintext-encoding proof for
missing versus empty function encoding, with contextual uncertainty and unchanged
exact digests. [Encoding comparisons](../../crates/gateway/src/agent_turn_history/boundary/function_encoding_compat_tests.rs)
and [concurrent controls](../../crates/gateway/src/agent_turn_history/concurrent_encoding_tests.rs)
bind complete tool identity/arguments, reasoning and ordering without changing branch ranking.
Optional content comparisons use the shared [fingerprint budget](../../crates/gateway/src/context_hold/fingerprint.rs)
before Replay reads and JSON key sorting; exhausting it loses evidence, not routing.
The [Replay range owner](../../crates/gateway/src/replay.rs) records canonical JSON
provenance. [JSON boundary cases](../../crates/gateway/src/agent_turn_history/boundary/json_reference_budget_tests.rs)
cover raw-byte prefix association after normalization refusal, without granting
canonical output equivalence or preventing later tool-stage recovery.
The [layout capacity regression](../../crates/gateway/src/core_runtime/decision_history/layout_budget_tests.rs)
refuses actual optional allocation, preserves captured execution and verifies a
complete later tool stage after capacity recovers.
Matching input text alone never deduplicates requests or proves a predecessor.
Concurrent requests receive separate tickets; frozen snapshots retain their own
accounted payload and exact target membership. A busy or stale affinity cache loses
only the hint. Unknown association permits disclosed contextual assessment of actual
same-scope execution, without advancing a verified branch or importing client output.
[Concurrency regressions](../../crates/gateway/src/agent_turn_history/concurrent_requests_tests.rs)
cover different actual outputs, overlap, frozen targets and capacity recovery;
[runtime admission regressions](../../crates/gateway/src/core_runtime/decision_history/tests.rs)
exercise optional cache faults. [Gap bookkeeping](../../crates/gateway/src/agent_turn_history/store/gap.rs)
uses bounded scope-hash stripes; a collision is conservative and recoverable.

[Native normalization](../../crates/gateway/src/context_hold/routing.rs) admits
known purpose independently from optional enrichment. Known auxiliary calls never
consume main history slots. The [DSH emitter](../../assets/agents/dsh/hiroute-routing.mjs)
also follows isolated LLM services while retaining its managed transport gate.
The [v2 codec](../../crates/gateway/src/context_hold/routing/v2.rs)
validates the optional native cursor and final-wire input window. The
[current input view](../../crates/gateway/src/core_runtime/native_routing_view.rs)
selects current content without changing any original message role.
Validated input windows also retain their canonical first/last positions through
capture admission, so trailing native context cannot create a false missing turn.
Earlier uncaptured input and output after the current window still fence or mark
the affected stage partial; see the [window regressions](../../crates/gateway/src/core_runtime/decision_history/window_event_tests.rs).
Legacy Hermes clean-text hints cannot choose a current input instance.
Codex [native metadata](../../crates/gateway/src/context_hold/routing/codex.rs)
recognizes known auxiliary purpose before requiring turn identity; its
[peer codec](../../crates/gateway/src/core_runtime/native_routing_view/codex.rs)
projects a readable native trigger at its actual canonical position while keeping
typed original peer bytes for evidence. [Codex regressions](../../crates/gateway/src/core_runtime/native_routing_view/codex_tests.rs)
cover peer-only, multiple-peer, opaque and shifted native positions. Source uncertainty,
association uncertainty and missing capture remain distinct; the existing classifier
content vocabulary discloses them without disabling evaluation or changing the
custom v1 schema. Unavailable known continuation input permits temporary fallback;
subsequent actual A/B must recover within the same native session. Neither a
compression link nor a native turn ID reconstructs discarded execution after restart.
The v2 `existing_turn` input descriptor applies that request-local guard while
retaining main execution output; a compression cursor alone does not apply it.
The [listener recovery regression](../../tools/e2e-harness/tests/p0_gateway_runtime/native_existing_turn.rs)
covers cold and previously captured sessions followed by actual A/B assessment.
[Continuation regressions](../../crates/gateway/src/core_runtime/decision_history/native_tests.rs)
keep summaries and unproved tool outcomes out of accepted history. The
[listener availability cases](../../tools/e2e-harness/tests/p0_gateway_runtime/native_routing_availability.rs)
and [fault cases](../../tools/e2e-harness/tests/p0_gateway_runtime/history_bypass.rs)
exercise the production boundary. Real client loading remains a separate gate.

[Sealed records](../../crates/gateway/src/agent_turn_history/store/records.rs) retain
actual model/profile/rubric attribution. New input may recover from an old partial
stage; retries cannot repair missing output. A routing activation or delivered
response is not proof of logical task completion. Observation
[turn projection](../../crates/observation/src/analysis/turns.rs),
[timeline](../../crates/observation/src/query_v2/routing_timeline/projection.rs) and
[quality receipts](../../crates/observation/src/receipt/plan_quality.rs) consume exact
versioned request membership for new facts and retain the old reader for old bytes.
[Composite evaluation](../../crates/gateway/src/agent_turn_history/store/evaluation.rs)
keeps a common frozen rubric when one stage uses several models. Its
[stage assessment](../../crates/domain/src/observation/stage_evaluation.rs) has no
model identity and cannot credit a single-model capability aggregate. Missing
attribution alone is not a capture gap. The request-owned
[input position](../../crates/domain/src/observation/input_selection.rs) lets
excerpt readers select the original input without reinterpreting late context.
The [catalog reader](../../crates/observation/src/query_v2/input_selection.rs)
authenticates this optional evidence within a bounded request-local read.
[Admission identity](../../crates/gateway/src/agent_turn_history/store.rs)
is emitted before classification and independently of affinity; the
[pending-stage projection](../../crates/observation/src/query_v2/routing_timeline/admission.rs)
groups interleaved requests by actual Gateway stage and producer epoch.
Admission never supplies completion or assessment membership. A later admission
reopens the view without changing the frozen target; delayed closure revisions
cannot roll it back. [Admission regressions](../../crates/observation/src/store/tests/plan_quality/stage_admission.rs)
cover pending branches, reopen, conflicts and request termination. The
[listener-to-observation bridge](../../tools/e2e-harness/tests/p0_gateway_runtime/observed_history.rs)
feeds actual Gateway envelopes through the production adapter, persistence and
reopened timeline query; encrypted tool retries and cancellation exercise it.
[Interleaved membership regressions](../../crates/observation/src/store/tests/plan_quality/history_membership.rs)
check reopen and prohibit a late execution from extending a frozen score,
including clipped or incomplete current membership whose retained IDs still
match the older assessment. The [quality reader](../../crates/observation/src/query_v2/plan_quality.rs)
derives current coverage without overwriting the frozen score.
The [ingress adapters](../../crates/gateway/src/adapters/ingress.rs) normalize
supported protocol history into the same representation before prefix checks;
Chat reasoning aliases share one state while their native fields remain intact.


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
The opportunity denominator counts recorded eligible low-score gate actions, including
complex decisions, without reinterpreting older not-needed decisions. [Replay](../../crates/application/src/trust/replay.rs)
keeps recorded protection at Primary across threshold previews.
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

The session timeline uses a [read-specific selection](../../crates/domain/src/observation/routing_timeline.rs)
to expose opening historical-protection evidence while preserving the stored decision.
Its [projection](../../crates/observation/src/query_v2/routing_timeline/projection.rs)
counts explicit tool-continuation reasons independently of model affinity;
[retained timeline checks](../../crates/observation/src/store/tests/plan_quality/routing_timeline_protection.rs)
cover pending, closed and reopened views.
The [protection evidence guard](../../apps/desktop/src/features/trust/protection-evidence.ts)
checks complete matching Plan, revision, time and branch scope before placing a
released-decision count beside protected stages.
The shared [Plan performance component](../../apps/desktop/src/features/PlanQuality.tsx)
pins both reads to a loaded publication or resolves the active revision for entries
without one. Its [trust read](../../apps/desktop/src/features/trust/use-plan-trust.ts)
exposes only the current request scope; pending resolution never means all versions.
The [session entry regressions](../../apps/desktop/tests/v3/browser/session-window-scenarios.mjs)
cover actual modal navigation, zero releases, revision changes and failed-read recovery.
Product requirements belong to
[smart-saving routing](../../issue-spec/specs/smart-saving-routing/spec.md) and
[session history](../../issue-spec/specs/session-history/spec.md).

The [suggestion read](../../crates/application/src/observation_query/control/trust.rs)
keeps its bounded work budget and returns deferred Plan IDs separately from real
read failures. The [visible-view loader](../../apps/desktop/src/features/trust/use-trust-suggestions.ts)
continues those reads with bounded backoff and pauses while hidden/inactive;
[Attention](../../apps/desktop/src/features/trust/TrustAttention.tsx) distinguishes
pending, unavailable and a complete empty result. Missing pending fields from an
older complete response retain their original meaning.

Custom observations carry extension identity and an optional bounded reason without a competence floor. Complete scores remain scored coverage and may enter averages; competence rates use only complete stages with a real floor, and custom protocol decisions never enter threshold replay. Current custom execution requires absent judgment; Domain Plan validation and Gateway snapshot installation reject the old present-judgment shape. [Current publication/Operation scenarios](../../crates/application/src/routing/authoring/tests.rs) preserve parked source exactly and reject inconsistent sealed inputs, without a custom-only normalization adapter. Native frozen validation and the common [observation boundaries](../../crates/observation/src/store/tests/plan_quality/timeline_boundaries.rs) retain supported historical bytes and reject new authority under old schemas.
