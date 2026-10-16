# Shared clients, CLI input, and installation

## Purpose

Desktop and CLI expose the same product operations through Rust Client Core. The
client owns interaction and transport, while Application/daemon owns durable truth.

## Requirements

### Requirement: Client Core is the common product access path

CLI and Tauri's native host MUST use the independent Rust Client Core to access
real hirouted/Application. Desktop cannot substitute shelling out to CLI; WebView
cannot reimplement control protocol or business policy. Application owns business
defaults/validation, canonical intent, revisions/digests, idempotency, transactions,
publication, and recovery. Client/bridge/CLI/test drivers cannot directly write
business stores, Secrets, native Agent settings, or publication, or hold a second
effective plan/Operation authority. Non-authoritative query caches, preferences,
and unsubmitted non-secret input are allowed; recovery rechecks the server.
Released help/manifests/output and helper-only boundaries remain compatible, and
input plumbing cannot promote staged commands to Released.

### Requirement: Transport and machine results preserve exact semantics

Client Core MUST provide protocol negotiation, request/result association, bounded
transport, and local failure classification, retaining stable MachineEnvelope
errors, warnings, next_actions, and Operation references. Unknown major/incompatible
capabilities fail before writes. Disconnect/unavailable/protocol failure/business
rejection cannot be rendered as success. Localized descriptions do not replace
machine codes. Unsupported local transport cannot fall back to unauthenticated TCP.

Ordinary Local Control newline-JSON requests and responses share a 10 MiB limit,
including trailing newline, on both ends. A complete verified metadata catalog
cannot be made to fit by dropping provenance/fields. Protected capability channels
retain their separate smaller bounds.

Both normal and protected-grant listeners share process-local limits: 128 accepted
connections, 64 long waits, and 16 not-yet-classified requests. Connection slots are
acquired after accept before handler creation and last until full response/exit;
unclassified slots last through parsing/classification; long-wait slots last from
classification through response/exit. WorkerWait and legacy WaitDelegation share
the same wait class. All error, timeout, disconnect, and spawn-failure paths release
held slots. Classified ordinary calls do not consume wait slots. A parsed legitimate
wait beyond capacity returns `CONTROL_WAIT_CAPACITY_EXCEEDED`/unavailable without
reading/changing/cancelling the run or faking timeout; earlier capacity exhaustion
may close transport. These limits do not alter Worker execution concurrency.

### Requirement: Local management trusts OS access with typed native boundaries

Local Control accepts processes that can access its socket, without extra peer-UID,
owner/mode checks or client-source-specific Apply capability. CLI and Desktop share
the same user/idempotency/recovery scope. Application still verifies normalized
intent, current versions, and exact admission; Preview/cancelled confirmation does
not write. Model grants and remote collaboration authority remain separate.

WebView may invoke only purpose-specific Tauri ACL commands. It cannot self-assert
principal, obtain arbitrary Local Control/shell/file/SQL access, or give remote
content local management authority. Domain pages expose real capabilities and
explicit unsupported/unavailable state, never fixture-backed success. The shell
can open offline and present implemented recovery without requiring internal
component knowledge; provider communication stays outside WebView.

### Requirement: Clients observe work without automatically repeating it

Accepted is not succeeded. Measured progress alone supports percentages; otherwise
show an indeterminate phase. Disconnect/timeout retains the original request/
Operation/key and reports uncertainty. Reconnect queries actual state; it cannot
use cached UI progress or change key to resubmit. Stop-wait, close-window, and server
cancel are separate. Unsupported cancel cannot become cancelled success.

Desktop retains pending identifiers/non-secret intent digest only within the
current interaction and does not read/write `pending-intent.json` or another client
recovery log. Reopening reads current server state; its journal recovers accepted
work. Another explicit interaction is independent of previous observation failure.
Same-interaction uncertain submission uses the original scope/key and cannot apply
newer edited intent under an old digest. Only real server recovery/writer state may
block further mutation.

### Requirement: Worker actions share one exact task/run interface

Exec/list/status/wait/read/result/cancel/Continue use the same backend and instance-
scoped submission identity. Continue is explicit and freezes the original task's
plan/session/cwd under current eligibility. Wait timeout is not execution deadline.
Client Core WorkerList sends only optional complete-title exact filter, exclusive
cursor, and bounded limit, with no protected grant. It decodes the same task/run
facts as status/detail. Current delegation switches do not hide admitted history.

