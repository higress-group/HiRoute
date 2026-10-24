# Remote Rust development workbench

## Purpose

Use the [HOME-configured entry](validation-routing.md) to select build/test locations.
Mac developers can continue using this Linux workbench; Linux developers can
select local backend validation and send Desktop/Pilot work to a configured Mac.
This is an SSH workbench,
not a GitHub runner, CI replacement, or mandatory full-Gate queue.

Commit and push the candidate first. Each invocation fetches the advertised branch,
checks that the full requested SHA is reachable, and creates a detached worktree
with its own default `target/`. No working-directory source is uploaded. The driver
itself is read from the caller's committed HEAD and identified separately in evidence.
Use a pushed driver revision; an older candidate can be tested by passing `--sha`.

## Personal configuration

Create `~/.config/hiroute/remote-rust.json` outside the repository:

```json
{"host": "your-ssh-alias", "workbench": ".local/share/hiroute-rust-github"}
```

Configure SSH authentication in your own SSH configuration. The remote host needs
Git access to this checkout's `origin`, Python 3.8+, the repository's Rust toolchain,
cmake, clippy, rustfmt and sccache. Proxy configuration belongs in the remote user's
shell configuration, not Git. The launcher sources the remote `.bashrc`; Rust binaries
are located through `~/.cargo/bin`. Only trusted repository commits may be run: build
scripts execute as your remote account, without an additional security sandbox.

The default remote workspace is `~/.local/share/hiroute-rust/`. Set `workbench` in
the HOME configuration, or pass `--workbench PATH` before the action, to keep
repositories with different origins in separate HOME-relative workspaces. The same
selection applies to submission, wait, status, logs and cleanup. Read old run records
with an explicit old `--workbench .local/share/hiroute-rust`; never relabel their SHA.
Paths cannot contain traversal or shell syntax. All configured workspaces still
reserve the original `~/.local/share/hiroute-rust/slot*.lock` capacity locks, including
locks used by older controllers. A separate repository does not add compiler slots. The shared sccache server must already report a 30 GiB limit; the
runner does not restart a shared server or reset its statistics.

Shared sccache scratch is an owner-only `sccache-tmp/` under that workbench root,
not any run's disposable `TMPDIR`. Before invoking Cargo, under the capacity lease,
the runner requires a unique cache-miss Rust compilation through sccache with
that stable directory and `SCCACHE_IDLE_TIMEOUT=0`. This connects to the existing
server or starts one using stable scratch, with a 30-second bound; statistics
alone do not prove startup or scratch health. An
existing server is never stopped automatically. Failed admission blocks Cargo and
retains `cache-admission.json` and `cache-probe.log`.
The command monitor retains capacity and checkout locks across a worker crash
until the probe exits, without passing them to cache clients or servers. A
timed-out probe's client process group is reaped before releasing capacity. Recovery
requires all compiler owners idle, exclusive account-wide capacity locks, retained
statistics and an explicit operator stop/start with stable scratch; never recreate
a deleted run directory or clear cache objects. Before/after stats
use the same server environment. Cargo and tests still receive their own short
private temporary directory. Run cleanup never removes shared server scratch.
An already-running server keeps its original environment: if it inherited a deleted
run directory from an older driver, coordinate a one-time server restart only after
reserving all eight workbench slots and confirming no other Rust builds are active.
Use the stable scratch directory, idle timeout zero and existing 30 GiB cache on
restart. Preserve before/after stats: server counters restart, disk cache objects do
not need clearing. Do not recreate a deleted run directory to conceal the fault.
Unexpected server failure during a run still requires failed-run diagnosis/retry;
the runner does not restart a busy shared service automatically.

## Daily development

From your Proposal's checkout, commit and push your changes, then run:

```sh
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal -- \
  cargo test --locked -p hiroute-gateway-core --all-features --test gateway_lifecycle
python3 scripts/remote-rust.py wait RUN_ID --timeout 50
python3 scripts/remote-rust.py status RUN_ID
python3 scripts/remote-rust.py logs RUN_ID
```

