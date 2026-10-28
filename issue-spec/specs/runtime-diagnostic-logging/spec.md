# Runtime diagnostics

## Purpose

Local diagnostics explain runtime phases and real outcomes using safe typed events.
They do not replace execution facts, alter business decisions, or collect conversation
content as a consequence of choosing a verbose level.

## Requirements

### Requirement: Log-level changes converge through the shared settings file

Error, Warn, Info, and Debug MUST be available. Unconfigured debug-assertion builds
use Debug; release builds use Info. Explicit override and valid saved value take
precedence without writing the build default into settings. Desktop's debug toggle
and level selector use one state, preserved across restart.

Save success means the local setting persisted. Desktop adopts it immediately;
managed processes independently poll the same file and eventually apply the latest
valid revision without service restart, request interruption, cross-process ACK,
event bus, or process-status aggregation. Temporary unreadability retains each
process's last valid level/revision and reports a configuration problem. Recovery
converges automatically when valid settings and normal execution return. A stopped,
blocked, or permanently broken process has no bounded-convergence guarantee.
The settings UI shows save result, not fabricated per-process confirmation.

### Requirement: Typed events preserve phase and execution identity

Diagnostics MUST cover startup/control/protected native actions, model requests
and Attempts, content/observation delivery, Worker installation/execution, and CPA
lifecycle. Safe correlation, phase, elapsed time, closed reason, and actual outcome
separate startup from ready, pre-send failure from uncertain submission, first
selection from relay, precommit failure from postcommit cancellation, and content
loss from model failure. A cross-group relay retains the actual Attempt association.
Redundant native endpoint aliases cannot fabricate separate credential failures.
Observation being disabled does not make the diagnostic model outcome automatically
successful or unknown if the runtime has an authoritative result.

#### Scenario: A Home read is rejected after successful Local Control calls

- **WHEN** Desktop snapshot or observation transport succeeds but the final product
  read fails its context, session, epoch or snapshot check
- **THEN** diagnostics MUST record the final read rejection separately from transport
  success, using a fixed operation and error vocabulary without response contents.
  The original failure and read fences MUST remain unchanged; diagnostic delivery
  failure MUST NOT replace the read result or authorize a retry.

### Requirement: Every level excludes raw sensitive values before enqueue

Allowlisted typed construction MUST exclude credentials, recoverable credential
locators, full configuration, user filesystem paths, environment values, raw URLs/
headers/bodies, questions/answers, tool content/names, third-party output, and panic
payloads from both queue and file. External IDs become irreversible correlation
values. Unknown fields/records are not written verbatim, and keyword substitution
is not the sole protection. JSONL files use one versioned legal record schema.

Safe closed model/protocol/reasoning controls may explain a prepared request without
its prompt. Upstream transport/auth/status, retry scope, hashed request correlation,
and header/body commit facts may explain failure without arbitrary upstream text.
HTTP 200 does not establish complete model termination. Deleted content cannot be
recovered from diagnostic output.

### Requirement: Response failures are classified without logging tool payload

Decode/projection/terminal failure MUST distinguish unsupported field/value, missing
tool identity, incomplete arguments, lifecycle/duplicate/missing terminator, and
resource-limit categories. Known fields use closed enums; unknown values emit only
unknown. Dynamic error strings, body, credentials, and tool names never enter the
product diagnostic record. Failure events use the same bounded queue/loss handling,
not per-token logging. Explicit private raw-sample/offline investigation remains a
separate controlled action, with its own bounds and ownership, not Debug's default.

### Requirement: Private raw capture is explicit, expiring evidence

Raw capture is disabled by default and permitted only for an explicitly selected
isolated diagnostic session, with private newly created storage and bounded bytes,
duration, and Attempts. It excludes authentication headers while retaining required
request/response bodies, read-block order/offsets, source identity, actual tool/profile
projection, and formal Attempt association. Association binds the published Attempt
token/index idempotently even if a response was fully decoded in the pre-body prefix
and no later raw-response event exists.

An owned supervisor retains a finite lifetime after candidate exit and deletes raw
session data within 24 hours, or earlier on orderly owner stop. Candidate crash/stale
locks cannot prevent expiry. Its isolated process group, including descendants after
root exit, is stopped without signaling another session or recycled identity. Host
shutdown or forced owner death leaves explicit recovery cleanup, not an impossible
always-on deletion guarantee.

Offline replay uses the same Attempt decoder/tool mapping with original and alternate
chunk boundaries, ignores informational HTTP heads, and requires one applicable
terminal result. Inapplicable projector/non-success/incomplete/expired/over-budget
evidence cannot be reported as complete replay. Read chunks are not original TCP/TLS
packets. A successful sample without the actual historical failure cannot establish
that its cause was fixed. This private artifact is never normal product logging.

### Requirement: Subscription validation identifies the unmet condition

CPA subscription discovery SHALL distinguish inventory/account reads, control
updates, account pin validation and downstream catalog validation. A phase emits
entry before blocking and a terminal outcome with its own elapsed time, the shared
budget consumed, explicit-refresh intent and poll count. One discovery span ties
these observations together without an account identity. Terminal failures remain
visible at Warn; normal details remain Debug.

Pin evidence SHALL report the last observed equality/boolean/count facts for account
identity continuity, retry policy, persisted prefix/retry/cooling controls, nonempty
account models, exact model prefixes, valid stripped IDs and missing downstream membership. Unreached
checks remain absent, rather than false. Polling MUST NOT emit a record per poll.
Paths, credentials, account/model names and raw responses remain excluded. Historical
records lacking these observations cannot establish which pin condition failed.

