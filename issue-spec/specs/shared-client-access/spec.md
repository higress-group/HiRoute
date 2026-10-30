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

Local Control accepts processes that can access its socket or private named pipe, without extra peer-UID, owner/mode checks or client-source-specific Apply capability. CLI, Desktop and console sessions share the same user/idempotency/recovery scope. Application still verifies normalized intent, current versions, and exact admission; Preview/cancelled confirmation does not write. Model grants and remote collaboration authority remain separate.

The HTTP console is the only network-reachable management path. It MUST authenticate every management request through a console session derived from a Local Control launch credential or an owner access token; network reachability, a Gateway grant or a page origin alone MUST NOT grant management authority.

WebView and browser pages may invoke only purpose-specific Tauri ACL commands or authenticated console operations. They cannot self-assert principal, obtain arbitrary Local Control/shell/file/SQL access, or give remote content local management authority beyond the authenticated console session. Domain pages expose real capabilities and explicit unsupported/unavailable state, never fixture-backed success. A host shell can open offline and present implemented recovery without requiring internal component knowledge; provider communication stays outside WebView and browser pages.

Authenticated console sessions have the same protected-operation authority as macOS Desktop, without a password or unlock step. Protected operations retain in-page confirmation and exact-digest registration. Collaboration/Skill-authorized callers cannot mint console credentials. Protected uses are audited without credentials, secret input or conversation bodies.

#### Scenario: Network peer without a console session

- **WHEN** a process reaches the console listener over the network but presents no valid console session
- **THEN** no management operation or product data is available to it, while OS-trusted Local Control access remains unchanged.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219725852

### Requirement: Native connection loss is truthful and safely recoverable

Protected grant registration SHALL use a launcher-only connection opened once
from the coordinated, admitted Control database. It SHALL retain that database's
physical identity, UUID, schema and startup ownership without creating, migrating
or initializing another file. Unrelated Runtime/Secrets reads under the shared
storage mutex SHALL NOT refuse registration. Registration SHALL try its narrow
connection and the actual SQLite writer reservation without waiting behind an existing owner. A refusal
before writer admission SHALL create no capability or business operation and
SHALL finish the exact acknowledgement with `registered:false`. This result is a
retryable registration refusal, never successful authorization. Only a complete,
strictly decoded grant reply with the original schema and registration identity
may preserve the synchronized native connection. Desktop SHALL return the busy
failure without issuing or automatically replaying the business request.

Successful registration SHALL retain its existing durable commit before
`registered:true`. Errors after writer admission, uncertain commit, expiry,
identity or framing failures SHALL NOT become a busy refusal. The connection's
ordinary SQLite wait policy SHALL be restored on all normal return paths.
The existing two-second host deadline remains unchanged; it still detects an
uncertain response or slow durable operation. Input and upgrade exchanges retain
their current strict meanings.

Positive v2 acknowledgement bytes remain compatible. A previous host may
conservatively reject the newly meaningful negative reply; paired current host
and daemon provide retryable busy behavior. No alternate long-lived handshake or
persistent-format conversion is introduced.

#### Scenario: Busy grant admission preserves a synchronized connection

- **WHEN** the registrar connection or an external SQLite writer is held while
  Desktop registers an exact grant
- **THEN** the daemon promptly returns a matching unregistered acknowledgement,
  no grant or business effect exists, and Desktop keeps its channel usable. A
  later user request after release uses a fresh registration and may succeed
  with the same daemon PID. A failed write/commit cannot claim this refusal.

- **WHEN** an unrelated reader holds the shared storage mutex while the Control
  writer is available
- **THEN** registration can commit without waiting for that reader, with the same
  durable authorization, expiry and exact request checks.

A responsive daemon SHALL NOT establish that the Desktop's inherited protected
channel remains usable. Uncertain, timed-out, malformed or mismatched exchanges SHALL
invalidate that channel without accepting a late response for another registration.
Protected observations and actions SHALL fail closed with an actionable connection
failure; service status SHALL distinguish that failure from daemon unavailability
and from an intentional read-only connection. Existing loaded records and unsaved
non-secret input SHALL remain available without being presented as fresh reads.

