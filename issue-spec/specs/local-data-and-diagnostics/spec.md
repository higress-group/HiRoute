# Local data, retention, and recovery

## Purpose

HiRoute owns local business stores and their supported upgrades. Logical visibility,
physical reclamation, recoverable backups, and native Agent-owned history have
different lifetimes.

## Requirements

### Requirement: Startup preserves damaged or unsupported data

Startup MUST distinguish stale socket, unready service, and unreadable local stores.
Ordinary current-format startup MUST strictly validate the complete dependency
closure of current business state and unresolved recovery work before admitting
business traffic. This includes current, prepared and last-known-good publications,
current Plan heads and their versions, exact active Secret grant proofs, versions
held by live Workers or continuations, and operations whose required completion
has not settled. Parked native-file tails, NeedsAttention operations and operations
marked Succeeded with required finalization outstanding remain unresolved work;
membership does not authorize automatic retries of parked user-file changes.

Damaged, unreadable, unsupported or incomplete state needed to establish that
closure stops affected business startup and retains original directories. The
membership and completeness of current dependencies and unresolved work MUST be
established from the authoritative storage and recovery boundaries; an empty
index, missing completion marker or absent writer claim alone is not proof that
there is nothing to recover. All production writes and supported conversions MUST
preserve this invariant atomically with the state they describe. Existing content,
ownership, generation, authorization, revocation and epoch checks remain required.
Optional Observation attribution remains distinct from admission authority:
unavailable attribution stays explicitly unassociated under its existing bounded
diagnostics policy and cannot fabricate a proof or become a new startup blocker.

Damage confined to historical records outside this dependency closure MUST NOT
block unrelated current work. Access to those records or a full audit MUST verify
the relevant stored content and references and report actual corruption or
unavailability, never substitute empty history or successful verification. Ordinary
startup admission MUST NOT depend on scanning, decoding or replaying all unrelated
completed history, nor certify the integrity of unchecked historical data.
Detected database structural errors or an unprovable current dependency closure
remain failures; no claim is made that a bounded startup detects every damaged
page or historical record. Full-store audits and the complete initial conversion
of supported older representations SHALL be identified separately from ordinary
startup, with their actual scope, progress and result. Incomplete conversion,
missing required recovery members or incompatible writers cannot enable a fast
path or initialize an empty current state.

Startup cannot silently reset, overwrite with an empty database, or claim migration
did not happen merely because the service later failed.
Only a genuinely fresh root may initialize; an incomplete existing three-store set,
unknown files indicating prior state, or recovery backup prevents fresh initialization.
The real bootstrap diagnostics directory may already exist without being discarded.

Desktop MUST expose actual local recovery/backup directories and actionable
instructions even when daemon is unhealthy. Opening them changes no data. A verified
backup requires whole-group restoration with its corresponding complete old binary,
not deletion of storage or a single database replacement. Archive/restart recovery
retains actual backup ownership and package identity. A changing filesystem device
number after reboot cannot alone invalidate logically intact supported store identity.

#### Scenario: Startup ownership follows the actual store lifetime

- **WHEN** current stores are transferred to independent owners and an unrelated Unix child or duplicate descriptor retains a copy of their startup-lock handle
- **THEN** each retained Control, Runtime or Secrets store continues to exclude another writer; releasing the last actual store relinquishes startup ownership without waiting for the unrelated descriptor to close or exec, preserving current storage and ordinary lock-conflict diagnostics.

#### Scenario: Independent historical damage does not disable current routing

- **WHEN** current dependencies and unresolved recovery work are verified, but an unrelated retained historical publication is damaged
- **THEN** normal startup and unrelated current routing remain available; a read or audit of that publication reports its actual failure and preserves the original data, without describing unchecked history as healthy.

#### Scenario: A historical version is still a current dependency

- **WHEN** a current or prepared publication, last-known-good state, active Secret proof, live hold or unresolved operation depends on an older record
- **THEN** that record remains inside the required validation closure, and its missing or invalid content blocks the affected admission or recovery; age alone cannot exempt it.

#### Scenario: Explicit native targets do not require historical replay

- **WHEN** an Agent target has several retained settings operations
- **THEN** ordinary discovery and startup validate the current explicit target owner and complete target membership, including pending work, without decoding superseded unrelated settings history. Supported conversion establishes those references once without changing the old journal bytes. Missing current owners or coverage remain failures; target hints do not grant execution authority.

#### Scenario: Native restore history is distinct from required recovery material

