# Desktop behavior tests

Start with the user capability, then choose the smallest layer that can observe its
contract. Node state tests explain decision rules; browser scenarios exercise the
production React components and their wiring through synthetic IPC. Neither layer
proves native authorization, persistence, process cleanup or real model calls.

## Capability map

| User capability | Representative tests | Unique guarantee |
| --- | --- | --- |
| Enable or edit Codex routing | `codex.enable.draft-cancel`, `codex.routing.fixed-binding`, `codex.routing.mixed-native-default` in [Agent scenarios](v3/browser/agent-trust-scenarios.mjs) | Cancelling never applies; configured fixed overrides, native reasoning and the chosen default survive save; service responsibility is explained. |
| Configure Claude routing and collaboration | `claude.enable.shared-presets`, `agent.configuration.independent-facets`, `claude.collaboration.check-before-save`, `claude.collaboration.retry-preserves-edit` | Ordinary Claude entry and account Default limitations remain clear; prerequisite checks precede mutation; retry preserves edits. |
| Enable, recover or disable Qoder task collaboration | `qoder.collaboration.enable-without-model`, `qoder.collaboration.retry-and-restore` | No model connection or published Plan is required; required capability checks happen during save. Login and changed-Skill failures retain edits and enabled collaboration with the shared retry action; restore keeps model configuration unchanged. |
| Add and adjust Qoder model routes | `qoder.routing.additional-plans`, `qoder.routing.adjust-and-restore` | Cancelled drafts do not write; only selected Plans are submitted; unavailable choices stay visible; token changes and independent restores keep the other facet. Native default/provider preservation and real model calls require production acceptance. |
| Remove Pi or DSH routes used by the native default | `pi.routing.default-in-use`, `dsh.routing.default-in-use` | A blocked removal retains model routes and task collaboration, directs the user to their selected client, and never changes the native default or calls a model. Both reuse the additional-provider fixture. |
| Configure Agents through common interactions | `agent.configuration.shared-interactions`; [Agent status projection](agent-status.test.mjs) | Codex, Claude Code and Qoder show configured routing without verification badges or extra check buttons. Ordinary navigation never probes a model; backend verification facts cannot hide actual configuration/recovery state. |
| Diagnose or recover an Agent | `codex.restore.conflict-active`, `agent.recovery.non-runnable` | A file conflict preserves current access and unrelated settings; executable diagnostics do not hide recovery. |
| Choose or replace a Worker installation | `worker.discovery.visible-only`, `worker.installation.harness-isolation`, `worker.installation.replace`, `worker.installation.latest-edit` in [routing/Worker scenarios](v3/browser/routing-worker-scenarios.mjs) | Discovery follows visible configuration; installations stay independent; one explicit save commits the latest edit exactly once. |
| Resume incomplete Qoder model configuration | `qoder.routing.resume-pending` | Recovery requires trusted authority and the exact original context/Operation; it refreshes backend state, creates no new setup, preserves collaboration and does not fabricate verification. |
| Select Qoder as a Worker | `qoder.installation.single-cli` | A single CLI is saved through the existing confirmation flow without adapter/Node placeholders; switching execution Agents preserves each installation. |
| Continue editing after route save | `routing.save.unobserved-editable` | An unobserved result does not lock the editor or other routes. |
| Publish a smaller budget used by Qoder | `routing.publish.qoder-budget-conflict`, `desktop.routing.qoder-budget-checkpoint` | Both immediate rejection and an accepted operation's checkpoint failure keep edits and explain disabling only Qoder model routing before publication and reconfiguration; neither disables collaboration nor calls models. |
| Follow a submitted operation | `desktop.operation.dismiss-observes`, `desktop.operation.identity-retry-converges`, `desktop.operation.repeated-submission` in [shell scenarios](v3/browser/product-shell-scenarios.mjs); [operation rules](operation-feedback.test.mjs) | Dismissal does not cancel observation; mismatched identity stays neutral through grace/retry; repeated accepted or unknown saves restart observation; success refreshes and dismisses. Pure tests retain identity, sequence and terminal-state edges. |
| Adjust presentation | `desktop.settings.persisted-scale`; [preference tests](presentation-preferences.test.mjs) | All supported scales reach the real shell and storage; parsing/storage failure rules remain separately testable. |
| Return to routing or configure a classifier | `desktop.routing.reactivation-models`, `desktop.routing.classifier-protocol` | An existing editor refreshes saved model names; quality uses active models; selection, curl copy and native-save outcomes are visible. |
| Connect decision models and custom extensions | `desktop.decisions.save-test-revision`, `desktop.decisions.discard-navigation`; [authentication and reference rules](decision-presentation.test.mjs) | Save-and-test uses the newly saved credential revision; failures remain actionable, origin changes clear credentials, and discard clears retained editors. |
| Correct an invalid collapsed connection field | `desktop.decisions.hidden-invalid-field` | Submission opens and focuses the field, scrolls the complete field and error above sticky actions, and does not save the invalid value. |
| Inspect models used by branch routes | `desktop.models.branch-route-references` | General-model details include regular and upgrade candidates of enabled branch routes, omit disabled routes, and open the referenced route. |
| Add a decision connection from an unfinished route | `desktop.decisions.route-detour` | The shell suspends the route draft, saves and returns the exact typed connection, preserves input on cancel, and never publishes automatically. |
| Inspect a model or open its documentation | `desktop.models.documentation-retains-input`; [model-to-route scenario](v3/browser/model-to-route-scenarios.mjs) | Local readiness does not claim inference success; browser failure retains form input; web anchors retain their default action; route creation uses the selected model. |