Desktop SHALL offer recovery for a fully written request whose exact acknowledgement
is still pending. It SHALL retain partial reply bytes within the existing bound,
require the original schema and registration identity, and complete that reply
before any new registration. This only restores transport synchronization; it SHALL
NOT authorize or automatically replay the failed business operation. Incomplete
request writes, closed channels and malformed/mismatched replies SHALL remain
denied and require normal reopening after existing work finishes.

Successful synchronization SHALL create a distinct product session, expire old
confirmations, retire old authority and release abandoned session-owned inputs.
It SHALL keep the existing daemon, inherited descriptors, host lease and listener;
it SHALL NOT pause admission or interrupt running requests/tasks. It SHALL preserve
saved configuration, retained sessions and operation identities without relaxing
digests, revisions or principals. A missing or failed recovery remains explicit.
External CPA revalidation SHALL NOT hold the shared storage needed by native grant
registration across a control/lifecycle wait.

The first failed exchange SHALL have bounded diagnostics identifying its phase,
closed failure category, duration and irreversible registration correlation. Neither
capability/input bytes, business payloads nor raw registration identifiers may appear.
A controlled timeout reproducer SHALL NOT be described as the cause of an earlier
unobserved failure.

#### Scenario: Daemon responds after an uncertain protected exchange

- **WHEN** the owned daemon is responsive after a protected registration times out
- **THEN** Desktop reports loss of its trusted connection, protected reads remain
  denied and ordinary Retry cannot treat a late ACK as new authorization.

#### Scenario: Recovery synchronizes a pending reply without stopping work

- **WHEN** the user restores a timed-out native connection with a fully written request
- **THEN** only its exact pending reply can restore synchronization; a new Desktop
  session restores protected reads/actions while the same daemon and running work
  continue, and old confirmations and authority cannot resume.

#### Scenario: Invalid framing cannot acquire recovery authority

- **WHEN** a request is incompletely written or its reply is malformed, mismatched or closed
- **THEN** in-place recovery remains unavailable, the daemon is not stopped, and
  Desktop explains normal reopening after existing work finishes.

Source Issue: https://github.com/higress-group/HiRoute/issues/187

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

### Requirement: Native Desktop presents the intended launch

A normal macOS application launch SHALL show the first host's main window and
load its initial Home without another open action or a manual read retry. The
host SHALL obtain the native launch reason through the platform lifecycle,
retain an early result until window setup completes, and preserve the framework's
handling of that event. A proven login-item launch or
explicit background start SHALL initialize the service without flashing a
window. A duplicate host SHALL hand off without presenting a second window.
An explicit later reopen SHALL show the existing window normally.

Ordinary reads SHALL validate the originating client's continued existence and
authority, current session and publication consistency independently of whether
its window is shown. Hiding a live native window SHALL NOT turn a successful read
into a service failure. A destroyed or revoked client SHALL NOT receive a late
result. UI request identity SHALL prevent superseded results replacing current
page state. A failed page read SHALL NOT by itself label the daemon unavailable.

Protected confirmations SHALL retain the originating context, exact operation
and revision, expiry and explicit user decision. An invisible or invalid native
window SHALL NOT accept a pending confirmation. Already admitted operations
SHALL retain their backend operation identity and observable terminal result
when the user hides or closes the window; reopening SHALL NOT resubmit them.

#### Scenario: Foreground launch event precedes window setup

- **WHEN** the native open event arrives before Desktop's presentation state exists
- **THEN** its foreground reason SHALL survive setup, the main window SHALL become
  visible, and its initial Home SHALL load without user retry.

#### Scenario: Login or background service starts without presentation

- **WHEN** the launch is proven to be a login item or explicitly requests background mode
- **THEN** startup SHALL not show a window; a later explicit reopen may present it,
  while reads from a destroyed or invalidated client retain their rejection.

#### Scenario: A live hidden window finishes an ordinary read

- **WHEN** a valid Home, session or trial-configuration read completes while the
  originating native window is hidden
- **THEN** the result remains usable under the current authority and publication
  checks, and showing the window does not require repairing a service connection.

#### Scenario: Client authority expires during a read

