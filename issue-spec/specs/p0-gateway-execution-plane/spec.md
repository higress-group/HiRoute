# Gateway execution plane

## Purpose

The execution plane authenticates published model routes, projects native protocols,
runs bounded candidate/credential relay through Gateway Core, and reports independent
telemetry, execution facts, and captured content.

## Requirements

### Requirement: The data plane consumes six explicit ports

Gateway MUST be able to run without Application, business SQLite, or local Agent
configuration. Its execution inputs/outputs are RuntimePublicationFeed,
CredentialResolver, RuntimeStateStore, LifecycleTelemetrySink, ExecutionFactSink,
and ConversationContentSink. Boundary DTOs have schema/digest and serializable
identity; unsupported versions/malformed closure fail before upstream effects.
Real control-plane publication/import/Agent configuration remains a separate product
path and cannot be inferred from a standalone execution-plane fixture.

### Requirement: Current grants and exact plan versions are pinned separately

Authenticate before model resolution. Alias dispatch, `/v1/models`, and catalog
instruction overlays MUST agree within one authorized GatewayPublicationRevision
containing exact AgentPlanRevision and route handles. Plan-native reasoning wins over
caller effort. Unknown/unauthorized names do not fall back to an Agent default.
Fixed bindings and protocol-carrier rules follow [request routing](../request-routing/spec.md).

Connector-owned publications retain stable source/CPA binding and selected models,
not CPA token generation as an enduring grant key. Every Attempt resolves a current
capability with current auth, stable account, and latest successfully pinned account
directory. Disabled/deleted/mismatched/excluded sources fail before connect with
permitted relay or failure; unobserved upstream permission changes may still reach
provider and receive its final rejection. A later revoke never automatically replays
an already sent Attempt or expands source membership.

### Requirement: Cross-protocol projection preserves declared semantics

Responses, Messages, and Chat requests parse once into bounded ModelRequestIR,
requirements, and client response contract. All nine protocol pairs have their own
expressibility contract. Native same-protocol response uses its own output path;
other responses use bounded canonical response conversion. Request/response tools,
vision, ordering, streaming, usage, errors, refusal, and terminal status cannot be
silently dropped, textified, or guessed when required for the selected path.
Candidate-local preflight excludes only an unrepresentable path with zero upstream
calls, using explicit reasons rather than losing all legal candidates.

Cross-protocol instructions/message boundaries preserve actual order, including
Messages mixed text/tool-call/tool-result blocks and multiple results in one user
message. Chat tool messages and interleaved text/images remain in source order.
Messages `tool_result.is_error` stays failure content; ordinary cache hints may be
omitted only under their documented non-execution-hint projection. Model-generated
native argument strings and IDs follow their payload contract, not an arbitrary
local byte limit. Output cap, native reasoning, structured-format/options mapping,
and Fixed-versus-Plan ownership follow [request routing](../request-routing/spec.md).

### Requirement: Namespace/custom tool mapping is exact and reversible

Responses function/custom namespaces retain group names/optional descriptions,
child declarations, top-level tools/additional_tools merged order, strict/schema,
required choice, parallel behavior, and supported controls. Chat mapping constructs
one request-owned bidirectional table: flat names remain flat; namespace children
use `namespace__local`, with global uniqueness and ASCII `[A-Za-z0-9_-]` length 1–64.
Reverse lookup uses that table, never parsing the delimiter. Identical duplicate
declarations may collapse; conflicting identity/schema, target collisions, invalid/
overlong names, or nested namespaces fail that path.

Chat cannot preserve namespace group description, so such requests exclude Chat.
Freeform custom without format may use only an exact closed input-string wrapper
with no invented strict value, unpacked precisely on return. Nonempty grammar/format
or inexact wrapper cannot map. Messages has no namespace/custom support. Native
requests do not acquire this target-specific name constraint or conversion proof.
Tool choice none/auto/required_any/required_named stays distinct; named choice retains
function/custom kind and name. Gateway typed input rejects missing/unknown kind or
extra fields. Product canonical Function omits default tool_kind while Custom writes
custom, preserving released Function fact/receipt digests without loosening input.