- **WHEN** completed Agent configuration operations leave superseded restore markers or backups
- **THEN** ordinary startup authenticates the current workspace/target owners and unresolved operations' exact direct restore inputs, including the current model's catalog, without visiting optional historical Restore menu entries or recursively retaining settled reconfiguration ancestors. Access to an old restore point still authenticates its original journal, marker and protected backup; unrelated damage is not reported as verified healthy.
- **AND** settled file ownership verifies the original immutable input, exact current identity and file receipt without requiring optional attribution proof from unrelated completed steps. Admission, owner replacement and direct restore reads use this same boundary; the entire unresolved operation remains strictly verified.
- **AND** admission captures the exact previous owner, and a successful receipt updates or retires the owner in the same transaction as the terminal journal and writer release. Selecting an old restore point cannot bypass this comparison; rollback keeps the prior owner. Missing current owners, membership, receipts or required backups remain failures. Supported initial conversion establishes this projection atomically from authenticated old journals without rewriting them.

#### Scenario: A settled idempotent retry cannot repeat old cleanup

- **WHEN** a completed removal is retried with its original idempotency identity after a new installation at the same target
- **THEN** the exact authenticated terminal Operation returns its original result without replaying settled cleanup against the new installation. Only an authoritative durable pending-finalization obligation can request its callback again; missing or inconsistent membership remains an error. A crash after local terminal commit but before required finalization retains that obligation for recovery.

#### Scenario: Current model checks do not scan replaced declarations

- **WHEN** publication admission checks installed model budgets or an active Agent access view checks the current configuration
- **THEN** those reads use authenticated current managed-configuration owners, retaining immutable admission, exact file and grant-step receipts, current Secret and budget checks, without decoding superseded unrelated settings journals. Missing optional completed service provenance does not block these reads or confer session attribution. A malformed current owner, required file or grant proof remains a failure. Explicit historical Restore reads retain their original strict validation.

#### Scenario: An empty recovery lookup cannot prove completion

- **WHEN** recovery membership is missing, inconsistent or incomplete, including an operation marked Succeeded whose required finalization is outstanding
- **THEN** startup refuses affected business admission and preserves recovery material rather than treating an empty lookup as no work; parked native-file tails retain their explicit retry policy.

#### Scenario: First conversion and full audit are distinct from ordinary startup

- **WHEN** supported older data needs a complete initial conversion or a full-store integrity audit is performed
- **THEN** the existing coordinated recovery boundary preserves the source and reports the actual work separately; interruption cannot publish partial current-state coverage or turn unchecked historical data into a successful integrity result.

#### Scenario: Completed upgrade backup is retained without reopening history

- **WHEN** the durable completion record names the current target format and agrees with the current three-store identities and secret binding
- **THEN** ordinary reopen validates current dependencies and unfinished work without reading the retired backup contents or auditing unrelated history. Damage in that retired backup remains a failure when it is explicitly opened for recovery; it is never reported as verified healthy.
- **AND** incomplete batches, older targets, malformed completion records, missing current stores and mismatched identities retain strict recovery admission. A current format number or an existing backup path alone cannot prove completion.

#### Scenario: Upgrade status follows entered work

- **WHEN** startup enters backup publication, data conversion, final current-data validation and service recovery
- **THEN** it reports the corresponding bounded phases on the existing readiness channel, without rescanning history for progress. Completing an older batch before the current batch does not repeat or regress visible phases. A failed stage remains the last entered phase; no later stage or ready result is fabricated. Ordinary current-format restart emits no upgrade phases and retains its normal readiness deadline.

### Requirement: Journal progress does not rewrite immutable payloads

Current Operations SHALL store their immutable admitted input and large compensation
payloads separately from mutable journal progress. The storage representation MUST
preserve the domain input, accepted digest, step order, ownership and recovery
semantics. A reference is not execution authority: cold reads authenticate payload
bytes and reconstruct the typed input; warm updates compare their exact committed
input, generation, identity and both journal copies in the same transaction.
Missing, substituted or changed required bytes SHALL fail validation. Supported
retained inline journals keep their original bytes and existing recovery checks;
ordinary startup does not rewrite completed history into the new representation.

The successful local Plan head/version/draft change and terminal journal SHALL
commit in one Control transaction under existing writer and admission guards.
Required external completion remains explicitly pending until its real callback
succeeds. Restart SHALL recover that completion without inferring it from a terminal
journal label, and SHALL preserve the explicit retry policy for parked native tails.

