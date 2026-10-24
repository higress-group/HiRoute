# Gateway Core execution and resource contract

## Purpose

Gateway Core owns filter scopes, one-attempt HTTP exchange, transport commitment,
configuration leases, body/SSE ownership, and bounded execution. The product compiler,
protocol adapter, and candidate manager retain their separate responsibilities.

## Requirements

### Requirement: The Core driver owns an exchange, not candidate policy

Core MUST use pingora-core server/HTTP/connection primitives with its own driver,
not ProxyHttp as filter-state-machine owner. The compiler produces request/attempt/
accepted-response plans; protocol adapter materializes PreparedAttemptHttpRequest;
Core instantiates the compiled Attempt filters and executes at most one semantic
upstream HTTP call, returning an uncommitted response. Only candidate manager selects
a candidate or creates the next RouteAttempt. Implicit proxy retry buffering is
not the native LLM replay owner.

Driver lifecycle and caller disposition MUST distinguish selecting, active Attempt,
uncommitted result, published terminal, and finalized states. A failed response cannot
reach final downstream encoding before explicit disposition. Bootstrap/plain-HTTP
fixtures construct compiled IR directly for integration, never a second product schema.

### Requirement: Requests own segmented pinned plans rather than publication roots

At ingress, acquire PlanRevision and independently owned ingress/attempt/accepted-
response/config handles, then release the active publication root. A long SSE cannot
retain unrelated routes/providers or a whole old arena. ResolvedTargetBindingId is
valid only inside its PlanRevision; unknown/cross-version binding fails before DNS,
credential resolution, or connect.

Structure/order/targets are RequestPinned. Compiler declares config binding as
ConnectionPinned, RequestPinned, AttemptPinned, PhasePinned, or EventLive with
compatibility hash and atomicity group. EventLive applies only to stateless compatible
values; its guard cannot survive a callback/await or long-lived context. Stateful
filters materialize bounded compact stream state from a PhasePinned generation and
release larger leases. Atomic related fields appear in one generation, never torn
new/old bundles. Actual plan/config generations are attributed per request/Attempt.

Connection reuse depends on stable target/transport compatibility and declared pool
connection epochs, not ordinary publication revision. Compatible unrelated config
updates cannot unnecessarily drain H2. Equivalent resolved target socket identity
cannot split pools solely because DNS answer ordering changed.

### Requirement: Logical, Attempt, and accepted-response scopes are isolated

LogicalRequest scope is created once per logical request. Every RouteAttempt has
its own filter scope/context/pause/error/finalize. At most one AcceptedResponseScope
is created only after final response selection, from a LogicalRequest terminal
local reply or candidate manager's published Accept/Terminate. Attempt Continue
never enters accepted encoder and never reruns logical-request filters.

Compiled encoder order is reversed. Stream callback counts track actual frames,
not one callback per full body. Scope finalization is exactly once across success,
local reply, error, cancellation, and failed construction. Terminal accepted/local
scope cannot return to candidate selection even if headers were not yet written.

### Requirement: Pause and resume retain exact filter/frame position

Decoder and accepted encoder MUST support Continue and StopIteration with bounded
retention/watermark behavior, accurate continuation position, and one-shot resume.
The stopped filter may resume from body/EOS/asynchronous completion; later filters
see each held frame exactly once. Header-stop crossings and nested pauses preserve
both header and body cursors rather than skip/repeat callbacks. Stale, wrong-scope,
wrong-epoch, duplicate, or finalized continuation fails closed.

Authorized body callbacks may change held request headers only before upstream
request-header commit, and held response headers only before final downstream
header commit where the direction's body plan permits buffering/prefix gating.
After resume, driver recomputes framing from final headers/body. Full BufferedTransform
may set exact Content-Length; a prefix/readiness gate cannot prove final length and
uses H1 chunked/H2 streaming with no invented length. Headers already committed cannot
be replaced with a second status/header block.

