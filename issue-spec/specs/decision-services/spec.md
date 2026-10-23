# Decision connections and custom branches

## Purpose

Saved decision connections supply task judgments and optional assessment of prior
execution. Built-in connections supply judgments for HiRoute group policy; custom extensions directly choose category/group. HiRoute owns candidate execution and attribution for both.

## Requirements

### Requirement: Connections are independently managed immutable revisions

Desktop's Models area MUST separate general models from decision models, including
clearly typed custom extensions, in the common list/detail flow. Built-in connection
configuration owns provider, model, full endpoint, auth, and timeout; category/degree/
competence instructions belong to the plan. OpenRouter, TypeSafe, Bailian Token Plan,
Bailian business-space, and compatible endpoints use the same System One codec.
Provider names alone do not define another protocol. Token Plan cannot be split
into individual/team eligibility paths. Known values are editable suggestions;
required business-space/custom endpoint/model fields remain visible.

Auth supports none, Bearer, or a declared header as appropriate. Plaintext uses
protected input/Secret transactions, never plan content, logs, argv, lists, or browser
storage. Existing auth shows configured state without guessing its type from the
header name. Changing provider or endpoint origin clears pending auth/old references.
Edits preserve input and do not reset a saved connection by changing its type.

Save uses service revision CAS and creates an immutable version; auth replacement
creates a new protected reference rather than modifying the old one. Plans may
explicitly select an available saved historical revision even after a newer save.
Publish verifies exact saved ID/revision/content and rejects unsaved or different
same-version copies. Connection/draft changes cannot reinterpret published plans.
Referenced versions/drafts prevent deletion. Inline legacy `rest` authoring is not
a current public alternative.

### Requirement: Testing is explicit, version-specific, and uses production transport

Save-and-test MUST save first, then test the exact saved version with synthetic
input through the actual transport/parser. Save-only makes no call. Test failure
reports that the configuration was saved but the test failed; it is not a rollback
or ongoing health/answer-quality verdict. Tests do not read real conversation history,
invoke a business model, or create competence samples.

HTTP/2 authority, total deadline, DNS/transport policy, authentication, and no-redirect
rules are shared with execution. Closed error codes distinguish recognized auth,
quota, and endpoint rejections without native text, credentials, account identifiers,
or request body. A generic rejection cannot be declared a bad Key.

### Requirement: A custom plan selects one category and at most two groups

Custom branches MUST contain 2–16 unordered categories, each with stable ID, name,
natural-language task condition, regular candidates, and optional primary candidates.
For built-in decisions, primary presence enables the product's simple/complex degree question; a single-group
category needs none. `smart_saving` is reserved for the built-in task scope and is
rejected in custom authoring/materialization.

The protocol can express categorical choices and ordered levels, but the current
product offers only two degree levels. It does not expose arbitrary trees, parallel
category execution, category-confidence thresholds, priorities, or tool-selection
controls. Overlapping conditions select the current principal intent. No match
selects the explicit default category as a valid decision with its degree result;
this is different from decision-call failure.

Category text, and effective built-in degree/rubric text, must be legal nonempty text for publication without an
independent 4096-character field cap. Drafts can retain incomplete text. Whole-request
resource/transport/provider limits remain enforced without silent prompt truncation.

Custom extensions receive published category IDs, conditions and available groups; they receive no degree/rubric text. A single-group extension category always executes regular regardless of a returned group.

#### Scenario: Single-group extension category

- **WHEN** a custom extension selects a published category with regular candidates only
- **THEN** HiRoute MUST execute regular without requiring a degree question or accepting another category.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246369555

### Requirement: Group policy consumes only applicable current assessment

For a dual-group category under the legacy native protocol,
regular requires current simple probability at least its threshold and no applicable
complete current assessment strictly below the competence floor.
Defaults are 0.8 and 0.5; finite 0..1 values and equality behavior follow
[smart saving](../smart-saving-routing/spec.md). Protection applies only to the
preceding actual stage when category, plan revision, and frozen policy/rubric agree.
Cross-category, partial, missing, malformed, mixed-model, or unbound assessment does
not drive escalation. Historical observation is not permission to reuse an old score.
Single-group categories record assessment without upgrading, and primary low scores
cannot create a third tier or change category.

