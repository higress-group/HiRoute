# Local platform verification and build-artifact retention

Use the [HOME-configured entry](validation-routing.md) to select a local or SSH
execution host. `scripts/local-rust.py` supports Linux and macOS checks. It is a
small foreground runner, not a CI server or background cleanup daemon. Python 3.8+
and Unix `flock` are required; this script is not a Windows runner.
`cargo`, `rustc`, `sccache` and `git` are taken from PATH (load the usual Cargo
environment in non-interactive shells). When a required tool is missing, the runner
appends the standard rustup `~/.cargo/bin` as a last-resort lookup — never overriding
a tool already on PATH — and refuses with an actionable error before creating any
record or worktree if a tool is still missing. Host runtime paths are not hardcoded.

## Run against an exact pushed candidate

```sh
python3 scripts/local-rust.py run \
  --ref refs/heads/codex/my-proposal --sha FULL_40_CHARACTER_SHA \
  --cargo-only -- cargo test --locked -p hiroute-daemon delegation::
```

By default the runner fetches the branch from this repository's `origin` and verifies the SHA
is reachable. It creates a disposable detached worktree under
`~/.cache/hiroute/local-rust/checkouts/`, uses its default `target/`, jobs=2 (override with `--jobs`),
incremental=0 and shared sccache. It never restarts or empties the shared server.
It holds `~/.cache/hiroute/mac-rust-validation.lock` on Mac, or
`~/.cache/hiroute/local-rust-validation.lock` on Linux, plus a checkout lock; a busy
lock rejects the run rather than starting another local compiler by default. With
`run --wait-for-lock`, the runner waits inside the same exclusive lock acquisition,
after preparing the exact checkout and before starting Cargo. This avoids releasing
an external readiness lock and racing another run during checkout preparation.
Waiting does not change capacity, source selection or the command timeout (which
starts with Cargo); interrupting a waiter releases its handles without starting a
compiler. Cleanup commands always remain nonblocking. The record stores the chosen
wait policy and existing queue timestamps. The runner checks the free-space minimum
again after a wait before Cargo can start. A command monitor
holds these locks until Cargo exits, even if the runner crashes; Cargo and its
children do not receive the lock descriptors. A crashed/unknown record is retained,
not recovered by guessing.

For local committed source, `--source local` fetches the branch from this local
repository instead of origin. No push is required and uncommitted edits are never
compiled. The result records candidate source, platform and compiler jobs. The
configured local backend selects this mode automatically. Remote Desktop keeps
the origin-fetch default. The independent SSH Linux workbench has its own locks;
do not run both runners on one host/account to bypass its capacity limits.

Before creating a run TMPDIR, cache admission holds the existing compiler lease,
compiles a unique tiny Rust library through sccache with owner-only stable
`sccache-tmp/` under the runner root. The request connects to an existing server
or starts one using that stable environment, with a 30-second bound. A cache miss
exercises the
server's effective scratch; environment metadata and `--show-stats` alone do not
prove that the server started or uses that directory. An already-running server
is admitted only if the probe succeeds. No server is automatically stopped.
Admission failure prevents Cargo execution and retains `cache-admission.json` and
`cache-probe.log` beside the ordinary result. The same command monitor retains
compiler and checkout locks until the probe exits if the runner crashes; cache
clients and servers never inherit those lock descriptors. A timed-out probe's
client process group is reaped before releasing capacity. After all
compiler owners are idle, an operator may hold the host's exclusive capacity
locks, preserve statistics and explicitly stop/start the server using the recorded
stable scratch, `TMP`/`TEMP`, idle timeout zero and existing cache configuration.
Never recreate an obsolete run directory or clear compiler objects to recover.

TMPDIR is private and canonical. If macOS's per-user temporary path is too long for
Unix sockets, the runner resolves the OS `/tmp` directory and creates a short
private child there. No host IP or user-specific absolute directory is in Git.

Records, command output, executor script hashes, the source SHA, Cargo process exit,
parsed test counts, scenario status and before/after sccache statistics are under
`~/.cache/hiroute/local-rust/runs/ID/`. Compilation failure, timeout and missing
scenario assessment are not green. A zero-match test selection is not success.