Text and JSON output derive from one next_actions source. Read is progress, status
is state, result is result; suggestions do not invoke actions or guarantee Continue
rights. Closing a panel does not cancel or promise survival of full application exit.
Installation selection binds selected canonical components and that Harness's saved
selection revision, not a discovery-wide digest. One explicit selection saves it,
including a single candidate, without a second Apply-capability prompt. Different
native/adapter/SDK launch forms use one typed selection contract, never blank-string
or duplicate-path placeholders. An uncertain selection replays its exact request/
Operation, not another revision.

### Requirement: JSON sources are strict, unique, and bounded before transport

For commands supporting JSON input, select exactly one `--request-file PATH` or
`--request-stdin`. Duplicate/mixed selectors and mixtures with command-specific human
arguments fail `INVALID_ARGUMENTS` before Local Control. Files are regular, opened
without waiting on a FIFO; relative paths resolve from caller cwd and option-shaped
filenames need `./`. File mode never consumes stdin. Input is strict UTF-8 JSON,
rejecting trailing JSON, nested duplicate keys including escaped-equivalent keys,
and unknown typed fields including omitted optionals/tagged unit variants.
Maps/JSON values keep free keys; supported positional arrays are checked in field order.

Default limit is 1 MiB; Operation idempotency lookup/dependency selection keep 64 KiB,
and staged WorkPlan listing 4 KiB. This parser selects no business schema, runs no
shell, grants no capability, and does not tighten persisted recovery readers.
Secret/capability inputs remain independent. Worker `--file` stays prompt text.
`sessions receipt RECEIPT_ID` retains V1 SessionLookup/Receipt semantics; JSON input
accepts only V2 Facts intent using its logical RequestId and returns a Facts page.
Wrong intent fails locally; SessionId/ReceiptId cannot be guessed into RequestId.

### Requirement: Worker CLI has no persistent client receipt authority

Exec/Continue submit directly via Client Core with server-side canonical intent/key
deduplication. CLI does not read/write/migrate/repair legacy local receipt files;
`HIROUTE_WORKER_RECEIPT_DIR` has no effect. A read-only invocation directory cannot
prevent valid submit, same-key recovery, or Continue. Old local files stay unchanged.
If an automatically generated key's response is uncertain, the result exposes that
original key for explicit query/retry; there is no automatic new-key replay. Titles
are lookup aids, not deduplication authority.

### Requirement: Installed CLI resolves one explicit service instance

On macOS, CLI defaults to the current user's Desktop runtime. Absolute
`HIROUTE_RUNTIME_DIR` takes precedence over `XDG_RUNTIME_DIR`; empty/relative overrides
fail rather than fall back to another instance. Client Core retains socket-type,
protocol, and applicable managed-process identity checks without peer-UID admission.
Unavailable service points to start/recovery, not implicit daemon launch or request
replay.

Desktop can explicitly install a user-level `~/.local/bin/hiroute` link to its own
CLI without administrator access. Entry deployment, current PATH/actual command
resolution, and service availability are separate facts. Missing PATH gets user
instructions without modifying shell files. Managed Agent environments include
the verified entry directory; unverified external shells cannot be declared ready.
Entry conflicts/replacement/uninstall preserve foreign objects and exact ownership.

### Requirement: Standalone installation is user-owned and repeatable

Linux/macOS headless installation MUST converge a verified package to fixed owned
component paths with explicit service lifecycle and core Released CLI configuration.
The same management Skill is installed by default in
`~/.agents/skills/hiroute-management` and `~/.claude/skills/hiroute-management`, without
a required skill-selector flag. Skills invoke public CLI/shared validation, not
databases or test-only APIs, and cannot become a configuration authority. Credentials
use protected descriptor input rather than Agent conversation/argv/logs.