- **WHEN** the originating client is destroyed or revoked while a read is pending
- **THEN** the late result SHALL be rejected even if another window is visible.

#### Scenario: A window is hidden while confirmation is pending

- **WHEN** a protected action awaits confirmation and its native window becomes
  invisible or its context expires
- **THEN** the action SHALL not be newly admitted from that confirmation; already
  admitted operations remain queryable by their original identity.

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

#### Scenario: Follow the published API connection-test examples

- **WHEN** a caller reads installed help and full schemas to test a catalog option or custom API
- **THEN** help commands specify an actual JSON input source and request document, and the schemas provide complete `registered` and `native` templates with their strict nested wrappers. Registered guidance obtains the complete expected catalog from current connection options and credentials through protected-input registration. Custom API guidance identifies caller-supplied endpoint/protocol facts. Templates select no inference model by default and distinguish syntactic validity from a successful provider check.
- **AND** missing input, a flattened request or malformed command options retain usage-error semantics and give bounded corrective guidance without exposing credentials or echoing the raw request.
- **AND** guidance distinguishes command completion and catalog candidate availability from provider verification. For a key-authenticated option with neither a supported inventory path nor an explicitly selected inference model, the check performs no remote probe; `reachability=not_run`, `authentication=unknown`, `directory=not_run` and `inference=not_run` leave connectivity and credentials unverified. Published recipes explain these results and do not claim remote verification from exit code 0 or a successful envelope.

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
- https://github.com/higress-group/HiRoute/issues/162

### Requirement: One canonical management Skill guides complete generic CLI journeys

HiRoute SHALL ship one canonical management Skill for all supported management Agent families, with the core Quickstart and bounded references for important branches: absent service/target/source, ambiguous scope, unavailable models or decision service, existing/partial setup, adjustment, effective-state diagnosis, observation and owned restoration. The Skill SHALL separate the caller from the managed target and consume installed public CLI capabilities, generated defaults and machine results rather than assuming Hermes, a particular filesystem layout or repository access. It SHALL explain how to obtain prerequisite CLI/Skill availability without coupling management to model routing or delegation. Instructions SHALL capture stdout, stderr and the CLI exit status separately when the native terminal merges them, inspect business/Operation results, preserve submission identity and avoid first-line JSON parsing or treating accepted as complete. Generated configuration and personal experience notes SHALL NOT be written into the product-managed Skill. Existing independent facet ownership, user-authorized effects, protected credentials and collaboration trigger policy remain authoritative. Before a restoration, reconnect or credential replacement that may revoke the current caller's own HiRoute access, the Skill SHALL establish an independent continuation path or leave that effect pending for an external operator. Verification SHALL distinguish saved state, native loading, request outcomes and actual classifier/group selection; manual model substitution or missing observations cannot establish a semantic decision branch or native activation. Before changing an existing Plan, the Skill SHALL retain its complete prior effective configuration and provide fully resolved recovery commands to the user, usable from an independent terminal if the caller fails. Plan restoration SHALL be distinguished from owned Agent-facet restoration. Restoring candidate names alone is not full configuration recovery; compare all effective settings after restoration.
 The same Skill SHALL guide a complete read-only analysis path for transition explanations, linked conversations, session/request spending ranking and recent savings using installed public commands. It SHALL resolve an unambiguous requested scope, reference and saved rates from current public facts, ask only for genuinely ambiguous/missing choices, and show basis/coverage with the answer. It SHALL distinguish retrying uncertain delivery from a new later restoration: after a completed restoration and subsequent edits, recovery uses a fresh current-head Preview and new Apply identity; replaying the old completed operation is not a new restore. The canonical Skill SHALL remain installable within the existing managed Skill resource limit and ownership contract; detailed options, enums, units and examples SHALL come from installed command help and schemas rather than an enlarged, copied or second Skill resource.

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

#### Scenario: Follow one analysis journey across supported callers

- **WHEN** the user asks any supported management Agent why a model changed, which conversation cost most or what routing saved recently
- **THEN** the installed shared instructions navigate summary/ranking, exact request evidence and relevant paged content without repository access, model probes or caller-specific playbooks; they disclose missing evidence and do not silently tune settings.