#### Scenario: Large immutable payload survives repeated progress updates

- **WHEN** an admitted Operation with a large immutable input or compensation payload updates attempts or state
- **THEN** only the compact progress changes, the payload bytes remain unchanged, and a later cold read reconstructs the same typed Operation.

#### Scenario: Required payload is damaged after an earlier successful read

- **WHEN** required retained payload bytes are removed or altered while their reference remains
- **THEN** both currentness validation and cold recovery reject the damaged value; an earlier in-memory value cannot authorize the update.

#### Scenario: Final local transaction or external completion is interrupted

- **WHEN** a Plan operation stops before its local terminal transaction commits or before required external finalization completes
- **THEN** head/version/draft and journal do not expose a partial local commit, and required external work stays in the durable recovery set until confirmed complete.

### Requirement: Supported upgrades use one three-store coordinator

0.2.0 is the first supported release baseline; historical 0.1.0/schema22 records are
retained evidence, not supported conversion inputs. A later latest-stable release
does not silently retire supported earlier sources. Current startup accepts only
the declared schema23–29 set with stable format marker `(1, 1)` and advances through
the current schema29 coordinator. Future/unmarked/unsupported formats fail before
business/key/backup mutation. Acceptance of a format alone does not expand the
public release-support promise.

Control, runtime, and secrets migrate as one verified BackupSet with source digests,
store/key identity, and recorded phase. An inconsistent mixed-version live set is
not permission to fix one database independently. Without a prior BackupSet only
a complete consistent supported source can start a new batch. An interrupted batch
finishes its original target and archives before a separate newer-target batch;
its source or target cannot be rewritten to latest. A next-store migration commit
may be resumed only when identity and migration ledger match the recorded phase.
A later store advancing out of order or key/source mismatch fails closed.

Registered schema24 dependency-selection and schema25 Agent-check conversions
preserve original JSON, identities, steps, effects, check revisions/times, and
replay meaning; they do not re-run inference or fabricate validation. Further
supported schema changes retain the same coordinator and frozen-source regressions.
Completed conversion never automatically restores old data. Manual recovery pairs
a complete old backup with a binary verified to read that source. Supported data,
configuration, and public CLI/Gateway semantics are ongoing obligations, not waived
by internal refactoring or development-stage terminology.

### Requirement: Local retention does not imply automatic external upload

Conversation bodies, per-request detail, and route-stage quality samples default
to seven-day local retention and share authorized expiry/deletion boundaries.
Longer-lived aggregate value contains neither body nor competence scores. Retention
itself makes no external upload. A plan explicitly selecting built-in System One
or a custom decision extension may send current input and bounded safe visible
history to its configured endpoint according to [decision policy](../decision-services/spec.md).
Heuristics make no decision-service request. This is independent of the user's
selected business-model calls.

### Requirement: Session cleanup has an explicit bounded scope

Users MUST choose between clearing body and deleting the session record.
Body-only removes questions/answers/tool content while retaining permitted route
facts and aggregate value. Session-record deletion also removes session detail,
while non-content accumulated value remains by default. Confirmation names the
scope; cancellation has no effect. Partial failure reports the actual visibility/
cleanup state, never full success.

Visibility applies immediately across session views, task results, search, old
cursors, and content references. Replay, background projection, cached text, or
native history cannot resurrect hidden data or Continue access. Physical reclaim
may finish later and has independent status. The confirmed identity/range is rechecked;
retry cannot expand into later requests, other sessions, or model/plan/Agent settings.

#### Scenario: Another request arrives during cleanup

- **WHEN** a confirmed cleanup is applied to the previously selected range
- **THEN** new unconfirmed data MUST not be deleted, and old content stays hidden
  even if physical reclamation or late ingestion is still pending.

### Requirement: Accessible local objects are governed by OS operations

On Unix, HiRoute MUST NOT reject existing local files/directories/sockets solely for owner
UID, group/other access bits, or not being 0600/0700. This includes native auth and
Agent configuration, product data/recovery, diagnostics/private samples, CPA-owned
login directories, and local control/protected-input transport. Actual open/read/
write/connect errors remain explicit. Existing objects are not chmod-repaired to
pass admission; newly created objects retain their private defaults.

