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

Current native connections use `hiroute.decision-service/v2` with an explicit
`system_one` or `openai_decisions` wire protocol. The latter adapts OpenAI
Decisions; it does not select a different logical routing protocol. Supported
untagged V1 revisions retain their exact content and digests. Preparing an old
connection for editing does not write or republish it. Custom extensions retain
their own protocol and optional independent header; they cannot reuse a model
source's credentials.

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

#### Scenario: Existing publication retains its service

- **WHEN** a newer immutable service revision is saved
- **THEN** the existing publication retains its exact selected revision and authentication until an explicit new selection and publication.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488573916

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

The current Product Session test resolves a saved ID/revision and expected
configuration digest server-side. Its V2 result identifies the tested configuration
and credential-state revision. A successful save acknowledgement requires readback
before testing or returning the selected version; a failed readback is not permission
to repeat the save. Testing does not require publishing a route. A previous pass
becomes stale when the tested configuration or credential state changes and cannot
be displayed as current success.

#### Scenario: Save succeeds but explicit test fails

- **WHEN** the exact saved service rejects the synthetic connectivity call
- **THEN** the saved revision remains available with an actionable failure and is not automatically republished, resaved or represented as a successful test.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488573916

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

For a built-in dual-group category with valid current simple probability, derive the base selection first: Regular when P(simple) is at least T, otherwise Primary. Evaluate eligible historical low-score protection independently of that base group. Supported legacy native protocols retain their existing protection without inventing a native C observation.
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

Missing, nonfinite or out-of-range current difficulty selects Primary/degree_unavailable without a competence trigger; C action remains not_considered, not not_needed. A valid non-low score and a missing/unrated target remain distinct. Raw difficulty, thresholds and assessment values are never rewritten.

For native C, applicability MUST gate only an otherwise eligible historical low-score protection opportunity, for either base group. Preserve exact Plan/revision/category, actual single-model attribution, selected and executed branch agreement, branch-execution evidence, frozen rubric/policy and complete valid preceding score guards. With finite 0 <= s < F, applicability at least 0.5 applies Primary/low_competence protection; below 0.5 suppresses only that trigger and returns the base group and reason, so complex work remains Primary/complex_task. Missing/unavailable applicability conservatively retains an eligible trigger with its real bounded reason. Applied or retained protection records the original stage, score, floor and actual previous group; a previous Primary remains Primary and is not a new tier or a claimed Regular-to-Primary transition. A missing internally required question is separately reported, not counted as a provider request/failure. No score is rewritten or transferred, and applicability cannot rescue invalid/missing assessment or select a category. Same-turn inheritance, ContextHold, candidate ordering and availability relay retain their existing rules. C is inapplicable to custom-extension v1 and heuristics. No extra model call is introduced by this policy reduction.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235110154

For a custom extension, HiRoute MUST execute the returned category/group without simple thresholds, competence floors or historical protection. Its optional same-category assessment is observation only. Single-group categories ignore the returned group. These decisions retain the same continuation, ContextHold, candidate and relay boundaries.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370180

#### Scenario: A new user turn keeps the selected group

- **WHEN** using a built-in decision connection, a custom-branch follow-up selects the same group and its held candidate remains valid
- **THEN** the Gateway MUST prefer that candidate while retaining the fresh category/degree decision.

#### Scenario: Unresolved problem remains relevant

- **WHEN** using a built-in decision connection, P(simple) meets T, a complete eligible score is below F and applicability equals 0.5
- **THEN** Primary protection MUST apply; equality does not suppress it.

#### Scenario: Problem no longer governs current work

- **WHEN** using a built-in decision connection, P(simple) is at least T, all historical protection guards hold and applicability is below 0.5
- **THEN** The historical trigger MUST be suppressed and the regular group selected without altering the raw score.

#### Scenario: Applicability is unavailable

- **WHEN** using a built-in decision connection, a low-score protection opportunity has no valid applicability result
- **THEN** Protection MUST be retained conservatively, with the unavailability reason recorded.

#### Scenario: Extension selection is authoritative

- **WHEN** an extension returns regular with a complete low score for the preceding same-category stage
- **THEN** HiRoute MUST execute regular without applying threshold or protection policy and may record the score for observation.

