# Desktop tauri-pilot validation

This entry is for explicitly enabled macOS development/test builds, providing real
Tauri page observation, IPC diagnosis and focused reruns. It is not a product E2E
platform. Ordinary local save/publish needs one click; operations requiring separate
consent keep the real WebView confirmation, with backend checks of window,
confirmation ID, revision and action summary.

Use [HOME routing](validation-routing.md) from Mac or Linux to select local/SSH Mac
Desktop/Pilot. All build, socket, session and tool paths below belong to that Mac.
For limited disk, prefer [managed build reuse](validation-routing.md#reuse-one-build-across-scenarios):
one binary set per exact candidate/input, frontend from the same checkout and
released npm dependencies. Register scenario leases and reclaim after the last
owner. Start those builds with `pilot-build start` to register the instance; this
manual flow remains for ordinary managed builds.

## Isolation

- Only `desktop-pilot` enables the plugin, pinned to upstream v0.7.3 at
  `ed31926726b4cd7b3ffbc11f188e046f2b5e0faa`, without OS-level `press`.
- Ordinary `desktop-runtime`/release builds exclude it. Production capability is
  `main`; `pilot:default` appears only with explicit `tauri.pilot.conf.json`.
  Upstream release implementation is no-op and HiRoute also requires
  `debug_assertions`. Do not include this feature/config in releases.
- Each start creates a real private 0700 root, runtime/TMPDIR/data and process
  group. Product-identifier socket lives in the instance runtime root. Always
  pass exact `--socket`, never discover an assumed latest socket.
- `config` derives the production window and adds only `incognito: true`, giving
  WKWebView a non-persistent store instead of user localStorage/cache. Product
  data is isolated by `HIROUTE_DESKTOP_TEST_ROOT`.
- Socket permissions are 0600 with peer-UID checks. This remains full development
  WebView DOM/eval/IPC debugging authority: no real secrets, paid credentials or
  user data in a Pilot session.

## Build the exact candidate

Push the checked branch first, keep tracked source clean and use the full candidate
SHA. Frontend builds in the same checkout; Mac Rust uses the shared-lock runner:

```sh
source "$HOME/.cargo/env"
CANDIDATE_SHA=$(git rev-parse HEAD)
TASK_BRANCH=$(git branch --show-current)
npm --prefix apps/desktop ci
npm --prefix apps/desktop run build
TAURI_CONFIG=$(python3 scripts/desktop-pilot.py config --frontend-dist apps/desktop/dist)
export TAURI_CONFIG
python3 scripts/local-rust.py run \
  --ref "refs/heads/$TASK_BRANCH" --sha "$CANDIDATE_SHA" --keep \
  -- cargo build --locked -p hiroute-desktop -p hiroute-daemon -p hiroute-cli \
  --features hiroute-desktop/desktop-pilot \
  --bin hiroute-desktop --bin hirouted --bin hiroute
unset TAURI_CONFIG
```

Both `hiroute-daemon` and standalone `hiroute-gateway` declare `hirouted`; select
packages explicitly instead of relying on write order for one target filename.
Agent delegation additionally needs the sibling production CLI, so build
`hiroute-cli --bin hiroute` at the same candidate. Missing CLI is an incomplete
artifact, not incompatible Agent configuration.

The E2E build returns `unassessed`, potentially making the runner's exit nonzero;
read JSON `process_exit` for Cargo. Keep run ID/checkout and reuse its `target/debug`
binaries. After the real scenario and saved unique evidence, use `local-rust.py
finish RUN_ID --verdict ... --evidence-saved`. Keep artifacts still needed; no
manual deletion or `cargo clean`.

`start` validates the caller's local-rust run record: zero build exit, explicit
Desktop/Pilot feature, Desktop/daemon bins, checkout identity, Git SHA and default
target identity. It records inode/mtime/SHA-256 before launch. Explicit CLI package
and bin must both appear and its identity is checked too. Release/wrong feature/
path/replaced artifacts fail before creating a session, not after socket timeout.

Install the matching CLI at a stable private HOME path rather than a temporary
version/ID path. Use the release lockfile and verify plugin/CLI 0.7.3 at each ping:

```sh
PILOT_TOOL_ROOT="$HOME/.local/share/hiroute/pilot-tools"
mkdir -p "$PILOT_TOOL_ROOT"
chmod 700 "$PILOT_TOOL_ROOT"
cargo install --root "$PILOT_TOOL_ROOT" --version '=0.7.3' --locked tauri-pilot-cli
PILOT="$PILOT_TOOL_ROOT/bin/tauri-pilot"
```

## Start and identify the instance

```sh
APP=/absolute/local-rust-checkout/target/debug/hiroute-desktop
RUN_ID=32-character-run-id-from-local-rust-output
START=$(python3 scripts/desktop-pilot.py start \
  --app "$APP" --build-run "$RUN_ID" --source-sha "$CANDIDATE_SHA")
SESSION=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["session"])' <<<"$START")
SOCKET=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["socket"])' <<<"$START")
python3 scripts/desktop-pilot.py status --session "$SESSION"
"$PILOT" --socket "$SOCKET" ping
```

`session.json` records verified source/build identity, artifact summaries,
identifier, socket, data root, merged Desktop/daemon log and captured PID/start
 time/executable. Each parallel instance requires its own start/session/socket.
The `diagnostics` index lists bounded subsystem-owned files/byte counts and each
role's latest-boot `level_applied` level/revision/source and observed write-loss
count (missing means unknown), all-record degradation counts and `complete` when
both `current.jsonl` files exist. It follows no symlinks, copies no log bodies and
counts unknown files without names. Log presence does not establish readiness.

To preserve a source already saved by Desktop, `--data-root` may reuse an inactive
offline root without database copying, retaining ID/revision/Operation. Session,
runtime/temp/log/process group remain fresh. The root must be owned, real 0700,
not symlinked/overlapping the session, previously an explicit test root and free
of real credentials. A still-held Desktop lock rejects the second instance.
For Agent settings, also select an owned real 0700 `--process-home`. Only product
children receive this HOME; the launcher validates runs using caller HOME. Clear
inherited model/auth/cloud variables first and place CODEX_HOME/CLAUDE_CONFIG_DIR
within that isolated process HOME.

```sh
START=$(python3 scripts/desktop-pilot.py start \
  --app "$APP" --build-run "$RUN_ID" --source-sha "$CANDIDATE_SHA" \
  --data-root /absolute/offline-test-data-root \
  --process-home /absolute/offline-test-home)
```

## Managed native App opening and reopening

For native macOS CU, use an exact-candidate reusable build and explicitly request
its App entry. Ordinary Pilot startup remains available for WebView/IPC checks.

```sh
python3 scripts/validation.py pilot-build acquire \
  --ref refs/heads/codex/my-change --owner my-task --case native-reopen
python3 scripts/validation.py pilot-build start \
  --lease LEASE --case native-reopen --native-app
```

Select the returned `native_app.bundle` absolute `.app` path in native CU. The
bundle has a unique per-session bundle identifier; its application executable,
daemon and CLI are private copies from that one managed build, preserving their
exact bytes and signatures with distinct file identities. Separate executable
inodes keep macOS process paths stable when multiple Apps use the same build.
An already staged adjacent CPA is included only after its hash/size matches the build’s frozen native-target CPA manifest; absent CPA keeps
the existing subscription-local missing-artifact behavior. Do not copy, move or
re-sign the entry: create a new owned entry if its identity changes. This mode requires a reusable owner lease and its recorded
non-persistent WebView build configuration. It does not package a release App or
alter production launch behavior.

Every ordinary Finder/LaunchServices/native-CU opening passes through the same
private launch guard. The guard checks the open registered lease, candidate,
original and deployed executable identities, wrapper/configuration files and
owned directories. It restores only recorded launch values: private test data,
runtime, temporary directory, working directory and HOME/native Agent locations,
plus explicitly captured PATH/locale settings and the diagnostic override. It
never saves or restores the caller's authentication environment. A new private
HOME is created unless `--process-home` explicitly selects an owned offline one.
The non-persistent WebView store starts empty on each process launch; product
data remains in the same test root.

A process-lifetime lock prevents duplicate entry launches. Each successful reopen
records a new generation, PID/start identity and the same profile. The existing
process attestation admits only this Desktop and its direct daemon child; an
unknown process or replaced artifact refuses cleanup and retains evidence.
Native menu/window operations remain real product behavior: closing a window may
leave Desktop running, while application quit ends that process.

Use `pilot status --session SESSION` to observe a stopped instance without
launching it. Native APIs that select, activate, open or capture an App may reopen
it; after quit, do not use those APIs as a termination check. An intentional
ordinary reopen is safe through the recorded entry and must then be checked
against the same session/data root. `pilot stop` stops the verified processes but
leaves the entry reopenable. `pilot-build finish` revokes the entry first, stops
its owned processes, removes only the verified deployed files and then releases
the lease. Later native opening fails closed. Session metadata and evidence stay
available; another owner's sessions and the daily application are untouched.

Before marking the native case green on the exact candidate, physically select
and open the App, prove the real daemon/CLI and profile identities, quit and
observe stopped state, reopen through the ordinary native entry and prove the
same profile with no duplicate/default instance. Exercise changed-artifact
refusal and owner-only retirement. The local fixture-process tests cover the
entry/ownership contract but cannot establish LaunchServices, code-signing,
physical CU selection or the real product journey. Missing Mac/native tools
leave this acceptance unexecuted.

## Real page and IPC

Snapshot references reset each time. Observe, choose the current reference, wait
for asynchronous effects, then assert/diff. `click` returning ok is not business
success. `ping` bypasses WebView; use `state` to prove the page callback before
waiting for a selector. Example bilingual production controls:

```sh
"$PILOT" --socket "$SOCKET" state --json
BEFORE=$(dirname "$SESSION")/before.snap
"$PILOT" --socket "$SOCKET" snapshot -i --save "$BEFORE"
"$PILOT" --socket "$SOCKET" snapshot -i -s '.sidebar'
"$PILOT" --socket "$SOCKET" click 'nav button[aria-label="模型"], nav button[aria-label="Models"]'
"$PILOT" --socket "$SOCKET" wait --selector '.hr-models-page' --timeout 10000
"$PILOT" --socket "$SOCKET" assert visible '.hr-models-page'
"$PILOT" --socket "$SOCKET" diff -i --ref "$BEFORE"
```

Use non-secret search/filter inputs. Read-only status refresh travels through
production Tauri commands, `main` ACL and Client Core to this instance's real
daemon. `ipc` can compare structured results/errors but does not replace the UI:

```sh
"$PILOT" --socket "$SOCKET" click 'nav button[aria-label="会话"], nav button[aria-label="Sessions"]'
"$PILOT" --socket "$SOCKET" wait --selector '.session-filters' --timeout 10000
"$PILOT" --socket "$SOCKET" fill '.session-filters input[placeholder]' 'pilot-no-secret'
"$PILOT" --socket "$SOCKET" click '.session-filters button:first-of-type'
"$PILOT" --socket "$SOCKET" watch --selector '.session-filters' --timeout 5000 --stable 500
"$PILOT" --socket "$SOCKET" assert value '.session-filters input[placeholder]' 'pilot-no-secret'
"$PILOT" --socket "$SOCKET" ipc desktop_snapshot --args '{}'
```

For IPC errors, check visible state and `logs --level error`, then isolate ACL/
command/backend with separate `ipc`. `network` sees WebView fetch/XHR, not Rust
Client Core→daemon communication. Merged stderr is at `session.json`'s log;
only the test feature inherits daemon stderr. Data/runtime stay in the test root,
without reading or editing real Agent configuration.

Production CSP rejects `unsafe-eval`. Dedicated snapshot/action/assert/IPC works
without it; arbitrary `eval` remains rejected, never weaken CSP for diagnosis.
Use wait/watch and final structured assertions. Real consent dialogs are DOM
buttons: verify rejection, acceptance and expiry through UI, not forged IPC
confirmation. Ordinary saves check one click's Operation/result. Menu/Dock/Finder,
OS focus/accessibility and physical input still require native CU.
Read [diagnostic troubleshooting](diagnostic-troubleshooting.md) before retries.

## Diagnostics and sharing

`--diagnostic-level` controls only the isolated launch:

- Default debug or explicit error/warn/info/debug uses
  `HIROUTE_DIAGNOSTIC_LEVEL_OVERRIDE` only in Pilot children; production ignores it.
  Desktop passes the explicit daemon override. It takes precedence without writing
  settings, records `smoke_override`, and the launcher clears inherited overrides
  before constructing child environment. Settings retain normal product UI;
  saving during override rejects instead of persisting the temporary level.
- `persisted` sets no override, using saved level or Debug development/Info release
  defaults. It is not a fifth level or configuration schema field.
- Ordinary launch keeps saved values and the same default rules.

Check actual `level_applied` in each role's latest boot. Settings, correlation key,
writer lock and logs belong to test `diagnostics/<role>/`. Safe logs exclude
question/answer/tool/request/response bodies, keys/auth, recoverable credential
locators, full config/env values, user paths/raw URLs, raw third-party output and
panic payload. External request IDs become irreversible correlation identifiers.

The product's Open log directory shows safe role JSONL for user-selected issue
reporting. The merged `desktop-and-daemon.log` is private development output and
may contain raw third-party stderr; it is not shareable. Keep raw Pilot traces
local, no real credentials/data. DOM click/IPC proves only page/command behavior,
not OS input or a complete user journey.

## Stop owned processes

```sh
python3 scripts/desktop-pilot.py stop --session "$SESSION"
python3 scripts/desktop-pilot.py status --session "$SESSION"
```

Only recorded, still-verified owned process groups receive signals; no scanning
for other Desktop/daemon/Vite/checks. Temp roots/logs/screenshots are retained for
explicit evidence/cleanup. PID/PGID/UID/start time/executable, artifact inode/mtime/
hash and recorded members must all match. After leader exit only a previously
recorded matching daemon is accepted. PID reuse, unknown members, changed binaries
or unreadable identity refuse signals and need session inspection. Verified members
remaining after SIGKILL produce error, not stopped.

Upstream Unix socket setup briefly changes process-global umask then restores it;
a private runtime root does not eliminate a theoretical concurrent file-creation
race. Current plugin setup precedes HiRoute setup; check 0600 socket/0700 instance
roots and unchanged later product permissions. If setup becomes concurrent with
other creation, repair/upgrade the pinned dependency before relying on this limit.
`click`/`fill` synthesize DOM events, `ipc` uses the page invoke under Tauri ACL,
and `network` covers fetch/XHR only. None claims full OS or Rust-network acceptance.