File kind, link policy, read/format/size bounds, content/inode stability, revisions,
managed child identity, ownership-limited cleanup, and locks still protect operations.
Metadata drift may require re-read, but permission differences alone cannot impose
permanent rejection. Explicit write modes/user-requested permission changes remain
output intent. Gateway/upstream auth, protected capabilities, OAuth state/session/
provider binding, and WebView ACL are independent and remain enforced.

### Requirement: Windows private state and configuration recovery retain native identity

New HiRoute-owned private objects MUST receive their user owner and private ACL at
creation, including child-created secret files and database sidecars. Parent shell
token defaults and unrelated existing user objects remain unchanged. Protected
runtime files and pipe endpoints retain their explicit SID/DACL admission; borrowed
native configuration follows its own access contract without automatic ACL repair.

Identity-sensitive operations MUST hold the actual file/directory objects and reject
reparse or replacement drift before privileged reads, publication or cleanup. A path
string, metadata snapshot or old PID is not authority over a replacement object.
Configuration fingerprints and authenticated recovery records include the relevant
Windows owner, DACL and read-only facts. Restore is conditional on the expected
current object and restores the captured bytes and native permissions together.

Windows-native fingerprint and recovery extensions cannot change supported Unix
fingerprint bytes or retire registered historical readers. Supported store conversion
continues through the existing three-store coordinator with platform-derived source
fixtures; parser acceptance or a development-package upgrade is not proof of a
released Windows-to-Windows upgrade.

### Requirement: Share repeated observation storage without changing logical evidence

HiRoute SHALL store repeated retained content and common observation metadata through shared representations rather than independent complete copies for every occurrence. Equal content may share bytes only within the existing digest and authorization boundaries. Each request, message, content/event identity, original order, canonical digest, delivered-frame association, exact retained ACK/NACK meaning and actual model/usage attribution SHALL remain independently recoverable. Repeated input history SHALL retain distinct request references without inventing extra execution or usage. Physical sharing SHALL preserve the existing session-history, request-routing-history and usage-and-savings contracts, including authorized original-byte content access, tool identity/pairing, explicit gaps and provider-state visibility. Sharing SHALL NOT turn a partial observation into complete evidence or merge distinct conversations, attempts or workspaces.

#### Scenario: Repeated history shares storage

- **WHEN** many requests contain identical tool definitions, messages or tool results
- **THEN** the shared values are stored once per applicable storage identity with compact per-occurrence references, while each request retains its original order, identities, attribution and inspectable content.

#### Scenario: Stream events preserve their logical boundaries

- **WHEN** a delivered response contains small text/reasoning deltas and tool events
- **THEN** their physical representation may be shared or compacted but the original logical events, kinds, order, accepted-delivery associations and composed answer remain unchanged.

#### Scenario: Replay a retained event after compaction

- **WHEN** an accepted retained event is replayed after representation changes and restart
- **THEN** the receiver returns the same committed acknowledgement and checks its original digest/sequence identity; conflicting replays remain rejected and do not add usage or occurrences.

#### Scenario: Do not merge equal content across authority

- **WHEN** equal bytes appear in distinct logical conversations or authorization scopes
- **THEN** physical reuse does not merge identities, grant access, change visibility, or assign one occurrence the other occurrence's execution facts.

#### Scenario: Resolve an incoming sequence without rescanning expired gap ranges

- **WHEN** a fact or content event arrives while retained gaps grow across producer epochs, streams and earlier sequences
- **THEN** gap lookup seeks within that exact channel/producer/epoch/stream past ranges ending before the event, without scanning unrelated or already-ended gaps for each event; all overlapping gaps are resolved with their original scope, known-loss status and reason, preserving acknowledgement, replay and completeness semantics.
- **AND** reopening an existing current-format store may install a derived lookup index without rewriting historical evidence or changing its format version; repeated reopen preserves the index and historical receipts.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/151#issuecomment-6248901316

### Requirement: Compressed observations remain losslessly and boundedly accessible

HiRoute SHALL support lossless compressed local observation storage in independently readable bounded units. Original object bytes, content identifiers, canonical digests and original-byte search anchors SHALL remain stable independently of physical placement. An authorized bounded content read SHALL decode only the units necessary for its selected range rather than decompressing the entire history. Existing ingestion durability, nonblocking model delivery, reader admission, query deadlines and explicit incomplete/error behavior SHALL continue to apply. A damaged, truncated, unsupported or over-budget encoded unit SHALL produce an explicit affected-content/storage failure without fabricated empty content, successful integrity verification or loss of otherwise independent readable records. Compression SHALL preserve existing logical size limits and visibility rules; encoded length cannot authorize excessive decompression.