Desktop includes CLI. A standalone installer detecting Desktop stops before writes
and explains that existing CLI; an absent PATH or stopped service cannot authorize
a second installation. Switching requires stopping/uninstalling standalone before
Desktop, with separate data/runtime/service ownership and no implicit takeover or
migration. Repeating an install/update produces the current package's binaries,
CPA/resources/entries/service definitions/Skills/marker without disturbing adjacent
files, business data, or user configuration. Old program/resource roots are removed
only after success. Validation/conflict checks precede writes; partial replacement
rolls back or allows exact repeat to converge, never reporting an incomplete marker
as installed. Uninstall preserves foreign changes and data unless explicitly selected.

Start/status/stop manage only that installation; a compatible running instance is
reused. Login-start preference is independent from starting now. Environments without
user services offer foreground run. Ordinary business commands never implicitly
launch daemon. Planned commands fail in both CLI/daemon. Desktop and standalone
share the bounded protected-input socket and safe secret channel.

### Requirement: Linux packages pass ABI and installed-runtime validation

Both x86_64 and aarch64 headless packages use the Debian 11/glibc 2.31 floor. The
[baseline manifest](../../../scripts/linux-release-baseline.json) owns pinned images,
libraries, loaders, and symbol bounds. After inventory/digest checks, every ELF,
including CPA and future packaged dynamic libraries, is checked for architecture,
loader/library/symbol requirements, and absence of build-host search paths. Static
executables do not acquire a fabricated loader dependency.

A separate pinned minimal runtime image with a fresh HOME MUST run the production
installer, verify installed payload, CLI/CPA help, real service startup, and healthy
system status/gateway show twice across the resident daemon. Orderly stop leaves
management/Gateway unavailable. This check has no external network/model calls and
cannot rely on build directories/libraries. Compile success does not prove runtime
compatibility; rejected file/dependency evidence is retained. Candidate/platform
identity is explicit, and this evidence cannot replace macOS native or Windows checks.

### Requirement: CLI and Desktop management of Hermes connections

HiRoute SHALL add the selected effective Hermes profile as a managed target in the existing public CLI and Desktop Agent settings workflow. Both clients SHALL support selecting a published Plan, connecting or adjusting the profile, restoring managed settings and reading connection state through the shared backend. The CLI workflow SHALL be usable without Desktop. HiRoute SHALL make the existing management Skill discoverable in that profile independently of a model connection or task collaboration, with guidance for the existing source, decision-service and Plan commands. The existing shared-client-access and agent-connections contracts govern access, input, Operations and failures; this increment does not introduce a separate management API.

#### Scenario: Configure Hermes through the headless CLI

- **WHEN** a caller configures a routing Plan using existing public commands and connects a selected Hermes profile
- **THEN** the Hermes-specific target and connection operations are available through the public CLI without a graphical step; a missing required service/profile capability is reported for that target.

#### Scenario: Use the same connection from either client

- **WHEN** the owner selects a locally accessible Hermes profile in Desktop or updates its connection through the CLI
- **THEN** the existing Desktop Agent flow can configure, adjust, restore and read that same scoped backend state without manual native-file editing.

#### Scenario: Prepare management guidance independently

- **WHEN** the owner prepares the management Skill for a selected Hermes profile before connecting a model
- **THEN** the Skill is discoverable and explains the public routing-configuration workflow without requiring a Plan, model grant or collaboration setup; unavailable CLI/service prerequisites remain explicit.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165289916

### Requirement: Management Skill is available to supported local Agents

HiRoute SHALL provide independent management-Skill preparation, status and owned restoration through Desktop Agent settings and the public CLI for Codex, Claude Code, Qoder, Pi, DeepSeek Harness and Hermes, using each selected effective native Skill root and the same canonical management instructions. Preparation SHALL NOT require a managed model connection, a published Plan or an available Worker, and SHALL NOT enable automatic delegation, change model configuration or add CLI authority. Both clients SHALL reflect the same saved state and distinguish installed preparation, prerequisites and unverified runtime loading. Unknown or incompatible Skill roots SHALL produce an actionable result without writing another root.

#### Scenario: Prepare without model routing

- **WHEN** a supported Agent has a valid Skill root but no HiRoute model connection or published Plan, and the owner enables management instructions
- **THEN** only the selected management resource is prepared; the model and task-delegation settings remain unchanged.

#### Scenario: Cross-client control

- **WHEN** management preparation is applied from Desktop or CLI and the other client reads the same context
- **THEN** both report the same management state and expose owned restoration without requiring unrelated configuration.

