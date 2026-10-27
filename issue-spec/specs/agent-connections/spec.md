# Agent connections and reversible native configuration

## Purpose

HiRoute can configure model access and collaboration independently in an Agent's
selected native context. Native files, local model grants, Skill ownership, and
published routes converge through the common recoverable settings workflow.

## Requirements

### Requirement: Settings confirmation reads its current target

After a settings change, Desktop SHALL read the exact selected context and confirm
its current result without first scanning every other Agent. The read SHALL retain
the same current Session, authority, revision and publication checks as a full
snapshot. Unrelated retained settings SHALL become unverified and be read before
their next use. Hidden pages SHALL defer refresh work until activated. Trial draft
save, publication and operation confirmation SHALL NOT launch unrelated full-Agent
or subscription scans ahead of the active workflow's result; affected background
summaries are invalidated and refreshed when their consuming view becomes active.

#### Scenario: Save while unrelated views have been visited

- **WHEN** an Agent is saved or a trial draft is published after visiting other pages
- **THEN** confirmation reads the changed target, hidden Agent pages do not scan, and opening an invalidated view reads current facts before presenting them as verified; failed readback remains unconfirmed without resubmitting.

### Requirement: Model and collaboration facets have independent intent

Each facet MUST use keep, configure, or restore, with omission meaning keep.
Model settings own default/Fixed/allowed Plan routes; collaboration owns the shared
Skill and explicit (default) or delegate_by_default trigger mode, never a caller
Worker allowlist. Keeping another facet cannot read/validate its unused native files.
Collaboration configuration needs neither active publication nor a plan; an empty
runtime directory is a legitimate outcome. A collaboration-only Qoder or OpenCode context
cannot acquire a fake model grant/token/file effect or ingress capability merely
because its model settings are absent/malformed.

The current native model surfaces are:

| Agent | Model configuration | Collaboration resource context |
| --- | --- | --- |
| Codex | One independent CLI profile by default, or explicit shared-root takeover | Actual native user root, independently of model profile |
| Claude Code | Ordinary `claude` user settings and three preset mappings | User/project Skills through native configuration |
| Qoder | Explicit supplementary Plan providers; native default remains independent | Selected normally logged-in native context |
| Pi | Per-Plan owned members in user models.json | Selected native Skills/resources |
| DSH | Public standard Web/home composition and per-Plan provider rows | Public native Skills/CLI capabilities |
| OpenCode | Supplementary per-Plan native providers; defaults remain independent | Selected native user Skill/resource context |
| Hermes | One selected effective profile default main model using a published Plan alias | Profile-local collaboration and independent management Skills |

A Fixed route always retains exact source/account/physical model. Selecting a plan
never implies a Worker process or shared main-Agent ownership of that plan.

HiRoute SHALL manage management and collaboration Skills as distinct resources alongside the model facet. Each uses keep, configure and restore, with omission meaning keep. Either Skill can be enabled first or both requested together. Changing or restoring one SHALL preserve the other and model settings. Restoring model settings SHALL preserve both Skills unless their own restore intents are selected. Ownership SHALL distinguish the effective file target and Skill identity, preserve shared references and identically borrowed files, and reject foreign content conflicts. Existing accepted operations and recovery references SHALL retain their original meanings. Collaboration owns delegation-trigger policy when both Skills are loaded. Disabling a Skill SHALL report externally retained content and required native reload rather than claim revocation of same-user CLI access.

Management preparation is available for all listed Agent families and requires neither a Plan nor a model grant. Hermes collaboration follows the same independent intent and trigger policy as other supported initiators, without adding a Hermes Worker executor. Neither Skill preparation nor model routing creates a new CLI authority.

Prior contract source: https://github.com/higress-group/HiRoute/issues/125#issuecomment-6186244519

#### Scenario: Prepare Hermes management without model connection

- **WHEN** a Hermes profile requests management-Skill preparation with its model facet kept
- **THEN** only the independent management resource is prepared; no Plan, model grant or collaboration setup is required.

#### Scenario: Both Skills in one context

- **WHEN** the owner enables management and collaboration in either order or in one settings request
- **THEN** both resources and their statuses coexist independently without replacing one another or changing model settings.

#### Scenario: Independent restore across restart

- **WHEN** both Skills are prepared and one is restored before or after service restart
- **THEN** only that owned resource/reference is removed, the other remains usable, and pending retry retains its original accepted operation.

#### Scenario: Installer and shared-root coexistence

- **WHEN** identical Skill content already exists from the installer or another context references the same target
- **THEN** HiRoute preserves borrowed ownership and shared references; restoring one context does not delete external content or another context reference, and retained native availability is reported.

#### Scenario: Foreign edit and interrupted apply

- **WHEN** a resource changes after Preview or a multi-resource operation is interrupted
- **THEN** the existing stale/ownership and recovery rules apply to exact targets; no successful state hides unfinished effects or removes unrelated content.

#### Scenario: Collaboration without model routing

- **WHEN** a user enables collaboration while OpenCode continues to use its own model provider and no HiRoute Plan exists
- **THEN** the owned Skill can be installed independently, no model grant/file effect is created, and the runtime directory truthfully contains no delegate target.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258607139

### Requirement: Settings bind current facts once through one transaction

V2 settings status supports explicit `full` and `model` projections. Omission
retains the existing full response and `include_skills` semantics, and the default
projection is omitted when serialized. Unknown projections and `model` combined
with `include_skills=true` or `effective` are rejected. Model projection reuses the
same strict model evaluation, including pending/recovery states, Configure/Restore,
native files, current grants and publication; it does not evaluate management or
collaboration. Omitted facets mean not projected, not unconfigured. This extension
requires a matching host/daemon; rejection by an older daemon remains a retryable
read failure, never evidence of empty configuration.

Preview/Apply MUST bind context, target file ownership, required native capability,
selected plans/protocols, resource revisions, and dependency digest, reproducing
facts at Application admission. One save click is explicit intent without a second
native authorization prompt or an extra scan just to create unseen confirmation text.
A pre-admission conflict reports not-submitted and retains edits; retry creates fresh
Preview from current facts, not endless replay of a stale digest. Current service
unavailability preserves input. Login-item state is not configuration permission.

Configuration retains unrelated/unknown fields and protected original recovery
material. Ordinary Configure/Edit prepares dependencies/files, activates service
publication, then switches the native file tail; this is recoverable, not a global
cross-store atomic pause. Ordinary Restore conditionally restores native files
before withdrawing their grants. An already sealed service result with a pending
owned file tail retains its same-Operation recovery guard. Registered historical
revoke tails keep their own existing ordering and cannot regrant durably withdrawn
authority merely because later cleanup fails. Rollback handles only owned effects.

File conflict before a switch leaves the original connection effective, preserves
user edits, and can abandon proven untouched staging to release the writer. Repair
then uses the applicable fresh operation or exact pending-tail retry, never one to
bypass another's ownership. Damaged recovery proofs or unverifiable shared effects
retain the recovery gate. Unmanaged unreadable targets cannot block service startup;
actual managed access/recovery still checks type, contents, and identity. No extra
owner/group-read/write-bit check or chmod is introduced.

#### Scenario: Read model settings without unrelated Skill resources

- **WHEN** a consumer requests model projection and an unrequested Skill resource is unreadable
- **THEN** valid model facts remain readable with their original proof checks; the full query retains its existing Skill checks and failures.

### Requirement: Connection bearer is separate from provider and Worker secrets

First model configuration generates a per-connection 256-bit bearer from system
secure randomness and stores it in the existing encrypted grant store. Ordinary
route/scope edits retain its value but publish updated authority; equivalent save
reuses it. Claude and Codex connections are independent; identical normalized Codex
scope/CODEX_HOME shares one connection token.

