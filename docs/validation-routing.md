# Build and test locations

The caller's `~/.config/hiroute/validation.json` selects execution locations.
Locations are personal configuration, never repository requirements. `backend`,
`desktop`, and `frontend` are independent targets; Pilot always follows `desktop`.
Linux can build backend code locally while macOS Desktop/Pilot runs over SSH.
A Mac can keep Desktop local and send backend commands to its Linux workbench.

Use `python3 scripts/validation.py show` to inspect routing and
`python3 scripts/validation.py doctor desktop` for read-only host/tool/GUI checks.
No configuration means local backend/frontend on Linux; on macOS, an existing
`remote-rust.json` keeps selecting the SSH Linux backend. Desktop defaults local
only on macOS; other hosts must configure a macOS target explicitly. Invalid
configuration or unavailable hosts fail rather than silently changing locations.

Example for a Linux caller (replace the SSH alias with your personal alias):

```json
{
  "version": 1,
  "targets": {
    "backend": {"transport": "local", "jobs": 20},
    "frontend": {"transport": "local"},
    "desktop": {
      "transport": "ssh",
      "host": "my-mac",
      "repo": "~/git/HiRoute",
      "path": ["~/.cargo/bin", "/opt/homebrew/bin", "~/.local/bin"],
      "pilot_cli": "~/my-private-pilot-tools/bin/tauri-pilot"
    }
  }
}
```

On macOS, use `desktop: {"transport": "local"}` and
`backend: {"transport": "ssh", "host": "my-linux"}`. The SSH backend uses the
existing `remote-rust.py` workbench; its checkout, capacity, committed driver,
retention and pushed-candidate rules remain unchanged. Select its HOME-relative
`workbench` in `remote-rust.json`; direct runner commands can override it with
`--workbench` to read old runs. Repositories with different origins use distinct
checkouts while sharing the same account-wide compiler capacity locks. `repo`/`path`/`pilot_cli`
are interpreted on the execution host, never expanded using the caller's HOME.
The wrapper sends argument arrays as JSON, not interpolated shell commands.

The submitted candidate and the validation driver have separate revision identities.
The driver uses committed bytes at the caller's HEAD; `--sha` selects the candidate
from the advertised branch. Both revisions must satisfy Git admission. A tooling-only
driver repair can validate an unchanged candidate without treating new source as old
runtime evidence. Related runs remain in their selected workbench; another origin
or workbench does not import their results.

Windows development execution uses `validation.py windows` below; hosted CI retains
its [own execution entry](github-actions-validation.md).
Linux backend success does not establish Windows runtime, real account, or logged-on
Task acceptance. Use the corresponding native environment and record the actual OS
and executed cases.

## Commands

```sh
python3 scripts/validation.py backend run --ref refs/heads/codex/my-change -- \
  cargo test --locked -p hiroute-daemon
python3 scripts/validation.py frontend build
python3 scripts/validation.py frontend test
python3 scripts/validation.py --frontend-dist /absolute/mac/candidate/apps/desktop/dist \
  desktop run --ref refs/heads/codex/my-change \
  --cargo-only -- cargo check --locked -p hiroute-desktop --features desktop-pilot --all-targets
```

`run` defaults to the **caller's** HEAD, including over SSH. Local backend runs
use `local-rust.py --source local`, select committed source into a fresh worktree,
and need no push; dirty working files are not included. SSH/native Desktop
validation retains origin-fetch validation and needs the selected commit pushed.
Local backend checks imply `--cargo-only`; Desktop preserves the existing explicit
scenario assessment. `jobs` applies to local-rust execution and defaults to 2;
SSH backend workbench retains its existing 8/64-job capacity policy.

For a busy local backend, pass `backend run --wait-for-lock` to wait on the existing
exclusive host lock inside the runner. Local-rust Desktop runs support the same
option; the SSH backend workbench keeps its own queue. Cleanup still fails fast
when another owner is active. Waiting never starts a second compiler or changes
the exact candidate, default target directory or evidence rules.

`desktop run --help` derives its admitted options from the caller's runner parser,
omitting `--source`, `--repo` and `--jobs`. Configure repository and compiler jobs
in `validation.json`; full or abbreviated overrides are rejected, with a
source-specific reason for source overrides.
Desktop always fetches from origin, even on a local Mac, so push the exact candidate
after public-content checks and omit `--source`. Local backend help follows the
same wrapper-owned-option rule and retains local committed-source selection.
The `--sha` help reflects the caller-HEAD default; `--wait-for-lock` remains available.
Help does not contact the execution host or require its checkout to be updated.

Arguments after `backend`/`desktop` otherwise follow the selected runner's CLI.
SSH backend returns a queued run ID and supports `wait/status/logs/cleanup/gc`.
Both local and SSH ordinary runs save reports and private evidence, then reclaim
their managed checkout and temporary directory after success, failure, timeout or
pre-command block. Use local `--keep` or SSH `--retention always` when subsequent
steps need those binaries; SSH `--retention on-failure` explicitly retains failures.
Desktop/Pilot acquisition retains its build until the registered owners finish.
Use `backend wait RUN_ID --timeout 50` for one compact terminal or timeout
observation; repeat a bounded wait when necessary, and inspect full status/logs
for failures or specific evidence. A wait timeout does not change the run state.
Local-rust (including Desktop over SSH) runs in the foreground, prints its result,
and supports `finish/preview/apply/keep/release/retire/gc`. Its logs stay beside its
result on that host. SSH interruption is not proof of completion: inspect that
host's run record before retrying. This wrapper adds no queue for Mac builds.
Existing direct runner commands remain available.

### Windows iteration and release validation

#### Development-machine execution

The unified entry is `python3 scripts/validation.py windows`. It reads
`~/.config/hiroute/windows-validation.json` (mode `0600`, outside the checkout).
The configuration contains machine locations, never provider tokens. Use an existing
SSH key and a dedicated already logged-on Windows acceptance account. The runner
neither creates accounts nor logs users in, restarts Windows or changes RDP.

