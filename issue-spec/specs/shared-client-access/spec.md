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

A role-all `hirouted` on Linux and Windows SHALL be able to serve the HiRoute product frontend and its control operations over HTTP on a console listener that is separate from the Gateway listener. Console pages SHALL use the same Application owners, typed operations, results and failure codes as CLI and Desktop, and SHALL present the same saved state. Gateway credentials SHALL NOT authenticate console requests and console sessions SHALL NOT authorize model requests. On Linux standalone the console SHALL be disabled until the owner opens or enables it; enabling it SHALL NOT start the daemon implicitly. Host-only features unavailable in the current host, such as in-app update, window behavior and login items, SHALL be absent or explicitly unavailable rather than failing or reporting fixture success.

#### Scenario: Open the console from the Linux CLI

- **WHEN** the standalone service is running and the owner runs the documented console open command
- **THEN** the command returns a single-use local link; opening it in a browser shows the product pages with the current service status, sources, plans, Agents, Workers and sessions.

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

Every console request other than static frontend assets and the credential exchange SHALL require an authenticated session. A local launch credential SHALL be issued only through the OS-trusted Local Control path, SHALL be single-use and short-lived, and SHALL be exchanged for a session credential bound to the console origin, including its port. The session credential SHALL be held only in browser sessionStorage and sent in an explicit Bearer request header. It SHALL NOT be placed in cookies, URLs, localStorage or other persistent browser storage, so cross-site requests and other services on the same host never receive console authority. The owner SHALL be able to create, list, expire and revoke named access tokens through the CLI; token secrets SHALL be shown only at creation and SHALL NOT be stored in recoverable form. Revoking a token SHALL end the sessions derived from it. State-changing requests SHALL be rejected when Host or Origin does not match the configured console or when cross-site request protection fails. Console operations SHALL carry a distinct principal identity that operation history and diagnostics can attribute; this increment supports a single owner principal.

An authenticated console session SHALL have the same protected-operation authority as macOS Desktop, without a password or unlock step. Every protected operation SHALL retain its in-page confirmation and exact-digest authority registration. Callers operating under collaboration or Skill authorization SHALL NOT mint console launch credentials or access tokens. Protected-operation uses SHALL be audited without credentials, secret input or conversation bodies.

Sessions SHALL remain in memory only, expire after 12 hours idle or seven days total, and be bounded to 16 active sessions. A valid new login at capacity SHALL retire and audit the least recently used session. Invalid credentials SHALL NOT evict a session. Failed exchanges SHALL be limited per peer and globally within independent loopback and network budgets, so network failures cannot block a local launch credential.

#### Scenario: Launch credential replay

- **WHEN** a launch link that was already exchanged, or has expired, is opened again
- **THEN** no session is created and the page explains how to obtain a new link.

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

The console listener SHALL bind only to loopback by default. Binding to a non-loopback address SHALL require an explicit owner configuration that persists across restarts, and remote sessions SHALL require an access token. TLS SHALL NOT be required. When a session is served over a non-loopback connection without TLS, the console SHALL persistently indicate that traffic is unencrypted and SHALL show an additional warning before secret entry, while still allowing it. A configured address or port that cannot be bound SHALL produce an explicit listener failure without changing Gateway or Local Control availability.

#### Scenario: Default exposure

- **WHEN** the console is enabled without address configuration
- **THEN** it is reachable only through loopback, and connections to other interfaces are refused.

#### Scenario: Remote access

- **WHEN** the owner configures a network address and opens it from another computer with a valid access token
- **THEN** the console works and shows the unencrypted-connection indicator; without a token the remote request is rejected.

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
- [Windows installation and manager boundaries](../../../docs/standalone-windows.md),
  [native platform owners](../../../crates/platform/README.md), and
  [installed background and upgrade scenarios](../../../crates/daemon/tests/windows_background.rs).
- [Real argv scenarios](../../../crates/cli/tests/request_input.rs) and
  [stateless Worker submission scenarios](../../../crates/cli/tests/worker_submission.rs).
- [Release builder/installation contract](../../../docs/release-builds.md).
