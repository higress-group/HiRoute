# Desktop behavior tests

Native startup presentation is owned by `src-tauri/src/bridge/launch_presentation.rs`
and the macOS launch observer in `hiroute-desktop-host-effects`. Their Rust cases
cover early/late reasons, foreground fallback, background/duplicate suppression,
login-event attributes and reentrant native-handler chaining. The macOS
`launch_lifecycle` executable creates a real NSApplication on a subprocess's main
thread, runs `finishLaunching`, then dispatches a synthetic foreground/login event
through the registered handler. It detects AppKit replacing a pre-run observer;
it does not establish LaunchServices or real login-item acceptance. Ordinary signed
Release acceptance must additionally observe the fixed process's on-screen window
before any desktop-control binding, then verify the initial Home and final read
verdicts. A bound-window screenshot or a successful service query cannot establish
that the application displayed its own first window. Preserve close/reopen and
background-start behavior separately from synthetic launch-event evidence.

## Console and Windows host coverage

The V3 browser runner accepts `HIROUTE_BROWSER_TRANSPORT=console` to execute the
same required product scenarios through the browser host seam. `--console-only`
checks the production login/session pages with synthetic HTTP, Bearer/context
headers, plaintext warnings and host capabilities, and captures zh/en light/dark
at 1200×813 and 755×600. These component scenarios do not establish real daemon,
native authorization or installation acceptance.