#### Scenario: Unavailable prerequisite

- **WHEN** the selected root or native Skill loading interface is unavailable
- **THEN** the affected action identifies the prerequisite, writes no alternate root and does not claim runtime loading.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6186243593

### Requirement: Intent-led CLI configuration binds an explicit target and resumes existing work

HiRoute SHALL provide a CLI Quickstart for connecting a selected supported local Agent through an existing Plan or a new fixed-model or smart-saving Plan, and for inspecting, adjusting and restoring that connection. The calling Agent and managed target SHALL be independent. The caller supplies meaningful source, model/group and target choices; product defaults and existing business operations prepare the complete configuration without requiring hand-authored internal editors or transfer of digests/revisions. Existing selected sources, Plans and connections SHALL be reusable. Ambiguous targets or names SHALL return explicit choices before target writes. An explicitly selected unavailable target SHALL NOT fall back to another profile. A complete prior user choice SHALL NOT acquire repeated confirmation questions. Source preparation, publication and native settings retain their existing separate recoverable Operations; interrupted or uncertain work SHALL be queried or retried with its original identity rather than silently recreated. Quickstart SHALL present remaining activation and verification actions instead of declaring completion from saved configuration alone.

#### Scenario: Configure another Agent

- **WHEN** a supported management Agent or human selects a different supported target and an existing compatible Plan
- **THEN** the same public CLI path prepares the selected target only, reports its native scope and leaves the caller's model and unrelated facets unchanged.

#### Scenario: Choose smart saving directly

- **WHEN** the user selects economy and primary models and a supported decision method
- **THEN** the workflow prepares the smart-saving Plan directly with authoritative defaults and explicit choices; no temporary fixed-model Plan or full editor JSON is required.

#### Scenario: Ambiguous or unavailable target

- **WHEN** multiple native contexts match the selection or the explicit target becomes unreadable
- **THEN** the workflow returns disambiguation or repair actions without choosing the first match, switching roots or reporting successful configuration.

#### Scenario: Resume and preserve completed work

- **WHEN** a source or Plan already exists, or an Apply response is lost after submission
- **THEN** the workflow identifies reusable state and queries or retries the exact original operation; it does not create duplicate sources/Plans or roll back unrelated completed work.

#### Scenario: Adjust or restore one facet

- **WHEN** the user changes a selected route choice or restores the model facet
- **THEN** only the requested intent is previewed/applied through the existing owners, other settings remain intact and any required native follow-up is reported.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6206259270

### Requirement: Core management CLI exposes callable contracts and authoritative initial configuration

For the Quickstart and its supported branches, HiRoute SHALL expose navigable family help, complete installed request/response schema definitions including supported wrapper variants, valid input examples and bounded field-level input errors. Dynamic IDs and model choices SHALL come from current queries rather than examples or invented references. The fixed-model and smart-saving convenience paths SHALL obtain initial settings from a common product-owned defaults/capability projection also consumed by Desktop, including judgment settings, request limits, context bounds and valid native reasoning choices. Querying defaults SHALL NOT save, publish, import credentials or invoke models. Existing Plan edits SHALL retain unmodified settings. A supported decision service and local rules SHALL remain explicit distinct choices; a normal Chat execution binding SHALL NOT be silently treated as a compatible decision service. Existing released machine output, exit semantics, protected channels and request interpretation SHALL remain compatible; new explanatory fields SHALL use a compatible or explicitly selected contract.

#### Scenario: Discover a valid request offline

- **WHEN** a caller has the installed CLI but no source checkout or validation artifacts and requests help/schema for a Quickstart command
- **THEN** the caller can determine required fields, enums, request wrappers and a valid minimal input; examples identify the queries supplying dynamic references.

#### Scenario: Prepare defaults without copying a sample

- **WHEN** a caller chooses fixed-model or smart-saving authoring and current candidates
- **THEN** the product returns the complete valid initial structure and effective defaults/capability bounds; neither the Skill nor CLI invents maximum context, retry limits or highest reasoning settings.

#### Scenario: Diagnose invalid input

- **WHEN** a request contains a missing required member, wrong wrapper, unsupported enum, wrong type or unknown field
- **THEN** the failure identifies the bounded field location and expected correction, retains usage failure semantics and does not expose credentials or echo the raw request.

