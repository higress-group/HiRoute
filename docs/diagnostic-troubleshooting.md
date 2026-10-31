# Development diagnostics and troubleshooting

## Default levels

Rust development and test builds with `debug_assertions` enabled default to Debug when no usable saved configuration is available; release builds default to Info. Saved levels and explicit overrides take precedence. The build default is not written to configuration. The shared diagnostics crate implements this rule for both Desktop and daemon, independently of `RUST_LOG`. Invalid settings use the same build default and report the configuration error without overwriting the invalid file.

Real product smoke uses the [Desktop/Pilot entry](desktop-tauri-pilot.md). Its `start` command temporarily overrides the level to Debug by default and passes that override to the managed daemon without changing user configuration. Use `--diagnostic-level persisted` explicitly when testing saved levels, filtering or configuration recovery. Use explicit arguments for other level tests and report them with the results. If a normal development instance has saved Info, select Debug in Settings; do not delete configuration to raise the level.

Unit tests that do not start the diagnostics runtime do not generate product logs or create diagnostics directories merely for testing. Inspect assertions, command output and the test's own evidence first. A Debug binary does not prove that product logging was initialized; verify the actual level and log state.

For a slow current Agent read, Debug `agent_read_timing` separates the bounded
read category, lock wait and hold, snapshot/locator work, selected verification,
native checks, final fence and current grant authentication. Counts are aggregate
captured operations, payloads and byte lengths; they contain no paths, payloads or
credentials. Stages can nest, so their inclusive durations must not be summed.
An absent event may mean diagnostics were disabled or dropped, not that a stage
did no work. A Home summary is a recorded hint and is not native verification.

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

The `control_shell` integration test retains startup failures under
`target/product-e2e-evidence/control-shell-startup-*/` before its temporary fixture
and daemon guard are cleaned up. The failure message names the private directory;
`report.json` records the child PID/status, original ten-second readiness budget,
observed boot/stage and missing or truncated files. Each retained stdout, stderr
and current daemon JSONL tail is limited to 64 KiB. The daemon uses a fresh Debug
diagnostics root, separate from the rejected role-composition check. No configuration
or data files are copied. Raw output tails are private and must not be posted directly.
An available diagnostic snapshot does not prove an asynchronous writer flush or
absence of lost events; missing, partial or ambiguous boot evidence remains explicit.
The managed runner preserves this non-debug target directory with its run artifacts.
The snapshot explains the last observed startup boundary, not the cause of a timeout.

For upstream authentication or gateway rejection, correlate Debug `upstream_wire` by `request_token` and `attempt_index`. Request events record whether Bearer / x-api-key is present, Authorization byte length, Content-Length, the actual `native_model` with bounded length/characters, and reasoning effort, Messages thinking type/budget and boolean switches read from the final serialized template. Unknown tiers are `other`; absence and an unavailable projection remain distinct. These facts describe prepared parameters, not delivery or the provider's internal computation. Response and attempt terminal events record HTTP status, actual H1/H2, Content-Type, safe `provider_error` and HMAC correlation of the upstream request ID, without retaining header values, addresses, bodies or raw provider error text. `request_kind=decision_service` uses the same safe channel for actual decision-service calls; `completed`/`failure` denotes the parsed terminal result. Combine upstream request and downstream headers/body facts in `attempt_end.commits` with the next attempt/binding in `fallback` to determine why retries or budget exhaustion occurred before body delivery, or why delivery ended in failure after it began. Missing optional fields in old records mean unknown. If request events are absent, first check levels, diagnostic loss and candidate-preparation failures; absence does not prove another upstream rejection.

For cross-protocol candidate exclusion, `request_projection` retains the first closed
cause from the actual adapter with a zero-based `request.messages` coordinate, when
known. This is a canonical coordinate, not a native `input` array index. An
`unsupported` candidate-preflight event does not start an attempt. A `declared_loss`
attempt-materialization event reports the number of historical commentary phase
labels omitted from that candidate's actual Chat projection; it does not claim an
upstream execution, change `reasoning_cleanup`, or replace original history/score
facts. Unknown affected-item counts remain null. The event contains no source text,
IDs or control values; correlation uses the existing request/binding HMAC tokens.

