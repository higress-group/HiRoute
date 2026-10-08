# Code map

Start with a user capability and one representative test, then follow the
production owner. This map describes the current implementation and its executable
contracts; it is navigation, not a second requirements system. Update it when
ownership or entry points change.

| User capability | Contract area | Production owners | Representative tests / local guide |
| --- | --- | --- | --- |
| Add model sources and inspect usable models | Model connections | [Application](../../crates/application/src/compute_management), [native integrations](../../crates/integrations/src/model_connections), [daemon assembly](../../crates/daemon/src/control/runtime/model_connections.rs) | [Typed/fixture contracts](../../tools/product-e2e/tests/compute_pool.rs); [installed CLI journey](../../crates/daemon/tests/publication_process.rs) |
| Create and publish a routing plan | Routing plans | [Authoring and lifecycle](../../crates/application/src/routing), [compiler](../../crates/application/src/compiler), [domain types](../../crates/domain/src/routing) | [Compiler/fixture contracts](../../tools/product-e2e/tests/routing_plans.rs); [installed CLI journey](../../crates/daemon/tests/publication_process.rs) |
| Connect, edit and safely disable an Agent | Agent connections | [Application transaction](../../crates/application/src/agent_connection), [filesystem adapters](../../crates/integrations/src/agents), [Desktop feature](../../apps/desktop/src/features/agents/README.md) | [Settings ordering](../../crates/application/src/operations/tests/settings_tail.rs), [daemon product entry](../../crates/daemon/src/control/runtime/settings_profile_tests.rs) |
| Select saved additional Qoder/Pi/DSH model routes while keeping task collaboration independent | Agent connections | [Shared settings transaction](../../crates/application/src/agent_connection/settings), [native provider ownership](../../crates/integrations/src/agents/additional_native/README.md), [Desktop model editor](../../apps/desktop/src/features/agents/AdditionalModelEditor.tsx) | [Shared native journeys](../../crates/daemon/tests/support/additional_model_product.py), [Qoder](../../tools/product-e2e/tests/QODER_DELEGATION.md) and [Pi](../../tools/product-e2e/tests/PI_INTEGRATION.md) entry and limits |
| Route a model request and apply bounded fallback | Request routing | [Gateway map](../../crates/gateway/README.md), [execution core](../../crates/gateway-core/src) | [Real listener scenarios](../../tools/e2e-harness/tests/p0_gateway_runtime.rs) |
| Connect decision models or custom extensions, configure branches and competence upgrades | [Decision models and extensions](../../decision-extensions/README.md) | [Provider, policy and observation owners](decision-foundation.md) | [Desktop connection and route journeys](../../apps/desktop/tests/v3/browser/product-shell-scenarios.mjs), [real CLI lifecycle](../../crates/daemon/tests/support/decision_services_product.py), [listener branch journeys](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Delegate, read and continue Worker tasks | Agent delegation | [Application authorization](../../crates/application/src/delegation), [daemon execution](../../crates/daemon/src/delegation), [OS launcher](../../crates/daemon/src/delegation/local_worker/README.md) | [Worker lifecycle](../../tools/product-e2e/tests/worker_delegation.rs), [read/continue](../../tools/product-e2e/tests/worker_read.rs) |
| Inspect sessions, content, cost and quality | Session history, usage | [Authorized queries](../../crates/application/src/observation_query), [observation](../../crates/observation/src), [managed content](../../crates/observation/src/managed_text/README.md) | [Fixture contracts](../../tools/product-e2e/tests/local_observation.rs); [installed CLI journey](../../crates/daemon/tests/publication_process.rs) |
| Manage the local service through CLI or Desktop | Shared access | [client-core](../../crates/client-core/src), [CLI](../../crates/cli/src), [Desktop host](../../apps/desktop/src-tauri/src), [control socket](../../crates/daemon/src/control) | [Real process journey](../../tools/product-e2e/tests/control_shell.rs), [Desktop test map](../../apps/desktop/tests/README.md) |

Read [architecture and ownership](architecture.md) for cross-module changes,
[tests as product documentation](testing.md) before changing coverage, and
[remaining cleanup work](maintenance.md) before starting another refactor.
For decision services and future tool selection, read the [provider and consumer boundaries](decision-foundation.md).
Conversation capture completeness is derived per request and required direction by
[the content completeness owner](../../crates/observation/src/content/completeness.rs),
shared by session summaries and the authorized content catalog. A finished stream
does not prove that sibling requests or delivered responses were captured.
The [V2 session query](../../crates/observation/src/query_v2/sessions.rs) aggregates
this evidence over matching authorized requests; Desktop title previews do not
own the session completeness badge. [Text search](../../crates/observation/src/query_v2/search.rs)
bounds directory scans before joins, carries index gaps in its existing cursor,
and distinguishes an unfinished search from a confirmed empty result. The
[status projection](../../crates/observation/src/query/status.rs) limits gap details
with an explicit truncation flag while aggregating completeness over all gaps.
The [capture worker](../../crates/gateway/src/observation/provider/capture.rs) shares
the existing stream memory budget with its decoder, projector and pending delivery
tracker; processed wire copies release their charge. The daemon owns the bounded
local channel capacity for history bursts. Neither a larger queue nor a readable
prefix certifies a complete capture.
The [text-index worker](../../crates/observation/src/text_index/worker.rs) scans a
bounded row window while holding the ingestion lock, revisits skipped blobs on
wraparound, and validates content before publishing search results.
The [Responses ingress adapter](../../crates/gateway/src/adapters/responses_ingress.rs)
preserves model-generated empty tool names in native history without relaxing
tool declarations or call IDs. Such history disables cross-protocol conversion,
ContextHold and reasoning cleanup; [the real continuation journey](../../tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs)
covers a streamed failed call followed by an authenticated native continuation.
For DeepSeek Harness, start with [the DSH boundary map](dsh.md).
Before extending Worker ecosystems, read the [native context, route and history owners](worker-context.md).

Pi static API discovery is owned by [the bounded native reader](../../crates/integrations/src/agents/pi_sources.rs)
and [closed candidate prepare](../../crates/daemon/src/control/runtime/model_connections/pi_discovered.rs).
The [Desktop scan projection](../../apps/desktop/src-tauri/src/bridge/model_connection_web.rs)
keeps provider/model identifiers visible while omitting privileged discovery material.
Adding a safe scan field also requires this projection and its existing privacy/visibility
test: a successful machine scan or native compile does not prove that its row reaches the UI.
Provider/model pairs remain distinct; auth commands and OAuth are reported without execution/import.
After save, follow [capability qualification](../../crates/application/src/compute_management/compilation.rs)
and [candidate materialization](../../crates/daemon/src/control/runtime/candidate_execution.rs)
into the compiler's source-local authority and the published Plan. Native-observed
facts retain their provenance and observed budgets; they are not a catalog match
or a runtime fallback. The required saved-source route case in
[Pi acceptance](../../tools/product-e2e/tests/PI_INTEGRATION.md) covers this path with
an ordinary native request, including the case where the UI can display a source
that was not yet usable by Plan consumers.

Native reasoning fields have one current wire owner in
[Domain](../../crates/domain/src/routing/gateway_execution.rs).
[Daemon profile rendering](../../crates/daemon/src/control/runtime/candidate_protocol_profiles.rs)
and [Gateway ingestion](../../crates/gateway/src/profiles/reasoning.rs) reuse its
paths and toggle semantics. The existing producer/consumer test in
[candidate materialization tests](../../crates/daemon/src/control/runtime/candidate_execution_tests.rs)
follows materialization → compilation → Gateway validation across supported
protocols and parameter aliases. Keep that assertion at the boundary when adding
a protocol; duplicating local DTOs or testing only the renderer misses drift.

Shared settings facts have common transaction inputs plus typed Codex, Claude and additional-provider
facts; native model/catalog policies belong to those typed leaves.

For an Agent implementation task, first identify the capability, its state owner,
and the failing or representative scenario. Give parallel contributors disjoint
file ownership; reserve shared contracts, manifests and generated registries for
one convergence owner. Prefer extracting a cohesive responsibility beside its
consumers over moving unrelated code into a global utility directory.
