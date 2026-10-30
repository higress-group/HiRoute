# Gateway request and decision boundaries

The production listener is composed in [server.rs](src/server.rs).
[publication/compiler.rs](src/publication/compiler.rs) turns one accepted
publication into immutable request authority. Read
[core_runtime.rs](src/core_runtime.rs) for the real request path: authenticate and
bind authority, prepare the canonical request and Replay, obtain a request-owned
branch decision, plan and freeze candidates, then admit execution into
`hiroute-gateway-core`. The core owns attempts, response commit, cancellation and
resource cleanup; the Gateway does not run a second execution loop.
[Compiled candidate facts](src/publication/candidate.rs) retain typed profiles
and their existing digests per execution. Requests still acquire current core
configuration and compare its exact bytes/generation before consuming those
facts. Planner reuses a digest only when every corresponding immutable field
matches; its ordinary untrusted-input entry retains complete digest validation.
[Prepared execution facts](src/publication/execution.rs) also retain the existing
sealed envelope, exact lease target and their canonical digests. A bounded
request-local handoff can reuse them only for the same binding and exact sealed
byte allocation. Copied or changed envelopes retain full validation; request
reasoning derives a separately checked profile. The current attempt still checks
its target, profile identity, protocol, credentials and operational authority.
Live credential/grant checks stay at their existing request/attempt boundaries.

The [model selector](src/dispatch/selector.rs) and
[structural scanner](src/content_ref/ingress/scanner.rs) skip ordinary string runs
while preserving scalar escape, control and boundary transitions. Small
memory-backed ingress and nested arguments feed the existing serde parser from a
temporary, budgeted slice; disk-backed and pressured inputs retain the bounded reader.
[Runtime fingerprints](src/runtime_fingerprint.rs) reuse complete normalized
content within one request. MD5 here is an ephemeral equality check with framed
domains; authorization, native proofs and persisted observation HMACs retain
their cryptographic algorithms. [Prepared input](src/composition/prepared.rs)
may retain bounded candidate templates until the selected attempt uses them;
current authority/profile checks still run, and cleanup rebuilds the template.
Template serialization borrows the existing JSON tree while expanding its wire
markers. Replay serializes each canonical range under its existing memory gate
and derives range lengths from the same checked stream counters.
Production ingress moves its native tree into the canonical request; context
identity keeps only the original `metadata` and `client_metadata` roots so
externalizing native content cannot change Claude/Codex identity enrichment.

[Optional content encoding](src/observation/content_work.rs) uses one ordered,
bounded worker per observation channel set. Its immutable snapshot retains only
instructions, messages, tools and charged semantic Replay backing. It does not
retain the raw ingress body. Request capture closes before response capture so
their transcript roots remain related. Accepted-output history and usage update
at delivery; neither waits for content encoding or SQLite. Capacity or capture
failure reports a gap and stops dependent capture for that request. The final
Replay owner releases files and reservations on all paths.
The existing content queue splits its byte ceiling between optional jobs and
encoded records. Fixed slots hold pointers; each admitted job's existing charge
covers its allocated header, rather than preallocating 256 maximum-size tasks.
Only the content worker can park on record credit, with a
bounded stalled-record wait; foreground producers remain nonblocking. Encoded
bursts leave room for concurrent foreground jobs; queued work still leaves room
for one complete chunk record. Both use the same unchanged byte ceiling. Delivery
releases credit through a condition variable, and a stalled job reports loss
without publishing dependent completion roots.

Ordinary Responses, Chat Completions, Messages and model-catalog clients may carry
their HiRoute connection token as `Authorization: Bearer <token>`. The current
publication still verifies the issued grant, protocol and aliases. Retained
`X-HiRoute-Token` clients keep explicit-header precedence, including when native
SDKs also send Authorization; an invalid explicit token never falls back to it.
Messages SDKs may send the same issued model-grant token through a single
`x-api-key` when both higher-priority carriers are absent. This carrier still uses
current grant/protocol/alias verification; arbitrary provider keys grant no access.
Qoder's dedicated entry and delegated run authority keep their existing rules.
Upstream authorization is rebuilt from the selected provider lease. The
[listener scenario](../../tools/e2e-harness/tests/p0_gateway_runtime/bearer_auth.rs)
covers admission, rejection, token rotation and outbound isolation.

[Context overflow classification](src/runtime/driver/context_overflow.rs)
preserves a closed HTTP 400 cause for native compaction recovery after bounded
candidate relay is exhausted. It emits static sanitized error text and does not
infer overflow from generic errors or replay committed output. The
[listener scenarios](../../tools/e2e-harness/tests/p0_gateway_runtime/context_overflow.rs)
cover all three ingress protocols, successful relay and stale-cause rejection.

Branch candidates may use distinct execution keys for different reasoning settings
on the same model. Publication freezes `source_binding_id` for their current Native
credential membership checks; execution keys continue to own selection and runtime
state. Ordinary candidates retain their existing stable key. Older aggregate
projections are authenticated before Application rebuilds this binding from Plans.

The standalone `hirouted` entry can attach the same typed diagnostics using
`--diagnostics-root <private-root>` and an optional temporary
`--diagnostic-level-override`. These diagnostics do not enable session content
capture or execution-fact storage. At Info, failed attempts retain the safe
actual model and reasoning controls, HTTP protocol/status and body commit state.

For operational lifecycle telemetry, [accepted response capture](src/observation/content.rs)
reports transport writes only after they succeed, including an empty end-stream write.
[Product lifecycle validation](../domain/src/observation/lifecycle.rs) preserves its actual
zero byte count and frame identity; the [bounded receiver](../observation/src/writer/lifecycle.rs)
ACKs it in sequence before RequestFinished. These transport events describe delivery;
semantic output, usage and the business receipt retain their own facts.
An accepted Attempt means its output was delivered. A later native model failure
or truncation retains that identity and its failed model event; the Request and
history stage remain failed, allowing the receipt to preserve actual execution.

