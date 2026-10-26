# Request routing and bounded relay

## Purpose

A logical model request resolves a currently authorized route, freezes eligible
attempts, and delivers one coherent response. Optional provider metadata cannot
replace actual request expressibility or current source authority.

## Requirements

### Requirement: Authentication resolves aliases within one exact grant

Ingress MUST authenticate for the current protocol before resolving a Plan or
Fixed alias within that grant. Equal aliases in different grants are isolated.
A Fixed route binds the chosen source/account/raw model without inventing plan
identity. Duplicate/conflicting names inside one grant fail publication. Directory
visibility alone grants no access.

Ordinary Responses, Chat Completions, Messages and model-catalog requests SHALL
accept a single standard `Authorization: Bearer <HiRoute connection token>` for
an issued current model grant. This carrier does not authorize native provider
API Keys, OAuth credentials, revoked grants, another protocol or another alias.
Retained `X-HiRoute-Token` clients SHALL continue to work: when present, that
explicit carrier remains authoritative and an invalid value SHALL NOT fall back
to Authorization. Ordinary Messages requests MAY also carry an issued current
model-grant token through one `x-api-key` header when neither explicit HiRoute
token nor Authorization is present. An invalid higher-priority carrier SHALL NOT
fall back to `x-api-key`. This Messages-only carrier grants no additional protocol,
alias or credential authority; the same current-grant verification and revocation
apply. Duplicate or malformed selected credentials SHALL fail authentication.
Qoder's dedicated local model-provider route retains its existing single-bearer,
no-run-authority and no-mixed-`X-HiRoute-Token` rules. An arbitrary `x-api-key`
is not model authority: provider keys, OAuth secrets and unissued tokens still
fail current-grant verification. The separately validated DSH run-model carrier follows
[Worker delegation](../agent-delegation/spec.md). Worker run-model and run-control
authority remain distinct. Outbound auth is rebuilt for the exact leased destination;
inbound HiRoute/native tokens or account-override headers cannot pass through.

Fixed execution shares request projection, pricing, deadlines, and observation but
retries only its exact source's eligible Key pool. It cannot fall back to another
account/model or an otherwise allowed Plan. Plan relay remains in published scope.

#### Scenario: Standard model Bearer across native protocols

- **WHEN** a client sends a current HiRoute connection token through a single Authorization Bearer header
- **THEN** the issued grant authorizes only its published protocol and aliases, including ordinary Responses, without requiring a HiRoute-specific header.

#### Scenario: Retained explicit carrier has precedence

- **WHEN** a retained client sends X-HiRoute-Token together with its native SDK Authorization header
- **THEN** only the explicit HiRoute token determines model authority; an empty, malformed, duplicated or unissued explicit token cannot fall back to the other header.

#### Scenario: A different secret does not become a model grant

- **WHEN** a Bearer value is a native provider API Key, OAuth credential, revoked token, run-control credential or an unissued value
- **THEN** ordinary model access is denied by the existing current-grant or separately validated run-model authority, and no upstream model attempt is made.

#### Scenario: Native Messages auxiliary requests use the issued connection token

- **WHEN** a native Anthropic SDK sends only `x-api-key` with an issued current
  Messages connection token, including a request without optional Agent middleware
- **THEN** only that grant's current protocol and aliases are authorized; token
  rotation or revocation rejects the old value, and inbound credentials are not
  forwarded to the provider. Classification and optional history use their normal
  evidence rules without manufacturing Agent identity from the credential carrier.

#### Scenario: Inbound credentials never become provider authorization

- **WHEN** a model request is accepted through a supported carrier
- **THEN** the outbound attempt reconstructs authorization from the exact leased provider credential and strips the inbound model token and private metadata.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6236001882

### Requirement: Native forwarding retains payload without granting new authority

Every Attempt MUST derive from immutable original input. Same-protocol requests
retain native fields except the routed model, transport controls, plan-owned
reasoning/output, and explicitly authorized bounded history cleanup. Unknown ordinary
payload fields are not rejected solely because a local field catalog omits them.
Unknown historical semantics disable ContextHold reuse/cleanup. Unknown tool kinds,
hosted-execution extensions, auth, or destination overrides cannot bypass admission.
Cross-protocol candidates fail explicitly when equivalence is unavailable; they
cannot silently drop images, tool semantics, reasoning, or ordinary extensions.

Protocol adapters SHALL derive continuity from their supported typed history,
using the existing common history representation and prefix comparison. In Chat
assistant history, textual `reasoning` is an alias of `reasoning_content`. A sole
alias or two byte-equivalent carriers represent one canonical reasoning state;
native forwarding retains the original fields. Equivalence MUST compare actual
bytes, including Replay-backed text, rather than reference locations. Conflicting
carriers, non-text aliases, aliases on other roles and other unknown historical
semantics remain native-only and cannot authorize provider-prefix reuse, conversion or
cleanup. Authorized prefix cleanup removes both recognized carriers only within
the selected prefix. Alias normalization does not bypass session/authority scope,
tool identity, message order or content-change checks.