```json
{
  "version": 1,
  "host": "windows-dev",
  "identity_file": "~/.ssh/id_ed25519",
  "python": "C:/Validation/python/python.exe",
  "root": "C:/Validation/hiroute-runs",
  "user": "acceptance",
  "session_id": 2,
  "environment": {
    "HIROUTE_WINDOWS_SOURCE_ROOT": "C:/Validation/source",
    "HIROUTE_WORKER_NODE": "C:/Validation/node/node.exe",
    "HIROUTE_WORKER_CODEX_BINARY": "C:/Validation/clients/codex.exe",
    "HIROUTE_WORKER_CLAUDE_BINARY": "C:/Validation/clients/claude.exe",
    "HIROUTE_WORKER_PI_BINARY": "C:/Validation/clients/pi/dist/bundle/cli.js",
    "HIROUTE_WORKER_DSH_BINARY": "C:/Validation/clients/dsh/lib/bin.js",
    "HIROUTE_WORKER_CODEX_ACP_ADAPTER": "C:/Validation/clients/codex-acp/dist/index.js",
    "HIROUTE_WORKER_CLAUDE_ACP_ADAPTER": "C:/Validation/clients/claude-agent-acp/dist/index.js"
  },
  "path": ["C:/Validation/node", "C:/Validation/clients", "C:/Validation/git/bin", "C:/Validation/pwsh"]
}
```

Replace example locations with installed, pinned clients. The source root contains
matching `tools/windows-agent-fixtures` declarations, lockfile and installed npm
modules. `doctor` reports the actual OS and existing sessions; select the intended
session in HOME rather than silently falling back to SSH service context. An
administrator session is not standard-user evidence. The configured account must
be able to read the bundles and register a task for its existing login session.
Use a separate workbench `root` for each native account. Its private evidence
directory must grant that account inheritable access; the runner rejects an
inaccessible directory before creating a Task and preserves its existing ACL.

```sh
python3 scripts/validation.py windows doctor
# Export on the cross-build host before continuing the managed checkout.
python3 scripts/validation.py windows bundle --result /path/to/run/result.json \
  --log /path/to/run/command.log --output /path/to/development-bundle.zip
python3 scripts/validation.py windows stage /path/to/development-bundle.zip
python3 scripts/validation.py windows inspect-installation --product PRODUCT_BUNDLE_ID
python3 scripts/validation.py windows run --harness HARNESS_BUNDLE_ID \
  --product PRODUCT_BUNDLE_ID --binary windows_headless.exe \
  --case actual_windows_complete_public_headless_business --ignored
python3 scripts/validation.py windows wait RUN_ID --timeout 50
python3 scripts/validation.py windows logs RUN_ID --tail 60
python3 scripts/validation.py windows collect RUN_ID --output artifacts/windows-dev/RUN_ID
python3 scripts/validation.py windows finish RUN_ID
```

For an installed NSIS candidate, `windows desktop-session --product BUNDLE_ID
--action launch` opens the measured installed shell in that same session.
`--action snapshot` collects its native Windows UI Automation tree and a window
screenshot. `invoke`/`text` require an exact Runtime ID from a recent successful
`--observation-run`; changed processes, elements and image digests are refused.
Use the observed UI for product actions; this entry does not inject page scripts
or replace the WebView with HTTP assertions. `close` uses the native window-close
pattern. Collect and finish each bounded operation before the next one.
Desktop operations use the configured Python runtime's `pythonw.exe` sibling and
hidden console children so the observer cannot obscure the product window. A
native accessibility hit test supplements incomplete WebView fragment traversal.
If needed, `click` requires coordinates from a recent screenshot of that exact
foreground window; it refuses changed geometry. `native-files` captures a bounded,
read-only Codex/Claude configuration and collaboration Skill inventory for restore
comparisons, including absence before a product-created file.
`quit` uses the visible HiRoute notification icon and the measured process's
native Quit menu item, then requires that process to exit. It does not kill a
process or stop the background service; an unavailable or ambiguous icon/menu
is an observation failure that must be inspected before retrying.

