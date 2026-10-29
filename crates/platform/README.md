# Operating-system primitives

This crate owns shared filesystem primitives and Windows kernel handles/FFI. It does
not select an Agent, authorize a business command, or own business state.
`file_security::macos` provides safe, read-only descriptor calls for native ACL/flags
and extended attributes, including compression attributes. Callers supply bounded
buffers and own snapshot validation, hashing and diagnostic policy; these opaque
native observations do not authorize access or define a persisted permission format.
`windows::background` owns the native Task Scheduler and Service Control Manager
adapters, fixed Known Folder resolution, and their closed installation definitions.
The CLI and host runtime select lifecycle policy and maintain installation journals.

`windows::install` supplies native x64 admission and the per-account NSIS/recovery
guard. `shell_login` owns only the fixed current-user Run value for an admitted
Programs-directory shell. `user_path` merges/removes only the fixed CLI entry and
broadcasts the environment change. `shell_open` admits bounded HTTP(S) URLs before
the native browser association. These leaves do not mint console credentials,
select releases or authorize product operations.

`private_fs` is the cross-platform entry for new private directories/files, storage
directory preparation, regular/private handle reads, single-link checks and owned
temporary directory removal. Unix and Windows implementations stay in separate
leaves. Storage preparation preserves existing Unix directory modes; it does not
adopt borrowed Agent directories. New objects receive 0700/0600 modes or the current
Windows user's private descriptor at creation. Executable run materials retain their
explicit Unix 0700 intent. Reads reject symlinks, reparse points and special leaf entries without changing
permissions; Windows private reads also validate the owner/DACL. Payload limits,
format checks, storage error mapping and borrowed configuration restoration remain
with their existing owners.

`private_fs::OwnedDirectory` admits only a newly created root and retains its
identity. Its explicit removal preserves Unix identity-checked recursive deletion
and Windows bounded handle-relative deletion. It has no automatic Drop cleanup:
the Worker owner decides when processes are stopped and materials can be removed.
The shared [private material cases](src/private_fs/tests.rs) cover non-adoption,
exclusive creation, executable intent, protected reads and cleanup boundaries.

`windows::security` is the shared implementation of the Gateway's existing
owner-only ACL creation, verification, and held volume/file-index identity.
New private objects receive their descriptor during creation. `open_identity`
checks the opened object type and rejects reparse points. Existing directory
handles retain identity across renames; private creation and ordinary file reads
deny deletion sharing. `open_owner_only` applies the explicitly requested private
descriptor check; it is not a repair or blanket admission rule for native files.
`open_read_lease` retains the whole reparse-free pathname for a bounded snapshot:
every ancestor denies deletion and the regular, single-link file denies data
writes and deletion. Reopening uses the final held parent; readers compare
metadata, identity and native permission facts without adopting external ACLs.
`open_read_write` opens an existing regular file without create, truncate or
DELETE permission and denies deletion while it is held. Storage lock owners use
it to preserve the admitted object's identity and native permissions.
`set_owner_only` may tighten a caller-owned object's DACL; it checks the actual
owner before mutation and never adopts an object owned by another SID.
`initialize_current_process_private_defaults` sets only the current primary
token's default Owner to its actual TokenUser and its default DACL to that SID's
generic full access. Daemon hosts call it before C/Go storage creation; it does
not change groups, privileges, integrity, session or existing objects. The Job
launcher initializes the independently identified child token before resuming
its initial thread, and duplicates a borrowed primary token before assignment.
`open_inherited_private_file_at` is limited to controlled CPA-generated files
under a held strictly private directory: actual user Owner, one user full-access
ACE, no reparse and one link. It accepts the inherited ACE flag, while ordinary
private verification continues to require the protected descriptor. Borrowed
native-client credentials retain their separate accessible-file contract.

`windows::pipe` provides owner-only local byte streams. Its namespace binds the
current account SID to the held parent file identity and endpoint leaf, so a service and a CLI in
different logon sessions can connect as the same user. First-instance binding
rejects endpoint occupation; remote connections are rejected. Kernel peer PID
queries support launcher-owned process checks. No filesystem socket is created.

Every pending read, write, or connect owns an event and a stable OVERLAPPED
allocation. A timed-out operation is cancelled and completed before its buffer
is released. Listener polling and connection deadlines retain their callers'
budgets. Protocol owners provide bounded frames and response completion; a byte
stream does not manufacture Unix write-half-close or unbounded flush semantics.

Windows native tests exercise held identity, bidirectional frames, endpoint
exclusivity/rebinding, peer queries, and deadline cancellation followed by another
frame. Windows CI must verify that these cases are actually selected; an empty
test inventory on another platform is not their acceptance evidence.