Every ordinary run defaults to immediate reclamation after execution, including
failed commands, timeouts, interrupted waits and preparation blocks. The runner
keeps its acquired compiler/checkout leases through report publication, evidence
export and cleanup. It saves results, logs and cache diagnostics in the run record,
exports non-compiler target artifacts to `runs/ID/artifacts/`, preserving Pilot's
existing consumer paths. Checkout `artifacts/` and surviving private TMPDIR contents
go to `runs/ID/evidence/`. The `evidence.json` manifest records each saved path and
file hash, including smoke build receipts with complete recipes.
Debug/release/custom Cargo profiles, cross-target profiles, generated documentation,
package outputs, smoke binaries and `target/validation-schedule` executable snapshots
are rebuildable and are not copied. Inherited Cargo targets are recorded; Cargo's
profile lock also identifies configured and previously used custom profiles. Keep unique evidence outside compiler profiles; use
`--keep` when arbitrary debug files or built binaries are still needed.

`--cargo-only` asserts a pure Cargo/component check with no separate product
assessment. It controls the scenario verdict, independently of cleanup. Omitting
it leaves exit-zero execution `unassessed` and the wrapper returns nonzero; the
saved record can still be assessed after its checkout is removed. Failure and
cleanup never manufacture a green scenario.

`--keep` is the explicit exception for investigations, subsequent binary use and
Desktop/Pilot acceptance. Native Pilot acquisition already requests retention;
its final registered owner releases it through `pilot-build finish`. For a manually
kept E2E run, assess actual scenarios and export any unique debug evidence before:

```sh
python3 scripts/local-rust.py finish RUN_ID --verdict green --evidence-saved
python3 scripts/local-rust.py release RUN_ID
python3 scripts/local-rust.py preview
python3 scripts/local-rust.py apply RUN_ID --token TOKEN_FROM_PREVIEW
```

Final cleanup of new explicitly kept runs uses the same evidence exporter before
removing the checkout and TMPDIR. Unknown source/untracked/ignored material or an
export failure retains the originals with a reason; save or remove the material
and retry cleanup. Existing records keep their earlier evidence-confirmation rules.

Use `red` or `expected_red` honestly; neither proves product completion. `finish`
records assessment but does not itself delete anything. Failed report/export,
links or special files in evidence, source changes, unknown generated files or
changed resource identities retain the checkout with an explicit cleanup reason.
Private runtime evidence is exported before its TMPDIR is removed. Developer
source worktrees, other owners and shared compiler caches remain separately owned.
Existing retained records keep their original evidence-confirmation requirements.

## Retire a developer target explicitly

```sh
python3 scripts/local-rust.py retire --checkout /absolute/worktree --evidence-saved
python3 scripts/local-rust.py preview
python3 scripts/local-rust.py apply RECORD_ID --token TOKEN_FROM_PREVIEW
```

Retirement is an owner/coordinator assertion that no task, test, launched binary or
raw Cargo command will use this debug directory, and its unique evidence has been
saved. Do not retire an active owner just because a process scan is empty. A raw
build that bypasses this locking protocol cannot be detected race-free; such trees
must not be retired until their owner confirms inactivity.

Only that checkout's real, untracked `target/debug` can be removed. Source changes
are preserved, as are `target/release`, E2E reports, Wasm/AOT outside debug and all
Git branches. If valuable files are inside debug, copy them out before confirming.
No developer worktree is removed by this script. Path/identity changes, tracked
target files, active locks or changed preview metadata reject deletion.

```sh
python3 scripts/local-rust.py keep RECORD_ID
python3 scripts/local-rust.py release RECORD_ID
python3 scripts/local-rust.py gc --older-than 7
python3 scripts/local-rust.py gc --older-than 7 --apply
```

GC considers only terminal, evidence-confirmed **managed** checkouts older than
seven days, never developer targets. `keep` excludes a record; `release` removes
that flag but cannot convert an active run into an inactive one. Preview is
non-destructive; apply rechecks the snapshot and holds the same locks until done.