#### Scenario: Complex work with relevant failure

- **WHEN** P=.80, T=.90 and an eligible score .24 below F=.50 have applicability .55
- **THEN** Primary/low_competence and Applied are recorded with the original trigger, while P remains .80.

#### Scenario: Release does not force Regular

- **WHEN** the same history has applicability .49 and the new task is complex
- **THEN** Suppressed is recorded, no trigger remains, and Primary/complex_task is selected.

#### Scenario: Equality and unavailable judgments

- **WHEN** P=T or applicability=.5, or difficulty/applicability is unavailable
- **THEN** equality follows the inclusive boundaries; missing difficulty remains degree_unavailable, whereas unavailable applicability only retains an otherwise eligible trigger.

#### Scenario: Ineligible and extension history

- **WHEN** history is partial/mixed/cross-scope, or a custom extension selects Regular with a low score
- **THEN** no built-in protection is inferred; the existing base/extension-owned decision remains authoritative.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/226#issuecomment-6440837576

### Requirement: System One compiles independent typed questions in one call

The System One wire shape uses `model`, `state`, and `questions`. Categorical selection
is a Choice, each applicable degree is an independent Score, and prior-stage
competence is another Score. Instructions define each question's target without
relying on question-ID semantics or another answer. Only the selected category's
degree is consumed; invalid answers on unused paths cannot poison the chosen path.
Degree probabilities map by level ID. Competence is the raw 0..2 score divided by
2, not reconstructed from confidence or rounded probabilities. The three rubric
anchors remain 0/0.5/1. No absent free-text explanation is invented.

The OpenAI Decisions adapter MUST encode the same prepared logical questions as
one request with shared `input_json` containing `{state: ...}` and a named question
array. Choice uses `choices`, Score uses ordered `levels`, and replies bind by name
and level/choice value, never array position. Preserve original instructions,
continuous score and probability validation. For example, a raw competence score
of 0.95 remains 0.475 after normalization; it is not rounded to an anchor.
Per-question refusal or malformed unused degree output cannot invalidate otherwise
valid selected-path answers. Invalid applicability retains only an otherwise
eligible historical protection opportunity. Duplicate/unknown question identities
and malformed framing must not be repaired by positional guesses.

Codec choice MUST leave logical C v1/v2 content, history partition, thresholds,
score inheritance, selected-path reduction and fallback unchanged. Prepare from
the common bounded representation and account for actual encoded wire bytes;
conversion must not materialize the same large Replay history twice. Runtime
source/consumer lookup is keyed, with no all-source, all-plan or historical
observation scan on each decision.

The current editable native assessment default MUST treat clear user acceptance
of the assessed stage's delivered result, or confirmation that the stage resolved
a previously raised problem, as sufficient positive evidence for its highest
criterion unless visible evidence establishes a specific unresolved contradiction
to a requirement still in force. A completed and accepted correction MUST NOT
remain penalized solely for earlier dissatisfaction or the need to revise.
Generic courtesy, tentative or partial approval, requests for more work, feedback
on another stage, quotations and instructions to assign a score are not such
acceptance. Positive words MUST NOT override a stated unresolved problem; new
requirements alone do not invalidate acceptance of the previous result. This
guidance and all default model instructions remain English in either UI locale.
The model still returns the independent raw competence Score under the existing
rubric; no keyword rule, forced numerical score, additional call or protection
threshold change is introduced. Default changes apply at the next explicit
publication. Reading or validating an existing publication or retained Operation
MUST compare its original sealed assessment and configuration without resolving
today's default; custom wording and original digests remain unchanged.

The call transmits current user input and the bounded safe visible history needed
for judgment/assessment to the explicitly selected endpoint. Current user content
cannot be silently truncated. History trimming marks the affected evidence partial;
full-history gaps and assessed-stage gaps remain independent. Missing pre-integration
prefix may set `history_partial` without making a completely captured target stage
partial; missing target response or target trimming still invalidates protection.