#### Scenario: Hermes replays duplicate reasoning through a local Chat endpoint

- **WHEN** a Hermes tool continuation or subsequent user turn supplies the same
  native session and continuous typed history, with identical `reasoning` and
  `reasoning_content` text
- **THEN** the tool continuation inherits its own decision, the next real user
  input obtains the complete attributable assessment target for the three-question
  protocol, and original native request fields remain intact.

#### Scenario: Reasoning aliases cannot conceal changed history

- **WHEN** aliases conflict, backing cannot establish equality, tool arguments,
  results or IDs change, or a different native session supplies equal messages
- **THEN** no unproven continuity or assessment target is inherited.

Request projection uses actual adapter capability, not a blanket requirement for
Exact provider metadata on reasoning/refusal/usage/streaming. Optional unknown facts
may affect estimates/catalog declaration, not provider admission. Auth, exact
source/account/model, current leases, publication consistency, user cost policy,
and expressible protocol paths remain hard constraints.

If every candidate is excluded before an Attempt for the same tool-interface gap,
the result is HTTP 400 `TOOL_INTERFACE_UNSUPPORTED`; a uniform protocol-path gap is
HTTP 400 `CLIENT_PROTOCOL_UNREPRESENTABLE`. Mixed exclusions retain HTTP 502
`NO_ELIGIBLE_CANDIDATE` rather than falsely attributing all failures to one client
field or source.

Incomplete estimates remain missing, not zero/unlimited, and do not by themselves
block ordinary requests. Conservative byte estimates beyond a model context limit
cannot reject otherwise safe input or trim history; upstream decides actual tokens.
Strict paid upper-bound policy may reject missing estimates. Output cap is the
minimum of client/configured values when both exist, otherwise the present one.
Neither means no added cap for Responses/Chat; Messages needs an explicit max_tokens.

### Requirement: Native reasoning distinguishes plan and Fixed ownership

Plan candidates MUST override caller effort/type/budget with exact saved native
settings. Fixed same-protocol execution preserves caller-native controls for the
provider, including unknown settings, or uses its sealed default when absent.
Cross-protocol Fixed controls require an explicit equivalent mapping. Unknown
controls cannot create exact cost evidence or change the bound account/model.
Request-local interpretation is fixed before Replay externalizes fields.

Responses toggles use `reasoning.effort=low/none`, preserving explicitly selected
discrete effort. Standard Responses has no reasoning-token-budget field; a budget
cannot be disguised as `reasoning.max_output_tokens`. Messages toggles use enabled
with 1024 tokens or disabled without budget. Explicit manual budgets are at least
1024 and below actual max_tokens. Adaptive profiles remain adaptive without guessed
model-name rules. Supported released toggle projections normalize at the current
wire boundary without changing stored profile/digest.

`thinking.display=omitted` controls returned reasoning text, not computation.
Plan-enabled/adaptive keeps representable omitted display; enabled uses plan budget,
adaptive never restores caller budget, disabled has no display/budget, and no final
type leaves no display-only object. Other native display values are retained only
when expressible. Fixed native controls remain provider-owned. If plan-controlled
thinking is neither enabled nor adaptive, remove only dependent
`context_management.edits` entries for `clear_thinking_20251015`, preserving other
edits/unknown siblings; objects emptied by that removal may disappear. Enabling
thinking to retain an edit is forbidden.

Omitted output retains answer, tools, usage, terminal status, and native signature/
redacted state while excluding reasoning text. Cross-protocol output never invents
foreign signatures. Structured-output format is independent of thinking controls.

### Requirement: Supported optional fields have explicit conversion rules

| Input | Required handling |
| --- | --- |
| Responses `store:false`, string `prompt_cache_key` to Chat | Preserve exact field/value in actual outbound request |
| Responses `store:true`, wrong option types, or unrepresentable include/client_metadata/reasoning.summary/reasoning.context | Reject at the relevant admission/projection boundary with field-level reason |
| Responses store/cache hints to Messages without an equivalent | Exclude that candidate rather than silently drop the controls |
| Messages `output_config.format.type=json_schema` | Preserve native format or map to Responses `text.format` / Chat `response_format.json_schema` |
| Structured Schema, description, explicit strict | Preserve exactly; absent required target name uses `hiroute_structured_output`, absent strict uses true |
| Unknown format or format additions | Native preservation; cross-protocol exclusion when no equivalent exists |
| Messages function-tool boolean `eager_input_streaming` and valid `cache_control` | Preserve native hints; cross-protocol may omit non-execution hints while retaining callable semantics |
| Chat response `service_tier` | Cross-protocol decoder accepts absent/null/string, rejects other types; native response passthrough remains unchanged |

Schema conversion does not recursively certify schemas, change required fields to
satisfy a provider, or fall back to generic JSON. Large formats use the same Replay
owner. A provider may reject a faithfully converted request.

`store:false` is not zero data retention and cache hints guarantee neither hits,
discounts, nor access. For OpenRouter, cache keys provide sticky-routing hints;
stateless Responses storage is distinct from account logging/data-use and downstream
retention. Wire mapping or HTTP 200 does not prove a provider implements an OpenAI
storage switch or disables those policies.

