# Worker delegation

## Purpose

A main Agent or local user explicitly delegates a task to a selected coding Harness.
The daemon owns durable task/run admission, frozen routing, observation, cancellation,
and exact native-session continuation.

## Requirements

### Requirement: Supported Harnesses share one lifecycle

HiRoute MUST use the same stable supported Harness order in plan editing and executor
availability: Codex CLI, Claude Code, Qoder CLI, Pi, DeepSeek Harness (DSH), and OpenCode. Native differences do not duplicate admission, run authority, result, or
retention workflows. Installation, protocol, policy, and Continue capabilities are
separate; being listed cannot promise every action is available.

An explicitly published enabled AgentPlan with delegation on and one Harness is
required for new work. A main Agent may retain its own model provider and use only
collaboration. A fixed single-model plan is sufficient, and ordinary model calls
never start an Agent process. Goals, needed context/files, constraints, and acceptance
criteria are explicit; a Worker cannot assume the complete main conversation.

#### Scenario: Native-only OpenCode installation

- **WHEN** OpenCode is selected as the execution Harness
- **THEN** installation selection MUST require its native CLI only and retain independent saved choices for other Harnesses; no fictitious ACP adapter or Node field is required.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258607139

### Requirement: Local submissions require product intent, not caller credentials

A local caller able to use trusted CLI/Local Control may submit stable Plan, cwd,
and text/file/stdin input without caller Agent registration, allowlist, permit,
sealed collaboration grant/generation, capability descriptor, or internal digest.
The backend validates current plan publication/enabled/delegation state, request
format, execution bounds, and required dependencies. Purpose/Skill text cannot
enlarge access. Model grants and run-model/control authority stay separate from
local management. cwd is execution/Continue/audit identity, not filesystem permission,
exclusive lock, or global authorization.

Known disabled/removed sources or exact Keys that leave the frozen candidate set
unusable MUST reject new work before run/task-content mutation. Continue uses the
original frozen candidates, not a newer published model list, so rejection does not
consume a previously successful task's recovery materials. Runtime still rechecks
each exact credential lease. Claude's proven effective client window must be at
least 100,000 tokens; an unknown/smaller window is unavailable before content save/
spawn, with the same reason in directory/admission. Continue checks its frozen version.

### Requirement: Submission identity prevents duplicate execution

Task and run have independent stable identities. Equivalent canonical intent under
the same instance/operation/submission key resolves the original run; the same key
with different intent conflicts. Both entry and admission transaction check replay.
A race may initially return current closed/capacity state before later original-key
recovery, but cannot double-execute. An uncertain response preserves the original
key for query/explicit retry and cannot automatically choose a new key/Harness.
[CLI submissions](../shared-client-access/spec.md) do not maintain local receipt files.

The task retains its first admitted full plan version, Harness, protocol, model
configuration, and native reasoning. New publication cannot change an existing
task or implicitly redo it. Frozen plan content is not frozen credential or price
permission. Same-input independent submissions with different identities remain
separate, regardless of plan/title/time/cwd.

### Requirement: Current run authority remains live throughout model execution

New plan listing/exec/Continue checks current lifecycle and delegation. Closing
delegation affects future admission only; accepted runs remain queryable/cancellable
and may continue under their live run token, frozen route, deadline/cancel state,
and current source/credential eligibility. Old plan/directory cannot revive revoked
new-execution rights. Tokens are scoped to exact run/plan/protocol with epoch/lease
revocation; they are not upstream secrets or reusable global management authority.

### Requirement: Capacity is durable and independent of cwd

`max_concurrent` is one setting per daemon storage root, default 10, valid 1–1000.
Save is atomic last-write-wins without revision CAS. Admission counts durable
`occupied=1` within the same transaction, including terminal runs lacking confirmed
cleanup. At capacity reject without creating a run, queueing, automatic retry,
priority/fairness policy, or extra Agent/plan/cwd caps. One task has at most one
active run, and delegation depth remains one. Equal/overlapping directories do not
prevent different task admission; callers own their concurrent file-write risks.
Lowering capacity preserves existing work and limits only later admission.

### Requirement: Native resource context is borrowed, routing is per run

Workers MUST use the product instance's effective HOME and explicit native roots
(CODEX_HOME, CLAUDE_CONFIG_DIR, QODER_CONFIG_DIR, PI_CODING_AGENT_DIR, DSH_HOME,
and the selected OpenCode HOME/XDG/config/data selectors), including
isolated Pilot contexts. Installation paths cannot choose a different user's root
or recover a hidden daily HOME. Existing user/project Skills are discovered by the
native client; directory existence or prompt echo is not invocation evidence.