### Requirement: Historical tools and reasoning do not require delivery registries

Ordinary native tool IDs retain exact values and call/result pairing through process
restart, memory expiry, and candidate changes. Conversion changes an ID only for a
proven target format and rejects collisions. Past delivery/memory membership/current
candidate cannot grant or deny future request authority. Current grant/route/credential
checks are independent. Output delivery counters reflect actual accepted frames only.

Native Responses history preserves message.phase, supported turn metadata, reasoning
fields/summary, nonempty encrypted_content, and legitimate completed item states;
no process-local reasoning-owner registration is required. Function-call-output
completed/incomplete/in_progress/absent maps to completed/failed/unknown/unknown for
judgment history, never by inspecting output text. Other supported history items
accept only their legal lifecycle values. Empty native generated-call names use the
narrow native-only exception in the request contract; declarations remain strict.

Known Responses `agent_message` history preserves its author, recipient, ordered
plaintext/encrypted content and native metadata on same-protocol requests. Its peer
role has no declared portable mapping: cross-protocol candidates are excluded.
Known visible messages may support observed-input classification and actual
execution history, with explicit missing-origin evidence and no provider-cache
prefix claim. The copied ancestor User is observed request content, not proof of
the current peer task; its association accuracy remains a disclosed limitation.
Origin uncertainty alone SHALL NOT permanently disable subsequent classification,
capture or cross-turn evaluation. Peer text is never fabricated as a human User
or accepted assistant output; encryption is not reinterpreted as reasoning state.
Malformed or unknown content variants retain their protocol errors.

Thinking signature/redacted/encrypted values are payload, not interchangeable
credentials. Same-protocol preserves raw values, empty versus absent, legal JSON
encoding, and ordering. Cross-protocol can omit structurally unexpressible reasoning,
never masquerade signature as encrypted_content. Implemented legal plaintext mappings
remain; omission emits safe WARN only when it actually changes projection. Preflight
and unchanged projection emit none, and ordinary omission creates no cleanup retry
or persistent watermark.

### Requirement: Decision history has one bounded process-local owner

One ContextHold scan returns message continuity and stronger candidate preference.
Before pure Planner, smart-saving/custom judgment inherits or starts exactly one
routing turn using that result; history store cannot independently hash prefixes,
compare latest text, or identify compaction. New user input always rejudges; tool
continuation with valid continuous decision does not. Instruction/tool/state changes
may invalidate preference without changing that turn's category/group. Unrelated
publication does not invalidate exact route/authority continuity; every request
still uses current authorization.

Routing continuity keeps a separate streaming digest that masks only text within
same-position System/Developer messages. Roles, names, content-part counts,
non-text parts, other messages and the measured search/annotation/reasoning
sidecars remain exact. Changed native
instruction text may preserve captured semantic history, but strict byte-prefix
and instruction digests still govern preference, commit and reasoning-cleanup
eligibility. Added/removed instructions or rewritten user/model/tool content do
not gain continuity from this exception.
Strict execution fingerprints always measure the original canonical request,
including native user-message partitioning. A borrowed routing-only view may
supply the semantic digest at identical message positions; its normalized batches
cannot authorize provider affinity or reasoning-cleanup reuse.

The history store defaults to 500,000,000 bytes and one-hour idle TTL, retaining
simplified sealed turns, one active decision, and actual contiguous execution stages.
It does not read Observation disk/SQLite or add another branch/cache state machine.
Concurrent same-session/late CAS rejects unsafe updates. LRU/TTL/restart yields
explicit partial history without blocking independent requests.