Each new user turn rejudges category/degree; same-turn tool continuation/replay
inherits a frozen decision while history/authority remain continuous. A valid ContextHold
MUST always prefer its candidate within the newly selected group, independently
of any saved follow-up field; it cannot override a new category or group. Regular exhaustion may
relay into the same category's primary group; direct primary selection stays there.
No cross-category relay or separate failure list is added. Exact candidates already
failed in the same relay are not attempted twice. Model/provider failures do not
become competence scores. Actual category/group/index/Binding/profile identity is
recorded directly, including a model configured in both groups.

For native C, applicability MUST gate only an otherwise eligible historical
low-score protection opportunity. Preserve the existing exact Plan/revision,
category, actual single-model attribution, rubric/policy and complete valid score
guards. If P(simple) is below T, primary wins independently. With P(simple) at
least T and eligible historical s below F, a valid applicability probability at
least 0.5 retains primary protection; below 0.5 suppresses that historical trigger
and allows regular. Missing/unavailable applicability conservatively retains the
eligible trigger. A missing internally required question is separately reported,
not counted as a provider request/failure. No score is rewritten or transferred,
and applicability cannot rescue invalid/missing assessment or select a category.
Same-turn inheritance, ContextHold, candidate ordering and availability relay
retain their existing rules. C is inapplicable to custom-extension v1 and heuristics.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235110154

For a custom extension, HiRoute MUST execute the returned category/group without simple thresholds, competence floors or historical protection. Its optional same-category assessment is observation only. Single-group categories ignore the returned group. These decisions retain the same continuation, ContextHold, candidate and relay boundaries.

#### Scenario: A new user turn keeps the selected group

- **WHEN** using a built-in decision connection, a custom-branch follow-up selects the same group and its held candidate remains valid
- **THEN** the Gateway MUST prefer that candidate while retaining the fresh category/degree decision.

#### Scenario: Unresolved problem remains relevant

- **WHEN** using a built-in decision connection, P(simple) meets T, a complete eligible score is below F and applicability equals 0.5
- **THEN** Primary protection MUST apply; equality does not suppress it.

#### Scenario: Problem no longer governs current work

- **WHEN** using a built-in decision connection, all historical protection guards hold and applicability is below 0.5
- **THEN** The historical trigger MUST be suppressed and the regular group selected without altering the raw score.

#### Scenario: Applicability is unavailable

- **WHEN** using a built-in decision connection, a low-score protection opportunity has no valid applicability result
- **THEN** Protection MUST be retained conservatively, with the unavailability reason recorded.

#### Scenario: Extension selection is authoritative

- **WHEN** an extension returns regular with a complete low score for the preceding same-category stage
- **THEN** HiRoute MUST execute regular without applying threshold or protection policy and may record the score for observation.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370180

### Requirement: System One compiles independent typed questions in one call

Built-in request shape uses `model`, `state`, and `questions`. Categorical selection
is a Choice, each applicable degree is an independent Score, and prior-stage
competence is another Score. Instructions define each question's target without
relying on question-ID semantics or another answer. Only the selected category's
degree is consumed; invalid answers on unused paths cannot poison the chosen path.
Degree probabilities map by level ID. Competence is the raw 0..2 score divided by
2, not reconstructed from confidence or rounded probabilities. The three rubric
anchors remain 0/0.5/1. No absent free-text explanation is invented.

The call transmits current user input and the bounded safe visible history needed
for judgment/assessment to the explicitly selected endpoint. Current user content
cannot be silently truncated. History trimming marks the affected evidence partial;
full-history gaps and assessed-stage gaps remain independent. Missing pre-integration
prefix may set `history_partial` without making a completely captured target stage
partial; missing target response or target trimming still invalidates protection.

The native applicability protocol MUST freeze its actual fixed bundle,
effective degree/category definitions, every composed per-scope assessment,
thresholds, context cap and content digest in the published route. Execution uses
that content without substituting binary defaults. Stock C_full questions and
criteria MUST exactly reproduce routing/fixtures/system_one_c_v1.json in Domain,
whose sorted-key compact UTF-8 checksum is
4dbab2f53954eb2176b6aa8d31b97808c4fee15ebb2fae337012eba348aef3c7.
The fixed assessment wrapper is its first three sentences; composition adds one
ASCII space and the editable suffix. No threshold or prompt tuning is implicit.

C adds one independent two-label Choice for applicability of an unresolved
historical execution problem to latest_user. The exact labels are applicable and
not_established. Only a finite 0..1 vector with exactly those labels and sum within
1e-6 of one is valid; normalize accepted vectors. Choice text/confidence cannot
substitute for probabilities. Missing, duplicate, extra, nonnumeric, nonfinite or
out-of-range values invalidate only applicability while preserving valid selected
degree and historical Score. Category plus sixteen independent degree questions,
historical Score and applicability fit the supported nineteen-question shape in
one call. Unused malformed degree answers do not poison the selected path.