Per-run overrides provide frozen route/model/policy/credentials without rewriting
shared native settings or copying the full configuration. Tool PATH includes selected
CLI/runtime parents, absolute service search directories, and platform defaults,
deduplicating and rejecting empty/relative entries; it cannot inherit the whole
environment, upstream tokens, or shell startup scripts. Private launch materials and
same-UID execution are not a sandbox. Concurrent routes keep credentials/settings/
process cleanup isolated.

#### Scenario: Borrowed resources and real tools

- **WHEN** a supported OpenCode Worker executes a coding task requiring an existing user/project Skill, read, edit and shell tools
- **THEN** the native resources are actually invoked, expected file/shell effects occur, and every admitted model request remains bound to the run and its frozen route without changing shared settings.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258611907

### Requirement: Codex startup overrides precede its native auth/catalog checks

Codex MUST select a task-specific provider and private single-alias catalog before
its native auth/catalog lookup, using frozen plan facts and the existing metadata
projection. It cannot modify shared config.toml, explicit user catalog, models cache,
or managed merged directory for Worker selection. Task provider cannot recursively
merge ambient provider auth/route fields. Start and Continue use the same task provider/
catalog, with fresh run credentials held only in environment/protected channel.
Provider uses Responses and `supports_websockets=false`, not a claim of support for
stored conversation/previous-response handles.

For concurrent cold Codex starts on the same canonical borrowed root, only the
spawn-to-successful-ACP-initialize/protocol-confirmation interval is coordinated.
Release it before New/Load/model confirmation/prompt. Wait honors the original
cancel/deadline and rechecks run permission; other roots/Harnesses remain independent.
Failed initialization keeps its exact gate until owned stop is confirmed; unresolved
process identity yields bounded failure for later same-root starts. No native SQLite
prewarming/rewriting, prompt replay, user-root lock file, or cross-process scheduler
is introduced. Coordination is daemon-local, not proof against another daemon/client.

### Requirement: Claude settings cannot override the run's model authority

Claude MUST load the native settings sources needed for user/project Skills and use
host-managed provider/session-model selection for the frozen route. Default autonomous
policy enables Skill; restricted policy cannot be expanded merely to invoke Skills.
A selected Node/private adapter bootstrap protects route/auth/context/CLI/loader
variables before importing the JavaScript ACP adapter, including protected absent
values. Settings cannot replace/delete them or the environment object. Secrets stay
out of argv, `--settings`, task metadata, and files. The guard is local to the adapter,
not propagated through NODE_OPTIONS to tools.

Local Gateway NO_PROXY/no_proxy overrides preserve the route despite borrowed
proxy settings; incompatible managed transport/TLS overrides fail without exposing
values. Ambient MCP and user/project/plugin hooks use the explicit restricted
profile settings; borrowed HOME is not a promise of all extensions. A missing
explicit Node can remain discoverable/selectable but borrowed execution fails before
an unprotected adapter is started. Its narrow native capability check is defined by
[Worker platform](../worker-platform/spec.md).

### Requirement: Qoder uses exact native model selection and proven budgets

Qoder uses selected native ACP CLI with per-run token-free provider overlay,
environment credentials, and frozen Responses/Messages protocol. Raw Plan aliases,
including `/` and `:`, map to native provider-prefixed selector only at the native
boundary. New/Load verifies the exact selector before prompt; native prefixes do
not enter Plan/authorization truth. User provider/model/auth settings remain unchanged.
Native login stays with the selected context; HiRoute cannot copy credentials or
auto-login in discovery. Unproven auxiliary model/tool or restricted permission
capability is unavailable, never direct fallback to native account/another source.

Native context uses existing frozen total/output/reasoning policy; output is the
minimum exact nonzero bound across all active candidates/protocols. Both context
and output are projected for native compaction. Unknown/unrepresentable budgets,
or nonpositive effective compaction window after output/safety reserve, reject
before spawn/model call. A positive threshold does not guarantee every prompt/Skill
fits. An independent configuration probe's synthetic budget is not Worker-plan proof.

### Requirement: Pi and DSH keep task-owned native history

Pi's selected official SDK bridge follows [Worker platform](../worker-platform/spec.md)
with borrowed resources, empty in-memory auth, frozen runtime, and exact task-owned
v3 history. Extensions/MCP and unsupported policies cannot be inferred from its
ordinary tool mode.