The native applicability protocol MUST freeze its actual fixed bundle,
effective degree/category definitions, every composed per-scope assessment,
thresholds, context cap and content digest in the published route. Execution uses
that content without substituting binary defaults. Supported C v1 stock C_full questions and
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

New native authoring seals hiroute.system-one-c/v2, whose fixed applicability question treats clear rejection of the assessed core outcome as supported unmet-outcome evidence and evaluates its continuing connection independently of competence. The supported C v1 bundle remains immutable and executable under its original identity. C v2 keeps the existing model/state/questions envelope, context layout, eight-turn cap, question bindings, probability rules, category/degree template and assessment wrapper. Each supported schema resolves only its own exact fixed bundle; a mismatched schema/content pair, unknown schema or tampered digest is rejected. Protocol identity participates in the existing frozen content and execution-policy identities, preventing assessment inheritance across different frozen policies.

Current readers accept authenticated v1 and v2 publications, retained Operations and observations without rewriting their original bytes or digests. New observations record the actual executed protocol and retain version/content-separated cohorts. Existing historical-applicability diagnostic v1 requests with frozen v1 protocols retain their meaning; the same request carrier may also accept the explicitly supplied supported v2 protocol. Its result shape, protected authority and one-call question-count limits are unchanged. Opening a draft prepares the current protocol for a later explicit publication; it does not change the active publication or custom assessment text. No storage migration or automatic republishing is implied.

#### Scenario: Frozen C questions

- **WHEN** a published native C route evaluates a complete prior stage
- **THEN** The Gateway MUST use its frozen question content and separate current demand, historical performance and applicability without depending on another answer.

#### Scenario: Invalid applicability only

- **WHEN** the two-label applicability answer is invalid but selected degree and assessment are valid
- **THEN** HiRoute MUST preserve those valid results and mark applicability unavailable.

#### Scenario: Defaults change after publication

- **WHEN** current category or degree defaults change after a C publication is saved
- **THEN** Reopen and execution MUST retain the frozen questions, role bindings and content identity, while corrupted or authoring-mismatched content remains rejected.

#### Scenario: Explicit acceptance after correction

- **WHEN** latest relevant feedback clearly accepts the assessed stage's result or confirms that it resolved the prior problem, with no specific unresolved contradiction to an existing requirement
- **THEN** the current default provides sufficient evidence for the highest competence criterion; earlier corrected dissatisfaction alone is not an ongoing defect, and no numerical score is forced.

#### Scenario: Praise with an unresolved problem

- **WHEN** feedback includes courtesy, partial approval or positive words while identifying a still-unresolved requirement
- **THEN** those words do not establish full acceptance or override the visible problem; a request for additional unrelated work does not retroactively fail the preceding stage.

#### Scenario: Current feedback protocol on new publication

- **WHEN** a new native route is explicitly saved and published with otherwise eligible same-core rejection feedback
- **THEN** the sealed protocol is C v2 and its applicability question recognizes the rejected outcome without requiring a diagnosed defect; routing still consumes independent finite probabilities and unchanged eligibility guards.

#### Scenario: Supported v1 execution and recovery

- **WHEN** an authenticated supported C v1 publication or unfinished Operation is loaded after the authoring default changes
- **THEN** its exact original protocol, questions, custom/default standard and digests remain valid and execute unchanged; startup never recompiles it against the new default.

#### Scenario: Version-content mismatch

- **WHEN** v2 fixed content is paired with a v1 identity, a v1 fixed bundle with v2, or an unknown protocol or corrupt digest is supplied
- **THEN** validation rejects the input rather than substituting a supported default or repairing authenticated content.

#### Scenario: Version-separated evidence and diagnostics

- **WHEN** retained v1 and new v2 observations share a service and plan scope, or an explicit diagnostic is run with either supported frozen protocol
- **THEN** observations keep their original authenticated protocol/content identities and remain readable without a rewrite, cohorts do not imply equivalence across identities, and diagnostics use exactly the supplied supported bundle under the unchanged protected request/result carrier.

#### Scenario: OpenAI difficulty decision with and without history

- **WHEN** a published OpenAI decision service receives a new user turn with no target history or with a complete eligible historical target
- **THEN** HiRoute sends the corresponding independent questions in one OpenAI Decisions request and reduces the mapped results using the published HiRoute policy