History is optional routing context, not model-request admission. A same-scope
request already in flight, unavailable continuation, exhausted/busy history store,
or request without a real user turn SHALL execute independently with empty visible
history, `history_partial=true`, and no inherited decision or prior-stage assessment.
ContextHold scans SHALL NOT advance shared checkpoints before history admission;
rejected side-path admission cannot invalidate the live request's completion or
tool continuation. A stale/failed optional decision write relinquishes its history
and hold write authority while retaining the request's validated routing decision.
Such failures SHALL NOT become `SESSION_TURN_CONFLICT`, `TURN_CONTEXT_UNAVAILABLE`,
or history-capacity HTTP errors. Optional judgment projection/transport resource
failure uses the published decision fallback. Current authorization, canonical
request/Replay integrity, source cancellation/deadline, and execution eligibility
remain mandatory. Observation/capture failure cannot replace, truncate, retry, or
delay delivery of the accepted model response; any lost history remains partial.

Native envelope roles alone do not prove a user turn. DSH appends runtime context
using the user role, so its installed adapter identifies native `source.kind=user`
events and turn IDs, then binds them to serialized request user positions with
SHA-256 over each ordered text-block array. Gateway verifies that optional proof
before building a routing-only semantic view. Synthetic context cannot open a turn;
same-turn user batches merge at their final original position without moving model
or tool exchanges. Provider execution and Observation retain the original request.
The adapter reads at most 512 events; proof is bounded to 4 KiB, 16 turns and 32
real user messages among 128 user envelopes. Gateway limits the compact projection
to 256 KiB and user text to 2 MiB, reserving its encoded, decoded and temporary
storage before allocation. Missing/invalid proof, unknown/image content, compaction
without a surviving true user, or insufficient projection capacity SHALL disable
shared history and classify as unavailable using the published fallback. It SHALL
NOT classify a trailing runtime instruction as a user task or fail model execution.

A later proven native request may reveal user turns that bypassed optional capture.
It may retain already accepted visible history, but SHALL mark the gap partial and
discard the previous assessment segment instead of inventing missed executions.

#### Scenario: Native auxiliary request overlaps the main tool turn

- **WHEN** two authorized requests share one native session identity while the first model response is still in flight
- **THEN** the second is classified and executed independently without modifying the first history checkpoint, and both can deliver their complete responses.
- **AND** the first request's subsequent valid tool continuation can retain its decision.

#### Scenario: Optional state or observation is unavailable

- **WHEN** bounded history cannot admit a request, a history write is stale, or observation/capture is full, slow, failing or panicking
- **THEN** the authorized model request still completes through normal routing, optional writes cannot overwrite another request, and unavailable context is disclosed without inventing an assessment target.
- **AND** an unauthorized request still cannot invoke the classifier or model.

Decision preparation, narrow header Secret lease, Replay/history, DNS/connect/write/
read share the earlier of classifier timeout and source absolute deadline/cancel.
One call supplies selection/optional prior-stage assessment under
[decision policy](../decision-services/spec.md). Observation persistence cannot
change the frozen decision. Planner is deterministic from materialized facts and
hard cost/protocol/source gates; no database/network/Secret/history/global mutation
or runtime score/price ranking is allowed.

### Requirement: Additional reasoning cleanup is one narrowly authorized retry

Only when the same-scope last complete successful candidate A exists, history prefix
is discontinuous, and frozen first choice B differs from A may B's explicit reasoning-
history rejection authorize one precommit omit_reasoning retry. It removes recognized
reasoning from the current original canonical prefix while preserving text/tools/
results/IDs. No changed projection or remaining budget means no send. The whole
request has one extra cleanup opportunity, sharing original deadline/max_attempts;
subsequent candidates cannot inherit it. Other errors use ordinary pre-body relay.
Continuous follow-up switch, tools, instruction-only change, no prior success, or
selection of A cannot create this extra privilege.

After complete cleanup success, existing success CAS stores the original prefix
length. Later same Binding/profile continuous history cleans only that prefix,
retaining new state and never enlarging the watermark. Tool/instruction changes may
invalidate strong preference without deleting a valid watermark. Failure/cancel/
partial/stale completion cannot commit it. Discontinuity/scope/profile/candidate
change, restart, or eviction removes it without a durable recovery registry.
Responses conversation handles remain separately unsupported; reasoning payload
cannot prove a stored handle's owner.

