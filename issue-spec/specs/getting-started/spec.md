# Getting started and Desktop lifecycle

## Purpose

HiRoute Desktop lets a local user connect model sources, publish routing plans,
configure coding Agents, and inspect execution without an embedded chat Agent.
This specification covers the application shell, installation, and upgrade handoff.

## Requirements

### Requirement: Development versions follow the latest recorded stable release

The product version used by CLI, daemon, Desktop, Windows shell and diagnostics
SHALL be consistent across their manifests, Cargo/npm lockfiles and generated
Local Control handshake schema/digest. Once a stable
release is recorded, subsequent development SHALL use a version with greater
SemVer precedence than that release. The next patch prerelease is the default:
after `0.3.1`, use `0.3.2-dev.1`, which remains below stable `0.3.2`. Build metadata
alone SHALL NOT make a same-core version qualify as later development.

A shared version tool SHALL update these fields together. CI SHALL reject stale,
regressed or inconsistent versions. The post-publication contract PR SHALL also
prepare the next development cycle when needed, retain an already newer planned
version, and require normal human review/merge. Frozen released sources and
snapshots SHALL remain unchanged. Development identity SHALL NOT establish a
published stable release or supported protocol/adapter compatibility.

The existing exact-version client/service handshake SHALL remain enforced.
Upgrade selection SHALL not offer the preceding stable version to its later
development build and SHALL offer the next matching stable release when available.

#### Scenario: Continue development after a stable release

- **WHEN** stable `0.3.1` is recorded and the repository still declares `0.3.1`
- **THEN** version CI rejects the stale development identity, and version advancement prepares `0.3.2-dev.1` consistently across shipped components and lockfiles.

#### Scenario: Upgrade a development build to its next stable release

- **WHEN** a `0.3.2-dev.1` build checks stable releases
- **THEN** `0.3.1` is not an update, while a matching `0.3.2` package can be selected without weakening the exact-version client/service handshake.

#### Scenario: Skip directly to a later minor release

- **WHEN** the next chosen release is `0.5.0` while development declares `0.3.2-dev.1`
- **THEN** the shared tool can prepare `0.5.0` for a committed release candidate without requiring intermediate releases; recording stable `0.5.0` prepares `0.5.1-dev.1` unless main already plans a newer version.

#### Scenario: Retry an open post-release PR after main advances

- **WHEN** the generated contract/version PR remains open while main adopts a newer version and changes dependencies
- **THEN** retry preserves main's planned version and dependency changes, recomputes the owned version projections, and reuses the existing PR without changing frozen released snapshots.

### Requirement: Service health and setup progress are separate facts

Desktop MUST display the actual local service state: starting, available,
unavailable, or incompatible. A successful service read establishes health;
a retained snapshot after a failed refresh does not. Source, plan, and Agent
queries that are loading or empty MUST NOT turn a healthy service indicator into
a setup warning. The home page owns onboarding progress separately.

Sources, plans, model access, and task collaboration MUST be configurable
independently. A user may start with a single fixed model, add resources later,
or enable collaboration while retaining the main Agent's existing model provider.
Home MUST show current configured resources, recent sessions, active delegated
work, and available valuation with its completeness. Empty usage is an onboarding
state, never invented savings or a chart of fabricated zeros.

Home SHALL obtain recorded Agent settings through a bounded summary without
reading every Agent's complete settings or Skill state. A recorded configuration
is not proof of native health, successful verification or permission to act.
Incomplete summary coverage SHALL remain unknown or unavailable, not become an
empty setup. Visible routing editors MAY request model associations separately;
an incomplete association read SHALL NOT be presented as unused.

#### Scenario: Open Home while Agent settings are unverified

- **WHEN** saved settings are recorded but their native state has not been checked
- **THEN** Home presents them as saved and awaiting a check, without showing verified readiness or requiring full settings reads for unrelated Agents.
- **AND** an incomplete summary or association keeps its uncertainty and remains retryable; it cannot establish first use or an unused route.

#### Scenario: Return while a resource query fails

- **WHEN** service status succeeds but a model or history query is pending or fails
- **THEN** Desktop MUST retain the correct service state, identify the affected read,
  and offer a retry without overwriting edits or claiming the resource is absent.

#### Scenario: A startup result arrives after a newer Home refresh

- **WHEN** an old snapshot or startup-status read completes after a newer Home result
- **THEN** it MUST NOT clear the current startup failure or replace newer successful
  service evidence with an earlier failure. Independent startup checks and failed-read
  follow-ups MUST share the same ordering of accepted startup results.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/249#issuecomment-6461278064

### Requirement: Preferences preserve the current interaction

The production shell MUST support English and Chinese, light and dark themes,
and 100%, 150%, and 200% text sizing. Without an explicit language preference,
Chinese or English follows the system locale; other locales use English. Explicit
choices and restoring system-following behavior MUST survive restart. Text size
starts at 100% without a saved choice and MUST NOT derive from device pixel ratio.

Changing locale, theme, text size, navigation width, or a supported window size
MUST preserve non-secret edits, selections, and the active Operation identity.
Headings, controls, supporting text, empty states, and dialogs MUST scale together;
primary actions remain reachable. Keyboard navigation, visible focus, readable
labels, dialog focus/return, and status indications beyond color MUST work.
Non-submitting dialogs support close and Escape; unsaved-loss confirmation occurs
once. A submission may prevent closing only for its actual pending phase, with an
explanation and a bounded recovery path after transport failure.

The effective macOS theme MUST extend to native window chrome. Native traffic
lights, title-area dragging, and the user's title-bar double-click setting remain
usable without a conflicting duplicate title layer. HTTP(S) product links open
through a main-window native browser command restricted to valid HTTP(S) URLs;
file, shell, custom schemes, and URLs without authority MUST be rejected. Browser
previews retain ordinary link behavior.

#### Scenario: Change appearance during a failed save

- **WHEN** a user changes language or text size after a submission/read failure
- **THEN** the same input and Operation association MUST remain available, with
  accessible retry/close controls and no new submission caused by the preference.

### Requirement: The shell reports the current Operation once

Shared submission feedback MUST follow server Operation identity and sequence.
Pending feedback names the action and object and uses an indeterminate indicator
unless measured progress exists. Succeeded, rolled back, blocked, and uncertain
outcomes replace that feedback at the same location. A new Operation replaces the
previous one; stale queries cannot overwrite a newer identity or confirmed terminal
state. A short read delay or transient failure stays neutral while checking the
original identity. Persistent uncertainty exposes one result-to-be-confirmed state
and a query action. Closing feedback hides it without cancelling server work;
success feedback settles after a short display period.

