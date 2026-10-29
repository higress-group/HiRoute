# Managed CPA patch

`source.json` pins the upstream revision, artifact version and patch digest.
`local-management-stdin.patch` includes the parent-pipe management credential
bootstrap and Codex account model discovery. Build with `scripts/build-cpa.py`.

## Browser sign-in callbacks

The pinned browser flow receives Codex and Claude callbacks directly on temporary
loopback listeners at their existing registered addresses. It binds available IPv4
and IPv6 loopback interfaces; an occupied address releases any partial bind and
keeps the same session for protected manual input. The private auth-URL response
projects only `hiroute_callback_mode=automatic|manual`. An old artifact lacking
this handshake must not be used with the current daemon.

Reception checks the method, path, host and exact pending state. Browser callbacks
and the management callback POST share atomic one-use admission, including after
the waiting OAuth worker removes the callback file. CPA reception may retry failed
file publication; HiRoute's manual-input owner does not resend an admitted code
after a failed or uncertain submission. Observe status or cancel and start again.
An accepted authorization code cannot be replaced. The browser response contains no code,
state, provider error or management redirect and acknowledges reception only.
The existing token exchange and credential persistence establish actual login.
Completion, cancellation, timeout and owned-process exit release the listeners.
There is one pending login per provider; saved accounts do not retain these ports.

The patch carries `TestHiRouteBrowserCallbackValidationAndOneUse`, atomic admission,
dual-loopback cleanup, occupied-port fallback, main-route error and actual Codex
browser-to-file-store regressions. Run these on the exact prepared source with:

```sh
go test ./internal/api/handlers/management ./internal/api -run '^TestHiRoute' -count=1
```

The Codex exchange regression supplies synthetic tokens through the pinned OAuth
service test seam. It proves callback reception and persistence, not fresh real-account
authorization. No native credential store or paid model call is used.

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

An enabled newer generation of the same registration can supersede an in-flight
lookup. The stale result is discarded and the registration owner can discover
once for the latest access-bearing generation, sharing the original five-second
deadline. Removal, disable, lost access and a different registration epoch cannot
request this handoff. Transport failure alone does not retry. Continued churn
remains bounded and observable; an old account cannot populate a replacement.

HiRoute's saved-source maintenance separately recovers an empty directory after a
current, quiescent transient failure. It may issue one existing control PATCH after
rechecking the exact enabled source, sharing the original 20-second control budget;
CPA's five-second discovery deadline remains unchanged. Repeated failures back off
from five seconds to one minute. Passive reads and inference do not initiate this
retry. Authentication/authorization rejection, invalid catalogs, successful empty
lists and missing diagnostics remain unavailable without an automatic refresh.
This does not add a catalog cache, an OAuth refresh chain or a new polling owner.

HiRoute's ordinary routing compilation and Codex preview read and verify the
registered catalog without repeating an already-correct auth control PATCH.
An explicit subscription check refreshes it; credential/version changes use the
existing auth update path. This keeps remote directory availability out of
unchanged preview/publication reads while retaining all account pin checks.
When the directory is empty and the whole registration is quiescent with a
consistent terminal observation, a pin read fails immediately instead of polling
that empty result for 20 seconds. The existing response's `registration_pending`
flag covers the whole registration, including the gap before a bounded handoff.
Queued auth updates and reconciliation passes share that same scope.
Two observations bracket the local catalog read; any intervening transition makes
that response pending. No registry lock spans discovery or management I/O.
Pending work still gets its existing convergence wait. Missing/malformed
observations, unknown registration state and an older retained failure cannot
shorten the wait. A nonempty retained directory still undergoes every normal pin check.
For native borrowing, the fields PATCH carries the validated lease's current `hiroute_client_version`
alongside its controls. CPA persists and synchronously discovers that version
even when its watcher still holds the preceding metadata. No access or refresh
token is included in this control request.

The existing authenticated account-model response also carries the private
`hiroute.cpa-model-discovery/v1` observation. CPA retains the first failed or
discarded attempt and the latest attempt per account, including phase, closed
reason, total/phase durations, HTTP or proxy status, model counts and up to six
phase timings. Credentials, native version, account/model IDs, URLs and raw errors
are excluded. The in-memory store holds at most 128 accounts and reports evictions;
process exit loses it. Retention starts when this artifact starts, so it cannot
recover historical failures from an older process. Older concurrent attempts
cannot overwrite the latest attempt. No new endpoint or maintenance channel is
introduced. Transport retry settings are unchanged; generation handoff shares
the original five-second discovery deadline.
In-flight registration counts are separate from the retained history, survive
history eviction and are deleted when the last overlapping registration exits.

HiRoute validates this metadata independently of model membership, emits bounded
`cpa_model_discovery` transitions on its existing diagnostic context and explicitly
marks missing, invalid or truncated observations. Repeated polling does not emit
the same transition again. Proxy CONNECT rejection has its own status; it must
not be interpreted as an upstream HTTP response. An empty successful upstream
list, filtering to zero models and discarding a stale authorization are distinct
observations. These facts diagnose listing and registration, not inference.
Registration also records generation and registration-epoch equality as separate
booleans, so a stale result identifies the actual failed guard without exposing
authorization identifiers or generation values.