#### Scenario: Reuse saved rates and resolve a comparison

- **WHEN** the requested Plan has one primary model and applicable saved rates in one currency
- **THEN** the Skill resolves and discloses the exact reference and basis without repeated technical-ID entry; ambiguous choices or missing rates lead to bounded choices or the existing price-completion branch.

#### Scenario: Restore again after a later change

- **WHEN** a previous recovery Apply succeeded and the user later changes the Plan again
- **THEN** the instructions generate a new Preview and Apply file using the retained complete baseline and current head; the prior successful operation is never presented as a fresh restoration.

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
 The installed management path SHALL provide navigable bounded conversation and search results at retained-history scale, associated with actual request/model/reasoning and available routing/value evidence. Relevant reads SHALL constrain the selected scope before expensive expansion; query/price/routing facts SHALL NOT depend on loading full bodies or a ready full-text index. Search SHALL distinguish actual no-match from pending indexes, incomplete scanning and failure. Timeout, busy-reader, index and storage unavailability SHALL have safe distinguishable reasons and actionable continuation through compatible public results, rather than falsely implying all content is absent. Default-period and calendar-day filters SHALL report the effective timezone and absolute bounds, and a continuation SHALL keep that resolved scope. Existing resource limits and retained old interfaces remain constraints; new paged views are explicit. Opt-in conversation/search views SHALL apply session-history's "Verified native compaction links preserve session identity" to expose proven compaction continuations with member anchors, current visibility and explicit gaps; unchanged legacy calls retain their physical-session scope. Related opt-in pages and conversation/member/request drill-down SHALL accept the originating result's relation watermark, without re-resolving a different live grouping; later relation additions do not invalidate that pinned scope. Invalidated proof and current visibility remain explicit. A new analysis scope SHALL disclose requested and retention-safe effective bounds and its lifetime before pagination. Within that lifetime, routine retention cleanup SHALL NOT change its selected population, grouping, coverage or monetary totals. Any clipped interval SHALL be explicit; complete results apply only to the effective interval, never an omitted part of the requested period. This SHALL NOT extend original evidence retention. Explicit deletion, proof withdrawal and authorization changes still apply, while token expiry has a distinct refresh action. A normal reopen with already aligned retained projections SHALL reuse them without introducing reconciliation pending. Only views requiring known missing or stale retained projections may report catch-up pending, with scoped progress and a supported retry action; ready reads and valid scopes SHALL remain usable under current source/visibility checks. Interrupted catch-up SHALL resume without restarting for unrelated aware traffic, and stalled work SHALL report its reason rather than remain indefinitely plain pending. Missing historical evidence is an explicit gap, not unfinished reconciliation.

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

#### Scenario: Read a small relevant page among large unrelated history

- **WHEN** a retained session/request has readable content and substantial unrelated history exists
- **THEN** the supported bounded CLI view returns the relevant page and continuation without requiring a whole-history scan or exposing unrelated content; an old unpageable request outside supported bounds gives an actionable bounded path.

#### Scenario: Search without fabricated emptiness

- **WHEN** the user searches within a selected interval or session and indexing is pending or the query cannot complete
- **THEN** the result identifies partial coverage or a safe failure reason with a supported next action; it does not report a complete empty match set or fabricate content loss.

#### Scenario: Read facts independently of body indexing

- **WHEN** full-text indexing or selected body data is unavailable but retained request/routing/usage facts remain readable
- **THEN** the CLI still exposes those facts and exact evidence references while reporting body availability independently.

#### Scenario: Preserve scoped continuation

- **WHEN** an Agent follows a body/search page while unrelated traffic arrives or a body is deleted
- **THEN** the continuation retains its declared query semantics, rechecks current visibility and follows existing retention/deletion rules without inventing a missing body or crossing sessions.

#### Scenario: Read across a proven compaction boundary

- **WHEN** retained content spans physical sessions linked by verified native compression evidence
- **THEN** the bounded conversation/search view can navigate the visible chain with member/request anchors and honest gaps, without counting repeated captured context as new messages or usage.

#### Scenario: Keep a conversation drill-down on the originating relations