Operation presentation SHALL follow exact server identity and sequence. Pending names the action/object and uses indeterminate progress unless measured. Terminal/uncertain outcomes replace the same feedback; stale reads cannot replace a newer identity or confirmed terminal result. Transient checking remains neutral; persistent uncertainty offers a read-only check of the original identity. Trial save→publish keeps stable controls and one local progression, without intermediate success feedback or duplicate submission. Global pending/success may be suppressed only for exact operations actively represented locally; errors, unrelated operations, no-ID recovery and unresolved operations after unmount remain discoverable. Closing feedback never cancels server work. A same-target, same-connection service refresh may retain last confirmed display with Updating, but never extends write authority; disconnect, failed checks, read-only or recovery failure immediately supersede stale display. Success feedback settles after a short display period. A ready response with gateway not_composed is distinct from a pending page read: it must not inherit a healthy status or gain write authority. Only explicit evidence of an expected same-operation transition may justify neutral progress wording; real unavailable, stalled or failed service states remain visible.

#### Scenario: Two saves overlap in observation

- **WHEN** a late status response for the first save arrives after the second starts
- **THEN** it MUST NOT replace the second save's feedback or trigger a replay.

#### Scenario: Continuous save and publish

- **WHEN** the trial receives exact draft and publication confirmations
- **THEN** each write and next-step callback occurs once; normal progress keeps controls stable and does not show duplicate local/global success.

#### Scenario: Uncertain result and navigation

- **WHEN** the response is lost or the page leaves while publication is pending
- **THEN** the original operation remains queryable without replay; global recovery returns on unmount.

#### Scenario: Refresh versus failure

- **WHEN** a healthy same-target display refreshes, then connection/error/read-only evidence arrives
- **THEN** refresh is neutral Updating without new authority; real invalidation/error becomes visible immediately.

#### Scenario: Concurrent operation feedback

- **WHEN** a late older status or unrelated operation arrives
- **THEN** it does not replace/suppress the current exact operation or hide unrelated failures.

#### Scenario: Returned service is not composed

- **WHEN** a service read completes with gateway not_composed, with or without a concurrent publication
- **THEN** the UI distinguishes returned service evidence from loading; it never claims healthy or enables writes from a prior display, and any neutral progress explanation requires an exact expected-operation basis while real failures remain visible.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/228#issuecomment-6440853030

### Requirement: Information and confirmation match the user's decision

Desktop MUST prioritize the current action and expose cost, permission scope,
completeness, and recovery information before they affect a decision. Infrequent
technical detail may be expandable but core configuration and risk cannot be
hidden. Expandable regions need a discoverable row, localized state, direction,
keyboard support, and correct focus; actions inside them do not toggle the row.
A short standalone field, including a plan's stable model invocation name, remains
visible. Searching options MUST NOT change selected model identity or order.

Save, draft, ordinary publication, and compatibility checks use a clear action
button without a second confirmation of the same intent. Destructive confirmations
identify target and consequence. Internal revisions and digests remain enforced
by the backend without requiring a user to inspect them.

Brand assets MUST be bundled, offline-available, consistent across source, Agent,
Worker, task, and session views, and paired with a readable name. Known Agents use
their actual logos. Provider branding cannot imply a model-specific identity;
unknown objects use a neutral fallback rather than a fabricated brand.

### Requirement: Upgrade preparation keeps the running application available

Desktop MUST select a newer stable release for the current architecture from the
trusted website release catalog. Download path, byte count, SHA-256, package
inventory, component architecture, and existing signature identity MUST be checked
by native code. A self-signed package's origin is established by HTTPS and the
catalog digest; its signature proves package integrity, not Developer ID identity.
WebView actions cannot supply an arbitrary URL, installation path, or trust identity.

The current catalog is the website's `releases.json`; its native reader selects the
actual local architecture rather than accepting a WebView-provided download target.

Downloading and validating MUST leave the service running. Upgrade staging is
native-owned and private. Standard group-writable installation ancestors such as
macOS `/Applications` MUST NOT be rejected using private-data-directory rules.
An unwritable destination fails explicitly without automatic elevation.

#### Scenario: Invalid or changed package

- **WHEN** the mounted App, inventory, digest, architecture, or identity differs
  from the selected release or changes after preparation
- **THEN** installation MUST fail before replacement; existing service/data remain
  available and the failure identifies the relevant stage.

### Requirement: Replacement requires an explicit orderly-stop handoff

The installer MUST originate from the current complete App, preflight, and report
ready before the protected launcher handoff commits replacement. Admission drains
active requests, durable workspace occupancy, and Worker cleanup. The user can
cancel while waiting; cancellation restores admission. A timeout or merely sending
a stop signal does not prove exit and MUST NOT authorize replacement.

Replacement proceeds only after the owned daemon and Desktop have actually exited,
the original host lock is acquired, and package identities are rechecked. Loss of
the parent before explicit commit does not authorize installation. Failures before
replacement restore admission or restart the verified old service through its
normal entry. Uncertain exit remains stop-pending; failed recovery requires an
explicit restart indication. Stable errors distinguish handoff, preflight, commit
wait, process exit, and replacement failures.

#### Scenario: Cancel while a Worker is active

- **WHEN** an upgrade waits for accepted work and the user cancels
- **THEN** work MUST NOT be killed or resubmitted, and normal admission resumes.

### Requirement: Installation and data recovery use verified complete packages

Automatic upgrade and manual overwrite MUST enter the same new-daemon storage
validation, backup, and supported migration path before business access opens.
[Storage recovery](../local-data-and-diagnostics/spec.md) owns its data contract.
Desktop MUST expose backup location and recovery instructions even when daemon
startup fails. Recovery identifies the complete matching old package, preserves
current directories before restoring the whole backup group, and warns that data
created after the backup will be discarded. No automatic restoration follows a
completed migration.

A migration requiring a readable matching old package MUST bind its actual local
identity before changing data. Manual installation without that binding exposes
native selection of the old App; failed selection changes no business data.
Rebuilding the native session invalidates old confirmations and pending approvals.
Cleanup for a completed subscription check stays bound to its original session:
closing a window still releases its resources, but callbacks cannot deliver a result
or consume confirmation in a replacement session.

macOS packages MUST be architecture-consistent for Apple Silicon or Intel and
include Desktop, daemon, CLI, frontend, CPA, and the current catalog. The minimum
OS must satisfy bundled dependencies. The CLI remains usable through its documented
full path without a development toolchain. Replacing an App in the same location
preserves data and managed entry points; moving it requires the normal connection
repair flow. Uninstall first restores managed Agent access, disables login items,
and exits. Deleting the App alone preserves data; optional cleanup cannot delete
projects or native Agent credentials. Package construction alone is not signing,
notarization, native installation, or live-Agent evidence.

### Requirement: Windows x64 Desktop installs per user and opens ready to use