#### Scenario: Maximum custom-branch shape

- **WHEN** a custom plan has sixteen categories with degree questions and eligible historical competence/applicability
- **THEN** one request carries the category question, sixteen degree questions and the two historical questions; only the selected category degree affects routing

#### Scenario: Continuous score and reordered answers

- **WHEN** the provider returns named answers in a different order and a valid raw competence score of 0.95
- **THEN** question identities remain correct and competence is 0.475 after existing normalization, without rounding to the most likely grade

#### Scenario: Applicability refusal and unused-path invalidity

- **WHEN** applicability is refused or an unselected category degree is invalid while the selected degree and assessment are valid
- **THEN** the valid selected results survive; applicability unavailability retains only eligible low-score protection and unused-path invalidity cannot fail the chosen path

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488571917

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

The connection page MUST retain one explicit connectivity test with a fixed
example using the saved service's native request shape. Current V2 testing uses
the saved-revision contract above; supported
`hiroute.classifier-decision-test/v1` requests/results retain their exact meaning
through the legacy entry. There is no route-editor applicability test. Save,
inspect and publication send no provider request.

The separately named hiroute.historical-applicability-test/v1 and its result remain CLI/acceptance diagnostics under the existing protected operation, not connectivity v2 or a new permission. Bind the exact saved service revision, endpoint/model, frozen content digest and requested standard/maximum shape. Use the production transport/parser and synthetic complete history. Maximum shape is one category, sixteen degree, one historical score and one applicability question in one call. Validate all bound answer types and probability vectors without prescribing semantic answers. Missing saved revisions cannot dispatch. Report verified/unsupported/not_tested/test_failed and bounded duration/reason; machine output retains identities and digests. Do not infer quality or universal provider certification from shape success. No automatic retries or implicit calls are introduced.

#### Scenario: One current connection test

- **WHEN** a user explicitly tests a saved native connection from Desktop
- **THEN** the version-specific saved connection test MUST exercise the current
  default request shape through production transport, without a route test or new
  permission; legacy v1 callers remain compatible.

#### Scenario: Maximum native diagnostic

- **WHEN** the CLI/acceptance probe selects the sixteen-category maximum shape
- **THEN** One production request MUST contain all nineteen questions and validate their complete results against the saved identity and frozen contents.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488571917

### Requirement: Division drafts disclose the change in protection

Model performance MAY open separate editable unpublished division drafts without
saving, publishing, changing Agent mappings or starting a Worker. A branch draft
uses the former Primary for design/planning and the trial model for implementation,
with editable conditions and an optional Primary group for implementation. It
copies the trial's decision connection and applicable assessment settings. Custom
extension judgment inputs remain parked editor values, not routing policy.

A delegated draft requires explicit executor selection, shows dependency results
and orders DeepSeek Harness first for DeepSeek models without preselection. Both
single-group task categories use the trial model and retain assessment policy.
Before publication a summary derived from the draft MUST distinguish two-group
next-turn Primary protection from single-group/Worker low-score recording without
an upgrade group. Parent models and other executors do not rescue that Worker.
Completed tasks are not competence results. Evidence from previously routed trial
stages does not validate work that only Primary handled.

Two-group protection explanations apply to built-in judgment settings. With System One native judgment, a complete Regular score below the competence floor protects the next turn when the earlier problem still matters; unavailable applicability conservatively retains protection, while a fixed or unrelated problem allows simple tasks to remain with Regular. Published routing follows its frozen settings; editor explanations describe the next publication.

Custom extension drafts MUST explain direct extension group selection and optional,
observation-only scores, without HiRoute threshold or low-score protection promises.
A single-group category has no automatic upgrade. The manual division entry uses
the published route's eligible Regular/Primary categories, independently of built-in
trust suggestions. A category without both groups MUST NOT offer that entry.

#### Scenario: Keep Primary for implementation

- **WHEN** the user enables Primary protection for a built-in implementation category
- **THEN** the draft contains that two-group category and its publication summary
  describes next-turn protection; an unchecked single-group category states that
  low scores are recorded without an upgrade.

