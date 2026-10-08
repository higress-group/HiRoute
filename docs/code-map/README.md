# Code map

Start with the user action below, follow its production owner, then read one
representative scenario. This map locates current code and contracts; requirements
belong in specifications, implementation detail beside the code, and validation
results in the relevant run record.

## Read progressively

1. Find the capability below, or search with
   `rg -n -i '<user action, symptom or domain term>' docs/code-map/`.
2. Once the relevant topic is located, prefer reading its complete code-map page,
   including ownership boundaries and cross-references. The search match locates
   the page; its surrounding context helps interpret the entry.
3. Follow the production owner and a representative scenario, then expand into
   linked code, specifications or local guides as needed. Unrelated map pages
   need not be read by default.

## Find a capability

| User action | Start here | Representative scenario |
| --- | --- | --- |
| Add model sources and inspect usable models | [Source admission and native configuration](architecture.md#model-sources-and-native-configuration), [Application owners](../../crates/application/src/compute_management) | [Installed CLI management loop](../../crates/daemon/tests/publication_process.rs) |
| Create, edit and publish a routing plan | [Authoring/lifecycle](../../crates/application/src/routing), [compiler](../../crates/application/src/compiler), [domain](../../crates/domain/src/routing) | [Publication and restart](../../crates/daemon/tests/publication_process.rs) |
| Connect a decision model or custom extension; configure smart saving or custom branches | [Decision and routing map](decision-foundation.md) | [Real CLI connection lifecycle](../../crates/daemon/tests/support/decision_services_product.py), [listener multi-turn routing](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Configure, adjust or disable an Agent's models and task collaboration | [Agent/Worker map](worker-context.md), [Desktop feature guide](../../apps/desktop/src/features/agents/README.md) | [Settings ordering](../../crates/application/src/operations/tests/settings_tail.rs), [saved native model journey](../../crates/daemon/tests/support/additional_model_product.py) |
| Route an authenticated model request and apply bounded fallback | [Gateway map](../../crates/gateway/README.md), [execution core](../../crates/gateway-core/src) | [Real listener scenarios](../../tools/e2e-harness/tests/p0_gateway_runtime.rs) |
| Delegate, read, cancel or continue a Worker task | [Worker lifecycle and native differences](worker-context.md#follow-a-task-through-its-owners) | [Delegation](../../tools/product-e2e/tests/worker_delegation.rs), [read/Continue](../../tools/product-e2e/tests/worker_read.rs) |
| Inspect sessions, content, cost and competence | [Observation owners](architecture.md#observation-and-conversation-boundaries), [decision attribution](decision-foundation.md#observation-and-public-consumers) | [Session queries](../../crates/observation/src/query_v2/sessions_tests.rs), [stage summaries](../../crates/observation/src/store/tests/plan_quality/summary.rs) |
| Manage the service through CLI or Desktop | [client-core](../../crates/client-core/src), [CLI](../../crates/cli/src), [Desktop host](../../apps/desktop/src-tauri/src), [daemon control](../../crates/daemon/src/control) | [Real control process](../../tools/product-e2e/tests/control_shell.rs), [Desktop test map](../../apps/desktop/tests/README.md) |
| Build macOS / Linux release installers for either architecture | [Release candidate entry](../../scripts/build-release.py), [build and publication guide](../release-builds.md) | [Package-to-publication contract](../../scripts/test-build-release.py); final publishing remains in [release.yml](../../.github/workflows/release.yml) |

## Read across boundaries

| Map | Scope |
| --- | --- |
| [Architecture and ownership](architecture.md) | Shared layers, source admission, durable state, observation and change boundaries |
| [Agent integrations and Workers](worker-context.md) | Shared configuration/lifecycle owners, with Codex, Claude, Qoder, Pi and DSH differences together |
| [Decision models and routing](decision-foundation.md) | Saved connections, provider adapters, per-turn policy and competence attribution |
| [Tests as product documentation](testing.md) | Proof levels, reusable fixtures, representative fault cases and validation selection |

Keep new navigation organized by stable responsibility, not by a provider or a
completed feature. Add an ecosystem to the shared comparison and link its native
leaf. Put detailed protocol rules in their canonical contract and operating steps
in the existing local guide. When an owner moves, update its entry and incoming
links instead of appending another implementation note to this index.
Keep each matched entry a short clue about where to look next and why. Detailed
rules, parameter lists, exhaustive exceptions and validation history belong in
the linked specification, local guide or run record.