### Requirement: Tool history is payload, not a Gateway credential

Native tool call/result IDs MUST preserve pairing and order through restart or
in-memory eviction; they do not require a prior Gateway-issued token. Function
argument strings may be empty or invalid JSON in native Responses/Chat history.
A Messages conversion requiring structured input cannot invent valid JSON from
those strings. Text/custom input/thinking signatures/redacted data may be empty,
and missing versus empty signature remains distinct. Gateway validates structural
association needed for execution/conversion, not provider cryptographic content.
Model/declared-tool/required-choice/call identity remains valid; `strict:false`
does not require strict-tool execution support.

A model-generated empty string name in Responses historical function/custom calls
may continue natively with its failed tool result. Missing/non-string names, empty
call IDs, or empty declarations/required choices still fail. The empty-history-name
exception makes the request native-only and disables conversion, ContextHold, and
reasoning cleanup; it cannot guess a tool name or delete failed history.

Namespace/custom tools map to Chat only with unique reversible identities and
expressible schemas/format/choice. Name collisions, illegal target names, nested
namespaces, or custom grammar without an equivalent produce
`TOOL_INTERFACE_UNSUPPORTED` for that candidate with zero outbound calls. Native
candidates do not need the conversion proof. A projection failure excludes only
that candidate, not all legal alternatives. Named function/custom identity is
preserved through the [execution-fact boundary](../p0-gateway-execution-plane/spec.md).

### Requirement: Responses function encoding is negotiated in both directions

An explicitly selected standard Responses JSON adapter SHALL own outbound tool-schema
negotiation, plaintext peer projection, and returned function-argument encoding together
for the actual request and attempt. The persisted Plan SHALL retain that semantic dialect
choice after restart. Native Responses and Codex subscription paths SHALL NOT acquire this
choice from provider branding, endpoint non-membership, a readable argument, or a missing
response field. New registered sources select the current adapter; retained sources keep
their original adapter and credential destination until explicitly replaced by a newly
confirmed source/Plan.

Missing `encrypted_function_args`, explicit `[]`, and a nonempty array SHALL remain
three distinct native/replay states. An explicit nonempty declaration SHALL remain
native-only; exact replay equality is contextual and SHALL NOT grant candidate affinity
or cleanup/portability authority. For an explicitly selected standard Responses JSON
target, an absent historical declaration SHALL remain a legal standard function-call
replay even when tool-schema annotations negotiated the response extension. Negotiation
SHALL NOT require every historical call to carry vendor-specific metadata, rewrite
the native record from absent to `[]`, or establish trusted origin/assessment evidence.
Null and malformed declarations remain invalid. The standard translator may emit `[]` only after
selecting the JSON argument contract and consuming the actual request encoding
declaration (supported schema annotations, explicit encoding replay, or a native peer
item). Ordinary standard Responses requests without that declaration SHALL retain ordinary
missing-field tool replay and SHALL NOT acquire the extension in their responses. The
prepared request, successful attempt, native/JSON response and observation capture SHALL
share the same per-attempt projection, never a preceding attempt's negotiation. Schema
annotations SHALL be consumed only at schema nodes. It SHALL NOT delete a business property/example named
`encrypted`, discard tools/native options, or accept a contradictory encrypted return.
Chat function arguments and Messages tool input are intrinsic JSON producers for their
representable conversions; they do not prove arbitrary native Responses plaintext.

A native plaintext `agent_message` SHALL preserve its author, recipient, content order
and bytes in an assistant representation. It SHALL NOT become a user message or acquire
an invented tool identity. Ciphertext, mixed/unknown peer content, and historical function
calls explicitly declaring nonempty encrypted paths SHALL fail that dialect candidate without
rewriting original history or disabling independent classification/capture/evaluation.
An incompatible existing conversation requires native compaction that actually removes
the unrepresentable items, or a fresh native session. A compaction retaining those items
is not a migration. JSON, full SSE lifecycle, bounded Replay, and accepted history boundaries
SHALL enforce the same encoding contract; later terminal success cannot erase a contradiction.

### Requirement: Judgment is frozen before the pure planner

Smart-saving/custom routes MUST obtain one validated decision before Planner when
there is no inherited decision, history is discontinuous, or a real new user input
was appended. Multiple reasons cause one call. Every new user turn rejudges; tool
continuations/replays inherit within continuous history. Changing instructions,
tool declarations, or stable provider state invalidates candidate preference but
alone does not reclassify an existing turn. History replacement rejudges even when
latest-user text is unchanged; no compaction detector or unfinished-task heuristic
controls the boundary. Heuristics read full latest-user input.

Planner consumes frozen decision, groups, current eligibility, and bounded relay;
it cannot call the decision service, reread conversation, or parse service responses.
Decision results cannot enlarge candidates/Keys or change cost/auth/relay boundaries.
Fixed and free-first routes make zero decision calls. Current low-score protection
and failure rules belong to [decision policy](../decision-services/spec.md).

