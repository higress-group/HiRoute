# Control-plane changes

## Purpose

All product clients submit typed intent through one recoverable Application
transaction. Preview, durable admission, external effects, and observed completion
have separate meanings.

## Requirements

### Requirement: One transaction path owns durable writes

Domain planners MUST interpret product intent and build a validated change. CLI,
Desktop, and protected host actions use the same Application admission, writer,
Operation journal, ownership, and recovery path. Clients cannot directly write
business databases, Secret storage, native Agent files, or Gateway publication.

Preview returns normalized intent, changes, relevant risk/external effects,
dependencies, and confirmation identity without creating an Operation or pending
executable object. Editable drafts are explicit domain data, not implicit Preview
effects. Apply binds local user scope, operation, confirmed intent, relevant versions,
and idempotency key. Accepted, running, succeeded, rolled back, repair-required,
conflict, denied, unsupported, and unavailable states MUST remain distinguishable.
No missing production adapter may return a fake success.

#### Scenario: Reuse a submission key

- **WHEN** the same local scope/action/key is retried with identical canonical intent
- **THEN** it MUST resolve the original Operation without duplicate effects;
  a different intent under that key conflicts instead of replacing it.

### Requirement: Changes validate the facts they consume

Planning/admission MUST capture and recheck exact resource revisions, source/Key
identity and generation, catalog/capability facts, related active publication, and
managed native-file fingerprints. Where a single atomic read is impossible, validate
consistency before additional effects or reject the stale intent. Serial writers
and conservative conflicts are allowed; automatic merges are not required.

Successful changes preserve unrelated plans, aliases, sources, and Agent grants.
Rollback acts only on effects the Operation still owns, never subsequent foreign
or user changes. Catalog/source changes cannot silently reorder frozen plans.
Restarts recover accepted operations through the same recorded steps/effect proofs;
read failure or an in-memory object is not proof of completion.

### Requirement: Local management is distinct from model-call authority

A client able to access the local control socket through the OS may manage local
configuration without a Desktop/CLI-specific Apply capability. Only validated
transaction admission writes. Queries/Preview do not. Extra owner/mode/peer-UID
rejection is not part of local access; [OS-access rules](../local-data-and-diagnostics/spec.md)
retain type, identity, revision, and cleanup protections.

Internal maintenance is allowed only for durably approved enabled connector-owned
sources in the trusted local context. It binds stable source ID, expected revision,
current source evidence, and idempotency to the same writer/Operation/validation
receipt. Source existence, enabled state, and evidence are rechecked before commit.
It cannot revive deleted/scan-only sources or bypass transaction authority.

Gateway grants remain separate from local management. Discovery and dispatch agree
on active publication, stable invocation names, protocol conditions, and the latest
successful pinned subscription directory/stable CPA binding. Token generation is
not their authorization join key; unobserved provider changes may still be rejected
upstream. Ordinary DTOs/Preview/Operations/diagnostics expose only safe allowlisted
references, not Key, Agent bearer, or native auth contents.

### Requirement: Publication success includes installed runtime state

Publication validates schema/digest, reference closure, immutable plan versions,
and stable aliases. Persistence, Gateway install, product active pointer, and
Operation completion MUST converge atomically or through durable recovery to the
actual result. New requests freeze the complete new catalog/grants/execution plan;
in-flight requests retain their admitted version.

Gateway reachable protocols derive from current Agent grants rather than creating
a second plan content version. Failure preserves/restores the previous executable
version and draft. If none exists, service is explicitly unavailable with no
upstream effects. Empty legal execution state stops corresponding new calls with
NoNewCalls; an unsafe transition fails and preserves old effective state. Saving a
database row alone cannot prove publication or revocation.

#### Scenario: Crash between install and active-pointer commit

- **WHEN** Gateway installation occurred but durable finalization was interrupted
- **THEN** restart MUST reconcile the same Operation to actual installed/current
  state or explicit repair; it cannot invent success or admit a partial publication.

### Requirement: Ordinary plan publication binds the full editing intent

Normal publish MUST bind submitted complete input, target plan, relevant versions,
and exact draft-consumption intent using the Application writer. The UI's explicit
publish action need not perform a second full Preview that the user did not see.
Independent Preview retains full validation and zero Operation/publication/inference
side effects. Other product actions keep their own required confirmation flow.

