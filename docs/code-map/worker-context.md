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
| Configure, edit and restore models | [Settings transaction](../../crates/application/src/agent_connection/settings), [typed facts](../../crates/daemon/src/control/runtime/settings_facts), [Desktop Agent feature](../../apps/desktop/src/features/agents/README.md) | [Additional native provider guide](../../crates/integrations/src/agents/additional_native/README.md), [saved-model journey](../../crates/daemon/tests/support/additional_model_product.py), [Windows ordinary Pi/DSH clients](../../crates/daemon/tests/support/windows_user_models.rs), [Windows Qoder official-account journey](../../crates/daemon/tests/support/windows_qoder_user_models.rs) |
| Choose an Agent for a model trial | [Choice/detail reads](../../crates/product-session/src/commands/trial_agents.rs), [flow lifetime](../../apps/desktop/src/features/trial/useTrialAgents.ts), [model projection](../../crates/daemon/src/control/runtime/settings_status.rs) | [Trial read races and mapping refresh](../../apps/desktop/tests/v3/visual/trial-agent-read-checks.mjs), [strict model/full query control](../../crates/daemon/src/control/runtime/settings_projection_tests.rs) |
| Show recorded Agent settings on Home or route associations | [Home and visible-route reads](../../crates/product-session/src/commands/agent_home.rs), [metadata-only summary](../../crates/daemon/src/control/runtime/settings_summary.rs), [Home projection](../../apps/desktop/src/product/home-projections.ts) | [Home states and fan-out](../../apps/desktop/tests/v3/browser/home-startup-scenarios.mjs), [session and authority fences](../../crates/product-session/src/commands/agent_snapshot/tests.rs) |
| Preview a trial's DSH model limits | [Editor option orchestration](../../crates/application/src/client_access/editor_options.rs), [Responses ingress and token budget](../../crates/daemon/src/control/runtime/routing.rs) | [Frozen publication equivalence](../../crates/daemon/src/control/runtime/routing_dsh_preview_tests.rs), [real daemon editor options](../../crates/daemon/tests/support/route_options_product.py) |
| Enable, edit or disable task collaboration independently | [CLI intent workflow](../../crates/cli/src/collaboration.rs), [Skill effects](../../crates/application/src/agent_connection), [shared session checks](../../crates/product-session/src/agent_check_session.rs) | [CLI workflow regressions](../../crates/cli/src/collaboration/tests.rs), [real main-Agent → public CLI → Worker journey](../../crates/daemon/tests/support/collaboration_product.py) |