For an HTTP/1.1 response whose body is allowed and has neither Content-Length nor
Transfer-Encoding, the transport MUST select chunked framing even when the body
plan is pass-through or the connection will close. Only an actual end_stream
write may complete that entity. A failure after response commit MUST leave the
entity incomplete; normal connection EOF cannot stand in for successful delivery.
Known lengths and existing framing remain authoritative. HEAD, body-forbidden
statuses and successful CONNECT responses do not gain a chunked body. HTTP/2
retains stream reset semantics, and HTTP/1.0 retains close-delimited compatibility;
its callers still require model-terminal evidence to establish semantic completion.

### Requirement: Body modes and budgets are scope- and direction-specific

Request, Attempt request, Attempt precommit response, and accepted-response body
plans MUST be independent. Each declares streaming/no aggregation, StreamingReplay,
BufferedTransform, or SseFramedStreaming with explicit limits. Replay retains bounded
framework bytes; semantic cross-provider replay materializes from Model IR instead
of treating the first wire representation as universal source.

Known Content-Length is checked early and actual cumulative unknown-length bytes
are checked incrementally. A data chunk without end_stream is not EOS. Full
transform finishes before the relevant header commit and recomputes length or
selects protocol-correct unknown-length framing. No full-body filter requirement
means a streaming path, not default aggregation.

Every allocation has one owner and consuming transfer. raw_request, model_ir_backing,
attempt_wire, and transport_inflight each have at most one framework complete
representation; controlled overlap is charged to the budget/peak. Borrowing IR via
RequestBodyLease makes lifetime and accounting explicit. Last raw consumer/parse
transfer releases raw scratch; wire source consumption/cancel releases application
wire; send/reset/teardown releases transport bytes. Terminal decision/request end
plus writer-quiescence/last-lease-drop releases Model IR before long response EOS.
Large reusable buffers above threshold are dropped, not merely cleared while pinning
capacity. Logical slice size cannot hide much larger retained backing.

### Requirement: Early Accept preserves the same response while writer quiesces

Early upstream responses may arrive before request EOS. Continue, Terminate, cancel,
deadline, or local terminal not using that response MUST cancel/reset and boundedly
join the writer, escalating to stream reset/connection teardown when necessary.
No next Attempt materializes until the prior destructive writer path is quiescent.

Accept MUST stay unpublished/downstream-precommit until normal request EOS/quiescence,
or an explicitly declared and actually verified H1/H2 response-preserving half-close,
while keeping the original response handle readable. It cannot reset/teardown that
stream to make writer cleanup appear complete. If capability is absent or the gate
fails/times out, reject Accept and return to the candidate owner for Continue/Terminate.
Only after destructive disposition is chosen may the response be discarded/reset.
Pending prefix/SSE remains bounded and invisible until gate passes; drain it exactly
once into that same continuing response. Capability/gate failure is not model success.

#### Scenario: Valid SSE arrives while H2 upload is blocked

- **WHEN** response is eligible for Accept before upload EOS
- **THEN** Core MUST prove writer quiescence without destroying the same response
  before publishing Accept; otherwise it returns a blocked result for new disposition.

### Requirement: Local replies honor scope and zero upstream semantic writes

Before upstream request-header commitment, local reply stops that scope's request
filters and guarantees zero upstream HTTP header/body semantic bytes, though an
unused preconnection may exist. LogicalRequest reply becomes TerminalLocalResponse
and bypasses candidate manager into the one final response scope. Attempt-local reply
remains uncommitted, is normalized by adapter, and receives Accept/Continue/Terminate
from candidate manager. Accepted-scope replacement cannot resume candidate selection.
After upstream commit, a failure cannot claim a zero-side-effect local rejection.
All local/terminal paths finalize and release owned resources once.

### Requirement: Three commitment fences are advanced by actual transport

Core MUST distinguish upstream_attempt_request_headers_committed,
downstream_final_response_headers_committed, and client_semantic_output_committed.
The first two reflect irreversible codec/transport writes. Model encoded units
carry framework SemanticProvenance; LogicalRequest local replies use framework/local
NonSemantic units without requiring model encoder/provenance.

Accepted filters conservatively propagate provenance. Drop strips it; transformed/
combined output cannot acquire unsupported provenance. The semantic fence advances
only when final provenance-bearing bytes are irreversibly accepted downstream, not
on decoder/encoder entry. Pending tool continuation association is activated only
after a complete output frame containing its trusted logical call identity is
accepted; discard/reset/Continue/uncommitted termination revokes pending association.