Lossless same-protocol opaque provider state is forwarded even across a source/model
family; the provider may reject it. Cross-protocol projection follows only declared
conversion rules, including the known reasoning omissions and actual-projection
warnings in the [execution-plane contract](../p0-gateway-execution-plane/spec.md#requirement-historical-tools-and-reasoning-do-not-require-delivery-registries).
That reasoning rule does not permit dropping unknown state, stored handles, tool
semantics or other unrepresentable fields; those exclude the candidate before an
Attempt. Responses `conversation` is an unsupported stored handle and
MUST be rejected rather than borrowing a reasoning-state owner's proof;
`previous_response_id` does not become a substitute for complete visible history.
On classified routes, a same-session contextual assessment target does not prove
provider-state ownership or authorize a cleaned retry. An unverified history rewrite
retains its original canonical state for every candidate/fallback projection; only
the same declared conversion rules apply, without additional history cleanup.
Ordinary reasoning omission does not create a cleanup retry or watermark. This does not
change the existing free-first ContextHold cleanup contract.

### Requirement: Candidate continuity has an exact scope and bounded lifetime

A ContextHold MUST bind workspace, authority epoch, grant generation, exact route/
content version, ingress protocol, and session identity. It cannot bind unrelated
aggregate publication revisions, so another plan's publication does not interrupt
continuous tool turns. Every request still authorizes against the latest publication
and checks exact candidate/profile eligibility.

Reliable native session/thread identity takes precedence. Otherwise only a complete
first real user message may establish a conservative process-local irreversible
association. Empty/conflicting/unstable/unencodable identities and generic Agent
wrappers degrade to request scope. No raw prompt, summary, or session ID is stored
in the hold. Complete typed ordered history and contextual instructions determine
continuity, not one or two prefix messages. Replacement/deletion/reordering and
system/developer/tool/state changes invalidate the strong candidate preference.
Namespace names/descriptions/child kinds/schemas/strict/format/order, choices,
parallel settings, reasoning summary, cache, and supported controls participate;
caller effort overridden by the same Plan does not falsely invalidate it.

Only one header-accepted, semantically complete, fully downstream-delivered, and
cleaned-up Attempt may update success preference through CAS. Failures, cancellation,
partial response, stale completion, or eviction cannot overwrite newer checkpoints.
The default process-local budget is 50,000,000 bytes with one-hour idle TTL, refreshed
on valid entry but not on completion; no fixed count or absolute TTL substitutes.
Restart/expiry/eviction/unavailable cache returns to normal routing without blocking.
Checkpoint preparation is read-only until optional decision-history admission
succeeds. Requests sharing an in-flight history scope use independent routing with
explicit partial history; they cannot advance its ContextHold checkpoint or inherit
its pending decision. Busy/capacity/stale optional state never rejects an otherwise
valid model request or changes an already accepted model response. This degradation
does not establish separate native main/auxiliary/subagent identities.
Checkpoint CAS and capacity MUST be reserved before decision-history admission can
change active turns, tool outcomes or transcript checkpoints. Once history admission
succeeds, publishing that reserved checkpoint MUST have no remaining fallible cache
admission step. A missed capture in an existing main history MUST be marked partial
and MUST NOT retain an assessment target across that gap; it cannot be silently
reported as complete history or reconstructed from client-supplied assistant text.
A native main marker with a matching authenticated session MAY identify only a
capture gap when its user proof is unavailable; it MUST NOT authorize history
admission or classification of an ancestor user. This includes missing same-user
continuation output, even when the next transcript contains no intervening true
user. Gap disclosure MUST remain nonblocking. If lock contention prevents a scoped
update, a fixed-size process-local fence MAY conservatively mark other retained
histories partial and discard their assessment segments on next admission.

For smart-saving and custom-branch Plans, a valid retained candidate MUST be
prioritized whenever its origin group is the freshly selected or inherited group,
regardless of any retired follow-up field. The remaining published order is unchanged. Availability relay from regular to primary
persists through that turn's tool continuations, even if weaker candidate preference
is invalidated, and records relay separately from the opening selection. New user
input rejudges and may return to regular. Old provider state/holds cannot resurrect
another category, FreeOnly-forbidden group, or old authorization.

#### Scenario: An unrelated plan is published between tool calls

- **WHEN** the exact route, authority, and continuous history are unchanged
- **THEN** the next request MUST use current authorization and inherit the same
  category/group without another decision call; actual route/grant/epoch changes
  require fresh judgment and eligibility.

#### Scenario: A fresh user judgment selects the same group

- **WHEN** a continuous follow-up selects the held candidate's origin group and that candidate remains eligible
- **THEN** the Gateway MUST prefer that candidate, including an in-group backup, even if normal group ranking prefers another candidate.

#### Scenario: A fresh user judgment changes the group

- **WHEN** the new user turn selects a different group or the held candidate is no longer eligible
- **THEN** the Gateway MUST use the newly selected group's normal eligible order and bounded relay; the hold cannot carry a candidate across groups.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/133#issuecomment-6220133330

### Requirement: Relay is bounded and cannot mix delivered answers

Each candidate tries its admitted eligible Key sequence before moving to the next
permitted candidate/group. Published relative order, attempt limit, total deadline,
and cancellation remain authoritative; price/score/cache cannot re-rank at runtime.
A failing held candidate uses the current frozen relay chain and is deduplicated,
not the historical chain that originally established the hold.

Any upstream failure, including ordinary 400, unknown error, or failure after
successful headers, may relay within the frozen Plan while no response body has
started downstream. Ambiguous faults do not infer Key/Binding isolation. Once body
delivery begins, there is no transparent retry, appended replacement-model text,
or second successful result. Fixed scope is never expanded by this rule. HTTP 200
plus complete transport without a valid complete model terminator is not success.

After a complete terminator has been delivered, later disconnect/tail/filter cleanup
failure preserves confirmed business delivery and records transport/cleanup
separately. Unsafe connections close. Only successful cleanup can create a new hold.

### Requirement: Pure temporary exhaustion exposes an accurate retry time

Before any upstream Attempt, if all candidates are excluded only by RuntimeState
cooldown/probe occupancy, return HTTP 503 in `hiroute.gateway.error/v1` with
`phase=runtime_state`, integer `Retry-After`, and matching JSON `retry_after_seconds`.
Cooldown-only uses `CANDIDATES_COOLING_DOWN`, the earliest eligible time rounded up
to at least one second. Any recovery-probe occupancy takes precedence with
`RECOVERY_PROBE_IN_PROGRESS` and one second. Mixed permanent/configuration/capability
exclusion or an already-created Attempt retains ordinary failure semantics.

### Requirement: HTTP request identity follows the negotiated protocol

Outbound H2 MUST use exactly one authorized `:authority` without a competing Host;
H1 uses the correct Host. Both retain port/path/query. This applies to model calls
and decision connection tests. Native HTTP identity cannot be supplied by inbound
credential/account override headers.

### Requirement: Hermes requests use existing task-dependent routing

HiRoute SHALL accept the managed Hermes main conversation and its model/tool continuations through one stable published Plan alias so different eligible new user turns can use the Plan's configured task-dependent execution models without rewriting Hermes per task. The existing request-routing decision, continuation, eligibility, protocol, budget and fallback contracts remain the routing policy. The Hermes integration SHALL carry real native conversation identity into the existing scoped correlation path so independent conversations retain separate decisions and observations. If native identity is missing or invalid on a trusted managed Hermes consumer, requests SHALL remain request-scoped rather than correlating conversations by equal prompt content. Integration guidance SHALL identify the main-conversation/tool-loop scope and auxiliary model slots left under native control.

#### Scenario: Different tasks keep one invocation alias

- **WHEN** different eligible new user turns reach the same published Plan from Hermes
- **THEN** HiRoute applies the existing Plan policy to select the corresponding execution models while Hermes continues to invoke the same alias.

#### Scenario: Continue a tool loop without merging conversations

- **WHEN** one managed Hermes conversation makes a tool continuation while another independent conversation submits equal user content
- **THEN** existing eligible same-turn reuse remains within the first conversation, and identity, decisions and recorded execution remain separate for the second.

#### Scenario: Native identity is unproven

- **WHEN** a trusted managed Hermes consumer sends requests without usable native conversation identity
- **THEN** those requests use request-scoped correlation and cannot obtain cross-request decision/history identity merely from equal prompts or cache affinity.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165296920

### Requirement: Execution facts and history association have separate authority

Every authorized request owns its actual execution identity, accepted output and
terminal outcome. A native session ID limits association lookup; it is not a
single active execution slot. A request's classification, selected group and
actual execution SHALL remain distinct facts. Optional association failures SHALL
NOT rewrite these facts or replace output already accepted by the client.

History branches and predecessor edges are bounded projections of captured facts,
not another execution authority. Exact, ambiguous and contextual association SHALL
remain distinguishable. A missing edge does not prove missing execution; a missing
capture does not prove an independent or complete execution. Projection rebuilding
may use only retained actual facts and their original authority, identities and
capture completeness. It SHALL NOT manufacture executions from client assistant
messages, summaries, equal text or later successful retries. Restart does not add
a persistence or reconstruction promise for discarded in-memory content.

A terminal failed request with no accepted downstream events may retry in the
same routing stage when its scoped native activation, complete comparable input
and effective instruction/tool setup all match exactly. All equal native-request
branches participate in ambiguity detection, including in-flight siblings. A
capture gap, unknown/interrupted outcome, changed setup, new activation, accepted
output or ambiguous match SHALL NOT establish this retry association. This narrow
association reuses the stage decision while preserving every failed and successful
request and frozen assessment. It remains contextual and source uncertainty is
retained; it does not establish an accepted-response edge or provider affinity.
Later real input still classifies and evaluates the preceding actual execution.

Gateway admission SHALL record request-to-stage identity independently of routing
decisions, model affinity and stage closure. The producer epoch, Gateway stage,
branch and admission revision identify this association; native session IDs and
inherited decisions alone do not. Observation consumers SHALL use this same
identity for interleaved requests before closure. Admission does not prove human
origin, completed work, complete capture or an assessment segment. A later
admission may reopen a stage without extending an earlier frozen assessment;
delayed older closure records SHALL NOT override a newer admitted revision.
Cancelled or failed requests SHALL end their own activity even after accepted
tool output. Missing closure evidence SHALL NOT imply perpetual execution or
successful task completion.

Assessment membership SHALL be frozen from the actual captured requests and their
executed model, profile, Plan and rubric before classification consumes it. Later
completion, concurrent requests, compaction or index repair SHALL NOT change that
membership. Origin uncertainty and association uncertainty SHALL remain separate
from execution completeness and eligibility for evaluation.

A retained score may remain valid for its frozen members without covering the
current enlarged stage. Explicitly clipped or incomplete current membership SHALL
NOT count as fully assessed in quality summaries merely because all retained IDs
appear in an older assessment. Derived coverage SHALL NOT overwrite the original
score or its frozen membership. Source or association uncertainty alone SHALL NOT
exclude complete execution from evaluation or reliable-score aggregation.

Protocol adapters own wire interpretation. Forwarding, accepted-output capture and
history comparison SHALL agree on tool identity, reasoning content and terminal
semantics; a provider-specific dialect remains scoped to its verified interface.
Replay owns bounded storage and integrity, not alternate protocol meaning.
Equivalent inline and externalized values SHALL produce the same supported
semantic interpretation and native output. Unsupported or opaque values retain
their original bytes without inventing a comparable semantic boundary.

For same-protocol Responses delivery, conflicting provider tool names or
namespaces SHALL retain their native values when the call ID and native tool
carrier remain stable. Such delivery SHALL remain uncertain and SHALL NOT
certify complete execution capture. A change between callable and provider-run
tool carriers, a changed call ID or reuse at another output index SHALL remain
an identity error. Cross-protocol interpretation SHALL retain its stricter
semantic requirements. Later complete requests may recover classification,
capture and evaluation without repairing the conflicting execution or rewriting
bytes already delivered to the client.

Native Agent adapters supply evidence of input position, request purpose, identity
and lineage. They SHALL NOT independently infer execution history or claim human
origin from an unverified callback. Missing evidence loses only the conclusion
that depends on it. Classification and actual capture retain their existing
availability contract. Capacity exhaustion and optional capture faults may leave
explicit bounded gaps; they SHALL NOT corrupt retained facts, block model output
or prevent fresh complete requests in the same session from recovering evaluation.

### Requirement: Native routing remains available without perfect input provenance

After authorization, usable requests SHALL retain classification, model selection,
actual execution capture and cross-turn quality evaluation independently of whether
the client proves fresh human input. This applies to Codex, Claude, Hermes, Pi, DSH,
OpenCode and Qoder. Origin uncertainty SHALL NOT globally disable these functions,
force every request to Primary, or require repeatedly creating new conversations.
A failed classification may use the existing temporary fallback; later usable
requests in the same native session SHALL recover.

Known native auxiliary purposes (including title, summary, warming, memory and
compaction calculation) SHALL remain detached from main routing history. Invalid,
missing or unsupported optional metadata instead permits observed-input routing,
without asserting that a wire User came from a human. Observed-input history SHALL
record the actual request and accepted output with an explicit
`unverified_request_user_origin` visible-content marker. Source uncertainty alone
SHALL NOT make complete accepted execution ineligible for cross-turn evaluation.
Current classifier input SHALL disclose the same uncertainty without removing its
readable content, assessment target or applicability question.
Observed and verified source evidence share the same authorized execution scope;
source uncertainty is retained on each record, not used to select separate stores.
For an accepted Responses reasoning boundary, absent, null or empty encrypted
content and absent, null or empty-array reasoning content SHALL represent no
additional content. A retained null lifecycle status SHALL NOT break the boundary.
These equivalences apply only to comparison with the bounded actually accepted
plaintext summary and subsequent output; they SHALL NOT rewrite forwarded native
items or establish an edge on empty content alone. Opaque state, nonempty extra
reasoning content, changed summaries and unknown native fields SHALL remain
ineligible for this exact-boundary proof.
Messages thinking with an absent or empty signature and no unknown fields SHALL
compare only its bounded actually accepted plaintext. Replay storage SHALL preserve
this comparison at every inline allowance without changing the native block.
Empty thinking alone SHALL NOT establish a predecessor. Nonempty signatures and
redacted thinking SHALL remain opaque and ineligible for this proof; reasoning
SHALL NOT become visible execution or acquire quality credit.
Request-local instructions, tool declarations and provider setup SHALL NOT be part
of execution branch-prefix identity. They remain part of strict ContextHold
validation. Historical message roles, order and content remain exact.
Whole-request conversion eligibility SHALL NOT determine execution association.
Ingress SHALL record each canonical message's native position and semantic
coverage once. Unmodeled retained items SHALL additionally bind their complete
native values into the bounded prefix fingerprint, including unknown fields and
Replay-backed content. Known items keep their existing canonical equivalences.
A unique matching original prefix and fully modeled accepted-output suffix MAY
continue a captured stage even when earlier items or current setup remain native-only.
Such association SHALL be contextual, retain association uncertainty and grant no
provider affinity or cleanup. Unknown semantics inside the matched assistant
suffix, missing/invalid position evidence, changed prior values and competing
candidates SHALL NOT establish that edge. A later current User or tool result
is actual new input, not part of the previously accepted assistant-output proof.
Coverage metadata is request-local and bounded; serialization, capacity failure
or missing Replay backing cannot manufacture it. Losing optional evidence SHALL
leave routing and actual capture available under their existing fallback rules.
When a native client omits the entire actually accepted plaintext reasoning from
its latest response replay, one uniquely matching old request prefix and nonempty
accepted visible text or typed tool boundary MAY continue that captured stage.
This is a visible projection, recorded as contextual association with
`association_unverified=true`, not a full response or provider-affinity proof.
Any such edge keeps that uncertainty through the stage's frozen membership,
including after later full matches. All competing matching branches participate
in ambiguity detection; native IDs, cursors and completion/admission order SHALL
NOT disambiguate a visible projection. Present reasoning must match fully; empty
reasoning carriers cannot erase nonempty accepted reasoning. Opaque or unknown
state, pure reasoning without a visible witness, changed old prefixes and changed
tool identity/arguments do not qualify. Captured facts and assessment membership
stay actual and complete when capture was complete; omitted reasoning is never
imported as visible execution or quality credit. The projection SHALL NOT grant
ContextHold reuse or provider-state cleanup. Forwarded native bytes are unchanged.
An uncertain request cannot overwrite or advance a stronger branch head. Without
a validated predecessor it starts an independent branch, while a retained completed
stage may remain an explicitly uncertain contextual evaluation target. Exact
predecessors rank before contextual candidates, and potentially missed captures
retain the existing gap fence. Native identity alone cannot grant ContextHold reuse.

An unverified native prefix rewrite SHALL start a separate execution segment and
fresh classification without permanently disabling evaluation of the preceding
captured stage. Evaluation SHALL bind to that stage's actual execution, model and
frozen rubric, retaining the source-uncertainty marker. A known capture gap or
changed publication SHALL still fence the old evaluation target. No rewritten
client summary may reconstruct missing accepted output.

A partial execution segment SHALL NOT make all later tasks on the same model
permanently partial. When a new input boundary completes with full capture, it
SHALL start an independently evaluable segment. Its opening still evaluates the
old partial target honestly; the next input can evaluate the newly captured stage.
Same-turn retries, refinalization and a new execution ID without a new input
boundary SHALL NOT repair missing earlier execution.

Native children with distinct identities SHALL remain independent, including in
verified Worker tasks. These headers are optional evidence, never authorization or
a route override. Qoder's shared main/child identity is an accepted association and
scoring-accuracy limitation; it SHALL retain classification, historical execution
and evaluation. Its reserved model base path retains the existing bearer contract
and does not itself disable routing. No body-prefix heuristic proves human origin.

Claude's native `auxiliary` and `compaction` request classes SHALL be recognized
before optional session, prompt or compression enrichment. A missing or invalid
optional identity SHALL NOT turn a positively recognized auxiliary into ordinary
history. An unknown, missing or malformed class remains observed and routable.
Claude prompt IDs identify routing activations, including notifications; they do
not prove fresh human input or completion of the previous logical task. The whole
latest native User envelope remains the current observed batch, including repeated
text and supplemental parts. Child/workflow requests are separated only by actual
native agent IDs; absent IDs SHALL NOT be replaced with an invented main/child ID.
Actual child results in a parent notification remain external context and SHALL
NOT acquire credit as the parent model's accepted output.

When a Claude producer supplies active-turn steering only as native System or
instruction context, without current-event evidence, that content SHALL remain
available to upstream execution without being relabelled as a new User through
text-template matching. The observed-User classifier and assessment context may
then lack that steering intent; this is an explicit capability limit, not a
source-verification or task-completion claim. Subsequent usable User activations
in the same native session SHALL recover classification, actual capture and
cross-turn evaluation without treating the System text as reconstructed User
execution. This condition depends on the actual producer, not all Claude versions.

Native same-turn evidence takes precedence over appended synthetic User envelopes
when the history owner still has the matching task, captured input and a valid
current-Plan decision. An explicitly identified continuation without that captured
input cannot reinterpret its summary as a new task; a subsequent verified fresh
input can recover. Pi and OpenCode's explicit continuation constraint and DSH's
verified semantic projection retain these protections. Missing or failed origin
adapters may use the separate observed-input lane. Native identity never proves a
byte prefix or supplies missing tool outcomes.

A valid main-purpose v2 adapter envelope may omit the optional input descriptor
when the producer cannot bind a current message window. The input consumer SHALL report
correlation-only evidence without selecting human content or a position; an
explicit malformed declaration, including null, remains invalid. A later DSH
model-selection notification SHALL NOT be relabelled as human input to obtain a
window. The original request remains available to observed routing and capture,
and a later independently valid window may recover. Neither omission nor an
explicit observed batch supplies accepted execution, task completion or an
assessment target.

Codex persistent window counters and Claude's one-call compaction flag remain
negative provenance evidence. Cold compacted Codex sessions can nevertheless start
observed-input capture from their current usable request. Gateway restart discards
in-memory history: it SHALL NOT import an old summary or claimed prior output as
accepted execution. New A and B in that same session SHALL be classified and
captured, and B SHALL be able to evaluate A's newly accepted execution. Uncaptured
older context remains partial. Counter rollback and unsupported producers disclose
uncertainty rather than permanently disabling smart routing.

Codex native metadata SHALL recognize only the supported explicit auxiliary kinds
`memory`, `prewarm` and `compaction`. A coherent canonical memory request remains
auxiliary when it omits thread/session/turn identities; compatibility projections
may still carry those identities. Unknown kinds remain observed and routable.
Duplicate or conflicting metadata SHALL NOT splice purpose or identity across
channels. Confirmed competing session/thread claims lose session association;
missing or malformed optional enrichment alone does not discard independently
valid native identity. Parent and fork references never join a child's execution
to the parent's stage.

The latest supported plaintext Codex peer batch MAY supply current classifier and
capture input at its actual native event position, with source uncertainty. The
original typed peer items, including author, recipient, content variants and order,
SHALL remain unchanged in execution and fingerprint evidence. A later ordinary
User event supersedes earlier peer context. Mixed plaintext/encrypted peer content
remains opaque: its readable wrapper cannot reconstruct encrypted intent or prior
accepted output, and its current event SHALL NOT fall back to an ancestor User as
the task input. Native turn/window metadata proves neither task completion nor
execution deduplication. Optional projection failure retains observed routing and
cannot reject otherwise successful provider output.

Hermes public whole-turn hooks also receive native async completion notifications.
Matching text, a turn change, an initiated-turn flag and older producers' false
continuation constraint do not prove human origin. Such requests are observed,
including on supported Hermes 0.20, rather than globally unavailable. Compression
lineage remains separate evidence. No adaptation may fabricate human provenance
or claim accurate task association merely because output and routing succeed.
Current Hermes adapters SHALL preserve observed-batch semantics and SHALL NOT emit
clean-text input selectors. Retained observed-input v1 digests and reverse ordinals
are non-authoritative compatibility hints: a unique old match cannot identify a
decorated current input, select an earlier User or demote later Users. Without a
validated final native message-instance descriptor, routing SHALL retain the
complete latest wire User envelope, including multipart content and reminders,
with explicit source uncertainty. Missing optional hooks or lineage SHALL preserve
normal authorization, classification, actual capture and cross-turn evaluation;
uncaptured old execution remains unavailable.

Sustained availability and scoring accuracy SHALL be recorded as separate acceptance
results. A-to-B acceptance requires valid decisions on both, captured A and B,
evaluation of A's actual execution on B, and actual selection of different model
groups. Optional history conflicts, resource pressure and side-call failures SHALL
remain isolated from main output. Native acceptance uses the real configured client
through Gateway to a controlled upstream, including compaction, same-session restart,
adapter fault/recovery and full configuration Restore followed by actual startup.

A verified new native user after compaction SHALL obtain a fresh judgment while
retaining accepted prior execution history. Semantic
continuity SHALL NOT establish byte-prefix, provider-cache, instruction or opaque
response affinity. Missing evidence permits an optimization miss, never invented
continuity. Restarts rebuild only state for which the native client supplies
evidence; Gateway in-memory history is not reconstructed from equal summaries.
For Pi, DSH and OpenCode adapters, an unchanged copied turn marker cannot establish continuity
for an otherwise rewritten transcript without a new native compaction checkpoint.
Such uncertain requests SHALL neither replace that verified checkpoint nor append
their execution to its proven branch. Independent observed branches in the same
authorized scope retain actual execution and contextual evaluation with explicit
source and association uncertainty.

#### Scenario: A native summary overlaps its parent

- **WHEN** a main tool call is in flight and a same-session auxiliary request arrives
- **THEN** the auxiliary request can execute independently, its output cannot enter
  main routing history, and main output and subsequent tool routing remain valid.

#### Scenario: Optional adaptation or history storage fails

- **WHEN** native metadata reads time out, a hook fails, a lease conflicts, history
  capacity is insufficient or an optional checkpoint write becomes stale
- **THEN** the affected optimization degrades without changing delivered model
  bytes or committing partial history; subsequent admitted main requests retain
  their valid history. Authentication, actual provider failures, cancellation and
  required native compaction retain their existing errors.

## Implementation references

- [Request adapters](../../../crates/gateway/src/adapters/request),
  [reasoning normalization](../../../crates/gateway/src/adapters/request/reasoning.rs), and
  [real protocol matrix](../../../tools/e2e-harness/tests/p0_gateway_protocol/production_matrix.rs).
- [Pure planner](../../../crates/gateway/src/planner.rs),
  [ContextHold](../../../crates/gateway/src/context_hold/store.rs), and
  [native continuation scenarios](../../../tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs).