Connection detail masks state and permits explicit replacement/regeneration. Custom
values are 16–128 ASCII letters/digits or `._~-`, entering via protected local input,
never ordinary journal, WebView, diagnostics, or provider-Key storage. Successful
rotation makes old values invalid for new requests and updates native consumption
in the same Operation. Failure/stale facts preserve the old connection/token.
Codex's owned local auth/config may contain its allowed plaintext copy; Claude's
managed helper reads the current grant instead of copying bearer into native settings.
Provider API Keys and automatically injected run credentials remain separate.
Released Worker CLI does not require this token.

### Requirement: Capability checks are specific to the requested action

Configuration depends on actual writable fields/paths, protocol expressibility,
required catalog facts, and current authority, not a generic client version allowlist.
Unknown/old/new labels or failed diagnostic version reads cannot alone block settings,
preview, restore, or ordinary managed launch. Missing optional usage/refusal/reasoning
metadata cannot be a blanket save gate; exact client window/protocol/vision declarations
still require proof. Version labels do not enter settings business identity.

Borrowed Claude Worker launch has the single narrow host-managed-provider lower-bound
check in [Worker platform](../worker-platform/spec.md); it is not a save/discovery gate.
Diagnostic version execution clears inherited credentials and uses explicit writable
disposable HOME/native/cwd roots, stopping its child chain before cleanup, so native
startup side effects cannot fall back into a daily home. Ordinary Claude settings
Preview/Apply does not re-run `claude --version`. An unknown native provider in scan
cannot make a valid executable/resource disappear when switching to an authorized plan.
Only the action requiring an unproven capability is unavailable.

Startup recovery SHALL prime its protected-input registry from current native configuration
without requiring informational native version probes. It SHALL retain permission findings
and exact source-revision checks before reading a recovered secret. Configuration priming
does not establish executable, managed-launch or connection authority; explicit Agent
inventory retains its complete discovery behavior.

Explicit Claude authentication and collaboration checks MUST replace inherited
HOME, native configuration and temporary-directory selectors with disposable
probe-owned roots. Both the generic temporary directory and Claude's dedicated
`CLAUDE_CODE_TMPDIR` MUST use the probe workspace, so native startup cannot fall
back to a shared user temporary directory. The check MUST stop its owned process
scope before removing those roots. An authenticated loopback Messages request
and successful native completion remain necessary authentication evidence;
isolated environment setup alone does not establish that capability.

Hermes connection admission SHALL depend on actual plugin/configuration interfaces and required transport/header delivery. Older version labels or missing optional reasoning replay SHALL NOT alone block a connection. CLI/management and Desktop guidance SHALL recommend a release with the upstream reasoning-replay fix and explain that older versions may omit reasoning content on tool continuations. Missing required integration interfaces still reject before native writes with actionable guidance, without automatically upgrading Hermes. Passive scan, independent management-Skill preparation, owned restore and accepted-operation recovery retain their action-specific capability and ownership checks.

#### Scenario: Reject incompatible Hermes integration without blocking owned restore

- **WHEN** a Hermes installation lacks a required integration interface but its owned restore effects remain safely expressible
- **THEN** new connection reports incompatibility before native writes, while existing recovery/conditional restore follows its persisted target and ownership rather than a blanket version gate.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165296337

### Requirement: Configuration UI reports applied state, not invented verification

Agents share enable, adjust, disable, and blocked-action recovery. Trusted new
Codex/Claude configuration may preselect the sole enabled route in a draft; zero
or several require explicit choices. Existing mappings/invalid selections stay
visible and are not auto-replaced. Cancel/draft neither writes nor grants.

Primary controls are route/default choices. Advanced controls retain native-model
retention, root/profile selection, and independent Claude presets, with no read-only
window preview in the enable dialog. Context lives in plan editing. Disable restores
the selected facet through its independent detail action, not another toggle in the
edit dialog. Model validation status or client-specific paid-validation buttons
cannot obstruct normal save or masquerade as configuration state.

Codex profile first-save may enable and copy a shell-specific launch command only
after the exact Operation succeeds and current target is reread. Pending/failed/
cancelled is neither configured nor copied. Clipboard failure preserves completed
configuration and offers manual copy, not replay. Claude save explains ordinary
entry/preset use without claiming a real model call. Independent isolated/live
checks retain their actual diagnostic result and cannot establish task completion.

### Requirement: Codex has one owned model slot per CODEX_HOME

Independent profile and root takeover are mutually exclusive in one normalized
CODEX_HOME, including pending Operations; file/context-ID differences cannot create
parallel slots. Accepted state binds selection, target, context, grant, and recovery
ownership across restart. Mode change requires complete restore before reconfigure.
Independent mode manages one profile, applies to CLI only, and does not alter root
provider/default or advertise Desktop configured. Advanced root explicitly affects
CLI/Desktop sharing that scope; discovering one surface does not require installing
or validating another. Desktop-only discovery uses its actual embedded engine.

Filename, comments, same-name empty file/provider, or an inherited HiRoute provider
are not ownership proof. Existing unowned files remain untouched; conflicting names
may suggest another name. Creating a profile rejects inherited same-name provider
auth residue rather than clearing it with an empty table. Added auth/header fields
inside an owned provider are conflicts, not silently cleaned. Root selector
`profile="hiroute"` conflicts just like the reserved inherited profile; other selectors
remain independent. Collaboration uses the native root, not the model profile;
misdirected collaboration configuration fails with its explicit root-context reason.

### Requirement: Codex native retention needs both metadata and account proof

Users choose HiRoute-only or retain-native-models as part of the shared settings.
HiRoute-only exposes only authorized Fixed/Plan names and requires an available
HiRoute default. With Plan aliases alone, catalog comes directly from published
plans without native cache/account login. Fixed native entries still need complete
native metadata for their exact binding.

Retention resolves actual account models through the approved same-account
subscription/CPA or saved API source, proves same original name/physical model and
compatible route, and seals connection-specific grant membership. Catalog names
alone do not grant access; unproven entries stay metadata-only. Missing source,
unproven default, or a proven model without an exact route blocks the switch before
writing, with HiRoute-only as an explicit user alternative. The subscription channel
may be access-only borrowed or independently managed; no native-auth passthrough,
new OAuth refresh owner, account substitution, or global source membership expansion
is introduced. A retained default still routes through HiRoute, not native direct mode.

Current Fixed bindings stay visible for repair/removal; automatically retained ones
are read-only while enabled. Restoring either mode returns the first-managed provider/
auth/model state, not the current managed selection. If original model field was
absent, remove the owned field without requiring native account/cache/model call.
Restoring an explicit/substitute model retains exact required directory checks.
An old restore point containing an invalid managed alias fails explicitly rather
than silently selecting another source or permanent native default.

### Requirement: Codex catalog resolution is target-specific and preserves bytes

Only retained-native mode reads the target catalog, in strict precedence: explicit
effective-scope `model_catalog_json`; only if absent, that CODEX_HOME's models_cache;
only if both absent, a proven target-bundled artifact using the same consumption
contract. An existing higher-priority unreadable/malformed/invalid source fails
without falling back to HiRoute's bundle, ReleaseFacts, another home, or a model-list
projection. HiRoute-only unused native cache drift cannot block save.

Merged output preserves top-level unknown fields, complete retained-entry JSON and
relative order, then appends qualified plans. It exposes only authorized retained
names. For target-cache native entries, absent supports_parallel_tool_calls defaults
to true; explicit booleans remain, null/wrong types fail. Explicit user catalogs get
no such missing default. Appended plans explicitly set false for current serial
capability. Metadata does not grant account/model rights.

