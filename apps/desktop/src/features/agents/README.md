# Agent settings feature

This feature presents supported Agent model routing, task collaboration and independent management Skills. It translates an
editable draft into the existing Agent settings intent; it does not discover installations,
read credentials, authorize a route, or implement file recovery. The [code map](../../../../../docs/code-map/README.md) connects these boundaries
to their production owners and executable contracts.

## Responsibilities and state

After a model or token mutation, the page uses the existing `trial_agent_model`
exact-context model projection. Its fresh file/grant/publication join and service
fences confirm the model state without repeating version diagnostics or reading
unchanged Skills. Discovery metadata remains a hint. Unread Skill state is explicit;
opening its ordinary editor performs the full scoped read before seeding the draft.
Codex access-mode/root metadata and Hermes native targets retain the full scoped
`agent_snapshot` path, as do Skill changes and explicit discovery. Other retained
Agents become unverified and are read when selected. An inactive page defers global
refresh invalidations until activation; background summaries do not scan during
save confirmation.

| Entry | Responsibility |
| --- | --- |
| [types.ts](types.ts) | WebView projections and current serialized model choices. These describe the existing native/API contract, not a second capability authority. |
| [status.ts](status.ts) | Reads the shared model/collaboration-only status union. A missing model capability is distinct from an unconfigured model or an unreadable status. |
| [discovery.ts](discovery.ts), [AgentDiscoveryFeedback.tsx](AgentDiscoveryFeedback.tsx) | Supported catalog, remembered non-secret target hints, common rescan and distinct undiscovered/unavailable states. Backend revalidation owns context and write authority. |
| [AgentSkillDetails.tsx](AgentSkillDetails.tsx) | Backend-supplied Skill scope, shared/borrowed availability, unobserved native loading and the existing CLI-entry preparation commands. |
| [ecosystems.ts](ecosystems.ts) | Explicit current Agent identity, display name, brand and surface labels. Unknown discovered identities have no settings editor. |
| [editor-state.ts](editor-state.ts) | Draft initialization, dirty comparison and local form completeness. Only a trusted first connection can preselect a sole enabled route. Existing mappings, including unavailable choices, are retained. |
| [settings-request.ts](settings-request.ts) | Ecosystem-specific intent construction, independent model/management/collaboration restore intents, token changes that retain the saved selection, and capability-based prerequisite check selection. |
| [AgentModelEditor.tsx](AgentModelEditor.tsx) | Explicit composition of [Codex](CodexModelEditor.tsx), [Claude](ClaudeModelEditor.tsx) and [supplementary route](AdditionalModelEditor.tsx) forms and their saved-selection summary. Codex allowlist/default/native retention, Claude preset mappings and supplementary routes remain different concepts. |
| [AgentSettingsFeedback.tsx](AgentSettingsFeedback.tsx) | Presentation of authoritative Preview blockers. |
| [HermesModelEditor.tsx](HermesModelEditor.tsx), [HermesRootFields.tsx](HermesRootFields.tsx) | One Plan/protocol and explicit effective profile; root fields are shared with source scan. Automatic adapter preparation and unobserved native loading stay distinct. |
| [collaboration-check-feedback.ts](collaboration-check-feedback.ts) | Versioned collaboration-check failures become safe login, Skill recovery and retry guidance; native output is never display text. |
| [CodexAccessPanel.tsx](CodexAccessPanel.tsx) | Profile/root facts, mode controls, launch commands, visible mode-specific session-history and original-entry guidance, ordinary file-conflict diagnosis and an existing pending Operation's recovery entry. |
| [codex-launch.ts](codex-launch.ts), [codex-surfaces.ts](codex-surfaces.ts) | Confirmed command-copy eligibility and the distinction between detected executables and mode applicability. |
| [mutation-feedback.ts](mutation-feedback.ts) | Submitted, pre-apply cancelled and uncertain outcomes; truthful disable/recovery messages. |
| [Agents page](../../agents.tsx) | Snapshot refresh, selected Agent/tab, in-memory draft and mode, discard guard, native calls, Operation handoff, confirmation-dependent copy effect and task-history composition. |

Qoder, Pi, DSH and OpenCode share the additional-route editor and explicitly selected Plan IDs
through `qoder_additional`, `pi_additional`, `dsh_additional` and `opencode_additional`. It does not
edit the native default or show native catalogs, fixed-source choices or import controls.
Hermes selects one default main-model Plan. Its management Skill has independent
prepare/restore controls and needs no model connection or published Plan. Model
restore retains that Skill. Pending model/Skill Operations use the shared exact
retry entry. Statically known channel models are shown separately from unobserved
live session selection. Hermes can independently prepare the shared collaboration
Skill and either existing trigger mode, retaining its native model and requiring
no model plugin. It is not a Worker executor.
All six supported Agents remain in the common catalog even when discovery returns
no usable installation/configuration. Common rescan preserves existing entries;
absence is not proof of uninstallation. Hermes appears before a profile is selected. This
presentation entry has no context or settings authority. Its details contain the
configuration-directory control, reusing `HermesRootFields` and `agent_snapshot`
for a selected target; successful discovery replaces the unscoped entry. Existing
backend profiles remain authoritative. A successful selected target is saved as a
Desktop preference and reread after remount/restart; an unavailable remembered
target retains its scope instead of silently selecting a default. Preference
storage failure keeps this session usable and reports that the choice was not saved.
Directory reads, cancellation and failures
never configure routing, prepare resources or imply that Hermes is installed.
With no explicit target, the Desktop host checks its caller's nonempty
`HERMES_HOME`, otherwise `HOME/.hermes`, and sends an existing profile through that
same backend scan. It does not guess an IM process's profile. An unavailable
explicit root never falls back to another; an unreadable default remains visible
without hiding other Agents. Known invalid paths, missing explicit roots/configuration
and read errors remain distinct. Hermes uses its own bundled mark in the common icon component.

