# Gateway request and decision boundaries

The production listener is composed in [server.rs](src/server.rs).
[publication/compiler.rs](src/publication/compiler.rs) turns one accepted
publication into immutable request authority. Read
[core_runtime.rs](src/core_runtime.rs) for the real request path: authenticate and
bind authority, prepare the canonical request and Replay, obtain a request-owned
branch decision, plan and freeze candidates, then admit execution into
`hiroute-gateway-core`. The core owns attempts, response commit, cancellation and
resource cleanup; the Gateway does not run a second execution loop.

The standalone `hirouted` entry can attach the same typed diagnostics using
`--diagnostics-root <private-root>` and an optional temporary
`--diagnostic-level-override`. These diagnostics do not enable session content
capture or execution-fact storage. At Info, failed attempts retain the safe
actual model and reasoning controls, HTTP protocol/status and body commit state.

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

Known incomplete nonstream JSON responses follow the same prebody relay boundary
as streaming failures, for both native and converted protocols. Reported usage
survives fallback; partial response content is not delivered. Attempt/deadline
limits and fixed-source authority still apply, and unknown completion remains
separate from an explicit failure.