#### Scenario: Unsupported decision binding

- **WHEN** a caller requests an ordinary Chat model as the decision service
- **THEN** the workflow identifies the unsupported role and offers explicit supported-service or local-rules choices without changing the requested policy silently.

#### Scenario: Preserve an existing editor

- **WHEN** a caller changes one field of an existing Plan using the convenience path
- **THEN** all other existing settings retain their values rather than being reset from new-plan defaults.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6206259606

### Requirement: One canonical management Skill guides complete generic CLI journeys

HiRoute SHALL ship one canonical management Skill for all supported management Agent families, with the core Quickstart and bounded references for important branches: absent service/target/source, ambiguous scope, unavailable models or decision service, existing/partial setup, adjustment, effective-state diagnosis, observation and owned restoration. The Skill SHALL separate the caller from the managed target and consume installed public CLI capabilities, generated defaults and machine results rather than assuming Hermes, a particular filesystem layout or repository access. It SHALL explain how to obtain prerequisite CLI/Skill availability without coupling management to model routing or delegation. Instructions SHALL capture stdout, stderr and the CLI exit status separately when the native terminal merges them, inspect business/Operation results, preserve submission identity and avoid first-line JSON parsing or treating accepted as complete. Generated configuration and personal experience notes SHALL NOT be written into the product-managed Skill. Existing independent facet ownership, user-authorized effects, protected credentials and collaboration trigger policy remain authoritative. Before a restoration, reconnect or credential replacement that may revoke the current caller's own HiRoute access, the Skill SHALL establish an independent continuation path or leave that effect pending for an external operator. Verification SHALL distinguish saved state, native loading, request outcomes and actual classifier/group selection; manual model substitution or missing observations cannot establish a semantic decision branch or native activation. Before changing an existing Plan, the Skill SHALL retain its complete prior effective configuration and provide fully resolved recovery commands to the user, usable from an independent terminal if the caller fails. Plan restoration SHALL be distinguished from owned Agent-facet restoration. Restoring candidate names alone is not full configuration recovery; compare all effective settings after restoration.

#### Scenario: Use the same Skill across callers

- **WHEN** any supported Agent invokes management for itself or another supported local target
- **THEN** the shared instructions guide the same public capability path and use the selected target's returned native requirements, without a per-caller copy of management logic.

#### Scenario: Operate without development artifacts

- **WHEN** an installed management Agent is asked to connect, adjust, inspect or restore an Agent
- **THEN** the instructions and installed public commands provide the needed path without source code, binary inspection, test fixtures or manually patched Skill contents.

#### Scenario: Capture a merged terminal result

- **WHEN** the native tool presents stdout and stderr together or wraps command execution
- **THEN** the guidance captures and parses the complete stdout JSON and preserves the real CLI return code separately; diagnostic text or a later shell command cannot be mistaken for CLI success.

#### Scenario: Keep capabilities independent

- **WHEN** the user enables management only or delegation only while leaving the caller's model native
- **THEN** the Skill uses the requested facet and applicable collaboration policy without configuring a model route or creating a Worker requirement for management.

#### Scenario: Preserve managed instructions

- **WHEN** an Agent learns an environment-specific workaround or encounters externally modified Skill content
- **THEN** it keeps operational notes outside the managed resource and reports existing drift/ownership rather than silently rewriting or force-upgrading the resource.

#### Scenario: Preserve the managing caller continuation

- **WHEN** a requested change may invalidate the active caller's model access
- **THEN** the guidance establishes independent access or an external handoff before the effect; restoring files alone is not evidence that the running caller has reloaded them.

#### Scenario: Report actual decision and execution evidence

- **WHEN** a smart-saving journey is verified
- **THEN** the guidance checks actual classifier/group/model facts and every matching request outcome, distinguishes pending or failed requests, and does not infer complex-branch coverage from changing the economy model or infer built-in rule behavior solely from custom keywords.

#### Scenario: Provide complete recovery before a change

- **WHEN** an Agent prepares a change to an existing Plan or managed connection
- **THEN** it retains the complete applicable baseline, identifies recovery scope, supplies concrete CLI Preview and Apply commands with real identities and paths, and verifies restored effective settings rather than only model names; uncertainty, foreign edits and caller continuation follow existing contracts.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6206260381

