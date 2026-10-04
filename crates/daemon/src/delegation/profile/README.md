# Worker native context and profile

[Worker lifecycle](../local_worker/README.md) · [Worker context map](../../../../../docs/code-map/worker-context.md)

The [installation source](../installation.rs) resolves the selected executable and
builds a profile for an admitted run. [NativeWorkerContext](native_context.rs)
reads only the effective instance `HOME` and the selected Harness's config-root
variable. It respects isolated Pilot/test instances and never searches for another
user's home from an executable path. The child receives a fresh explicit
environment; preserving command search paths does not import upstream credentials.

## State and ownership

New tasks bind their effective home, native config root and canonical workspace
once in the task-owned metadata directory. Continue reads that descriptor instead
of resolving today's environment. Only Codex/Claude accept the registered legacy
private-root path when the descriptor is absent; Qoder requires it. A damaged
descriptor must fail rather than fall back.
`TaskSessionRoot` still owns HiRoute metadata and the frozen Codex alias catalog.
The borrowed native config/history tree is never a launcher material or cleanup
target. History binding and retention belong to [materials](materials.rs) and the
existing delegation lifecycle, not these renderers.

The [profile builder](mod.rs) projects the admitted permission policy, supplies
explicit context and temporary paths, and combines one Harness renderer with the
validated native or adapter launch form. Tests and the older adapter-only loopback
installation probe explicitly request isolated contexts. They never silently use
the developer's HOME. Qoder requires the borrowed context; that older probe does
not establish its login or execution capability.

The shared validated launch form rejects Qoder adapter/Node fields and missing
Codex/Claude adapters. Qoder discovery checks metadata for both `qoder` and
`qodercli` names and the official standalone directory. It never runs the
discovered CLI or replaces a missing explicit selection with another candidate.

## Native launch contracts

- [Codex](codex.rs) uses a task-specific provider key so a user's ordinary provider
  table cannot accidentally contribute extra routing/auth fields through native
  table merging. The private per-run executable launcher passes the same managed
  settings used by `CODEX_CONFIG` as native `-c` overrides before `app-server`
  starts. This covers the adapter's early `account/read` and model-catalog lookup.
  Only the environment contains the run token; startup arguments and files refer
  to its variable name. Continue reuses the task's provider key/catalog and obtains
  a fresh launcher/token. The current launcher supports Unix; other platforms fail
  explicitly rather than executing a POSIX script as a native program.
- [Claude](claude.rs) loads user/project/local setting sources and exposes `Skill`
  in the default approve-all tool set. Its selected Node runs a private
  [adapter bootstrap](claude_adapter_bootstrap.mjs) before importing the selected
  JavaScript adapter. The bootstrap prevents managed settings from changing or
  deleting the host's routing, credential, context, CLI and runtime-loader
  environment values; protected absent provider credentials stay absent too.
  Ordinary settings variables remain available. This guard belongs only to the
  adapter process: it does not use `NODE_OPTIONS` or install a child-process
  preload, and it is not a sandbox for arbitrary JavaScript.
  Native host-managed provider protection, a custom model option and explicit
  session model maintain the selected route after the native CLI starts. Restricted
  profiles retain their existing tool sets. `strictMcpConfig` with an empty explicit
  MCP map suppresses ambient MCP; `disableAllHooks` suppresses user/project/plugin
  hooks without disabling plugin skills. Native managed-policy hooks and adapter
  callbacks remain separate. These options are not a claim that every extension
  is available or universally disabled.
- [Qoder](qoder.rs) starts the selected native CLI with `--acp`, explicit
  `--config-dir` and user/project/local resource sources. It consumes the single
  [shared Qoder renderer](../../../../integrations/src/agents/qoder.rs) for a
  token-free per-run settings file. The task-specific provider prefix belongs
  only to the native selected model ID; Plan, run authorization and Gateway retain
  the raw alias. New/Load use the shared typed ACP model confirmation before prompt.
  Default `yolo` permission applies to the explicit Read/Write/Edit/Bash/Grep/Glob/Skill
  tool set. Native Agent and other unproven model-using tools are not enabled;
  ApproveReads/DenyAll fail before launch rather than upgrading permission.
  Explicit empty strict MCP configuration and disabled hooks preserve that boundary
  without excluding user and project skills. The overlay disables native updates,
  periodic history cleanup and plan-driven model switching, and fixes supported
  model purposes to the frozen alias. Actual auxiliary routing needs native wire
  evidence; the presence of these settings is not that evidence.

Qoder's [budget projection](../../../../integrations/src/agents/qoder_budget.rs)
uses the frozen Plan's existing effective context policy, including total and
reasoning reservations, and the smallest exact output bound across all active
candidates and protocols. The shared renderer writes both `contextWindow` and
`maxOutputTokens` into the native provider model entry. Qoder 1.1.65 uses this
catalogue output value for both request limits and compaction reservation; its
ACP path does not apply the CLI `--max-output-tokens` flag as a substitute.
Unknown, zero, unrepresentable or nonpositive effective compaction budgets fail
before native launch. A positive threshold does not promise every skill or
prompt fits. The independent collaboration probe uses its own synthetic 2048
output budget without claiming a Worker Plan context. Existing Codex/Claude
budget projections are unchanged.