Preview/Apply bind producer kind, exact path, complete content/context digest.
Drift fails context_drift before config write. Artifact is private and immutable;
input cache/catalog cannot be modified/deleted. Reconfiguration derives from the
original pre-managed pointer or current same-target source, never the previous
HiRoute merge. Restore reinstates the original pointer or removes the owned one.
CLI/Desktop share only equal normalized scope and CODEX_HOME, without a new binary
pin/version gate or retroactive surface history merging.

### Requirement: Codex launch and restore preserve the native user's control

Independent access provides the actual `codex --profile` command for its profile
and CODEX_HOME, properly quoted and labeled for the shell. A short form is allowed
only for default context with known terminal resolution; otherwise include original
CODEX_HOME explicitly without changing HOME/shell startup files. Safe local access
DTO may expose only the exact codex_home/target_file/commands needed for this action;
other paths remain masked and no credential is included. This exception does not
apply to diagnostics or remote telemetry. Gateway validates token/model scope,
not directory hiding or launch argument filtering. Pure HiRoute needs no Codex
OpenAI login and does not promise Desktop profile launch or migration of sessions
between providers. Native history labels/databases are not rewritten.

Before enabling model routing, the ordinary Codex form visibly explains session
discovery for the selected mode; the configured connection retains this guidance.
Default on-demand mode preserves ordinary Codex configuration and session history.
Earlier-provider sessions can be hidden from `/resume` inside the HiRoute profile;
returning to the original entry does not require disabling HiRoute. Advanced
default takeover warns that the provider filter can hide earlier sessions even
in the all-directories view, while retaining their files. Only this mode requires
turning off model routing and completing native configuration restoration before
returning to the original default entry. An incomplete or conflicting restore
does not authorize claiming that entry is restored.

Both modes direct users to their original CODEX_HOME and original profile with
`codex resume --all` to select an earlier session. This original entry uses the
original provider, bypasses HiRoute and needs that provider's credentials. Native
acceptance proves earlier-session discovery and resume through the ordinary entry
while the on-demand HiRoute profile remains configured; profile isolation alone
does not establish resume success.

Restore preserves unrelated user fields and new content in a created profile.
Originally missing profile fields resume inheriting current root rather than writing
old inherited values into the profile. No online/model-auth probe is required to
undo owned selection. Before ordinary Disable, validate target/protected restore
record and conditional write; drift preserves file and original connection with
not-completed feedback. Proven untouched staging may roll back; a restored-file then
service failure uses journal/conditional compensation, releasing writer when safe.
Unparseable UTF-8 or inaccessible target is local scan diagnostic, not total scan
failure. Persisted service-tail recovery needs its exact recorded dependency bytes;
user-changed safe defaults do not authorize reinterpretation of an accepted tail.

Login-item creation/check is not a model-save prerequisite. Users ensure the service
runs when needed. Existing journal-owned historical login items may be released on
last restore; preexisting user items are preserved. Saving config does not guarantee
future service availability or real CLI/Desktop model inference.

### Requirement: Claude ordinary entry preserves its preset and auth semantics

The ordinary `claude` command consumes configured user settings. Opus/Sonnet/Haiku
presets independently keep native mapping or map an allowed Plan; mappings may repeat.
There is no fourth slot/arbitrary plan menu or forced single default. HiRoute cannot
write native settings.model as a convenience. It conditionally owns only confirmed
Gateway/preset/auth/window fields, retaining other user/project permissions, MCP,
Skills, and unknown fields. Original helper/user-file auth env is replaced by one
managed helper while active; protected restore stores the old values, never normal
journal/WebView/logs. Unsafe process/project/enterprise route/auth overrides block
configuration instead of leaving a falsely connected Agent.

Effective explicit current model is env.ANTHROPIC_MODEL before top-level model,
and must be covered by this grant. With neither, account-dependent Default cannot
be inferred from three preset mappings: mappings may save with Default unverified,
then user explicitly selects/tests a mapped preset. Native user changes survive
restore except conflicting owned fields, which fail closed.

`hiroute agent launch` remains optional isolated diagnosis, not a configuration
activation requirement or displayed setup command. It derives from current successful
Operation/presets/grant/stable endpoint, retains original cwd/TTY/stdio/signals and
private overlay, and cannot run old auth helpers, re-discover sources, version-probe,
or independently reauthorize from current user files.

### Requirement: Client windows use the selected native client's policy

[Plan windows](../routing-plans/spec.md) are per plan. Codex keeps per-plan catalog
windows. Claude's process-wide compaction window is the selected plans' minimum,
capped at 1,000,000; below 100,000 rejects Claude configuration with actionable detail
without changing the plan or blocking Codex. Retained native models may compact
sooner using their recognized capacity. This is compaction policy, not a request-
size/token-cost guarantee.

Chosen Claude window feeds persistent user settings, managed launch, Workers, and
Gateway alias capacity hints. Conflicting process/project/managed window or disabled
compaction overrides block persistent configuration. Re-saving/restarting applies
changed plan windows; publishing alone does not reload existing native sessions.
Restore preserves prior user values, and supported old immutable operations without
window fields retain recovery. Preview data identifies target Agent, per-plan and
shared window; ordinary enable dialog need not expose a read-only capacity panel.
Fixed native effort and plan-owned effort follow [request routing](../request-routing/spec.md);
plan catalog cannot imply one client effort changes all candidates.

### Requirement: Supplementary providers retain native defaults and exact protocols

Qoder/Pi/DSH/OpenCode model configuration MUST contain explicitly selected published Plan routes,
one stable owned provider per plan and user-chosen Responses/Messages protocol.
Recommendation considers all active/fallback candidates' native protocols and
explains conversion, never overrides selection or starts paid calls. Protocol change
cannot regroup unrelated plans or rewrite old operation/provider identity. Restore
checks native default references before deleting an old selector.

Qoder does not scan/import native models/API/subscriptions/login through this surface.
Owned context provider preserves other providers/models/default/purpose/hooks/MCP/
unknown fields/comments. Native model provider uses its dedicated local model-grant
carrier/path rather than run credentials; ordinary saved native startup must consume
that config without transient route/token overrides. No-model collaboration stays
independent. Strict existing Model V2 Codex/Claude responses keep field order/shape;
collaboration-only clients use their distinct schema/context/collaboration result,
not fabricated Model/NotConfigured/verification fields. Installed Skill state and
actual native validation are different facts.

Pi owns only selected per-plan members of user models.json with current alias and
exact context/output, not native auth/default/extensions/Skills. Current user
settings.json defaultProvider/defaultModel is an independent remove dependency
checked at Preview/staging/activation/exact retry; a late default pauses the original
operation until changed, not another operation's bypass. Project defaults retain
native semantics. Configure checks only the same-package interface needed for
models or native Skill/read/bash/trusted CLI; Worker APIs are not requirements for
collaboration alone. Restore adds no incompatible-SDK prerequisite. Checks are
offline without native auth store/helpers/extensions/downloads/model calls. Changed
selected CLI invalidates process-local proof, not a generic version-label whitelist.

Every supplementary provider obeys [installed-budget protection](../routing-plans/spec.md)
for surviving current grants, including Pi/DSH/OpenCode; completed service with pending file
tail remains recoverable before conflicting publication. Scope/rotation/restoration
never chooses a native default or takes over an unowned equal-name provider.

#### Scenario: Referenced managed route retains its identity

- **WHEN** a native model reference still selects an owned supplementary provider/model while its token is rotated
- **THEN** rotation MUST retain that selector; removing it MUST report an actionable reference conflict before the file switch.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258609244

### Requirement: DSH Desktop owns an independent native model target