- **WHEN** a caller opens a ranked or searched conversation after a new native compression has been recorded
- **THEN** reusing the originating analysis scope retains its relation watermark and member anchors; a new analysis is needed to include the later continuation, and current visibility is still enforced.

#### Scenario: Continue through routine history cleanup

- **WHEN** a paged analysis is opened near the rolling retention boundary and ordinary cleanup runs during its declared valid lifetime
- **THEN** the upfront effective bounds and retained evidence keep pagination and drill-down consistent, without global invalidation from unrelated cleanup; clipping and token expiry remain visible and deleted content is never restored.

#### Scenario: Reuse aligned history after an ordinary restart

- **WHEN** HiRoute reopens already aligned retained data without an intervening unaware writer or interrupted catch-up
- **THEN** new analysis reads reuse the saved projections without a new reconciliation-pending period; actual source gaps, unsettled usage and turn incompleteness keep their distinct meanings.

#### Scenario: Expose genuine catch-up without blocking ready reads

- **WHEN** retained data needed by one view is missing from derived projections after compatible writer mutations
- **THEN** only the affected view reports scoped progress and retry guidance; ready reads continue, interrupted work resumes, and stalled or unavailable work has an explicit reason instead of endless plain pending.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6220387323

### Requirement: Windows headless preserves the logged-on user's native authority

Windows standalone MUST use the actual user's Profile and LocalAppData KnownFolders for installation and runtime ownership. HOME/XDG values cannot redirect an installed instance. Native headless operation uses the shared CLI, role-all daemon, Application, Gateway and business stores without requiring Desktop or WSL. Windows installation, update and uninstall are performed by the native installation transaction used by the Windows Desktop installer; no separate Windows headless installer or Python runtime is required, and a user who needs only the CLI and daemon installs the same package.

The default Task Scheduler manager runs as the current logged-on user with an InteractiveToken and least privilege. Ordinary install/start/stop does not require administrator access or an account password. The background process MUST survive closure of its launching terminal. Its lifetime requires that user's logged-on session; login-start preference and starting now remain separate operations. Unexpected business-process exit uses bounded recovery, while an explicit stop terminates the owned child tree and cancels pending recovery. Manager registration or action launch alone is not proof of healthy Control and Gateway service.

Installation MUST verify the complete package inventory, source revision, file digests and native executable architecture before publishing stable entries. A prepared replacement blocks business startup until package files, installation record and actual manager definition agree. Repair/rollback preserves saved Sources, secrets, routes, Agent recovery references and observations instead of replaying business mutations or initializing empty stores. Uninstall preserves business data and unrelated native configuration.

Local Control, lifecycle and protected input use private named pipes bound to the account SID, held runtime objects and verified native peers. Protected credential carriers use explicitly inherited non-stdio pipe handles with bounded input and EOF; argv, environment variables and ordinary JSON input do not supply that authority. Remote peers, replaced objects and occupied endpoints cannot impersonate the instance.

Task Scheduler is the only installation manager offered in this release. SCM service installation has no supported entry point or user documentation; installation, update and uninstall refuse to modify an SCM-managed installation and explain why. Retained SCM platform code MUST NOT execute an arbitrary user-selected image while privileged. Task acceptance cannot establish service-account, logout or SCM acceptance; native OS, architecture and real-provider claims require their own revision-bound execution evidence.

#### Scenario: Install without Python

- **WHEN** a standard user on Windows x64 without Python installs HiRoute
- **THEN** the native installation transaction publishes and registers the Task Scheduler installation, and the CLI and daemon run headless without Desktop being opened.

#### Scenario: SCM installation is not offered

- **WHEN** a user looks for a way to install HiRoute as a Windows service, or the installer finds an SCM-managed installation
- **THEN** no supported command or documented path installs an SCM service, and the existing SCM installation is refused unchanged with an explanation.

#### Scenario: Task lifetime

- **WHEN** the user closes the terminal that started the service and later logs off
- **THEN** the service survives the terminal and stops with the user's session, and login start remains a separate preference.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6228861461

### Requirement: The daemon serves the shared product console