Qoder/Pi/DSH/OpenCode saved models use one owned provider per Plan with an explicit
Responses/Messages choice. [Route grants](../../crates/domain/src/agents/model_grant.rs)
and [Gateway trust](../../crates/gateway/src/publication/trust.rs) preserve alias,
protocol and receipt authority across publication/restart. `work.protocol` is the
independent frozen Worker choice. [Publication budget guards](../../crates/daemon/src/control/runtime/additional_model_budget.rs)
report the actual installed consumers; [safe conflict facts](../../crates/domain/src/agents/model_budget_conflict.rs)
retain rejection identity through compensation and client observation.
Native defaults and collaboration stay independent;
restoration must verify ownership rather than infer it from a provider name.
[Native routing resources](../../crates/integrations/src/agents/routing_adapters.rs)
use the same protected artifact journal as model settings. The
[request-purpose boundary](../../crates/gateway/src/context_hold/routing.rs) admits
main/child history and detaches known auxiliary calls; [listener regressions](../../tools/e2e-harness/tests/p0_gateway_runtime/native_history.rs)
cover interleaving and semantic compaction independently of provider affinity.
Pi's [provider adapter](../../assets/agents/pi/hiroute-routing.mjs) binds native
compaction/tree-summary signals at the public provider seam and retains branch
cursors as observed evidence. Its [native guide](../../tools/product-e2e/tests/PI_INTEGRATION.md#ordinary-routing-metadata)
separates installed SDK fixtures, actual CLI acceptance and Restore inference.
Claude's native [identity leaf](../../crates/gateway/src/context_hold/identity/claude.rs)
keeps session/agent scope separate from its [request-class and prompt leaf](../../crates/gateway/src/context_hold/routing/claude.rs);
known auxiliary classes survive optional enrichment failure, while unknown classes
retain complete observed input and normal routing.
The [native continuity owner](../../crates/gateway/src/context_hold/continuity.rs)
separates known continuations from usable observed input. Missing origin proof
retains classification, actual capture and evaluation in an uncertain lane without
replacing verified task history. Native identities remain scoped independently;
Qoder's shared main/child identity has an accepted association limit. Follow the
[routing contract](../../issue-spec/specs/request-routing/spec.md#requirement-native-routing-remains-available-without-perfect-input-provenance)
for recovery limits rather than equating successful output with full adaptation.

The [v2 input consumer](../../crates/gateway/src/context_hold/routing/v2.rs) keeps
an omitted optional input descriptor distinct from an invalid declaration.
[Input projection](../../crates/gateway/src/core_runtime/native_routing_view.rs)
reports the former as correlation only without selecting a human window. DSH's
late model-selection notification is covered by the
[producer controls](../../scripts/tests/dsh-routing-adapter.test.mjs) and
[projection controls](../../crates/gateway/src/core_runtime/native_routing_view/dsh_tests.rs);
the unchanged wire request remains available to normal routing and capture.
The DSH producer recognizes native runtime, skill-catalog and time-context
supplements, and validates both retained instruction roles and older User-role
projections against the complete final User scope. Unknown producers still lose
the optional input proof. Native failed-request retry association belongs to the
[shared history owner](../../crates/gateway/src/agent_turn_history/store/retry.rs),
not to the DSH adapter or a session-wide lock.

Codex's [connection guidance](../../apps/desktop/src/features/agents/CodexAccessPanel.tsx)
explains native provider-scoped session discovery before enabling routing. Default
on-demand access preserves the ordinary entry for earlier sessions; advanced default
takeover first requires configuration restoration. See [session recovery](../local-user-configuration.md#find-earlier-codex-sessions). Worker Continue below
has its own exact-history binding and does not replace the native `/resume` picker.
OpenCode's [startup target binding](../../crates/daemon/src/control/runtime.rs) keeps
the physical JSON/JSONC file verified by existing recovery markers;
[effective model facts](../../crates/integrations/src/agents/opencode_scanner.rs)
also feed the shared configured/drift status.
Its ordinary [routing producer](../../assets/agents/opencode/hiroute-routing.mjs)
freezes per-call purpose/session before a bounded optional SDK page; exact native
page positions provide compression references while synthetic continuations keep
unavailable-input semantics. The [OpenCode guide](../../tools/product-e2e/tests/OPENCODE_INTEGRATION.md#ordinary-routing-metadata)
separates hook regressions from native dependency initialization, recovery and
Restore inference; a hook deadline cannot bound the native loader that precedes it.
The persisted OpenCode metadata fallback is decided by
[additional-model facts](../../crates/daemon/src/control/runtime/settings_facts/additional_model.rs)
before writes and uses the existing model Restore/Apply journal. Its
[settings regressions](../../crates/daemon/src/control/runtime/settings_entry_opencode_metadata_tests.rs)
cover restart, re-enablement, borrowed resources and stale previews; it changes
metadata precision without changing model authorization or Gateway routing.
Its [bounded Markdown reference reader](../../crates/integrations/src/agents/opencode_resources.rs)
adds native user Agent/Mode selectors to the same settings dependency checks;
removal requires complete references, while retaining selectors permits grant rotation.
Worker project ancestry uses the [native Git boundary check](../../crates/integrations/src/agents/opencode_project.rs)
with the filtered child HOME/XDG/PATH and shared bounded probe process. Both configuration
and resource inspection require a confirmed Git stop or the filesystem root within the
existing bound; an invalid marker or unavailable Git cannot truncate admission.
The shared [probe process owner](../../crates/integrations/src/agents/native_probe_process.rs)
retains an exited Unix leader until group signals finish. Only a still-running
leader receives termination grace; completed probes immediately stop surviving
group members and still require reap and proven group absence.

Hermes uses an [explicit effective profile](../../crates/integrations/src/agents/hermes/README.md)
and one default main-model Plan. Desktop keeps Hermes in the common Agent list;
its native host resolves the caller's existing default profile and its details can
select another through the same target-read flow before settings become available.
The common [discovery preference](../../apps/desktop/src/features/agents/discovery.ts)
remembers explicit targets as hints, with backend revalidation and per-profile failure isolation.
Startup [protected-input priming](../../crates/daemon/src/control/runtime/mutation.rs)
reads exact native configuration descriptors without running informational version probes;
explicit Agent inventory retains full discovery and later secret reads recheck source revisions.
Its [native request middleware](../../assets/agents/hermes/__init__.py) carries the real
session into trusted Gateway correlation and emits observed batch context
independently from optional bounded read-only compression evidence;
clean callback text and retained legacy hints cannot select an input instance.
Gateway [routing evidence](../../crates/gateway/src/context_hold/routing.rs) validates
compression scope independently of task-envelope parsing, so an adapter fault and
recovery cannot strand otherwise comparable captured history.
The managed provider also uses the same
scoped model grant as its native API key for auxiliary Chat requests; global model
headers are not a credential carrier. All six supported Agents have independent management and collaboration Skills;
[shared Skill facts](../../crates/daemon/src/control/runtime/settings_facts/skills.rs)
resolve their native roots through the existing ownership/Operation engine. Legacy
primary roles retain their original resource meanings; the second Skill has a separate
registered effect and per-facet restore reference. Hermes initiates delegation through
the existing local Worker CLI and does not become a Worker executor.

## Follow a task through its owners

| Step / question | Production owner | Boundary to preserve |
| --- | --- | --- |
| Select and confirm an installation | [Desktop preparation](../../apps/desktop/src-tauri/src/bridge/worker_tasks.rs), [protected confirmation](../../crates/product-session/src/session/worker_dependencies.rs), [launch type](../../crates/domain/src/delegation/installation.rs), [storage](../../crates/local-storage/src/control/worker_dependencies.rs) | Exact canonical components, window-bound single-use confirmation and durable selection; discovery is not authority |
| Publish the machine-readable request shape | [Worker producer](../../crates/application-api/src/generated/worker.rs), [Plan producer](../../crates/application-api/src/generated/plan_authoring.rs), [CLI generator](../../crates/cli/src/bin/generate_cli_contract.rs) | Typed producer sources generate [CLI contracts](../../contracts/cli); retain native-only forbidden fields and adapter requirements |
| Submit and admit work | [CLI submission](../../crates/cli/src/worker.rs), [daemon admission](../../crates/daemon/src/control/runtime/delegation_tasks.rs), [Application](../../crates/application/src/delegation) | Daemon admission owns durable key/intent deduplication; CLI preserves returned locators and uses no local receipt |
| Resolve resources and construct a run | [Installation](../../crates/daemon/src/delegation/installation.rs), [native context](../../crates/daemon/src/delegation/profile/native_context.rs), [profile guide](../../crates/daemon/src/delegation/profile/README.md) | Effective instance HOME/config root supplies resources; per-run overrides supply route, credential and permission policy |
| Project native context/output budgets | [Shared budget policy](../../crates/domain/src/agents/additional_model.rs), [native projection](../../crates/integrations/src/agents/qoder_budget.rs) | Preserve the frozen Plan's active-candidate/protocol bounds; Gateway output capping alone cannot prove native compaction limits |
| Admit and attribute model requests | [Run authority](../../crates/daemon/src/delegation/run_authority.rs), [Gateway dispatch](../../crates/gateway/src/dispatch/run.rs) | Sealed Plan, live run, harness, protocol and observation identity must agree |
| Confirm the native session/model | [ACP model confirmation](../../crates/daemon/src/delegation/acp/model.rs), [ACP driver](../../crates/daemon/src/delegation/acp/mod.rs) | Confirm typed mode before the final typed model for OpenCode; confirm the exact native model before prompt; missing capability cannot silently change the route |
| Continue the same task | [History binding](../../crates/daemon/src/delegation/profile/native_history.rs), [resume](../../crates/daemon/src/delegation/executor/resume.rs) | Restore the exact frozen context/session with a fresh credential; never select the newest history or start a replacement conversation |
| Start, cancel and finalize processes | [Executor](../../crates/daemon/src/delegation/executor.rs), [OS launcher](../../crates/daemon/src/delegation/local_worker/README.md), [finalization](../../crates/daemon/src/delegation/finalization.rs) | Explicit argv/environment/materials and owned process identity; stopping and successful task completion are separate facts |
| Read, hide, expire or clean up content | [Reads](../../crates/daemon/src/control/runtime/delegation_reads.rs), [Continue admission](../../crates/daemon/src/control/runtime/delegation_continue.rs), [maintenance](../../crates/daemon/src/control/runtime/delegation_maintenance.rs), [native cleanup](../../crates/daemon/src/delegation/native_cleanup.rs) | Saved progress is independent of execution/cleanup state; current content permission and exact cleanup ownership still apply; borrowed history is not a deletion target |

## Ecosystem differences

| Ecosystem | Resource context | Worker transport / native owner | History and acceptance |
| --- | --- | --- | --- |
| Codex | HOME/CODEX_HOME and project Skills | Selected ACP adapter; [profile](../../crates/daemon/src/delegation/profile/codex.rs) | Borrowed native history, exact session; [native context guide](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md) |
| Claude Code | HOME/CLAUDE_CONFIG_DIR and project Skills | Selected ACP adapter with protected environment; [profile](../../crates/daemon/src/delegation/profile/claude.rs) | Borrowed transcript, exact identity and flush; [native context guide](../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md) |
| Qoder | HOME/QODER_CONFIG_DIR and project Skills | Selected native ACP CLI; [profile](../../crates/daemon/src/delegation/profile/qoder.rs) | Borrowed opaque history, exact ACP load; [Qoder guide](../../tools/product-e2e/tests/QODER_DELEGATION.md) |
| Pi | HOME/PI_CODING_AGENT_DIR, project and installed package Skills | Selected npm CLI + Node, bundled SDK→ACP bridge; [profile](../../crates/daemon/src/delegation/profile/pi.rs) | Task-owned v3 transcript, validated before open; [Pi guide](../../tools/product-e2e/tests/PI_INTEGRATION.md) |
| DeepSeek Harness (DSH) | HOME/DSH_HOME, project and custom Skill roots | Selected native ACP CLI and public run patch; [profile](../../crates/daemon/src/delegation/profile/dsh.rs) | Task-owned opaque history, exact ACP resume; [DSH guide](../../tools/product-e2e/tests/DSH_INTEGRATION.md) |
| OpenCode | HOME/XDG, explicit config and additive resource roots, project Skills | Selected native ACP binary; [profile](../../crates/daemon/src/delegation/profile/opencode.rs) | Borrowed opaque SQLite history, bounded format admission and exact ACP load; [OpenCode guide](../../tools/product-e2e/tests/OPENCODE_INTEGRATION.md) |

OpenCode's passive Agent and Worker discovery also inspect the official installer's
`~/.opencode/bin` directory when a GUI launch lacks its shell PATH. Agent discovery
keeps the PATH entry first; a saved explicit installation never falls back to a
different binary. This does not grant a Worker selection or native-format capability.

The [profile guide](../../crates/daemon/src/delegation/profile/README.md) owns startup
flags, credential delivery, native tool permissions and resource restrictions.
Do not duplicate the shared admission, process or retention lifecycle for an ecosystem.

On Windows, [native launch admission](../../crates/integrations/src/agents/windows_runtime.rs)
resolves known npm producers to their declared JavaScript entry and selected Node;
it never evaluates a `.cmd` shim. System shell selectors come from the actual
Windows system directory. Worker and CPA subscription-source HOME discovery use
the same account native-profile selector in the daemon control runtime,
while explicit ecosystem configuration roots retain their existing meaning.
[Managed Claude drive paths](../../crates/integrations/src/agents/managed_launch/windows.rs)
project Rust's verbatim spelling only after reparse-free native file identity
checks. Discovery and launch share that boundary; the launch retains the selected
image and trusted helper handles. UNC and device aliases remain outside this contract.
[Live checks](../../crates/daemon/src/control/runtime/agent_live_check_windows.rs)
capture output in private files and retain the process Job until all owned tools
stop. Follow those owners for shell, environment, timeout or background-session
differences rather than duplicating ecosystem lifecycle code.
OpenCode's isolated collaboration probe pins the system Windows PowerShell and
quotes the public CLI for that shell. The native tool ID `bash` does not imply
Git Bash. Its npm locator uses the shared declared-producer reader after PATH
resolution, then selects the unique native platform package without running a shim.

The Windows [Qoder collaboration probe](../../crates/integrations/src/agents/qoder_probe.rs)
retains the actual Skill directory and file before launching the client. Its
[receipt binding](../../crates/integrations/src/agents/qoder_probe_skill_diagnostic.rs)
accepts equivalent disk spellings only after native identity, captured permissions and
content checks; it rejects foreign/reparse paths, changed files and split messages.
The [HTTP peer](../../crates/integrations/src/agents/qoder_probe_http.rs) still requires
correlated native Skill and CLI results. No credential contents enter these checks.
The [private workspace projection](../../crates/integrations/src/agents/qoder_probe_workspace.rs)
binds Qoder and Git Bash to one ordinary drive spelling of the same private directory.
Its `Bash` call uses Git Bash syntax on Windows: the selected CLI's native DOS
path is quoted for Bash. The separate PowerShell tool is not that execution boundary;
see the [probe ownership guide](../../crates/integrations/src/agents/QODER_PROBE.md).

The [Codex collaboration check](../../crates/integrations/src/agents/collaboration_probe.rs)
selects the native restricted-token sandbox for its Windows read-only process.
Its independent [write rejection control](../../crates/integrations/src/agents/collaboration_probe/readonly_control.rs)
must observe the actual shell's denied write before accepting Skill reads and
execution of the public CLI; a policy rejection before shell execution is not
successful sandbox evidence.
Its [closed tool-output facts](../../crates/integrations/src/agents/collaboration_probe/tool_diagnostic.rs)
are observed before failed receipt checks return, so early rejection retains
fixed execution-header facts without logging native output or session identifiers.
Nonce-bound script checkpoints retain the entered phase, fixed language mode and
numeric failures; they cannot replace the actual denied-write receipt. Follow the
read-only control owner for the Windows parse-only gate and shell boundaries.

[CLI dependency input](../../crates/cli/src/worker/selection_windows.rs) projects
ordinary local drive paths to the existing canonical selection before transport,
retaining file and ancestor leases through the call. Already canonical requests
preserve completed-operation replay even after the selected files disappear;
Application still owns the canonical tuple, revision and durable transaction.
The shared npm reader lets callers inspect original manifest and declared-entry
names before reading or canonicalizing them. The [Pi locator](../../crates/integrations/src/agents/pi_native.rs)
uses that boundary to bind a generated shim to the package's declared entry and
manifest digest without evaluating the shim or changing borrowed ownership.
Direct Pi entries retain the [single package reader](../../crates/integrations/src/agents/pi_runtime.rs)
and its nearest-match semantics; original declared-bin names are inspected before
canonicalization, so an alias cannot disappear from the identity check.

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

[Official Desktop admission](../../crates/integrations/src/agents/dsh_desktop.rs)
uses the app's bundled public CLI and independent `profiles/desktop` settings.
The Web entry retains `profiles/web` ownership; both reuse protected additional
provider effects and bundled routing registration. Desktop discovery does not
require npm DSH, and its model support does not advertise Worker/Skill support.

DSH uses public CLI/configuration and native ACP; there is no SDK import or native
history decoder. [Static source/configuration rules](../../crates/integrations/src/agents/additional_native/README.md#dsh-static-composition)
belong to the native leaf. [Local capability checks](../../crates/integrations/src/agents/dsh_native.rs)
inspect public composition without model activation and keep collaboration resources
independent of model-provider edits.
Settings discovery reads native files without a version subprocess. The
[facts capture](../../crates/daemon/src/control/runtime/settings_facts.rs) owns one
public composition probe per capture and checks its dependencies after both native
snapshots; Preview, Apply and locked admission keep independent proofs.

The [Worker profile](../../crates/daemon/src/delegation/profile/README.md#dsh-public-composition)
adds a final run-owned patch while preserving borrowed resource roots. Continue
requires advertised `session/resume`, the exact native/ACP session ID and frozen
context/route; DSH interprets its own history. Missing or unusable history fails
without `session/new`. Custom compositions and cross-version continuation require
separate evidence; an acceptance pin is not a production version allowlist.

## State and lifetime

Codex adapters missing AIR `sessionFailure` fail during actual initialize, before
session creation/load. The [ACP owner](../../crates/daemon/src/delegation/acp/README.md)
adds an actionable `[HiRoute]` reason to the existing Worker progress window;
[lifecycle](../../crates/daemon/src/delegation/lifecycle.rs) flushes it after owned
process cleanup. This host explanation is separate from the model result.

Claude [terminal finalization](../../crates/daemon/src/delegation/lifecycle/terminal.rs)
keeps ACP live while the exact native transcript drains after result publication
and credential revocation. Terminal tool/text admission closes first; bounded
history observation controls Continue eligibility. An interrupted observation
retains the published result and finalization lease with history readiness false.
Follow the [ACP guide](../../crates/daemon/src/delegation/acp/README.md) and
[causal tests](../../crates/daemon/src/delegation/lifecycle/tests/terminal.rs) for
this ordering; installed native tools and Continue require their own acceptance.

The task root owns context descriptors and exact-session bindings. Run roots own
temporary launch materials and credentials. Borrowed HOME/config/history remain
native-owned; Continue reads the original descriptor rather than today's service
environment. Pi and DSH history is task-owned, but permission to Continue still
comes from HiRoute's current task/content state.

Only registered old Codex/Claude tasks may use the legacy private-root recovery
path in the [compatibility registry](../../contracts/compatibility-support.v1.json).
A corrupted current binding is not legacy absence. Resource discovery and native
permission policy are separate; using one OS account does not make HOME a sandbox.
The legacy scanner encodes filesystem components with `/` at the retained-path
boundary; native Windows separators do not change that portable contract.

`Preparing` is committed before any native process can spawn. Cancelling an
`Accepted` run therefore atomically records `Cancelled` with complete cleanup;
a stale executor cannot cross the preparation revision check. Once preparation
has started, cancellation still needs owned-process stop evidence. Startup may
settle the old revision-2 `Cancelling` record only when the exact record proves
that preparation, process binding and prompt intent never occurred.

The shared [startup coordinator](../../crates/daemon/src/delegation/lifecycle/startup.rs)
protects same-daemon Codex borrowed-root initialization and OpenCode database
initialization. OpenCode rechecks the captured configuration and read-only database
admission after waiting on its frozen database path. Coordination ends at successful
ACP initialize, before session creation/load or prompt; tasks remain concurrent.
An unconfirmed initializer retains occupancy until
[owned stop](../../crates/daemon/src/delegation/lifecycle.rs) verifies its exact process.
Retry Cancel through the existing task action; residual acknowledgement or daemon
restart is not proof that the original processes stopped. Other roots and clients
retain independent execution.

## Representative scenarios

| Behavior | Start with |
| --- | --- |
| Installation confirmation, one-shot authority and exact saved selection | [Shared session tests](../../crates/product-session/src/commands/worker_dependencies_tests.rs), [Desktop scenario map](../../apps/desktop/tests/README.md) |
| Actual user/project Skills and exact Continue after restart or source replacement | [Native context journey](../../crates/daemon/tests/support/native_context_product.py), ecosystem acceptance guide above |
| Concurrent routes, cancellation isolation and missing-history rejection | [Shared boundary journey](../../crates/daemon/tests/support/native_context_boundaries.py), [assertion guide](../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md) |
| Native tool summarization and compaction remain on the frozen route | [Compaction journey](../../crates/daemon/tests/support/native_compaction_product.py), [Qoder native leaf](../../crates/daemon/tests/support/qoder_compaction_product.py) |
| Token-free startup, foreign settings preserved, exact history and cleanup | [Profile tests](../../crates/daemon/src/delegation/profile/tests.rs), [history tests](../../crates/daemon/src/delegation/profile/native_history_tests.rs), [material tests](../../crates/daemon/src/delegation/profile/materials_tests.rs) |
| Unconfirmed initializer recovery, credential revocation and owned stop | [Startup journeys](../../crates/daemon/src/delegation/lifecycle/tests/startup.rs), [journal tests](../../crates/daemon/src/delegation/persistent_journal_tests.rs), [platform tests](../../crates/daemon/tests/local_worker_platform.rs) |
| Hidden/expired content blocks read and Continue; borrowed neighbors survive | [Read/Continue journey](../../tools/product-e2e/tests/worker_read.rs), [maintenance tests](../../crates/daemon/src/control/runtime/delegation_maintenance_tests.rs) |

Use the [test map](testing.md) and selected ecosystem run guide for inputs, writable
roots, required outcomes and cleanup. Shared request changes require the configured
Mac `desktop-pilot --all-targets` compile gate; ignored native targets need explicit
execution. Shared Product Session filters are `commands::worker_tasks::tests`,
`session::checks::tests` and `commands::model_connection_web::tests`; zero selected
tests prove nothing about those consumers.

The OpenCode native Windows leaf is
[`windows_opencode_business.rs`](../../crates/daemon/tests/support/windows_opencode_business.rs),
selected by `actual_windows_opencode_worker_native_history_and_cancellation` in
[`windows_headless.rs`](../../crates/daemon/tests/windows_headless.rs). It shares the
public routing/Worker receipt oracle and Job cancellation assertions with existing
Windows acceptance; it adds no alternative production lifecycle.

Model-only post-save confirmation reuses the exact model projection in
[`trial_agents.rs`](../../crates/product-session/src/commands/trial_agents.rs);
its authority fences remain independent of retained discovery hints. Unread Skills
are read when their editor opens. Codex/Hermes target-sensitive reads and explicit
selected discovery are owned by
[`agent_discovery.rs`](../../crates/daemon/src/control/runtime/agent_discovery.rs)
and the shared
[`Agent snapshot reader`](../../crates/product-session/src/session/agent_snapshot.rs).
The exact context crosses `ScanAgents`; full discovery remains explicit. Agent
settings effects are prepared once under the Application writer in
[`prepared.rs`](../../crates/application/src/operations/prepared.rs).