The official macOS DSH Desktop installation SHALL be discoverable independently of
an npm DSH CLI. Its model entry SHALL use `agent_dsh_desktop` and surface
`dsh_desktop`, retaining the registered DSH integration profile. Existing
`agent_dsh_default` Web settings and their restoration authority SHALL retain
their identity and `profiles/web/cordis.patch.yml` target.

Desktop configuration SHALL use the selected DSH_HOME's
`profiles/desktop/cordis.patch.yml`. Provider mapping, bundled routing adapter and
its native registration SHALL be installed through the ordinary protected
settings transaction. The Desktop adapter SHALL be adjacent to that patch, with
independent artifact ownership and access grants. Saving, rotating or restoring
one profile SHALL preserve the other profile, native providers and unrelated
user content. Restore SHALL remain available after the Desktop application is
removed; missing or unsupported installation SHALL block new Configure attempts.

Configure SHALL verify the installed app's bundled public CLI in an isolated
HOME, DSH_HOME and working directory, without activating user plugins or models.
The shipped Desktop profile uses the standard Web bundles; because upstream
forbids a Desktop-profile CLI dump, admission SHALL combine the bundled CLI's
standard Web composition with the actual Desktop manifest and static patches.
Custom bundle/dependency composition SHALL be rejected rather than inferred from
a default dump. Capture SHALL bind executable/archive identity, home patch,
Desktop patch and manifest, and recheck them before returning. A Web-profile
change SHALL NOT invalidate an independent Desktop proof.

Desktop model support SHALL NOT imply Worker, collaboration Skill or management
Skill support. Those operations remain unavailable until independently supported.
A composition dump or fixture SHALL NOT establish actual Desktop loading or
Gateway request provenance; those require native client evidence.

#### Scenario: Desktop-only install and independent recovery

- **GIVEN** the official Desktop app is installed without an npm DSH CLI
- **WHEN** a user saves published routes in the Desktop entry
- **THEN** the native Desktop model selector SHALL load those routes with the bundled routing adapter
- **AND** Web configuration SHALL retain its previous content and ownership
- **WHEN** HiRoute restarts and the Desktop app is removed
- **THEN** the retained Desktop operation SHALL still restore only its owned model configuration and adapter.

### Requirement: DSH configuration uses public static composition

DSH uses public CLI/ACP/config, no internal SDK/plugin/session decoder. Static scan
accepts safe block/flow YAML or JSON, but edits require a preservable block sequence.
Executable tags/includes/merge ambiguity/dynamic or shadowing composition are
rejected, not evaluated. Unrelated lines/providers/defaults remain unchanged; native
compatibility options that cannot be preserved block import. Candidates require
explicit nonempty model IDs; missing catalog models are not placeholder Observed
members. Only nonempty explicit input proves vision; missing/empty is unknown.

Resolve effective standard credential module before import. The home patch replaces
the Web module/config as a whole, not field-merging. Credential location follows
static absolute config.path, config.dshHome/.credentials.yaml, then selected DSH_HOME's
file. An explicit nonempty inherited environment Key precedes file references.
Evidence binds effective file path/bytes and selected environment secret; unselected
fallback file drift cannot invalidate it. Relative/tilde/HOME/cwd-dependent paths,
disabled/removed/unknown module composition fail without guessing a default or helper.
Passive dotenv/OAuth import is not supported. Public composition checks stay path-
capability-based, not exact release pins; cross-version execution needs separate
native evidence. Model and collaboration restore remain independent.

Settings reads SHALL resolve the current executable and native configuration without
running a diagnostic version command. Explicit discovery MAY obtain the version.
A model facts capture SHALL check the public composition once and bind that result
to its observed installation and model-configuration dependencies. It SHALL reread
those dependencies after both native snapshots and reject changes. Preview and
Apply admission SHALL perform independent captures; no composition result may be
reused across them. Apply SHALL reproduce current facts, confirm the accepted intent
and seal effects once inside the existing writer boundary, after revision and
principal checks. It SHALL NOT build and clone another complete plan before the
writer. Idempotent replay SHALL return the admitted operation without repeating
native preparation. Current dependency, accepted digest, login-item restoration,
external fingerprint and atomic journal checks remain mandatory; unchanged route
selections do not authorize skipping native drift checks.
The typed settings planner SHALL confirm original caller intent against those facts
and bind the normalized specification to the request digest, Operation identity and
journal. Existing callers MAY submit the original intent with the returned preview
digests, including an omitted known Hermes target or preserved Codex model entries.
Normalization SHALL NOT bypass intent, dependency, revision or Restore authority
checks; genuinely stale settings continue to report `CHANGE_PREVIEW_STALE`.

#### Scenario: DSH composition changes during or after preview

- **GIVEN** a supported DSH installation and selected published model routes
- **WHEN** the public composition probe or either native snapshot observes changing dependencies
- **THEN** the capture is rejected instead of publishing a mixed configuration
- **AND** Apply checks the then-current composition inside locked admission,
  even if the earlier Preview succeeded.

### Requirement: Collaboration proofs distinguish preinstall from installed Skill

Codex/Claude/OpenCode initial collaboration install MUST use user-triggered private-context proof
of Skill loading and controlled read-only trusted HiRoute CLI, without daily config,
upstream calls, or borrowing another client's proof. Current-client/installation/
short-lived evidence binds Preview/Apply; missing/expired proof retains edits and
recheck. Plain model settings do not require this collaboration challenge.

Qoder proof explicitly borrows the selected normally logged-in context, isolates
workspace/provider/tool/challenge, and never copies auth, auto-logs in, collects PAT,
falls back to another HOME, or forwards model requests. Native client's own refresh
does not grant HiRoute auth/config deletion rights. It preserves settings bytes/mode
without promising all borrowed HOME is immutable.

Preinstall private project/plugin Skill proof establishes a basic capability only.
Postinstall verification must load/invoke the actual user
skills/hiroute-collaboration/SKILL.md with user source included, correlated native tool
call/result and trusted CLI outcome. Same-named project/plugin, prompt echo, or model
self-report cannot replace it. Method/context/CLI/contract/target/content binds the
consumed dependency evidence; changed/expired/shadowed/invalid resources require fresh
proof. Unknown route/hook/MCP/tool constraints fail rather than infer isolation from
empty settings/mode names. Installed, native-verified, and real delegated completion
remain distinct. Closed diagnostics distinguish login missing, installed-Skill absence/
drift/invalidity, context/dependency/check failure without raw output/paths/credentials.
An auth challenge followed by collaboration proof cannot erase still-valid auth
capability, nor grant unrequested Skill access to model settings.

OpenCode checks use private native configuration/history and controlled upstream
replies. The installed-Skill check loads the actual owned user resource path through
the native resource loader and binds its exact path/body and generation to the
nonce-correlated trusted public CLI result. It does not parse daily model settings,
require a published Plan/model grant, or inject a copied substitute installed Skill.
The real main-Agent journey separately proves native context use and delegated
completion; prerequisite evidence alone cannot make that claim.

#### Scenario: Collaboration without model routing

- **WHEN** a user enables collaboration while OpenCode continues to use its own model provider and no HiRoute Plan exists
- **THEN** the owned Skill can be installed independently, no model grant/file effect is created, and the runtime directory truthfully contains no delegate target.

#### Scenario: Real delegated journey

- **WHEN** OpenCode loads the actual installed user collaboration Skill and the user requests work through an available HiRoute Plan
- **THEN** OpenCode invokes the trusted public CLI, retains the returned task identity, and obtains the actual Worker outcome rather than treating Skill installation as task completion.

#### Scenario: Shadowed or edited resource

- **WHEN** a same-named project Skill shadows the installed user Skill or the managed user Skill is edited
- **THEN** verification cannot accept the substitute as the owned installed resource and restoration cannot overwrite user edits without resolving the ownership conflict.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258610209