C category and category-conditioned degree templates MUST be immutable versioned
content. Domain compilation and C Gateway execution share
the supported v1 template. Editing current defaults MUST NOT alter a supported
publication's questions or make it unloadable. C request construction and answer
bindings consume the stored frozen decision, while authoring/compiled consistency,
fixed-content validation and digest checks remain mandatory.

The frozen scope standard MUST record authored assessment intent as default or
custom, including categorical scopes. Resolve defaults at compilation and seal
actual effective contents; observations use that stored standard directly without
relabeling or claiming experimental equivalence. Keep C category wording in its fixed
v1 fixture; legacy native wording has its own current source, independent of that
fixture. Pure composition logic may be shared. Disposable unreleased C encodings
need no support adapter; supported released data and retained authenticated
historical evidence preserve their existing compatibility treatment.

#### Scenario: Frozen C questions

- **WHEN** a published native C route evaluates a complete prior stage
- **THEN** The Gateway MUST use its frozen question content and separate current demand, historical performance and applicability without depending on another answer.

#### Scenario: Invalid applicability only

- **WHEN** the two-label applicability answer is invalid but selected degree and assessment are valid
- **THEN** HiRoute MUST preserve those valid results and mark applicability unavailable.

#### Scenario: Defaults change after publication

- **WHEN** current category or degree defaults change after a C publication is saved
- **THEN** Reopen and execution MUST retain the frozen questions, role bindings and content identity, while corrupted or authoring-mismatched content remains rejected.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235109346

### Requirement: Conservative fallback preserves a valid selected category

A valid category with invalid degree MUST use that category's primary group with an explicit degree-unavailable reason, not
probability 1 or another default category. For a custom extension, a valid category with a missing or invalid group uses
that category's primary group with an explicit decision-unavailable reason; HiRoute does not switch category or invent a
group. A wholly failed call uses heuristic policy for smart saving, or the explicit default category's primary group when
present, otherwise regular. Valid decision plus invalid assessment keeps the decision and discards only assessment.
Cancellation, source deadline, and replay integrity failures terminate instead of fallback. One decision call uses the
shared total deadline without a new retry/state machine.

#### Scenario: Review selected but degree is missing

- **WHEN** the response selects review, while writing is the plan default
- **THEN** HiRoute MUST select review's primary group and report degree failure; it cannot switch to writing or manufacture a probability.

#### Scenario: Category valid but group missing

- **WHEN** a custom extension selects a published dual-group category but omits or corrupts the group
- **THEN** HiRoute MUST execute that category's primary group with an explicit decision-unavailable reason; it cannot switch category or invent a group.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370180

### Requirement: Overrides, route editing, and protocol help preserve user intent

New connection flow from a route MUST preserve its draft, return to the exact saved version, and never publish implicitly.
Custom-category overrides copy the entire current effective judgment settings, including thresholds, degree, rubric,
anchors, and supplemental assessment requirements. They become independent until restore-plan-default clears the whole
override without changing category/name/models. Single-group categories hide inapplicable degree controls. Hidden invalid
fields expand on publication failure, and switching judgment mode preserves parked edits.

A route whose selected connection is a custom extension MUST NOT show or publish simple thresholds, competence floors,
degree text, assessment rubrics or category judgment overrides; categories, conditions and candidates remain editable.
Its draft judgment settings stay parked and return when a built-in connection is selected again. Users never choose an
extension protocol version.

Extension help MUST show first-decision and preceding-stage examples, request copy, and the current OpenAPI download,
without future tool-selection entries or another Secret-management surface. A response need not contain assessment.
Plan/session presentation of a built-in decision freezes probability, thresholds, selection/protection/relay reasons,
and actual positions. An extension decision is presented as selected by the decision extension with its optional reason
and actual positions, without HiRoute threshold or protection explanations. Both use the same
[quality queries](../session-history/spec.md).

Parked unfinished prompt text MUST NOT block custom-extension publication. New derived execution payloads omit judgment settings; source drafts retain their bytes, including complete branch overrides. Current custom publications and Operations MUST use the execution shape without judgment and validate their exact parked source against sealed configuration. Old custom-only execution and Operation formats have no compatibility adapter. Existing native frozen configuration and common historical Observation authentication keep their supported behavior.

#### Scenario: Extension route hides decision settings