HiRoute SHALL provide a Windows x64 Desktop installer that a standard user can run without administrator rights, Python or a terminal. In this release it is the only supported Windows installation path. It SHALL verify the package, publish the user-owned Windows standalone installation (CLI, daemon, CPA and Skills), register the default Task Scheduler manager, add the Desktop shell, start the service and open the shell when the service is ready. Login start and tray residency SHALL be enabled by default and SHALL be optional during installation. An unattended mode SHALL install with the defaults without prompts. An existing Task Scheduler installation for the same user SHALL be upgraded in place with its data and Agent recovery references preserved. SCM service installation is not offered in this release: an existing SCM-managed installation SHALL be refused before writing, with an explanation, and SHALL remain unchanged. The installer SHALL refuse non-x64 systems before writing. A missing WebView2 runtime SHALL be installed or explained. An interrupted installation SHALL be repairable by running the installer again. Uninstall SHALL restore managed Agent access before removing the manager registration, shell and entries, and SHALL preserve business data unless the user explicitly selects its removal. When neither Agent storage nor its upgrade backup exists, first-install recovery SHALL NOT require a working daemon. If the service is unavailable and Agent state cannot be enumerated, recovery or uninstall SHALL offer a separate default-No interactive confirmation to stop the owned unresponsive Task and proceed with restoration unverified. It SHALL warn that affected Agents cannot be enumerated, configured Agents may need manual restoration and active work may be interrupted; this continuation SHALL retain all business data. Silent and passive modes SHALL refuse this continuation, and enumerated restore conflicts SHALL still block removal. Verified obsolete versioned packages SHALL be pruned only after both installation owners commit; rollback sources SHALL be retained until then. Uninstall SHALL remove its verified current package and preserve changed or foreign files.

#### Scenario: Fresh standard-user installation

- **WHEN** a standard user without Python runs the installer on Windows x64 and accepts the defaults
- **THEN** installation completes without elevation, the HiRoute window opens with a healthy service, and `hiroute` in a new terminal reaches the same instance.

#### Scenario: Existing user installation

- **WHEN** the user already has a Task Scheduler installation of HiRoute with saved sources and plans
- **THEN** installation keeps that data and manager, and the console shows the same sources and plans.

#### Scenario: Existing SCM installation

- **WHEN** the installer finds an SCM-managed HiRoute installation for the user
- **THEN** it stops before writing any file, explains that SCM service installation is not offered in this release, and leaves that installation unchanged.

#### Scenario: Unattended installation

- **WHEN** an administrator or script runs the installer in its unattended mode on Windows x64
- **THEN** installation completes with the default choices without prompts, and the service is healthy afterwards.

#### Scenario: Unsupported architecture

- **WHEN** the installer runs on a Windows ARM64 system
- **THEN** it stops before writing any file and explains that only x64 is supported.

#### Scenario: Interrupted installation

- **WHEN** installation stops after files were partially published
- **THEN** the service is not admitted with mixed files, and rerunning the installer repairs or completes the installation without losing data.

#### Scenario: Unavailable service during recovery or uninstall

- **WHEN** the service cannot start or Agent state cannot be enumerated
- **THEN** recovery or uninstall requires separate explicit interactive consent to continue without verified restoration, explains the possible Agent and active-work effects, and retains all business data; silent/passive execution refuses this exception.

#### Scenario: Restore conflict remains protected

- **WHEN** a managed Agent has an enumerated restoration conflict
- **THEN** uninstall stops before removal and reports the affected Agent; unavailable-service consent cannot bypass the conflict.

#### Scenario: Versioned package cleanup

- **WHEN** installation commits or uninstall removes its package
- **THEN** verified obsolete payloads are removed, rollback sources survive until both owners commit, interrupted cleanup can resume, and changed or foreign entries remain untouched.

#### Scenario: Replacement changes frontend resource names

- **WHEN** an existing Windows Desktop installation is replaced by a verified package with different hashed frontend asset filenames
- **THEN** the installer durably backs up the prior owned files before retiring obsolete payload paths, validates the complete new package without accepting foreign or modified content, and preserves the old versioned core until both installation owners commit.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219731276

### Requirement: The Windows shell is a thin host for the console

The Windows Desktop shell SHALL run as a single instance, show the console in an embedded WebView after the service is ready, and offer tray actions to open the window, open the console in the default browser and quit the shell. It SHALL obtain console sessions through the private named-pipe Local Control path and SHALL NOT persist session or launch credentials. Quitting the shell SHALL NOT stop the service. When the service is stopped or unhealthy, the shell SHALL start it through the installed manager and show progress, failure and the location of logs. The shell SHALL NOT execute business operations of its own; product actions go through the console and its HTTP control interface. Host-only features available in the shell, such as update and login-start preference, SHALL be exposed to the console as explicit host capabilities. If Control and Gateway are ready but no local console listener is bound, the shell SHALL identify the console-listener failure separately from service startup and show explicit owner CLI configuration and restart instructions. Retrying SHALL NOT change saved listener settings or start a temporary listener, and the owner SHALL retain the ability to disable the console.

#### Scenario: Second launch

- **WHEN** the user starts HiRoute again while the shell is running
- **THEN** the existing window is focused and no second shell or service is started.

#### Scenario: Service stopped

- **WHEN** the shell starts while the background service is stopped
- **THEN** the shell starts it through the manager and opens the console once Local Control and Gateway are ready, or shows the failure with its log location.

#### Scenario: Quit the shell

- **WHEN** the user quits from the tray
- **THEN** the shell exits while the Gateway keeps serving configured Agents.

#### Scenario: Open in browser

- **WHEN** the user selects open in browser
- **THEN** the default browser opens a fresh single-use launch link and reaches an authenticated console session.

#### Scenario: Local console disabled or unable to bind

- **WHEN** the service is healthy but the saved console is off or its explicit port could not bind
- **THEN** the shell identifies the missing local console, shows how to save loopback with automatic port selection and restart the service, and warns that restart may interrupt active work. Retry only rechecks; it does not overwrite the owner configuration.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219733037

### Requirement: Windows Desktop updates verify unsigned packages by catalog digest

The Windows shell SHALL select a newer stable Windows x64 Desktop release from the trusted website release catalog and download it over HTTPS while the service keeps running. Before any stop it SHALL verify byte count and SHA-256 against the catalog. Windows packages are not code-signed; package origin SHALL be established by the HTTPS catalog and digest, and HiRoute SHALL NOT claim a publisher identity. Release pages SHALL publish each package SHA-256, and documentation SHALL guide users through the unknown-publisher prompt. Replacement SHALL follow the existing orderly-stop handoff: preflight, drain active requests and accepted Worker work with user cancellation, stop the shell and service, replace through the journaled installation transaction, run storage validation and migration, restart and report health. A failure before replacement SHALL restore the old running service; an interrupted replacement SHALL be repaired on the next start or installer run. A manually downloaded newer installer SHALL use the same transaction. The in-app passive update SHALL preserve the existing login-start and tray choices and SHALL NOT wait on an interactive options page. In this release, Windows publication SHALL consist only of the x64 Desktop installer named `HiRoute-<version>-<rev12>-windows-x86_64-setup.exe`. Windows standalone archives of any architecture and Windows ARM64 packages SHALL NOT be published, and release catalog validation SHALL reject a Windows standalone artifact. An unavailable or invalid catalog SHALL be reported as an update-check failure while the current application and service remain usable. A missing or retired Windows catalog SHALL NOT be presented as a failed installation or as proof that no update exists, and the shell SHALL NOT fall back to the released V2 catalog.