### Requirement: Skill and reference maintenance never becomes Worker authority

The shared Skill queries published purposes and public task CLI, explains local trust,
approve-all default/optional policies, preserves original submission identity, and
hands off results. Trigger description/body update together. explicit is default;
delegate_by_default guides execution tasks but cannot technically intercept every
request or recursively delegate coordination/Workers. Installing model access does
not enable default delegation. Configuration/restoration of owned Skill references
is transactional, shared-file-reference-aware, and preserves equal-content unowned
or user-edited files. Old sealed artifacts may be cleaned only with current-key
proof of ownership; obsolete runtime/task records remain isolated unchanged.

Released Worker calls reject protected grants/capability FDs before business work.
Dependency selection uses local trust with exact Harness revision CAS, deterministic
intent/digest, key, and Operation, never Apply-capability issuance. Plan reference
queries return independent default-model/allowed-model relations with stable subject/
revision/digest, not removed collaboration allowlists. Unfinished/corrupt records
are not zero references. Plan admission/deletion rechecks them under its writer/gate.

### Requirement: Live model checks consume one exact explicit consent

Live CheckAgentConnection binds context, native surface, current tested publication,
model names/client, call count, and possible cost. Validate target/surface/configuration/
current grant membership before starting or consuming consent. Valid consent is
atomically consumed before potentially paid execution; expired/replayed/cross-subject/
cross-request use fails. Result belongs to that tested surface/revision only, not
all configured Agents or future service state. Verification uses isolated configuration
rather than altering a live user's environment, and cannot substitute for ordinary
native saved-route or real main-Agent delegation acceptance.

### Requirement: Routing adapters share native configuration ownership

Applying managed Pi, DSH or ordinary OpenCode model settings SHALL install the
bundled native routing adapter through the existing protected artifact transaction.
OpenCode MAY explicitly persist `routing_metadata: false` in its additional-model
selection to omit only the HiRoute metadata plugin. Omission or `true` retains the
normal installation behavior. The selection SHALL preserve the authorized models,
classification, actual execution capture and cross-turn evaluation; missing native
purpose and continuity evidence remains uncertain. It SHALL NOT disable unrelated
plugins or promise to bypass their native dependency initialization.
An existing managed adapter SHALL require complete model Restore before this
selection can be applied. Preview SHALL identify that recovery step before writes;
a surviving borrowed or foreign adapter SHALL instead require explicit user review.
Reapplying the normal selection SHALL reinstall the adapter through the same
transaction, including after a daemon restart or a plugin-free configuration.
Reconfiguration, activation, drift detection, interrupted-operation retry and
restore SHALL verify the adapter's exact owned bytes. A conflicting foreign file
SHALL be preserved. Identical pre-existing files MAY be borrowed and SHALL survive
restore. Published operations without adapter effects retain their original
recovery contract; explicit reconfiguration can add the current adapter.
DSH SHALL preserve the original ownership of its Cordis adapter registration across
unrelated user edits, model changes and token rotation. Restore SHALL remove an owned
registration together with its owned adapter file, while retaining a genuinely
pre-existing registration and borrowed file. A registration created by an earlier
HiRoute configuration MUST NOT become user-owned merely by reconfiguration.

Pi delegates managed physical Responses and Messages providers through the public
provider registry and registration seam. It disables managed-provider cache warming,
whose purpose is otherwise indistinguishable. DSH attaches one public Cordis middleware
insert and preserves unrelated patch rows. OpenCode uses its native per-call hook
with a bounded optional session read. OpenCode Workers retain pure mode and split
main and auxiliary provider declarations within the same authorized frozen Plan;
they do not enable arbitrary external plugins to obtain routing evidence.

Hermes optional compression evidence SHALL support both the bounded current SDK
chain API and the compatible 0.20 public row/unique-live-child APIs. The legacy
unbounded lineage enumeration SHALL NOT run. Missing, locked, ambiguous, corrupt,
replaced or unsupported native state SHALL suppress optional continuity evidence
without preventing the model request or changing its authorization. Ordinary
children and forks SHALL NOT be reported as compression continuations.

Hermes public whole-turn hooks cannot distinguish real input from native async
completion notifications. The producer SHALL NOT claim fresh human provenance.
Gateway treats these as observed requests with explicit source uncertainty,
including older producers' false freshness flag, and retains classification,
actual execution history and cross-turn evaluation. Missing optional hooks SHALL
NOT block compatible authorization or model execution; absent native identity
retains request scope. Compression lineage does not authenticate input origin.
Accuracy of callback/task association remains a separate limitation, not a reason
to disable smart routing. Known auxiliary clients remain outside the main history.
Pi compaction and requested nonempty tree summaries SHALL bind purpose to the
actual native operation's AbortSignal. Serial calls and retries on that signal
remain auxiliary without a session phase, timeout or terminal callback. A failed,
cancelled, extension-supplied or omitted summary SHALL NOT mark subsequent ordinary
requests. Known summary purpose survives unavailable optional cursor evidence.
Ordinary calls SHALL attach the extension session's cursor only when the provider
call's sessionId matches that live native session; a summary requires its own
matched operation signal. An unmatched call keeps its original provider options.
Missing signal remains unknown. Session, purpose and branch facts SHALL be frozen
before asynchronous provider execution. Native projection tri-state supplies
observed event/compression cursor evidence, never final wire input positions or
human provenance. Branch evidence includes the leaf and ordered native path; a
shared user entry alone cannot distinguish branches.
Pi SHALL write request metadata through options.headers at the provider seam and
retain the original stream implementation exactly once, including auth, payload,
retry and cancellation semantics. Reload, session/model replacement and provider
refresh SHALL NOT stack wrappers or revive a superseded delegate. Missing public
registration-identity capability or optional metadata failure SHALL preserve native
provider availability. The exact-metadata scope is one managed CLI runtime per
process; arbitrary SDK hosts that share and independently unload a mutable
provider registry, third-party virtual routes and signal-replacing summaries
require separate evidence. A transient optional failure SHALL NOT latch routing
off, and summary content SHALL NOT reconstruct prior accepted task execution.
DSH managed model streams SHALL retain minimal purpose/session evidence across
native LLM service isolation, including Web session main and title calls. Listening
across service scopes SHALL NOT attach metadata to unmanaged providers or remove
the existing Agent/grant transport gate.
DSH relay/settlement envelopes after the latest actual input SHALL
NOT acquire that input's proof merely by arriving in its active native turn.
OpenCode binds a bounded native message lookup to the current message ID, preserving
native page order rather than deriving chronology from opaque IDs. A positively typed,
synthetic-only compaction followup emits the `existing_turn` input guard with that
current message ID; a successful summary's exact parent and checkpoint are advisory
compression cursors, never authority to select an old or newer queued task. Without
an independently captured boundary, the continuation temporarily falls back and
captures its actual output with unavailable input. Later ordinary A/B requests SHALL
recover classification, capture and cross-turn evaluation in the same session.
Lookup failure retains minimal purpose/session evidence and observed-batch semantics;
it SHALL NOT erase a known auxiliary purpose or invent a synthetic-input guard.
Unbound synthetic child results remain observed with uncertainty. These adapter facts
do not authenticate arbitrary third-party transformations of the final provider
payload. Such unsupported composition MUST NOT be claimed as verified recovery.

Retained Hermes `X-Hermes-Observed-Input` v1 headers are non-authoritative
compatibility hints. A clean-text digest or reverse ordinal SHALL NOT select a
current event, remove later Users or reconstruct original task input. Current
Hermes adapters SHALL use observed-batch semantics and retain actual execution
and cross-turn evaluation with explicit source and association uncertainty.