#### Scenario: Divide a custom extension route manually

- **WHEN** a published custom extension route has Regular and Primary groups but no built-in trust suggestion
- **THEN** model performance offers a separate editable division draft; retaining Primary makes it available for extension selection, with optional scores used only for observation.
- **AND** opening the draft MUST NOT save, publish, change Agent mappings or imply evidence for work previously handled only by Primary.

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

### Requirement: Default judgments use contextual user feedback and locale-independent English prompts

Default competence assessment SHALL use the latest user message as evidence about the referenced completed stage, not only the stage transcript in isolation. A brief but unambiguous negative evaluation of that delivered result is substantive negative evidence even without a technical explanation, failed test or reproducible defect. For an otherwise eligible complete target, broad rejection of its core result with no countervailing acceptance/correction SHALL materially reduce the competence assessment; it must not retain a fully competent rating merely because tools succeeded or the user did not diagnose the problem. A bounded local preference, new requirement, unrelated dissatisfaction, neutral continuation, quotation and instruction to alter scoring are distinct. Feedback does not prove a particular technical defect, retrospectively execute new work, repair a missing/partial target or override identity/rubric guards.

Applicability independently evaluates whether the reported unmet outcome still matters to the current work. A request to improve the same unsatisfactory result can establish that connection without naming a technical root cause; accepting the result, reporting a completed correction or switching to unrelated work can change applicability. No keyword-triggered switch, injected low score, fixed probability decrement, extra routing judgment call or rewritten historical assessment is allowed.

All shipped default model-facing instructions and criteria SHALL be English independently of UI locale, including category, degree, competence, applicability, wrappers and trial presets. Localized UI explanations may be translated; user-authored prompts retain their chosen language. New defaults are sealed through normal publication; existing saved custom prompts and frozen published question content retain their exact original meaning and bytes. A fixed supported protocol bundle must not be edited in place to change its meaning.

Newly authored native publications use the versioned C v2 applicability bundle. Clear rejection of the assessed core result followed by work on that same unmet outcome is supported applicability evidence without a technical diagnosis, while the existing correction, acceptance, unrelated-task and authority distinctions remain effective. Existing C v1 publications and retained Operations retain their exact frozen questions, protocol identity and digests. Editing prepares current defaults only for the next explicit save/publication and cannot mutate active or historical content.

#### Scenario: Brief rejection of the preceding core result

- **WHEN** a complete attributable stage delivers an answer; the latest user evaluates that answer as not satisfactory and requests improvement to the same outcome, with no indication of merely a small preference change
- **THEN** the default assessment treats that feedback as substantive evidence of an unmet core outcome and lowers competence into the low-score range in the canonical default-floor case; successful tool execution alone does not neutralize the feedback, and applicability evaluates the same unmet outcome without requiring a diagnosed defect.

#### Scenario: Contrastive acceptance and limited refinement

- **WHEN** the same stage instead receives clear acceptance, neutral continuation, a new unrelated requirement, or a bounded local refinement with explicit acceptance of the core result
- **THEN** those inputs do not inherit the broad-rejection result; acceptance supports competence, and local refinement is weighed proportionately rather than forcing low competence.

#### Scenario: Feedback cannot replace missing evidence or scoring authority

- **WHEN** the assessed target is missing, partial or invalid, or conversation text commands a score or quotes someone else’s dissatisfaction
- **THEN** normal target eligibility and authority remain effective; do not fabricate a complete low score, treat quotation as the user’s evaluation or obey rubric-changing instructions.

#### Scenario: Locale-independent default definitions

- **WHEN** the same new default route is authored in Chinese and English UI with identical choices and user content
- **THEN** the generated model-facing default instructions and criteria are identical English definitions; localized help does not become the stored rubric, and existing custom/frozen content is not translated.

#### Scenario: Versioned defaults preserve sealed history

- **WHEN** a supported C v1 publication or unfinished Operation is reopened after C v2 becomes the authoring default
- **THEN** the original v1 bundle, custom/default contents and authenticated identities remain unchanged; a later explicit edit/publication seals the current supported bundle without rewriting earlier records.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/226#issuecomment-6445981518