Core candidate retry requires Continue, no AcceptedResponseScope, clear downstream
fences, replayable IR, and remaining deadline/attempt budget. Published Accept/
Terminate never goes back to Selecting merely because no semantic bytes were written.
Product precommit selection must finish before that publication boundary.

### Requirement: SSE framing is provider-neutral, incremental, and owned explicitly

SseFramer MUST split arbitrary transport chunks into complete events preserving
order, CR/LF/CRLF, multi-line data, event/id/retry/comment, and declared EOS behavior.
Adapters cannot treat raw chunks as complete SSE/JSON. Synchronous visitor events
are callback-lifetime views: they cannot enter context, cross await, or be given to
an asynchronous writer. Explicit OwnedSseEvent/encoded units charge retained capacity.
Unmodified native forwarding may transfer Bytes with output/transport accounting.

At EOF, a remaining tail containing only comments and empty lines has no pending
event description, even without a final line ending or empty separator. The shared
framer MUST deliver its exact bytes through the normal bounded visitor path,
including NeedDrain retries with the same charged owner. This rule supplies no
model-terminal evidence. Strict EOF MUST still reject unfinished data, event, id,
retry and unknown control fields, including fields mixed with comments.

Complete single-buffer events should avoid payload copies. Fragmented input retains
only needed tail; small retained views must copy out or charge the full backing.
Byte-by-byte input remains O(n). max_event_bytes applies to complete single-chunk
and unfinished fragmented events, alongside pending/output/expansion/per-stream limits.
Transform errors/overflow fail the Attempt, never silently discard/resynchronize.
Borrowed views release at callback exit; owned events release after consume/write/reset.
Bounded queues and upstream flow control apply backpressure without lost chunks or
unbounded accumulation. Long streams do not retain historical output.
Provider JSON, usage, readiness, and error meaning belong to adapters.

### Requirement: Connection sub-attempts stop at the first semantic write

A TransportTarget may try another authorized address only before any HTTP request
header/body semantic byte is written. After write, return that exchange's outcome
rather than implicitly replay to another address. Host/port/path/query and TLS target
identity are preserved by materialization. TLS policy is explicit; missing trust/
configuration fails instead of downgrade or unapproved target. Connection attempts,
retry scope, and failure remain separate from product candidate Attempts.

### Requirement: Structured cancellation and executors preserve latency meaning

Every sidecall and pump belongs to the request's structured scope, inheriting absolute
deadline/cancel/shutdown. No detached work may survive request authority. Filter
logic may use sequential async/await without blocking the I/O runtime. Network/body/
HTTP/Redis sidecalls use nonblocking I/O; CPU-heavy and blocking/control work use
separate bounded observable rejecting executors. Running/queue/wait limits shed load
before heavy work; ordinary fast path cannot require an executor hop.

Record sidecall I/O completion separately from request resumption, distinguishing
backend latency from scheduler_resume_lag. Data crossing await/thread is owned/Send;
socket/session/config guards, synchronous locks, and SSE borrowed views cannot cross
those boundaries. Cancel/deadline while paused unwinds filter continuations, writer,
connection/body/output children and blocks new connects. A native callback panic
becomes current scope/stream failure, not process/worker failure.

Request cancellation MUST remain a typed result across the core-to-product
transport boundary, including before any upstream Attempt starts or is accepted.
Ordinary preparation failures remain failures. A final cancellation-token state
or an error message is not proof of the termination cause: deadlines and cleanup
also cancel child work. A late cancellation SHALL NOT rewrite an already completed
delivery. Product request/stage outcomes consume this typed cause independently
of accepted-execution membership and capture completeness; no Attempt or missing
capture may be invented to represent a cancelled request.

### Requirement: Publication installation is independent of configuration sources

Core accepts a compiled envelope through source-neutral PublicationInstaller,
without SQLite/file/HTTP/MQ/watch client dependencies in runtime public types.
Envelope binds authority ID/epoch, authority-local opaque ConfigRevision, PlanRevision,
schema/compiler versions, payload digest, and separate plan/config handles.

