# Architecture and ownership

[Code map](README.md) · [Agent/Worker map](worker-context.md) ·
[Decision map](decision-foundation.md) · [Test map](testing.md)

## Shared layers

| Responsibility | Production owner | Boundary |
| --- | --- | --- |
| Business types and invariants | [Domain](../../crates/domain/src) | Typed intent, identity, immutable plans and operation checkpoints |
| Local control contract | [Application API](../../crates/application-api/src), [client-core](../../crates/client-core/src) | Shared requests/responses for CLI and Desktop; generated schemas come from typed producers |
| Business workflows | [Application](../../crates/application/src) | Planning and authorization through ports; external effects are supplied by the daemon |
| Composition and process lifecycle | [Daemon control](../../crates/daemon/src/control/runtime), [delegation](../../crates/daemon/src/delegation) | Concrete adapters, accepted operations, Worker execution and recovery |
| Durable state and external effects | [Local Storage](../../crates/local-storage/src), [Integrations](../../crates/integrations/src) | Store current state; read or conditionally change external Agent configuration through its native leaf |
| Request execution | [Gateway](../../crates/gateway/README.md), [gateway-core](../../crates/gateway-core/src) | Authenticate against published authority, plan attempts, adapt protocols and enforce execution budgets |
| Business evidence | [Observation](../../crates/observation/src), [authorized queries](../../crates/application/src/observation_query) | Execution facts, managed content, session queries and valuation; [diagnostics](../../crates/diagnostics/src) remain separate |
| Product presentation | [Desktop product shell](../../apps/desktop/src/product), [native host](../../apps/desktop/src-tauri/src), [CLI](../../crates/cli/src) | The native host projects safe DTOs; frontend state does not grant authority or establish durable success |

CLI/Desktop → client-core → daemon control → Application → storage/native adapters
is the control path. Published plans and grants feed Gateway execution. Worker
admission freezes a plan and run authority before the native client calls Gateway.
Observation records what actually ran and supplies authorized reads and continuation.

`LocalControlAdapter` remains a broad assembly facade. Follow its focused runtime
module before extending it; a local feature does not require a facade-wide rewrite.

## Model sources and native configuration

CLI secret entry uses the shared local [protected input socket](../../crates/daemon/src/control/standalone.rs)
in both standalone and Desktop-owned `role=all` processes. The [CLI input owner](../../crates/cli/src/host_commands.rs)
reads inherited descriptors; ordinary control requests never carry secret bytes.
All current hosts and the CLI resolve its `hiroute/input.sock` path through host-runtime.
The name must fit any runtime root that supports `hiroute/control.sock`, including macOS.