`--action install` runs the measured NSIS `/S` entry. Its specific 720-second
whole-installer watchdog includes the product adapter's 600-second install request
and NSIS extraction/hooks; CLI/UI watchdogs and product request deadlines stay
unchanged. `installer-process.json` records elapsed time, bounded transaction
phase changes and the actual installer exit. A committed marker without that exit
does not pass installation. On timeout inspect the retained transaction and any
remaining product children before recovery or another attempt. For a different installed
candidate, `--from-product OLD_BUNDLE_ID` verifies the old images before invoking
the new installer. Preserve existing data and identify both packages in the
result. Different source revisions carrying the same semantic version prove a
cross-build replacement, not a newer-version catalog selection or N-to-N+1 update.
`--action cli --cli-args PRIVATE_JSON_FILE` executes a bounded argument array
through the installed public CLI for preparation and effect readback; it does
not establish a UI action. The selected standard account must have no
Administrators SID, including deny-only membership. All native observations and
screenshots remain in ignored, private evidence.
`native-files` captures the fixed Codex/Claude configuration files and inventories
both collaboration and management Skill directories for restoration readback.
It also fingerprints Codex sessions/history without copying their contents; compare
immediately around installation/restoration, with no Codex call in between.
`scroll` uses a recent screenshot's window geometry and pointer coordinates, plus
an explicit `--scroll-direction up|down`; it sends a bounded native mouse wheel
event to that foreground window. `codex-call` runs the configured real Codex CLI
with a fixed `Get-Location` tool prompt, a read-only sandbox and no approvals. It
requires saved configuration selecting the local HiRoute Gateway and rejects
authentication/routing environment overrides. Use `--codex-profile hiroute` for
the product-owned standalone `hiroute.config.toml`, or omit it for the default
`config.toml`. The native inventory records both files independently; root config
preservation is expected when using the standalone profile. This optional
scenario needs Python 3.11+ and verifies the completed tool output and nonce-bearing
answer; the completed command must be `Get-Location` and report the actual run
directory. Process exit alone is insufficient. Preserve its actual client image hash,
product revision and native output. It does not alter sandbox configuration or
substitute a model fixture for a real upstream.
`codex-inference` is a separate pure model-call scenario using the same saved
configuration, client identity and sandbox. It requests no tools and requires a
completed turn with the exact per-run answer. It establishes no native tool
execution and cannot replace or waive a failed `codex-call` result.
`codex-two-turn --codex-profile hiroute` starts a new native session, then resumes
the exact client-reported session with a separate feedback prompt in the same
temporary working directory. Each turn retains its own stdout/stderr and must
complete with its own exact answer; the helper never resumes an unproved ID. This
establishes two native turns, not classification or cross-turn assessment: check
those actual Gateway and observation records separately. Each call keeps the
existing 150-second subprocess bound; the outer task covers both calls.
The runner records the actual client version before the call and preserves partial
stdout/stderr when its bounded wait expires. Such a timeout is a failed attempt,
not an inference verdict. The bounded public-CLI action also accepts `sessions`
for product receipt readback; protected content still uses the product's authority.
For a connection failure, `network-probe --source-id SOURCE` resolves one ready
saved HTTPS source through the installed CLI and performs a bounded, verified TLS
handshake in the same account. It sends no HTTP request or credentials, changes no
trust settings, and records the certificate-store count, TLS result and error.
This is diagnostics only; it cannot establish a product model call.
`--native-tls` compares Windows `SslStream` verification under the existing system
trust policy. Normal Windows certificate-chain caching may occur; the probe does
not import a trust anchor or override verification.
For a failed NSIS attempt with a prepared shell transaction, `installer-resume`
captures the actual post-install adapter's output. It requires the matching
recent inspection in `--observation-run`, the exact retained `--from-product`,
the measured replacement shell/CLI/daemon and no running shell. It resumes the
normal Rust installation entry with the captured login/tray defaults; it does
not force a stop or bypass restoration. This mutates the prepared installation
and is diagnostic evidence, never a successful full NSIS result. Inspect its
output and resulting transaction before any further installation attempt.
If that failed unreleased producer never registered its generated uninstaller,
manual `installer-quarantine` can preserve that one file in the run's evidence.
It requires the same inspection/prior-product admission and an independently
extracted digest from the verified failed NSIS artifact in
`--generated-uninstaller-sha256`. A changed file, missing verified old backup or
already registered replacement is refused. This changes no journal or business
data and is manual preparation, not a compatibility reader or an installation
result; the Rust transaction still owns all restoration and installation.
To test a normal NSIS retry over that prepared replacement, `install` accepts
`--recover-product INTERRUPTED_BUNDLE` with `--from-product RETAINED_BUNDLE` and
the recent matching `--observation-run`. It verifies both retained and interrupted
product images before launching the new installer. Recovery itself remains the
normal NSIS/Rust path; this admission does not repair files or journals.
Before native input, the runner checks that the existing session's input desktop
is accessible and matches its ordinary desktop. `--action desktop-state` reads
that prerequisite without unlocking or creating a login. UI observations require
the measured window to be foreground and use only its visible native controls.
Content-addressed validator helpers are verified and published once by atomic
rename. Wait/status/collection never overwrite helper files that a running
standard-user task may be importing.

The generic complete-source, matching-program and native-state recovery design
remains in production. A Windows development-build historical upgrade/restore
matrix is not required first-release acceptance and does not establish a supported
released predecessor. Preserve prior simulation evidence; do not mutate an
existing installation to extend that deferred matrix.

`recovery-lock-ui` requires a recent inspection of the currently installed product
and an accessible existing input desktop. It starts from actual Gateway readiness,
stops the owned service, holds only an owned startup-lock file handle and opens the
real bundled failure page. It observes both languages through the visible language
button, releases its handle, quits and reopens the same installed Desktop, then
checks actual Control/Gateway readiness. It never corrupts databases, changes keys,
manufactures recovery reports or launches an older program. Its screenshots and
native UI observations remain private evidence, separate from fixture parser tests.
`recovery-file-use-ui` uses the same bounded sequence against the current
Control file using only the admitted current installation. Both lock cases require offline current-version
retry advice rather than borrowing a retained source as a reason to restore old data.

Stage separate product and harness bundles when reusing an unchanged product for
a test-only repair. Results record both full revisions, binary hashes, driver hash,
client hashes, actual OS/session/elevation and exactly one test outcome. Zero exit
without the selected passing case is failure. Archive entries must be flat,
manifested x64 executables with matching size/hash; partial stages cannot be reused.
Cross-built artifacts never become native CI producers.

The four `hiroute_local_storage.exe` cases in
`control::windows_artifacts_tests` use only unique temporary directories. The runner
admits these exact non-ignored cases alongside an existing installation and records
`temporary_artifact_component`; it does not collect that installation's diagnostics.
Other test cases retain the fresh-profile requirement. Temporary ACL/recovery checks
prove native file behavior, not installed-product acceptance.