### Requirement: Optional native routing evidence separates purpose, input and history

The `X-HiRoute-Routing-Context` v2 object SHALL carry `version:2`, the matching
`agent` family and `purpose`. Known auxiliary purposes (`compaction`, `summary`,
`title`, `session-title`, `prewarm`) are resolved before optional enrichment.
Ordinary purposes are `main` and `unknown`. One header of at most 4096 bytes is
accepted. Duplicate fields/headers and contradictory identity channels reject the
affected evidence; malformed JSON is never mined for a trusted purpose substring.
Missing/invalid enrichment cannot disable compatible model execution.

Optional `session_id`, `turn_id`, `request_id` and `checkpoint` use the existing
printable native-ID limit of 128 UTF-8 bytes. `session_id` must match an independently
admitted native identity. `request_id` is advisory and never deduplicates Gateway
executions. `branch:{leaf_id,path_digest}` supplies a bounded native cursor;
`continuation:{kind,from_turn_id?,from_checkpoint?,to_checkpoint?}` supplies an
optional `same_turn` or `compression` claim. Neither grants history authority.
Only the existing authenticated consumer/workspace/grant/route/protocol scope may
own captures. Unknown versions retain independently usable native identity and
ordinary observed routing.

`input` may be `observed_batch` with optional `event_id`, or `native_messages`
with `event_id`, `coverage:"window"`, `messages:[{index,sha256}]` and `scope_digest`.
The latter admits at most 32 strictly ordered unique positions among at most 128
final User envelopes and 2 MiB supported text. Each message digest hashes the UTF-8
compact JSON array of its text parts; scope_digest hashes the compact ordered array
of lowercase per-User digest strings. Tool-only envelopes do not take User ordinals.
Unsupported media/tool/provider-state parts invalidate the proof instead of being
dropped. Valid proof selects only current input and never reclassifies unlisted
Users as System or proves human authorship. Failure falls back to the full latest
wire User envelope. Legacy v1 inputs remain readable through one ingress normalization;
they cannot acquire stronger current-version semantics by adding unknown fields.

`input:{kind:"existing_turn",event_id}` is a negative guard for positively identified
synthetic continuation input on a `main` execution. Its bounded `event_id` SHALL
equal the current `turn_id` and its session/family SHALL match independent native
identity. It SHALL NOT replace that ID with a predecessor, prove human origin, grant
ancestry or detach the actual output as auxiliary. Without independently captured
input/decision continuity, this request temporarily falls back, retains actual output
and marks its input `native_continuation_input` unavailable. Later ordinary input
is independently eligible for classification and assessment. Compression cursors alone
SHALL NOT apply this guard. Invalid guards retain observed routing. Matching current
installed resources and Gateway are required for this additive v2 capability; an
older reader may discard unknown enrichment and does not promise synthetic-input
protection. The registered legacy v1 reader retains its existing semantics.

Validated current-input windows SHALL preserve their canonical first/last positions
through capture admission. Multiple input envelopes in that window and trailing
native context SHALL NOT create a false uncaptured execution between completed A
and new B. A window cannot span model/tool execution. Earlier uncaptured turns
still fence the preceding assessment, and uncaptured output after the selected
window still makes its captured suffix partial. Original roles, bytes, response
boundaries and source uncertainty remain unchanged.

#### Scenario: New input follows compaction and carries trailing context

- **WHEN** A completes after shortened history and B has a valid input window followed by a native User-role context envelope
- **THEN** B classifies its selected input and can assess A's actual completed stage, without capturing the summary or trailing context as A's execution or treating B as a missing earlier turn.

#### Scenario: Synthetic continuation has a new native message ID

- **WHEN** a main compaction continuation names a former turn but has no independently captured execution boundary, including after Gateway restart
- **THEN** its synthetic input cannot be classified or captured as new task intent, its actual output remains captured with unavailable input, and subsequent A/B requests resume classification with B assessing A's actual execution without importing the summary.

### Requirement: Reversible Hermes profile connection and activation guidance

Desktop SHALL show Hermes alongside other supported Agents before a profile is
selected. Configuration-directory selection belongs in Hermes details. An
unscoped entry SHALL NOT imply installation, connection or settings authority;
settings actions require the existing backend-validated effective profile.
Selecting or reading a directory SHALL NOT configure routes or install resources.
Without an explicit target, Desktop SHALL discover its caller's existing effective
default (`HERMES_HOME`, otherwise `HOME/.hermes`) through the shared backend and
show the resolved scope before Apply. An unavailable explicit root SHALL NOT fall
back to another profile. A failed default-profile read SHALL preserve the other
Agent entries and allow selection of another directory.

HiRoute SHALL configure the selected effective Hermes profile's default main model to the chosen published Plan alias and automatically install and enable the required HiRoute integration resources when the connection is applied from CLI or Desktop. Native connection fields and resources SHALL follow existing reversible ownership and transaction contracts, preserving unrelated native configuration and user data. Connection status/guidance SHALL identify the configured scope, report statically known selection overrides and applicable reload/model-selection actions, and keep unobserved live selection unknown. Installed/enabled resources SHALL NOT be represented as proof that an already-running Hermes process loaded them. HiRoute SHALL publish its Hermes integration requirements and reject a new connection before native writes only when required plugin/configuration or transport/header interfaces are incompatible. Older version labels and optional reasoning-replay limitations SHALL produce upgrade guidance rather than block an otherwise compatible connection. Independent Skill preparation, passive scan and existing owned recovery/restore SHALL use their own required capabilities rather than a blanket new-connection version gate.

Each journaled native plugin module, manifest or routing adapter SHALL contain
nonempty UTF-8 content of at most 32 KiB, measured in bytes, without NUL. Actual
bundled Hermes, Pi, DSH and OpenCode resources SHALL fit through the current intent
constructor and decoder. This per-file bound preserves admitted content of
16 KiB or less and the existing ownership, authorization and digest bindings;
independent Skill resources retain their 16 KiB limit.

#### Scenario: Open Hermes before selecting a profile

- **WHEN** Desktop has no discovered Hermes profile
- **THEN** Hermes remains in the common Agent list, with directory selection in its details and settings disabled until a valid selected profile is returned; cancel or read failure preserves the current connection and native files.

#### Scenario: Discover an existing local Hermes profile

- **WHEN** the Desktop caller has an existing default Hermes configuration and has not selected another target
- **THEN** the existing backend reads that effective root, its scope appears in Hermes details without a manual directory step, and discovery neither changes native files nor restarts Hermes.

#### Scenario: Connect and prepare resources in one workflow

- **WHEN** the owner applies a supported Hermes profile connection to a published Plan
- **THEN** the owned default/provider and required resources are prepared through the existing Operation without a separate manual plugin installation/enable step; successful saved state reports its scope and any remaining native loading action.

#### Scenario: A conversation may retain another selection

- **WHEN** a statically known channel/session selection overrides the managed default, or live selection cannot be established
- **THEN** HiRoute reports the known configuration or unknown live state and applicable native switching/reload guidance without rewriting the override or resetting a conversation.

#### Scenario: Restore the managed connection

- **WHEN** the owner restores the Hermes connection after unrelated native edits
- **THEN** existing conditional ownership restores the managed fields/resources and preserves unrelated changes; an owned-field conflict retains the existing conflict/recovery outcome.

#### Scenario: An installation is outside the supported connection boundary

- **WHEN** a selected installation lacks a required plugin/configuration or transport/header interface
- **THEN** HiRoute reports the missing support/capability and upgrade action before native connection/resource writes, without automatically upgrading Hermes or falsely completing the connection; readable scans and safe owned restore are not rejected solely by that gate.

