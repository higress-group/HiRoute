# Managed CPA patch

`source.json` pins the upstream revision, artifact version and patch digest.
`local-management-stdin.patch` includes the parent-pipe management credential
bootstrap and Codex account model discovery. Build with `scripts/build-cpa.py`.

Codex OAuth registration queries `https://chatgpt.com/backend-api/codex/models`
using the account's access token and account ID over CPA's existing proxy/TLS
transport. For native access-only leases, `client_version` comes from the auth
metadata `hiroute_client_version`, populated by HiRoute's bounded native engine
`--version` probe. A new access lease re-probes the selected engine. Prerelease
suffixes are removed, matching Codex's models-manager. Missing native version
evidence still fails closed.

For independent OAuth credentials containing their own refresh token, discovery
extracts the version from CPA's upstream `codexUserAgent` constant. This is the
same client identity the pinned executor sends, rather than a second version
number maintained by HiRoute. It works on initial OAuth registration before any
HiRoute account-controls PATCH, and requires no native Codex installation or
login. Neither mode reads a local models cache or substitutes bundled membership.

Remote membership replaces the bundled membership for access-token accounts.
Existing bundled metadata can enrich matching IDs. Registration still applies
normal exclusions, OAuth aliases and account prefixes. Discovery has a five
second deadline and a 16 MiB response bound; redirects are rejected. Failure
leaves the existing registered catalog unchanged, without logging tokens or
upstream response bodies. An initial failure does not create model entries.
This does not add polling or a new catalog cache. Inference does not discover
models. Successful listing is not proof that inference succeeds.

HiRoute's ordinary routing compilation and Codex preview read and verify the
registered catalog without repeating an already-correct auth control PATCH.
An explicit subscription check refreshes it; credential/version changes use the
existing auth update path. This keeps remote directory availability out of
unchanged preview/publication reads while retaining all account pin checks.
For native borrowing, the fields PATCH carries the validated lease's current `hiroute_client_version`
alongside its controls. CPA persists and synchronously discovers that version
even when its watcher still holds the preceding metadata. No access or refresh
token is included in this control request.

Focused upstream tests are shipped in the patch:

For the stale-manager/current-file regression, copy
`vendor/cpa/tests/managed_version_sync_test.go` into the prepared pinned source's
`internal/api/handlers/management/` directory, then run
`go test ./internal/api/handlers/management -run '^TestHiRouteManagedVersionSyncBeforeWatcher$' -count=1`.
It uses the real fields handler, file store and patched discovery executor with
a synthetic transport. Legacy requests reproduce rollback; current requests
verify synchronous version consumption and access-only ownership.

```sh
go test ./sdk/cliproxy ./internal/runtime/executor ./cmd/server ./internal/api/handlers/management \
  -run 'TestCodexDiscovery|TestCodexModelDiscovery|TestRegisterModelsForAuth|TestLocalManagementPasswordPipeBounds|TestLocalMachinePasswordPreservesRemotePolicy'
```

Managed children allowlist HTTP(S)_PROXY and NO_PROXY (both cases). The local
patch applies Go's standard per-destination environment proxy selection to
protected-host TLS transports when no explicit proxy/injected transport exists.
It preserves NO_PROXY and does not retry direct after proxy failure. ALL_PROXY
is not part of this contract; configure HTTPS_PROXY for subscription endpoints.

## Current macOS development artifacts

The checked-in `development-cpa-artifacts.v1.json` identifies a historical
`.1` binary; its hash is retained as measured, not relabeled as the current
`.3` build. Do not stage that old binary with the current daemon.

On macOS, `python3 scripts/package-desktop.py build --cpa-source-repo /absolute/CPA-checkout`
generates the current patched CPA and a measured `cpa-artifacts.json` in its
candidate output directory. For a Debug Desktop build, set
`HIROUTE_CPA_MANIFEST=/absolute/candidate-output/cpa-artifacts.json` and use that
same output's `HiRoute.app/Contents/MacOS/cliproxyapi` beside the Debug `hirouted`:

