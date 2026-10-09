# Agent integrations and Worker execution

[Code map](README.md) · [Architecture](architecture.md) · [Test map](testing.md)

Discovery, model import, saved model routes, task collaboration and Worker execution
are separate capabilities. They share configuration and lifecycle owners; native
leaves supply only the ecosystem differences. A successful local check establishes
a prerequisite, not a model call, installed Skill invocation or resumable task.

## Agent configuration

| Capability | Shared owners | Native detail / evidence |
| --- | --- | --- |
| Scan, prepare and import model sources | [Source admission map](architecture.md#model-sources-and-native-configuration), [model connection assembly](../../crates/daemon/src/control/runtime/model_connections.rs) | [Pi acceptance](../../tools/product-e2e/tests/PI_INTEGRATION.md), [DSH acceptance](../../tools/product-e2e/tests/DSH_INTEGRATION.md) |
| Configure, edit and restore models | [Settings transaction](../../crates/application/src/agent_connection/settings), [typed facts](../../crates/daemon/src/control/runtime/settings_facts), [Desktop Agent feature](../../apps/desktop/src/features/agents/README.md) | [Additional native provider guide](../../crates/integrations/src/agents/additional_native/README.md), [saved-model journey](../../crates/daemon/tests/support/additional_model_product.py) |
| Enable task collaboration independently | [Skill effects](../../crates/application/src/agent_connection), [local checks](../../apps/desktop/src-tauri/src/agent_check_session.rs) | [Real main-Agent → public CLI → Worker journey](../../crates/daemon/tests/support/collaboration_product.py) |

Qoder/Pi/DSH saved models use one owned provider per Plan with an explicit
Responses/Messages choice. [Route grants](../../crates/domain/src/agents/model_grant.rs)
and [Gateway trust](../../crates/gateway/src/publication/trust.rs) preserve alias,
protocol and receipt authority across publication/restart. `work.protocol` is the
independent frozen Worker choice. Native defaults and collaboration stay independent;
restoration must verify ownership rather than infer it from a provider name.

## Follow a task through its owners

| Step / question | Production owner | Boundary to preserve |
| --- | --- | --- |
| Select and confirm an installation | [Desktop preparation](../../apps/desktop/src-tauri/src/bridge/worker_tasks.rs), [protected confirmation](../../apps/desktop/src-tauri/src/session/worker_dependencies.rs), [launch type](../../crates/domain/src/delegation/installation.rs), [storage](../../crates/local-storage/src/control/worker_dependencies.rs) | Exact canonical components, window-bound single-use confirmation and durable selection; discovery is not authority |
| Publish the machine-readable request shape | [Worker producer](../../crates/application-api/src/generated/worker.rs), [Plan producer](../../crates/application-api/src/generated/plan_authoring.rs), [CLI generator](../../crates/cli/src/bin/generate_cli_contract.rs) | Typed producer sources generate [CLI contracts](../../contracts/cli); retain native-only forbidden fields and adapter requirements |
| Submit and admit work | [CLI receipts](../../crates/cli/src/worker/receipt.rs), [daemon admission](../../crates/daemon/src/control/runtime/delegation_tasks.rs), [Application](../../crates/application/src/delegation) | A local idempotency conflict is distinct from daemon admission; freeze intent, Plan and task identity |
| Resolve resources and construct a run | [Installation](../../crates/daemon/src/delegation/installation.rs), [native context](../../crates/daemon/src/delegation/profile/native_context.rs), [profile guide](../../crates/daemon/src/delegation/profile/README.md) | Effective instance HOME/config root supplies resources; per-run overrides supply route, credential and permission policy |
| Project native context/output budgets | [Shared budget policy](../../crates/domain/src/agents/additional_model.rs), [native projection](../../crates/integrations/src/agents/qoder_budget.rs) | Preserve the frozen Plan's active-candidate/protocol bounds; Gateway output capping alone cannot prove native compaction limits |
| Admit and attribute model requests | [Run authority](../../crates/daemon/src/delegation/run_authority.rs), [Gateway dispatch](../../crates/gateway/src/dispatch/run.rs) | Sealed Plan, live run, harness, protocol and observation identity must agree |
| Confirm the native session/model | [ACP model confirmation](../../crates/daemon/src/delegation/acp/model.rs), [ACP driver](../../crates/daemon/src/delegation/acp/mod.rs) | Confirm the exact native model before prompt; missing capability cannot silently change the route |
| Continue the same task | [History binding](../../crates/daemon/src/delegation/profile/native_history.rs), [resume](../../crates/daemon/src/delegation/executor/resume.rs) | Restore the exact frozen context/session with a fresh credential; never select the newest history or start a replacement conversation |
| Start, cancel and finalize processes | [Executor](../../crates/daemon/src/delegation/executor.rs), [OS launcher](../../crates/daemon/src/delegation/local_worker/README.md), [finalization](../../crates/daemon/src/delegation/finalization.rs) | Explicit argv/environment/materials and owned process identity; stopping and successful task completion are separate facts |
| Read, hide, expire or clean up content | [Reads](../../crates/daemon/src/control/runtime/delegation_reads.rs), [Continue admission](../../crates/daemon/src/control/runtime/delegation_continue.rs), [maintenance](../../crates/daemon/src/control/runtime/delegation_maintenance.rs), [native cleanup](../../crates/daemon/src/delegation/native_cleanup.rs) | Current content permission and exact cleanup ownership; borrowed history is not a deletion target |

## Ecosystem differences

| Ecosystem | Resource context | Worker transport / native owner | History and acceptance |
| --- | --- | --- | --- |
| Codex | HOME/CODEX_HOME and project Skills | Selected ACP adapter; [profile](../../crates/daemon/src/delegation/profile/codex.rs) | Borrowed native history, exact session; [native context guide](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md) |
| Claude Code | HOME/CLAUDE_CONFIG_DIR and project Skills | Selected ACP adapter with protected environment; [profile](../../crates/daemon/src/delegation/profile/claude.rs) | Borrowed transcript, exact identity and flush; [native context guide](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md) |
| Qoder | HOME/QODER_CONFIG_DIR and project Skills | Selected native ACP CLI; [profile](../../crates/daemon/src/delegation/profile/qoder.rs) | Borrowed opaque history, exact ACP load; [Qoder guide](../../tools/product-e2e/tests/QODER_DELEGATION.md) |
| Pi | HOME/PI_CODING_AGENT_DIR, project and installed package Skills | Selected npm CLI + Node, bundled SDK→ACP bridge; [profile](../../crates/daemon/src/delegation/profile/pi.rs) | Task-owned v3 transcript, validated before open; [Pi guide](../../tools/product-e2e/tests/PI_INTEGRATION.md) |
| DeepSeek Harness (DSH) | HOME/DSH_HOME, project and custom Skill roots | Selected native ACP CLI and public run patch; [profile](../../crates/daemon/src/delegation/profile/dsh.rs) | Task-owned opaque history, exact ACP resume; [DSH guide](../../tools/product-e2e/tests/DSH_INTEGRATION.md) |

The [profile guide](../../crates/daemon/src/delegation/profile/README.md) owns startup
flags, credential delivery, native tool permissions and resource restrictions.
Do not duplicate the shared admission, process or retention lifecycle for an ecosystem.

### Pi compatibility is a capability contract

Selected dependency checks allow bounded cold startup: native version reads have
a 10-second deadline and the offline Pi SDK check has 15 seconds. This is separate
from directory discovery's scan budget and from the admitted task deadline. A
timeout does not invalidate the saved selection or consume a submission key.
Failures expose a closed check stage and reason through the existing machine
error message key, with transient inspection/timeouts marked retryable. No native
output, environment value or credential is included. A dependency failure after
admission leaves the run Failed and writes an explicitly HiRoute-authored reason
to its existing progress stream, never a successful model result. The same
selection and exact Continue request can be retried after a transient check.

The existing error envelope and managed progress format remain current; this
adds no saved dependency fields, schema conversion or alternate legacy reader.

[Runtime admission](../../crates/integrations/src/agents/pi_runtime.rs) and the
[SDK contract](../../crates/integrations/src/agents/pi_sdk_contract.mjs) bind the selected
CLI's declared executable and SDK export from the same package. Production checks
interfaces required by the operation, not the acceptance release label:

- Static import and saved-model configuration do not require Worker session APIs.
- Collaboration requires the native resource loader and read/bash/public CLI path.
- New work requires provider/key binding, resources and new-session interfaces.
- Continue additionally requires native open and the exact supported transcript;
  missing history never falls back to creating a new session.

The [bridge](../../crates/daemon/src/delegation/profile/pi_worker_bridge.mjs) keeps an
empty in-memory credential store, offline package discovery and task-owned history.
It validates branch-local v3 context edits without rewriting original messages.
[Process regressions](../../crates/daemon/src/delegation/profile/pi_worker_bridge.test.mjs)
cover creation/cancel/disconnect races; [failure ingestion](../../crates/daemon/src/delegation/acp/pi_failure.rs)
maps closed failure stages without exposing native exception text. These checks do
not replace real native compaction, Continue or concurrent-route journeys.

### DeepSeek Harness

DSH uses public CLI/configuration and native ACP; there is no SDK import or native
history decoder. [Static source/configuration rules](../../crates/integrations/src/agents/additional_native/README.md#dsh-static-composition)
belong to the native leaf. [Local capability checks](../../crates/integrations/src/agents/dsh_native.rs)
inspect public composition without model activation and keep collaboration resources
independent of model-provider edits.

The [Worker profile](../../crates/daemon/src/delegation/profile/README.md#dsh-public-composition)
adds a final run-owned patch while preserving borrowed resource roots. Continue
requires advertised `session/resume`, the exact native/ACP session ID and frozen
context/route; DSH interprets its own history. Missing or unusable history fails
without `session/new`. Custom compositions and cross-version continuation require
separate evidence; an acceptance pin is not a production version allowlist.

## State and lifetime

The task root owns context descriptors and exact-session bindings. Run roots own
temporary launch materials and credentials. Borrowed HOME/config/history remain
native-owned; Continue reads the original descriptor rather than today's service
environment. Pi and DSH history is task-owned, but permission to Continue still
comes from HiRoute's current task/content state.

Only registered old Codex/Claude tasks may use the legacy private-root recovery
path in the [compatibility registry](../../contracts/compatibility-support.v1.json).
A corrupted current binding is not legacy absence. Resource discovery and native
permission policy are separate; using one OS account does not make HOME a sandbox.

`Preparing` is committed before any native process can spawn. Cancelling an
`Accepted` run therefore atomically records `Cancelled` with complete cleanup;
a stale executor cannot cross the preparation revision check. Once preparation
has started, cancellation still needs owned-process stop evidence. Startup may
settle the old revision-2 `Cancelling` record only when the exact record proves
that preparation, process binding and prompt intent never occurred.

Codex [startup coordination](../../crates/daemon/src/delegation/lifecycle/startup.rs)
serializes only same-daemon, same borrowed-root spawn/initialize, releasing before
session creation or prompt. An unconfirmed initializer retains occupancy until
[owned stop](../../crates/daemon/src/delegation/lifecycle.rs) verifies its exact process.
Retry Cancel through the existing task action; residual acknowledgement or daemon
restart is not proof that the original processes stopped. Other roots and clients
retain independent execution.

## Representative scenarios

| Behavior | Start with |
| --- | --- |
| Installation confirmation, one-shot authority and exact saved selection | [Native bridge tests](../../apps/desktop/src-tauri/src/bridge/worker_dependencies_tests.rs), [Desktop scenario map](../../apps/desktop/tests/README.md) |
| Actual user/project Skills and exact Continue after restart or source replacement | [Native context journey](../../crates/daemon/tests/support/native_context_product.py), ecosystem acceptance guide above |
| Concurrent routes, cancellation isolation and missing-history rejection | [Shared boundary journey](../../crates/daemon/tests/support/native_context_boundaries.py), [assertion guide](../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md) |
| Native tool summarization and compaction remain on the frozen route | [Compaction journey](../../crates/daemon/tests/support/native_compaction_product.py), [Qoder native leaf](../../crates/daemon/tests/support/qoder_compaction_product.py) |
| Token-free startup, foreign settings preserved, exact history and cleanup | [Profile tests](../../crates/daemon/src/delegation/profile/tests.rs), [history tests](../../crates/daemon/src/delegation/profile/native_history_tests.rs), [material tests](../../crates/daemon/src/delegation/profile/materials_tests.rs) |
| Unconfirmed initializer recovery, credential revocation and owned stop | [Startup journeys](../../crates/daemon/src/delegation/lifecycle/tests/startup.rs), [journal tests](../../crates/daemon/src/delegation/persistent_journal_tests.rs), [platform tests](../../crates/daemon/tests/local_worker_platform.rs) |
| Hidden/expired content blocks read and Continue; borrowed neighbors survive | [Read/Continue journey](../../tools/product-e2e/tests/worker_read.rs), [maintenance tests](../../crates/daemon/src/control/runtime/delegation_maintenance_tests.rs) |

Use the [test map](testing.md) and selected ecosystem run guide for inputs, writable
roots, required outcomes and cleanup. Shared request changes require the configured
Mac `desktop-pilot --all-targets` compile gate; ignored native targets need explicit
execution. Native bridge filters are `bridge::worker_tasks::tests`,
`session::checks::tests` and `bridge::model_connection_web::tests`; zero selected
tests prove nothing about those consumers.
