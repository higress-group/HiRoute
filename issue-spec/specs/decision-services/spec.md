# Decision connections and custom branches

## Purpose

Saved decision connections supply task judgments and optional assessment of prior
execution. HiRoute owns category/group policy, candidate execution, and attribution.
Built-in System One connections and custom extensions share one current contract.

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
Primary presence enables the product's simple/complex degree question; a single-group
category needs none. `smart_saving` is reserved for the built-in task scope and is
rejected in custom authoring/materialization.

The protocol can express categorical choices and ordered levels, but the current
product offers only two degree levels. It does not expose arbitrary trees, parallel
category execution, category-confidence thresholds, priorities, or tool-selection
controls. Overlapping conditions select the current principal intent. No match
selects the explicit default category as a valid decision with its degree result;
this is different from decision-call failure.

Category/degree/rubric text must be legal nonempty text for publication without an
independent 4096-character field cap. Drafts can retain incomplete text. Whole-request
resource/transport/provider limits remain enforced without silent prompt truncation.

### Requirement: Group policy consumes only applicable current assessment

For a dual-group category under the legacy native or custom-extension v1 protocol,
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

#### Scenario: A new user turn keeps the selected group

- **WHEN** a custom-branch follow-up selects the same group and its held candidate remains valid
- **THEN** the Gateway MUST prefer that candidate while retaining the fresh category/degree decision.

#### Scenario: Unresolved problem remains relevant

- **WHEN** P(simple) meets T, a complete eligible score is below F and applicability equals 0.5
- **THEN** Primary protection MUST apply; equality does not suppress it.

#### Scenario: Problem no longer governs current work

- **WHEN** all historical protection guards hold and applicability is below 0.5
- **THEN** The historical trigger MUST be suppressed and the regular group selected without altering the raw score.

#### Scenario: Applicability is unavailable

- **WHEN** a low-score protection opportunity has no valid applicability result
- **THEN** Protection MUST be retained conservatively, with the unavailability reason recorded.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235110154

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

### Requirement: Custom extensions have one constrained v1 request/response

The current `/v1/decisions` contract MUST use `decision`, `latest_user`,
`visible_conversation`, `history_partial`, and `assessment_target`. The last field
is null or `{from, instructions, criteria}` selecting the retained suffix of one
actual stage. HiRoute binds internal execution identity without sending internal
model/stage IDs. Responses contain `decision` and optional `assessment`, constrained
to the published allowed choices. The extension implements its provider integration,
assessment, and context handling, never selects arbitrary models, branches, or tools.

The typed producer, parser, official extension, examples, and OpenAPI MUST remain
one current v1 shape with no old-format negotiation/dual path. Earlier development
structures are not supported migration inputs. Supported released data still uses
[registered compatibility](../../../contracts/compatibility-support.v1.json) rather
than treating a historical fresh-data exercise as permission to discard user data.
The API guide/OpenAPI supply complete wire schemas; the durable policy is here.

### Requirement: Conservative fallback preserves a valid selected category

A valid category with invalid degree uses that category's primary group with an
explicit degree-unavailable reason, not probability 1 or another default category.
A wholly failed call uses heuristic policy for smart saving, or the explicit default
category's primary group when present, otherwise regular. Valid decision plus
invalid assessment keeps the decision and discards only assessment. Cancellation,
source deadline, and replay integrity failures terminate instead of fallback.
One decision call uses the shared total deadline without a new retry/state machine.

#### Scenario: Review selected but degree is missing

- **WHEN** the response selects review, while writing is the plan default
- **THEN** HiRoute MUST select review's primary group and report degree failure;
  it cannot switch to writing or manufacture a probability.

### Requirement: Overrides, route editing, and protocol help preserve user intent

New connection flow from a route MUST preserve its draft, return to the exact saved
version, and never publish implicitly. Custom-category overrides copy the entire
current effective judgment settings, including thresholds, degree, rubric, anchors,
and supplemental assessment requirements. They become independent until restore-
plan-default clears the whole override without changing category/name/models.
Single-group categories hide inapplicable degree controls. Hidden invalid fields
expand on publication failure, and switching judgment mode preserves parked edits.

Extension help MUST show first-decision and prior-stage-assessment examples, request
copy, and the current OpenAPI download, without future tool-selection entries or
another Secret-management surface. The first response need not contain assessment;
later scoring remains attributed to the old stage. Plan/session presentation freezes
probability, thresholds, selection/protection/relay reasons, and actual positions,
using the same [quality queries](../session-history/spec.md).

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

## Implementation references

- [Decision domain](../../../crates/domain/src/routing/decision.rs) and
  [definition types](../../../crates/domain/src/routing/decision_definition.rs).
- [System One codec](../../../crates/gateway/src/core_runtime/classification/system_one.rs),
  [custom codec](../../../crates/gateway/src/core_runtime/classification/protocol.rs), and
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