`--shell-host-only` adds the actual Windows startup HTML/styles and production
shell settings/update states, host-only command routing and credential-free
reconnect presentation. It uses synthetic host replies and captures the same
language/theme/size matrix. Native origin ACLs, NSIS, Task/login effects and real
upgrades keep their [installed gates](../../../docs/windows-desktop-installation.md#evidence-and-release-claims).
It also distinguishes an unavailable Windows update catalog from installation
recovery, and checks the bundled local-listener recovery instructions and usable
Recheck action at both supported window sizes. Incomplete installation has a
neutral heading and exact version/build installer repair, without storage retry
or an unrelated recovery location.

Start with the user capability, then choose the smallest layer that can observe its
contract. Node state tests explain decision rules; browser scenarios exercise the
production React components and their wiring through synthetic IPC. Neither layer
proves native authorization, persistence, process cleanup or real model calls.

## Capability map

| User capability | Representative tests | Unique guarantee |
| --- | --- | --- |
| Keep current Home startup and service results | `desktop.home.stale-startup-*` in [Home startup scenarios](v3/browser/home-startup-scenarios.mjs) | Late snapshot success, failure follow-up and an independent startup poll cannot overwrite a newer accepted result. Controlled completion ordering uses production React components; actual cold-start recovery remains a native gate. |
| Restore a lost native connection while the daemon responds | `desktop.connection.loss-retry-recovery`, `desktop.connection.pending-and-failed-recovery`, `desktop.connection.unowned-service` in [shell scenarios](v3/browser/native-connection-scenarios.mjs), [production captures](v3/visual/native-connection-captures.mjs) | Accurate Home/Sessions read errors, explicit recovery, duplicate prevention, pending/failure states, external ownership refusal, and zh/en light/dark geometry at both window sizes. Native Resident/daemon faults, exact reply synchronization with unchanged daemon identity, running work, old confirmation expiry and retained data require separate macOS execution. |
| Continue a bounded suggestion read | [Trust progress scenarios](v3/browser/trust-progress-scenarios.mjs), [production captures](v3/visual/trust-progress-captures.mjs) | Deferred work stays pending and continues automatically; real errors retain Retry and older complete responses remain readable. Chinese/English progress and error states fit both themes and supported sizes. Application tests separately cover the work budget, reuse and actual failure classification; browser IPC remains synthetic. |
| Recover after a storage upgrade refuses startup | [Recovery presentation](storage-recovery.test.mjs), [production-style scenarios](v3/visual/storage-recovery-scenarios.mjs), [storage owner proof](../../../crates/local-storage/src/migrations/startup_recovery.rs) | Original-program reinstall requires the authoritative admitted unchanged group; unknown causes never grant downgrade. Verified source guidance names the exact batch, source directory and guides; long names wrap in both languages/themes. Windows supplies a route from the displayed log folder; macOS offers the preserved-material action. Missing/unsafe locations grant neither located recovery nor resume. Native installed upgrade, complete restoration and real requests remain separate acceptance. |
| Search session content and continue reading matches | [Session scenarios](v3/browser/session-window-scenarios.mjs), [session layout captures](v3/visual/session-window-active.mjs) | Results represent distinct matching sessions; keyword edits discard stale pages, failed continuation withdraws its cursor, and clearing search resumes automatic selection after an in-flight detail read. Titles skip only affirmative native auxiliary requests, including across pages; auxiliary details and unclassified legacy content stay readable. Request excerpts distinguish pending, unavailable and explicitly deleted content. Pending indexing and complete empty results remain distinct. Chinese/English searching, results, empty, pending and retry states stay readable at 1200×813 and 755×600 using production styles. Real lookup, fresh authorization and Worker activity require native acceptance. |
| Keep the actual input and joint execution understandable | [Session scenarios](v3/browser/session-window-scenarios.mjs) | Selected-input positions exclude late context without filtering literal user text. Legacy or unavailable positions stay explicit. The opening group survives a later relay, accepted executors remain visible, and joint stage quality is distinct from a single-model capability score. |
| Identify a connected Agent in session history and discovery | [Agent presentation projections](agent-presentation.test.mjs), the supported-Agent scenario in [Session scenarios](v3/browser/session-window-scenarios.mjs) | All seven supported integrations retain their names in both the session list and selected heading in Chinese and English; unknown and unlinked identities remain distinct. Home summaries retain the native identity for branding. Sessions, scans and configuration summaries reuse the existing Agent presentation owner in `src/features/agents/ecosystems.ts`. |
| Resume a published trial mapping and watch its session | `desktop.routing.recovered-mapping-session` in [shell scenarios](v3/browser/product-shell-scenarios.mjs) | The recovered Routing entry opens the exact observed session or the session list, honors unsaved-route navigation guards, and releases its dialog without repeating the mapping. Production components use synthetic IPC; live mapping and observation remain native acceptance gates. |
| Map a trial route to an Agent with additional models | `quickstart.<dsh/pi/qoder/opencode>.<en/zh>.no-claude-presets` in [Trial scenarios](v3/visual/new-model-trial-active.mjs) | Clearing Claude presets before selecting another Agent does not block review. The completed mapping names the selected Agent without inventing Claude preset changes; the request retains the corresponding additional-model mode. |
| Load trial choices and current selected settings | `quickstart.agent-reads.*` in [trial read scenarios](v3/visual/trial-agent-read-checks.mjs), also selected with `new-model-trial.mjs OUTPUT --trial-agent-reads` | Opening reads discovery once and one model detail. Selection/retry avoid rediscovery; A→B→A and close/reopen reject late details. Mapping refresh blocks a changed native selection before Preview/Apply. Browser IPC is synthetic; actual Release time to usable configuration remains native acceptance. |
| Connect an API through the service directory | [Connection flow scenarios](v3/visual/connection-flow-active.mjs), [directory and draft rules](connection-directory.test.mjs) | Full branded directory, separate product drafts and keys, checked source name, cancellation, explicit model selection and inherited append behavior. Uses production styles; native save and live calls need separate acceptance. |
| Enable or edit Codex routing | `codex.enable.draft-cancel`, `codex.routing.fixed-binding`, `codex.routing.mixed-native-default` in [Agent scenarios](v3/browser/agent-trust-scenarios.mjs) | Cancelling never applies; configured fixed overrides, native reasoning and the chosen default survive save; service responsibility is explained. |
| Configure Claude routing and collaboration | `claude.enable.shared-presets`, `agent.configuration.independent-facets`, `claude.collaboration.check-before-save`, `claude.collaboration.retry-preserves-edit` | Ordinary Claude entry and account Default limitations remain clear; prerequisite checks precede mutation; retry preserves edits. |
| Enable, recover or disable Qoder task collaboration | `qoder.collaboration.enable-without-model`, `qoder.collaboration.retry-and-restore` | No model connection or published Plan is required; required capability checks happen during save. Login and changed-Skill failures retain edits and enabled collaboration with the shared retry action; restore keeps model configuration unchanged. |
| Add and adjust Qoder model routes | `qoder.routing.additional-plans`, `qoder.routing.adjust-and-restore` | Cancelled drafts do not write; only selected Plans are submitted; unavailable choices stay visible; token changes and independent restores keep the other facet. Native default/provider preservation and real model calls require production acceptance. |
| Remove Pi or DSH routes used by the native default | `pi.routing.default-in-use`, `dsh.routing.default-in-use` | A blocked removal retains model routes and task collaboration, directs the user to their selected client, and never changes the native default or calls a model. Both reuse the additional-provider fixture. |
| Configure Agents through common interactions | `agent.configuration.shared-interactions`; [Agent status projection](agent-status.test.mjs) | Codex, Claude Code and Qoder show configured routing without verification badges or extra check buttons. Ordinary navigation never probes a model; backend verification facts cannot hide actual configuration/recovery state. |
| Discover Agents and retain a selected local profile | `hermes.discovery.parallel-entry`, `hermes.discovery.brand-and-known-profile`, `agent.discovery.remember-and-revalidate` in [Agent scenarios](v3/browser/agent-trust-scenarios.mjs) | Hermes stays alongside other Agents before discovery; only its details offer directory selection. Cancel/error retains context without writes; a successful selected-target read replaces the unscoped entry and enables the existing settings flow. |
| Prepare independent management and Hermes task routing | `agent.management.all-supported-families`, `agent.management.owned-restore`, `hermes.collaboration.native-model-independent` | Six management controls, independent owned restore, retained shared/external files and existing Hermes trigger modes without a managed model or Plan. Real installed Skill loading and Worker completion retain separate native acceptance. |
| Diagnose or recover an Agent | `codex.restore.conflict-active`, `agent.recovery.non-runnable` | A file conflict preserves current access and unrelated settings; executable diagnostics do not hide recovery. |
| Choose or replace a Worker installation | `worker.discovery.visible-only`, `worker.installation.harness-isolation`, `worker.installation.replace`, `worker.installation.latest-edit` in [routing/Worker scenarios](v3/browser/routing-worker-scenarios.mjs) | Discovery follows visible configuration; installations stay independent; one explicit save commits the latest edit exactly once. |
| Resume incomplete Qoder model configuration | `qoder.routing.resume-pending` | Recovery requires trusted authority and the exact original context/Operation; it refreshes backend state, creates no new setup, preserves collaboration and does not fabricate verification. |
| Select Qoder as a Worker | `qoder.installation.single-cli` | A single CLI is saved through the existing confirmation flow without adapter/Node placeholders; switching execution Agents preserves each installation. |
| Continue editing after route save | `routing.save.unobserved-editable` | An unobserved result does not lock the editor or other routes. |
| Publish a smaller budget used by installed Agents | `routing.publish.qoder-budget-conflict`, `desktop.routing.qoder-budget-checkpoint`, `desktop.routing.additional-budget-consumers` | All actual Pi/DSH consumers and output reductions are visible; both immediate rejection and an accepted operation's checkpoint failure keep edits and explain disabling only Qoder model routing before publication and reconfiguration; neither disables collaboration nor calls models. |
| Follow a submitted operation | `desktop.operation.dismiss-observes`, `desktop.operation.identity-retry-converges`, `desktop.operation.repeated-submission` in [shell scenarios](v3/browser/product-shell-scenarios.mjs); [pending recovery scenarios](v3/browser/operation-recovery-scenarios.mjs); [operation rules](operation-feedback.test.mjs) | Dismissal does not cancel observation; mismatched identity stays neutral through grace/retry; repeated accepted or unknown saves restart observation. A resolved unknown or older hint cannot repeatedly clear the terminal result and restart a slower Home read; a distinct later submission still needs its own lookup. Success refreshes and dismisses. Pure tests retain identity, sequence and terminal-state edges. |
| Adjust presentation | `desktop.settings.fixed-scale`; [preference tests](presentation-preferences.test.mjs) | Settings retain language and appearance controls; application text stays at 100% with no size selector. Previous 150/200% browser preferences normalize to 100% without changing language or theme, including after remount. Storage failure rules remain separately testable. |
| Configure the shared subscription proxy from a device scan | `desktop.models.subscription-proxy.inline-cancel`, `desktop.models.subscription-proxy.read-retry`, `desktop.models.subscription-proxy.save-retry-return`; [restart hint tests](subscription-proxy-return.test.mjs) | The inline editor preserves scan results and selection, removes the Settings entry, prevents an unread-policy overwrite and retains failed-save input. A simulated restart reloads the scan and saved policy; native restart, persistence and provider transport require production acceptance. |
| Choose independent subscription sign-in from a device scan | `desktop.models.subscription-scan.recommended-login`, `desktop.models.subscription-scan.append-to-login` | Scan explains native credential refresh and links to the existing independent sign-in without starting authorization or saving. Changing from an append scan to independent sign-in clears the previous connection target, so a new account saves as a new connection. Chinese/English at the fixed 100% size and 755×600 protect wrapping, visible actions and the initial login explanation while session loading is delayed, including previously enlarged browser preferences. Real OAuth refresh retains its separate native gate. |
| Return to routing or configure a classifier | `desktop.routing.reactivation-models`, `desktop.routing.classifier-protocol` | An existing editor refreshes saved model names; quality uses active models; selection, curl copy and native-save outcomes are visible. |
| Connect decision models and custom extensions | `desktop.decisions.save-test-revision`, `desktop.decisions.discard-navigation`; [authentication and reference rules](decision-presentation.test.mjs) | Save-and-test reads back the saved revision and tests its configuration digest; failures map to repairs, origin changes clear credentials, and discard clears retained editors. |
| Reuse an API connection or configure a decision model independently | `desktop.decisions.shared-source`, `desktop.decisions.source-closure`, `desktop.decisions.test-freshness`, `desktop.decisions.nested-detour`, `desktop.decisions.source-read-states`, `desktop.decisions.selector-labels`; [form-state rules](decision-presentation.test.mjs); [production captures](v3/visual/decision-model-captures.mjs) | Reuse is the default only after a ready read and never picks a connection; slow or failed reads offer independent setup and Retry; bundles stay per mode/connection and pending keys clear on switches. A save names only the newly selected API connection (none for independent setup) and deletion names every related one; the synthetic store rejects any other set. Save reads back the exact revision before testing its digest; a pass counts only against credential state read back after the test, so rotation marks it stale and a failed readback leaves it unverified without retesting. Route → decision model → API connection → decision model → route returns explicitly, keeps every non-secret bundle and stays guarded. Route selectors accept V1 and V2 and disambiguate same-name entries. Shared, independent, read-error, detour, stale, unverified and each nested-detour step fit zh/en light/dark at 1200×813 and 755×600 without visible digests. Synthetic IPC only: it does not prove credential persistence, shared-key resolution or routing. |
| Read retained C counts and coverage | `desktop.quality.c-observations` in [C scenarios](v3/browser/c-protocol-scenarios.mjs) | Counts retain explicit denominators; a pending refresh clears stale counts and preserves the expanded section when results arrive. Incomplete coverage hides rates and explains unconfirmed origins; long identities and empty scopes remain readable in Chinese and English. Mock IPC covers rendering, while daemon product tests own event ingestion and retained reads. |
| Edit native default/custom assessments | `desktop.routing.native-default`, `desktop.routing.custom-assessment` in [routing scenarios](v3/browser/c-protocol-scenarios.mjs); [released intent adapter](native-protocol.test.mjs) | One native default without capability/background controls or route test; exact custom text, thresholds and reset. Side-by-side threshold controls and helper rows stay aligned when descriptions wrap. Browser IPC is synthetic; native publication/reopen and real provider diagnostics have separate evidence. |
| Correct an invalid collapsed connection field | `desktop.decisions.hidden-invalid-field` | Submission opens and focuses the field, scrolls the complete field and error above sticky actions, and does not save the invalid value. |
| Inspect models used by branch routes | `desktop.models.branch-route-references` | General-model details include regular and upgrade candidates of enabled branch routes, omit disabled routes, and open the referenced route. |
| Add a decision connection from an unfinished route | `desktop.decisions.route-detour` | The shell suspends the route draft, saves and returns the exact typed connection, preserves input on cancel, and never publishes automatically. |
| Inspect a model or open its documentation | `desktop.models.documentation-retains-input`; [model-to-route scenario](v3/browser/model-to-route-scenarios.mjs) | Local readiness does not claim inference success; browser failure retains form input; web anchors retain their default action; route creation uses the selected model. |
| Change model grouping or inspect page controls | `desktop.models.view-switch`, `desktop.presentation.control-layout`; [presentation scenarios](v3/browser/presentation-interactions.mjs) | Production styles preserve model/connection counts, selected model and visible selection. Chinese/English and light/dark checks reject browser-default buttons and horizontal overflow from long recent-session titles at the supported small window. |
| Try a new model, judge its evidence and follow live routing | `timeline.initial-failure-retry-recovers.*`, `timeline.polls-only-while-displayed-and-visible`, `trust.scoring-states.both-groupings.*`, `quickstart.*.inline-connection-guard.model-change-and-back` in [trial scenarios](v3/visual/new-model-trial-active.mjs) | A failed timeline read keeps its tab, error and Retry until recovery, while only a confirmed empty read hides the tab; polling continues only while the timeline is shown and the window is visible, after one availability probe. Per-model and per-executor groupings both show scoring coverage and complete/partial/awaiting/unscored counts. Changing models, going back or leaving the quick start asks before discarding the inline connection form; Cancel keeps the model and input. `HIROUTE_BROWSER_TRANSPORT=console` runs the same scenarios over the console host. Real feed retrieval, saved trials and live sessions require native acceptance. |
| Reuse a connected model and complete trial setup | `readiness.binding-and-primary-reasoning`, `picker.exact-source-no-discovery`, `picker.append-to-exact-saved-api`, `picker.append-to-exact-saved-subscription`, `save.user-declared-binding`, `save.lost-reply-recovers-original-operation`, `trust.initial-presets-exact-review-back` and `trust.publish-and-reopen-exact-values` in [setup scenarios](v3/visual/trial-setup-followup-active.mjs), selected by `new-model-trial.mjs <evidence-directory> --setup-followup`; [readiness predicates](trial-readiness.test.mjs) and [name conformance](plan-display-name.test.mjs) | Production components and styles use synthetic save receipts and a combined source/options capture. `capture.retry-at-most-once` and `capture.descriptive-catalog-does-not-grant-binding` cover bounded retries and catalog admission independent of recommendations; review repair cases cover guarded replacement, retained recovery, read-only retained API selection, independent recommendation retry and exact name/editor values. Explicit saved-model selection, registered/custom check paths, changed or cancelled reads, retries, keyboard focus and preserved exact trust are exercised without provider calls; native authorization, ordinary Release acceptance and real inference remain separate. |
| Explain built-in protection and custom extension trials | `protection.native-three-outcomes.*`, `protection.custom-observation-only.*`, `division.custom-no-eligible-group.*` in [trial scenarios](v3/visual/new-model-trial-active.mjs); [copy tests](protection-copy.test.mjs) | Both languages, themes and supported window sizes retain the three built-in applicability outcomes and distinguish release from the difficulty-based group. `desktop.routing.native-default` compares displayed and saved default instructions across locales. Custom quick start preserves parked judgment, shows no built-in level or rubric, and explains optional observational scores. A branchless custom trust report still permits manual division of eligible published groups; no eligible groups means no entry. Opening the derived draft does not save, publish or change mappings. Other trust/replay scenarios explicitly select a built-in fixture. Browser IPC remains synthetic. |
| Keep selected sessions and delayed excerpts current | [Live session refresh scenarios](v3/visual/session-live-refresh-active.mjs) | Narrow summary refresh preserves list pages, selection and scroll. Visible unresolved excerpts retry independently of timeline digests; hidden/inactive views pause. Production-shaped empty catalogs after cleanup, late responses, externally deleted/expired summaries, reopening and new-request revival preserve current body visibility. |
| Read exact turn ranges and recover live timeline reads | [Timeline presentation scenarios](v3/visual/timeline-presentation-active.mjs) | Assessment Agent-turn IDs resolve to display coordinates across producer epochs; unavailable or ambiguous identities remain explicit. Transient reads preserve the last view and recover with bounded backoff; real capacity failures require Retry. An extension's selected group and reason survive timeout recovery and a full refresh when a late low score arrives, without a built-in floor or protection claim. Fixed labels occlude moving cards without hiding keyboard focus. |

Run the focused production-component cases with
`node tests/v3/visual/new-model-trial.mjs <evidence-directory> --session-live-refresh`
or `--presentation` from the Desktop directory. They use production styles and
controlled IPC; they do not establish native persistence or real model outcomes.

`--routing-presentation` exercises evidence-backed pending-to-resolved layout,
focus and bounded exact-name/source reads. `--trust-continuous` exercises exact
slider/preset interaction and bounded replay across the supported language, theme
and window combinations. Both use production components and styles with controlled
IPC; native execution and actual saved/reopened data remain separate checks.

The focused timeline pricing checks run with
`node tests/v3/visual/new-model-trial.mjs <evidence-directory> --timeline-pricing`.
They also run in the default trial suite. Production components must remove a
provisional row absorbed by exact completed-request membership, retain selection,
actual keyboard focus and correct counts, and show known API-equivalent subtotals alongside unknown
contributions in Chinese and English. Mock IPC proves presentation, while the
native model-save publication journey covers installed price edits, real Gateway
valuation, frozen history and restart.

Completion focus scenarios exercise overview and lane controls with real keyboard
and mouse events, including visible keyboard focus. They retain focus on an
unrelated search field, a surviving turn or the page body instead of moving it
because selection changed. English/light at 755×600 and Chinese/dark at 1200×813
use production components and styles; native WebView acceptance remains separate.

The [historical protection and overview scenarios](v3/visual/new-model-trial-protection-active.mjs)
run with `node tests/v3/visual/new-model-trial.mjs <evidence-directory> --protection`.
They verify scope-matched release counts and explicit unavailable reasons, a selectable
24-hour interval inside independently clipped retention boundaries, the opening
selection's historical-protection outcome/previous score, and forty-turn overview
navigation, viewport tracking, keyboard access and live append. Use the same
`HIROUTE_BROWSER_TRANSPORT=console` switch for the console host. These remain
production-component scenarios with synthetic data; real protection/release
records and native acceptance need their own evidence. Run browser suites serially.

The [session scenarios](v3/browser/session-window-scenarios.mjs) also open the actual
timeline-to-Plan-performance dialog without a supplied revision. They pair current,
historical and all-version reads, retain a genuine zero despite small sample counts,
recover failed active-revision resolution and reject late earlier trust/quality
responses while respecting the production reader capacity. Unknown and genuinely
mismatched evidence keep the existing unavailable-state assertions above.

The targeted trial connection-reuse cases run with
`node tests/v3/visual/new-model-trial.mjs /path/to/evidence --trial-reuse`.
The same production-component group checks several available subscription logins
against a source/revision-scoped lookup and confirms a saved original operation
after later model or Agent changes without overwriting those choices.
They retain the chosen model while reusing an API/subscription, verify exact saved
binding reuse without writes, one-target append and automatic selection, editable
or missing defaults, failure and original-save recovery, and localized form geometry.
Production components and styles use controlled IPC; these are not provider or
native persistence evidence. The `ListConnectionOptions` mapping is opt-in and
catalog-only; ordinary responses preserve their released shape.

The [trial interaction refinements](v3/visual/new-model-trial-polish-active.mjs)
run with `node tests/v3/visual/new-model-trial.mjs <evidence-directory> --polish`.
They cover cached trust-level replay, precise-value debounce, shared reason labels,
bounded two/forty-turn lanes and sticky labels, visible arrivals and unread counts,
automatic following until an explicit selection, reduced motion, suggestion hierarchy,
Home suggestion start times under unrelated announcements and retention boundaries,
model-card facts and footer clearance across both supported sizes, languages and themes.
Small-window timeline captures use the product's Compact view. Browser suites in
one checkout share the Vite dependency cache and should run serially. These
fixtures do not establish native or provider effects.

The [trial setup and connection-session scenarios](v3/visual/trial-setup-polish-active.mjs)
run with `node tests/v3/visual/new-model-trial.mjs <evidence-directory>
--trial-setup-polish`. They mount the production trial explanation and connection
form with controlled IPC. They cover the three classifier explanations, keyboard
disclosure, exact single-product eligibility and return, per-product edit and Key
retention, reverted edits, selected models, failure and same-Operation save states.
The connection cases also cover authentication None, primary protocol swaps and
protected-input re-entry; prefill Key isolation and current/retained prefill baselines;
exact current option qualification, live provider details,
cancelled or superseded catalog recovery, and pristine versus edited model rows.
Chinese and English captures use production styles at 755×600 and 1200×813 with
light and dark states. The fixture does not establish native save or provider behavior;
the combined trial custom-connection path needs its own integration acceptance.

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

For an interactive presentation regression check, open
`product-shell.html?checks=presentation` on the fixture server at 755×600 and
click **Run presentation checks**. It runs the same two required presentation
scenarios with production styles and reports each result; IPC remains mocked.

The full model-information views run with `node tests/v3/visual/new-model-trial.mjs
/path/to/evidence --metadata`. They mount production Models, default-intent controls,
metadata directory and removal dialogs with synthetic IPC. The scenarios cover
Chinese/English, light/dark, 755×600 and 1200×813, retained edits after read failures,
stale-cursor restart, four price components and confirmation only after all impact
pages are reviewed. Actual generated inferred and observed records cover visible
field markers, linked source/rule reads bound to the selected snapshot, and retry
after source-read failure. [Fact presentation tests](metadata-facts.test.mjs) preserve
unknown, conditional, conflicting and not-applicable meanings. Installed CLI/daemon tests separately own persistence and
actual feed/default/price effects.

## Session quality presentation

Session quality names come from the exact retained execution request (same session and request ID), then the current configuration display name when available. Opaque configuration/branch IDs belong in diagnostic tooltips, not primary labels. Mixed or unknown attribution must not be presented as one model. History gaps and partial assessment evidence remain separate visible flags; an unrated stage is not assigned a score.

`plan-quality-models.test.mjs` protects evidence identity, missing evidence and request deduplication. The session-window browser scenarios exercise the real Sessions/PlanQuality wiring, both model stages, coverage flags, unrated copy and evidence navigation; these are mock IPC checks, not native or model-quality evidence.

`trust-cost-presentation.test.mjs` checks exact/range/unknown selection, currency,
safe integer rounding and invalid or incomplete evidence. Run
`node tests/v3/visual/trust-cost-presentation.mjs /path/to/evidence` for the
production-style cost card, tiles, selected and step-up scopes, keyboard disclosures,
and Chinese/English light/dark layouts at both supported window sizes. The browser
uses controlled IPC; retained-data readback and native user journeys remain separate.

## Run levels

Custom API transport and error presentation use the production form in
`node tests/v3/visual/connection-flow.mjs /path/to/evidence`. It exercises manual
HTTP save without a directory, closed backend error reasons with retained input,
and plaintext notices in zh/en, light/dark, at 1200×813 and 755×600. Browser IPC is
synthetic; `real_process_custom_http_save_recheck_and_validation_errors` in the
daemon publication tests separately proves actual control/save/restart behavior.

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
`--subscription-proxy-only`, and `--shell-only` (after the output directory). Unknown flags fail. For selection
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

The focused Agent page checks run with `node tests/v3/visual/agent-trust.mjs
<evidence-directory> --agents-only`. Optional `HIROUTE_BROWSER_SCENARIOS` selects
explicit scenario IDs without dropping their completeness assertions. Model-save
readback covers exact model proof, delayed/wrong-context responses, unrefreshed
Skill states and fresh Skill editor selection. Readback captures use production
styles in Chinese/dark and English/light at 1200×813 and 755×600. Native save
confirmation and latency still require the normal Release app and real data.

Home bounded-read regressions in `v3/browser/home-startup-scenarios.mjs` exercise
shared queue capacity, typed busy recovery/exhaustion, terminal failures and
superseded retries through the production hook. They remain React/mock-IPC
evidence; native first-open/focus reads and retained data require the real Desktop.

`desktop.trial.connected-models-and-models-return` runs the actual DesktopApp
navigation from the saved-model control to Models and back, preserving exact
binding, route name and trust without writes. The existing setup append cases
now enter through the connection-point choice in Connect here and retain their
exact API/subscription source, candidate, append-only and no-key-edit assertions.
`--trial-identity` checks compact reasoning-control geometry, disclosure, keyboard
choice, localization and unchanged submitted profiles in both themes and sizes.
All of these browser paths use controlled IPC; native business effects remain
separate acceptance.