### Requirement: Health transitions and relay remain exact CAS operations

After frozen candidate ordering, RuntimeStateStore checks exact Binding/credential
health and lease. Key order is fill-first; Key failure relays before another model.
Credential 401/403 disables the exact credential, quota 429 cools its exact Key,
including the existing Binding fallback when no credential exists. These authority
failures remain shared across requests. Recognized overload/5xx/timeout/disconnect
are soft Binding failures. The default runtime kernel records their shared health;
all production Gateway model execution (authenticated Agent model grants and
verified delegated Workers) keeps that feedback within the current request. Their
Binding health keys do not include a grant/run dimension. A grant also carries
native auxiliary calls without portable purpose/session markers, so this policy
does not guess Main/Aux/Unknown from a UA, payload or Agent-specific header. Real
main calls, verified Worker calls and ordinary API calls using the grant also lose
cross-request soft-failure suppression and may try an unavailable source more
often. Frozen candidate order, key relay, Attempt limits, original
failure responses and classification remain unchanged within each request.
Ambiguous errors do not invent isolation. Attempt/deadline/cancel budgets never
reset on relay, and committed output cannot be replayed.

For shared Binding transient feedback without trusted waiting guidance, backoff
steps are 2, 4, 8, 16, 30 seconds, saturating at step five. Trusted Retry-After/reset is used
as given beyond the local cap and resets step zero. Recognized overload without
hint defaults to 15 seconds/reset zero; credential quota defaults to 60 seconds,
auth disables it, both with step zero. Successful recovery probe resets Binding
step. Health/generation/step/lease update in the same CAS; acquisition/cancel cannot
drop step and a stale CAS cannot repeatedly increment one observed failure.
Current RuntimeState requires the step field without an alternate old-state reader.
For an active Binding, request-local soft feedback does not change shared
health/generation/backoff and cannot invalidate a concurrently executing main permit.
If the failed Attempt owns an expired Binding or credential cooldown's probe, it
releases only that exact generation's lease with CAS while preserving the preceding
health/expiry/backoff step; it neither
confirms recovery nor creates a new cooldown. A stale release cannot affect a newer
probe, and required store operations still fail closed.
Pure temporary pre-Attempt exhaustion exposes accurate retry time under the
[request contract](../request-routing/spec.md); store failure never means healthy.

#### Scenario: Auxiliary failure cannot suppress an independent main call

- **WHEN** an authenticated Agent model grant's unmarked title request receives a
  soft 503 from a source, even with trusted Retry-After
- **THEN** its original normalized failure returns to that caller, while one
  immediately following main HTTP request can reach the same source without client
  retry or waiting for shared cooldown; an already in-flight main can still commit
- **AND** credential authentication/quota failures retain exact shared authority
  transitions; verified Worker model execution uses the same isolation, while the
  default kernel/provider retains its shared Binding backoff contract

### Requirement: Replay externalizes large content without a second body owner

Request-local ReplayStore is the raw/semantic backing for later Attempts. Current
production threshold is 1 MiB; explicit environment configuration may override it.
This is an allocation/spill choice, not a protocol message-size cap. Above threshold,
private newly created plaintext spill is bounded by request/daemon resource ownership
and read sequentially through pinned handles. System-temp aliases are canonicalized;
explicit/replay leaf symlinks remain rejected, and ordinary OS-access policy is not
an owner/mode repair operation.

Replay cannot encrypt/authenticate/hash/fingerprint the entire raw body, pre-scan it
again before provider solely for integrity, or seal-copy a second full representation.
Existing syntax/depth/structure checks remain. ContentRef contains request-local
locator/length, not a digest. Reads reject range violations, early EOF, or whole
backing length mismatch; equal-length same-OS-user modification is not a cryptographic
integrity guarantee. ContextHold alone may stream HMAC relevant externalized fields
through the existing reader for continuity without another cache; failed reads
create no hold.