Submission returns immediately with a run ID. The worker survives SSH disconnection.
`wait` polls the run record on the workbench and returns one compact JSON line when
the run terminates or its bounded wait expires. `wait_state=timeout` means the run
is still queued, waiting or running; it is not a test failure. Use another bounded
wait when needed. If a shell tool yields a live session before `wait` returns,
continue that session instead of issuing a separate `status` request. Read full
`status` and logs after a failure or when a specific evidence field is needed;
the compact summary does not establish a product scenario verdict. The five-minute
performance field appears only for a complete-workspace DAG run, not for format or
focused commands.
`queued`/`waiting` means no result yet; `running` means the slot was acquired and the
checkout prepared. `blocked` means Cargo did not start because of an environmental
precondition such as insufficient free disk, so `process_exit` is null. `failed` can
mean setup failure or a non-green Cargo/test result; neither is automatically a Rust
test-process failure. `completed` means the Cargo process succeeded and, for ordinary
Rust test runs, nonzero passing tests were observed. It is **not** a Product E2E
semantic verdict.

Eight runs may execute concurrently, each with `CARGO_BUILD_JOBS=8`. All runs use
`CARGO_INCREMENTAL=0`, `RUSTC_WRAPPER=sccache`, and `SCCACHE_CACHE_SIZE=30G`.
The runner keeps slot locks in a command monitor. Cargo and its descendants do not
inherit those lock descriptors, so an sccache server cannot retain a finished run's
capacity lock.

`--exclusive` reserves all eight slots and uses 64 compiler workers. Use it for large
integration checks and benchmarks; benchmarks require it. This limits workbench
compiler concurrency, not arbitrary test-created threads or processes outside the
workbench. Scheduling is deliberately simple, without priority or FIFO guarantees.

For a change that affects several integration targets, submit them in one focused
Cargo invocation. This pays checkout, dependency discovery and link startup once. Choose
the explicit collection policy below when diagnosis needs results across the selected
targets; default fail-fast may leave later targets unexecuted. Pure in-process targets
should use the Rust harness's default parallelism. Bundles containing real `hirouted` or protocol-matrix
processes must use one harness thread: those targets create listeners and helper
processes before their target-local lifecycle guards can serialize them.

```sh
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal -- \
  cargo test --locked -p hiroute-e2e --all-features \
  --test p0_gateway_context_hold --test p0_gateway_observation \
  --test p0_gateway_planner --test p0_gateway_protocol --test p0_gateway_runtime \
  -- --test-threads=1
```