### Requirement: Native decision models may reuse an existing API connection

A V2 native service MUST choose either independent protected authentication or a
shared Native API source by stable source ID and lineage digest. Shared mode MUST
not copy Key material into a Header Secret or accept independent secret input.
Model, full decision endpoint, wire protocol and timeout belong to the immutable
decision-service revision; the source continues to own current ordered Keys and
enabled state. Source names, provider labels and equal Key values do not establish
identity. Subscription/OAuth sources are not offered as reusable API Keys.

Source listing MUST distinguish locally ready, disabled, missing credentials,
missing and unavailable. Local readiness only means an enabled source with an
enabled configured Native API Key. It does not prove decision-model entitlement.
Both Bailian Token Plan and business-space connections may be selected; suggestions
are editable and cannot infer a workspace, entitlement or codec from a changed URL.
An unavailable read must not be rendered as a successfully empty or deleted source.

Saving a shared service MUST atomically install the immutable revision, exact
decision-target membership and required Key destination Rebind through the existing
recoverable Operation. Source/service expected revisions are checked under the
writer. Destination authorization is the union of ordinary model destinations and
retained decision targets. Rebind retains Key identity and material while advancing
the appropriate generation; it cannot require plaintext re-entry. Source changes
or deletion MUST preserve references from retained service revisions, publications,
drafts and unfinished recovery. Failed or unknown reference reads block destructive
changes. Partial preparation must compensate or resume without overwriting later
source/Key generations.

Execution freezes service and source lineage, but checks current source/Key
eligibility for every exact lease. It uses a decision-specific consumer and target,
not a fabricated inference model or the independent Header Secret path. Key rotation
and disablement affect subsequent calls without republishing. Retry visits each
admitted eligible Key at most once within the one total decision deadline, and only
for recognized credential failure classes. It never switches sources or treats all
malformed provider responses as reasons to try every Key. Automatic credential
health stays local to the decision binding; it does not infer shared inference quota.

#### Scenario: Shared Key changes after publication

- **WHEN** a published decision service's selected source rotates or disables a Key
- **THEN** subsequent leases use current eligibility and generation without changing
  the frozen endpoint/model/protocol or copying credentials into the publication.

#### Scenario: A newer service selects another source

- **WHEN** service revision 2 selects source B while revision 1 remains referenced
- **THEN** revision 1 retains its authorized target on A, revision 2 uses B, and
  deletion or narrowing cannot orphan the retained revision.

#### Scenario: Save races with source deletion or Key replacement

- **WHEN** either source or Key state changes after the form's read
- **THEN** writer-side revision and authority checks reject stale intent or recover
  its exact Operation; no partial service grants access to an unrelated destination.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488572844

### Requirement: Setup and repair preserve user intent across management navigation

The decision-model form MUST offer reuse of an existing API connection and
independent configuration. Initial loading provides an explicit independent escape;
late reads cannot override a user's mode, source or edits. Reuse may be the initial
mode when a locally ready source exists, but no account is silently selected.
TypeSafe and other independent compatible endpoints remain available.

Model, endpoint and codec form one non-secret draft bundle per mode/source.
First selection of source B uses B's suggestions; returning to A restores A's edits.
Refreshing source state does not mix bundles. Reset to suggestions is explicit.
An unknown codec requires a choice; changing origin alone does not clear a chosen
codec. Independent authentication retains its existing origin-change protection.
A saved independent reference survives a harmless mode roundtrip, while pending
unsaved secrets are cleared at context switches and never stored in resumable state.
Preparing V2 from an untouched V1 service is the form's unmodified baseline.

Managing the selected API connection is a nested detour: route/trial, decision
model, source management, decision model, then the parent. It MUST preserve the
decision and parent drafts, target the selected source and provide an explicit
return. Existing model-page drafts still receive normal replacement protection.
Unrelated navigation retains its discard guards. Return refreshes source state
without changing the selected bundle or automatically testing or publishing.