Management is a separate enable/disable control for every supported family. It
uses the same settings Preview/Apply/owned restore as the other facets, with no
Plan or model requirement. Scope/shared/external facts come from backend status;
retained native files are not presented as globally disabled. CLI entry installation
uses the existing host command only after an explicit click. Current clients opt
into extended Skill status with `include_skills`; legacy queries retain their
original response shape and primary restore references.
Qoder's editor explains that the selected Qoder CLI must complete its own `/login`
before using these routes. Saving configuration does not perform native sign-in or
make a model call. A valid native login rejection in the separate Qoder Live check
remains a failed surface result with `LIVE_NATIVE_LOGIN_REQUIRED`; other native
business errors use `LIVE_CLIENT_FAILED`, while malformed output remains distinct.
Its model configuration uses the existing Preview, protected token and independent
restore flow. A model restore keeps the collaboration Skill; a collaboration restore
keeps the model routes. Token updates retain the saved ecosystem-specific selection.

Task collaboration remains independently usable without any model connection or
published Plan: its form submits `model: keep`. DSH configures the standard Web profile model picker; global provider shadowing is
reported by the backend before Apply. Each selected Plan has its own provider and
user-selected Responses/Messages protocol. OpenCode uses its saved native model picker;
its native default and small-model choices remain independent. Relevant native references
are backend Preview facts: removing a referenced selector retains the draft and explains
how to change that reference before restoring.

Both model-capable status and actual
collaboration-only status retain this path. Capability and installed-Skill checks
provide scoped prerequisite or diagnostic facts; neither proves a delegated task
completed or a model route worked. They do not create another settings workflow.
Context IDs bind requests and are not user-facing configuration scopes. Show only
readable locations already supplied by the native projection (native configuration and Skill directories/files); do not infer a path from an opaque context ID.
The Plan editor owns its single-CLI installation selection through
[WorkerDependencies](../WorkerDependencies.tsx); `qoder` and `qodercli` paths are presented
as returned by discovery, without adapter or Node placeholders. OpenCode also selects
only its native executable. Its Worker format prerequisite is separate from ordinary
model configuration and collaboration capability; installation guidance names the
supported official package without adding a setup wizard.

The page owns interaction state so changing forms does not create another submission or
Operation store. The native session owns confirmation, protected token registration and
submission identity; the daemon owns persistent Operations, authorization, journals and
recovery. The top-level `agents.tsx` type exports and `agent-editor-state.ts` /
`agent-mutation-feedback.ts` exports remain compatibility entry points for existing callers.
New Agent feature code imports its local owners directly.

## Current call paths

1. The page invokes `agent_snapshot`. The native
   [bridge](../../../src-tauri/src/bridge.rs) and
   [Agent session](../../../src-tauri/src/agent_session.rs) use Client Core to read
   `ScanAgents`, `GetAgentConnectionStatus` and the plan catalog. A per-Agent status error
   stays unknown; discovering an executable does not prove model access.
2. Enable/Edit creates a draft. `agentSettingsSpec` builds the current settings intent;
   `preview_agent_settings` reaches native Preview, confirmation and protected Apply.
   If authoritative blockers require a local check, the page invokes
   `check_agent_authentication` and re-previews. The form's completeness check cannot
   authorize a route or prove a live model call.
3. A returned Operation goes to the shell's existing Operation observer. Missing identity
   after a possible submission remains uncertain. First Codex profile enable copies a
   command only after the **same** Operation succeeds and a fresh snapshot confirms the
   target. Clipboard failure preserves the completed connection and manual-copy entry;
   it never replays Apply.
4. Ordinary Disable submits a new restore intent through Preview. The backend restores
   files conditionally before withdrawing access. A normal file conflict preserves the
   original connection and permits a fresh Preview/Operation after repair. The separate
   `retry_agent_settings` entry is for an existing pending Operation, including historical
   sealed revoke tails; it is not the ordinary conflict retry path.

The native [check session](../../../src-tauri/src/agent_check_session.rs) separately
owns exact ecosystem/surface checks, paid-call consent and response validation for
real-call diagnostics. These backend checks and their acceptance evidence are not
additional enable/save steps or model-verification badges on the settings page.

## Keep the product capabilities separate

