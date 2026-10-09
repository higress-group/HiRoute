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

## Response failure evidence

`runtime/driver/response_diagnostics.rs` owns the closed, payload-free failure
classification; `adapters/response/decoder.rs` reports SSE event ordinal and
received-byte upper bound without per-token logs. Unix Debug-only
`runtime/stream_capture.rs` can capture an explicitly enabled private session;
its `stream_replay` example restores the actual attempt profile and tool mapping.
This is offline decoder evidence, not proof of network timing or task completion.

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