UI recovery MUST distinguish saving, saved-but-readback-required, test failure,
stale test and publication. Source disable/no-Key errors lead to source management;
a missing source permits reselection; independent credential/model/endpoint/codec
errors lead to editing; transient failures permit retry. Ambiguous provider rejection
retains alternate repair actions. A revision conflict preserves non-secret edits
and requires an intentional save after refreshed state is available. Diagnostics may
show digests, but they are not the primary user explanation. Same-name selections
include model and source identity. V1 System One and V2 native configurations appear
as decision models; custom extensions remain separately typed.

#### Scenario: Nested management changes credentials

- **WHEN** a user edits a route, adds a decision model, manages its shared API source
  and changes its Keys
- **THEN** both drafts survive the return, source state refreshes, a previous pass
  is stale, and neither testing nor route publication occurs automatically.

#### Scenario: Changing sources and returning

- **WHEN** an edited source A is replaced by B and then selected again
- **THEN** B initially has its own complete suggested bundle and A recovers its own
  complete edited bundle; no Key or field is silently copied between sources.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/266#issuecomment-6488573916

## Implementation references

- [Decision domain](../../../crates/domain/src/routing/decision.rs) and
  [definition types](../../../crates/domain/src/routing/decision_definition.rs).
- [System One codec](../../../crates/gateway/src/core_runtime/classification/system_one.rs),
  [custom codec](../../../crates/gateway/src/core_runtime/classification/extension.rs), and
  [policy scenarios](../../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs).
- [Current API/OpenAPI](../../../decision-extensions/api/README.md) and
  [Desktop connection owner](../../../apps/desktop/src/features/decision-services/DecisionServicesPage.tsx).

### Requirement: Concurrent executions own independent history and assessment snapshots

An authorized native session is a scope, not a single in-flight slot. Overlapping
requests SHALL receive distinct Gateway execution identities and independent
capture tickets. Native request IDs, equal input, shared transcript prefixes and
response completion order SHALL NOT deduplicate executions or identify a unique
logical task. Routing resolution SHALL inspect at most 32 retained heads in the
same authenticated workspace/consumer/authority epoch/grant generation/served
alias/protocol/Plan namespace. Evidence from another scope is ineligible.

Optional fingerprint work SHALL have independent content and structural budgets.
Each instruction/history digest admits at most 16 MiB of content and framing,
65,536 structural items and JSON depth 64. Inline JSON strings are charged at
their worst-case escaped length; Replay references are checked before reading.
Response-boundary comparisons share one such budget across the candidate heads.
Captured response-boundary hashing also stops at its budget. Exceeding these
limits loses comparable evidence, never forwarding, current-input classification
or actual-output capture. Contextual evaluation remains available with association
uncertainty, and later bounded requests can regain strong continuity in the same
scope. No truncated fingerprint may establish an edge.

A strong edge requires a comparable typed request prefix and the captured actual
response boundary, with native cursor/turn agreement where available. Only this
response-boundary comparison ignores zero-byte text placeholders that clients may
add when replaying a tool-only response. Replay references still require complete
integrity verification; nonempty whitespace/text, tool identity and typed arguments,
roles and supported metadata retain their existing comparisons and bounds. Empty
text alone cannot establish an edge or erase a previously accepted nonempty delta.
Missing and explicit-empty function encoding MAY support contextual association only
through a separate bounded full-boundary proof. It SHALL retain the entire ordered
accepted reasoning/text, tool kind/namespace/name/logical call ID and canonical JSON
arguments. Original full and visible digests SHALL keep their exact three-state encoding
distinction and existing zero-content folding. The separate compatibility proof retains
the number and ordered positions of empty reasoning summaries. Nonempty encoding or
opaque reasoning/provider state on either side disables
this compatibility proof. Unknown fields, unsupported content and exhausted fingerprint
budgets lose the edge. The proof reuses measured tool digests without additional Replay
reads or larger budgets; it does not combine encoding changes with reasoning omission.
It SHALL NOT grant exact ancestry, human origin or provider affinity. Multiple compatible
predecessors remain ambiguous under the existing pending-request and branch-ranking rules.
Original forwarding, capture and strict request fingerprints remain unchanged. Unavailable
or ambiguous evidence starts an independent branch. A completed actual same-scope
stage MAY remain a contextual assessment candidate with association uncertainty
explicitly disclosed. It SHALL NOT become a claimed unique predecessor or grant
provider-prefix affinity. Uncertain executions cannot overwrite a verified head.
Every classifier invocation freezes at most one target, its exact request members,
revision, actual model/profile and rubric before calling the decision service.
Late output, retry, fallback or another branch's completion cannot change that
snapshot, assessment object or already selected route.