`windows::job` launches inspected native executables suspended, assigns the
kernel Job and explicit inherited handle list before execution, and stops the
whole Job when the leader exits. Numeric PIDs do not grant termination authority.
`windows::launcher` supplies a fixed system PowerShell helper for managed commands
whose argv must cross a caller's cmd invocation. Credential pipes are exclusive,
private, bounded and explicitly inherited; their raw handle values are locators.

`windows::console` attaches an explicitly foreground GUI daemon to its parent's
console while preserving redirected I/O. It restores Ctrl-C processing after
attachment, including when a shell passed the inheritable Ctrl-C-ignore attribute;
the daemon then registers its own shutdown handlers. Background launches do not
attach a console or change this foreground policy.

Background lifecycle messages authenticate a held process image, creation time,
and pipe peer identity. Task mode uses the same interactive account at LUA level;
SCM primitives are retained without a supported installation entry or user
installation guide in this release; the Rust transaction refuses SCM ownership
before writes. The retained SCM code authenticates a qualified owner account and uses a restricted business
token beneath an administrator protected service host. `scm_owner` reads the
owner's private prepared journal only while impersonating that authenticated
account, then reverts before machine registration. Sealed publication copies a
held daemon from the administrator's trusted package and requires its digest to
match the owner's prepared image. A user supplied path or self reported digest
cannot select the administrator's executable.

Filesystem traversal uses synchronous `NtCreateFile` with an inspected
`RootDirectory`, one component, `OBJ_DONT_REPARSE` and a handle type check.
Traversal handles share deletion to coexist with DELETE-capable private parent
handles; the relative operation remains bound to the inspected object across
ancestor renames. Private creation attaches the DACL before publication. Files
and created directories deny delete sharing; existing directory identity handles
retain identity but do not promise to prevent a rename. Replay child creation
uses the caller-owned parent directly. Pipe names hash the held parent volume /
file identity and endpoint leaf instead of reopening its pathname.

`create_owner_only_buffered_file_at` is the separate temporary Replay payload
factory. It preserves the same private descriptor, exclusive sharing and relative
identity checks, while allowing filesystem caching during fragmented writes.
Replay flushes the complete payload with `sync_all` before sealing it. Persistent
keys, journals and namespace transitions retain their write-through factories.

`durable` replaces files through unique private stages. Initial publication cannot
replace an existing key: Unix preserves the released `create_new`, mode-0600 write
and file flush, without requiring hard links; Windows stages a complete payload
before no-clobber publication. Failed Unix replacements remove only the writer's
still-unpublished stage with matching device/inode identity. Windows stages and namespace
transitions use write-through handles; file payloads use `FlushFileBuffers` before
and after held-source, held-parent rename. Windows directory barriers retain the
inspected identity and do not claim POSIX directory fsync. Existing operation and
backup journals keep their writing, committed and final checkpoints. This follows
[Windows file caching](https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching)
and [handle-relative rename](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info).
Process interruption tests do not establish behavior under hardware power loss.

`durable::sync_unpublished_copy` is limited to unpublished complete-source snapshot
copies. macOS flushes each copied file's bytes and permissions with POSIX `fsync`,
then the snapshot owner retains `directory_barrier` (`F_FULLFSYNC`) at each completed
directory and before manifest/stage publication. The final device flush includes
the preceding file writes; no incomplete stage becomes a recovery source. This
avoids one device-cache flush per small body without omitting files, hashes,
permissions or recovery validation. General key/journal writes, Linux file syncs
and Windows descriptor-bound copying are unchanged. See Apple's
[fsync and full-device flush contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html).

Native configuration restoration has a separate sealed-file namespace operation.
It opens an already flushed source without requesting data-write access, retains
its read-only facts, and asks the kernel to check `WRITE_ATTRIBUTES` when replacing
a read-only target. General durable renames retain their ordinary replacement
rules; neither path clears target attributes by name or bypasses sharing checks.
`write_windows_atomic_with` lets stopped-snapshot owners stream into the same
unpublished descriptor-bound stage and validate their held source before the
platform flushes and publishes it.

`file_security` captures the actual Windows owner/DACL SDDL and read-only flag
from a held file. Conditional native restoration authenticates these facts along
with content; it creates its stage with the recorded descriptor and verifies the
result before publication. Native creation may recompute DACL inheritance control.
The unpublished held stage restores the captured DACL through `SetSecurityInfo`
when necessary and must retain the exact captured descriptor; control bits are
never discarded from equality or journal authentication. Existing native files
retain their accessible ACLs during reading. A descriptor that the account cannot restore fails before target
replacement. Unix artifact modes and released fingerprints remain unchanged.