The registered Codex subscription request dialect omits redundant completed or
explicit-null reasoning-item status that its input parser rejects; null conveys
no completion proof. Public Responses paths keep the field, and unfinished states
are never normalized into completed history. [Native request projection](src/adapters/request/native.rs) applies
this normalization per candidate without changing the original request, reasoning
payload or other item metadata. Cross-protocol Responses output uses one shared,
response-scoped synthetic item identity across incremental and buffered renderers;
provider-native item IDs stay exact. The [request regression](src/adapters/tests/codex_replay.rs)
and [response lifecycle scenarios](src/adapters/response/client_stream_tests.rs)
cover endpoint isolation, replay and identity consistency.
[The standard Responses JSON dialect](src/adapters/responses_dialect.rs) is an explicit
request/attempt adapter contract. New registered Token Plan sources select it through
their exact Responses endpoint, including the ordinary source-local `UserDeclared`
model path. The endpoint registry does not imply executable catalog model qualification;
the [save/reopen regression](../daemon/src/control/runtime/model_connections/registered_token_plan_tests.rs)
uses the real registered check and durable source transaction without inventing catalog capabilities.
Native Codex subscriptions and old saved sources keep
their original dialect. It consumes boolean `encrypted` annotations only at JSON Schema
nodes, retains the complete tool declarations, and projects all-plaintext `agent_message`
content into assistant blocks carrying the original author/recipient and ordered text.
The [schema-position owner](src/adapters/schema_encoding.rs) is shared with
[ingress compaction](src/content_ref/ingress/schema.rs): schema nodes retain their protocol structure
for encoding negotiation and shape validation, while large descriptions, defaults and examples
remain Replay content. Business keys with the same names are not annotations. The canonical
IR and all request templates collect nested content references; the request memory budget
still charges retained nodes, keys and strings, including schema containers named `type` or
`role`; only string tags use the inline identity path. The shared
[template JSON value locator](src/adapters/request/template_json.rs) excludes literal object
keys from reference substitution without changing serde's wire bytes. Retained structure and
Replay-backed data keep equivalent output; value-reference counts and encoding validation
remain strict.
Peer author/recipient/item identities stay budgeted inline, and peer body text remains independently backed. Reserved Replay-marker
literals in those identities are preserved on native passthrough but rejected by this
metadata-text projection, which cannot embed a backed identity safely.
[Production-path Replay controls](src/adapters/responses_dialect_replay_tests.rs) compare
scan/parse/compact/IR/template output with inline bytes across native Responses, standard
Responses and portable Chat/Messages tools, including large data and budget refusal.
Original ingress history/source evidence is untouched. Cipher/mixed peer content and
historical function calls explicitly declaring encrypted paths cannot cross this boundary.
Such a function-call replay uses the same closed candidate-local portability failure;
malformed declarations remain ingress errors and encrypted source responses remain strict.
For reasoning history, [native request projection](src/adapters/request/native.rs)
uses the retained canonical encrypted state classified by ingress. Missing, null and
empty-string carriers keep their original shape; nonempty or Replay-backed opaque state
excludes the standard target, including legacy provider state without a sidecar.
Authorized prefix cleanup uses original canonical positions and excludes tool declarations.
The [reasoning regressions](src/adapters/responses_dialect_reasoning_tests.rs) and
[Replay checks](src/adapters/responses_dialect_reasoning_replay_tests.rs) cover these
boundaries without rewriting original history. The [preflight owner](src/composition/ports.rs)
records this closed portability failure per candidate so a compatible native member can
still execute; [listener controls](../../tools/e2e-harness/tests/p0_gateway_runtime/codex_nullable_reasoning.rs)
also cover the standard-only no-candidate result and subsequent tool-stage assessment.
Standard Responses function-call replay may omit vendor encoding metadata even when
schema annotations negotiated the response extension. Codex 0.147's
[custom-provider request builder](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/core/src/client.rs#L837)
intentionally removes this optional field from its outgoing history. That omission
does not rewrite the native record or supply trusted origin evidence. A fresh native session, or native compaction which actually removes unsupported
items, is required before choosing the new source for such a conversation; no history is
rewritten and compaction itself may require a call through the old source.

The selected translator declares `encrypted_function_args: []` only when this actual
request negotiated the encoding extension. Chat/Message JSON tool producers use the same
request-bound result when mapped to Responses. Ordinary Responses clients receive no new
encoding field, so dropping unknown extensions cannot create an artificial history break.
The prepared request passes the same projection to the successful attempt, native/JSON
renderers and observation tracker; retry and fallback attempts derive their own result. Native Responses preserves missing, empty and nonempty declarations; malformed
or contradictory declarations fail without a successful terminal claim. Input replay and
accepted-boundary digests retain those distinctions. Nonempty encoding remains native-only
and its exact replay is contextual; it grants neither cross-protocol/cleanup permission nor
candidate affinity. [Encoding tests](src/adapters/responses_dialect_tests.rs)
cover the full Codex V2 declaration shape, complete SSE lifecycle, buffered JSON, peer
Replay, producer-specific conversions and negative cases. [Stored choices](../domain/src/routing/stored.rs)
persist the standard dialect across restart; old missing choices are never upgraded.
These deterministic checks do not establish real model child-task execution or scoring.

Request history and response rendering share the
[phase representation rule](src/adapters/mod.rs): `final_answer` maps to the ordinary
assistant channel on Chat and Messages; other named phases require Responses.
Native Responses keeps the original phases. The
[history projection cases](src/adapters/tests/message_phase.rs) cover this boundary.

Messages thinking computation is projected in
[request/native.rs](src/adapters/request/native.rs); request-local `thinking.display`
is resolved by [ingress.rs](src/adapters/ingress.rs) before Replay externalization.
[Native response projection](src/adapters/response/native_passthrough.rs) preserves
signatures and usage while enforcing omitted text. Cross-protocol Messages
renderers omit foreign reasoning without inventing signatures. The
[production protocol matrix](../../tools/e2e-harness/tests/p0_gateway_protocol/production_matrix.rs)
covers the listener boundary, including tool continuation.

Authenticated request intake uses the same request-owned [Replay backing](src/replay.rs)
before and after alias selection. The [model locator](src/dispatch/selector.rs)
retains only bounded lexical state and the alias; canonical JSON/protocol validation
still runs before Provider admission. [Real listener regressions](../../tools/e2e-harness/tests/p0_gateway_replay/ingress.rs)
cover large model-last bodies across all ingress protocols and partial-owner cleanup.

Same-protocol Responses forwarding preserves provider tool names and namespaces
even when they disagree across snapshots of the same call. The call ID and
native carrier (function, custom or provider-run web search) must remain stable.
Conflicting metadata makes completion unknown; canonical capture remains strict
and the affected history stays partial. The projector neither buffers the whole
response nor replaces an already delivered instruction with the final snapshot.
This keeps delivery available without claiming the upstream semantics are valid.
The [native tool conflict scenario](../../tools/e2e-harness/tests/p0_gateway_runtime/native_tool_conflict.rs)
checks unmodified delivery, partial history and recovery of later complete work.
Cross-protocol conversion and Messages/Chat identity checks retain their existing
rules. A client that cannot consume a contradictory native stream may still fail.
Native Chat argument deltas retain empty, null or absent tool IDs on the wire.
The bound index supplies identity for validation only; repeating the full ID in
each delta would make Chat clients concatenate it. The [Chat delta regression](src/adapters/response/chat_tool_delta_tests.rs)
checks client-side accumulation against canonical capture and tool-result replay.

[Historical request projection](src/adapters/request/history.rs) separates completed
Responses commentary replay to Chat from the live response renderer's phase rules.
The adapter returns candidate-local phase loss; actual materialization and rejected
candidate diagnostics use the closed `request_projection` event without rewriting
native history or assessment facts. The [mixed-protocol return scenario](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches/responses_chat_history.rs)
checks that a later Regular Chat execution remains possible after Primary Responses
commentary, tools and a final answer.

Response-side [moderation notifications](src/adapters/response/responses_sideband.rs)
are validated separately from model output. The known `response.metadata` moderation
shape keeps its exact native wire and does not add canonical content or change a
terminal, refusal or error. Unknown variants, including safety-control siblings,
remain unsupported; [stream regressions](src/adapters/response/moderation_metadata_tests.rs)
cover fragmentation, lifecycle placement, tool completion and negative controls.

Responses reasoning history is admitted by
[the ingress shape owner](src/adapters/responses_options.rs) before Replay
externalization. Known summary/content text and encrypted state keep their declared
protocol mappings and actual-projection WARN accounting. Unknown item/part fields
or plaintext variants remain intact for native forwarding, exclude cross-protocol
candidates and cannot authorize provider-prefix affinity or cleanup. The
[reasoning-extension regressions](src/adapters/tests/reasoning_extensions.rs)
check inline/Replay equivalence and known signed/redacted/encrypted controls.

## Native session correlation

The `native_routing_evidence` debug event records the bounded inbound adapter
header's presence, a local HMAC token, the actual V2 consumer verdict and applied
history hints. The projection verdict comes from the actual input consumer and
distinguishes applied/rejected message windows from correlation-only hints.
An accepted envelope does not by itself prove that its window was adopted.
It retains no raw headers, native IDs or credentials. A present header without
a consumer verdict does not prove that the adapter was accepted.
The event shares the model request token with execution diagnostics; internal
headers remain removed upstream. Disabled or saturated diagnostics cannot change
routing or capture. See the [identity diagnostic boundary](src/context_hold/identity/diagnostic.rs).

[context_hold/identity.rs](src/context_hold/identity.rs) resolves native identity
before request content is externalized. Routing purpose and validated lineage
select history branches; observations retain the native physical identity.
Verified Worker tasks take precedence. Ordinary clients supply these native fields:

| Client | Accepted native identity |
| --- | --- |
| Claude Code | Coherent `X-Claude-Code-Session-Id` / Messages JSON session inside `metadata.user_id`; optional actual `X-Claude-Code-Agent-Id` separates native agents |
| Codex | `thread-id` / `session-id`, with matching Responses `client_metadata` dimensions |
| Qoder | `session-id`, retaining its existing generic session namespace |
| Pi | pi-ai session headers scoped by `pi (...)` User-Agent |
| DeepSeek Harness | `x-deepseek-harness-session-id`; pi-ai session headers scoped by `deepseek-harness/<version>` User-Agent |
| OpenCode | `x-opencode-session-id`, including custom providers and all three supported ingress protocols |

OpenCode's [released request producer](https://github.com/anomalyco/opencode/blob/733b2c627e5aec39cb33cc22373bdce33053ef2c/packages/opencode/src/session/llm/request.ts)
sends the actual session in `x-opencode-session-id`. Its
[current cache affinity](https://github.com/anomalyco/opencode/blob/d21b0790a6c146815f28fea604e41e922f009590/packages/core/src/session/affinity.ts)
can share a parent or fork source, so `x-opencode-session`, `x-session-affinity`
and `X-Session-Id` are not OpenCode identity aliases and need not match that ID.
Parent, project and request headers also do not identify the current session.
This input support does not add OpenCode managed configuration or Worker launch.

For pi-ai, Responses sends its native `sessionId` as `x-client-request-id` and
optionally `session_id`; OpenRouter compatibility uses `x-session-id`. Messages
can send `x-session-affinity` or `x-session-id` when native affinity is enabled.
See the pinned [Responses transport](https://github.com/earendil-works/pi/blob/e6b9a6af5d45fd191f8e90b56176a0c41aab3199/packages/ai/src/api/openai-responses.ts)
and [DSH adapter](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/llm-pi-ai/src/adapter.ts).
Branding limits interpretation of these fields; it is not authentication.
Generic request IDs, prompt cache keys and other clients' affinity headers cannot
establish a native session. A client that sends no native identity, including
DSH's current standard pi-ai Messages configuration or disabled pi-ai caching,
retains the existing conservative first-user inference or request scope.

Recognized IDs must be nonempty, at most 128 bytes, and free of control characters.
Duplicates, invalid values, conflicting dimensions or competing vendor families
stay request-scoped. Coherent generic Codex dimensions retain their historical
namespace, including Pi's Codex transport. Claude header and metadata share the
existing Claude namespace. Session storage uses the producer's scoped HMAC;
raw IDs are not persisted and existing observations are not rewritten.
Chat user messages may also carry a client `message_id` annotation: a nonempty,
ASCII graphic string of at most 128 bytes. The adapter retains it in the native
payload and excludes it from normalized history. It supplies no session identity
or model-state authority. Opaque values, other roles and unknown semantic fields
retain the conservative history boundary.

[Claude identity](src/context_hold/identity/claude.rs) retains the released session
namespace, requires header/metadata agreement and rejects duplicate metadata
session claims. Its actual agent ID is a separate scope dimension; missing agent
identity is never replaced with an invented main or child.
[p0_gateway_native_sessions.rs](../../tools/e2e-harness/tests/p0_gateway_native_sessions.rs)
covers the real listener and observation boundary across Responses and Messages,
plus OpenCode's native identity across Chat Completions and shared cache affinity.

[decision_history.rs](src/core_runtime/decision_history.rs) derives optional capture
scope from authorization independently of ContextHold. One native session can have
multiple concurrent executions. [Branch resolution](src/agent_turn_history/store/branches.rs)
combines the typed request prefix with captured accepted response boundaries;
identical text, native request IDs and completion order do not establish parentage.
The response comparison ignores zero-byte text placeholders added to a tool-only
response by a native client, after checking any Replay reference. Whitespace and
other actual content remain significant; original requests and capture are preserved.
Semantic JSON uses one [canonical Replay producer](src/replay.rs). Only an exact
registered range with that provenance can prove canonical tool-output equivalence;
raw argument strings retained under memory pressure remain forwardable. Branch
lookup can bind such an old prefix by distinct raw-byte evidence, with contextual
association and no provider affinity, so later normal tool stages still recover.
The [JSON boundary regressions](src/agent_turn_history/boundary/json_reference_tests.rs)
and [capacity recovery cases](src/agent_turn_history/boundary/json_reference_budget_tests.rs)
cover both protocol decoding and the actual shared allocation refusal.
The `e2e-test-control` feature enables the JSON map-order feature used by the
shipped daemon through ACP, including separately built listener processes.
Outer test-runner flags alone do not establish a child executable's feature graph;
the prepared SUT recipe binds its manifest and source revision. Default-feature
library checks separately cover the standalone Gateway's JSON representation.
Bounded Responses reasoning summaries and encrypted content are compared with
the actual accepted reasoning events, including empty-summary placeholders.
The [boundary owner](src/agent_turn_history/boundary.rs) also computes one bounded
plaintext function-encoding proof from the already measured tool digests. It permits
missing versus explicit-empty encoding only as contextual association, binding the
whole ordered reasoning/text/tool boundary and canonical arguments. Original full and
visible digests stay unchanged; opaque state, changed content and ambiguous concurrent
predecessors do not gain an edge. [Encoding boundary cases](src/agent_turn_history/boundary/function_encoding_compat_tests.rs)
cover both directions and production Replay; the listener tool-stage case checks two
actual members followed by a new input's assessment. No request or execution fact is rewritten.
Encrypted content is compared only by its exact bounded digest after Replay
integrity verification. It is never decrypted or projected into assessment;
equality provides contextual continuity without provider affinity. Changed or
uncaptured ciphertext, changed summaries and unsupported native fields lose the
edge. [Encrypted continuation cases](src/agent_turn_history/boundary/encrypted_tests.rs)
include a failed tool result followed by recovery in the same execution stage.
Known native maintenance calls retain
their receipts and costs but carry an explicit optional auxiliary marker so the
timeline does not manufacture pending user turns. Older unmarked facts remain
visible under their original conservative interpretation.
Each admission owns a unique ticket and frozen assessment snapshot. ContextHold
commits are branch-scoped; optional cache misses cannot discard execution capture.
Transient history-metadata contention waits for the in-memory checkpoint/admission
transition rather than dropping that request's assessment. No request, external
call or streaming lifetime holds the mutex. Incoming projection and transcript
measurement run outside it. [Idle maintenance](src/agent_turn_history/store/maintenance.rs)
visits at most 32 indexed entries plus the current scope's 32 heads; current-scope
TTL remains exact and protects pending requests and pinned snapshots. The
[contention and expiry regressions](src/agent_turn_history/admission_contention_tests.rs)
cover both admission entrances, unrelated growth and eventual reclamation.
Freezing request membership does not prove that the stage ended. The store keeps
capture coverage, actual provider outcomes and stage-closure proof independently.
A known successful tool response can leave its stage open; an unknown provider
outcome cannot be repaired by a later activation or final answer. A proven new
activation or terminal can close a known stage, while association loss leaves
evaluation partial. A later independently complete stage recovers normally.
[Opening evidence](src/agent_turn_history/store/opening.rs)
compares the old request and actual current input separately. Reused branches
also require an appended current input to prove a new activation; a changed native
identifier alone leaves an exact tool continuation in its existing stage. Conversely,
a repeated identifier cannot suppress a witnessed new current input. Explicit
continuation metadata makes that input unavailable before boundary selection. Missing response
completion does not prove a rewritten request. New-message evidence cannot reuse
the old execution or repair its partial score; missing comparison remains unknown.
[Closure regressions](src/agent_turn_history/stage_closure_tests.rs) check both
directions and preserve previously frozen snapshots.
Request JSON observation uses the same Replay-aware canonical serialization as
comparison, with bounded chunks and integrity checks instead of retaining internal
locators. [Observation regressions](src/observation/content/request_json_replay_tests.rs)
cover nested strings, tool JSON, literal-marker isolation and missing backing.
Content-part owners compute message/content identities once and pass them to their
chunks; the capture lifetime owns its fixed fork identity. Digest algorithms,
transcript ordering and queued envelope shapes remain unchanged.
Response ciphertext remains excluded from retained observation content.
The response tracker retains accepted lifecycle evidence separately from that
presentation policy. Converted responses use the protocol renderer's omission
rules: upstream-only content cannot become an accepted client boundary, while
deferred events are bound to the later accepted wire unit. Buffered conversion rejects a failure that would discard previously
decoded content; it cannot certify that content as delivered.
Native passthrough keeps the evidence of its actual accepted bytes.
An admitted request keeps its capture ticket if publishing its frozen decision
temporarily lacks history resources. The [request guard](src/core_runtime/decision_history/deferred_decision.rs)
retries that publication once after downstream delivery, under the same store
budget and ticket checks. This loses optional provider affinity while the result
is pending. A final capacity failure leaves a gap; it cannot turn already delivered
output into a model error. Initial capture admission remains bounded and may fail
independently, so this does not promise lossless capture under every resource fault.
Same-scope actual completed stages remain contextual candidates with disclosed
association uncertainty when no unique predecessor is established. Unknown association
never grants provider-prefix affinity. Unsupported response shapes may lose an exact
edge while retaining capture and evaluation.

The store retains a 500 MB total budget and a one-hour idle lifetime, with 32 live
requests/heads and 128 retained terminal records per scope. Snapshot pins and indexes
are charged. A full budget or optional capture fault records a recoverable gap and
cannot replace delivered model output. Scope-hash gap stripes are fixed at 256;
collisions disclose extra uncertainty rather than false complete capture. Authorization,
Replay integrity, provider failures and source cancellation retain their own behavior.
Core preserves request cancellation as a typed transport result even before an
upstream attempt starts. Product observation records that request as cancelled and
its admitted stage as interrupted; complete request capture can remain complete.
Ordinary preparation failures remain failed. Neither a final cancelled token nor
error text replaces the actual termination cause, and late cancellation cannot
rewrite completed delivery.
The [ingress coverage map](src/model_ir/native_history.rs), shared by all three
protocols, to keep conversion eligibility separate from execution association.
An unmodeled prior item binds its complete native content into the original
request prefix. Later fully modeled accepted text/tool output can continue its
actual stage with contextual association; unknown replayed output cannot.
Position metadata is ephemeral and cannot authorize provider affinity or cleanup.
The [cross-protocol coverage cases](src/agent_turn_history/boundary/coverage_tests.rs)
and [retained-peer stages](src/core_runtime/decision_history/retained_peer_tests.rs)
exercise fresh recovery without removing old input.
Native Codex peer `agent_message` items use the
[Responses ingress](src/adapters/responses_ingress.rs) native-only path: same-protocol
forwarding preserves author/recipient, text and ciphertext. Cross-protocol conversion
and provider-prefix affinity remain unavailable. The
[Codex current-input codec](src/core_runtime/native_routing_view/codex.rs) selects
the latest plaintext peer batch at its canonical event position, including when no
wire User exists, without changing original roles or history. A later User event
supersedes old peer context. Classification and capture disclose source uncertainty;
mixed plaintext/ciphertext stays opaque and cannot recover encrypted intent from
a wrapper or ancestor User. The
[native protocol regressions](src/adapters/tests/native_responses.rs) cover forwarding
without fabricating a user role for peer content.
[Codex metadata admission](src/context_hold/routing/codex.rs) checks coherent native
channels before purpose, accepts memory without thread/turn IDs, and detaches only
known memory/prewarm/compaction calls. Unknown kinds remain observed. Positive
session/thread contradictions discard association; malformed optional enrichment
alone does not erase independently valid identity. Parent/fork references remain
separate from the current thread's scope.
[Claude native routing](src/context_hold/routing/claude.rs) recognizes only its
supported `auxiliary` and `compaction` request classes before optional identity and
prompt enrichment. Unknown classes stay observed and routable. A prompt ID enables
a routing activation without proving human origin, a clean input instance or prior
task completion; the complete native User batch remains visible with uncertainty.
The observed Claude Code 2.1.287 runtime sends messages queued during an active
tool call as native System context with the same prompt ID. Those bytes still
reach the upstream, but the observed-User classifier and assessment context can
miss their intent. This is a current producer capability limit: no English
wrapper is extracted or promoted into a new User. A later actual User activation
in that same live session restores classification, capture and exact-target
evaluation. Other Claude versions need their own producer evidence.
[Native routing admission](src/context_hold/routing.rs) separates usable input from
source proof. Known auxiliary purposes remain detached. Missing or unsupported
metadata instead uses an observed-input lane, with
`unverified_request_user_origin` in both current classifier input and captured
history. Complete accepted output stays eligible for cross-turn evaluation. This
covers Claude main/subagent/workflow, Hermes, Qoder and cold compressed Codex;
unknown provenance cannot permanently pin an Agent to Primary. Qoder's shared
main/child identity remains an accepted association limit. Hermes compression roots
still require complete native lineage and fresh current-state evidence.
That lineage is validated independently of the optional routing-context envelope.
A missing, malformed or unsupported envelope preserves an independently valid
compression root while leaving task/input origin unverified; it cannot move the
captured request to a different scope merely because the adapter recovers.
Conflicting native identities, invalid lineage and known auxiliary purposes keep
their existing isolation. The [Hermes listener cases](../../tools/e2e-harness/tests/p0_gateway_runtime/hermes_identity.rs)
verify exact assessment membership across context failure and recovery.
The [continuity guard](src/context_hold/continuity.rs) retains saved native tasks for
known continuations without granting provider/KV affinity. A known summary with no
captured original input uses the temporary unavailable-input route; subsequent real
new A and B must restore classification, capture and evaluation in the same session.
Gateway restart never imports claimed prior client execution as captured history.
Partial old stages remain partial; a fully captured independent new input can start
a complete stage even on the same model. Retries cannot repair missing execution.
The [current input view](src/core_runtime/native_routing_view.rs) uses a validated
final-wire v2 event window, or the complete latest observed User envelope. It keeps
all original message roles and content. Legacy DSH v1 proofs can select their last
validated input batch without demoting other Users; Hermes clean-text v1 hints are
non-authoritative and cannot reposition input. Source, association and capture
uncertainty are disclosed separately using existing decision content markers.
Known summary input is never recorded as an executed original user task. Its actual
accepted output can still be captured with unavailable-input evidence. New real A/B
must restore ordinary classification, history and evaluation after compaction,
optional faults or Gateway restart. Logical task completion is separate from
response delivery. These boundaries preserve availability without claiming scoring
accuracy for ambiguous callbacks or Qoder's shared main/child identity.
[Native listener cases](../../tools/e2e-harness/tests/p0_gateway_runtime/native_history.rs)
cover these boundaries; the managed producer hooks live in `assets/agents/`.
[history_bypass.rs](../../tools/e2e-harness/tests/p0_gateway_runtime/history_bypass.rs)
covers same-identity overlap across the supported native identity families, main
tool continuation, capacity exhaustion and observation faults through the listener.

## Decision operation

[core_runtime/classification.rs](src/core_runtime/classification.rs) owns the
local, built-in System One and custom REST classifier operation. Its clock starts before history
preparation in `core_runtime.rs`; preparation, Secret resolution, DNS, connection,
write and read share the bounded operation. The earlier source deadline and
source cancellation remain authoritative. A classifier failure can use the
published failure policy only while the source request remains active: heuristic rules for smart saving, or the configured default for custom branches.

[classification/protocol.rs](src/core_runtime/classification/protocol.rs)
serializes the five-field Decision API request from Replay-backed current input
and sealed execution history. It validates the returned branch against the
publication's allowed set and handles an optional prior-segment assessment
separately. It does not choose model candidates.

`TargetResolver` in
[runtime/driver/materialization.rs](src/runtime/driver/materialization.rs)
owns only DNS and the optional controlled dial mapping. `ProductionProvider`
captures it at construction; the classifier creates its own resolver at the same
point where it previously created a full business-model provider. The resolver
uses the caller's `ExecutionScope`, keeps the existing error categories, and
does not read credentials or runtime-state stores. E2E configuration remains
captured at construction, not reloaded on each resolution.

[core_runtime/credentials.rs](src/core_runtime/credentials.rs) binds current managed Native
Key IDs/order once at request admission. The control authority verifies the saved source,
model and target; exact attempt leases still enforce current generation and revocation.
Only the request-owned attempt view changes, leaving shared publication content immutable.
[publication/lease_target.rs](src/publication/lease_target.rs) derives the same selected
protocol target for admission and materialization; neither caller may substitute a destination.

[planner.rs](src/planner.rs) validates the selected category and chooses eligible
groups/candidates under the frozen policy. Availability relay may move execution
from regular to primary within that category. History and assessment retain the
selection and actual execution separately; neither is inferred from model names.

## Extension ownership

The existing [official Jev service](../../decision-extensions/extensions/jev-decider/README.md#code-and-responsibility-map)
separates deterministic request/answer logic from its Python HTTP lifecycle.
General inference, purpose policy, provider transport and execution authorization
have different owners: a model/tool selection purpose defines its question and
allowed IDs; a provider handles its wire protocol and credentials; only the
calling product authorizes actual execution. The current smart-saving policy is
not a universal inference or tool-routing contract.

The current compiler freezes category IDs and conditions, the default category,
regular/primary candidates, and effective degree and competence standards. The
[System One codec](src/core_runtime/classification/system_one.rs) owns the built-in
Choice/Score questions; it shares transport with the custom REST codec.
[Group policy](src/core_runtime/classification/group_policy.rs) decides each new
user turn from the current degree probability and a fresh, applicable preceding-stage
assessment. It keeps no persistent upgrade cursor. Tool continuations inherit the
same-turn decision while its history and authority remain valid. The existing
history store owns stage identity; there is no second session or execution loop. See the shared
[decision map](../../docs/code-map/decision-foundation.md) for service persistence,
provider references, observation and the future tool-selection boundary.

## Representative capability tests

| User capability or invariant | Existing entry |
| --- | --- |
| Built-in decisions assess the previous actual stage, protect the primary group when needed and decide again on each new user turn | [decision_branches.rs](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) covers both presets and observation |
| One decision selects the actual business model across supported ingress protocols | `real_listener_rest_classifier_*` in [p0_gateway_runtime.rs](../../tools/e2e-harness/tests/p0_gateway_runtime.rs) |
| Classifier timeout may fall back; source deadline or disconnect must not start a model | The three timeout/deadline/disconnect cases in the same target |
| Decision diagnostics use the production transport/parser without running a business model | `classifier_diagnostic_uses_the_production_transport_and_exact_protocol` in [classification.rs](src/core_runtime/classification.rs) |
| Accepted execution history survives client rewrite/compaction with observation disabled | [accepted_history.rs](../../tools/e2e-harness/tests/p0_gateway_runtime/accepted_history.rs) |
| Tool continuation survives restart with authentication | [continuation.rs](../../tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs) |
| Claude direct tool callers preserve native history and portable call/result pairing | `messages_direct_tool_caller_preserves_native_history_and_portable_tool_pairing` in [adapter tests](src/adapters/tests.rs); ingress rejects programmatic execution authority |
| Live publication cutover pins each request and preserves the last good version | [p0_gateway_request_authority.rs](../../tools/e2e-harness/tests/p0_gateway_request_authority.rs) |

Use the repository [test planner](../../scripts/test-plan.py) and
[configured validation runner](../../scripts/validation.py). An isolated Jev or
diagnostic success does not establish product routing success. Source-text
assertions and private helper names are not substitutes for these behaviors.

## Native controls and completed responses

[Message phase reconciliation](src/adapters/response/message_phase.rs) separates
provisional added-item metadata from authoritative item completion. Text/refusal
completion carries an optional final-phase envelope: missing preserves released
event semantics, while a present empty value clears the provisional phase.
[Client completion](src/adapters/response/client_message_phase.rs) uses the same
value for done-item rendering and retains cross-protocol capability checks.
Identity, accepted-content prefixes and sealed terminal metadata still reconcile
strictly. The [phase regressions](src/adapters/response/message_phase_tests.rs)
cover correction, late/missing metadata, native preservation and conflicting data.

[Reasoning serialization](src/adapters/request/reasoning.rs) owns protocol-level
switch semantics and normalizes supported released projections without rewriting
frozen publication identities. The daemon produces current profiles; explicit
effort, manual budget and adaptive controls remain distinct. Shared Pingora
transport assigns HTTP/2 authority once for model and classifier calls.

[Response classification](src/runtime/driver/response.rs) classifies upstream
failures for bounded relay within the frozen plan before the first downstream body.
Streaming headers alone leave relay open; a later failed model terminal stays a failure in attempt,
request and Agent turn observations even when its native bytes were delivered.
The decision diagnostic uses closed error codes; provider text is not a public
error message or a substitute for evidence of credential failure.

[Runtime health](src/runtime/state.rs) keeps the kernel's default shared Binding
backoff. The production Gateway explicitly selects request-local soft Binding
feedback for authenticated Agent model grants and verified Workers sharing the
same target keys, including unmarked native auxiliary requests. A transient
failure still relays and returns its original
normalized error within that request, but cannot cool the next independent main
call or invalidate its in-flight permit. An owned expired probe releases neutrally
with exact-generation CAS. Credential authentication/quota state remains shared.
Main calls, Worker calls and ordinary API calls using that grant consequently may retry an
unavailable source more often across requests. The [owning contract](../../issue-spec/specs/p0-gateway-execution-plane/spec.md)
and [listener regression](../../tools/e2e-harness/tests/p0_gateway_runtime/auxiliary_failure_isolation.rs)
cover both immediate recovery and concurrent main completion.

## Response failure evidence

The [Pingora response adapter](../gateway-core/src/transport/pingora/server.rs)
selects chunked framing for an unknown-length HTTP/1.1 body, including when the
client requests connection close. A post-commit deadline or failure closes the
incomplete entity; only an actual end-stream write emits its final chunk. This
lets native SDKs detect truncation instead of accepting ordinary close-delimited
EOF. Known lengths, body-forbidden responses, HTTP/1.0 and HTTP/2 retain their
protocol boundaries. Model-terminal proof and partial history remain independent
of HTTP entity completion. The [socket regressions](../gateway-core/tests/gateway_lifecycle/downstream_completion.rs)
and [full listener cases](../../tools/e2e-harness/tests/p0_gateway_commit_boundary.rs)
cover completion, deadline and disconnect delivery without another attempt.

### Candidate preparation errors

A published plan freezes candidate selection, while the exact credential authority
still checks current source and Key permission before decryption. A managed source
or Key denial retains its closed cause through the daemon adapter; an empty lease
means only that the exact credential is exhausted. Neither outcome starts an upstream
attempt or broadens the frozen fallback policy.

Temporary CPA runtime unavailability excludes only that subscription candidate and
allows the next saved candidate to run, including after a cold service start.
Rejected CPA capabilities and unavailable shared native credential authority retain
their fail-closed behavior. Skipped preparation creates no upstream attempt.

When preparation terminates with no upstream attempt, the core asks the existing
decision-session local-reply port for the product response. HiRoute returns HTTP 502
with `Content-Type: application/json`, `schema_version: hiroute.gateway.error/v1`,
and these closed fields:

| Preparation evidence | `code` | `phase` |
| --- | --- | --- |
| Every rejected candidate has a managed source disabled | `SOURCE_DISABLED` | `credential_lease` |
| Every rejected candidate has a managed Key disabled | `CREDENTIAL_DISABLED` | `credential_lease` |
| Every exact credential lease is exhausted | `CREDENTIAL_UNAVAILABLE` | `credential_lease` |
| Other or mixed preparation failures | `CANDIDATE_PREPARATION_FAILED` | `candidate_preparation` |

These replies have no `Retry-After`. Pure cooldown/probe exhaustion retains its
existing HTTP 503 runtime-state response and bounded retry hint. A failure after a
formal upstream attempt keeps the existing attempt response; this port cannot rewrite
committed bytes. No provider text, credential identity or authorization material is
included in these local replies.

Request-level `credential_lease` execution facts retain the denial as `source_disabled`
or `credential_disabled`; exhausted and authority errors remain distinct. Their
existing envelope is unchanged; the Domain ingestion enum accepts these two new values
alongside the original leased, exhausted and authority-error outcomes. With observation enabled and a
healthy sink, the request finishes as failed with zero started/finished attempts,
no accepted attempt and no fabricated usage, and can produce a zero-attempt receipt.
Follow the [live source/Key regression](../daemon/tests/support/model_connections_product.py)
for disable, restart, independent-source isolation and re-enable evidence.

`runtime/driver/response_diagnostics.rs` owns the closed, payload-free failure
classification; `adapters/response/decoder.rs` reports SSE event ordinal and
received-byte upper bound without per-token logs. An observation-side decoder
failure adds an optional `capture` context to the same diagnostic: actual protocol
pair and the last Responses event's closed kind, indexes and array counts. The
capture worker records only its first failure against the ordinal frozen at
materialization, with no IDs, body text, hashes or dynamic error strings. This
diagnostic preserves the original capture abort and cannot certify completion.
Native delivery separately emits `native_completion_unknown` when its terminal
proof is incomplete. The protocol projector retains only the first closed reason,
frame ordinal and event shape; the driver attributes it to the actual attempt.
This diagnostic carries no content or native identities and cannot change the
provider result, optional capture or downstream output. Native snapshot equality
uses complete canonical JSON values, independent of object key order; pure SSE
comments retain the model's terminal state across transport fragmentation.
The shared SSE framer also preserves a comment-only EOF tail without a final
separator; unfinished data/control fields still fail strict EOF, and a comment
does not supply missing model-terminal evidence.
See [transport and value regressions](src/adapters/response/native_transport_tests.rs).
Unix Debug-only
`runtime/stream_capture.rs` can capture an explicitly enabled private session;
its `stream_replay` example restores the actual attempt profile and tool mapping.
This is offline decoder evidence, not proof of network timing or task completion.
After successful provider materialization, private capture registers its existing
weak handle once a pending attempt is staged, including explicitly unauthenticated
profiles whose staging occurs after materialization rather than a credential read.
Private capture binds that request-owned weak handle at formal attempt promotion,
after validating the pending binding and credential. A prebody failure closes
body capture and seals the file before `stopped`; later promotion may replace
the sample atomically with an identical content prefix, one typed correlation
record and a new durable seal. A private temporary copy is synced before rename,
so termination always leaves either complete version; temporary bytes count toward
the session quota. If that quota cannot hold both copies, the original survives. It
rechecks the private directory, file identity/permissions/owner, expiry and
original file/session/record budgets. Content capture never reopens, and the
weak handle cannot retain a writer or bind another request or attempt.

The helper requires Unix Python with non-reaping `os.waitid`/`os.WNOWAIT`
(Linux, or Python 3.13+ on macOS). It checks support before creating a session.
From a clean checkout of the exact candidate, wrap the managed Debug daemon and
its isolated settings arguments with:

```sh
python3 scripts/private-stream-capture.py run \
  --root /absolute/new-private-session --source-sha FULL_CANDIDATE_SHA \
  --client isolated-client-version --seconds 300 --attempts 3 \
  -- /absolute/managed-debug-hirouted ISOLATED_DAEMON_ARGUMENTS
```

The helper issues no requests. Disable client retries, use a separate API-key
context, and stop on the first failure. Each private file is limited to 8 MiB,
the session to 32 MiB and four underlying attempts, and capture expires within
one hour. Credentials in transport headers are excluded. Never commit or upload
the files. Run the same candidate's Debug `stream_replay` executable with the
private `attempt-N.capture` path to check original, one-byte and 4096-byte chunks;
it prints only safe results. Incomplete or unsealed samples are rejected.
Replay supports successful cross-protocol decoder paths, skips informational
HTTP heads, and rejects native projector or non-success paths as
`unsupported_capture_path` without a decoder verdict.
A separate, bounded retention owner stops the isolated candidate's entire process
group, including descendants after the leader exits, and reaps the leader. It
deletes the raw session within 24 hours, even after the CLI returns. Its PID is printed;
SIGTERM also stops the candidate and deletes the session immediately. Keep the
owner running until deletion. Host shutdown or SIGKILL of that owner cannot run
cleanup; after recovery, delete the exact expired session with
`python3 scripts/private-stream-capture.py cleanup /absolute/new-private-session`.
Kernel locks release on candidate crashes, so stale lock files do not prevent
recovery cleanup. No shared scheduler or daily daemon is modified.

Known incomplete nonstream JSON responses follow the same prebody relay boundary
as streaming failures, for both native and converted protocols. Reported usage
survives fallback; partial response content is not delivered. Attempt/deadline
limits and fixed-source authority still apply, and unknown completion remains
separate from an explicit failure.

Successful converted nonstream responses retain their reported usage in the
provider completion facts before releasing the decoded response. Completion
publishes that usage before request termination, so its receipt does not depend
on canonical capture scheduling. Nonstream capture still records delivered
content but leaves usage to provider completion; converted streaming capture
continues publishing accepted usage events. Missing usage remains unknown.

### Current-pool admission failures

Current-pool admission checks the frozen model and target before reporting a managed
source denial. It keeps the request's members and order fixed, excludes denied sources
and records a request-level permission fact without a leased Key or generation. A
successful membership read does not create a lease fact.

If admission excludes every candidate, HTTP 502 uses `hiroute.gateway.error/v1`:

| Current-pool evidence | `code` | `phase` |
| --- | --- | --- |
| Every source is disabled | `SOURCE_DISABLED` | `credential_lease` |
| Every managed pool is empty/unavailable | `CREDENTIAL_UNAVAILABLE` | `credential_lease` |
| Mixed causes | `CANDIDATE_PREPARATION_FAILED` | `candidate_preparation` |

Admission exclusions also join the existing request-local preparation summary when
another candidate remains. Mixed permanent failures do not become a pure cooling
response, and an actual upstream attempt retains its existing response. These errors
contain no credentials or provider text and have no `Retry-After`.

## Bounded local text processing

The ingress serde reader uses an accounted 8 KiB buffer to amortize Replay reads;
syntax/end validation and the Replay terminal checks remain authoritative. The
sequential request encoder copies unchanged text runs and raw JSON in bulk from
its existing bounded content buffer, retaining scalar escaping only at special
bytes. It still respects write quantum, exact declared length and release rules.
[Boundary tests and the opt-in release benchmark](src/adapters/tests/local_work.rs)
exercise the same parser/encoder with memory and disk backing. Benchmark timings
cover those local operations; actual request summaries also include history,
observation capture and scheduling, and are not CPU-time measurements.
Canonical JSON range construction coalesces the existing serializer's small writes
through a fixed 8 KiB stack buffer. It seals a range only after a successful final
flush; partial writes, interruptions and errors retain the existing Replay failure
semantics. It adds no heap admission, retained body or alternative serializer.
Request-local spill seals completed writes with the existing handle/length checks,
without a durable flush. Independent readers retain their pinned handles; restart
cleans orphaned spill rather than recovering in-flight requests. Observation storage
and recoverable publication keep their own durability barriers.