### Requirement: Discover and securely connect reusable decision services

HiRoute SHALL provide an installed, discoverable CLI path for choosing and connecting a supported decision service during smart-saving setup. It SHALL first expose reusable saved connection identities and revisions, separately from connectivity or decision-quality evidence, and expose the installed decision-provider presets and compatible/custom extension choices with their required non-secret inputs. Presets SHALL NOT assert current provider availability or account entitlement. CLI and Desktop SHALL consume one preset definition. A user SHALL be able to enter a credential in their own terminal without echo or inclusion in chat, command arguments, shell history, environment variables, ordinary request files or logs, using the existing protected-input and Secret lifecycle. Agents SHALL receive registration results and references only. Cancelling input or lacking a controlling terminal SHALL fail without saving a service or changing existing credentials. The journey SHALL continue through existing reviewable save, explicit connection test, exact revision selection for a Plan and publication, preserving exact retry identity and unrelated state. Local rules SHALL remain an explicit alternative, without a claimed model decision or implicit replacement of a selected service. This path SHALL NOT reinterpret an ordinary Chat model as a decision service.

#### Scenario: Reuse a saved service with honest readiness

- **WHEN** smart saving is configured and decision connections already exist
- **THEN** the installed path lists exact identities and revisions for explicit reuse, with testing unknown until a current explicit probe establishes its outcome; a saved record is not presented as usable solely because it exists.

#### Scenario: Choose a supported connection

- **WHEN** no suitable saved service is selected
- **THEN** the installed path exposes its preset provider/model/endpoint choices, required endpoint/model choices for compatible/custom connections, and the secure user credential step without requiring a source checkout or claiming account access.

#### Scenario: Register a credential outside the conversation

- **WHEN** the user runs the provided terminal prompt
- **THEN** input is hidden, the selected authorization format is applied without exposing the credential, and only a registration result/reference is returned through the ordinary output; cancellation or no controlling terminal leaves saved resources unchanged.

#### Scenario: Continue from registration to published routing

- **WHEN** a protected credential has been registered and the user authorizes the connection
- **THEN** existing Preview/Apply saves a revision with an exact replayable request, an explicit probe reports its own result, and the selected immutable revision can be bound to a Plan and published without new duplicate services or a new routing backend.

#### Scenario: Retain an explicit local alternative

- **WHEN** a user chooses local rules or cannot yet provide a usable decision connection
- **THEN** the CLI keeps that choice explicit, preserves existing setup, explains remaining verification and does not claim semantic model decisions or silently downgrade an existing service.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6213380806

### Requirement: Recover complete saved Plan configuration through the public CLI

HiRoute SHALL let an installed CLI caller retain the complete effective configuration of an existing Plan and prepare restoration of that configuration without reconstructing editor fields or guessing defaults. Recovery SHALL preserve candidate order and reasoning, decision service and judgment, requirements, limits/context, alias and supported mode/delegation settings. It SHALL target the same existing Plan, use current revision-checked Preview and exact Apply, report invalid or unavailable referenced resources and preserve unrelated resources and observation history. A failed, malformed or different-target saved response SHALL NOT become a restoration. Plan configuration recovery does not rewind immutable revisions or replace native Agent-facet restoration.

#### Scenario: Recover after candidate replacement

- **WHEN** a caller changes a group candidate and later restores a retained complete Plan configuration
- **THEN** the restored effective configuration matches the retained baseline including reasoning and policy, even if selecting that candidate afresh would choose different defaults.

#### Scenario: Review and submit recovery

- **WHEN** a valid saved configuration for the selected Plan is supplied
- **THEN** the installed path resolves the current target revision, produces a reviewable existing Plan content Preview and optionally an exact private Apply file; no restoration occurs until the caller applies it.

#### Scenario: Reject invalid or different-target recovery

- **WHEN** the saved output is incomplete, unsuccessful, malformed, from another Plan/workspace, or includes incompatible extra change inputs
- **THEN** the CLI rejects it before target writes, without silently creating a Plan or dropping saved fields.

#### Scenario: Keep stale and dependency failures safe