DSH starts selected `dsh --profile acp` with a final run-owned public patch, borrowed
HOME/DSH_HOME/project/Skill roots, and task-owned native history/attachments. Only
the frozen model route is exposed; native account/auxiliary models cannot silently
serve work. Secrets stay in environment via `apiKeyEnv`, not YAML expressions or
patch contents. On `/v1/messages` only, a unique signed live run-model token may use
DSH's native `x-api-key` carrier. Ordinary API Keys, model grants, run-control tokens,
ambiguous/duplicate/mixed auth, and other paths do not acquire that authority.

DSH Continue uses advertised public `session/resume` for the exact bound native/ACP
session and directory, checks frozen model, and uses a new run token. It does not
replay old updates, select latest history, decode/migrate DSH format, or create a
replacement session. Native client is the history interpreter. Version/composition
changes need their own real evidence, not an acceptance-pin allowlist.

### Requirement: Permission policy is native, explicit, and scoped to one run

Default `approve-all` uses each supported Harness's autonomous/no-per-tool-approval
mode. HiRoute is not a normal tool-permission manager and creates no directory/tool
capability. An unexpected ACP request may choose AllowOnce only while run is live,
uncancelled/unexpired and the option is offered; never AllowAlways or persisted
permission. `approve-reads`/`deny-all` require explicit choice and actual corresponding
native tool/sandbox constraints. A mode name alone is not proof: unsupported mapping
fails before prompt, never upgrades to approve-all. Read-only permits only native
read AllowOnce when applicable, rejects write/execute/unknown; deny-all rejects
permission requests while retaining non-tool answer/state/result/cancel. cwd is not
an OS file sandbox. Another run's policy/deadline remains unaffected.

### Requirement: Preparation and cancellation have a durable fence

Preparing checkpoint MUST commit before any process spawn. Cancelling a still-Accepted
run with no preparation/process/session/stop material or possible prompt execution
atomically records Cancelled, revokes lease, and releases occupancy. Exact cancel
replay survives restart. A stale executor revision cannot cross the preparation
fence. Supported old revision-2 Cancelling recovery may settle only with its exact
registered evidence that preparation/process/prompt never occurred; missing PID or
timeout is insufficient proof.

After preparation, cancel/deadline stops only currently owned resources with bounded
real stop/cleanup evidence. No old PID creates kill authority, no cancel rolls back
file effects, and no unknown result permits automatic task replay. Unknown residuals
remain explicit and keep occupancy where required. Terminal cleanup-unknown may be
acknowledged locally through Desktop or
`worker cleanup confirm --run ... --expected-revision ... --handled`, with exact
run/revision/idempotency and no known live process. Worker self-query/protected grants,
stale states, and known-active sessions cannot acknowledge. It records
residual_acknowledged and releases the relevant local slot, not system-proven stop,
task success, or undo. Uncertain response replays the same acknowledgement.

### Requirement: Continue restores exactly the original session

Continue MUST bind task/expected-latest-run, original canonical cwd/context/session,
frozen version/Harness/protocol, retained visible required content, and current
eligibility. Missing history/version, expired/deleted material, unsupported native
load/resume, or identity mismatch fails explicitly with no new-session fallback.
Native tool IDs/history keep their pairing without turning into Gateway credentials.

Success publication and retained native history/stop evidence/continuation materials
share a finalization boundary. Immediately after observing succeeded, Continue may
wait for that boundary but cannot fail solely because success became visible before
materials were retained. Every Continue receives a new run/admission time/credentials,
not today's replacement plan. Failed admission preserves the previous successful
task and recovery qualification.

OpenCode binds one exact native/ACP session ID and the original canonical workspace,
resource/config/data/database locations and frozen route. Continue uses native
load with fresh credentials and mode/model confirmation, including after daemon
restart. It MUST NOT replay, fork, choose newest history, migrate/repair opaque
history or create a replacement session on failure. Borrowed history remains outside
HiRoute recursive cleanup. Root exit, ACP cancel acknowledgement or closed stdio
alone cannot prove the owned tool chain and loopback listener stopped; existing
process-group/Windows Job finalization and uncertain-cleanup rules apply.

#### Scenario: Exact Continue after restart

- **WHEN** a completed OpenCode task is continued after the HiRoute daemon restarts and current defaults or Plan publication have changed
- **THEN** the original workspace/resource context/session and frozen route are used with a new credential; newly published defaults do not create a replacement conversation.

#### Scenario: Missing or unusable history

- **WHEN** the bound OpenCode history is absent, inaccessible, incompatible or cannot be loaded by the selected native client
- **THEN** Continue fails explicitly without session/new, newest-session selection, history mutation or replay of previous work.

#### Scenario: Concurrent cancellation

- **WHEN** one of two independent OpenCode Worker runs is cancelled while a native shell tool is active
- **THEN** the cancelled run stops its owned execution and listener with actual cleanup evidence, while the neighboring task retains its session, credentials and progress.