#### Scenario: Older compatible Hermes remains connectable

- **WHEN** required integration interfaces are available but Hermes lacks optional reasoning replay
- **THEN** connection Preview/Apply remains available, and guidance recommends an upgraded Hermes version without claiming that reasoning replay is complete.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165296337

### Requirement: Hermes can initiate task delegation through existing Workers

HiRoute SHALL let the owner independently configure, inspect and restore task delegation for the selected Hermes profile through Desktop and the public CLI. Hermes SHALL use the existing collaboration Skill, explicit-only (default) or delegate-by-default policy, and public Worker commands to select a published delegation-enabled Plan, submit work to its supported executor such as Qoder, observe completion and retrieve the actual result. Preparation requires neither a Plan nor a configured Worker; those are execution prerequisites. This capability SHALL work with Hermes native model configuration and without the HiRoute model-session plugin. Hermes SHALL NOT become a selectable Worker executor. Missing prerequisites, pending work and failed execution SHALL remain explicit rather than claimed success or implicit alternate execution.

#### Scenario: Hermes delegates to Qoder

- **WHEN** Hermes uses its native model, collaboration is prepared and an authorized task matches a published delegation-enabled Qoder Plan
- **THEN** Hermes can submit through HiRoute and retrieve the actual Qoder result; the Worker uses the selected Plan and Hermes model configuration remains unchanged.

#### Scenario: Trigger policy and answer-only intent

- **WHEN** Hermes has explicit-only delegation configured, or default delegation is configured but the owner requests an answer only
- **THEN** the instructions preserve that policy; enabling management does not override it.

#### Scenario: Unavailable executor or uncertain submission

- **WHEN** no eligible Plan or executor is available, or an accepted run is pending or failed
- **THEN** the actual condition is reported; uncertain submission is recovered using its original identity without duplicate submission or fabricated completion.

#### Scenario: Initiator is not executor

- **WHEN** Hermes task delegation is enabled
- **THEN** existing Worker executor choices remain unchanged and do not include Hermes.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6186244084

### Requirement: Selected Agent refresh reads only its current context

Refreshing a known Agent after a save SHALL verify that exact context without
scanning neighboring Agents or the caller's default Hermes directory. A model-only
save MAY use the fresh model projection without repeating version diagnostics or
unmodified Skill reads. The current native file, grant and installed publication
join remains required; a successful Apply receipt alone is not readback evidence.
Unrefreshed Skill state SHALL remain explicit rather than appearing configured or
disabled. Opening that Skill's normal editor SHALL read its current selection before
seeding the draft. Codex root/profile/access metadata and explicit Hermes native
targets retain their scoped full read where required to validate the selected target.
`ScanAgents` MAY accept an optional `context_id` with its exact native target;
omitting it retains the existing full-scan behavior. An invalid context, mismatched
native target or changed selected Codex root/profile SHALL fail without rebinding
the caller to another context. Explicit full rescan remains the recovery path.
The selected read SHALL retain before/after service revision, publication, recovery,
client authority and window-lifetime validation; discovery alone never proves
that settings or a saved operation are effective.

#### Scenario: A save refresh has an unrelated installed Agent

- **WHEN** the client refreshes the saved Agent's exact current context
- **THEN** the affected settings are freshly verified, without launching or reading
  the unrelated Agent's configuration
- **AND** an external change to the selected context is reported for explicit recovery.

### Requirement: Supported Agent discovery preserves scope and remains actionable

Desktop SHALL retain the supported local Agent catalog when an installation or effective configuration has not been discovered. Discovery SHALL distinguish an undiscovered configuration, an unavailable selected configuration and a discovered usable context without treating absence as proof of uninstallation. Users SHALL be able to rescan through the existing scanner. Where an integration supports explicit configuration directories, selection SHALL live in that Agent's details and be remembered as a non-secret discovery preference; every read and settings action SHALL revalidate it through the existing backend. An unavailable explicit directory SHALL remain selected and SHALL NOT silently fall back to another profile. One failed profile read SHALL preserve unrelated Agent entries. Discovery, preference persistence and cancellation SHALL NOT install an Agent or Skill, change native model configuration, invoke a model or restart a native process. Settings require the backend-validated effective context.

#### Scenario: Undiscovered configuration remains visible

- **WHEN** a supported Agent has no discovered effective configuration
- **THEN** its catalog entry remains visible with an honest discovery state and a rescan action; settings cannot be applied without a validated context.

#### Scenario: Remember and revalidate a custom directory

- **WHEN** the owner selects a valid custom configuration directory and later reopens Desktop
- **THEN** Desktop rereads the remembered directory through the backend before presenting settings authority; it does not require the owner to reselect it.

#### Scenario: Unavailable explicit scope remains selected

- **WHEN** a remembered or explicitly selected configuration becomes unavailable or unreadable
- **THEN** Desktop retains the selected scope and unrelated Agent entries, explains the known condition and permits rescan or directory replacement without selecting another profile or overwriting files.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6193859603

### Requirement: Connection guidance distinguishes saved configuration from effective native requests

HiRoute SHALL provide target/Plan-oriented completion and troubleshooting through its existing status, check and observation capabilities. Results SHALL distinguish saved preparation, known native prerequisites/activation, current selection when observable, and actual recorded request outcomes. Native loading or selection that cannot be observed SHALL remain unknown with an actionable supported next step. Evidence SHALL bind the selected connection/Plan and relevant configuration/publication rather than an unrelated or older request; observed success SHALL describe only the observed surface/session and not every running conversation. A successful direct provider check SHALL NOT establish that a native Agent is using the route. Native activation guidance SHALL use actual adapter capabilities and supported native controls, preserve native lifecycle protections, and keep preparation separate from restart or model-call effects. Verification without model-call consent SHALL remain pending. Restore SHALL retain existing ownership and recovery guarantees and report any remaining activation or selection step.

#### Scenario: Saved but unobserved

- **WHEN** a native configuration and plugin resource are saved while the running Agent has not been observed using them
- **THEN** the result reports preparation separately from runtime/effective-request state and gives a target-specific follow-up instead of a verified success.

#### Scenario: Verify the intended route

- **WHEN** a request from the selected connection after the relevant change completes
- **THEN** the user can locate its Plan, actual model, outcome and recorded usage through target/Plan-oriented output; stale or other-target evidence cannot satisfy the result.

#### Scenario: Native control available or unavailable

- **WHEN** the selected Agent exposes a supported reload/new-process/model-selection action, or its effect cannot be established
- **THEN** the result offers the supported action with its actor/effect and observation limits; it does not bypass protection, promise universal reload or execute it merely because status was queried.

#### Scenario: No live-call consent

- **WHEN** configuration is saved but the user has not authorized a model call and no matching request is recorded
- **THEN** the result remains configured with verification pending, without a paid call, fabricated savings or false task completion.

#### Scenario: Session override or ownership conflict

- **WHEN** the current native session is known to select a different model, or restoration encounters a foreign edit
- **THEN** the result explains the observed mismatch/conflict and the applicable native selection or owned recovery action without overwriting unrelated changes.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6206260018

### Requirement: Use compact scoped Hermes provider identities

HiRoute SHALL configure Hermes with a compact recognizable managed-provider identity rather than displaying complete internal context and Plan digests. Native identities SHALL remain deterministic for the selected profile/Plan, distinct across Plan choices within that profile, and bound to the existing context, grant, plugin and ownership rules. Existing managed configurations and accepted operations SHALL retain readable recovery and exact replay; an explicit owned reconfiguration may adopt the compact representation. No foreign provider or remaining unowned native reference may be overwritten or silently redirected. Existing user data and current running-session activation requirements remain authoritative.

#### Scenario: Present a compact provider

