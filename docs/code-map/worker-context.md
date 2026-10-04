# Worker native context, route and history

[Code map](README.md)

A delegated task uses the product instance's effective native user context to find
skills, while HiRoute controls the frozen Plan, per-run credential and permission
policy. A task owns its HiRoute metadata; the native client owns borrowed user
configuration and history. This page locates those responsibilities. Executable tests define the boundaries described here; validation reports must
record their exact revision, environment and scenario outcomes.

## Follow a task through its owners

| Product question | Read this owner | Boundary to preserve |
| --- | --- | --- |
| How does Desktop confirm a Worker installation? | [React selection](../../apps/desktop/src/features/WorkerDependencies.tsx), [native preparation and confirmation](../../apps/desktop/src-tauri/src/bridge/worker_tasks.rs), [protected submission and recovery](../../apps/desktop/src-tauri/src/session/worker_dependencies.rs) | Native preparation validates the shared launch shape and canonical file metadata before freezing the selection in a window-bound, expiring, single-use confirmation. Qoder contains only a CLI; Codex/Claude retain their adapter requirements. The confirmed canonical request feeds the shared digest and revision plan; discovery is not authority. |
| Why does a selected CLI survive restart and upgrade? | [Validated launch form](../../crates/domain/src/delegation/installation.rs), [selection storage](../../crates/local-storage/src/control/worker_dependencies.rs), [upgrade](../../crates/local-storage/src/migrations/worker_dependencies_v24.rs) | Harness chooses native ACP or adapter ACP. Preserve previous selection and Operation bytes during migration; resume interrupted upgrades only through the existing complete three-store backup coordinator. |
| Where does an Agent learn the valid installation and Plan request shape? | [Worker schema producer](../../crates/application-api/src/generated/worker.rs), [Plan schema producer](../../crates/application-api/src/generated/plan_authoring.rs), [generator](../../crates/cli/src/bin/generate_cli_contract.rs), [published schemas](../../contracts/cli) | The Rust `generated/` directory contains producer source; `contracts/cli` contains its output. Update both request families when adding a harness. Generate twice, retain old adapter requirements and native-only forbidden fields, then run the [exact artifact gate](../../crates/cli/tests/contracts.rs). |
| Did a Worker submission reach the daemon? | [CLI submission](../../crates/cli/src/worker.rs), [local submission receipts](../../crates/cli/src/worker/receipt.rs), [daemon task admission](../../crates/daemon/src/control/runtime/delegation_tasks.rs) | The CLI rejects a reused submission key with different selectors or intent before transport. A local receipt conflict is distinct from daemon admission. Product fixtures borrowing native HOME must still give HiRoute receipts their own directory, stable for replay within one scenario and separate between scenarios. |
| How does a Qoder main Agent prove it can use task routing? | [Metadata discovery and checks](../../crates/integrations/src/agents/qoder_discovery.rs), [native Skill/CLI verification](../../crates/integrations/src/agents/qoder_probe.rs), [product journey](../../tools/product-e2e/tests/QODER_DELEGATION.md) | A pre-install capability check does not prove the installed user Skill; either scoped check is distinct from actual delegation. Installed-Skill evidence binds the actual user target and trusted CLI result; failed rechecks invalidate prior evidence. These backend facts do not require a separate settings-page verification step. Borrowed HOME is not an isolation claim. |
| Which user resources does this Worker see? | [Installation](../../crates/daemon/src/delegation/installation.rs), [native context](../../crates/daemon/src/delegation/profile/native_context.rs) | New tasks use effective HOME plus explicit CODEX_HOME, CLAUDE_CONFIG_DIR or QODER_CONFIG_DIR. Pilot/service overrides are authoritative; executable location does not identify a different user's home. |
| Which endpoint, model and credential can it use? | [Profile guide](../../crates/daemon/src/delegation/profile/README.md), [Codex](../../crates/daemon/src/delegation/profile/codex.rs), [Claude](../../crates/daemon/src/delegation/profile/claude.rs), [Qoder](../../crates/daemon/src/delegation/profile/qoder.rs) | Apply native startup/session overrides without editing shared settings. Credentials stay transient. Qoder's shared native settings renderer lives in integrations; Gateway retains the frozen Plan's upstream reasoning authority. |
| Can the native client use the frozen Plan's context and output budget? | [Qoder Plan projection](../../crates/integrations/src/agents/qoder_budget.rs), [shared budget policy](../../crates/domain/src/agents/qoder_model.rs), [shared native renderer](../../crates/integrations/src/agents/qoder.rs), [budget contract and limits](../../crates/daemon/src/delegation/profile/README.md) | Project both context and output from the frozen Plan into native model metadata. Reject unknown or unusable budgets before launch. Gateway output capping alone cannot prove the native client's compaction reservation; preserve the small-window real-Agent journey and version-specific native control. |
| Who admits its requests and attributes them to the right Worker? | [Run authority](../../crates/daemon/src/delegation/run_authority.rs), [Gateway admission and observation metadata](../../crates/gateway/src/dispatch/run.rs) | Profile construction alone cannot admit a request. The sealed Plan and run must agree on the typed harness, and the Gateway must accept its current observation identity while rejecting unknown values. Check both consumers when adding an ecosystem. |
| Has the native session accepted the planned model? | [ACP model confirmation](../../crates/daemon/src/delegation/acp/model.rs) | New and restored sessions confirm the profile's exact native model ID before prompt. Qoder adds its native provider prefix only here; the Plan alias stays unchanged. Missing or ambiguous capability fails explicitly. |
| What exactly does Continue restore? | [Native history binding](../../crates/daemon/src/delegation/profile/native_history.rs), [resume](../../crates/daemon/src/delegation/executor/resume.rs) | Bind exact context and native ID, then restore that session with a fresh run credential. Never adopt the newest file or silently create a replacement session. |
| Who starts and stops processes? | [OS launcher guide](../../crates/daemon/src/delegation/local_worker/README.md), [execution](../../crates/daemon/src/delegation/executor.rs) | Explicit argv/env/materials and owned process groups; no native configuration parsing or inferred history deletion in the launcher. |
| Can two Workers start against a cold Codex root? | [Startup coordination](../../crates/daemon/src/delegation/lifecycle/startup.rs), [ACP initialize receipt](../../crates/daemon/src/delegation/acp/mod.rs) | Only same-daemon, same borrowed config-root spawn/initialize is coordinated. Release before New/Load and prompt; no per-root task concurrency limit or native SQLite access. |
| What remains after hiding or expiring content? | [Read authorization](../../crates/daemon/src/control/runtime/delegation_reads.rs), [Continue](../../crates/daemon/src/control/runtime/delegation_continue.rs), [maintenance](../../crates/daemon/src/control/runtime/delegation_maintenance.rs), [owned cleanup](../../crates/daemon/src/delegation/native_cleanup.rs) | HiRoute read/Continue authorization and physical native retention are different. Borrowed history is never a cleanup target. |