#### Scenario: Digest mismatch

- **WHEN** a downloaded package size or SHA-256 differs from the catalog
- **THEN** the update fails before stopping the service and the existing version remains in use.

#### Scenario: Cancel while draining

- **WHEN** the user cancels while accepted Worker work is still running
- **THEN** the work is neither killed nor resubmitted and normal admission resumes.

#### Scenario: Replacement failure

- **WHEN** file replacement fails after the service stopped
- **THEN** the previous verified version is restored and restarted, and the failure stage is reported.

#### Scenario: Successful update

- **WHEN** an update completes
- **THEN** the new version starts with existing sources, plans, Agent configuration and observations intact, and the shell reopens the console.

#### Scenario: Windows standalone archive in a release

- **WHEN** a release catalog candidate contains a Windows standalone archive or a Windows ARM64 package
- **THEN** catalog validation rejects it before publication and released readers never receive it.

#### Scenario: Passive update preserves preferences

- **WHEN** the shell hands off a verified update to the passive installer
- **THEN** installation proceeds without an interactive options page, retains the existing login-start and tray preferences, and returns to a healthy service.

#### Scenario: Windows catalog not available

- **WHEN** the trusted Windows catalog returns HTTP 404 or 410
- **THEN** the page reports that the Windows update catalog is unavailable and offers retry, without claiming that installation failed or that the latest version is installed, changing product data, stopping the service or selecting a V2 artifact.

#### Scenario: Shell replacement fails before the core changes

- **WHEN** a prepared shell replacement fails while the standalone marker and verified Task still refer to the previous installation
- **THEN** recovery restores the verified shell files and captured login/tray/listener settings without reinstalling or draining that unchanged core, retains active work and business data, and a subsequent installer run can proceed through the same ownership checks.

#### Scenario: Manual repair supersedes a prior failure

- **WHEN** recovery reported an earlier replacement failure and a subsequent manual installer commits the verified new core and shell
- **THEN** the persisted startup and Settings outcome reflects the committed installation instead of claiming that the prior version was restored, and interrupted committed-phase reporting completes idempotently through recovery.

#### Scenario: Same-current-installer repair after storage cutover

- **WHEN** storage may have changed and a prepared Windows installation is retried with the same complete accepted current package
- **THEN** early installer recovery verifies that candidate before generic rollback, preserves existing data/recovery material and continues the current installation. A prepared standalone journal can proceed only after the complete current package, target namespace and owned Task definition match; it never registers the older daemon. Reapplication preserves current data and retained recovery material, commits idempotently and checks actual service/Gateway readiness. This is current-installation repair, not support for an unpublished historical Windows source.

#### Scenario: An interrupted installation cannot be safely matched

- **WHEN** the candidate, current files, native registration or prepared journal cannot be verified for safe current-version repair
- **THEN** recovery preserves both program versions, all data and journals, refuses automatic old-program launch and explains preservation, correction of stated prerequisites, exact-current-installer repair where admitted, and diagnostic/support guidance when repair remains unavailable. Installer exit alone does not establish restored service.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219734656

### Requirement: New-model trials compose the existing protected workflow

Home MUST show at most six featured model entries with origin, availability and
version disclosure. Recommendations come only from the selected validated official
feed. When the service provides no featured entries, Home shows no recommendations;
bundled announcements and unrelated catalog entries are not substituted. Users can
still choose models from the available catalog. Models needing a newer client cannot
start a trial.
Home and routing creation open one full-page, four-step flow for trial model, in-place connection,
Primary model, decision connection, selected Agent/presets and review.

Opening SHALL discover Agent choices without reading every Agent's settings. Selecting a known choice SHALL read only that context's model settings, without repeating discovery or reading management Skill/collaboration facets. Choices MAY persist only within the open flow, including publication/mapping; they are hints, never proof of editable, empty or current settings/installation. A new flow or explicit rediscovery refreshes them. Missing, failed or superseded details SHALL block review with retry; results arriving after selection changes/closure SHALL NOT replace current details.
Before mapping, refresh selected model settings, require review of changed selection/context, and retain fresh Preview/Apply checks. Step changes or fresh selected-settings reads MUST NOT alone repeat all-Agent discovery. Mapping refresh/retry SHALL rediscover choices and reread selected settings to recover external native-context/authority changes without closing the flow; it SHALL NOT submit mapping. A changed target requires new user confirmation. Pending-operation reconciliation still matches its original exact context and submitted selection.
Primary chosen while Agent details are pending SHALL persist without preventing default Agent initialization. Late details SHALL NOT replace an explicit Agent or Primary.

Catalog-linked registrations require observed model identity; unlinked saved models require explicit binding selection. Protected input/check/save and subscription-import authority remains.
Primary is prefilled only from a recorded unambiguous fixed-model resolution; a
native alias does not identify a model version. Decision connections can be reused
or created in place; this flow offers no heuristic classification. Review uses the chosen initial threshold (default 0.90), explicit low-progress/corrected-result competence criteria, both groups, saved connections and proposed mapping scope.

Confirmation saves the ordinary draft, publishes it after its exact save and model
facts are confirmed, then uses normal Agent Preview/Apply. Mapping only changes the
selected presets/default; unrelated mappings, permissions and collaboration stay
intact. Flow IDs are retained in memory across retries without new daemon trial
state. Uncertain replies reconcile the same operation and exact resulting revision,
never the preceding successful operation. Completed connections, drafts and Plans
remain visible after later failures. A published Plan can be mapped from Routing
after reload without recreating it.

After a mapping Apply reply, the flow SHALL immediately read the exact selected
Agent settings and reconcile the submitted operation/selection. A completed mapping
SHALL NOT wait for a periodic timer before that read. Pending or uncertain outcomes
retain same-operation reconciliation without resubmission or invented success.

After this flow confirms that the Agent mapping applied successfully, it SHALL wait for matching Plan activity while the trial page and window remain visible. It SHALL query the existing sessions view from that confirmation time at a five-second cadence, skipping an in-flight read. The first matching candidate observed by the flow SHALL remain selectable through "Watch this session", which opens its live routing timeline only on user action. A Plan match SHALL NOT be described as proof of the Agent identity or a newly created, chronologically first session. Closing or hiding the flow stops reads; showing it resumes them. A read failure SHALL offer retry while retaining the model-performance and session-list actions.

#### Scenario: Switch Agent while a previous read is pending

