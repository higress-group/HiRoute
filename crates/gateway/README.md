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
Qoder's dedicated entry and delegated run authority keep their existing rules.
Upstream authorization is rebuilt from the selected provider lease. The
[listener scenario](../../tools/e2e-harness/tests/p0_gateway_runtime/bearer_auth.rs)
covers admission, rejection, token rotation and outbound isolation.

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

## Native session correlation

[context_hold/identity.rs](src/context_hold/identity.rs) resolves one identity for
ContextHold, decision history and observation before request content is externalized.
Verified Worker tasks take precedence. Ordinary clients supply these native fields:

| Client | Accepted native identity |
| --- | --- |
| Claude Code | `X-Claude-Code-Session-Id`; Messages also retains the JSON session inside `metadata.user_id` |
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
[p0_gateway_native_sessions.rs](../../tools/e2e-harness/tests/p0_gateway_native_sessions.rs)
covers the real listener and observation boundary across Responses and Messages,
plus OpenCode's native identity across Chat Completions and shared cache affinity.

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
received-byte upper bound without per-token logs. Unix Debug-only
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