## Gateway listener fixture failures

The black-box `RuntimeFixture` retains its private temporary root after an unexpected assertion panic and prints its location. Its child is still stopped and reaped. Use `launch_rest_classified_with_diagnostics` when current boot and request diagnostics are needed; inspect the publication, native daemon output and applied diagnostic level before retrying. A successful rerun does not explain the original failure.

Export the needed evidence into ignored task artifacts before owner cleanup removes the runner's temporary directory. The fixture contains raw test credentials and request data: keep it private, publish only sanitized conclusions, and retire the stopped fixture after evidence export. Successful fixtures keep their normal automatic cleanup.

## Stream decoding failures and private replay

`response_failure` uses closed `reason` and `field` enums. Unknown fields are recorded only as `unknown`, without provider text or tool names. `frame_index` is the SSE frame number visited by the decoder, including the failed frame. `received_bytes` is the cumulative byte count submitted to the decoder. When one read contains several frames, it is an upper bound rather than an exact error-character offset. Correlate the production terminal result by request token and attempt token/index.

When native delivery finishes with `provider_model_event=response_unknown`, correlate
`native_completion_unknown` by the same request and attempt tokens/index. It records
the projector's first loss of completion proof: a closed reason, one-based SSE frame
index (including transport-only frames), and an optional closed event shape with
numeric positions. Snapshot, identity, delta/final-content, evidence-budget and
post-terminal causes remain distinct. It contains no response text, native IDs,
hashes or dynamic field names. The event is emitted once when the native projector
certifies Unknown; optional canonical-capture failures do not produce it. A `generic`
cause without frame/shape means that protocol has no finer cause instrumentation.
Missing diagnostic delivery is not evidence of a particular cause.

