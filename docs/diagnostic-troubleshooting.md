# Development diagnostics and troubleshooting

## Default levels

Rust development and test builds with `debug_assertions` enabled default to Debug when no usable saved configuration is available; release builds default to Info. Saved levels and explicit overrides take precedence. The build default is not written to configuration. The shared diagnostics crate implements this rule for both Desktop and daemon, independently of `RUST_LOG`. Invalid settings use the same build default and report the configuration error without overwriting the invalid file.

Real product smoke uses the [Desktop/Pilot entry](desktop-tauri-pilot.md). Its `start` command temporarily overrides the level to Debug by default and passes that override to the managed daemon without changing user configuration. Use `--diagnostic-level persisted` explicitly when testing saved levels, filtering or configuration recovery. Use explicit arguments for other level tests and report them with the results. If a normal development instance has saved Info, select Debug in Settings; do not delete configuration to raise the level.

Unit tests that do not start the diagnostics runtime do not generate product logs or create diagnostics directories merely for testing. Inspect assertions, command output and the test's own evidence first. A Debug binary does not prove that product logging was initialized; verify the actual level and log state.

## Diagnose failures before rerunning

1. Record the exact SHA, managed build/run ID, scenario, observed result, failure time and your session path. Classify observed failures and batch related fixes. A fail-fast stop leaves the failure set incomplete and unexecuted or unknown checks pending. For diagnosis across test binaries, declare the scope and explicitly use `--no-fail-fast` under the [failure collection rules](test-selection.md#failure-policy-and-completeness). Do not rerun the full suite immediately after one failure.
2. For isolated smoke, run:

   ```sh
   python3 scripts/desktop-pilot.py status --session "$SESSION"
   ```

   Inspect both roles' current boot, applied `level_applied` level/source/revision, write loss and degradation in `diagnostics`. Normal smoke should show Debug for Desktop and daemon, with source `smoke_override`. Mark missing records unknown. `complete` means only that both current files exist; it does not establish business success or absence of lost logs.
3. Follow `session.json`'s `data_root` to `diagnostics/desktop/current.jsonl`, `diagnostics/daemon/current.jsonl` and their rotated files. Correlate startup, control calls, request/upstream attempts and Worker events using the current boot, failure time and existing correlation identifiers. Do not treat an old boot as current evidence. Find the first failure and adjacent events, rather than only the final error.
4. For startup/readiness failures, inspect Desktop startup phases and daemon startup events. For IPC failures, combine page errors with Pilot `logs --level error`, using a separate IPC probe when needed. For request or Worker failures, inspect control events, upstream attempts, terminal task state and the actual result. WebView network inspection covers fetch/XHR, not Rust-to-daemon communication. A logged request does not establish success, and process exit 0 does not prove task success.
5. If product logs are absent or writing is degraded, inspect the merged stdout/stderr file named by `session.json`'s `log`. Distinguish missing initialization, startup failure, write failure and business failure. This private development file may contain raw third-party output and must not be shared directly. Read managed remote results and output with:

   ```sh
   python3 scripts/remote-rust.py status RUN_ID
   python3 scripts/remote-rust.py logs RUN_ID
   ```

   For managed local validation, follow the returned result JSON and command-log paths. Runner command logs and product JSONL serve different purposes and cannot replace one another.
   Real Worker product E2E without Desktop explicitly starts the daemon at Debug. Before a failed exit, it copies safe `diagnostics/daemon/*.jsonl` into the candidate checkout's `target/product-e2e-evidence/<scenario>-<harness>-<pid>/`. The scenario's red JSON report names that directory, current boot, actual `level_applied`, files and record/invalid-line counts. Read the same boot's product logs through that report first, then use the managed runner's `command.log` for compilation, process output and test assertions. Successful scenarios report diagnostic completeness, boot and level without retaining the temporary log directory. Runner output must not be presented as product diagnostics.
6. Report the cause or remaining unknown boundary, associated evidence, repair and smallest retest scope. Separate process exit, scenario result and log completeness. Stop only instances you created, using Pilot's identity checks. Preserve unique evidence and do not clean other tasks or everyday instances.

Debug does not relax log-content boundaries. Do not add bodies, credentials, complete configuration or raw third-party output for temporary diagnosis. Before sharing, distinguish safe product JSONL from private merged output, real configuration and request evidence. Do not upload an entire test root.

For upstream authentication or gateway rejection, correlate Debug `upstream_wire` by `request_token` and `attempt_index`. Request events record whether Bearer / x-api-key is present, Authorization byte length, Content-Length, the actual `native_model` with bounded length/characters, and reasoning effort, Messages thinking type/budget and boolean switches read from the final serialized template. Unknown tiers are `other`; absence and an unavailable projection remain distinct. These facts describe prepared parameters, not delivery or the provider's internal computation. Response and attempt terminal events record HTTP status, actual H1/H2, Content-Type, safe `provider_error` and HMAC correlation of the upstream request ID, without retaining header values, addresses, bodies or raw provider error text. `request_kind=decision_service` uses the same safe channel for actual decision-service calls; `completed`/`failure` denotes the parsed terminal result. Combine upstream request and downstream headers/body facts in `attempt_end.commits` with the next attempt/binding in `fallback` to determine why retries or budget exhaustion occurred before body delivery, or why delivery ended in failure after it began. Missing optional fields in old records mean unknown. If request events are absent, first check levels, diagnostic loss and candidate-preparation failures; absence does not prove another upstream rejection.

## Stream decoding failures and private replay

`response_failure` uses closed `reason` and `field` enums. Unknown fields are recorded only as `unknown`, without provider text or tool names. `frame_index` is the SSE frame number visited by the decoder, including the failed frame. `received_bytes` is the cumulative byte count submitted to the decoder. When one read contains several frames, it is an upper bound rather than an exact error-character offset. Correlate the production terminal result by request token and attempt token/index.

Unix Debug builds offer private capture, disabled by default. Run `scripts/private-stream-capture.py run` around an isolated candidate daemon, specifying exact `--source-sha`, `--client`, a new `--root` and the candidate executable. Python must support `os.waitid`/`os.WNOWAIT` without reaping the child: Linux supports this; macOS requires Python 3.13+ ([platform reference](https://docs.python.org/3.13/library/os.html#os.waitid)). Check capability before creating a session. Use managed validation artifacts and verify their SHA. The script records source identity and binary digest; it does not replace build evidence. It sends no model requests. The client must still use isolated API-key configuration, while the main auth/config bytes, permissions and mtime are audited. Start with one minimal tool task and at most three model requests. Stop immediately if capture fails; do not let the client retry or continue looping. Supervisor polling is not an atomic network-request quota gate.

Capture directories use 0700 and files 0600, with one active writer. Limits are 8 MiB per file, 32 MiB per session, at most four underlying attempts and a one-hour active lifetime. Request bodies and response entities emitted by the production reader are saved in binary record order; cumulative lengths define each direction's offsets. These boundaries represent reads, not TCP/TLS packets. Capture retains the actual candidate profile, tool projection and correlation. Transport headers are not saved, profile header values are redacted, and the response decoder does not consume those values. Ordinary product JSONL contains no raw data. Exceeding a limit, expiration or a write failure stops capture without changing request semantics. Unsealed or incomplete requests cannot be replayed. Complete request bytes mean the reader generated them, not that the peer received them. A disconnected response cannot be claimed complete. The first failure's `stopped` signal is published only after sealing and syncing the sample, then further writes are disabled. Failed sync or capture is not complete evidence. A sealed sample whose request reader has not completed is still rejected for replay.

Run the `stream_replay PRIVATE_CAPTURE` example in the same candidate's Debug build, using original chunks, single-byte chunks and 4096-byte rechunking. Its entry is `new_for_attempt`, restoring the actual tool projection and production ActiveResponseDelivery context. Print only safe results, without dynamic error strings. Replay checks decoder equivalence; it does not prove network timing, runtime resource contention or native client task success.

Replay supports only successful cross-protocol decoder paths. It skips 1xx and requires one final status. Same-protocol projectors and unsuccessful statuses explicitly return `unsupported_capture_path`; they must not produce decoder judgments from a substitute production entry. Raw captures must not be committed or attached to an Issue. A separate owner with a bounded lifetime stops the isolated candidate's entire process group, including descendants after the main process exits, and reaps the main process. It does not stop other sessions or processes with reused identities. After the CLI returns, retain the session for at most 24 hours, then delete it. Output includes the owner PID; SIGTERM to that owner stops the candidate early and deletes the session immediately. Do not force-kill the owner during retention. Host shutdown or SIGKILL prevents cleanup; after recovery, use `private-stream-capture.py cleanup ROOT` to delete the exact expired session. Locks use kernel ownership and release when the candidate crashes. A leftover lock file does not permanently block cleanup; an active writer still prevents it.

macOS may stop reporting PGID/SID for an exited but unreaped child. The supervisor may use the process group ID established at creation only if it created the candidate with `start_new_session` and `waitid(WNOWAIT)` still proves that child is waitable. This fallback remains forbidden for live, reaped or unknown-origin processes. After normal exit, retain sealed samples until their deadline; a failed zombie PGID query must not trigger early deletion. Darwin may also return EPERM when signaling a group containing only a zombie. Accept this result only when kernel PID and process-group snapshots match exactly and prove the group contains only that unreaped leader. Other members or a failed query remain errors. Report diagnostic repairs, historical failure attribution and semantic repairs backed by actual old failing samples separately. Failure to reproduce does not establish a repair.

## Native connection lost while the daemon responds

A successful Local Control/CLI query does not establish that Desktop's separate
inherited protected channel is healthy. Home and Sessions also require that
channel. `TRUSTED_AUTHORITY_UNAVAILABLE` after an exchange failure stays denied;
ordinary read retry cannot repair it. Use **Restore local connection** in Desktop.
Recovery completes the exact pending acknowledgement on the same inherited channel
and creates a fresh product session. It never resends the failed business operation,
pauses admission or restarts the daemon, so running requests/tasks continue. A
pending reply can be retried; partial reply bytes are retained within the frame
bound. Incomplete request writes, malformed/mismatched replies and closed channels
require normal reopening after work finishes; they cannot be safely synchronized.

Routing snapshot revalidation must release shared storage before waiting for CPA.
Otherwise its control/lifecycle deadline can block native grant registration past
the shorter host ACK deadline. A long CPA call and routing snapshot are evidence
of contention, not proof of which historical protected exchange first failed.

Inspect `protected_exchange` in both roles before attributing the original trigger.
The event records a closed exchange family, phase, duration, failure category and
HMAC registration correlation, with the authority-loss transition on the host.
A late successful daemon ACK can coexist with a host timeout; it does not authorize
reuse. A successful later recovery is not proof of the earlier failure's cause.
Keep raw diagnostics private and report only revision-bound observable outcomes.