#### Scenario: Read an original range from compact data

- **WHEN** a user opens an existing text/tool-content reference or Unicode search anchor after its physical representation changed
- **THEN** the existing content interface returns the identical original range, digest, byte count and offsets under current authorization without scanning unrelated history.

#### Scenario: Read across a block boundary

- **WHEN** one logical object spans multiple compressed blocks
- **THEN** bounded continuation composes the exact original bytes in order, with no truncation, duplicated text or rewritten public identity.

#### Scenario: Interrupt compact publication

- **WHEN** the process stops between persisting compact bytes and committing their authoritative reference
- **THEN** acknowledged data in the previous committed representation remains recoverable, unpublished bytes never become successful evidence, and restart resumes or discards only owned incomplete work.

#### Scenario: Reject damaged or excessive encoded data

- **WHEN** a selected compressed block fails length, codec or integrity validation
- **THEN** the read reports its actual unavailable/corrupt state within existing resource bounds while unrelated valid objects remain readable; no oversized allocation follows an untrusted size header.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/151#issuecomment-6248901714

### Requirement: Reclaim obsolete observation representations through one lifecycle

HiRoute SHALL apply the existing confirmed deletion and seven-day detail-retention scope to every obsolete representation introduced or retained by observation storage, including content objects, event metadata, feedback/deduplication payloads, dead physical ranges where introduced, and rebuildable indices. Compact tombstones, sequence/checkpoint and conflict evidence needed to prevent replay resurrection SHALL remain authoritative. Reclamation SHALL release eligible physical storage through bounded resumable maintenance without waiting for an otherwise live stream or shared physical container to become wholly obsolete. It SHALL preserve every live reference and lawful longer-lived non-content monetary aggregate, distinguish content-only from facts-and-content deletion, and retain replay evidence until a provable retirement boundary permits its removal. Ineligibility or failed reclamation SHALL NOT be reported as completed physical release. Age alone SHALL NOT make active or unproven protocol evidence disposable, and compaction/replay SHALL NOT restore deleted bodies, membership links or precise expired detail.

#### Scenario: Delete one object in shared physical storage

- **WHEN** a confirmed deletion removes one request while other requests reference shared content or the same physical storage
- **THEN** deleted content becomes immediately inaccessible and maintenance eventually reclaims its eligible obsolete representation while retaining exact live bytes and current references.

#### Scenario: Expire old requests in an active stream

- **WHEN** some requests reach detail retention while later requests under the same producer stream remain retained
- **THEN** eligible old content/event/feedback/deduplication storage is reclaimed without waiting for the active stream to end, preserving required retirement evidence and later requests.

#### Scenario: Replay after retirement

- **WHEN** an old event or cached reference arrives after its body or facts were deleted or expired
- **THEN** existing barriers and retained retirement proof prevent resurrection or duplicate charging; missing retired content cannot receive a fabricated successful content acknowledgement.

#### Scenario: Content-only deletion retains monetary meaning

- **WHEN** the user clears body while retaining permitted request facts
- **THEN** body-bearing copies and references are removed from all relevant representations, while retained route facts, grouping and monetary evidence keep their existing semantics.

#### Scenario: Resume interrupted reclamation

- **WHEN** compact-storage publication, shared-object collection or physical page reclamation is interrupted or fails
- **THEN** logical visibility remains enforced and bounded maintenance resumes from durable state without deleting a live reference or falsely reporting released space.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/151#issuecomment-6248902164

### Requirement: Seal old observation history and start a fresh generation recoverably

HiRoute SHALL preserve supported old Observation history unchanged in a protected local archive and start a fresh current-format Observation store during upgrade. Startup SHALL retain the complete original storage group, keys, auxiliary state and matching old program before the cutover, preserve current configuration, credentials and routing, and avoid work proportional to old observation rows, body bytes or file count: no full historical copy, hashing, recompression, normalization or index rebuilding. The original Observation DB, WAL/SHM, objects and staging SHALL stay together through a durable same-filesystem namespace transition. Current history and statistics begin at cutover; old observations are available by whole-group matching-version recovery rather than silently combined into current queries. Current-format data and policy SHALL survive ordinary restart. Interrupted cutover SHALL resume the same original batch or expose actionable preserved-source recovery; missing recovery material, insufficient work space, unsupported/corrupt metadata or failed moves SHALL preserve the source and stop the affected startup. An old local archive is not an independent backup or freed disk space. Active capacity retention SHALL NOT reclaim it. Manual recovery SHALL preserve newer data separately and restore the complete corresponding old group with its matching old program.

