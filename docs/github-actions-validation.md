# GitHub Actions validation

GitHub Actions runs on the checked-out commit. Linux checks use `scripts/ci-run.py`;
native Windows acceptance uses `scripts/windows-headless-ci.ps1`.

## Workflows

| File | Execution | Scope |
| --- | --- | --- |
| `gateway-core.yml` | PR, main push, manual, reusable | Affected PR scope or full main Linux backend formatting, size boundary, clippy, tests, E2E contract; full plans use coverage-checked parallel shards; separate frontend build/tests and model-data generator checks |
| `p0-gateway-gates.yml` | Manual, reusable | Linux listener build, neutral loopback, H1/H2 lifecycle, replay/privacy tests, contract checks, benchmark harness smoke |
| `gateway-core-dedicated.yml` | Manual or reusable | Hosted Linux production stability and socket churn |
| `p0-gateway-final-gates.yml` | Manual | Aggregate the three current hosted workflows |
| `build-release.yml` | Manual, exact dispatch SHA | Builds macOS/Linux candidates independently; only Windows archives require the reusable Windows validation gate; uploads candidate assets and integrity evidence for the existing release publisher |
| `build-release-platform.yml` | Reusable, or explicit manual x64 Windows build | Shared per-target candidate build and evidence upload; the manual defaults build only an NSIS acceptance input with an `awaiting_runtime` identity |
| `windows-compile.yml` | Path-filtered PR or manual | One cached x64 MSVC check of CLI, daemon, platform, local-storage and the console shell, including test targets and transitive production libraries. The shell compile excludes release payload resources; packaging validates the real payload separately. No runtime tests. |
| `windows-headless.yml` | Release-candidate call, explicit manual diagnosis | Native MSVC primitives, shared business libraries, foreground control/Gateway and fixed upstream clients; complete background artifacts with explicit deferred Task acceptance; x64 artifact replay on Windows 11 ARM64 |