### Requirement: Diagnostic degradation cannot block business execution

Queues, per-event size, file rotation, and retention MUST be bounded. Request
threads cannot wait for file writes. Saturation, disk failure, invalid file type,
or writer error preserves business semantics while exposing available degradation/
drop counters, including records that could not reach disk. Error reporting cannot
recurse or retry forever, and cleanup cannot remove foreign or active-process files.
OS access follows [local data rules](../local-data-and-diagnostics/spec.md), with
private defaults for newly created files and no extra owner/mode admission rule.

### Requirement: Safe local logs are available without a healthy daemon

Desktop MUST directly open the local diagnostics directory containing client and
engine role JSONL files through the file manager, including before service ready
or after exit. This action cannot read arbitrary user paths, databases, credential
roots, third-party logs, or full configuration, and never automatically uploads.

Isolated production smoke defaults to Debug for its own processes and managed
service, records selected source/candidate/applied role levels/completeness, and
supports explicit level or the isolated root's saved value. It cannot change a
normal user's preference, business parameters, security gates, or success criteria.
Failure/cancellation retains available safe logs and evidence gaps with normal
owned-process cleanup. Logging success is not a substitute for the real smoke path.

### Requirement: Startup recovery guidance remains available before service readiness

Storage startup SHALL retain bounded typed cause/action facts through its existing local startup channels without requiring a functioning console. The CLI and native failure surfaces SHALL use the storage-owned action rather than infer downgrade permission from a raw error, generic code or backup-directory presence. Unknown reasons, unsupported records and stale original-state bindings SHALL receive a conservative preserve-and-diagnose presentation. Diagnostic exports SHALL exclude complete private recovery backups, credential bytes and arbitrary native error output.

#### Scenario: Admission cannot write a recovery report

- **WHEN** root access, an existing writer lock or an unavailable authority key refuses startup before the storage writer barrier
- **THEN** the current attempt carries its closed cause through the startup failure channel and diagnostics without writing or replacing a report owned by another writer. Offline consumers may explain that cause with preserve-and-diagnose only; diagnostic records never authorize downgrade, resume or restoration. A later successful/unfinished/unknown startup cannot borrow an earlier admission reason.

#### Scenario: Storage refusal while the server is unavailable

- **WHEN** startup refuses missing recovery material, unavailable keys, low space or an unknown upgrade state
- **THEN** the installed failure surface and offline CLI can explain the safely established cause and next action without contacting the failed daemon; service availability is not fabricated from manager-command success.

Source Issue:
- https://github.com/higress-group/HiRoute/issues/176
- https://github.com/higress-group/HiRoute/issues/245

## Implementation references

- [Typed events and records](../../../crates/diagnostics/src/record.rs),
  [shared settings](../../../crates/diagnostics/src/settings.rs), and
  [runtime scenarios](../../../crates/diagnostics/src/runtime/tests.rs).
- [Gateway failure diagnostics](../../../crates/gateway/src/observation/provider/wire_diagnostic.rs).
- [Private capture supervisor](../../../scripts/private-stream-capture.py) and
  [ownership/expiry scenarios](../../../scripts/test-private-stream-capture.py).
- [Smoke evidence contract](../product-smoke-verification/spec.md).

### Requirement: Info request timing is exclusive and keeps asynchronous ingestion separate

Each production model HTTP request MUST produce at most one typed
`request_timing_summary` at Info, including rejected and cancelled requests. It
starts at lifecycle entry, freezes at lifecycle completion or cancellation, and
reuses the accepted request's diagnostic context and request token. Rejection
before a request identity exists retains its own span and no invented token.
Readiness and model-discovery GETs are excluded.

Local phase counters and classified pending-wait counters MUST form one exclusive
partition of the elapsed time. Local phases include ingress, Replay, parsing,
externalization, prevalidation, history projection, decision handling, planning,
preparation and response processing. Waits distinguish client input, decision
service, upstream connect/write/receipt, downstream writes, local background work
and unclassified work. Nested waits belong to the leaf; concurrent waits with
different owners remain unclassified. Pending waits include scheduling delay;
local polling time is elapsed local work, not a CPU-time measurement. Existing
transport spans and reader-suppression facts overlap and MUST NOT be added to the
partition.

Asynchronous observation queue and sink timing MUST remain outside the request
partition. Report record counts, delivered/lost counts, maximum record queue wait,
summed sink-call work and the delay from frozen request completion until release.
The summary may be emitted after existing bounded capture/ingestion work releases
its request ownership; no request waits for this diagnostic summary or a sink
flush. Sink delivery work includes projection, locking, commit and ACK validation,
and may overlap other channels. It is not a CPU measurement or an additional
request latency. Disabled observation still permits the timing summary.

Instrumentation MUST use fixed request counters and existing bounded producers,
without payload fields, per-chunk log events, new worker threads, global request
registries, new settings or persistence-format changes. Timing metadata MUST be
included in the existing queue byte accounting. Diagnostic loss/degradation keeps
its existing best-effort semantics and cannot change the business result.

#### Scenario: Large input, streaming and relay preserve the partition

A request crossing the Replay spill threshold, or relaying to a second candidate,
retains one request token and one summary. Sum of local and wait buckets equals
the frozen elapsed time. Codec polling remains local, and downstream waiting is
not counted again as upstream waiting.

#### Scenario: Slow or lost observation records do not extend the request

A blocked sink allows the model request to finish. Later queue/sink work changes
only the separate asynchronous counters. Queue overflow, invalid ACK, NACK and
panic retain explicit loss accounting, while cancellation freezes the request
clock once and still releases its summary through the existing logger.