| User step | Owners to follow | Representative guard |
| --- | --- | --- |
| Discover a native source without executing it | [Native model connections](../../crates/integrations/src/model_connections), [Pi reader](../../crates/integrations/src/agents/pi_sources.rs), [DSH reader](../../crates/integrations/src/agents/dsh_sources.rs) | Static declarations and supported credential references only; OAuth/helpers are not executed by discovery |
| Display, prepare and save the selected source | [Safe Desktop projection](../../apps/desktop/src-tauri/src/bridge/model_connection_web.rs), [daemon preparation](../../crates/daemon/src/control/runtime/model_connections/pi_discovered.rs) | Provider/model identity remains visible; privileged scan material stays private; source changes invalidate the prepared candidate |
| Rename a connection, append models or remove saved configuration | [Shared lifecycle planner](../../crates/application/src/compute_management/mutations/lifecycle.rs), [transactional reference guards](../../crates/local-storage/src/control/compute/management/references.rs), [Desktop lifecycle dialog](../../apps/desktop/src/features/models/ConnectionLifecycleDialog.tsx) | Templates do not own credentials. Explicit append preserves saved members; enabled/disabled routes and retained recovery versions block removal. Removing the last model requires explicit connection deletion; owned secrets follow the same recoverable operation. |
| Connect a native Codex or Claude subscription | [Source discovery](../../crates/integrations/src/agents/subscription_sources.rs), [credential adapters](../../crates/cpa-bridge/src), [shared lifecycle](../../crates/daemon/src/control/runtime/subscriptions.rs), [provider runtimes](../../crates/cpa-bridge/src/runtime_set.rs) | Native clients retain refresh authority; only access leases enter isolated CPA instances. Claude Keychain interaction is limited to an explicit check; discovery reads metadata only. Codex borrowing reads file storage only and rejects a configured non-file store even if stale auth.json remains. |
| Sign in independently to a Codex or Claude subscription | [Shared login control](../../crates/application/src/control_plane/subscription_login.rs), [protected lifecycle](../../crates/daemon/src/control/runtime/subscriptions/login.rs), [private sessions](../../crates/cpa-bridge/src/managed_sessions.rs), [Desktop sign-in](../../apps/desktop/src/features/subscriptions/SubscriptionSignIn.tsx) | CPA owns the new authorization and refresh; login is separate from Check/Save and routing. Daemon maintenance owns pending completion/expiry, and saved revisions own runtime selection. [The pinned CPA adapter](../../vendor/cpa/README.md) explains refresh ownership and request recovery. |
| Lease a subscription credential without blocking Gateway workers | [Async adapter](../../crates/daemon/src/gateway_ports/credential/cpa.rs), [request budget](../../crates/cpa-bridge/src/request_context.rs), [admission and observations](../../crates/cpa-bridge/src/runtime/admission.rs), [Claude identity resolution](../../crates/cpa-bridge/src/claude_profile.rs) | Bounded provider executors own compatibility I/O. Short admission updates revoke before lifecycle work; identity lookup runs outside the lifecycle owner. Cancellation and saved revisions guard later publication. Follow [concurrency regressions](../../crates/cpa-bridge/src/runtime/tests/request_io.rs) for races with disable, replacement and delayed I/O. |
| Inspect saved subscription mode and repair sign-in | [V3 query](../../crates/application/src/compute_management/query_v3.rs), [mode and repair copy](../../apps/desktop/src/features/models/subscription-copy.ts) | Mode derives from the committed source selection and survives missing credentials. Desktop consumes V3; published V2 queries retain their strict response shape. Existing release-version matching still applies; an external service without V3 requires an update. |
| Make a saved source usable by a plan | [Capability qualification](../../crates/application/src/compute_management/compilation.rs), [candidate materialization](../../crates/daemon/src/control/runtime/candidate_execution.rs), [compiler](../../crates/application/src/compiler) | Native-observed facts retain source-local authority and budgets; [unavailable candidate reasons](../../crates/daemon/src/control/runtime/routing_candidates.rs) are read-only and never execution facts. Upstream model values use the shared opaque-ID rule; internal references remain strict |
| Distinguish a missing subscription candidate from a runtime outage | [Exact candidate projection](../../crates/daemon/src/control/runtime/compute_routing.rs), [source authorization](../../crates/daemon/src/control/runtime/source_authorization.rs), [provider discovery](../../crates/cpa-bridge/src/runtime_set.rs) | Resolve the requested provider from the trusted connection option. Its discovery error remains Unavailable; a best-effort aggregate list cannot establish that a selected source is absent. |
| Configure models and collaboration independently | [Shared settings transaction](../../crates/application/src/agent_connection/settings), [typed native facts](../../crates/daemon/src/control/runtime/settings_facts), [native provider guide](../../crates/integrations/src/agents/additional_native/README.md) | Native defaults, model grants, installed Skills and conditional restore have distinct owners |
| Project native reasoning settings | [Wire types](../../crates/domain/src/routing/gateway_execution.rs), [daemon renderer](../../crates/daemon/src/control/runtime/candidate_protocol_profiles.rs), [Gateway reader](../../crates/gateway/src/profiles/reasoning.rs) and [wire normalization](../../crates/gateway/src/adapters/request/reasoning.rs) | Keep one field/path/toggle definition; [materializer → compiler → Gateway tests](../../crates/daemon/src/control/runtime/candidate_execution_tests.rs) cover accepted aliases |

