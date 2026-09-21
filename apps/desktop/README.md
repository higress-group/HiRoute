# HiRoute Desktop

[简体中文](README.zh-CN.md)

Desktop provides model management, routing configuration, Agent integration, task results,
and session history. It reaches the real local service through the shared Client Core. See
the [website documentation](https://hiroute.ai/en/docs/) for the public user entry point.

## Development entry point

Build the UI from this checkout, then build the matching native application and daemon:

```sh
cd apps/desktop
npm ci
npm run build
cd ../..
cargo build --locked -p hiroute-desktop --features desktop-runtime -p hiroute-daemon -p hiroute-cli --bin hiroute-desktop --bin hirouted --bin hiroute
target/debug/hiroute-desktop
```

macOS builds and real window interactions must be validated on macOS. See the repository
root `CONTRIBUTING.md` for common development checks. Keep all Cargo output in this
checkout's default `target/` directory.

This command starts the development binary. Use the public
[download page](https://hiroute.ai/en/download/) for installers and supported-platform
details. The native application starts the adjacent `hirouted` binary and initializes
ReleaseFacts from the embedded, signed resources. The WebView loads only locally built
assets and exposes no arbitrary command, path, HTTP, or authorization IPC.

Closing the window hides it while the native host continues running; activating the Dock
icon opens it again. Quitting the native application stops the daemon that it started, and
the next launch recovers from the real backend. An externally started daemon is attached
read-only and is not managed by Desktop shutdown. The website release notes define the
actual installation and platform scope.

After a trusted launch, the Desktop lock records the child PID, private endpoint directory,
and socket identity. If the lock is still owned, the original PID has exited, identities are
unchanged, and the endpoint is no longer listening, recovery is delegated to the daemon.
When those facts cannot be established, Desktop remains read-only and neither deletes nor
takes over the endpoint.

## Recovery and authorization boundary

The native backend owns confirmation context. A confirmation is bound to one Preview's
object, input, digest/revisions, and window lifetime, and expires after 60 seconds. The local
WebView only returns a one-time accept or reject for a current confirmation ID; it cannot
supply context, skip confirmation, or acquire a capability. Acceptance registers a one-time
Desktop capability through the inherited channel, and Apply is sent only after the matching
acknowledgement succeeds. An uncertain registration result closes that write relationship;
restart the native application to establish a new one.

Desktop keeps only the current interaction's submission key, digest, and Operation reference
in memory so it can query or explicitly retry after a lost response. Reopening reads actual
server configuration; transaction recovery remains the daemon Journal's responsibility.
The client does not read or write `pending-intent.json`. Switching editors does not cancel a
background operation, and each new action is checked independently for version and
idempotency.

The current interaction also retains a versioned Plan ID and target-name summary. If no
accepted operation is found after a revision change, the same intent may be Previewed and
confirmed again with its original key. If the original Operation is found, Desktop observes
it first; a different digest reports that the latest edit has not been applied and retains
the draft. A failed query or delayed routing projection does not leave the editor disabled.
A late result from an older edit cannot overwrite newer input.

“Restore previous name” creates a new Preview and confirmation against current desired state,
changing only the name and never restoring an old revision. The prior name is native-session
state. Non-sensitive drafts and language/text-size preferences are local WebView data.
Persistent Operations remain queryable after restart, but the in-session previous-name action
does not.

## Validation entry points

- Core/CLI/API/Application/daemon/Desktop: `cargo test --locked -p hiroute-client-core -p hiroute-cli -p hiroute-application-api -p hiroute-application -p hiroute-daemon -p hiroute-desktop -- --test-threads=1`. Desktop production-bootstrap tests use this checkout's `target/debug/hirouted`; build it first or include the daemon integration tests.
- Frontend: `npm run build`.
- Deterministic CLI generator: run `target/debug/generate-cli-contract` twice; the second run must produce no changes.
- Isolated macOS Plan setup: `python3 apps/desktop/tests/prepare_plan.py /tmp/hr02-new-empty-root`. It requires the real local Claude CLI, production model facts, and built production CLI/daemon, and uses production Preview/protected Apply rather than direct database writes.
- Real GUI acceptance: start the debug native application from the isolated root's `agent-input/` directory with `HIROUTE_DESKTOP_TEST_ROOT` set. Verify confirmation cancellation has zero writes, confirmation replay/window-close fails closed, rename A→B→A, two independent successful Operations, and desired/actual publication identity. The debug path override is absent from release builds.

Component doubles do not replace the reversible Tauri → Client Core → production-daemon
slice. Record process exit, scenario result, platform, and exact revision separately.

The real Tauri ACL probe is a development example only. Build it with
`cargo build --locked -p hiroute-desktop --features desktop-runtime --example acl_probe`,
then run `target/debug/examples/acl_probe` with an isolated `HIROUTE_DESKTOP_TEST_ROOT`. It
uses production handlers and permissions, creates an unpermitted local window and a
remote-origin main window, verifies ten denials, and writes `acl-probe.json` under that root.
It adds no product bridge. Exit 0 means ten denials, 1 means an assertion failed, and 2 means
the probe timed out.