Unix Debug builds offer private capture, disabled by default. Run `scripts/private-stream-capture.py run` around an isolated candidate daemon, specifying exact `--source-sha`, `--client`, a new `--root` and the candidate executable. Python must support `os.waitid`/`os.WNOWAIT` without reaping the child: Linux supports this; macOS requires Python 3.13+ ([platform reference](https://docs.python.org/3.13/library/os.html#os.waitid)). Check capability before creating a session. Use managed validation artifacts and verify their SHA. The script records source identity and binary digest; it does not replace build evidence. It sends no model requests. The client must still use isolated API-key configuration, while the main auth/config bytes, permissions and mtime are audited. Start with one minimal tool task and at most three model requests. Stop immediately if capture fails; do not let the client retry or continue looping. Supervisor polling is not an atomic network-request quota gate.

Capture directories use 0700 and files 0600, with one active writer. Limits are 8 MiB per file, 32 MiB per session, at most four underlying attempts and a one-hour active lifetime. Request bodies and response entities emitted by the production reader are saved in binary record order; cumulative lengths define each direction's offsets. These boundaries represent reads, not TCP/TLS packets. Capture retains the actual candidate profile, tool projection and correlation. Transport headers are not saved, profile header values are redacted, and the response decoder does not consume those values. Ordinary product JSONL contains no raw data. Exceeding a limit, expiration or a write failure stops capture without changing request semantics. Unsealed or incomplete requests cannot be replayed. Complete request bytes mean the reader generated them, not that the peer received them. A disconnected response cannot be claimed complete. The first failure's `stopped` signal is published only after sealing and syncing the sample, then further writes are disabled. Failed sync or capture is not complete evidence. A sealed sample whose request reader has not completed is still rejected for replay.

Run the `stream_replay PRIVATE_CAPTURE` example in the same candidate's Debug build, using original chunks, single-byte chunks and 4096-byte rechunking. Its entry is `new_for_attempt`, restoring the actual tool projection and production ActiveResponseDelivery context. Print only safe results, without dynamic error strings. Replay checks decoder equivalence; it does not prove network timing, runtime resource contention or native client task success.

Replay supports only successful cross-protocol decoder paths. It skips 1xx and requires one final status. Same-protocol projectors and unsuccessful statuses explicitly return `unsupported_capture_path`; they must not produce decoder judgments from a substitute production entry. Raw captures must not be committed or attached to an Issue. A separate owner with a bounded lifetime stops the isolated candidate's entire process group, including descendants after the main process exits, and reaps the main process. It does not stop other sessions or processes with reused identities. After the CLI returns, retain the session for at most 24 hours, then delete it. Output includes the owner PID; SIGTERM to that owner stops the candidate early and deletes the session immediately. Do not force-kill the owner during retention. Host shutdown or SIGKILL prevents cleanup; after recovery, use `private-stream-capture.py cleanup ROOT` to delete the exact expired session. Locks use kernel ownership and release when the candidate crashes. A leftover lock file does not permanently block cleanup; an active writer still prevents it.

macOS may stop reporting PGID/SID for an exited but unreaped child. The supervisor may use the process group ID established at creation only if it created the candidate with `start_new_session` and `waitid(WNOWAIT)` still proves that child is waitable. This fallback remains forbidden for live, reaped or unknown-origin processes. After normal exit, retain sealed samples until their deadline; a failed zombie PGID query must not trigger early deletion. Darwin may also return EPERM when signaling a group containing only a zombie. Accept this result only when kernel PID and process-group snapshots match exactly and prove the group contains only that unreaped leader. Other members or a failed query remain errors. Report diagnostic repairs, historical failure attribution and semantic repairs backed by actual old failing samples separately. Failure to reproduce does not establish a repair.

## Native connection lost while the daemon responds

For failed Home reads, also inspect `product_read_end` for `desktop_snapshot` and
`observation_read`. These events report the final product boundary after transport,
authorization and publication checks. `client_context_invalid` distinguishes an
inactive presentation context; `read_snapshot_changed`, `confirmation_stale` and
`resident_unavailable` identify different rejected-read boundaries. Other failures
retain their closed transport/business category, with unknown native errors folded
into `external_error`. Failed reads survive Info filtering; successful reads are
Debug. Neither event contains request contents or raw exceptions. A missing event
does not establish success or failure, and a successful control response cannot
replace this final-read evidence.

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

For slow subscription validation, follow `cpa_control` records sharing a span.
`controls_patch` can include synchronous upstream model discovery; an entered phase
without a terminal observation identifies the last reached boundary, not its cause.
`account_pin` and `catalog_pin` report their elapsed/shared budget and poll counts.
Their `checks` distinguish index continuity, retry settings, persisted controls,
empty account models, wrong prefixes and missing downstream models. Missing fields
mean the check was not reached; false is an actual unmet condition. These are last
observations within one call, not an atomic snapshot of the external process.

For the upstream cause, inspect `cpa_model_discovery` in the same span. It records
CPA's credentials, client-version, request, response, catalog and registration
phases, elapsed times and closed failure reasons. `retained_first_failure` marks
an earlier failure retained by the same CPA process, not a new failure caused by
the current read. Compare process/attempt IDs before attributing a retry. The
record distinguishes proxy rejection from upstream HTTP status, and a successful
empty list from filtering or a discarded stale account. Missing/invalid metadata,
account eviction, observation limits and process exit constrain completeness;
older logs lacking these facts cannot establish an original network cause.

An empty account catalog after a failed initial discovery can coexist with correct
persisted controls. Ordinary pin reads do not refresh an already-correct account;
repeated reads alone therefore cannot establish that upstream discovery recovered.
Use an isolated explicit check to distinguish this state from slow management I/O.
Current pin reads fail promptly only for a quiescent registration with a consistent
terminal observation and no account models. The management response marks the
whole registration as pending across an authorization handoff, and also marks a
catalog/status read as pending when registration changes during that read.
Pending, inconsistent or unobserved discovery remains subject to the ordinary
control deadline; an older retained failure does not make the latest attempt fail.
Generation-only supersession may hand off once to current access credentials in
the same upstream deadline. A different registration epoch, disable or removal
never follows that handoff. Inspect both attempts before attributing a new request
to a transport retry.
Keep remote availability, local pin convergence and successful inference separate.
Do not infer the historical failing condition from the generic
`control_pin_not_applied` code alone.

For managed CPA inference failures, inspect the response `upstream_wire` event's
`cpa_execution.inference` observation. It separates CPA's downstream HTTP status
from the first and last actual transport attempts' upstream status and typed
transport/body failure. Attempt ordinals prevent combining different retries.
Middle attempts retain counts only. Request preparation is not proof of a send;
a transport invocation is not proof of remote receipt or billing. A successful
title request or model-directory check does not establish main-inference health.
The private header freezes when downstream headers commit, so later stream
failures are not included. An upstream 200 and normal body EOF cannot establish
application completion or prove a conversion failure. Missing detail, including
older counter-only records, remains unknown; a generic downstream 500 must not be
reported as the provider's original error. No raw body or error text is retained.

Inspect `protected_exchange` in both roles before attributing the original trigger.
The event records a closed exchange family, phase, duration, failure category and
HMAC registration correlation, with the authority-loss transition on the host.
A late successful daemon ACK can coexist with a host timeout; it does not authorize
reuse. A successful later recovery is not proof of the earlier failure's cause.
Keep raw diagnostics private and report only revision-bound observable outcomes.

## Model request timing at Info

`request_timing_summary` reuses the request token in `request_begin`,
`request_end` and attempt events. It covers lifecycle entry through completion or
cancellation, including early rejections (which may have a span but no request
token). GET readiness/catalog calls are excluded. All timing fields use
microseconds.

The values in `local` and `wait` add up exactly to `elapsed_us`; do not also add
legacy `model_stage`, attempt transport spans or reader-suppression durations.
Local fields measure elapsed work while the request future is polled, including
synchronous Replay I/O. They are not thread CPU counters. Pending gaps are labeled
at the actual awaited boundary: client input, decision service, upstream
connect/write/receipt, downstream write backpressure, or local background work.
They include executor scheduling delay. Competing or unidentified pending work
stays in `wait.other_us`; upstream receipt wait is not a provider-only inference
measurement. Background reader/capture CPU requires a process profile.

`asynchronous` is separate from HTTP latency. `queue_wait_max_us` is the maximum
wait of a single observation record; `sink_work_us` sums delivery durations across
channels, including projection, locking, commit and ACK validation. These channels
can run concurrently. `after_response_us` measures the release delay after the frozen
request ends, including retained capture ownership. None of these fields should
be added to `elapsed_us`. Lost records contribute to `dropped`, not fabricated
successful ingestion. No exporter/network latency is implied for the local OTEL
mapper.

`capture_jobs`, `capture_dropped_jobs`, `capture_queue_wait_max_us` and
`capture_work_us` describe the optional ordered content encoder before those
sinks. Capture work is elapsed encoding time, not CPU time. The same content
record may appear in both capture and sink counts; these are different stages,
not additive request latency. A failed capture job contributes explicit loss and
stops that request's dependent capture. For full local critical-path cost, include
`wait.local_work_us` with the local phase sum; do not exclude mandatory worker
readiness merely because it runs outside the request future.

One summary is enqueued after existing request/capture/queued-record owners are
released, so a slow sink can delay its appearance without delaying the response.
The existing `request_end` remains immediate. If a sink never returns or the
process is killed, the summary may be absent under the existing best-effort
logging contract; inspect sink failures and the immediate lifecycle events.

Use bounded function timings and CPU samples to locate local hotspots before
changing algorithms. Nested timings overlap, and sample proportions are estimates,
not exact whole-request CPU percentages. To compare equivalent HMAC-SHA256
implementations, run `cargo bench --locked -p hiroute-gateway --bench gateway_hash_paths`
through the configured validation runner. The benchmark checks identical framed
bytes across padding and update boundaries, then alternates backends on fixed
inputs. It changes no production backend. Measure its release executable on the
affected CPU: SHA instruction support can reverse a result from another machine.
Hash-only throughput does not establish a whole-request latency target; rerun the
actual request path before claiming an improvement.