```sh
python3 scripts/stage-desktop-cpa.py \
  --manifest /absolute/candidate-output/cpa-artifacts.json \
  --cpa /absolute/candidate-output/HiRoute.app/Contents/MacOS/cliproxyapi \
  --hirouted /absolute/checkout/target/debug/hirouted
```

Use the default ad-hoc package identity for this development staging path.
The staging tool rejects stale versions before copying; it still verifies the
exact digest, architecture, signature, dependencies and version of the binary.
A Linux backend test does not validate this native macOS build/install path.

## Managed OAuth authentication recovery and execution evidence

The pinned auth manager owns proactive refresh and its bounded same-account,
same-model 401 recovery. HiRoute keeps `request-retry=0`,
`max-retry-credentials=1`, and `streaming.bootstrap-retries=0`; it does not add a
second OAuth retry loop. Access-only native leases still lack refresh credentials
and cannot enter this recovery path. Existing executor protocol repairs can make
additional inference sends, so one logical HiRoute attempt is **not** a promise
of at most two wire sends. Cross-account/model fallback remains disabled.

The local patch attaches one request-scoped trace to the API handler context.
The common usage HTTP transport records each inference `RoundTrip` invocation
and HTTP 401 response; it does not count OAuth exchanges or account discovery.
These are transport attempts, not proof that bytes reached the remote server.
It deliberately does not count `MarkUpstreamAttempt`, whose outer Claude send
marker and inner transport marker can refer to the same request. Auth recovery
attempts/successes are counted separately in the existing auth manager; a
success includes reusing a newer token that another request already refreshed.
No token-usage records are created from these counters or failed 401 responses.
Codex uses this HTTP transport for non-WebSocket downstream calls; the HiRoute
bridge uses that path. WebSocket inference is outside this metadata contract.

Before the first downstream header/body/flush commit, a response-writer wrapper
sets the reserved `X-HiRoute-CPA-Execution` header to
`v1;<inference_attempts>;<http_401_responses>;<auth_recovery_attempts>;<auth_recovery_successes>`.
Each count is at most 32; overflow is `v1;unknown`. The trace carries no account
identifier, token, prompt, provider error body or usage. The upstream header
filter excludes this reserved name, and the local writer overwrites any supplied
value at commit. The same rule covers success and error replies. The existing
stream bootstrap recovery finishes before downstream commit; forwarded stream
failures do not trigger this auth recovery.

Gateway trusts these facts only for a validated CPA connector whose current
credential lease resolved to loopback. Other upstreams' identically named headers
are ignored. Missing, malformed, duplicate, unsupported-version or overflow
metadata is explicitly unknown, not zero or one. The typed `upstream_wire`
diagnostic carries the counters under `cpa_execution`; the ordinary logical
attempt/usage producer remains unchanged. Gateway reconstructs public response
headers and does not forward the private bridge header.

Gateway's existing deadline/cancellation closes the downstream call; CPA forwards
request cancellation into the existing refresh and retry context. A canceled
waiter now checks its context immediately after acquiring the account refresh
mutex, before reading/updating credentials or invoking a refresher. The stock
mutex wait itself remains non-interruptible; it can retain a canceled goroutine
until the active refresher exits, but must not cause another refresh/send.

The additive diagnostic field defaults to absent for historical records. No
persisted business-data migration or public response-contract change is needed;
the private CPA handshake is delivered by the matching managed artifact version.

Focused patch checks (run by the assigned validation owner, not implementation):

```sh
go test ./sdk/cliproxy/executor ./internal/runtime/executor/helps ./sdk/api/handlers ./sdk/cliproxy/auth -run '^TestHiRoute' -count=1
```

These cover the HTTP counter against a controlled upstream ledger, OAuth
exclusion, spoofed/error response headers, metadata commit, pinned same-account
401 recovery, no-refresh-token rejection, no recovery loop, canceled refresh
waiters, and stream payload followed by 401 without replay. Controlled tests do
not replace independent real-account expiry/refresh and restart acceptance.