PR and main pushes do not automatically run the Windows matrix. During normal
development, use [managed cross-compilation and affected execution on the Windows
development machine](validation-routing.md#windows-iteration-and-release-validation).
The release-candidate workflow calls the local reusable Windows workflow at the
same commit, and only Windows package builds require its success. Unix builds have
no dependency on that gate and can complete if Windows fails or is cancelled. Both native producer
and Windows 11 replay bind `github.sha` and record the `final` phase. This gate
runs before Windows candidate assets are created, not after a GitHub Release is published.
Manual Windows dispatch defaults to `all`, using the same final matrix. Select a
single `scope` for CI-specific diagnosis: it builds and replays only that scope,
records the `focused` phase and cannot establish complete Windows acceptance.
The reusable release entry accepts no scope input and always runs all five scopes.
Neither manual mode publishes packages or establishes the separately required
user/account acceptance.

For installer iteration, manually dispatch **build-release-platform** with its
default x64 Windows target, Windows runner and Desktop option. This uses the same
release builder without starting the other platform or headless assessment jobs.
Its uploaded installer is an acceptance input. The identity remains incomplete
until the required Windows 11 x64 and Server standard-user scenarios pass, and
the release publisher still enforces those artifact-bound results.

Native Windows jobs enable sccache's GitHub Actions backend so compiler objects
can be reused across runs. Cargo targets remain local to each checkout; artifact
source, executable-byte and runtime-import verification still run for every candidate.

Normal Rust checks select the organization `Default` runner group with the
`cncf-ubuntu-16-64-x86` label. The organization must permit this public repository
to use that group. Dedicated stability continues to use `Default Larger Runners`
with `ubuntu-latest-16-cores`. Frontend, selection and result-only jobs use the
standard GitHub-hosted Ubuntu runner. Jobs receive read-only repository permissions,
checkout does not persist credentials, and pull-request jobs receive no repository or
deployment secrets.

An affected PR plan remains one backend shard and executes the exact commands selected by
`test-plan.py`. A full plan expands into eight bounded jobs: formatting/clippy, unit and
binary tests, three ordinary integration-test groups, Gateway E2E, Product smoke, and the
remaining doctest/contract/transport gates. Each Cargo test shard keeps the full workspace
package and all-features context. `ci-shards.py check` compares every non-Desktop Cargo
integration target with the explicit shard inventory; a missing, duplicate or same-name
cross-shard target fails selection instead of being silently omitted. The frontend job
exports its exact candidate console build; the daemon integration shard downloads those
bytes for the installed Linux package scenario. Affected PR plans that run the daemon
package suite also build and download that candidate console. Backend shards keep their configured
Rust runners and depend on the frontend result without rebuilding it there. The required
`backend` status is a small aggregate job, so the public branch-protection contract remains
stable while individual shard jobs can run concurrently.

Pilot manifest inputs, reusable-build tooling and selection-rule changes select
`python3 scripts/test-pilot-builds.py --real-consumer` in Actions; full CI plans
select it as well. This uses the existing selected-tooling executor for tooling-only
PRs, or the affected/full backend gates when Rust product checks are also selected.
Failure blocks the aggregate `backend` result. The explicit entry requires exactly
one executed, unskipped case and a real compiler. Its four bounded subprocess phases
retain exit codes and stdout/stderr under `artifacts/ci/pilot-manifest-consumer/`,
included in the existing CI artifact upload even on failure. The default local
`test-pilot-builds.py` invocation runs fixture-only regressions and never executes
these temporary Rust programs, regardless of compiler availability. The CI case
proves build-script/embedding consumption of the frozen snapshot, not native Desktop,
CPA, provider or installation acceptance.

Changes to `gateway-core.yml`, `ci-run.py`, or `ci-shards.py` force a full plan on the pull
request because those files define the hosted backend execution contract. Changes only to
their tests or documentation keep the ordinary tooling-only selection.

The website release catalog (`apps/website/data/releases.json`) also selects the
`hiroute-host-runtime` library tests and package Clippy in backend CI. The native
upgrade reader consumes this data, so website validation alone cannot cover a new
release. Other website-only edits retain their dedicated workflow.

Most Windows jobs use the standard hosted `windows-2025` image. Native inventories must contain every
required case; zero, ignored or missing required cases fail. The shared library
scope executes the complete ordinary native test inventory. Explicitly registered
installation-input gates and child fixtures have their own execution entries;
unregistered ignored cases fail the ordinary inventory. Fixed Agent gates
explicitly select their opt-in real client tests. Their npm versions and all
transitive package integrities come from `tools/windows-agent-fixtures/package-lock.json`.
Qoder's fixed official archive has an independent SHA-256 check. The ordinary
Agent scope uses the shared required recipe inventory without provider accounts, including
rejection of missing or ambiguous account profiles. Qoder's four
official-account recipes are a separately selected `accounts` scope, requiring
an officially prepared private profile; missing credentials fail that scope.
Public Windows jobs receive no provider credentials. Passing ordinary CI does not
establish official-account acceptance, which remains a separate release gate.

Native x64 Debug checks reuse the Windows Release static CRT configuration from
`build-release.py`, including its CMake toolchain and compiler-override rejection.
Both configuration sources are bound in the captured artifact provenance.
Producer and replay import audits reject ordinary and delayed redistributable VC
runtime DLL dependencies; the hosted image's installed runtime must not conceal
a missing dependency on a clean workstation. Native OS `msvcrt.dll` and UCRT
imports remain allowed. Static imports alone do not prove process launch or
release acceptance; those require their actual runtime gates.

Background jobs build the unchanged pinned CPA with its two native bootstrap
security cases and use the production package generator twice. The administrator
receives a separate verified complete package. Task acceptance exercises the
actual logged-on owner, least-privilege launch and stop/recovery. The
`background-standard-owner` recipe uses the controller's own standard token and
ordinary process creation; it rejects every Administrators group SID, including
deny-only membership. It does not require an administrator controller, alternate
logon, linked token or impersonation. Initial markers, stable entries, definitions
and the SID-derived native Task must be absent before installation. Autostart
policy changes, real login-trigger execution and externally closing a console or
disconnecting/reconnecting RDP require distinct evidence.
The current hosted images execute as the built-in administrator without a linked
limited token. Native inspection found `EnableLUA=1` and administrator consent
prompts disabled; that policy does not give this account a split token. Creating
a standard account with `LogonUser` also does not create an existing interactive
desktop login. These defaults cannot establish the required standard-owner Task
result. The standard-owner gate requires an actual logged-in standard token; the
separate administrator-controlled Task gate also requires its linked limited token.
With the default `-Assessment full`, each selected gate
remains failed when those prerequisites are absent. Read-only RDP service and
port facts alone do not establish such a login or a passing background task.
SCM and logout continuity are optional, outside the required background scope.

The public hosted workflow selects `-Assessment hosted` for the `background`
scope on both machines. This mode requires `-CaptureArtifacts` on the producer
and preserves the complete six-recipe inventory, actual compiler records, PE
verification and full package. Its single source disposition table is in
`windows-native-scopes.ps1`. It executes `background-capture`,
`packaged-proxy-oracle` and `packaged-foreground` (eight current cases). It compiles,
captures and lists `background`, `background-standard-owner` and
`background-standard-upgrade` without running their bodies. The last recipe
requires separately admitted genuine baseline and candidate packages and the
actual standard owner's session; hosted compilation cannot establish upgrade
or rollback. A missing or additional case, compile/capture/PE failure, unknown
recipe or failed executed case fails the hosted assessment.

Deferred results have `scenario=not_assessed`, `executed=0`,
`runtime_executed=false`, `process_exit=null`, and verified compilation, inventory
and capture facts. They do not represent a failed Task prerequisite being turned
into a pass. `validation-feedback.json` records the complete runtime/deferred
partition; even a successful hosted assessment has `scenario=not_assessed`,
`full_scope=false`, `candidate_complete=false` and
`background_acceptance=not_assessed`. Real logged-owner execution and upgrade
remain distinct acceptance gates. A workstation caller selects `full` (the
default) and runs every original gate. Existing focused drivers without an
assessment field retain their selected-case verdicts without a full-scope claim.
The separate candidate upgrade gate uses two distinct, fully verified source revisions
and independently restages the original baseline with fresh administrator trust.
It preserves actual public Source/Key/Plan, Agent restoration, Gateway traffic and
usage receipts across prepared-fault rollback and candidate replacement. A new
Windows platform has no released Windows baseline to invent.

For a separately selected Windows 11 workstation run, `-CaptureArtifacts` records
the exact Cargo `compiler-artifact` executable for every required test recipe,
the product binaries, locked source inputs and complete package bytes. The
`windows-native-replay.ps1` consumer rejects a partial scope, verifies every hash
and source SHA, and uses the same assessment partition without rebuilding Rust.
In `full` mode it runs every captured recipe. It restages
the whole verified package with fresh administrator ACLs before Task/SCM tests.
Static PE import auditing rejects Debug CRT dependencies; actual process launch
still owns DLL resolution and runtime acceptance. The `windows-11-arm` hosted
consumer explicitly records x64 emulation on ARM64, which does not establish
native ARM64 product support. An explicitly selected `windows11-x64` replay also
accepts a real x64 Windows 11 workstation; it checks the actual OS and process
machine and records native x64 separately. This option does not provision a runner
or supply an interactive login. Consumer results retain the producer revision and
have their own runtime scenario evidence.

The shared Linux setup installs build prerequisites through
`scripts/ci-prerequisites.py`. Known Ubuntu direct sources and runner mirror-list
references use the official Ubuntu HTTPS endpoints, retaining suites, components
and signing keys. Package-index refresh fails on any source error and is limited
to three minutes; installation is limited to five minutes. Both stages retain
bounded network retries and fail the job on timeout or package failure, before
Rust validation starts. `scripts/test-ci-setup.py` covers source formats,
mirror indirection and failure propagation without privileged package operations.
Setup changes require the complete hosted matrix.

The toolchain comes from `rust-toolchain.toml`. Cargo sources are cached, not
`target/`; Linux shards do not exchange compiled binaries. Windows replay retains
the original producer build evidence and assesses the consumer separately.
The shared Rust setup exports `CARGO_NET_GIT_FETCH_WITH_CLI=true` for
subsequent job steps, so pinned Git dependencies use the installed Git client
instead of Cargo's embedded libgit2 transport. This avoids the libgit2 SSL failure
observed when fetching Pingora on a cold hosted runner; TLS verification and locked
dependency revisions remain enforced. Fetch failures still fail the job, and the
setup does not retry test commands.
Build concurrency defaults to the runner CPU count; incremental compilation is
disabled. Every job keeps its own checkout-local Cargo output. The hosted job is
disposable, so there is no persistent worktree cleanup service.

### Rust test concurrency

Compiler jobs and host CPU count do not describe effective test concurrency.
Full hosted unit/binary, integration and doctest bundles use the `hosted-default` policy:
they pass no `--test-threads` override, including for CPA bridge unit tests and
process-backed CLI, daemon and Gateway targets. Libtest can select its default or
consume `RUST_TEST_THREADS` from the launch environment. The report retains these
settings separately and leaves effective runtime concurrency explicitly unknown.

This is an explicit hosted exception to the focused/SSH process-target one-thread
policy and the workspace DAG's per-target bounds. The hosted bundles retain their
existing workspace/all-features graph and complete target inventory; the runner
does not split those bundles or silently insert a focused target's bound. Affected
plans and explicit gates retain their selected commands, including any explicit
harness bound, under `selected-command`. Shard planning tests cover both policies.

Use these recorded differences when comparing a focused pass with a hosted failure.
A later pass does not establish that concurrency caused an earlier failure.
Changing the hosted bounds requires separate execution evidence; neither a retry
nor this diagnostic metadata changes production deadlines, assertions or the
meaning of an earlier failure.

## Evidence and failures

Each invocation checks the full expected SHA against HEAD and rejects tracked
source changes. On pull requests this is GitHub's checked-out test commit, not
an inferred branch tip. The wrapper waits synchronously and returns failure
on process failure, timeout, revision mismatch, or a required Rust test invocation
with no passing tests. It preserves the child process exit separately from its
own decision. `scenario_state: not_assessed` is intentional: process success does
not establish product scenario acceptance.

`artifacts/ci/<name>/` contains the complete command log and JSON result, including
revision, CPU count, platform, timestamps, and Cargo/Rust versions for Cargo
commands. Rust test results and derived reports also retain `harness_concurrency`:
the requested thread arguments (an empty list means omitted), the launch
`RUST_TEST_THREADS` value and whether that environment was observed, the configured
limit/source when unambiguous, the applicable policy, and an unknown effective
thread count. A configured limit is not measured active concurrency: Cargo config,
custom harnesses and test-created threads may change runtime behavior. Older or
non-hosted records without an environment snapshot remain unknown rather than
being reconstructed from today's environment or CPU count.

Failure prints the log tail; Actions uploads available artifacts even
when a check fails. Each shard uses a distinct artifact name; the aggregate job does not
replace those per-command records. Timeout and cancellation stop the command process group
before removing its private short TMPDIR. A forcibly terminated VM cannot be
expected to finish uploading artifacts.

The adapter can be tested without Rust compilation or remote access:

```sh
python3 scripts/test-ci-run.py
python3 scripts/test-ci-shards.py
python3 scripts/ci-shards.py check
python3 scripts/test-windows-ci-report.py
```

## Coverage boundaries

The fixed-machine benchmark and semantic result-checking scripts remain available for
maintainer validation; hosted benchmark smoke is not fixed-machine performance evidence.

The hosted Rust checks exclude `hiroute-desktop`. Frontend build/tests do not
prove native Tauri interaction. macOS Desktop remains on the existing local
validation path. The separate [release candidate workflow](release-builds.md)
builds native macOS, Linux and Windows packages on both architectures. Building
an archive records package integrity and an awaiting-runtime state; it does not
replace that architecture's native business acceptance or Desktop validation.
Real-provider credentials and separately provisioned production E2E fixtures are
not supplied by this CI setup. Existing failing checks remain failures, not
expected-red success or waived gates.

Tooling-only pull requests run their selected Python checks in the selection job and retain
CI reports. Contributors can run `python3 scripts/test-plan.py --base origin/main` to inspect
the affected command set before opening a pull request.