#### Scenario: Content retention

- **WHEN** HiRoute hides or expires an OpenCode task body while native user history remains present
- **THEN** existing visibility/Continue policy remains authoritative and cleanup does not delete or reimport the borrowed native history.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258613091

### Requirement: Task projections are durable and consistent across clients

Instance task list is newest-latest-admission first with exclusive cursor, bounded
limit, and no fake total or caller/cwd partition. Title is explicit or mechanically
extracted from initial Goal, may repeat, and does not change on Continue. Exact full
title filtering happens before backend pagination. Initial body deletion/expiry/
missing state hides title/brief without reconstructing it from result/Continue/
native history; task identity/history remains. List/detail use one DTO with frozen
Harness, canonical cwd, policy, deadline, ordinal, continued_from, and visibility.

created_at remains stable; first-run admission can derive from task creation,
Continue stores its actual new time. Unprovable legacy continuation time is null/
legacy_unavailable, never current time. Visible task detail refreshes latest run
including after an earlier terminal run; hidden/switched pages stop old observation,
stale replies cannot replace the current task, and an explicit historical selector
cannot be changed into latest. Polling does not resubmit. Wait expiry/observer close
is not task cancel. Terminal state and unavailable result body remain separate.

Negotiated error-severity terminal sessionFailure in ACP PromptResponse is an
authoritative failed run even when RPC returned successfully; warning alone is not
failure. Invalid extension kind/version fails closed. Codex adapter must advertise
required AIR sessionFailure at initialize before New/Load/prompt; absence gives a
fixed marked `[HiRoute]` actionable explanation in progress, without native text/
paths/secrets, and normal owned cleanup rather than a software-version allowlist.

### Requirement: Progress is a bounded best-effort public-text stream

Read exposes saved assistant text while a turn is still running, after short-cycle
batching, in a rolling per-run window with bounded per-run/aggregate buffering.
It contains no user input, tool parameters/output, or reasoning. Fixed clearly marked
host explanations may appear but are not model Result. Progress does not change
Result, native history, Continue materials, or introduce an instance disk quota.

No cursor reads the current window; a valid cursor reads a contiguous UTF-8-safe
increment without duplicate/missing in-window data. Normal later eviction does not
invalidate an already consistent page, but privacy is rechecked. An evicted cursor
reports a gap with explicit recovery, not silent catch-up/full-log reconstruction.
Known write/buffer loss starts a new marked segment when saving resumes. Unsaved
tail may be lost; old committed visible bytes may still read successfully despite
writer failure, without claiming current capture health. Query/transport failure
cannot become successful empty output or permanently poison the whole instance.

Reliable no-cursor deleted/expired state returns null text/cursor/truncated and
has_more=false with actual task/run/state/revision. Old cursors become stale on
visibility-generation/delete/expiry changes; illegal/cross-run/out-of-window positions
are rejected. Read never refreshes TTL, advances run revision, wakes/cancels/resubmits,
or restores deleted body from another source. Empty increment/has_more=false is not
terminal state. Idle save checks do not write until progress exists.

### Requirement: Executor availability is a prerequisite, not a behavior probe

Availability MUST be independent of plans/main-Agent configuration and return all six
Harnesses with ready/unavailable/unknown plus closed reasons. Basic paths/executability
and Pi's explicitly bounded offline SDK checks establish only start-approve-all
prerequisites. Cancel-ready indicates an entry, not confirmed stop. Without a real
handshake, Continue is unknown/capability_unverified and restricted policy is unknown/
restricted_policy_unverified. It cannot install, mutate configuration, grant access,
fingerprint programs, or simulate New/Load/Cancel.

Passive discovery reads bounded directories/file/package metadata from the actual
service environment, including supported global/npm prefix/cache locations. Multiple
candidates require explicit choice, including one candidate; disappearance retains
the saved choice and reports absence without auto-switch/download/change monitoring.
Partial scan/permission/budget limits are not a complete no-install result. Selection
is instance+Harness shared and affects later launches, not running tasks. Installation
UI belongs to plan editing; other pages navigate there. Core ready never runs Worker
behavior probes, and Worker-only failures cannot block model routing/control/settings.

Supported legacy tasks convert at the boundary with original identity/state/exact
version/continuation. Unregistered obsolete digest/profile records are isolated by
task rather than blocking startup, and may conservatively retain durable occupancy.
Real Exec/Continue reports actual spawn/handshake/model/mode/load/timeout stages.
Environment variables or a successful installation query are not execution evidence.