Admission rechecks sources, credentials/accounts, aliases, resource/draft versions,
and dynamic CPA/runtime evidence. An unchanged database revision or reused old
observation cannot prove external identity unchanged. Direct publication consumes
only the exact bound draft revision and may publish current edited input; newer
drafts survive. Failure/replay resumes the same Operation and installed recovery
contract, not merely a saved plan record. [Installed consumer budgets](../routing-plans/spec.md)
are checked at both admission and the install checkpoint, with safe conflict facts
retained through compensation and query.

### Requirement: Cached validation does not replace durable checkpoint truth

Within one valid publication execution context, immutable validated Operation
content MAY be reused instead of fully decoding it at every step. Committed steps,
generation, identity, authorization, and actual installation still require their
normal checks. In the normal path, successive stages do not re-decode the complete
Operation JSON. Failed journal writes cannot advance the in-memory committed view.
Cold loads, independent recovery, changed objects, or invalidated context perform
full durable-input validation rather than trusting matching Operation ID/generation.
Dynamic dependencies are always rechecked at their original boundaries.
Cold reconstruction MAY retain the authenticated immutable publication effect for
registry admission and its separately supplied Plan/control join, so those checks
do not repeatedly decode the same full publication. Changed persisted bytes still
enter complete authentication; this proof does not cover mutable journal state.

Completed provenance reads MAY reuse a bounded decoded Operation only after
comparing the complete current journal and all retained payload bytes in the same
SQLite snapshot. This memo contains decoded content, not a currentness verdict;
selected indexed receipts, current identity, native files and grants SHALL still
be checked at their existing boundaries. Changed or missing bytes and nonterminal
operations take the cold path. Caller transactions compare actual bytes on every
lookup, including after rollback; reopening does not retain the memo. No stored
format or migration is introduced for this optimization.

A publication marker's parsed immutable record MAY be shared while the exact
compensation object and all effect identity fields are unchanged. Mutable caller
copies, replacement records and changed effect identities SHALL be revalidated.
Every use still joins the marker to its owning Operation and current durable
checkpoint; a cached record never establishes current installation or permission.

Read-only successful Agent projections used by status, installed-model budget
checks and restored grant attribution MAY share verified immutable facts after
checking exact current row identity, raw journal and indexed steps in one storage
snapshot. Budget reads retain strict rejection; startup attribution counts and
skips unverifiable candidates without inventing an association or blocking startup.
Current grants, publication membership, pending operations and authorization are
not cached. This reuse does not authorize execution or replace recovery's strict
journal reader.

Successful read projections SHALL retain only the validated facts consumed by
status, grant joins and service publication references, not unused publication
rollback payloads. The entire
original journal, including discarded steps and payloads, remains part of their
validation closure. Memory accounting SHALL describe the retained projection.
Unrelated database commits invalidate unchanged-byte proofs but MAY preserve
decoded facts for reuse after exact closure comparison. A bounded cache SHALL
admit a newly requested current fact by replacing an older entry when necessary;
bulk history scans SHALL NOT continually evict the current working set.

Within the same live storage connection, already verified immutable facts MAY be
reused without hashing the entire raw closure again when an owned read transaction
proves that the database snapshot has not changed. The proof SHALL account for
external commits, same-connection writes and schema changes. Unknown or invalidated
proof falls back to exact row/step/payload validation. It is process-local and SHALL
NOT survive connection replacement or reopening. Caller-owned transactions SHALL
use the exact-byte path and SHALL NOT publish a shortcut proof: rollback, savepoint
rollback or an interrupted read cannot make their temporary facts authoritative
on a subsequent read. The proof and facts belong to the same established read
snapshot, never a newer token read after that snapshot ends.

Current Agent status and settings preparation MAY stop at the newest authenticated
fact that decides the requested facet or exact current grant. Older facts after that
match are not dependencies of that answer. Every visited row, its indexed identity
and raw dependency closure still require snapshot-bound validation or the unchanged
snapshot proof above; a corrupt visited
row is not a no-match. Explicit history/restore-list reads retain their full strict
scope. Ordering a history query SHALL NOT require sorting complete retained payloads
before the first selected fact can be read.
Single-current-fact reads SHALL release unmatched facts as they traverse history,
rather than retaining a full result list until the lookup ends.

#### Scenario: Background commits and growing history do not reconstruct unchanged facts

- **WHEN** an unrelated external commit occurs or a newer current operation is added to a full in-memory cache
- **THEN** the next read revalidates the exact relevant closure before reusing immutable facts, can retain the newer current fact within the existing bound, and does not retain every unmatched history object; damage to a discarded journal step still fails strict status reads.

#### Scenario: Current fact without historical payload work

