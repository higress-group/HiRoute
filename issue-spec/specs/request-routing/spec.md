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
to Authorization. Duplicate or malformed credentials SHALL fail authentication.
Qoder's dedicated local model-provider route retains its existing single-bearer,
no-run-authority and no-mixed-`X-HiRoute-Token` rules. An arbitrary `x-api-key`
is not model authority. The separately validated DSH run-model carrier follows
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

#### Scenario: Inbound credentials never become provider authorization

- **WHEN** a model request is accepted through either supported carrier
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
semantics remain native-only and cannot authorize history reuse, conversion or
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
family; the provider may reject it. Unrepresentable cross-protocol state is excluded
before an Attempt. Responses `conversation` is an unsupported stored handle and
MUST be rejected rather than borrowing a reasoning-state owner's proof;
`previous_response_id` does not become a substitute for complete visible history.

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

### Requirement: Native auxiliary calls cannot change conversation routing history

The Gateway SHALL admit reusable routing history only after authenticating the
request and resolving its conversation branch. Known native auxiliary requests
(including title, summary, warming, memory and compaction calculation) and missing,
contradictory, malformed or unsupported native routing evidence SHALL use independent
empty partial history, without a history lease, inherited decision, assessment
target or checkpoint mutation. Native children with a distinct native identity
SHALL have independent branch state, including within a verified Worker task.
These headers are optimization evidence, never authorization or a route override.

Codex uses coherent native turn metadata; Claude uses request-class, prompt and
native child identities. Managed Pi, DSH and ordinary OpenCode install reversible
native adapters that convey request purpose, session, logical turn and available
durable compaction checkpoint. Hermes uses its selected conversation middleware
and validated compression lineage. Unsupported Qoder purposes remain independent
for every request. Generic API callers retain their existing identity behavior.

Native same-turn evidence MAY preserve the logical routing decision across a
rewritten compacted transcript. A new native turn after a proven compaction SHALL
obtain a fresh judgment while retaining accepted prior execution history. Semantic
continuity SHALL NOT establish byte-prefix, provider-cache, instruction or opaque
response affinity. Missing evidence permits an optimization miss, never invented
continuity. Restarts rebuild only state for which the native client supplies
evidence; Gateway in-memory history is not reconstructed from equal summaries.
For bundled adapters, an unchanged copied turn marker cannot establish continuity
for an otherwise rewritten transcript without a new native compaction checkpoint.
Such uncertain requests SHALL neither replace the existing checkpoint nor append
their execution to its routing history.

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
