# Local Worker launcher

Implements the daemon consumer `WorkerPlatformPort`. It owns OS child objects,
stdio, and explicitly requested temporary materials. It does not interpret ACP,
permissions, Harness configuration, Plan, tokens, or sessions.

## Production ownership

The daemon's [profile builder](../profile/mod.rs) prepares the candidate;
[execution](../executor.rs) drives the lifecycle through
[`WorkerPlatformPort`](../platform.rs). This module consumes the prepared
materials and session root, owns process/stdio handles, and cleans up only its own
private root. Harness discovery, ACP interpretation, authorization and session
history remain in their respective sibling modules.

`launch` directly consumes `CandidateWorkerProfile.materials` and `session_root`.
There is no preparation registry, path template, or alternative launch API.
The external session root must already exist and must not equal or nest with the
new private root. Materials may contain explicitly rendered directories and opaque files, within
the entry-count and content-size limits in [materials.rs](materials.rs). Paths are
relative to the owned root and parent directories must be explicit; the launcher
does not interpret file contents or infer additional paths.
The launcher never derives ownership from HOME/CODEX_HOME/CLAUDE_CONFIG_DIR.
The launcher checks the explicit executable path and basic executability without
reading program contents or verifying pins. Installation discovery and Harness
profile rendering remain with the selection path.

On Unix each child starts a fresh process group. Stop sends TERM once, reserves a
one-second native cleanup window, then uses KILL on that same owned group. Native
Harnesses use this window to reclaim ordinary tools in separate process groups;
turn cancellation alone may keep background terminals alive. Safe `waitid(NOWAIT)`
observes without reaping the group leader; the retained leader protects group
identity throughout both phases, including an adapter exiting before its Harness.
A caller whose budget expires retains ownership and can continue the same stop.
After reaping, group identifiers are used only for read-only absence checks, never
another destructive signal. Neither the KILL fallback nor a stopped original group
proves arbitrary detached processes are gone. Real native cancellation scenarios
must verify actual tool side effects stop while a neighboring run continues. Windows
uses a held child only: root scope remains insufficient proof of ordinary child
chain cancellation until native Windows validation and any required mechanism.

Tests execute this production module with a no-model OS probe. They do not prove
real Harness/ACP or CLI/Desktop wiring; those remain consumer/integration checks.

Cleanup keeps an open directory identity handle, verifies the current root is the
same object, and rejects symlink/reparse replacement before deleting its own root.
This avoids authorizing deletion solely from a stale path/inode number.

## Representative checks

[Platform scenarios](../../../tests/local_worker_platform.rs) cover process-group
stop and cleanup ownership. The [Worker product journey](../../../../../tools/product-e2e/tests/worker_delegation.rs)
and [read/continue journey](../../../../../tools/product-e2e/tests/worker_read.rs)
exercise the broader consumer path. Record the actual platform and proof level;
an OS probe cannot establish real Agent compatibility.