| User capability | Owner and evidence boundary |
| --- | --- |
| Scan installations / read connection state | Native Agent snapshot and backend integrations; this feature renders the projection. |
| Import an Agent's existing model source into HiRoute | [Model management](../../product/ModelManagementPage.tsx), [device scan](../device-scan/DeviceScanList.tsx) and the protected candidate flow. Import does not configure that Agent to use HiRoute. |
| Connect an Agent to HiRoute routing | These forms and settings intents, followed by native/backend Preview and Apply. |
| Safely restore managed configuration | Application transaction and integration/storage effects. The UI presents progress and recovery actions; it cannot infer safety from file appearance. |
| Copy a command / start an Agent | Copy is a host clipboard effect. It does not launch a process, enable service startup or prove a model call. Existing Codex profile/root applicability stays explicit. |
| Enable delegation / execute a Worker task | Collaboration configures the caller's task-routing skill. [Task history](../AgentTasks.tsx) and Worker contracts separately own execution, cancellation and results. Model routing support does not imply Worker harness support. |

## Product contract tests

| User journey or invariant | Representative evidence |
| --- | --- |
| Shared configuration actions and state across the supported Agent ecosystems, without an automatic live model call | `agent.configuration.shared-interactions` in the [Agent browser scenarios](../../../tests/v3/browser/agent-trust-scenarios.mjs) |
| First enable, exact existing selections, unknown state and discard protection | [agent-editor-state.test.mjs](../../../tests/agent-editor-state.test.mjs) |
| Correct ecosystem intent, independent restores, saved-selection token rotation and unsupported identity rejection | [agent-settings-request.test.mjs](../../../tests/agent-settings-request.test.mjs) |
| Collaboration without a model or published Plan, prerequisite failure/retry and restore; independent single-CLI selection | `qoder.collaboration.enable-without-model`, `qoder.collaboration.retry-and-restore`, `qoder.installation.single-cli` in the browser scenarios; [status projection](../../../tests/agent-status.test.mjs) |
| Additional routes, cancelled drafts, unavailable saved routes, token rotation and independent restores | `qoder.routing.additional-plans`, `qoder.routing.adjust-and-restore`, `opencode.routing.additional-plans`, `opencode.routing.default-in-use`; the native [settings session](../../../src-tauri/src/agent_session.rs) and [check session](../../../src-tauri/src/agent_check_session.rs) retain protected-boundary tests. Browser IPC does not prove native settings preservation or real provider calls. |
| OpenCode native-only installation selection and independent saved Harness choices | `opencode.installation.single-cli` in the [routing browser scenarios](../../../tests/v3/browser/routing-worker-scenarios.mjs). |
| Submission uncertainty and restore-first feedback | [agent-mutation-feedback.test.mjs](../../../tests/agent-mutation-feedback.test.mjs) |
| Same successful Operation and fresh target before copying; discovery versus applicability | [codex-launch.test.mjs](../../../tests/codex-launch.test.mjs), [codex-surfaces.test.mjs](../../../tests/codex-surfaces.test.mjs) |
| Reachable forms, cancellation, mappings and native clipboard IPC selection | [Agent browser scenarios](../../../tests/v3/browser/agent-trust-scenarios.mjs), [clipboard.test.mjs](../../../tests/clipboard.test.mjs) |
| File conflict keeps the grant, service-failure compensation and delete races | [backend profile tests](../../../../../crates/daemon/src/control/runtime/settings_profile_tests.rs) |
| Historical sealed-tail restart and same-Operation recovery | [legacy recovery tests](../../../../../crates/daemon/src/control/runtime/settings_profile_legacy_tests.rs) |

Pure Node tests prove draft/intent contracts. Browser scenarios exercise components with
mock IPC; backend fixtures prove their specific transaction invariants. None alone proves
real Tauri, installed-Agent or account behavior. Select checks using the repository's
[test planner](../../../../../scripts/test-plan.py), retaining that distinction.

## Adding an actual ecosystem

Start with the existing enable, adjust, disable and failure-recovery interaction.
The scenarios above document those product rules. During review,
compare the new ecosystem with Codex and Claude Code: an extra check action, status
badge or confirmation needs a product requirement, not just a callable native API.
Necessary checks use the authoritative Preview/prerequisite path above; normal
configuration does not automatically call a paid model.

First establish its real scan/read/import/connect/restore/launch/delegation capabilities
and the native/backend contract. Add an explicit identity in `ecosystems.ts`, its DTO and
draft/intent cases, and any required blocker presentation. Add a dedicated form through
`AgentModelEditor` only when model management is supported; collaboration-only status
must not acquire an empty model form. Unknown identities must not borrow Claude settings or an existing launch
command. Keep the common page's submission/Operation flow unless the new behavior needs
an independently designed contract.

Use a user journey to decide what can be shared after the first real integration. This
feature has no universal plugin protocol or dynamic form schema, and does not claim support
for additional ecosystems. New native commands, Worker harnesses, configuration formats or
recovery semantics require their own contract work rather than a new UI label.
## Acceptance feedback

Settings Preview blockers remain authoritative. Show the missing capability and an
actionable next step for configuration conflicts, unsafe write targets, and native
authentication checks; do not collapse distinct blockers into repeated generic text.
