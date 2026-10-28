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
| Receive complete official model data, inspect defaults and remove its cache | [Feed service](../../crates/daemon/src/model_feed), [shared admission](../../crates/integrations/src/model_feed), [scoped defaults](../../crates/integrations/src/compute/defaults.rs), [metadata queries](../../crates/daemon/src/control/runtime/model_feed/metadata.rs), [publication guide](../../assets/model-feed/README.md), [shared Desktop/console commands](../../crates/product-session/src/commands/model_feed.rs) | [Acceptance checkpoint, confirmed cache recovery and restart](../../crates/daemon/src/model_feed/tests.rs), [producer admission](../../tools/release-facts/src/feed/tests.rs) |
| Try a model gradually, inspect evidence and review a division draft | [Trial workflow and readiness](../../apps/desktop/src/features/trial), [combined setup capture](../../crates/daemon/src/control/runtime/routing/trial.rs), [trust report](../../crates/application/src/trust), [shared trust controls](../../apps/desktop/src/features/trust) | [Retained evidence queries](../../crates/observation/src/store/tests/plan_quality/trust.rs), [trial interaction scenarios](../../apps/desktop/tests/v3/visual/new-model-trial-active.mjs), [setup identity and feedback](../../apps/desktop/tests/v3/visual/trial-setup-followup-active.mjs) |
| Create, edit and publish a routing plan | [Authoring/lifecycle](../../crates/application/src/routing), [compiler](../../crates/application/src/compiler), [domain](../../crates/domain/src/routing) | [Publication and restart](../../crates/daemon/tests/publication_process.rs) |
| Connect a decision model or custom extension; configure difficulty routing or custom branches | [Decision and routing map](decision-foundation.md) | [Real CLI connection lifecycle](../../crates/daemon/tests/support/decision_services_product.py), [listener multi-turn routing](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs) |
| Configure, adjust or disable an Agent's models and task collaboration | [Agent/Worker map](worker-context.md), [Desktop feature guide](../../apps/desktop/src/features/agents/README.md) | [Settings ordering](../../crates/application/src/operations/tests/settings_tail.rs), [saved native model journey](../../crates/daemon/tests/support/additional_model_product.py) |
| Route an authenticated model request and apply bounded fallback | [Gateway map](../../crates/gateway/README.md), [execution core](../../crates/gateway-core/src) | [Real listener scenarios](../../tools/e2e-harness/tests/p0_gateway_runtime.rs) |
| Delegate, read, cancel or continue a Worker task | [Worker lifecycle and native differences](worker-context.md#follow-a-task-through-its-owners) | [Delegation](../../tools/product-e2e/tests/worker_delegation.rs), [read/Continue](../../tools/product-e2e/tests/worker_read.rs) |
| Inspect sessions, content, cost and competence | [Observation owners](architecture.md#observation-and-conversation-boundaries), [decision attribution](decision-foundation.md#observation-and-public-consumers), [timeline presentation](../../apps/desktop/src/features/RoutingTimeline.tsx), [exact model labels](../../apps/desktop/src/features/model-labels.ts) | [Concurrent routing timeline and incremental read bounds](../../crates/observation/src/store/tests/plan_quality/routing_timeline.rs), [admission and presentation](../../crates/observation/src/store/tests/plan_quality/stage_admission.rs), [opening protection projection](../../crates/observation/src/store/tests/plan_quality/routing_timeline_protection.rs), [stage summaries](../../crates/observation/src/store/tests/plan_quality/summary.rs) |
| Connect, resume or adjust through a terminal/Agent | [CLI journey](../../crates/cli/src/quickstart.rs), [complete Plan recovery](../../crates/cli/src/quickstart/plan_restore.rs), [shared authoring](../../crates/application/src/routing/editor_options.rs), [installed schemas](../../crates/application-api/src/command_schemas.rs), [shared management Skill](../../assets/skills/hiroute-management/SKILL.md) | [Installed Quickstart](../../crates/daemon/tests/support/quickstart_product.py), [effective evidence](../../crates/daemon/src/control/runtime/settings_effective_tests.rs) |
| Manage the service through CLI or Desktop | [client-core](../../crates/client-core/src), [CLI](../../crates/cli/src), [Desktop host](../../apps/desktop/src-tauri/src) and [native connection recovery](../../apps/desktop/src-tauri/src/bootstrap/recovery.rs), [daemon control](../../crates/daemon/src/control) | [Real control process](../../tools/product-e2e/tests/control_shell.rs), [Desktop test map](../../apps/desktop/tests/README.md) |
| Configure and open the authenticated console locally or remotely | [Console owner](../../crates/daemon/src/console), [shared product session](../../crates/product-session/src), [console guide](../web-console.md) | [Real role-all authority/HTTP composition](../../crates/daemon/src/console/tests.rs), [production browser scenarios](../../apps/desktop/tests/v3/visual/console-scenarios.mjs) |
| Install and operate native Windows headless, including background service | [Windows operating guide](../standalone-windows.md), [native kernel owners](../../crates/platform/README.md), [Rust installation transaction](../../crates/host-runtime/src/standalone_install) | [Actual installed Control/Gateway/restart](../../crates/daemon/tests/windows_headless.rs); Task is the only offered manager; SCM is refused unchanged and retains low-level tests |
| Upgrade Desktop and recover a failed installation | [Updater](../../apps/desktop/src-tauri/src/updates.rs), [installer handshake](../../apps/desktop/src-tauri/src/updates/installer.rs), [resident restart](../../apps/desktop/src-tauri/src/bootstrap.rs), [shared recovery facts](../../crates/host-runtime/src/storage_recovery.rs), [early admission diagnostics](../../crates/host-runtime/src/storage_recovery/admission.rs), [storage admission](../../crates/local-storage/src/migrations/startup_recovery.rs), [completed-batch identity boundary](../../crates/local-storage/src/migrations/completed_batches.rs) | Preflight → orderly stop → explicit commit; [macOS installation and recovery](../macos-installation.md), [upgrade/reopen phase and recovery scenarios](../../crates/local-storage/src/migrations/upgrade_batch_recovery_tests.rs) |
| Install, open or update Windows Desktop | [Windows shell](../../apps/shell/README.md), [shell payload and rollback ownership](../../apps/shell/src-tauri/src/runtime/install_transaction.rs), [current startup failure](../../apps/shell/src-tauri/src/runtime/startup_failure.rs), [local-console recovery and catalog failures](../windows-desktop-installation.md), [Rust Task transaction](../../crates/host-runtime/src/standalone_install), [restoration policy](../../crates/host-runtime/src/standalone_install/uninstall.rs), [owned package cleanup](../../crates/host-runtime/src/standalone_install/cleanup.rs) | [Native NSIS smoke](../../crates/daemon/tests/windows_desktop.rs), [shell page/host scenarios](../../apps/desktop/tests/v3/visual/shell-host-scenarios.mjs), [required native gates](../windows-desktop-installation.md#evidence-and-release-claims) |
| Maintain agent instructions, shared skills and issue-native planning | [Instruction ownership](../../.agents/README.md), [project workflow](../../issue-spec/project/workflow.md), [feedback routing](../../.agents/skills/hiroute-workflow-improvement/SKILL.md) | Pinned generated manifest, individual Claude links and current GitHub artifacts |
| Select, execute or diagnose a build/test run | [Validation workflow](../../.agents/skills/hiroute-validation/SKILL.md), [test map](testing.md) | [Selection](../../scripts/test-plan.py), [configured runner](../../scripts/validation.py) and revision-bound reports |
| Build macOS / Windows Desktop or Linux / Windows headless release candidates | [Release candidate entry](../../scripts/build-release.py), [build and publication guide](../release-builds.md), [independent Unix/Windows candidate jobs](../../.github/workflows/build-release.yml) | [Package-to-publication contract](../../scripts/test-build-release.py), [Linux ABI gate](../../scripts/linux-release-baseline.json), [Windows Desktop identity](../../scripts/package-windows-shell.py); final publishing remains in [release.yml](../../.github/workflows/release.yml) |

## Read across boundaries

| Map | Scope |
| --- | --- |
| [Architecture and ownership](architecture.md) | Shared layers, source admission, durable state, observation and change boundaries |
| [Agent integrations and Workers](worker-context.md) | Shared configuration/lifecycle owners, with Codex, Claude, Qoder, Pi, DSH and OpenCode differences together |
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