Large strings, including images/tool arguments/reasoning/signatures/metadata/formats,
externalize without truncation. An inline threshold, queue size, chunk size, or
retained allocation threshold cannot act as a hidden field/message/protocol cap.
Actual memory/disk/depth, target-specific name/shape, identity, and semantic bounds
still apply. Control objects needed by reasoning/output handling remain structurally
readable after externalization. Termination/cancel/deadline/orphan maintenance removes
owned spill, and IR/wire/scratch/config leases release at their last consumer under
[Core ownership](../gateway-core-foundation/spec.md).

### Requirement: Native response output and evidence extraction are independent

Actual selected same-protocol Attempt MUST use native output, not reconstruct it
from ModelResponseIR/EventIR. Preserve legal native SSE event/data order, extensions,
JSON values, snapshots, final markers, signature/state, usage, and refusal. Only
explicit owned model/ID paths may be rewritten; unknown same-named nested keys are
not recursively patched. Equal transport chunking/JSON whitespace is not required.
Same-protocol response passthrough does not loosen request/auth admission.

Incremental text/reasoning/tool/control/unknown events are forwarded before terminal
classification, complete arguments, IR capture, or storage. Bounded framing/projection/
backpressure is allowed; retaining a whole stream as retry prefix is not. Mandatory
owned identity/pairing/lifecycle/framing/resource contradictions fail at actual commit
state. Unsupported side-channel parsing/queue/storage degrades evidence to unknown/
incomplete/dropped without blocking native output. Missing usage is never zero.
Non-owned semantic/terminal disagreement preserves legal output where safe but cannot
prove complete success or update ContextHold.

### Requirement: Terminal evidence cannot be replaced by HTTP success or EOF

Native normal/refusal/truncation/tool/error/unknown outcomes retain their protocol
meaning. HTTP 200, any text, or connection EOF is insufficient proof. Delta/completed
item/terminal consistency is checked for owned identities; extra Chat choices,
duplicate ambiguous stop, or unexplained snapshot disagreements produce conservative
unknown rather than invented single-choice/tool success. Responses reasoning-text
may be proven against matching final summary only when item/position/all delta/done
text agree and content is absent/null; explicit content/conflict/missing summary
cannot be hidden by that rule.

One optional `[DONE]` after the unique Responses terminal is retained and authenticated
once. When terminal verification needs EOF, buffer terminal/tail boundedly and deliver
it with downstream EOS only after verification, preventing normal client exit from
becoming false cancellation. Earlier increments remain prompt. Cross-protocol Chat/
Messages complete terminal converts to Responses completed; length/refusal-related
incomplete states keep response.incomplete/details. A proven converted terminator is
delivered with downstream EOS without waiting for connection-tail cleanup; malformed
same-chunk tail and known prior usage retain their actual evidence.

Known incomplete nonstream termination before body may relay under the same bounded
failure policy, keeping first-attempt usage and not exposing partial JSON. Unknown
completion evidence is not itself an explicit failure. Successful upstream headers
cannot alone close pre-body relay. Any first body write closes transparent retry.
After proven delivered terminal, pure tail/disconnect/cleanup failure remains separate
from business success and cannot replay; unsafe connections close, and hold additionally
requires clean cleanup. Cancel before delivered terminal records RequestFinished
cancelled with postcommit_cancelled Attempt and actual usage, not generic transport
failure or success.

### Requirement: Three observation channels have independent completeness

Lifecycle is zero-body telemetry; Fact carries route/candidate/Attempt/usage/cache/
price/failure/commit; Content streams begin/append/finish/abort for visible canonical
request and complete downstream-accepted response/tool units. Fact and Content have
independent identity, producer epoch, sequence, ACK/NACK, and loss/gap watermark.
Any sink failure leaves model response semantics unchanged and cannot cause another
channel to claim complete evidence. Content append requires actual downstream
acceptance; a coalesced chunk partially delivered captures only its accepted units.