## State and lifetime

The private task root stores the context descriptor, exact-session binding and
Codex alias catalog. Private run roots store temporary files and token-free
launchers. Neither root grants ownership of the borrowed HOME/config tree.
Continue reads the original descriptor instead of today's service environment.
Claude requires its exact transcript to be published before successful completion;
Codex and Qoder restoration are confirmed by the native ACP load operation,
without scanning the user's history store.

Old Codex/Claude tasks without a borrowed-context descriptor keep their existing private-root
recovery and cleanup behavior. This one recovery reader is registered in the
[compatibility registry](../../contracts/compatibility-support.v1.json); corruption
or a partially missing new binding must not be interpreted as an old task.

Permission policy and resource discovery are separate. Claude exposes Skill in
the approve-all profile and reads user/project/local settings. Restricted profiles
keep their narrower tool sets. Ambient MCP and ordinary hooks have explicit
Claude overrides; that is not a promise that all plugins, managed hooks or Codex
extensions are universally enabled or disabled. Native permissions run under the
same OS account and do not make HOME a security sandbox.

## Recover an unconfirmed Codex initializer

A failed or cancelled initialization retains its exact process identity until
the existing owned-stop path confirms the scope stopped without unknown residuals.
Later starts against that root return `Busy` after acquiring the bounded startup
gate; they do not wait indefinitely or silently retry a prompt. Other roots and
Claude do not acquire this gate. A successful version-checked initialize releases
it immediately, before native session creation or restoration.