OpenCode Worker/Continue format admission is a separate bounded capability. Its
initial supported producer is the official npm platform package for 1.19.3, identified
through adjacent package/layout/platform metadata without launching a shim or
fingerprinting a binary. Standalone, unknown or ambiguous installations remain
unavailable for this action without disabling independent model/Skill configuration.

#### Scenario: Separate capabilities

- **WHEN** OpenCode model configuration is supported but native ACP or Continue capability is unavailable
- **THEN** model configuration remains usable and Worker/Continue availability is reported independently, without claiming model success from discovery.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258607139

### Requirement: OpenCode Worker execution binds native ACP to one frozen route

HiRoute MUST execute an explicitly selected OpenCode native ACP installation through its existing Worker admission, run-authority and execution lifecycle. A run MUST use the frozen Plan alias, selected Responses/Messages protocol, exact native model selector and fresh run credential. Both new and restored sessions MUST confirm the exact model through typed ACP config options before prompt intent; missing or contradictory model capability fails explicitly without native-provider fallback. Native resource roots, project rules and user/project Skills are borrowed; per-run settings MUST NOT rewrite daily configuration or copy ambient upstream credentials. All admitted main, small/title and compaction model calls MUST stay on the frozen route and use the Plan-derived native context/output budget. Effective project/managed settings and native extensions MUST NOT override routing, authentication, budget or permissions; an unenforceable composition fails before model work. The initial Worker surface supports native Skill/read/search/edit/shell under approve-all; native nested delegation, external MCP and executable plugins/custom tools are excluded, and unsupported restricted policies MUST NOT be upgraded silently. Native terminal errors and cancellations MUST NOT become successful assistant results. The ACP-internal HTTP listener MUST stay local, protected as necessary for per-run ownership, and within the same owned lifecycle; no separately managed persistent server is introduced.

The selected native context freezes HOME, XDG roots, optional absolute OPENCODE_CONFIG, additive OPENCODE_CONFIG_DIR and the exact database selector. Supported existing history is admitted by a bounded read-only native migration-ID set check before startup, without decoding or migrating conversations. Missing history may be initialized for new work only. Typed primary mode selection MUST complete before final model confirmation from the latest config options. Unknown late configuration, native schema/conversion writes, organization/managed sources or executable custom composition fail before launch.

Concurrent OpenCode starts sharing one native database MUST coordinate only native initialization and final read-only history/composition admission. Release that coordination after successful ACP initialize, before New/Load or prompt, and preserve the existing deadline, cancellation, live-authority and uncertain-cleanup boundaries. Do not prewarm, repair or migrate history or serialize whole task execution.

#### Scenario: Exact model before work

- **WHEN** a new OpenCode ACP session reports a different current model than the task-selected provider/alias
- **THEN** HiRoute selects and confirms the exact expected model before prompting, or fails the run without model work if that cannot be established.

#### Scenario: Borrowed resources and real tools

- **WHEN** a supported OpenCode Worker executes a coding task requiring an existing user/project Skill, read, edit and shell tools
- **THEN** the native resources are actually invoked, expected file/shell effects occur, and every admitted model request remains bound to the run and its frozen route without changing shared settings.

#### Scenario: Auxiliary routing and compaction

- **WHEN** native title/small-model work or context compaction occurs during an OpenCode Worker task
- **THEN** those model requests use the frozen authorized route and native budget rather than ambient provider defaults or another model account.

#### Scenario: Unenforceable overrides

- **WHEN** project or managed configuration or an enabled executable extension conflicts with the required route, credential, tool or budget boundary
- **THEN** the affected Worker action reports a scoped capability/configuration failure before model work, without disabling unrelated Agent configuration capabilities.

#### Scenario: Native provider failure

- **WHEN** OpenCode reports a model authentication or inference failure through its ACP response
- **THEN** the run records failure with bounded safe diagnostics instead of accepting error-shaped assistant prose or an ordinary success result.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258611907

## Implementation references

- [Admission](../../../crates/daemon/src/control/runtime/delegation_tasks.rs),
  [exact Continue](../../../crates/daemon/src/control/runtime/delegation_continue.rs), and
  [preparation/cancel storage scenarios](../../../crates/local-storage/src/runtime/delegation/tests.rs).
- [Native profile ownership](../../../crates/daemon/src/delegation/profile/README.md),
  [executor](../../../crates/daemon/src/delegation/executor.rs), and
  [executor availability](../../../crates/daemon/src/delegation/installation/availability.rs).
- [Real Worker journey](../../../tools/product-e2e/tests/worker_delegation.rs) and
  [progress/Continue journey](../../../tools/product-e2e/tests/worker_read.rs).