Same epoch/revision/digest is idempotent. Same revision/different digest, stale epoch,
unauthorized rollback, mixed plan/config closure, or incompatible schema/compiler
fails. Revision gaps request authoritative full-state resync rather than silently
merging authorities or local last-good with remote last-write-wins. Explicit full
snapshots/rollback/authority switch follow their validated envelope policy.

Candidate prepare/publish is serialized, bounded, cancellable, and all-or-nothing,
preserving last-good on failure. Poll/watch/MQ discovery, auth/fetch/compile and
reconciliation belong to source adapter/control executor. Notifications are hints;
periodic full reconciliation repairs loss/duplicates/out-of-order results. Fast updates
may coalesce boundedly but cannot partly install state. I/O workers do constant-time
active-handle reads/switches and required connection-epoch actions, not source I/O.

### Requirement: Telemetry accounts for real lifetimes without sensitive payload

Structured telemetry MUST correlate revision/binding/stable target, request/decision/
Attempt, scope/phase, pause, body mode, queue/retained peaks, fences, connection trials,
TTFB, cancellation/deadline, writer join/reset, and closed failure. Memory categories
include raw, IR, wire, replay, prefix, SSE, semantic, output queue, and transport bytes,
with live/peak retained capacity and release latency from their actual last consumer.
Early disposition candidate, close mode, gate wait/result, response preservation,
reset/teardown, and publication time are separate. Backend completion/resume latency
and budget rejection remain visible. Labels are bounded low-cardinality; prompt/
response/credential/Authorization/tool payload and arbitrary external strings do
not enter operational logs/metrics. Observation consumes safe facts independently.

### Requirement: Acceptance evidence is specific to revision, platform, and claim

Affected formatting, warnings-denied Clippy, tests, and explicit production gates
follow [test selection](../../../docs/test-selection.md) and
[validation routing](../../../docs/validation-routing.md). Hosted backend runs Linux;
configured macOS Desktop/Pilot and actual Windows coverage are recorded separately.
No build/process exit/fixture can stand for missing native/HTTP/half-close evidence.
Real socket tests must cover pause/resume, framing, scope-local replies, SSE/backpressure,
commit/cancel/fence, single exchange, and snapshot binding. Half-close claims need
per-adapter/protocol proof that the same response survives after writer quiescence.

Fast-path performance claims require the same minimal pingora transport baseline,
hardware/OS/toolchain/release flags, H1/H2, reuse/payload/concurrency/warmup/duration,
and at least five independent interleaved measurements. Report each and aggregate
throughput, p50/p99, variation/confidence, and exact revisions. The retained acceptance
criterion is at most 5% aggregate throughput loss and p99 increase. This criterion
is not evidence it was measured/passed. Buffered/readiness paths report extra first
byte and actual retained/inflight peak independently.

SSE measurements cover tiny-to-limit events, fragmented/coalesced/byte input,
1:1/1:N/drop/replace, allocation/copy/scanning linearity, callback ownership,
copy-on-retain, slow downstream, and stable long-stream memory. Large upload/early
Accept/Continue/long-SSE cases verify bounded writer convergence, zero Accept-side
reset/teardown, same-response continuity, and prompt-sized memory release before
response end. Long keepalive reuse drops exceptional backing capacity and cannot
accumulate old response history. Validation results live in run records, not this spec.

## Implementation references

- [Core driver](../../../crates/gateway-core/src/runtime/driver.rs),
  [scope/filter machine](../../../crates/gateway-core/src/core/filter/machine.rs), and
  [resume implementation](../../../crates/gateway-core/src/core/filter/machine/resume.rs).
- [Disposition/writer gate](../../../crates/gateway-core/src/runtime/attempt/exchange/disposition.rs),
  [SSE framer](../../../crates/gateway-core/src/runtime/sse/framer.rs), and
  [publication installer](../../../crates/gateway-core/src/core/publication.rs).
- [Core contract fixtures](../../../crates/gateway-core/src/test_support/contracts.rs)
  and [real transport tests](../../../crates/gateway-core/src/transport/pingora/client/tests.rs).