Before local validation, less than 30 GiB free triggers this bounded managed GC,
aiming for 60 GiB. If still below 30 GiB, compilation is refused. Unknown or kept
directories are not sacrificed to meet the target. The SSH runner uses the same immediate-cleanup default through `--retention never`;
`--retention always` and `on-failure` are explicit retention exceptions.

## Owner cleanup during build/fix/retest

For an explicitly requested bounded maintenance pass, the manual wrapper defaults
to preview and a minimum age of 48 hours:

```sh
python3 scripts/local-rust-gc.py --limit 10
python3 scripts/local-rust-gc.py --limit 10 --apply
```

`--older-than-hours` may raise this threshold; `--limit` bounds candidate attempts,
including attempts rejected after preview. The JSON report separates selected,
removed and retained records with reasons; exit zero does not mean every candidate
was removed. This wrapper delegates preview, snapshot revalidation, locks and
deletion to `local-rust.py`; it adds no record schema or second cleanup policy.
It never overrides `keep`, missing evidence confirmation, Pilot ownership, source
changes, generated files outside target, or a changed/missing directory identity.
Automatic low-space GC retains its existing seven-day threshold.

After a volume remount, a changed device number still refuses automatic deletion.
Do not infer identity from an inode alone or rewrite the old run's identity.
After the owner confirms the exact checkout is no longer in use and exports unique
debug evidence, use the existing explicit `retire` flow below. This records the
current identity separately and removes only debug output; the old run's verdict,
source and non-debug artifacts remain intact.

Cleanup is an owner action using the commands above, not an additional admission
check in `run`. Before another local build, after assessing a validation run, and
at handoff, review this task's earlier run records (`runs/ID/result.json`; `repo`
identifies the originating checkout). Retire outputs no longer needed rather than
accumulating `--keep` runs. Preserve any version still used by an application,
comparison or investigation; record its run ID, purpose and release condition in
the existing task notes. There is no fixed number of retained runs or new approval
step. A busy cleanup lock or an output still in use does not block unrelated work.

For a managed run with an actual scenario assessment, use `finish` with that
verdict and `--evidence-saved`, then `release` if kept, `preview`, and `apply` with
the preview token. Preserve process exit and scenario outcome separately.

When only obsolete compiler outputs should be discarded, including a managed
checkout whose original scenario remains unassessed, use the existing debug-only
retirement tool:

```sh
python3 scripts/local-rust.py retire --checkout /absolute/old-validation-checkout --evidence-saved
python3 scripts/local-rust.py preview
python3 scripts/local-rust.py apply RETIREMENT_RECORD_ID --token TOKEN_FROM_PREVIEW
```

Use the new retirement record ID, not the original managed run ID. This preserves
the source checkout, non-debug artifacts and original run verdict; it does not
claim that the original scenario passed or failed. The owner must first stop using
the selected debug output and save any unique evidence outside it. Do not retire
another owner's output or infer inactivity from age or an empty compiler scan.
If generated files block whole-checkout cleanup, debug-only retirement preserves
them without bypassing source checks or inventing a scenario verdict. Keep the
existing shared locks and safety checks; do not add a cleanup daemon, retention
quota or cross-task build barrier.

## Tests

```sh
python3 scripts/test-local-rust.py
python3 scripts/test-local-rust-gc.py
python3 scripts/test-remote-rust.py
```

Tests use disposable Git repositories, filesystem fixtures, held locks and child
processes. Fake Cargo lifecycle tests verify the runner, **not Rust or HiRoute
product functionality**. A real Mac Cargo invocation still requires the relevant
exact pushed candidate and its platform environment.

An opt-in integration test uses a tiny dependency-free crate pushed to a disposable
local bare origin, real Cargo and an isolated sccache endpoint/cache under the shared host lock. It tests this runner's
fetch/build/evidence/reclamation path, two compilations across run cleanup, stale-server refusal and default failed-test reclamation and explicit retention without compiling the HiRoute workspace:

```sh
HIROUTE_REAL_CARGO_TEST=1 python3 scripts/test-local-rust.py Cleanup.test_real_cargo_pushed_fixture_lifecycle
```