`inspect-installation` runs a read-only inventory in the configured account. Collect
and finish it normally; its success is inspection evidence, not a product scenario.
When the user explicitly authorizes clearing old validation data, use
`windows archive-installation-data INSPECTION_RUN_ID --product PRODUCT_BUNDLE_ID`.
It requires a recent matching standard-account inspection, no installed product,
no product processes/tasks and no native Agent references to the old data. It moves
only the three fixed HiRoute data, CLI state and shell state directories into private run evidence, with
complete before/after hashes; native settings remain unchanged. Installed products
must use their normal uninstall entry first. For the exact measured candidate,
`windows uninstall-candidate INSPECTION_RUN_ID --product PRODUCT_BUNDLE_ID
--shell-sha256 MEASURED_SHELL_SHA` invokes its actual NSIS `/S` entry, preserving
data. It pins the installed CLI/daemon, shell and uninstaller inventory, rejects
other active work, and waits for the owned installation to disappear. It cannot
force an unavailable restore or remove business data. It copies the verified
uninstaller outside the program directory and uses NSIS's final `_?=` argument
to observe the actual uninstall process and exit code instead of its detached
launcher. `--adapter-diagnostic` is a separate **mutating** diagnostic: it runs a
verified shell copy's existing uninstall adapter and retains its typed result;
it does not run the rest of NSIS or establish complete uninstall success.
Read-only probing remains `--probe-only` and cannot be combined with that option.
These operations create no account. A preparation record alone establishes no
upgrade, Agent restoration or migration acceptance; business acceptance also
requires actual before/after Agent files, Operation/status and package identity.

For the NSIS candidate, manually dispatch `build-release-platform.yml` with its
default x64 Windows/Desktop options. Its `artifacts/windows-desktop` upload
contains a `development-bundle.zip` made from the measured installer, final shell image hash, embedded
product binaries and exact compiled installer test. The explicitly declared NSIS
entry has an x86 bootstrap stub; ordinary product and test images remain x64.
Stage this bundle with the same `windows stage` entry, then select:

```sh
python3 scripts/validation.py windows run --harness BUNDLE_ID --product BUNDLE_ID \
  --binary windows_desktop.exe \
  --case actual_nsis_silent_install_repeat_console_and_uninstall_preserve_data --ignored
```

This bounded case requires a fresh profile in the configured existing standard
session, including an absent or empty versioned-package base. Retained payloads
are refused before installer invocation so backlog cleanup cannot consume the
bounded fresh-install scenario. It rejects even deny-only Administrators membership, and uses the actual
NSIS `/S` entry for installation, repeat installation and uninstall. It verifies
default login/tray choices, production Control/Gateway and packaged console
readiness, and data preservation. Its result explicitly leaves WebView interaction,
Agent restoration and update recovery unassessed. Preserved standalone data stays
in that test profile after uninstall; `finish` does not remove it. The complete
[Windows Desktop inventory](../scripts/windows-desktop-cases.json) retains its
separate native obligations and artifact-bound release gate.

Execution uses a one-off `Interactive`/`Limited` Task, with no startup trigger or
saved password. One workbench lock prevents overlapping product mutation. Product
profile and request deadlines remain intact; `run --timeout` is only a whole-test
watchdog. Existing installations cause prerequisite failure, not automatic deletion.
After failure, inspect logs/diagnostics and apply the product recovery procedure
to only test-owned configuration before retrying. `finish` verifies the stopped
task action and removes only that task and owner lock; it keeps evidence, bundles
and native configuration. Collected output is private raw evidence and must stay
ignored, not be attached to public records unreviewed.

When Claude acceptance is explicitly excluded for a development run, select
`actual_windows_public_headless_business_without_claude` instead. It retains the
same routing, observation, Worker, restart and restore assertions; Pi supplies the
independent Messages grant. It does not establish native Claude authentication,
managed launch or Claude configuration recovery, and does not replace the complete
release case. No product request deadline is changed.

For a failed disposable business case whose product has already been removed,
`windows recover RUN_ID` can preserve recognized test-owned Codex configuration,
collaboration Skill and untouched Claude marker in that run's private evidence.
First collect and `finish` the run. Recovery rejects active tasks, existing products,
foreign content, links and file modification times outside that run. It never deletes
unrecognized user settings; use public restore while the original product still exists.

If the focused foreground Gateway recovery case hits its whole-process watchdog,
the same `recover` entry can retain its disposable installation and state in private
run evidence. It requires the stopped task, original run marker, matching bundle
hashes and stable hard links. Any orphaned daemon is stopped through a retained
kernel handle only after its image, session and birth time match that run; other
live fixture processes block recovery. This path never recovers a daily installation.

For an active disposable case that has already emitted its exact Rust panic,
`windows stop-runtime RUN_ID` can stop only its verified daemon and let the test
owner finish unwinding and preserve its real failed result. It retains the owner,
Task and lock; collect and finish normally afterward. It refuses healthy tests,
foreign accounts, changed images and daemon births outside the run.

Transport uses SSH host-key verification, hashed uploaded scripts/JSON and encoded
PowerShell invocation. Interrupted submission is an unknown outcome: inspect its
run/task/lock before retrying. Missing Python, clients, login session or account
access is a prerequisite gap, not permission to substitute a different identity.
Large bootstrap scripts use a bounded encoded bridge with source passed through
stdin, so publishing a new helper cannot exceed the Windows SSH shell's command
length limit. Helper hash checks and atomic publication still apply.

#### Compilation and release boundary

Ordinary development uses the configured Linux workbench for Windows MSVC
cross-compilation and the configured Windows development machine for affected
native execution. The expensive hosted Windows producer/replay matrix runs from
`build-release.yml` before release assets can build; `windows-headless.yml` also
retains an explicit manual entry for diagnosis. PR and main pushes retain their
ordinary Linux/frontend selection without automatically starting that matrix.
Affected PRs also run one cached Windows headless compile check; it runs no
business or account acceptance.
That job also runs only the filtered package/journal/restoration-policy component
fixtures from `hiroute-host-runtime`. It records OS/build, architecture and token
elevation; fixture success never satisfies native installation or standard-user
acceptance. The full native matrix remains release/manual-only.

1. Record the comparison base, committed candidate, affected consumers and native
   cases from `test-plan.py --integration`. Run affected Linux tests as selected;
   compile affected macOS consumers on the configured Mac when their contracts
   change. Cross-compiling Windows does not validate either Unix runtime.