Native configuration precedence belongs to each reader. For example, Pi provider
`baseUrl` overrides inherited catalog addresses, while provider `api` defaults apply
only to explicit models; [filesystem tests](../../crates/integrations/src/agents/filesystem_tests/pi.rs)
cover that distinction. DSH module replacement and credential-store precedence
are located in the [native composition guide](../../crates/integrations/src/agents/additional_native/README.md#dsh-static-composition).
Do not move either rule into a generic provider merge.

Follow discovery through an ordinary request using the saved source. The
[Pi](../../tools/product-e2e/tests/PI_INTEGRATION.md) and
[DSH](../../tools/product-e2e/tests/DSH_INTEGRATION.md) acceptance entries cover
Scan → Prepare → Save → restart → native use, including stale-source rejection.

## State and lifecycle boundaries

| Boundary | Invariant | Read next |
| --- | --- | --- |
| Agent settings | Configure/Edit seals service state before the file tail. Ordinary Disable conditionally restores files before withdrawing grants; conflicts preserve foreign edits and require the appropriate fresh operation or exact retry. | [Transaction](../../crates/application/src/agent_connection/settings/transaction.rs), [ordering tests](../../crates/application/src/operations/tests/settings_tail.rs) |
| Native target registration | An unused discovered target is not access authority or a global startup dependency. Actual access and accepted-operation recovery retain path identity and safe-write checks; OS I/O determines accessibility. | [Registration tests](../../crates/local-storage/src/agents/native_registration_tests.rs), [startup journey](../../crates/daemon/tests/publication_process.rs) |
| Operation journal | Domain checkpoints and stored journals agree before publication or native effects; immutable plan bytes and generation/identity guards remain authoritative. JSON member ordering is distinct from a content change. | [Checkpoint](../../crates/domain/src/operation/journal.rs), [verification](../../crates/local-storage/src/control/journal.rs), [transaction guards](../../crates/local-storage/src/control/transaction_v2_tests.rs) |
| Managed credential rollback | Authenticate the exact compensated Secret before converging source references under the same writer. Forward generations and source/workspace revisions reject stale authority; the atomic receipt makes restart idempotent. | [Coordinator](../../crates/application/src/operations/compute_compensation.rs), [Secret proof](../../crates/local-storage/src/secrets/compensation.rs), [Control receipt](../../crates/local-storage/src/control/compute/management/compensation.rs) |
| Reference refresh | A checkpoint copies verified installed routing authority at the next revision through the normal publication writer. Preview pins revisions and content; disabled routes, drafts and held versions remain deletion blockers. | [Strict checkpoint](../../crates/domain/src/operation/publication_checkpoint.rs), [shared entry](../../crates/application/src/control_plane/publication_checkpoint.rs), [installed-state facts](../../crates/daemon/src/control/runtime/routing.rs) |
| Profile deletion | Untouched proof binds the current store instance and exact marker; a stage directory alone cannot prove deletion never began. | [Profile lifecycle](../../crates/daemon/src/control/runtime/settings_profile_tests.rs) |
| Worker/content lifetime | Result completion, process stop, body availability and permission to Continue are separate facts; cleanup requires exact ownership. | [Worker map](worker-context.md#state-and-lifetime), [managed-text contract](../../crates/observation/src/managed_text/README.md) |
| Recovery and compatibility | Current producers have one ingestion contract. Registered legacy revoke tails retain their own ordering; frozen fixtures alone do not establish production upgrade support. | [Startup admission](../../crates/local-storage/src/migrations/startup_format.rs), [legacy-tail tests](../../crates/daemon/src/control/runtime/settings_profile_legacy_tests.rs), [compatibility registry](../../contracts/compatibility-support.v1.json) |

## Observation and conversation boundaries

| Concern | Owner | What callers must preserve |
| --- | --- | --- |
| Capture completeness | [Content completeness](../../crates/observation/src/content/completeness.rs), [session query](../../crates/observation/src/query_v2/sessions.rs) | Aggregate every required request/direction; a finished stream or readable preview does not prove the session is complete |
| Streaming memory | [Capture](../../crates/gateway/src/observation/provider/capture.rs), [pending delivery](../../crates/gateway/src/observation/provider/capture/tracker.rs) | Decoder, projector and delivery tracker share the stream budget; queue capacity is not capture evidence |
| Upstream fault attribution | [Wire projection](../../crates/gateway/src/observation/provider/wire_diagnostic.rs), [attempt association](../../crates/gateway/src/observation/request/diagnostic.rs), [decision calls](../../crates/gateway/src/core_runtime/classification/diagnostic.rs) | Actual bounded model/control fields, negotiated HTTP protocol, closed error codes, hashed request IDs and separate client headers/body commit facts must preserve the real outcome without prompts or provider error text |
| Inspect CPA authentication recovery | [Private response metadata](../../crates/gateway/src/observation/provider/cpa_execution.rs), [pinned CPA adapter](../../vendor/cpa/README.md) | Logical attempts and actual HTTP sends differ; only verified CPA targets can supply recovery counters. Missing evidence remains unknown and does not create token usage. |
| Search and gaps | [Query](../../crates/observation/src/query_v2/search.rs), [index worker](../../crates/observation/src/text_index/worker.rs), [status](../../crates/observation/src/query/status.rs) | Bounded scan windows, wraparound and explicit gaps distinguish incomplete search from an empty result |
| Business relations and visibility | [Relation owner](../../crates/observation/src/query_v2/relation.rs), [managed content](../../crates/observation/src/managed_text/README.md) | `query_v2` also writes relations; every content read checks current authorization and visibility |
| Native tool-history continuation | [Responses ingress](../../crates/gateway/src/adapters/responses_ingress.rs), [real continuation journey](../../tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs) | Preserve model-generated empty tool names in native history without relaxing declarations/call IDs; unsafe conversion, ContextHold and reasoning cleanup stay disabled for that history |
| Decision and competence attribution | [Decision observation map](decision-foundation.md#observation-and-public-consumers) | Attribute scores to the stage that actually ran, with its frozen category/group/candidate and rubric |

## Changing boundaries

Keep a change with the owner that can enforce it. Extend a native leaf for a real
ecosystem difference; reuse settings, admission, lifecycle and observation owners.
Built-in decision models and custom branches are implemented in the
[decision map](decision-foundation.md); tool selection remains protocol-only.

Before extracting or consolidating code, identify the user scenario, state owner
and unique failure assertions. Keep recovery, cancellation, authorization and
foreign-file preservation tests. Assign shared contracts and generated registries
to one convergence owner. Use the [test map](testing.md) to select evidence; neither
file size nor test churn alone justifies another abstraction or a broad rewrite.