- **WHEN** the user switches A to B and back to A while earlier detail reads remain pending
- **THEN** only the latest A result may become editable; the switches do not rediscover other Agents or read their settings, and a failed read can be retried without inventing defaults.

#### Scenario: Publication succeeds but its reply is lost

- **WHEN** saving a trial draft succeeds and the subsequent publication reply is lost
- **THEN** refresh MUST verify the publication operation and exact Plan revision,
  without repeating either mutation or treating the draft save as publication;
  Agent mapping remains a separate reviewed action.

#### Scenario: Post-mapping session handoff

- **WHEN** the mapping is confirmed and a subsequent session read finds activity using the published Plan
- **THEN** the flow offers Watch this session without automatic navigation; old activity before confirmation is excluded, and leaving the trial page stops waiting.

#### Scenario: Preserve reasoning or use a supported model default

- **WHEN** the trial initializes or changes a selected binding
- **THEN** Preserve a proven selected-Agent setting or explicit user Selection, including its binding and reasoning, subject to normal validation. For a newly selected binding, use only a supported authoritative model default from that binding's native reasoning capability. A valid discrete default_profile must be an exact member of profiles; when no authoritative supported default exists, require an explicit choice. Do not choose a low/high/max profile from list order or Regular/Primary role. A fixed/no-control capability can omit reasoning where its existing contract permits it; missing reasoning for selectable discrete/toggle/budget controls is not provider-default and must not bypass their existing required-choice validation. Unsupported or stale choices remain visible and block review rather than silently changing. This trial policy does not change ordinary PlanEditor role defaults or add a new provider-default wire mode.

#### Scenario: Distinct identities and invalidated Primary provenance

- **WHEN** configurations share a display name, an Agent changes, or selected capability evidence changes
- **THEN** Same-name configurations and bindings SHALL remain distinct; show a proven current source or explicit multiple/unconfirmed-source state. Review/mapping SHALL preserve exact configuration, binding, source and reasoning. Changing Agent SHALL invalidate previous-Agent Primary provenance and reject stale results; absent a proven current routable Primary, require user selection. Valid explicit settings survive same-Agent refresh; invalid choices remain visible and block continuation. Trial and Primary SHALL NOT be the same configuration.

#### Scenario: Stable readable aliases and consistent trial wording

- **WHEN** a trial is created, retried, restored or reviewed
- **THEN** New trial aliases SHALL be readable and fixed for their persistence/retry identity. Existing aliases and saved drafts SHALL remain unchanged; collisions or uncertain replies SHALL NOT silently regenerate aliases or repeat writes. Preserve previous-routing restoration. Version/source uncertainty SHALL remain accessible without hiding blocking credential/capability/obsolete-client errors. Trial review and division-draft confirmation SHALL use Low-score protection and Competence floor consistently.

#### Scenario: Equal model names differ in source

- **WHEN** two configurations share a name but have different sources/bindings
- **THEN** both remain selectable with visible source and accessible identity; all summaries/published selections preserve the chosen binding.

#### Scenario: Current selection cannot be proven

- **WHEN** DSH exposes only an allowlist
- **THEN** Primary remains unset until selected; a recent request or catalog order cannot provide a default.

#### Scenario: Reasoning defaults and late refresh

- **WHEN** a new binding has a supported model default, no default, or stale/invalid capability results arrive after an explicit selection
- **THEN** use only its supported model default or require an explicit choice; preserve user/proven choices, reject stale results, and never turn missing discrete reasoning into provider-default or an automatic maximum.

#### Scenario: Stable publication identity

- **WHEN** a new readable alias is saved and publication response is lost
- **THEN** reconciliation checks the same identity; no silent alias regeneration/repeated create occurs, and existing aliases remain unchanged.

#### Scenario: A routable Plan session lacks Agent attribution

- **WHEN** mapping is confirmed and post-confirmation activity matches the Plan while its Agent attribution is unavailable
- **THEN** Watch this session remains offered under the Plan/time match without asserting Agent identity; the view opens only on user action.

#### Scenario: Supported default versus missing default

- **WHEN** a new Flash binding exposes low/high/max with default high and a new Primary binding exposes low through max without a default
- **THEN** Flash starts at high; Primary requires an explicit reasoning choice; a proven or manual low selection remains low, and review/publication reflect that exact setting.

#### Scenario: Trial default rubric does not follow UI language

- **WHEN** a new trial is reviewed in Chinese or English, including a language change before review
- **THEN** model-facing default instructions and criteria remain the same English text; only help text is localized. Brief dissatisfaction about the completed core result counts as negative feedback without requiring a diagnosed technical error; new requirements or bounded preferences are not automatically core failures. Preserve user-edited prompts, explicit default/custom intent, saved drafts and already published frozen contents.

#### Scenario: Reuse a saved connection or explicitly choose an unlinked model

- **WHEN** the user selects an available saved API or subscription binding, including a user-declared model without catalog identity
- **THEN** the trial uses that exact current routable binding and actual configuration without another source save, credential prompt, automatic scan or inferred catalog match; an unlinked model's catalog version remains explicitly unconfirmed.

#### Scenario: A successful save returns a model without catalog identity

- **WHEN** a completed connection save exposes an exact current executable receipt binding without a catalog ID for the recommendation
- **THEN** the connection remains visibly saved; the user can explicitly choose the actual saved model after seeing its source and identity basis. The flow does not demand another connection, assign a catalog identity, repeat Apply or claim inference verification. Read failure offers read-only retry of the original saved result.

#### Scenario: Starting trust survives review and connection detours

- **WHEN** a user selects Entrusted zero or an arbitrary threshold such as 0.723, opens or saves a connection, changes Agent, enters review, edits the threshold and returns
- **THEN** the latest exact chosen/editor judgment survives custom-extension and language detours as well as review and is saved/published only on explicit confirmation; the initial default 0.90 never overwrites a choice. Initial setup and unpublished review do not show active-version evidence or request replay, and existing published-route protection and evidence behavior remain unchanged.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/228#issuecomment-6440850201

### Requirement: Home lists routing suggestions that need attention

The Desktop Home "Needs your attention" section SHALL list each current level suggestion for difficulty-routing Plans and two-group branches: step up, step down, create a division draft, or return to previous routing. Suggestions SHALL come from the same backend computation as the model performance view. Each item SHALL name the Plan and its published trust level, state the facts behind the suggestion (scored stages, scoring coverage, contributing sessions, competence rate, protection count and, for a level change, the replay share at the suggested level), carry the version disclosure and the actual suggestion evidence window/cohort of the model performance view when they apply, and offer two actions. "View evidence" opens the Plan's model performance view. The primary action opens the existing draft, division-draft or Agent settings Preview/Apply. Neither action SHALL change routing until the user publishes or applies. An item SHALL disappear when its condition no longer holds for the active published revision. While Home is not shown, navigation SHALL mark that suggestions exist. Suggestions SHALL be presented as optional; keeping the current level SHALL remain a valid outcome, and trust levels SHALL NOT be presented as onboarding progress or completion. When no suggestion applies, the section SHALL say so without inventing progress. A failed suggestion read SHALL be shown as a retryable read failure and SHALL NOT be shown as an absence of suggestions.