A role-all `hirouted` on Linux and Windows SHALL be able to serve the HiRoute product frontend and its control operations over HTTP on a console listener that is separate from the Gateway listener. Console pages SHALL use the same Application owners, typed operations, results and failure codes as CLI and Desktop, and SHALL present the same saved state. Gateway credentials SHALL NOT authenticate console requests and console sessions SHALL NOT authorize model requests. On Linux standalone the console SHALL be disabled until the owner explicitly configures it during installation or initialization; enabling it SHALL NOT start the daemon implicitly. Host-only features unavailable in the current host, such as in-app update, window behavior and login items, SHALL be absent or explicitly unavailable rather than failing or reporting fixture success.

#### Scenario: Open the console from the Linux CLI

- **WHEN** the console is configured, the standalone service has established its listener and the owner runs the documented console open command
- **THEN** the command returns a single-use link for the actual local or explicitly configured network listener; opening it in a browser shows the product pages with the current service status, sources, plans, Agents, Workers and sessions.

#### Scenario: Cross-client consistency

- **WHEN** a source or plan is changed through the CLI while a console page is open
- **THEN** after refresh the console shows the same saved revision the CLI reports, and a console edit based on the old revision is rejected as a conflict.

#### Scenario: Credential separation

- **WHEN** a request presents a Gateway grant key to the console or a console session to the Gateway
- **THEN** the request is rejected without returning data or executing a model request.

#### Scenario: Service not ready

- **WHEN** the console listener answers while Local Control or the Gateway is not ready
- **THEN** the console shows the actual unavailable state and recovery action instead of an empty or successful page.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219723541

### Requirement: Console sessions require explicit owner authentication

Every console request other than static frontend assets and the credential exchange SHALL require an authenticated session. A launch credential SHALL be issued only through the OS-trusted Local Control path for an actually listening console. It SHALL be single-use, expire after two minutes, and be bound to the selected console origin including its actual port. The owner CLI SHALL return a local or remote login URL according to that live listener. A network-origin credential MAY be exchanged remotely only when network listening was explicitly configured; a loopback-origin credential SHALL NOT be redeemed remotely. A mismatched origin or port SHALL NOT consume an otherwise valid code. The page SHALL remove the code from its URL before exchange. The resulting session credential SHALL be bound to the console origin including its port. The session credential SHALL be held only in browser sessionStorage and sent in an explicit Bearer request header. It SHALL NOT be placed in cookies, URLs, localStorage or other persistent browser storage, so cross-site requests and other services on the same host never receive console authority. The owner SHALL be able to create, list, expire and revoke named access tokens through the CLI; token secrets SHALL be shown only at creation and SHALL NOT be stored in recoverable form. Revoking a token SHALL end the sessions derived from it. State-changing requests SHALL be rejected when Host or Origin does not match the configured console or when cross-site request protection fails. Console operations SHALL carry a distinct principal identity that operation history and diagnostics can attribute; this increment supports a single owner principal.

An authenticated console session SHALL have the same protected-operation authority as macOS Desktop, without a password or unlock step. Every protected operation SHALL retain its in-page confirmation and exact-digest authority registration. Callers operating under collaboration or Skill authorization SHALL NOT mint console launch credentials or access tokens. Protected-operation uses SHALL be audited without credentials, secret input or conversation bodies.

Sessions SHALL remain in memory only, expire after 12 hours idle or seven days total, and be bounded to 16 active sessions. A valid new login at capacity SHALL retire and audit the least recently used session. Invalid credentials SHALL NOT evict a session. Failed exchanges SHALL be limited per peer and globally within independent loopback and network budgets, so network failures cannot block a local launch credential.

#### Scenario: Launch credential replay

- **WHEN** a launch link that was already exchanged, or has expired, is opened again
- **THEN** no session is created and the page explains how to obtain a new link.

#### Scenario: Launch credential target binding

- **WHEN** a launch credential is presented at a different host or port, or a local-only credential is submitted remotely
- **THEN** no session is created, the credential is not consumed, and it remains usable once at its original authorized origin until expiry.

#### Scenario: Foreign origin

- **WHEN** a page from another origin submits a state-changing console request using the owner's browser session
- **THEN** the request is rejected and no Operation is admitted.

