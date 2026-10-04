# Gateway request and decision boundaries

The production listener is composed in [server.rs](src/server.rs).
[publication/compiler.rs](src/publication/compiler.rs) turns one accepted
publication into immutable request authority. Read
[core_runtime.rs](src/core_runtime.rs) for the real request path: authenticate and
bind authority, prepare the canonical request and Replay, obtain a request-owned
branch decision, plan and freeze candidates, then admit execution into
`hiroute-gateway-core`. The core owns attempts, response commit, cancellation and
resource cleanup; the Gateway does not run a second execution loop.

## Decision operation

[core_runtime/classification.rs](src/core_runtime/classification.rs) owns the
current local/REST classifier operation. Its clock starts before history
preparation in `core_runtime.rs`; preparation, Secret resolution, DNS, connection,
write and read share the bounded operation. The earlier source deadline and
source cancellation remain authoritative. A classifier failure can use the
existing local fallback only while the source request remains active.

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

[planner.rs](src/planner.rs) validates the selected branch and chooses eligible
groups/candidates under the frozen policy. Selected branch and actual executed
branch can differ after an authorized fallback; history and assessment must
retain that distinction. They cannot be reconstructed from display names.

## Extension ownership

The existing [official Jev service](../../decision-extensions/extensions/jev-decider/README.md#code-and-responsibility-map)
separates deterministic request/answer logic from its Python HTTP lifecycle.
General inference, purpose policy, provider transport and execution authorization
have different owners: a model/tool selection purpose defines its question and
allowed IDs; a provider handles its wire protocol and credentials; only the
calling product authorizes actual execution. The current smart-saving policy is
not a universal inference or tool-routing contract.

The Decision API can carry a generic branch map, but current published Gateway
smart-saving policies remain binary. Future natural-language branch conditions
need caller-owned stable IDs, a publication-pinned allowed set and explicit
failure/default policy. Keep provider response interpretation outside candidate
execution. This responsibility map adds no provider, branch or fallback behavior.
See the shared [decision foundation map](../../docs/code-map/decision-foundation.md)
for provider references and the separately owned future selection contracts.

## Representative capability tests

| User capability or invariant | Existing entry |
| --- | --- |
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
