# Gateway request and decision boundaries

The production listener is composed in [server.rs](src/server.rs).
[publication/compiler.rs](src/publication/compiler.rs) turns one accepted
publication into immutable request authority. Read
[core_runtime.rs](src/core_runtime.rs) for the real request path: authenticate and
bind authority, prepare the canonical request and Replay, obtain a request-owned
branch decision, plan and freeze candidates, then admit execution into
`hiroute-gateway-core`. The core owns attempts, response commit, cancellation and
resource cleanup; the Gateway does not run a second execution loop.

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

The registered Codex subscription request dialect omits the redundant completed
reasoning-item status that its input parser rejects; public Responses paths keep
that field. [Native request projection](src/adapters/request/native.rs) applies
this normalization per candidate without changing the original request, reasoning
payload or other item metadata. Cross-protocol Responses output uses one shared,
response-scoped synthetic item identity across incremental and buffered renderers;
provider-native item IDs stay exact. The [request regression](src/adapters/tests/codex_replay.rs)
and [response lifecycle scenarios](src/adapters/response/client_stream_tests.rs)
cover endpoint isolation, replay and identity consistency.
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
Bounded plaintext Responses reasoning summaries are compared with the accepted
reasoning events, including empty-summary placeholders. Changed summaries, opaque
provider state and unsupported native fields lose the edge; reasoning stays out
of the competence-assessment transcript. Known native maintenance calls retain
their receipts and costs but carry an explicit optional auxiliary marker so the
timeline does not manufacture pending user turns. Older unmarked facts remain
visible under their original conservative interpretation.
Each admission owns a unique ticket and frozen assessment snapshot. ContextHold
commits are branch-scoped; optional cache misses cannot discard execution capture.
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

### Candidate preparation errors

A published plan freezes candidate selection, while the exact credential authority
still checks current source and Key permission before decryption. A managed source
or Key denial retains its closed cause through the daemon adapter; an empty lease
means only that the exact credential is exhausted. Neither outcome starts an upstream
attempt or broadens the frozen fallback policy.

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