Default plans preserve Cargo's fail-fast behavior because structured feedback is saved
at command completion. For batch diagnosis within a declared scope, opt in to
`--no-fail-fast`: Cargo then continues other selected test binaries after a test failure.
The runner preserves this choice; it never injects the flag automatically. Build failure,
timeout, interruption or missing output can still leave the collection incomplete; this
flag does not make a multi-command plan continue. See [failure policy and completeness](test-selection.md#failure-policy-and-completeness)
and [validation feedback](validation-feedback.md) for result timing and remaining obligations.

```sh
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal --exclusive -- \
  cargo test --locked --no-fail-fast -p hiroute-e2e --all-features -- --test-threads=1
```

```sh
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal --exclusive -- \
  cargo clippy --locked --workspace --exclude hiroute-desktop --all-targets --all-features -- -D warnings
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal --exclusive -- \
  cargo test --locked --workspace --exclude hiroute-desktop --all-features
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal \
  --exclusive --schedule workspace-dag --plan my-change --phase final -- \
  cargo test --locked --workspace --exclude hiroute-desktop --all-features
python3 scripts/remote-rust.py run --ref refs/heads/codex/my-proposal --exclusive -- \
  cargo bench -p hiroute-gateway-core --bench resource_paths
```

`workspace-dag` is the bounded full-backend schedule described in
[validation routing](validation-routing.md#continue-an-integration-checkout-and-schedule-the-final-workspace).
It accepts only the exact workspace test command above, optionally with
`--no-fail-fast`. It reserves eight slots, overlaps ordinary targets with one
product-build writer, verifies every selected test case and runs doctests.
The result separately records correctness and `performance_goal_met` for the
five-minute execution target. `workspace-smoke` remains an explicit older entry;
neither entry silently retries through the other.

On the current backend-only Linux workbench, workspace build/check/test/clippy/bench
commands use `--workspace --exclude hiroute-desktop`; focused checks may instead
select backend packages explicitly. This machine has no Desktop validation
environment. Current Desktop validation uses the available macOS environment;
Record Windows coverage from the checks actually executed and their environment; missing evidence remains an explicit validation limit.
These limits describe available environments and evidence, not product support:
they do not declare Linux or Windows unsupported. Reassess the validation scope
when another Desktop environment is provisioned. Do not install D-Bus/GTK/WebKit
desktop dependencies as an incidental backend-gate repair. Report these runs as
backend workspace checks; they do not prove Desktop compilation. The Linux CI
backend jobs below follow the same package scope; a configured platform-smoke job
alone is not proof of a usable Desktop environment or a successful platform run.
Non-compiling `cargo fmt --check` may still cover the entire workspace. The runner
preserves the requested command rather than silently rewriting package selection.

Supported commands are build/check/test/clippy/bench and read-only `cargo fmt
--check`; shell expressions and Cargo config/manifest/target-directory overrides
are not supported. Use `--timeout SECONDS` before `--` to override the one-hour
command timeout. Ordinary invocations use separate checkouts and share only
sccache objects; `--reuse-checkout` and `--continue-checkout` explicitly retain
one checkout's default target under its lease. Local `cargo fmt --check` and Python
tooling tests need no remote compilation. Windows/macOS-specific behavior still
needs the appropriate platform.

### Checkout retention and cleanup

For local macOS/platform checks, use the separate [local runner and cleanup
commands](local-rust-validation.md). They preserve the same default-target boundary;
the HOME configuration selects the execution location. Both runners reclaim ordinary
terminal runs after saving evidence; explicit retention supports subsequent use.

Every run has an isolated default `target/`; it is deliberately never shared with a
different checkout. sccache reuses compiler objects safely, but Cargo must still write
link outputs, test binaries, fingerprints and build-script products into that run's
own target. Keeping every successful target therefore consumes disk without being
required for normal incremental development.

`run` defaults to `--retention never`:

| Policy | Completed run | Failed or blocked run | Appropriate use |
| --- | --- | --- | --- |
| `on-failure` | Automatically removes checkout, private target, private TMPDIR and validation ref after evidence is written | Retains them for diagnosis | Explicit failure diagnosis |
| `always` | Retains them | Retains them | E2E artifacts, benchmark investigation or an intentional inspection checkpoint |
| `never` (default) | Automatically removes them | Automatically removes them | Ordinary builds and tests |

Run logs, result JSON, cache statistics and worker diagnostics are retained in the
run record. Before automatic cleanup, the worker holds its checkout lease while
exporting non-compiler target artifacts to `runs/ID/artifacts/`, and checkout
`artifacts/` and surviving private TMPDIR contents to `runs/ID/evidence/`. The
`evidence.json` manifest records the source revision, saved paths and file hashes.
Smoke build receipts preserve complete recipes; adjacent binaries and scheduler
executable snapshots remain rebuildable cache and are not copied. Cargo
debug/release/custom profiles, cross-target profiles, generated documentation and
package outputs are also excluded. Inherited Cargo targets are recorded and Cargo
profile locks identify configured and previously used custom profiles. Put unique evidence outside compiler profiles;
select explicit retention for binaries or arbitrary compiler-profile files needed
later. Report/export failure retains the originals and records the cleanup reason;
cleanup does not change the command's exit or scenario verdict.
Automatic cleanup first verifies a terminal result, the recorded worker lifecycle, the
direct managed-worktree path and Git registration. New records also reject modified
source and unknown untracked/ignored files, preserving the checkout and TMPDIR until
the owner saves or removes that material. Explicit cleanup re-exports evidence under
the checkout lease. Known generated roots such as target, exported artifacts and
Python `__pycache__` may use Git's force removal **only after** those checks.
It never force-removes an arbitrary
path, an active run, a shared cache or a user checkout.

Inspect and explicitly reclaim one completed run when its retained artifacts are no
longer useful:

```sh
python3 scripts/remote-rust.py cleanup RUN_ID
```

Preview aged terminal-run cleanup first; apply it only after reviewing the candidate
list. Runs submitted with `--retention always` are excluded unless explicitly included.

```sh
python3 scripts/remote-rust.py gc --older-than 7
python3 scripts/remote-rust.py gc --older-than 7 --apply
python3 scripts/remote-rust.py gc --older-than 7 --apply --include-retained
```

Cleanup also removes that run's ephemeral `refs/validation/RUN_ID` reference after its
checkout is safely gone. It does not automatically run repository-wide `git gc`: that
operation affects the shared bare repository and must be scheduled separately when no
workbench activity is in progress.

## Evidence and maintenance

Each remote `runs/RUN_ID/` contains the immutable request, result JSON, command log,
worker diagnostics and before/after cache statistics. The result records candidate
SHA, fetched branch SHA, executor SHA/digest, exact argv, toolchain, compiler jobs,
checkout, timestamps, Cargo process exit and parsed Rust test counts. Cache counters
are shared with other runs: before/after deltas are approximate under concurrency,
not exclusively attributable to one run. Do not reset counters for measurement.

Each run resolves the system temporary directory to its real path, creates a unique
owner-only `0700` directory, and passes it as `TMPDIR` to Cargo and its test children.
The short `hr-` name does not include the checkout or run ID: result JSON records
the association. The full path is limited to 40 bytes, leaving room for nested Unix
socket names. An overly long configured system path fails before compilation with
an actionable error; the runner never substitutes a hardcoded host directory or
weakens product symlink, ownership or permission checks. Arbitrarily deep test-created
paths can still exceed the socket limit and remain the fixture's responsibility.

The private temporary directory is recorded in `temporary_directory` and `command.log`.
Its lifetime follows the selected retention policy. Test-owned `TempDir` objects may
still clean up their own children; retaining the parent does not override test
destructors. No global temporary-directory permissions or real Agent configuration are
changed.

`HOME` is intentionally unchanged for real Agent discovery. Tests that depend on a
clean home must isolate their own fixtures: setting `TMPDIR` does not make the real
user's Agent configuration safe or absent. A daemon failing on an unsafe existing
Agent file remains a separate test/product issue, not justification to chmod that
file or bypass its validation. Tests hardcoding `/tmp` also bypass this isolation.

For E2E, inspect the scenario artifacts in the recorded checkout and report `green`,
`expected_red`, or `red` separately. Exit zero or Rust harness success alone does not
prove product completion. Unknown test output or zero passing tests require review.
After changes, push a new SHA and rerun the affected checks; do not relabel old logs.

New work stops below 30 GiB free disk space. Use the runner's `cleanup` or reviewed
`gc` operation instead of hand-written broad deletion commands. Never delete an active
checkout, share target directories, reset sccache or use `cargo clean`. After a host
restart, old running records are not success evidence: check worker processes and
resubmit interrupted runs. No automatic restart or distributed lease system is provided.

## Stage-level validation

Use the commands at the candidate SHA in `.github/workflows/gateway-core.yml` and
`.github/workflows/p0-gateway-gates.yml` when reaching a milestone; the examples above
do not constitute their complete Gate. This runner intentionally does not maintain
a second workflow engine or pretend native Linux checks cover macOS/Windows,
durable-spec checks, sealed final-product evidence or dedicated performance Gates.
Existing specialized shell harnesses remain the authority for those checks.

`act` is deferred. It can later check Docker compatibility of direct workflows,
with separate Cargo registry/sccache mounts and no shared target volume. It must not
be the sole final Gate: see the [official compatibility limitations](https://nektosact.com/not_supported.html)
and the [open local reusable-workflow validation report](https://github.com/nektos/act/issues/5992).