#### Scenario: Upgrade a large old history

- **WHEN** a supported instance with retained old-format observations upgrades
- **THEN** the complete old history remains unchanged and recoverable, current configuration/credentials/routing continue, and service starts with a fresh current-format history without processing all historical rows or files; current history/statistics start at cutover.

#### Scenario: Interrupt the directory cutover

- **WHEN** the process stops before or after sealing the old directory or publishing its recovery material
- **THEN** restart uses the same durable batch and complete source, never mistakes a missing/partial live root for permission to discard history, and finishes the cutover or supplies actionable recovery guidance.

#### Scenario: Fail archive admission or publication

- **WHEN** recovery material or keys are unavailable, small-work space is insufficient, an input/phase is unsupported or corrupt, or the same-filesystem move fails
- **THEN** startup reports the failure, preserves existing source and progress, and does not fall back to a full historical copy or delete records/backups to proceed.

#### Scenario: Reopen a current-format store

- **WHEN** the upgraded instance restarts after recording new observations
- **THEN** it preserves those records and its current capacity policy, without resealing or clearing the current generation or scanning old archives.

#### Scenario: Recover old observations

- **WHEN** the owner chooses matching-version recovery
- **THEN** the complete original group, original locations and matching old program are restored together while newer data remains separately preserved; the original observations retain their bytes, identities and monetary facts.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/151#issuecomment-6248902605

### Requirement: Upgrade recovery actions come from whole-group storage admission

Windows has no supported released predecessor in its first release. Generic complete-group recovery and native-state preservation remain required where applicable; an unpublished development build or simulated upgrade SHALL NOT establish historical release support or require an exhaustive historical acceptance matrix.

HiRoute SHALL provide a bounded typed startup cause and storage-owned recovery action on macOS, Linux and Windows, including while the daemon or console is unavailable. Missing/invalid recovery packages, insufficient space, missing keys, access failures, unsupported/corrupt input and execution/unknown states SHALL retain their safely established meaning. Unknown values SHALL preserve material and SHALL NOT authorize downgrade. Stable legacy startup failure envelopes remain supported; a private bounded recovery report may carry additional safe facts without adding arbitrary error strings or filesystem authority.

Current recovery actions SHALL be prerequisite correction and current-version retry, verified current-batch resume, verified complete-group restoration, or preservation and diagnosis. Program-only original-version reinstallation SHALL NOT be offered: no current production entry establishes the required whole-group compatibility proof. Missing packages, equal SQL markers, an unmoved Observation directory and absence of a backup SHALL NOT authorize downgrade. Diagnostics and transient lock files are not business data.

Installers SHALL retain the complete matching previous program and resources before an admitted incompatible replacement. Ordinary current-format installation/repair SHALL NOT require unnecessary complete-program archival or Task/Apps export. Before allocating a standalone recovery archive for a changed candidate, preflight SHALL determine whether a history cutover is required. Repair with the exact same-version installed daemon image SHALL preserve existing recovery handoffs without requiring a newer preflight protocol from the old package; it SHALL NOT grant storage compatibility, downgrade authority or skip normal startup admission. Failed preparation SHALL discard only its own unpublished archive; a handoff that may have been published or consumed SHALL retain its recovery material even if later installation rollback succeeds. Native standalone startup SHALL obtain verified recovery input through an installation-owned storage-root and target-image-bound handoff rather than assume inheritance of the invoking shell's environment. The explicit absolute `HIROUTE_RECOVERY_PACKAGE` operator entry remains supported. Cleanup SHALL retain material still required by installation or storage recovery. Windows shell rollback SHALL refuse automatic original-program launch when storage has changed or cannot be established as compatible.

The verified candidate SHALL check safely observable recovery/space prerequisites before installed-program replacement, and startup SHALL repeat authoritative admission under its writer barrier. Provisional checks SHALL NOT grant later write authority. Existing active-work waiting, cancellation, ownership and finite startup limits remain. A source release's actual shipped updater and any required transition sequence SHALL be validated; installing a new updater does not retroactively add checks to an old executable.