2. Use the managed backend runner with a nonempty plan and retained checkout.
   The Linux host must already have the matching Rust target, MSVC-compatible
   linker and Windows SDK/import libraries in its personal toolchain configuration.
   Record their versions and compiler flags. Keep SDK paths and remote host details
   outside the repository. An unavailable toolchain is a build prerequisite gap,
   not permission to substitute a GNU target for the supported MSVC target.
   For release-equivalent x64 builds, preserve the static CRT contract owned by
   `build-release.py`; a development compiler configuration is not release evidence.

   ```sh
   python3 scripts/validation.py backend run \
     --ref refs/heads/codex/my-change --sha CANDIDATE_SHA \
     --plan windows-my-change --phase focused --retention always -- \
     cargo build --locked --target x86_64-pc-windows-msvc \
     -p hiroute-cli -p hiroute-daemon --bins --message-format=json
   python3 scripts/validation.py backend run \
     --ref refs/heads/codex/my-change --sha CANDIDATE_SHA \
     --plan windows-my-change --phase focused --retention always \
     --continue-checkout LAST_STOPPED_RUN_ID -- \
     cargo test --locked --target x86_64-pc-windows-msvc \
     -p hiroute-daemon --test windows_headless --no-run --message-format=json
   ```

   Replace the example package/test selection with the affected inventory and
   feature context. Continue from the last stopped run after each repair; preserve
   its default `target/`, locks and cleanup owner. `--no-run` proves compilation
   and linking only. A configured target runner must not silently execute via Wine.
3. Export the exact Cargo `compiler-artifact` paths from the successful JSON output,
   with the source SHA, driver SHA, target/features, compiler configuration, build
   run IDs and SHA-256/size of every transferred EXE and fixture. Verify PE target
   identity and recheck hashes after transfer. Use an isolated Windows staging
   directory; do not select files by modification time or a glob over old builds.
   Cross-built bundles are development evidence. The native artifact producer
   deliberately requires a native Windows rustc host; do not rewrite that provenance
   or feed a cross-built bundle into the native Windows 11 replay as if it were native.
4. Execute only the affected test bodies and business paths on the Windows machine.
   Use the [Windows scope inventory](../scripts/windows-native-scopes.ps1) and
   [operating guide](standalone-windows.md) to identify required fixtures and owner
   context. Installation/background/upgrade changes require the real installed
   package and logged-on standard user; provider-account cases require an officially
   prepared account. Reuse the same verified binaries across cases. A Server host
   result records Server, not Windows 11; x64 execution does not prove native ARM64.
5. Keep the original failed run and targeted repair result. At convergence, report
   each required case's actual execution, source/artifact identity, OS/architecture,
   user context and evidence. Reuse unaffected evidence only at its original SHA
   with an explanation. Do not add RDP connectivity or real next-login exercises
   unless the changed behavior or agreed acceptance scope requires them.

At release, freeze the source and require the complete release-candidate workflow
to pass for that same SHA. Hosted background compilation still leaves the documented
logged-on-user and real-account gates separate; the release matrix does not turn
them into runtime passes. A missing development capability requires an explicit
capable environment or remains an unexecuted obligation. It must not be hidden by
waiting until release or reporting compilation as complete acceptance.

### Native consumer compilation

The plan's `native_checks` contains a configured-Mac `command_template` for known
Desktop consumers, including shared Worker request and launch contracts. Fill its
required branch ref, exact candidate SHA, same-candidate built frontend directory
on the Mac and feature plan before executing; the placeholders are not executable
arguments. Run it after those contracts integrate,
before expensive native journeys. Add applicable `--related-run` options before
the `--` boundary and use `--phase final` for final validation. SSH requires the
candidate pushed on the specified branch.

`hiroute-desktop` has no default runtime feature. A featureless `cargo check` or
library test does not compile the Tauri bridge. The managed `--frontend-dist`
procedure uses `desktop-pilot.py config`, including the `pilot:default` capability;
enable `desktop-pilot` to supply that plugin's ACL manifest. It includes
`desktop-runtime`, so `--all-targets` still checks both production and test
consumers without executing them. Runtime-only features do not match this Pilot
configuration. Missing Mac/configuration remains an unexecuted obligation, not a
reason to run the command on Linux.

A successful same-candidate Pilot build includes the production bridge through
`desktop-pilot`, but ordinary `cargo build` does not compile its test targets.
Record feature/target coverage before reusing evidence; frontend/mock IPC tests
and backend builds cannot substitute for this compile gate. Native business
acceptance remains a separate obligation.

### Reuse one retained backend checkout for product smoke