Suggestion failures SHALL be isolated per Plan: an oversized or unavailable Plan appears as a retryable unavailable item alongside other valid suggestions. The response SHALL provide suggestions and unavailable arrays, with unavailable entries naming the Plan and a stable scope_too_large, unavailable or stale reason. Suggested branches SHALL expose their counts, cohort, protection count, identity-stability disclosure and replay at the suggested threshold without returning full report rows or session lists. A failed whole query remains a retryable failure. Desktop SHALL refresh on Home display, window visibility and the user's own publish/apply actions, and otherwise every five minutes while visible; navigation and Plan markers reuse this read. Cached evidence SHALL still validate reader authority and all relevant control, observation, Worker and retention changes, including expiry with time.

#### Scenario: Two Plans, two directions

- **WHEN** one Plan qualifies to step up and another to step down
- **THEN** Home lists both items with their facts, and opening the step-up draft changes nothing until it is published.

#### Scenario: Condition resolved

- **WHEN** the user publishes the suggested level for a Plan
- **THEN** that Plan's item disappears, and no new suggestion appears for it until the new revision reaches the minimum scored stages.

#### Scenario: Read failure

- **WHEN** the suggestion query fails while other Home reads succeed
- **THEN** the section reports the failed read with a retry, and the other Home content stays visible.

#### Scenario: One Plan exceeds the read limit

- **WHEN** one enabled Plan exceeds its read bound while another has an available suggestion
- **THEN** Home shows the healthy Plan's suggestion and a retryable failure for the oversized Plan, without calling the result empty or disabling the healthy Plan's actions.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6236472105

### Requirement: Trial setup reuses saved models and explains every blocked continuation

Trial setup SHALL derive review eligibility, inline explanations and the complete blocker summary from one consistent current readiness result. It SHALL distinguish incomplete or invalid user choices from pending work, failed reads, saved-but-unresolved model selection and missing authority. Every gate that prevents review SHALL have a localized reason and a relevant field or recovery action. Required user choices SHALL be visibly labelled Required and accessibility-linked to their explanation. A missing explicit reasoning choice SHALL be visible as required, without inventing provider defaults or automatic extreme profiles. Fixed reasoning and an available generated route name SHALL NOT become false missing fields.

For ordinary incomplete/invalid user input, Review changes SHALL remain an accessible validation action. Activating it SHALL list all current blockers in form order, open and focus/scroll the first affected field above fixed actions, and issue no write or model call. Summary items SHALL navigate to their own controls, and errors SHALL update as the user corrects them without losing other entered values. Before first submission, required labels and neutral guidance are shown without treating every untouched field as an error. Valid input enters review once through the same guards.

An action disabled by active loading/saving/reconciliation or unavailable authority SHALL visibly explain that state and provide supported recovery where applicable; a completed read failure SHALL not remain an indefinite loading indicator. Saved sources SHALL remain described as saved while model choice or verification is unresolved. Retry SHALL be scoped to the retained original operation/result and cannot create duplicates. Language changes, late reads, role switching and closed dialogs SHALL not substitute stale identity or validation results. All existing credential, authority, preview, publication and mapping checks remain effective.


Trial setup SHALL offer selection of actual saved API and subscription models without recreating connections or re-entering their saved credentials. Exact source and binding identities, resolved through current routable candidates, govern selection; display names or absence of a catalog link do not establish or negate that executable identity. A catalog-linked registered selection retains its selected-model observation and catalog-admission requirements. An unlinked saved model may be explicitly selected as its actual configured identity with its catalog version unconfirmed; it SHALL NOT silently acquire the recommended model's catalog identity, verification or capabilities. Save completion and selected-model resolution are distinct: reconcile the original operation and exact returned bindings, preserve completed saves, perform bounded current-state rereads and never repeat Apply to repair lost replies or stale UI state.

The trial's connected-model control SHALL list only actual persisted API/subscription
bindings, with their source names and current availability. It SHALL NOT mix
unconnected catalog entries into that chooser. Its Add a model action opens the
normal Models tab; returning preserves the same trial instance, chosen bindings,
route name and exact trust and refreshes current availability without selecting a
new model implicitly. Catalog recommendations remain separately selectable and do
not create a second recommendation card merely because an unlinked saved binding
has the same display name.

Connect here SHALL offer existing connection points inside the connection dialog,
alongside creating a new connection. Selecting an existing API point uses the
normal exact-source append/check/Preview/Apply flow and retains its credentials,
existing models and enabled state. A subscription stays with its existing connector
owner and authorization flow; it is never cloned into an API-key form. Protected
input replacement still requires its existing guard, and the old connection dialog
must close before the selected source's flow takes input ownership. Choosing a
saved model is distinct from adding a model to an existing connection point.

Trial section headings SHALL use consistent typography for peer model, decision
and Agent sections. Required items use a small visual marker with an accessible
required label and one shared explanation; missing values retain their existing
inline errors and Review changes focus behavior.

Adjustable reasoning SHALL use a compact bordered control next to its label, with
an explicit disclosure affordance, keyboard focus and current value.
Native profile names remain in English in both UI languages; the exact profile
sent to Preview/Apply, provider defaults and reasoning policy remain unchanged. Fixed
reasoning remains non-editable and invalid choices retain actionable field errors.

The initial trial threshold defaults to 0.90 but is selectable before review using the shared continuous control and directly clickable presets, including Entrusted at zero. The chosen exact thousandth value SHALL survive connection detours, refresh, model/Agent changes, review/back/edit, ordinary save, publication and reopening. Review SHALL use the latest chosen/editor value rather than reset it to 0.90. Initial setup SHALL not show active-version evidence-loading copy or request historical replay. Custom extensions hide and park built-in controls. Existing ordinary authoring defaults, competence floors, English default prompts, protection, permissions and explicit publication remain unchanged.

Protected connection editors SHALL pass their existing replacement guard before a saved-model choice would replace them. A rejected replacement retains the editor, temporary credentials, selected identities and selection context; a Primary selection SHALL NOT release an unrelated trial editor. A pending or uncertain save retains the original operation, accepted digest, idempotency key and candidate and SHALL block review with a visible same-operation read-only recovery action. A confirmed save awaiting binding selection remains recoverable across selection of another model and review/back, without a new Apply or empty new-source form.

Selection SHALL join exact current management and executable-candidate facts with required current catalog admission only under one coherent client/session/authority read. All consumed local inputs, catalog identity and relevant connector runtime authority must be proven current at the shared boundary. Superseded reads may retry the whole read once; continued change or missing proof is a retryable failure that retains user input. Unlinked models SHALL NOT require an unrelated catalog-availability scan or acquire current catalog admission from retained descriptive data. Existing Preview and Apply authority remain unchanged.