Recovery presentation SHALL distinguish correcting prerequisites, current-batch resume, verified whole-group restoration and unavailable/unknown material. It SHALL identify the correct original batch rather than select a directory by presence, provide Chinese/English platform instructions, preserve newer data and its matching program separately before restoration, and require actual service readiness and a request before claiming successful restoration. Missing essential material SHALL lead to preservation and diagnostic/support guidance rather than a promise that retries, manual overwrite or an arbitrary older release will recover it.

#### Scenario: Missing package does not authorize program-only downgrade

- **WHEN** startup lacks a matching complete previous package
- **THEN** consumers preserve all data, keys, programs and journals and provide diagnostic/support steps; they do not offer program-only downgrade without a production whole-group compatibility proof.

#### Scenario: Windows stopped-service startup returns a typed failure

- **WHEN** the shell starts a stopped service and the CLI returns failure after waiting for readiness
- **THEN** a newly observed daemon startup failure supplies the bounded cause and admitted recovery action; unchanged, incomplete or unreadable previous diagnostics cannot replace the current manager failure. Non-storage failures do not borrow old storage advice.

#### Scenario: Current-format repair requires no historical archive

- **WHEN** a verified standalone candidate reports that no history cutover is required
- **THEN** repeated installation or repair creates no complete old-package copies, imposes no archival space reserve and leaves existing recovery handoffs, backups and business data unchanged.

#### Scenario: Changed or uncertain group

- **WHEN** core migration/cutover has begun, an original authority changes, a phase is corrupt/unknown or a prior batch prevents proving compatibility
- **THEN** no consumer offers unverified program-only downgrade and no installer automatically starts an older daemon; all generations and journals remain available for current-version recovery or the corresponding complete-group guide.

#### Scenario: Prepared native installation after possible storage writes

- **WHEN** Windows installation recovery observes storage cutover facts while a native install journal is still prepared
- **THEN** generic rollback refuses before manager or file effects. Only an independently matched complete accepted current package and its owned native definition may reapply that journal forward; changed native entries or mismatched packages remain preserved and refused. The existing original source is not resealed or replaced, and a successful current repair does not imply that an older program can read the current group.

#### Scenario: A retained source does not authorize blind resume

- **WHEN** an original complete batch is verified but the current generation is damaged or its phase cannot be admitted
- **THEN** its existence authorizes only the matching complete-group restoration target; it does not grant current-version resume. Resume after fresh Observation initialization failure requires successfully admitted core stores and an unpublished live Observation root, with the same verified original batch still available.

#### Scenario: Managed standalone upgrade

- **WHEN** Linux systemd or a Windows ordinary-user Task starts the installed candidate after an admitted format-changing upgrade
- **THEN** the matching complete old package is retained and its protected handoff is verified against the storage root and candidate image; package cleanup cannot delete it prematurely and an unrelated terminal variable is not required for ordinary installer operation.

#### Scenario: Complete Windows Desktop recovery material

- **WHEN** verified candidate preflight requires a format-changing cutover over an existing Desktop
- **THEN** preflight precedes installed-file replacement and retains both program namespaces, shell/uninstaller/inventory, the installation owner's complete native file map, verified native Task XML/definition/login choices and fixed current-user Apps registration. Space budgets combine allocations sharing a volume and keep separate volumes independent. Whole-group recovery restores those matching materials under the same ordinary account, with a batch-specific bilingual guide, import of the exact original Task and actual readiness/request verification. Instructions use published lifecycle commands and Task Scheduler rather than unpublished CLI installation verbs. A same-payload repair preserves the accepted original handoff instead of replacing it with the current program.

#### Scenario: Prerequisite refusal and cancellation

- **WHEN** preflight finds unavailable/unverified material or insufficient recovery space, or the user cancels while waiting for work
- **THEN** replacement is not committed, the existing installation remains usable when verified, and the user receives the specific correction or cancellation state; startup rechecks races before any incompatible data mutation.

#### Scenario: Failed startup without a console

- **WHEN** the daemon cannot start or an upgrade batch is interrupted
- **THEN** Desktop, the Windows bundled failure surface and the installed CLI expose the bounded cause and admitted recovery action without depending on the normal server; known resumable batches use the current program, while missing/substituted material never appears as a valid restore target.

#### Scenario: Locate the exact admitted recovery material offline

- **WHEN** a failure admits current-batch resume or complete-group restoration
- **THEN** the failure surface displays the verified original batch ID, source directory and guide filenames. Windows explains the folder route from its displayed log directory; macOS provides the preserved-material action. Long portable names wrap without hiding content in Chinese and English. The offline CLI names the absolute source directory derived from its actual storage root. Missing/unsafe locations and unrecognized causes cannot grant located recovery or resume.