A successful zero-byte end_stream is a real ResponseFrameAccepted with zero count
and valid identity, not a sequence gap. Operational acceptance, model semantics,
usage, and request outcome remain distinct. Known converted nonstream usage is
published from completion before termination independently of capture scheduling;
nonstream capture does not duplicate it. Default OTel is zero-body small attributes/
references; explicit content policy is separate and secrets are never exportable.
Each valid RequestFinished closes its [immutable request receipt](../session-history/spec.md)
without waiting for AgentTurnFinished.

### Requirement: Listening configuration cannot expose local management

CLI/Linux and Desktop/macOS share persisted Gateway bind IP/port and explicit restart.
Default is loopback; a user may choose a local interface or 0.0.0.0. Automatic first
port selection persists, and a fixed port failure cannot silently choose another.
Desired config, actual listener, readiness, and Agent connection address are separate.
Binding wildcard is not a usable client destination. Address changes retain data/
route identity but require explicit stale-client reconfiguration; existing files
are not silently rewritten. Remote model ingress retains auth; Local Control,
protected grants, and CPA management stay local.

### Requirement: Readiness identity is supplied only by a trusted launcher

hirouted starts directly without reading/hashing current_exe to invent readiness
identity. An external launcher MAY provide already verified exact
`sha256:<64 lowercase hex>`; process validates format and publishes unchanged.
Absent identity is null, not an extra launcher requirement. Invalid format fails
before listener/readiness. Names alone cannot equate builds or product roles.

### Requirement: Authentication recovery is distinct from business relay

CPA-managed one business Attempt may contain active refresh and at most one same-
account/same-model 401 auth retry before downstream commitment, under original
deadline/cancel/scope. NativeBorrowed supplies no refresh token; HiRoute cannot add
another refresh retry or interpret this as permission to replay a revoked Attempt.
Inference HTTP subrequests and business Attempt are separately observable; OAuth
exchange is not model tokens and a 401 cannot invent usage.

### Requirement: Execution-plane claims require real listener and resource evidence

Typed E2E corpus, request/SSE goldens, native semantic ledger, fault injection, and
sensitive-data checks MUST exercise real ingress/auth/planner/adapter/Core/sinks via
shared Responses, Messages, and Chat emulators. Fixtures/internal seams/expected-red
can prove contracts but cannot claim real native client/control-plane completion.
Protocol acceptance includes nine pairs, arbitrary fragmentation, tools/vision/
reasoning/terminal errors, Key relay, cancellation, and sink degradation. Removing
an essential step or replacing real evidence with a fixture must break its assertion.

Release/optimized replay comparisons record exact baseline/candidate, payload,
spill threshold, iterations/environment/raw results. Readiness is checked while
three provider-backpressured Replay readers are independently confirmed active;
RSS sampling starts before requests and includes a sample before releasing the
provider body barrier. Timing outside that interval cannot prove concurrency memory/
responsiveness. These records establish only the measured revision/platform, not a
blanket unmeasured performance/native/Windows result.

## Implementation references

- [Gateway production guide](../../../crates/gateway/README.md),
  [publication compiler](../../../crates/gateway/src/publication/compiler.rs), and
  [credential/preparation failures](../../../crates/gateway/src/runtime/driver/selection/preparation.rs).
- [Replay implementation/default regression](../../../crates/gateway/src/replay/tests.rs) and
  [native response evidence](../../../crates/gateway/src/adapters/response/native_passthrough_evidence.rs).
- [Real runtime listener corpus](../../../tools/e2e-harness/tests/p0_gateway_runtime.rs),
  [protocol corpus](../../../tools/e2e-harness/tests/p0_gateway_protocol/production_matrix.rs), and
  [strict product fact projection](../../../crates/integrations/src/gateway/tests.rs).