- **WHEN** a current Agent facet has a verified latest matching operation
- **THEN** its status/settings preparation does not reconstruct older irrelevant
  operations; changed bytes or indexed identity of the selected operation are rejected
  on the next read, including after a warm read, while an explicit history query still
  checks every requested member.

#### Scenario: Multiple current reads of an unchanged history

- **WHEN** multiple Agent facets read the same verified successful operations with
  no intervening database change
- **THEN** they share immutable validation without rereading and hashing the full
  journal closure for each facet; a local write, external commit or schema change
  invalidates the shortcut, and a caller transaction's temporary result is rejected
  after rollback if it does not match the committed bytes.

#### Scenario: Status observation while a writer owns an operation

- **WHEN** the durable writer claim is present
- **THEN** service status reports mutation/recovery unavailability without expanding
  the in-progress plan solely to report busy; this does not authorize execution or
  recovery, which retain complete current-operation checks. After release, any parked
  native-file tail still requires its authenticated service-completion receipt.

#### Scenario: Select multiple routes from one publication

- **WHEN** one Agent settings selection includes multiple published Plans
- **THEN** the immutable publication catalog is authenticated/projected once for the
  selection, while each route retains its own active-state, ingress, revision and
  semantic-digest binding; adding a selection does not repeat whole-publication work
  for every selected route.

#### Scenario: Publication checks other pending operations

- **WHEN** activation checks for conflicting recoverable operations while holding
  its committed current operation
- **THEN** one SQLite snapshot verifies recovery membership coverage and the exact
  current journal, including its indexed steps and immutable inputs, before excluding
  that operation from decoding. All other pending operations remain strictly decoded
  and checked by every applicable conflict guard. Stale current facts, damaged
  coverage or a damaged pending operation fail the check; independent recovery still
  decodes the complete unresolved set.

### Requirement: Durable business identity survives implementation changes

Persisted contracts MUST store frozen business facts and their contract identity,
including original confirmation, idempotency, steps, and effect ownership. Compiler,
serializer, decoder, memory layout, and rebuildable derived checks cannot alone
change plan/grant/task/Operation identity. Step proofs, completion receipts, and
journal CAS bind stable input. One current core model receives strictly validated
supported legacy data at its conversion boundary. Reader removal requires explicit
support retirement with a viable upgrade/recovery path and registry/test updates.

### Requirement: Console mutations keep confirmation, secret and conflict semantics

Console mutations SHALL use the existing Preview/admission, idempotency, revision and recovery contracts. A destructive or authority-changing action that requires confirmation SHALL use a daemon-issued single-use challenge bound to the session, the exact previewed intent and an expiry; expired, reused or mismatched challenges SHALL be rejected without effect. Secrets submitted through the console SHALL enter the existing protected input path, SHALL NOT be echoed in responses, logs, diagnostics or browser storage, and SHALL be released when the request ends. Several console sessions, the CLI and a host shell MAY manage the same instance concurrently; a stale revision SHALL produce a conflict that identifies the changed object instead of overwriting it.

Every admitted product command, including versioned names containing digits, SHALL
retain its normal audit and dispatch behavior. Audit-field validation SHALL NOT
turn a valid management session into an authentication failure. Command catalog
admission, session expiry and revocation remain independent enforced checks.

#### Scenario: Versioned API-save command retains its authenticated session

- **WHEN** an authenticated owner checks an API connection and saves its selected models through an admitted versioned product command
- **THEN** the console audits and dispatches the normal Preview/confirmation/Apply flow and preserves the valid session for saved-state readback.
- **AND** malformed command arguments report their request error; absent or expired credentials still require authentication, and unregistered command versions remain unavailable.

#### Scenario: Confirmation replay

- **WHEN** a confirmed deletion challenge is submitted a second time or after the target changed
- **THEN** the second submission is rejected and the current state is unchanged.

#### Scenario: Secret custody

- **WHEN** the owner saves an API connection key through the console
- **THEN** the connection becomes usable, and the key does not appear in any console response, daemon log, diagnostic bundle or browser storage.

#### Scenario: Concurrent edit

- **WHEN** two browser tabs edit the same plan and the second saves after the first
- **THEN** the second save reports a revision conflict and the first saved content remains.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/132#issuecomment-6219729524

## Implementation references

- [Application operation workflows](../../../crates/application/src/operations) and
  [durable journal](../../../crates/domain/src/operation/journal.rs).
- [Publication checkpoint](../../../crates/domain/src/operation/publication_checkpoint.rs)
  and [transaction guards](../../../crates/local-storage/src/control/transaction_v2_tests.rs).
- [Production publication scenarios](../../../crates/daemon/tests/publication_process.rs).