#### Scenario: Correct a file prerequisite before considering restoration

- **WHEN** the current failure establishes a file-use or access prerequisite, including while a complete original source is retained
- **THEN** consumers offer correction followed by current-version retry, without borrowing that source as a reason to restore old data. A key/unsupported-format diagnostic may retain a complete-group target only when the storage owner revalidates that exact batch and the recorded cause matches the current failure; a diagnostic alone never grants original-program reinstall, current-batch resume or restoration of unverified material.

Source Issue:
- https://github.com/higress-group/HiRoute/issues/176

### Requirement: Optional capacity retention uses restrained observation maintenance

HiRoute SHALL allow the local installation owner to save or disable one durable active-observation capacity threshold. The default threshold SHALL be 20 GB (20,000,000,000 bytes), including a fresh generation after old-history sealing. The owner may change it or explicitly disable capacity expiry; a disabled choice SHALL remain disabled across restart. Seven-day age retention remains independent. When enabled and exceeded, the existing lifecycle SHALL retire oldest eligible completed-request detail before seven days, preserving in-flight writes, live shared references, deletion/replay barriers, lawful non-content aggregates and recovery backups. Retained observations remain lossless; policy removals SHALL be distinct from representation savings and actual physical release. Accounting/reclamation SHALL use spaced bounded work, yielding and idle/failure/no-progress backoff rather than sustained high CPU or an unbounded catch-up loop. HiRoute SHALL report actual measured usage and pending/blocked release with its reason, without promising an instantaneous hard cap, reporting reusable pages as released, or dropping acknowledged writes. Changes/disabling affect uncommitted future retirement and do not restore already expired history.

#### Scenario: Default is 20 GB with independent age retention

- **WHEN** the owner has not saved a capacity preference, including after a fresh-generation upgrade
- **THEN** the effective threshold is 20,000,000,000 bytes and eligible detail may expire early when it is exceeded; ordinary seven-day expiry still applies independently.

#### Scenario: Explicit disable survives restart

- **WHEN** the owner disables capacity expiry and restarts HiRoute
- **THEN** the effective threshold remains disabled rather than reverting to 20 GB; capacity pressure alone does not authorize early expiry, while ordinary age retention continues.

#### Scenario: Young eligible history can expire

- **WHEN** a saved threshold is exceeded while completed eligible requests are younger than seven days
- **THEN** oldest eligible request detail is retired in spaced bounded batches through current visibility/reference rules while active requests, shared survivors, lawful aggregates and backups remain protected.

#### Scenario: Sustained pressure cannot busy-spin

- **WHEN** continued writes, contention or failed or ineffective release keeps usage above the threshold
- **THEN** maintenance yields and backs off between bounded passes, reports pending/blocked overage, and never continuously consumes CPU or drops acknowledged writes to force the limit.

#### Scenario: Logical retirement precedes physical cleanup

- **WHEN** capacity retention commits a request's logical retirement and its physical cleanup is interrupted or deferred, including across restart
- **THEN** fresh Plan-quality reads immediately omit stages ending in that request and omit assessments triggered by it from samples, filters and summary scores; existing quality cursors become stale, while unrelated retained evidence remains unchanged and physical cleanup resumes in bounded passes. Content-only deletion retains current request membership, stages and scores.

#### Scenario: Physical release is reported honestly

- **WHEN** retired detail is hidden but shared survivors or reusable database pages prevent immediate file shrink
- **THEN** status distinguishes policy deletion from released bytes and does not count removed requests as a lossless compression gain.

#### Scenario: Restart and disable preserve policy meaning

- **WHEN** a saved valid policy survives restart and is later disabled
- **THEN** its effective value is restored on restart, further uncommitted early expiry stops after disabling, and ordinary retention/replay barriers remain without resurrecting already expired data.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/151#issuecomment-6252214214

## Implementation references

- [Startup format admission](../../../crates/local-storage/src/migrations/startup_format.rs),
  [current migrations](../../../crates/local-storage/src/migrations/mod.rs), and
  [interrupted-batch scenarios](../../../crates/local-storage/src/migrations/upgrade_batch_recovery_tests.rs).
- [Managed content lifetime](../../../crates/observation/src/managed_text/README.md).
- [Supported release policy](../../../docs/release-contracts.md) and
  [compatibility registry](../../../contracts/compatibility-support.v1.json).