The generated route name SHALL be valid under the domain's 128 Unicode scalar-value limit and preserve readable portions of both model names. Readiness, visible errors and the submitted editor SHALL use the same effective name. Manual blank or overlong input remains visible and invalid; explicitly clearing the field SHALL NOT restore a default. Built-in judgment edited in review SHALL remain parked across custom-extension, language and back-to-setup transitions. Unpublished review uses initial-trial context without current-version evidence copy or historical replay; existing published routes retain their evidence and replay semantics.

Trial selection SHALL also offer an otherwise current and executable retained API binding without V2 management ownership using its exact captured source/binding/configuration identity and safe source description. This read-only choice SHALL NOT invent a management record or action, infer identity by name, perform another connector discovery, or bypass a V2 owner's existing checks. It SHALL NOT create a subscription authorization fallback. An explicit later trial binding choice SHALL survive reconciliation of an earlier saved operation, including a different binding of the same model; declining protected draft replacement SHALL preserve both selection and generation.

A failed descriptive recommendation read SHALL show a localized independent retry without treating the catalog as empty or blocking previously proven saved connections. Retry SHALL preserve selections and inputs, be single-flight, ignore stale/closed-flow responses, and not grant executable admission merely because recommendations load.

Trial names SHALL satisfy the complete existing `AgentPlanDisplayName` domain validation, including Unicode scalar length, whitespace/control, URL and sensitive-marker restrictions. If an automatically generated combination is invalid, use a safe localized default; manually entered invalid text remains visible with a specific error and is never silently replaced. Setup and review SHALL preserve the same text and domain boundary, accept 128 Unicode scalar values including supplementary characters, explicitly reject 129, and never silently truncate by UTF-16 units. An invalid name cannot be published.

#### Scenario: Add a model without restarting the trial

- **WHEN** the user chooses Add a model in the connected-model control and later returns from the Models tab
- **THEN** the original route name, exact trust and chosen bindings remain, current availability is reread, and browsing/returning causes no configuration write or inference. Any edited model form still passes its normal replacement guard.

#### Scenario: Reuse a connection point from Connect here

- **WHEN** the user chooses a saved API or subscription point inside the connection dialog
- **THEN** the previous guarded form closes, only that exact source's normal append flow proceeds, and existing credentials/models remain owned by that source. Unknown or changed sources cannot silently fall back to creating a new connection or a different source.

#### Scenario: Retained API candidate without V2 management ownership

- **WHEN** a supported retained API binding is currently admitted as an executable routing candidate but has no V2 management owner
- **THEN** it is selectable for either trial role by its exact captured identity and safe source description without inventing an editable management source; a disabled, credential-invalid, mismatched or V2-owned invalid entry is not selected through this fallback. Existing subscription proof remains required.

#### Scenario: A later same-model binding choice wins

- **WHEN** a saved binding A awaits recovery and the user explicitly selects binding B of the same model before A's reread succeeds
- **THEN** B remains selected; A remains available as a saved result and can be selected explicitly, with no repeated Apply. Declining a protected replacement preserves the prior choice.

#### Scenario: Recommendations recover independently

- **WHEN** the initial descriptive recommendation read fails while an existing saved binding is usable
- **THEN** the saved binding remains selectable and a visible retry independently rereads recommendations; retry failure and stale responses preserve the user's selections and do not establish admission.

#### Scenario: Domain-valid route names in setup and review

- **WHEN** a model name would make the generated route name violate the domain URL or sensitive-marker rules, or the user enters 128 or 129 Unicode scalars in either editor
- **THEN** an invalid generated name uses the safe localized default, manual text is retained with a concrete error, 128 scalars are accepted and 129 are rejected without truncation, and only a valid exact name reaches Preview/publication.

#### Scenario: Reject an unknown trial snapshot schema

- **WHEN** the current session receives a trial snapshot with an unexpected schema identifier or invalidated read authority
- **THEN** it returns a safe read failure without exposing candidates or raw data, and selection does not advance on that response.

#### Scenario: All missing fields are discoverable together

- **WHEN** the trial model lacks a selected usable binding and Primary reasoning still needs an explicit choice
- **THEN** Review changes lists both blockers and links to each control; focus lands on the first, fixing it leaves the second visible, and no write occurs.

#### Scenario: A saved connection is not reported as absent

- **WHEN** save is complete but a post-save read fails or its exact binding still needs selection
- **THEN** the page shows Connection saved with the concrete unresolved state and scoped recovery; it never asks to create the same connection again or treats another source with the same name as its result.

#### Scenario: Reasoning requirement is visible

- **WHEN** the selected candidate has discrete/toggle/budget reasoning with no supported authoritative default
- **THEN** the field says Required and asks for a choice; an unset value blocks review with a field-specific message and never silently becomes Low, Max or provider-default. Fixed/no-control candidates remain valid under their existing contract.

#### Scenario: System and field blockers stay distinct

- **WHEN** selected Agent settings cannot be read, authority is lost, or model facts are loading
- **THEN** the exact system state and supported retry are visible, any disabled action has a persistent reason, and prior healthy presentation cannot grant write permission or masquerade as a missing user input.

#### Scenario: Initial trust is a visible draft choice

- **WHEN** a new built-in trial opens and the user clicks a preset or drags between presets
- **THEN** the default is Trial 0.90, controls accept exact thousandths through zero, one live threshold caption changes, and the chosen value reaches review and publication unchanged without evidence reads or automatic mutation.

#### Scenario: Keyboard and narrow-window correction

- **WHEN** a validation attempt identifies a field inside a collapsed section or near sticky footer controls
- **THEN** the section opens, keyboard focus and the complete field/error remain visible above the footer, all summary links work, and cancel/retry preserve unrelated input.

#### Scenario: Reuse a saved connection or explicitly choose an unlinked model

- **WHEN** the user selects an available saved API or subscription binding, including a user-declared model without catalog identity
- **THEN** the trial uses that exact current routable binding and actual configuration without another source save, credential prompt, automatic scan or inferred catalog match; an unlinked model's catalog version remains explicitly unconfirmed.

#### Scenario: A successful save returns a model without catalog identity

- **WHEN** a completed connection save exposes an exact current executable receipt binding without a catalog ID for the recommendation
- **THEN** the connection remains visibly saved; the user can explicitly choose the actual saved model after seeing its source and identity basis. The flow does not demand another connection, assign a catalog identity, repeat Apply or claim inference verification. Read failure offers read-only retry of the original saved result.

#### Scenario: Starting trust survives review and connection detours

- **WHEN** a user selects Entrusted zero or an arbitrary threshold such as 0.723, opens or saves a connection, changes Agent, enters review, edits the threshold and returns
- **THEN** the latest exact chosen/editor value survives and is saved/published only on explicit confirmation; the initial default 0.90 never overwrites a choice. Initial setup does not load active-version evidence or replay, and all existing protection and custom-extension rules remain unchanged.