Known encrypted reasoning content MAY establish contextual continuity only when
its bounded, integrity-verified replay bytes exactly match the encrypted content
actually accepted downstream, together with the supported visible response
boundary. Equality SHALL NOT imply decryption, semantic understanding or provider
affinity. Missing, changed, uncaptured or unsupported opaque state cannot be
discarded to manufacture a visible-only match. Representation as inline text or
a Replay reference SHALL NOT change this comparison.

Accepted history SHALL describe the actual client protocol projection. Internal
response-boundary evidence SHALL NOT be filtered by the retained-content display
policy. Conversely, content explicitly omitted by protocol conversion SHALL NOT
be associated with later accepted bytes. Deferred events remain pending until
their wire unit is accepted. A nonstream error that replaces the body SHALL NOT
record discarded text or tools as delivered execution output.

Capture admission SHALL be independent of optional ContextHold preparation/commit.
Branch-scoped affinity uses stronger validity checks; missing/stale/busy affinity
loses only the preference. Capacity/conflict/observation faults SHALL NOT block
otherwise valid model output. Known auxiliary calls consume no main history heads,
capture slots or gap epochs. Invalid or absent purpose evidence retains observed
routing; it SHALL NOT globally disable classification, capture or cross-turn quality
evaluation or permanently select Primary.

Source uncertainty, association uncertainty and incomplete captured execution SHALL
be separate evidence dimensions. Missing preceding context does not make a newly
captured independent execution incomplete. Summary/peer/client transcript content
SHALL NOT be imported as accepted model output. A routing activation or completed
HTTP delivery does not prove logical task completion. A known continuation with no
usable input may temporarily fall back, but later genuine A and B in the same
session SHALL classify and capture, and B SHALL assess A's actual execution where
retained. Retry IDs cannot heal a missing old execution. No additional model call,
sidecar, disk history or session lock held across network execution is introduced.

Assessment completeness SHALL require both complete captured membership and proven
closure of every included execution stage. Freezing a revision after association
loss is not closure proof. Actual provider outcomes SHALL remain distinct from a
stage waiting for tool continuation: a known successful tool response may belong
to an open stage, while an unknown provider outcome remains unresolved even after
a new activation, configuration change or later final response. A new activation
can delimit an otherwise known stage without asserting that its logical task
succeeded. Partial targets remain assessable as partial; later independently
complete stages SHALL recover complete evaluation. Reopening a proven same-stage
continuation may extend its actual membership but cannot mutate a prior snapshot
or invent a missing earlier provider outcome.

The store SHALL bound total history to its configured budget (default 500,000,000
bytes), including branch indexes and pinned snapshots, with default one-hour idle
expiry, 32 live captures/heads and 128 terminal records per scope. Pinned payloads
cannot be silently uncharged. Optional overflow retains explicit incomplete evidence
where possible and otherwise uses bounded scope-hash gap bookkeeping. Collisions
are conservative and recoverable; new complete A/B must recover after pressure or
faults clear. ContextHold retains its independent 50,000,000-byte budget.

#### Scenario: Concurrent responses have the same input prefix

- **WHEN** two requests start from one prefix, select different models and finish out of order
- **THEN** both accepted outputs remain attributable to their actual executions; a later request carrying one captured output selects that eligible branch, while an unproved/ambiguous relation stays explicit.

#### Scenario: Optional affinity fails while the main model succeeds

- **WHEN** ContextHold is contended, stale or full
- **THEN** the request can still classify, capture accepted output and supply a later assessment target; optional failure cannot cause a 409 or replace the model result.

#### Scenario: Compressed session survives Gateway restart

- **WHEN** the Gateway loses its memory while the native client retains a compressed session
- **THEN** summaries do not recreate old execution; genuine new A/B recover classification, capture, selectable groups and assessment without requiring a new native session.