- **WHEN** a user selects a saved custom extension for smart saving or custom branches
- **THEN** the editor MUST show categories, conditions and candidates only, publish no thresholds or rubric, and restore the parked judgment settings when a built-in connection is selected again.

#### Scenario: Extension decision presentation

- **WHEN** a user inspects a request routed by a custom extension
- **THEN** plan, session and request views MUST show that the decision extension selected the group, its optional reason and actual positions, without HiRoute threshold or protection explanations.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: C context allocation protects the complete assessed stage

C MUST partition safe supplied content into prior_context, assessment_stage and
latest_user. Background is the newest contiguous whole-turn suffix before the actual assessment
target, capped at eight with no setting.
Preserve bounded interrupted/unknown turns and their recorded status; execution
status alone MUST NOT erase earlier user requirements or cut off older background.
This admission does not certify assessment-target completeness or score eligibility. The complete actual target
stage is not limited by that turn cap. Latest user input remains separately
represented; tool-result bodies, goal parsing, semantic extraction and invented
provider-only history are excluded.

The native request MUST fit 262144 actual escaped wire bytes. Initially complete
targets reserve all required question bytes, including applicability, before
admitting a suffix of the target and then optional background. Remove background
before losing target content. If that reserved allocation makes the target
partial, omit applicability and never refill target or background with the freed
bytes. Initially partial targets omit applicability from the start. When no target is admitted, omit historical Score and applicability together. Latest user content and
question text cannot be truncated; reject a final over-budget request. Repeated
equal inputs and frozen content MUST yield identical prepared requests.

#### Scenario: Reserved boundary

- **WHEN** a complete target fits only if applicability bytes are reclaimed
- **THEN** Preparation MUST mark the target partial, omit applicability and retain the original allocation without a refill pass.

#### Scenario: Interrupted or unknown background

- **WHEN** an already-bounded safe background turn has interrupted or unknown status
- **THEN** Its user content and recorded status MUST remain in the contiguous admitted suffix when within the cap and byte budget; earlier background MUST NOT be removed solely because of that execution status.

#### Scenario: Background cap

- **WHEN** a ten-turn actual target follows earlier completed turns and the cap is eight
- **THEN** All ten target turns MUST be retained when the byte budget allows, with at most eight background turns.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235109346

### Requirement: C capability evidence is explicit and session scoped

The connection page MUST retain one hiroute.classifier-decision-test/v1 test with unchanged request/result contract and a fixed example using the current native request shape. There is no route-editor applicability test. Save, inspect and publication send no provider request.

The separately named hiroute.historical-applicability-test/v1 and its result remain CLI/acceptance diagnostics under the existing protected operation, not connectivity v2 or a new permission. Bind the exact saved service revision, endpoint/model, frozen content digest and requested standard/maximum shape. Use the production transport/parser and synthetic complete history. Maximum shape is one category, sixteen degree, one historical score and one applicability question in one call. Validate all bound answer types and probability vectors without prescribing semantic answers. Missing saved revisions cannot dispatch. Report verified/unsupported/not_tested/test_failed and bounded duration/reason; machine output retains identities and digests. Do not infer quality or universal provider certification from shape success. No automatic retries or implicit calls are introduced.

#### Scenario: One current connection test

- **WHEN** a user explicitly tests a saved native connection from Desktop
- **THEN** The existing v1 contract MUST exercise the current default request shape through one call, without a route test or new permission.

#### Scenario: Maximum native diagnostic

- **WHEN** the CLI/acceptance probe selects the sixteen-category maximum shape
- **THEN** One production request MUST contain all nineteen questions and validate their complete results against the saved identity and frozen contents.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235109346

### Requirement: Division drafts disclose the change in protection

Model performance MAY open separate editable unpublished division drafts without
saving, publishing, changing Agent mappings or starting a Worker. A branch draft
uses the former Primary for design/planning and the trial model for implementation,
with editable conditions and an optional Primary group for implementation. It
copies the trial's decision and competence policy.

A delegated draft requires explicit executor selection, shows dependency results
and orders DeepSeek Harness first for DeepSeek models without preselection. Both
single-group task categories use the trial model and retain assessment policy.
Before publication a summary derived from the draft MUST distinguish two-group
next-turn Primary protection from single-group/Worker low-score recording without
an upgrade group. Parent models and other executors do not rescue that Worker.
Completed tasks are not competence results. Evidence from previously routed trial
stages does not validate work that only Primary handled.