Read the [budget contracts](../../../../integrations/src/agents/qoder_budget_tests.rs)
for mixed-route minima, total/reasoning reservations and exact rejection edges;
the [Qoder profile test](tests/qoder.rs) checks the rendered New/Continue materials.
The real Qoder core journey separately verifies 32768-context/4096-output native
skill execution and exact Continue through the public Worker entry.

Qoder retains only the task-owned context descriptor and exact native-session
binding in HiRoute metadata. Its native history is opaque: Continue requests that
exact ACP session ID and fails if the native client cannot load it. There is no
Qoder private-root legacy reader, transcript scanner or shared-history deletion.
The native client must already be logged in under the selected context; HiRoute
does not copy authentication or inherit an ambient personal access token.

Borrowed Claude supplies only the Gateway IP as `NO_PROXY` and `no_proxy` in both
the adapter environment and the native flag settings. These two non-secret values
bypass user-configured proxies for the local model route; other native tool proxy
settings remain available. The bootstrap rejects conflicting managed proxy/TLS
environment writes before ACP starts: managed policy outranks flag settings and
the native CLI would read it again, so silently ignoring that write is insufficient.
Compatible managed bypass values are allowed. Rejection diagnostics contain no
setting values or credentials. This is a boundary for the local HTTP Gateway,
not general transport management for native tools.

Borrowed Claude execution requires an explicitly selected Node runtime and a
JavaScript ACP adapter entry. An executable/shebang adapter with no saved Node
selection can still be discovered and selected under the existing installation
contract, but this capability fails with `CapabilityUnavailable` before starting
the unguarded adapter. Select a Node runtime alongside that adapter in the Worker
installation configuration. HiRoute does not guess a runtime from PATH or inspect
the adapter's program text. Existing isolated tasks/probes retain their direct
adapter launch behavior.

At a borrowed Claude launch, the [runtime capability check](claude_runtime.rs)
executes only the selected CLI's bounded `--version` with the selected command
search path and no HOME, native config, or run credential. This narrowly requires
the known Claude 2.x host-managed-provider capability (2.1.231 or later); it is not
an installation readiness probe or a compatibility guarantee. An unsupported,
unknown, failed or timed-out response rejects this launch before model execution.

Native client effort preferences may still affect client-side behavior. They are
not the authority for the frozen Plan's upstream reasoning settings: AgentPlan
routing projects the frozen candidate configuration at the Gateway. This change
does not introduce a new effort policy.

Context discovery, skill discovery/invocation, model access, native session
restoration and process startup are separate capabilities. This module does not
add ecosystem scanning/import, general Agent launch, a new execution backend or a
plugin framework. ACP model/session verification remains in [ACP](../acp/mod.rs),
and the OS launcher owns executable material permissions and process cleanup.

## Representative product contracts

[Profile tests](tests.rs) cover token-free startup configuration, literal argument
passing for quoted paths, preserved daily settings, task catalog/provider reuse,
fresh run credentials, and Claude skills without widening restricted tools.
[Context tests](native_context.rs) cover explicit native roots, Pilot HOME and
rejection of unusable selectors. Existing isolated-root tests retain legacy
history/material lifetime assertions.

[Qoder profile tests](tests/qoder.rs) cover single-CLI launch, user/project sources,
tool boundaries, exact selected ID, fresh credentials and token-free materials.
[Discovery](../installation/discovery/tests.rs) and
[availability](../installation/availability.rs) test metadata-only single-CLI
selection without replacing an unavailable explicit choice. Native identity and
borrowed-binding tests cover Qoder alongside the existing exact-ID contracts;
they do not establish real login, history flush or auxiliary-model routing.

Run `node --test crates/daemon/src/delegation/profile/claude_adapter_bootstrap.test.mjs`
with the repository's Node version for the bootstrap boundary. The tests execute
the production bootstrap with a synthetic settings-writing adapter, prove the
unguarded regression, check child environment delivery without a propagated guard,
reject managed transport conflicts, and verify that rejected startup diagnostics
do not expose the runtime credential.

These tests do not prove a real installed adapter discovers or invokes skills.
Start with the [native context journey](../../../../../tools/product-e2e/tests/worker_native_context.rs)
and its [acceptance guide](../../../../../tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md).
The existing Worker scenarios under [product-e2e](../../../../../tools/product-e2e/tests/worker_delegation.rs)
and [Worker read/Continue](../../../../../tools/product-e2e/tests/worker_read.rs)
must exercise the selected installation on Linux and macOS, including two runs,
frozen routing, exact Continue and hidden-content rejection. Native history may
remain physically present after HiRoute hides its own content. A real Claude run
with conflicting user proxy settings must also complete with no proxy-trap
requests; bootstrap fixtures alone do not establish native flag-layer precedence.