Retry cancellation of the original run through the existing Worker Cancel action.
The [public handler](../../crates/daemon/src/control/runtime/delegation_tasks.rs)
wakes cancellation even when the receipt is replayed. Once the active lifecycle
has detached, the [dispatcher](../../crates/daemon/src/delegation/dispatcher.rs)
calls [owned stop](../../crates/daemon/src/delegation/lifecycle.rs) for that run's
persisted process identity. Only verified stop of that exact object clears its
unconfirmed startup occupancy. No automatic periodic retry is promised here.

If the original process binding could not be persisted, or its stop still cannot
be verified, cancellation cannot manufacture the missing ownership evidence.
Acknowledging residuals is not a stop receipt and does not clear this gate. A
daemon restart discards this in-memory coordination state; it does not prove old
processes stopped and does not remove the existing residual-cleanup obligation.
This coordination cannot serialize another daemon or a user's independent native
CLI, and creates no lock files in the borrowed root.

## Read the tests as product documentation

Start with [Worker native acceptance](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md).
The `worker_native_context` target has separate core and boundary journeys for
Codex and Claude. [Qoder task routing](../../tools/product-e2e/tests/QODER_DELEGATION.md)
has its own explicit-login target, including the real main-Agent user Skill →
public CLI → Worker path; a preinstallation probe cannot substitute for that path. Its case map identifies their product assertions and the component
contracts they do not execute; an ignored test or an unexecuted outcome is not a
successful product check.

| Behavior to understand | Representative executable evidence |
| --- | --- |
| Select one executable Qoder CLI without adapter placeholders; reject invalid components and preserve the exact confirmed selection | [Native installation boundary tests](../../apps/desktop/src-tauri/src/bridge/worker_dependencies_tests.rs), alongside the React `qoder.installation.single-cli` scenario in [Desktop tests](../../apps/desktop/tests/README.md). The native tests also retain window ownership, one-shot use, replacement and expiry guarantees. |
| Enable or restore a main Agent's collaboration without configuring its models; preserve Skill ownership and report drift | [Independent collaboration facts](../../crates/daemon/src/control/runtime/settings_facts/qoder.rs), [public settings transaction scenarios](../../crates/daemon/src/control/runtime/settings_entry_qoder_tests.rs) (historically registered beneath native_model tests; these are collaboration tests) |
| Upgrade an existing CLI selection while an operation is staged, activated or complete; replay and conditionally restore its owner | [Original producer fixture and provenance](../../crates/local-storage/src/migrations/fixtures/README.md), [production startup upgrade scenarios](../../crates/local-storage/src/migrations/worker_dependencies_v24_tests.rs) |
| Discover and invoke user/project skills; Continue the same session after restart and replacement of the Plan's source/model | [Real installed native Worker entry](../../tools/product-e2e/tests/worker_native_context.rs), [core journey](../../crates/daemon/tests/support/native_context_product.py) |
| Route simultaneous tasks independently in one native context; cancel one native tool while its neighbor completes; refuse Continue when its exact transcript is missing | [Boundary journey](../../crates/daemon/tests/support/native_context_boundaries.py), [boundary assertion guide](../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md) |
| Keep native tool summaries and automatic compaction on the frozen route, then Continue the same compacted session | [Qoder auxiliary-route journey](../../crates/daemon/tests/support/qoder_compaction_product.py), [version-pinned evidence limits](../../tools/product-e2e/tests/QODER_DELEGATION.md) |
| Token-free native startup, shared settings unchanged, restricted tools preserved | [Profile tests](../../crates/daemon/src/delegation/profile/tests.rs), [native context tests](../../crates/daemon/src/delegation/profile/native_context.rs) |
| Exact history identity and borrowed/owned retention | [History contracts](../../crates/daemon/src/delegation/profile/native_history_tests.rs), [material lifetime tests](../../crates/daemon/src/delegation/profile/materials_tests.rs) |
| Model selection must succeed before prompt | [ACP boundary](../../crates/daemon/src/delegation/acp/mod.rs) and its model/session tests |
| Cold starts share only their initialize window; waiting respects cancel/deadline and fresh authority; exact cancellation recovery clears an uncertain initializer | [Lifecycle startup journeys](../../crates/daemon/src/delegation/lifecycle/tests/startup.rs), [identity and gate contracts](../../crates/daemon/src/delegation/lifecycle/startup/tests.rs) |
| Revoke the exact run credential and stop owned processes through precise failure branches | [Journal contracts](../../crates/daemon/src/delegation/persistent_journal_tests.rs), [local process boundary](../../crates/daemon/tests/local_worker_platform.rs), [separate lifecycle journey](../../tools/product-e2e/tests/worker_delegation.rs) |
| Read/Continue must reject hidden or expired task content; maintenance preserves borrowed history and neighboring task metadata | [Read/Continue journey](../../tools/product-e2e/tests/worker_read.rs), [real-store maintenance tests](../../crates/daemon/src/control/runtime/delegation_maintenance_tests.rs) |