Two-group protection explanations follow the copied judgment settings. With System One native judgment, a complete Regular score below the competence floor protects the next turn when the earlier problem still matters; unavailable applicability conservatively retains protection, while a fixed or unrelated problem allows simple tasks to remain with Regular. Other decision services retain unconditional low-score protection. Published routing follows its frozen settings; editor explanations describe the next publication.

#### Scenario: Keep Primary for implementation

- **WHEN** the user enables Primary protection for the implementation category
- **THEN** the draft contains that two-group category and its publication summary
  describes next-turn protection; an unchecked single-group category states that
  low scores are recorded without an upgrade.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235189421

### Requirement: Custom extensions return the routing decision

The current custom extension contract MUST return the routing decision. HiRoute sends the current safe user content,
retained safe visible history with `{user,status,steps}` turns, `history_partial`, the published route shape and, only
when it executed under the same Plan revision and connection revision, the preceding actual stage identified by
its starting index, executed group and record completeness. The route shape is the smart-saving scope or the custom
categories with stable IDs, conditions, available groups and the default category. HiRoute binds internal execution
identity locally without sending model or stage IDs, and sends no degree text, assessment rubric or threshold.

The response contains the category for custom branches and the regular or primary group for a dual-group scope, plus an
optional bounded reason and an optional `{score, partial}` assessment of the preceding stage that is used only for
quality reporting. Unknown members and unpublished IDs are rejected. The extension implements provider integration,
context handling and its own decision policy; it never selects models, candidates, unpublished categories or tools.

The typed producer, parser, official extension, examples and OpenAPI MUST describe one current shape. It replaced
the earlier v1 judgment shape in place, without a legacy reader, negotiation or user-visible protocol version.
Custom execution payloads, publications and Operations support only the current format without judgment settings; a custom execution payload carrying judgment MUST be rejected, with no legacy normalization adapter. Current editor judgment settings remain parked for switching back to built-in connections. Existing native data and shared historical Observation authentication retain their support.
An extension that has not been upgraded fails like any invalid decision call and uses the existing fallback. The API
guide/OpenAPI supply complete wire schemas; the durable policy is here.

The request carries the preceding actual category before the new category is known. HiRoute MUST verify exact Plan and saved connection revision, unique actual execution attribution and membership in the published route locally. A response choosing another category MUST discard its optional assessment while preserving its category/group decision. This does not identify human authorship or a client subdialog, and adds no history beyond the existing safe view.

#### Scenario: Extension keeps primary after a failed stage

- **WHEN** the preceding actual stage ran on the regular group in the same category and Plan revision, and the extension returns primary for the new user turn
- **THEN** HiRoute MUST execute that category's primary group and record the decision extension as the selection source with its optional reason.

#### Scenario: Unrelated simple task returns to regular

- **WHEN** after a primary stage, the extension returns regular for a new user turn in a dual-group scope
- **THEN** HiRoute MUST execute the regular group without applying any simple threshold, competence floor or historical-protection rule.

#### Scenario: Preceding stage from another revision

- **WHEN** the preceding actual stage executed under an earlier Plan revision or another connection revision
- **THEN** The request MUST carry no preceding stage; HiRoute MUST NOT send degree text, rubrics or thresholds in any case.

#### Scenario: Extension not yet upgraded

- **WHEN** a published route calls an extension that still implements only the replaced v1 shape
- **THEN** the call MUST fail validation and use the existing decision-failure fallback; the current published Plan stays valid and save-and-test reports that the extension does not implement the current contract.

#### Scenario: The extension chooses a different category

- **WHEN** the request includes an eligible actual writing stage and the extension selects review
- **THEN** HiRoute MUST preserve the review decision and discard the returned assessment; no writing score may affect review.

#### Scenario: Legacy custom execution judgment is rejected

- **WHEN** a custom smart or branch execution payload contains judgment settings
- **THEN** Plan validation and Gateway publication installation MUST reject it; current custom payloads omit judgment, while source editor settings stay parked and native payloads still require their valid frozen judgment.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246369555

## Implementation references

- [Decision domain](../../../crates/domain/src/routing/decision.rs) and
  [definition types](../../../crates/domain/src/routing/decision_definition.rs).
- [System One codec](../../../crates/gateway/src/core_runtime/classification/system_one.rs),
  [custom codec](../../../crates/gateway/src/core_runtime/classification/extension.rs), and
  [policy scenarios](../../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs).
- [Current API/OpenAPI](../../../decision-extensions/api/README.md) and
  [Desktop connection owner](../../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx).