Task-route save scenarios also cover Pi and Qoder executor reselection: an explicit
Messages choice survives clicking the selected executor and reaches the saved draft.
They reuse the real Plan editor and existing IPC trace, without launching a native Agent.

The focused Agent fixture explicitly spans the application grid and keeps its evidence
note outside the page viewport. Layout assertions must observe usable component width;
a DOM element that exists outside a valid viewport does not establish readability.

The shell fixture mounts the real `DesktopApp`, including navigation, forms and
observation effects. It controls only IPC responses and records boundary calls;
it does not call private component callbacks. In particular, browser checks of
`save_classifier_openapi` and `open_external_url` verify UI dispatch/outcomes, not
the native OS action.

## Session quality presentation

Session quality names come from the exact retained execution request (same session and request ID), then the current configuration display name when available. Opaque configuration/branch IDs belong in diagnostic tooltips, not primary labels. Mixed or unknown attribution must not be presented as one model. History gaps and partial assessment evidence remain separate visible flags; an unrated stage is not assigned a score.

`plan-quality-models.test.mjs` protects evidence identity, missing evidence and request deduplication. The session-window browser scenarios exercise the real Sessions/PlanQuality wiring, both model stages, coverage flags, unrated copy and evidence navigation; these are mock IPC checks, not native or model-quality evidence.

## Run levels

From `apps/desktop`, with the repository's Node 24 and installed dependencies:

```sh
npm test
npm run build
node_modules/.bin/tsc -p tests/v3/browser/tsconfig.json --noEmit
CHROME_BIN=/path/to/chrome node tests/v3/visual/agent-trust.mjs /tmp/hiroute-agent-tests
```

The full browser run discovers all Agent, routing/Worker and shell scenarios, plus
the model-to-route scenario. Focused browser flags remain available:
`--claude-collaboration-only`, `--worker-replacement-only`, `--route-save-only`,
and `--shell-only` (after the output directory). Unknown flags fail. For selection
logic alone, run `node --test tests/scenario-selection.test.mjs`.

Choose the configured native Desktop/Pilot entry separately for real authorization,
recovery, file changes, task cancellation and external-browser/download acceptance.
See [validation runner](../../../scripts/validation.py) and
[test planner](../../../scripts/test-plan.py). Do not cite a mock-IPC report as
native or daemon acceptance.

## Keep selection and assertions stable

Scenarios carry a stable `id`, capability tags and a display name. IDs describe the
user guarantee and remain stable when names, files or implementation details change.
The reviewed [required sets](v3/browser/scenario-requirements.mjs) are independent of
the discovered catalog: deleting a required scenario, selecting nothing, returning
duplicate results or omitting a selected case fails. Full runs include newly added
scenarios automatically. Batch size is a transport detail, never a scenario selector.
Keep required-ID changes in the same review as an explicit replacement of the
removed guarantee. `green`, `red` and missing execution remain distinct.

[UI consistency](ui-consistency.test.mjs) retains static stylesheet conventions,
shared Disclosure usage, localized copy, native capabilities and the public OpenAPI
byte contract. The native command authority check compares all bundled Tauri handlers
with the AppManifest, generated permission files and explicit main-window permissions.
Adding a command requires reviewing all four together; mock IPC and native compilation
cannot establish that the real WebView may invoke it. Its former source-shape assertions for Agent forms are covered by the
Agent IDs above; settings, classifier actions, model links, route reactivation and
operation effect wiring are covered by the shell IDs. Moving a component no longer
requires preserving a callback expression, a hook dependency spelling or a source
file location. Keep pure state tests when they protect independent identity,
cancellation, ordering or recovery edges that one browser path cannot cover.

When reviewing churn, use `git log -p -- <test> <producer>` to distinguish a product
contract change from fixture repair. Do not use test-count reduction as an acceptance
target, or rewrite an expectation merely to match the implementation. Current
fixtures follow current product types; historical recovery fixtures keep their
separate registration and meaning.

Remaining work outside this batch: Observation contract constants that compare to
themselves need a producer/consumer oracle; duplicated Rust process harnesses need
an ownership review before extraction. Those changes must preserve their unique
cancellation, identity and cleanup assertions and do not belong in these UI fixtures.