#### Scenario: Protected connection draft and saved-model choice

- **WHEN** a user has edited a connection and chooses an existing trial or Primary model
- **THEN** a replacement of that editor asks through the existing guard; refusal retains the draft, credentials and selection, while acceptance selects the exact binding. A Primary choice that does not replace the trial editor leaves it intact.

#### Scenario: Save recovery survives leaving the connection card

- **WHEN** a user already has a usable trial model, starts another connection and its save is uncertain or completes before a binding is chosen
- **THEN** uncertain completion blocks review and offers read-only recovery of the same operation; a confirmed result remains selectable after another binding is used for review and the user returns. Neither path repeats Apply.

#### Scenario: One coherent model selection view

- **WHEN** candidate reasoning, routability, source state or relevant connector authority changes while trial selection facts are read, including the same binding identity
- **THEN** no mixed snapshot grants selection or review. At most one complete reread may recover a stable view; continuing change or missing authority remains visibly retryable, while unrelated reads do not wait for the entire trial read.

#### Scenario: Parked judgment and initial review context

- **WHEN** review changes 0.90 to 0.723, switches to a custom extension, returns to setup, changes language and switches back to the built-in decision model
- **THEN** 0.723 and the other parked built-in judgment survive repeated detours; unpublished review shows no active-version evidence or historical replay, while published routes retain their existing evidence behavior.

#### Scenario: Unicode route name matches submission

- **WHEN** model names would produce a default above 128 Unicode scalar values, or a user supplies a blank, 128-character or 129-character name including supplementary-plane characters
- **THEN** a generated name is valid and readable, the exact manually entered value is retained, blank and 129-character input receive field errors, and the same valid effective name reaches the editor after correction.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/228#issuecomment-6460225695

### Requirement: Trial connection reuse retains the selected model intent

Choosing an existing connection from Connect here SHALL retain the already selected
trial model. The interaction SHALL request only the connection and an editable
upstream request model ID, rather than reopen the general model chooser. The ID
defaults to one unambiguous saved catalog binding or the selected connection
product's exact executable catalog mapping. Display names and internal model
configuration IDs SHALL NOT be guessed as request IDs. An unavailable or ambiguous
default leaves the field empty with a localized explanation.

An exact usable saved binding SHALL be reread and selected without creating an
Operation or checking a provider directory. Otherwise the existing API or
subscription owner SHALL check the selected source and append only the requested
supported model through normal Preview/Apply. The current source revision,
credential owner, other models and enabled state SHALL remain protected. An edited
ID does not establish catalog identity, capabilities or verification. An unsupported
ID or failed read SHALL retain the ID and explain the failure without choosing
another model or running paid inference.

A completed save SHALL be reconciled to its exact returned source/model/binding and
current routable trial candidate, then return selected without a second chooser.
An uncertain save SHALL only observe its original accepted Operation and SHALL NOT
submit another Apply. Returning to setup SHALL retain that submission and offer
Review connection result to reopen it; ordinary review cannot advance past the
unresolved connection. Cancellation and stale results SHALL NOT replace later user
choices. Other trial selections, explicit route name and exact trust remain intact.

#### Scenario: Existing binding

- **WHEN** the selected connection already has one usable binding for the request ID
- **THEN** confirmation selects its exact current identity without adding a model,
  registering a credential or reading a provider directory

#### Scenario: Add the selected target

- **WHEN** the existing API or subscription owner confirms the requested supported model
- **THEN** only that model is appended, existing membership and credentials remain
  unchanged, and the resulting binding returns selected in trial setup

#### Scenario: Unknown or edited ID

- **WHEN** no unambiguous source-specific default exists, or the user edits the ID
- **THEN** the field remains editable and explicit; unsupported input gets a local
  explanation without granting the recommended model's identity or capabilities

#### Scenario: Save reply is lost

- **WHEN** a submitted append has an unknown result or its saved binding cannot be reread
- **THEN** Refresh reconciles only that submission and never repeats Apply

#### Scenario: Several subscription sign-ins exist

- **WHEN** both native and independently authorized subscription sign-ins are available
- **THEN** saved-source reuse locates only the persisted source's original candidate and credential owner, bound to its current source revision; it does not choose the first provider match or request a different sign-in
- **AND** the original owner still performs a new Check and the normal Preview/Apply; consumed saved validation is not reused as new approval
- **AND** normal saved-source renaming, model removal and prior targeted appends do not invalidate the credential-owner lookup when its account, candidate, committed validation and successful-save identity remain valid

#### Scenario: Selection changes while a save is unconfirmed

- **WHEN** the user returns to setup, selects another trial model or Agent, and later confirms the original save succeeded
- **THEN** confirmation ends the original recovery state without replacing the newer selection or repeating Apply; unresolved or mismatched receipts remain protected

### Requirement: Trial guidance progressively explains routing and protection

The trial setup SHALL initially show a concise explanation matching the active classifier and an accessible disclosure of the complete current protection explanation and applicable trial supplement. System One explains suitable work for the trial model, complex work for Primary, and Primary for a turn when low-score protection applies. Local rules explain current task signals and no low-score protection. Custom extensions explain their own choice and that HiRoute adds no trust threshold or low-score protection. The full explanation retains score eligibility, the previous actual stage, problem relevance, conservative protection when unknown, difficulty selection after resolution or irrelevance, conditional continuation of Primary and mode-specific exceptions. Toggling, changing language or classifier SHALL preserve other input and cause no backend action. The candidate area and existing protection policy remain unchanged.

#### Scenario: Built-in protection

- **WHEN** System One is selected
- **THEN** the concise explanation is visible by default and the disclosure explains the complete applicability conditions without promising every low score triggers a change

#### Scenario: Other classifiers

- **WHEN** local rules or a custom extension is selected
- **THEN** the concise and expanded explanation describe that mode without claiming built-in protection

#### Scenario: Disclosure and localization

- **WHEN** the user operates the disclosure by keyboard or changes language or classifier
- **THEN** the correct text is available without duplicate focusable hidden content and without a write or loss of unrelated input

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/246#issuecomment-6461014114

## Implementation references

- [Retain the selected trial model when reusing a saved connection](https://github.com/higress-group/HiRoute/issues/263).

- [Native updater](../../../apps/desktop/src-tauri/src/updates.rs) and
  [installer handoff tests](../../../apps/desktop/src-tauri/src/updates/installer/control/tests.rs).
- [Desktop preferences](../../../apps/desktop/src/ui/preferences.ts) and
  [product shell](../../../apps/desktop/src/product).
- [Release identity](../../../crates/host-runtime/src/releases.rs) and
  [macOS installation guide](../../../docs/macos-installation.md).
- [Earlier new-model trial source SPEC](https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235502571).