On Windows, native Codex and Claude file borrowing uses
[`borrowed_native_windows.rs`](../../crates/cpa-bridge/src/borrowed_native_windows.rs).
The reader holds the inspected parent and regular file, rejects reparses and
hardlinks, and checks native identity, timestamps and permission facts before
publishing an access lease. Readable external ACLs and read-only attributes stay
unchanged; only the managed access-only copy receives HiRoute's private ACL.
These local admission checks do not establish official account authorization,
model discovery or inference. Native adversarial cases live in
[`borrowed_native_windows_tests.rs`](../../crates/cpa-bridge/src/borrowed_native_windows_tests.rs).

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
pinned build. Do not stage that old binary with the current daemon.

On macOS, `python3 scripts/package-desktop.py build --cpa-source-repo /absolute/CPA-checkout`
generates the current patched CPA and a measured `cpa-artifacts.json` in its
candidate output directory. To prepare only that component, without rebuilding
Desktop or claiming installation acceptance, use the same producer:

```sh
python3 scripts/package-desktop.py prepare-cpa \
  --cpa-source-repo /absolute/CPA-checkout \
  --output /absolute/new-component-output
```

This builds the pinned source and patch, signs and probes the actual binary,
and measures its architecture, dependencies and digest. It never promotes an
existing development manifest by changing its flag. For either Debug or Release, set
`HIROUTE_CPA_MANIFEST=/absolute/candidate-output/cpa-artifacts.json` and use that
same output's `HiRoute.app/Contents/MacOS/cliproxyapi` beside `hirouted`:

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
Release Desktop rejects development-only, invalid or wrong-target manifests in
its build script, using the same parser as native launch. An optional launch-time
artifact rejection emits the closed CPA `artifact_validate` failure rather than
silently disappearing. A missing subscription runtime remains unavailable in
management and routing; it does not establish that native authentication failed.

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
`v2;<inference_attempts>;<http_401_responses>;<auth_recovery_attempts>;<auth_recovery_successes>;<downstream_status>;<first>;<last>`.
Each count is at most 32; overflow is `v2;unknown`. The trace carries no account
identifier, token, prompt, provider error body or usage. The upstream header
filter excludes this reserved name, and the local writer overwrites any supplied
value at commit. The same rule covers success and error replies. The existing
stream bootstrap recovery finishes before downstream commit; forwarded stream
failures do not trigger this auth recovery.

The fixed first/last slots contain `<ordinal>,<upstream_status>,<phase>,<reason>`;
an absent slot is `-` and an unobserved upstream status is `0`. Ordinals correlate
status and failure to the same transport attempt. Intermediate attempts contribute
to the count without allocating more slots. Phases are `started`, `headers`,
`transport` and `body`; reasons are the closed values `none`, `cancelled`, `timeout`,
`eof`, `unexpected_eof`, `dns`, `tls`, `network` and `other`. Classification uses
typed errors, never error text. The downstream status is the actual response
writer status and must match the HTTP response seen by Gateway. An upstream 500,
a transport failure without headers, and a 200 followed by a body read failure
are therefore distinct observations. A 200 plus normal EOF does not establish
successful protocol completion or identify a response-conversion failure.

The snapshot freezes at the first downstream commit. Later stream failures and
cancellations are outside this snapshot, even if another diagnostic observes
them. No new polling endpoint or retained per-request ledger is introduced.

Gateway trusts these facts only for a validated CPA connector whose current
credential lease resolved to loopback. Other upstreams' identically named headers
are ignored. Missing, malformed, duplicate, unsupported-version or overflow
metadata is explicitly unknown, not zero or one. The typed `upstream_wire`
diagnostic carries the counters and optional inference observations under
`cpa_execution`; the ordinary logical
attempt/usage producer remains unchanged. Gateway reconstructs public response
headers and does not forward the private bridge header.

Gateway's existing deadline/cancellation closes the downstream call; CPA forwards
request cancellation into the existing refresh and retry context. A canceled
waiter now checks its context immediately after acquiring the account refresh
mutex, before reading/updating credentials or invoking a refresher. The stock
mutex wait itself remains non-interruptible; it can retain a canceled goroutine
until the active refresher exits, but must not cause another refresh/send.

Gateway still reads the old four-counter `v1` header, without inventing inference
detail. The additive diagnostic field defaults to absent for historical records. No
persisted business-data migration or public response-contract change is needed;
the private CPA handshake is delivered by the matching managed artifact version.

Focused patch checks (run by the assigned validation owner, not implementation):

```sh
go test ./sdk/cliproxy/executor ./internal/runtime/executor/helps ./sdk/api/handlers ./sdk/cliproxy/auth -run '^TestHiRoute' -count=1
```

These cover the HTTP counter against a controlled upstream ledger, OAuth
exclusion, spoofed/error response headers, metadata commit, pinned same-account
401 recovery, no-refresh-token rejection, no recovery loop, canceled refresh
waiters, stream payload followed by 401 without replay, real loopback HTTP/transport
failures, truncated response bodies, cancellation, first/last attempt correlation
and commit-time freezing. Controlled tests do
not replace independent real-account expiry/refresh and restart acceptance.