- **WHEN** a Hermes main-model connection is newly configured
- **THEN** the native provider name identifies HiRoute with only a bounded distinguishing suffix; complete internal context hashes are not required in the user-facing name.

#### Scenario: Update or restore a prior managed connection

- **WHEN** a retained older Hermes connection is inspected, recovered, explicitly reconfigured or restored
- **THEN** old native names and sealed accepted effects remain understood, current owned reconfiguration can produce the compact name, and the original native configuration remains recoverable.

#### Scenario: Preserve foreign native consumers

- **WHEN** a proposed compact provider collides with foreign native content or removing a previous provider would break another native reference
- **THEN** the existing ownership conflict prevents the write; neither short naming nor a shared endpoint establishes ownership.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6219938656

### Requirement: Previous routing is an ordinary scoped edit, separate from Restore

Return to previous routing MUST follow the succeeded configuration journal chain
within the current Agent context, including Codex's active root/profile slot. For
each selected preset it skips unchanged resolved mappings and finds the mapping
before that preset's latest change. Only selected presets are overlaid on current
settings; unrelated presets retain their current mappings. This produces a draft
for normal Preview/Apply, with manual-edit checks and no deletion of Plans/evidence.

Traversal is bounded to 128 configurations and detects cycles. Missing history,
context mismatch, a non-succeeded entry or the traversal bound yields an explicit
unavailable reason. None is evidence of a previous native configuration. Restore
to native remains a separate labelled action with its existing ownership rules.

#### Scenario: Sonnet changes after the Opus trial starts

- **WHEN** Opus is mapped to a trial and a later configuration changes only Sonnet
- **THEN** returning Opus skips the unchanged Opus entry and restores its earlier
  mapping while preserving the later Sonnet choice.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235502571

### Requirement: OpenCode discovery preserves exact native context and scoped capabilities

HiRoute MUST recognize OpenCode as an explicit Agent identity and selectable native ACP Harness through existing Desktop and public CLI surfaces on macOS, Linux and native Windows. Passive discovery MUST use bounded installation/configuration metadata without launching OpenCode, executing shims, authenticating, downloading, changing native files or making model requests. It MUST distinguish executable discovery, model-configuration capability, collaboration capability and Worker execution/Continue capability. It MUST resolve the selected context using native HOME/XDG and explicit OpenCode configuration-root semantics without guessing another account from executable location. An explicit selected installation remains selected when missing or unusable; no automatic candidate substitution is permitted. Worker selection uses the native OpenCode CLI, not a fictitious separate ACP adapter. Platform-specific launch dependencies MUST be explicit where the actual installation form requires them. Normal configuration actions MUST reuse the shared enable/edit/restore and Operation flow rather than introduce a separate OpenCode setup or verification workflow.

#### Scenario: Partial discovery

- **WHEN** one OpenCode configuration or installation candidate is unreadable while other candidates remain valid
- **THEN** HiRoute reports the affected item as unavailable or unknown and preserves the other findings without running the client.

#### Scenario: Missing selected installation

- **WHEN** the explicitly selected OpenCode installation disappears or becomes unusable
- **THEN** subsequent affected actions report the selected dependency failure and retain its identity instead of launching another PATH candidate.

#### Scenario: Separate capabilities

- **WHEN** OpenCode model configuration is supported but native ACP or Continue capability is unavailable
- **THEN** model configuration remains usable and Worker/Continue availability is reported independently, without claiming model success from discovery.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258607139

### Requirement: OpenCode supplementary model routes are native and reversibly owned

HiRoute MUST configure explicitly selected published Plan aliases as supplementary OpenCode providers, one owned provider per Plan, with a selected Responses or Messages protocol. The ordinary OpenCode client MUST be able to select and use these aliases from saved native settings without transient route or credential injection. HiRoute MUST preserve native defaults, other providers, unrelated fields, comments and native extensions. Native provider-prefixed selectors MUST map to the original Plan alias without changing routing authority. Ordinary model grants MUST use the existing protected connection-grant channel, separate from upstream keys and Worker credentials. Configure, adjustment, token rotation and restore MUST use the shared ownership/dependency/recovery transaction, preserving original protected recovery material and rejecting unowned collisions or ambiguous JSON/JSONC composition. Effective configuration precedence and relevant native model/default references MUST be checked for the requested operation; conflicting higher-priority settings cannot produce a false connected/active claim. Restore MUST remove only owned effects and retain unrelated user edits; references to a removed managed model require actionable conflict handling. Context/output declarations MUST follow the selected Plan and OpenCode native compaction policy and participate in existing installed-budget protection. Saving configuration MUST NOT change the native default or perform paid inference.

#### Scenario: Ordinary saved model use

- **WHEN** the user saves selected Plan routes and then starts ordinary OpenCode and selects one of their native model entries
- **THEN** requests use that exact Plan alias and selected protocol through the saved local grant, with native defaults and unrelated settings preserved.

#### Scenario: Rotation and independent restore

- **WHEN** the user rotates the model grant or restores OpenCode model settings while collaboration remains enabled
- **THEN** rotation revokes the old grant for new requests and updates native consumption consistently; model restore preserves the collaboration Skill and unrelated native configuration.

#### Scenario: Late configuration conflict

- **WHEN** a relevant effective override, owned member or native default dependency changes after Preview
- **THEN** Apply or the affected recovery path rejects stale authority without overwriting user changes, falsely claiming activation or bypassing the original pending Operation.

#### Scenario: Budget change

- **WHEN** publication would invalidate the native context/output declaration of a surviving installed OpenCode Plan route
- **THEN** the shared publication guard reports the affected installed consumer rather than leaving a silently invalid native declaration.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/159#issuecomment-6258609244

### Requirement: Trial capability preview follows the selected Agent

Trial review SHALL preview the selected Agent's actual current candidate combination. DSH preview is explicitly opted in, reuses the same Responses-ingress and token-budget facts as native mapping, and reports unavailable/invalid inputs honestly. Converted upstream protocols are not mistaken for missing Responses ingress. A published preview uses the exact referenced revision, not a newer same-name Plan. Old callers without opt-in keep their serialized contract. Preview does not authorize mapping/publication or replace compatibility/grant checks. Unsupported Agents receive a neutral no-preview explanation, not another client's warnings. Requests remain bounded and stale-target results cannot replace the current preview.

#### Scenario: DSH plan supports converted Responses

- **WHEN** all actual candidates expose Responses ingress and the shared budget is available
- **THEN** the DSH preview context/output equals native mapping for that frozen Plan; no Codex/Claude warnings are shown.

#### Scenario: Invalid, unsupported or stale preview

- **WHEN** an editor is invalid, a candidate lacks ingress/budget, or the target changes during a read
- **THEN** the precise unavailable state is shown and no stale success is used; mapping checks remain authoritative.

#### Scenario: Legacy and narrow readers

- **WHEN** preview_agent is absent, or candidates are requested without an editor/published Plan
- **THEN** existing fields/shape remain unchanged and no unsupported preview is inferred or additional client compilation performed.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/228#issuecomment-6440851627

## Implementation references

- [Shared settings transaction](../../../crates/application/src/agent_connection/settings/transaction.rs)
  and [native supplemental-provider guide](../../../crates/integrations/src/agents/additional_native/README.md).
- [Codex slot/restore scenarios](../../../crates/daemon/src/control/runtime/settings_profile_tests.rs)
  and [legacy-tail scenarios](../../../crates/daemon/src/control/runtime/settings_profile_legacy_tests.rs).
- [Saved native-model journey](../../../crates/daemon/tests/support/additional_model_product.py)
  and [real collaboration journey](../../../crates/daemon/tests/support/collaboration_product.py).