- **WHEN** a prepared restore becomes stale or its retained resource references are unavailable
- **THEN** existing revision/dependency failures remain authoritative; the caller reviews current facts and prepares a fresh request instead of bypassing checks or silently substituting models/services.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6219937964

### Requirement: Analyze retained session evidence through same-user CLI management

HiRoute SHALL let the authenticated same-OS-user Local Control CLI read retained session timelines, explicit message/tool content, searchable content and routing/competence evidence for user-authorized model and decision analysis without a separate graphical approval or protected read ticket. Content remains explicitly selected, bounded, paginated and subject to current workspace/run visibility and retention. An explicitly supplied protected grant SHALL still be validated and a model Gateway credential SHALL NOT grant Local Control access. The common management Skill SHALL connect actual content to request/publication, decision source, branch and competence evidence, distinguish missing evidence, and propose reviewable changes with complete recovery rather than silently tune routing.

#### Scenario: Analyze a historical conversation

- **WHEN** a local management Agent requests a retained session and explicitly selected content
- **THEN** the CLI returns the available conversation and request evidence without requiring a Desktop-created grant; missing or expired content is reported rather than fabricated.

#### Scenario: Follow bounded evidence pages

- **WHEN** analysis requires content search, request facts or competence samples
- **THEN** the public CLI can follow the existing scoped queries and continuations, preserving identifiers, visibility and partial coverage.

#### Scenario: Preserve actual authority

- **WHEN** a caller supplies an invalid protected grant or only a model Gateway credential
- **THEN** the invalid authority is rejected; the read change does not grant remote model clients management access.

#### Scenario: Review an evidence-based adjustment

- **WHEN** an Agent identifies an apparent simple/complex or competence mismatch
- **THEN** it reports the linked evidence and uncertainty, retains complete prior configuration and uses the existing reviewable change/recovery path.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6220387323
### Requirement: Windows headless preserves the logged-on user's native authority

Windows standalone MUST use the actual user's Profile and LocalAppData KnownFolders
for installation and runtime ownership. HOME/XDG values cannot redirect an installed
instance. Native headless operation uses the shared CLI, role-all daemon, Application,
Gateway and business stores without requiring Desktop or WSL.

The default Task Scheduler manager runs as the current logged-on user with an
InteractiveToken and least privilege. Ordinary install/start/stop does not require
administrator access or an account password. The background process MUST survive
closure of its launching terminal. Its lifetime requires that user's logged-on
session; login-start preference and starting now remain separate operations.
Unexpected business-process exit uses bounded recovery, while an explicit stop
terminates the owned child tree and cancels pending recovery. Manager registration
or action launch alone is not proof of healthy Control and Gateway service.

Installation MUST verify the complete package inventory, source revision, file
digests and native executable architecture before publishing stable entries.
A prepared replacement blocks business startup until package files, installation
record and actual manager definition agree. Repair/rollback preserves saved Sources,
secrets, routes, Agent recovery references and observations instead of replaying
business mutations or initializing empty stores. Uninstall preserves business data
and unrelated native configuration.

Local Control, lifecycle and protected input use private named pipes bound to the
account SID, held runtime objects and verified native peers. Protected credential
carriers use explicitly inherited non-stdio pipe handles with bounded input and EOF;
argv, environment variables and ordinary JSON input do not supply that authority.
Remote peers, replaced objects and occupied endpoints cannot impersonate the instance.

An optional SCM installation is a separate administrator operation over verified
administrator-controlled package bytes. Its privileged host cannot execute an
arbitrary user-selected image. Task acceptance cannot establish service-account,
logout or SCM acceptance; native OS, architecture and real-provider claims require
their own revision-bound execution evidence.

## Implementation references

- [Client Core](../../../crates/client-core/src) and
  [CLI request-input guide](../../../crates/cli/INPUT.md).
- [Windows installation and manager boundaries](../../../docs/standalone-windows.md),
  [native platform owners](../../../crates/platform/README.md), and
  [installed background and upgrade scenarios](../../../crates/daemon/tests/windows_background.rs).
- [Real argv scenarios](../../../crates/cli/tests/request_input.rs) and
  [stateless Worker submission scenarios](../../../crates/cli/tests/worker_submission.rs).
- [Release builder/installation contract](../../../docs/release-builds.md).
