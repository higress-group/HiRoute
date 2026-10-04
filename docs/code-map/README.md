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
| Select saved additional Qoder model routes while keeping task collaboration independent | Agent connections | [Shared settings transaction](../../crates/application/src/agent_connection/settings), [native provider ownership](../../crates/integrations/src/agents/qoder_native/README.md), [Desktop model editor](../../apps/desktop/src/features/agents/QoderModelEditor.tsx) | [Ordinary native startup journey](../../crates/daemon/tests/support/qoder_model_product.py), [five Qoder journeys and evidence limits](../../tools/product-e2e/tests/QODER_DELEGATION.md) |
| Route a model request and apply bounded fallback | Request routing | [Gateway map](../../crates/gateway/README.md), [execution core](../../crates/gateway-core/src) | [Real listener scenarios](../../tools/e2e-harness/tests/p0_gateway_runtime.rs) |
| Ask an external service to select a branch | [Decision API](../../decision-extensions/api/README.md) | [Gateway classification](../../crates/gateway/src/core_runtime/classification.rs), [Jev reference service](../../decision-extensions/extensions/jev-decider/README.md) | [Jev tests](../../decision-extensions/extensions/jev-decider), real listener classifier scenarios above |
| Delegate, read and continue Worker tasks | Agent delegation | [Application authorization](../../crates/application/src/delegation), [daemon execution](../../crates/daemon/src/delegation), [OS launcher](../../crates/daemon/src/delegation/local_worker/README.md) | [Worker lifecycle](../../tools/product-e2e/tests/worker_delegation.rs), [read/continue](../../tools/product-e2e/tests/worker_read.rs) |
| Inspect sessions, content, cost and quality | Session history, usage | [Authorized queries](../../crates/application/src/observation_query), [observation](../../crates/observation/src), [managed content](../../crates/observation/src/managed_text/README.md) | [Fixture contracts](../../tools/product-e2e/tests/local_observation.rs); [installed CLI journey](../../crates/daemon/tests/publication_process.rs) |
| Manage the local service through CLI or Desktop | Shared access | [client-core](../../crates/client-core/src), [CLI](../../crates/cli/src), [Desktop host](../../apps/desktop/src-tauri/src), [control socket](../../crates/daemon/src/control) | [Real process journey](../../tools/product-e2e/tests/control_shell.rs), [Desktop test map](../../apps/desktop/tests/README.md) |

Read [architecture and ownership](architecture.md) for cross-module changes,
[tests as product documentation](testing.md) before changing coverage, and
[remaining cleanup work](maintenance.md) before starting another refactor.
For later decision features, read the [provider and consumer boundaries](decision-foundation.md).
Before extending Worker ecosystems, read the [native context, route and history owners](worker-context.md).

For an Agent implementation task, first identify the capability, its state owner,
and the failing or representative scenario. Give parallel contributors disjoint
file ownership; reserve shared contracts, manifests and generated registries for
one convergence owner. Prefer extracting a cohesive responsibility beside its
consumers over moving unrelated code into a global utility directory.