#### Scenario: Other local services

- **WHEN** the owner's browser holds a console session and visits another web service on the same host but a different port
- **THEN** no console credential is sent to that service, and the console session remains unaffected.

#### Scenario: Token revocation

- **WHEN** the owner revokes an access token while a remote browser session created from it is open
- **THEN** the next request from that session is rejected and requires new authentication.

#### Scenario: Unauthenticated request

- **WHEN** a client calls a console operation without a valid session
- **THEN** the response contains no product data and an authentication-required result.

#### Scenario: Abandoned sessions at capacity

- **WHEN** the owner successfully signs in after 16 sessions have accumulated
- **THEN** the least recently used session ends, recently used sessions remain valid, and the new session works without restarting the daemon.

#### Scenario: Network login failures

- **WHEN** network peers exhaust the failed-exchange budget
- **THEN** further network exchanges are limited while a valid local launch credential remains usable; existing authenticated sessions remain usable.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219725852

### Requirement: Console exposure beyond loopback is explicit

On Linux, the console SHALL remain disabled on a fresh unattended installation unless the owner explicitly chooses a scope and port during installation or initialization. Enabling without a network address SHALL select loopback. Installation upgrades without console options SHALL preserve existing settings. Listening SHALL begin only at service startup using the persisted scope and port. `hiroute console open` SHALL NOT create a listener or start a service: it SHALL issue a single-use login URL from actual live listener state or return an actionable unavailable result. Binding to a non-loopback address SHALL require explicit persisted owner configuration and plaintext-network consent. Remote sessions SHALL require an owner CLI-issued single-use credential bound to that network origin or a named access token. A wildcard bind SHALL require a concrete advertised address for a usable login URL. The owner CLI SHALL allow explicit selection of the already-listening loopback endpoint for local recovery in network mode, without changing configuration or creating a socket. Changing desired settings while a service is running SHALL NOT change the default URL until that service applies the configuration. An occupied explicit port SHALL NOT silently fall back to another port. TLS SHALL NOT be required. When a session is served over a non-loopback connection without TLS, the console SHALL persistently indicate that traffic is unencrypted and SHALL show an additional warning before secret entry, while still allowing it. A configured address or port that cannot be bound SHALL produce an explicit listener failure without changing Gateway or Local Control availability.

#### Scenario: Default exposure

- **WHEN** the console is enabled without address configuration
- **THEN** it is reachable only through loopback, and connections to other interfaces are refused.

#### Scenario: Remote access

- **WHEN** the owner configures a network address and port during installation or initialization, starts the service and runs `console open`
- **THEN** it returns a one-use URL for that live network origin, works from another computer and shows the unencrypted-connection indicator; a request without a valid launch credential, token or session is rejected.

#### Scenario: Open does not enable listening

- **WHEN** the console is off or its configured port failed to bind
- **THEN** repeated `console open` calls fail with setup or restart guidance and do not open any listener or change configuration.

#### Scenario: Live configuration and upgrade

- **WHEN** desired scope or port changes during a running service, or the installer upgrades without console options
- **THEN** `console open` still uses the actual bound origin until restart, and an upgrade preserves the chosen configuration.

#### Scenario: Wildcard advertisement

- **WHEN** a network listener binds a wildcard address
- **THEN** initialization requires a concrete advertised address, the returned URL uses that address and the actual bound port, and Host/Origin checks admit only the configured console origin or its bound local origin.

#### Scenario: Secret entry over plaintext

- **WHEN** a remote plaintext session opens an API key or credential form
- **THEN** the form warns that the secret will cross the network unencrypted and accepts submission only after that warning is presented.

#### Scenario: Port unavailable

- **WHEN** the configured console port is occupied
- **THEN** the console reports a listener failure while the Gateway and CLI continue to work.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219727813

## Implementation references

- [Client Core](../../../crates/client-core/src) and
  [CLI request-input guide](../../../crates/cli/INPUT.md).
- [Real argv scenarios](../../../crates/cli/tests/request_input.rs) and
  [stateless Worker submission scenarios](../../../crates/cli/tests/worker_submission.rs).
- [Release builder/installation contract](../../../docs/release-builds.md).