Before a real-Agent run, use the acceptance guide's
[cheap preflight contracts](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md#cheap-preflight-contracts).
For Qoder, first read the [run contract](../../tools/product-e2e/tests/QODER_DELEGATION.md#run-contract)
to distinguish borrowed login, fixture-owned materials and dedicated model-setting
writes. Each has a separate cleanup owner; native HOME is not a Product temporary root.
The [fixture checks](../../scripts/test-native-context-product.py) exercise actual
SSE wire discriminators/order, private newly created directory ancestors and the
selected native client's context budget. The
[boundary oracle checks](../../scripts/test-native-context-boundaries.py) reject
cross-routing, fabricated child completion and replacement native history.
These checks validate the test inputs and verdicts without claiming native-client
or Desktop behavior; keep them separate from the product assertions above.

Use the [test planner](../../scripts/test-plan.py) for affected checks and explicit
real-Agent commands. The normal Rust suite compiles ignored native targets but
does not execute them. Linux headless and macOS Desktop use real daemon, Gateway,
CLI/adapter and native Agent processes; a controlled upstream establishes routing
and tool/history behavior, not live-model answer quality. Keep assertion-specific
low-level tests when they cover ownership, credentials or cancellation more
precisely than a broad product scenario.

Changes to the shared Worker installation request or its native consumers require
the configured macOS Desktop compile/test gate with `desktop-pilot --all-targets`
and the same-candidate frontend directory. Use
[validation.py](../../scripts/validation.py) to select the configured build host.
Pilot includes `desktop-runtime`; without runtime the native bridge is excluded. Node, Vite, browser fixtures
and Linux backend checks cannot establish that the typed Desktop consumer compiles.
The installation unit-test filter is `bridge::worker_tasks::tests`; real Desktop
selection and task execution remain separate Pilot acceptance evidence.

## Adding an ecosystem

First establish native launch form, resource/config precedence, exact restore
semantics and history ownership. Keep client-specific behavior in the profile or
native integration leaf; share the existing admission, lifecycle, Gateway and OS
launcher. A native ACP executable must not be forced into a fictitious Node
adapter shape. A client that exposes resume instead of load needs an explicit
restore capability, not a hidden new-session fallback.

Main-Agent provider scanning/Preview/Apply, Worker execution, subscription model
discovery and calling HiRoute tools are separate capabilities. Confirm each
through its production entry. Main-Agent settings reuse the common interaction
owners in the [Agent feature guide](../../apps/desktop/src/features/agents/README.md);
native protocol differences alone do not justify new user verification steps or
status badges. Future System One/Jev providers and model/tool
selection belong to the [decision foundations](decision-foundation.md); this
Worker change does not implement built-in Jev.

The implementation retains
HiRoute's explicit environment and ownership contracts rather than importing an
entire daemon environment or copying a user's configuration tree.