For repeated validation on the same pushed SHA, first run with `--retention always`.
After it stops, submit another command with `--reuse-checkout FIRST_RUN_ID`, the
same `--sha` and branch, and the same build-slot class. The workbench checks the owner
run, SHA, origin, committed executor, clean worktree and Git registration before using
its default `target/`. A checkout lease serializes reuse and prevents owner cleanup
while a borrower runs. Borrower cleanup keeps the owner checkout; after borrowers
finish, `backend cleanup FIRST_RUN_ID` releases it. Owner cleanup preserves each
run's original export and writes current evidence to an independent cleanup
generation, even when the SHA has not changed. Its manifest records the actual
checkout revision and latest stopped run. Inspect `cleanup.evidence_export` for
that capture; see [retention and cleanup](remote-rust-validation.md#checkout-retention-and-cleanup).
Refused explicit cleanup returns nonzero and retains the owner resources.

Same-candidate reuse remains available after a non-timeout test failure, including
when no `--plan` was supplied or the original `on-failure` owner stopped at a
recoverable preparation block. Each retry creates a new result; the failed result
keeps its original verdict.

```sh
python3 scripts/validation.py backend run --ref refs/heads/codex/my-change \
  --retention always --plan my-change --phase focused -- \
  cargo test --locked -p hiroute-product-e2e --test smoke_cli default_two_domain_smoke -- --exact
# Read its run ID and confirm completion before submitting the next command.
python3 scripts/validation.py backend run --ref refs/heads/codex/my-change \
  --reuse-checkout FIRST_RUN_ID --plan my-change --phase final \
  --related-run FIRST_RUN_ID -- \
  cargo test --locked -p hiroute-product-e2e --test smoke_cli default_two_domain_smoke -- --exact
```

This is an explicit workflow for exact-candidate repeats; ordinary backend runs
still get separate checkouts. Reuse is refused after source, driver or checkout
changes. Product smoke checks the cached recipe and binary digest, keeping per-run
runtime directories, challenges, process evidence and verdicts separate. Gateway
and Control have distinct staged `hirouted` binaries under that checkout's `target/`.
The first build remains necessary. Compare actual cold/warm timing in the reports.

### Continue an integration checkout and schedule the final workspace

Start the integration's first backend command with `--retention always` and a
nonempty `--plan`. For each subsequent command, use `--continue-checkout LAST_RUN_ID`
and the same plan/branch. The source may advance to a descendant committed SHA;
the runner retains the checkout's own target while validating current source and
recording a new result. Continue only after the previous worker stops. Stale run
IDs, dirty source, changed executor implementation and a different plan are rejected.
The exact latest run may also be a recoverable pre-command console-assets or cache
preflight block. Prepare the missing input or repair the cache using its diagnostic,
then execute the generated recovery command from the same committed driver. Recovery
creates a new run; the blocked result and all earlier source revisions remain intact.
Cleanup belongs to the first checkout owner, after all commands finish. Keep each
run's original SHA when citing evidence; cached artifacts are not scenario evidence.

For the frozen final full backend candidate, use the bounded DAG schedule:

```sh
python3 scripts/validation.py backend run --ref refs/heads/codex/my-change \
  --continue-checkout LAST_RUN_ID --schedule workspace-dag --exclusive \
  --plan my-change --phase final --related-run LAST_RUN_ID -- \
  cargo test --locked --workspace --exclude hiroute-desktop --all-features
```

The DAG reserves all eight workbench slots through checkout, compilation and
execution. It compiles the exact workspace selection once, lists every compiled
test target, records each executable's digest and checks the case-name set against
its terminal result. The smoke test executable is copied before its preparation
command can relink it. Other test executables are checked before and after use;
a changed artifact fails the run. The checkout keeps its default `target/`.

The CPA bridge library runs first on its own with one test thread. Its external
process fixtures release a selected loopback port before the child binds it;
overlapping listeners can take that port and receive a plaintext readiness probe.
This isolation keeps every listed case and the same executable digest/audit checks.
It does not change the production port handoff. A failed isolated target follows
the same default fail-fast rule before preparation or other runtime targets start.

One Cargo writer then prepares the attested CLI, ordinary daemon and Gateway roles,
then separately builds the feature-qualified daemon required by publication crash
recovery tests. Normal tests run in a bounded four-target pool while products are
prepared. Product process targets wait for the correct role; the three prepared
smoke cases use independent private runtimes. Doctests run after the other tasks
finish. Missing or changed artifacts and missing listed cases fail rather than
falling back to a runtime Cargo build. Default failure handling stops admitting
new targets; `--no-fail-fast` explicitly collects the whole selected scope.

The preparation report binds exact product and Gateway receipt paths and their
file digests. Admission verifies only those selected receipts, preserving their
attested candidate, executable identity, checkout containment and Cargo runtime
environment. Different recipes (including different compiler job counts) can
retain identical executable bytes in the same checkout; cache-wide digest
uniqueness is not required. Missing or changed selected evidence still fails.
If preparation fails, completed independent-target results remain in the aggregate,
with the unexecuted target inventory and doctest admission explicitly recorded.

The run result distinguishes correctness from the five-minute performance target.
`queue_seconds`, `execution_seconds` and `end_to_end_seconds` use the workbench's
monotonic clock; execution starts when capacity is acquired and includes checkout,
builds, tests, cleanup and reporting. Phase durations overlap and must not be
summed. A new checkout, a retained checkout with another candidate SHA, and a
same-SHA repeat are separate performance conditions.

Both `workspace-dag` and the older `workspace-smoke` schedule accept only the exact
command above, optionally with `--no-fail-fast`. `workspace-smoke` remains an
explicit rollback path and is never run automatically after a DAG failure. Other
selectors, profiles and filters use the ordinary runner. A bridge over the old
three-slot locks keeps legacy and eight-slot workers mutually exclusive.

The installed Linux management, Quickstart and model-feed scenarios require the same
candidate's production frontend at `apps/desktop/dist`. The SSH backend inspects the
Cargo/libtest selection and checks this prerequisite before Cargo or sccache starts.
Missing receipts, another source SHA, missing/extra files and changed hashes block
the run with `blocker=console_assets`, a null process exit and preparation commands
in `result.console_assets.preparation`. Pure library checks, compilation, listing,
other targets and filters excluding these cases do not require console assets.

Use the exact clean committed candidate on the configured frontend target:

```sh
python3 scripts/validation.py frontend ci
python3 scripts/validation.py frontend build --console-assets
```

Copy the resulting dist, including its receipt, with standard file tools to a
separate directory on the configured backend host. For a local frontend target,
using your configured SSH alias and a fresh staging directory:

```sh
backend_host=backend
staging_dir=.cache/hiroute/console-assets/CANDIDATE_SHA-BLOCKED_RUN_ID
ssh "$backend_host" "mkdir -p '$staging_dir'"
scp -r apps/desktop/dist "$backend_host:$staging_dir/"
python3 scripts/validation.py --frontend-dist "~/$staging_dir/dist" backend run \
  --ref refs/heads/codex/my-change --sha CANDIDATE_SHA \
  --plan my-change --phase final -- \
  cargo test --locked -p hiroute-daemon --test publication_process installed_ -- --nocapture
```

`frontend build --console-assets` records the successful build's source SHA and every
output hash. Ordinary `frontend build` keeps its development behavior without a receipt.
For SSH backend runs, `--frontend-dist` is an **execution-host** absolute or `~/`
directory, including that receipt. With a remote frontend target, copy its output
from that host to the backend staging directory. The runner reads those files,
verifies source and hashes under the existing checkout lease, and stages a verified
snapshot into the managed checkout. It does not transfer frontend bytes or require
a backend frontend toolchain. Do not copy files directly into a managed checkout.
The supplied staging directory remains caller-owned and may be removed after the
run finishes. A valid retained dist is rechecked on subsequent installed runs
without supplying `--frontend-dist` again.
The result's `recovery.command` uses the existing `remote-rust.py` SSH entry and
preserves the selected host/workbench, candidate, timeout, schedule, plan, phase,
executed related runs and Cargo arguments. For an integration checkout retained by
`always`, it continues the latest blocked run, including after multiple source
advances; the original owner remains responsible for cleanup. An early blocked owner
retained by `on-failure` uses exact-candidate reuse. Both paths recheck stopped-worker,
current-state, clean-source and driver identity under the checkout lease before
checking the supplied assets again. The default `never` removes a fresh checkout,
so its recovery command starts a new one and keeps the immediate-cleanup policy. Old results remain unchanged. The checkout state records resource use;
`--related-run` accepts executed reports, so do not pass a setup-blocked run there.
Do not replace production assets with a dummy page
or weaken package assertions. Hosted CI retains its existing frontend artifact
handoff; this SSH preflight does not change that boundary.

## Pilot from either development machine

### Reuse one build across scenarios

`validation.py pilot-build acquire --ref REF --sha SHA --owner TASK --case CASE`
acquires an explicit owner lease on a retained build. Repeat `--case` for the
required scenarios. The fixed Debug profile builds Desktop, daemon and CLI plus
the frontend in the same exact-candidate managed checkout. It never adopts old
unattested binaries. A match requires candidate, origin, command/profile,
toolchain/build environment and executor hashes, and rechecks exported frontend
contents and all three executable identities/digests. Same-owner acquire is
idempotent; different owners share binaries with independent leases and sessions.
An invalid existing bundle is retained and reported, not silently rebuilt.

The recipe binds the effective Desktop CPA manifest's exact content digest. The
checked-in default is read from the exact candidate blob; an explicit
`HIROUTE_CPA_MANIFEST` is read on the execution host, resolving relative paths from
the execution repository's `apps/desktop/src-tauri` directory. Ingestion captures
and validates the bytes once, then hashes that buffer. Equal bytes reuse the same
build even at different paths or through default versus explicit selection, when
all other inputs match. The original path is not retained in recipes, lease state
or status output.

Compilation consumes a private build-owned snapshot of the captured bytes. Reuse
and each Pilot start check that snapshot along with the existing frontend and
executable attestations. Changing or deleting the ingestion source afterwards does
not invalidate an acquired build or lease. A fresh acquisition reads the current
input: changed bytes select another identity, and a missing, unreadable, symbolic-link,
non-regular or invalid-JSON input fails before the managed build starts. JSON must be
a UTF-8 object without non-finite constants (`NaN` or `Infinity`); Python does not
duplicate the native manifest schema. Hardlinked regular source files are admitted
by their captured contents.

`python3 scripts/test-pilot-builds.py` runs the default local fixture suite. It
checks snapshot bytes/permissions, build-environment wiring, source changes,
rejection paths and lease/cleanup ownership with fake build and launch tools, even
when `rustc` is installed. It does not establish native product acceptance.
The explicit `python3 scripts/test-pilot-builds.py --real-consumer` entry belongs
to required CI validation, selected through `test-plan.py` for affected manifest
producer/consumer/tooling paths and full CI plans. It must execute exactly one
case with a real compiler; a missing compiler, skipped case, empty selection or
failure is nonzero. Each subprocess phase retains exit, stdout and stderr in
`artifacts/ci/pilot-manifest-consumer/` before temporary fixture cleanup. The case
checks the production `build.rs` and embedding declaration after the original
source is removed; it does not launch Desktop, CPA or a provider.

Use `pilot-build start --lease LEASE --case CASE` to launch each scenario with
the existing isolated Pilot launcher. It returns the session and socket for
`pilot-cli`. Add `--native-app` for an attested macOS App entry whose ordinary
opening/reopening preserves the same isolated profile; see the [native App
lifecycle](desktop-tauri-pilot.md#managed-native-app-opening-and-reopening).
All scenarios for that lease use the same build. These managed
bundles must be launched through this entry so sessions are registered before
launch; direct `pilot start` is refused for them. There is no arbitrary command
or release-profile override in this reusable build profile.

After actual assertions and saving unique debug evidence, use
`pilot-build finish --lease LEASE --case CASE=green --evidence-saved`, repeating
`--case` for every declared case (also accepting red/expected_red/not_executed).
Missing, duplicate and unstarted green cases fail. Finish stops only that lease's
verified sessions, saves diagnostics and verdicts, and releases its ownership.
The last finished lease triggers the existing local-rust preview/apply cleanup;
another owner, uncertain process identity or changed source retains the build.
Finish can be repeated to retry a refused cleanup without rewriting verdicts.

The npm dependency directory and generated frontend/native build files are
removed from the exclusively owned managed checkout after a successful build;
the small frontend output/digest and build logs are saved alongside run evidence.
No second frontend checkout or binary copy is retained. Cleanup preserves session
roots/screenshots/logs and original run evidence, including failed and unexecuted
scenarios; it never turns successful compilation into E2E green. Build failures
and interrupted/unknown acquisitions remain kept for diagnosis. `pilot-build
status --lease LEASE` reports the exact build and ownership state. There is no
automatic cleanup of other owners or historical Mac runs.

Both `pilot-build` and `pilot config/start/status/stop` transfer the caller's same
small content-addressed Python tooling bundle to the selected host's HOME. Build
inputs record its hashes. These commands do not modify or execute the host's
developer-checkout drivers, and missing or invalid bundles fail without falling
back to those drivers. Use the same caller checkout throughout a session so all
operations use the same tool revision. `pilot config` reads Desktop configuration
from the execution repository set in `validation.json`; the wrapper supplies that
path to the bundled driver. Direct local `desktop-pilot.py config` still defaults
to its own checkout. Both callers use the same execution account and configuration
to share the host's cache and locks. Existing manual build/start/finish commands
below remain available for ordinary, non-reuse builds.

```sh
python3 scripts/validation.py pilot-build acquire \
  --ref refs/heads/codex/my-change --owner my-task \
  --case model-save --case agent-restore
# Use the lease printed above. Both starts use the same three binaries.
python3 scripts/validation.py pilot-build start --lease LEASE --case model-save
# Use the returned socket with pilot-cli; run real actions and assertions.
# Optionally stop each completed session with pilot stop before starting the next.
python3 scripts/validation.py pilot-build start --lease LEASE --case agent-restore
python3 scripts/validation.py pilot-build status --lease LEASE
# Only after real assertions and saving any unique evidence:
python3 scripts/validation.py pilot-build finish --lease LEASE \
  --case model-save=green --case agent-restore=green --evidence-saved
```

`acquire` defaults to the caller's committed HEAD; that candidate must be available
from the execution repository's origin. Its frontend is built automatically, so
this flow needs neither `--frontend-dist` nor a separate frontend checkout. The
execution checkout's Rust toolchain file must match the candidate before tool
versions are captured. Compiler/frontend environment overrides are fingerprinted;
different PATH/build flags/tool versions intentionally prevent reuse. Configure
the same Desktop PATH on the Mac and Linux callers when they use the same Mac.
For reusable builds an explicit `path` is followed by standard Unix system paths
and the user's Cargo bin, without appending the caller's interactive/SSH PATH.
The cached run remains evidence for its original SHA, never for later edits.
Build process exit and per-owner scenario verdicts are recorded separately.

Missing session metadata after interrupted startup is unknown ownership, even
when a process scan is empty. Such a lease stays open for diagnosis; do not bypass
it with raw release/retire commands. A failed build prints its retained run ID and
does not silently create another build on retry. These retained failures still
use the existing owner-led diagnostic and cleanup procedure.

Use the [existing accurate-candidate build procedure](desktop-tauri-pilot.md).
Build the frontend on the Mac from the same exact candidate as Rust, using a
dedicated checkout if its developer checkout has changes. Configure an additional
SSH frontend target in a separate `--config` file when necessary; ordinary Linux
frontend checks and the Mac's embedded frontend are different outputs.

For the Rust step, `--frontend-dist` is an execution-host path. The wrapper calls
`desktop-pilot.py config` there and passes its output to the Rust child as
`TAURI_CONFIG`; it does not forward the caller's environment or Linux paths.

```sh
python3 scripts/validation.py --frontend-dist /absolute/mac/candidate/apps/desktop/dist \
  desktop run --ref refs/heads/codex/my-change --sha FULL_SHA --keep -- \
  cargo build --locked -p hiroute-desktop -p hiroute-daemon -p hiroute-cli \
  --features hiroute-desktop/desktop-pilot \
  --bin hiroute-desktop --bin hirouted --bin hiroute
python3 scripts/validation.py pilot start --app /absolute/mac/managed/target/debug/hiroute-desktop \
  --build-run RUN_ID --source-sha FULL_SHA
python3 scripts/validation.py pilot-cli --socket /absolute/mac/session/runtime/PILOT.sock ping
python3 scripts/validation.py pilot-cli --socket /absolute/mac/session/runtime/PILOT.sock state --json
python3 scripts/validation.py pilot status --session /absolute/mac/session/session.json
python3 scripts/validation.py pilot stop --session /absolute/mac/session/session.json
python3 scripts/validation.py desktop finish RUN_ID --verdict green --evidence-saved
```

Install/configure the private Pilot CLI on the Mac per the existing guide. Its
version must be 0.7.3 and must match the plugin. Socket, screenshots, session paths,
build records and cleanup all belong to the Mac. The wrapper does not weaken
artifact/process ownership checks or expose the socket over TCP. Preserve actual
scenario assertions and both roles' Debug evidence; `ping` alone is not WebView
or product success. E2E build exit 1 with process_exit=0/scenario=unassessed retains
its existing meaning. Only finish green after assessing real scenarios.

Desktop requires an available GUI login session for the execution user. The
read-only doctor checks `gui/<uid>` and the console owner on macOS. A successful
check is a prerequisite, not proof that SSH-launched WebView works; validate
`state` and the real page journey. If the host needs GUI LaunchAgent dispatch,
configure and validate that separately; the wrapper does not install a service,
log in users, or replace the existing process-group launcher.

## Evidence and rollout

Ordinary remote calls execute the scripts in the configured repository; reusable
Pilot lifecycle calls use the hashed tooling bundle described above. `doctor` reports
their hashes and checkout revision; keep tooling revisions in sync when using
new runner flags. No source sync or edits to a developer checkout happen implicitly.
`path` explicitly extends execution PATH without depending on interactive shell
initialization. Configuration contains no passwords or tokens; SSH uses the user's
normal keys/configuration and host-key verification.

Local-rust retains the Mac's existing shared lock; Linux has its own host lock.
Do not concurrently use a separately configured SSH workbench and local-rust on
the same account to bypass capacity limits; select one runner for that host.
Each checkout retains its default target and shares only sccache compiler objects.

Routing/runner changes use their Python regression tests, including source
selection, quoting, platform selection and failures, plus contract convergence.
These tests do not prove Rust compilation or Pilot product journeys. Final
environment acceptance requires a real local Rust candidate and a real Mac
Desktop/Pilot journey, recorded separately with actual revisions and scenarios.
