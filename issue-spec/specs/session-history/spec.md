# Sessions, request evidence, and model performance

## Purpose

A session records requests and delivered content actually visible to HiRoute.
Request, Agent turn, route execution stage, and Worker task are distinct units;
none can be completed or attributed by guessing from another.

## Requirements

### Requirement: Session correlation uses current native identity conservatively

Verified Worker identity takes precedence and binds one task across Start/Continue
while retaining each exact run/request association. Different tasks remain isolated
even if they claim an equal native session. Legacy per-run observations are not
rewritten/merged and do not acquire invented assessment.

Ordinary native correlation uses these current inputs:

| Client | Recognized identity |
| --- | --- |
| Claude Code | `X-Claude-Code-Session-Id`; Messages JSON session inside `metadata.user_id` |
| Codex | `thread-id` / `session-id` and coherent Responses `client_metadata` dimensions |
| Qoder | `session-id` in its existing namespace |
| Pi / branded pi-ai | Native Responses `x-client-request-id` / optional `session_id`, or configured `x-session-id` / Messages `x-session-affinity` |
| DeepSeek Harness | `x-deepseek-harness-session-id`; branded pi-ai fields when actually sent |
| OpenCode | `x-opencode-session-id` across all three ingress protocols |
| Hermes | `X-Hermes-Session-Id` from the actual native conversation on the managed main-request path |

Pi `pi (...)` and DSH `deepseek-harness/<version>` User-Agent qualify interpretation
of their pi-ai fields, not authentication. Generic request IDs/cache keys/affinity
cannot create native identity for other callers. OpenCode's `x-opencode-session`,
`x-session-affinity`, `X-Session-Id`, parent/project/request headers are not current
session identity and need not match it; shared cache affinity cannot merge sessions.
OpenCode recognition does not add managed settings or Worker support.

IDs must be nonempty, at most 128 bytes, and free of controls. Duplicates, invalid or
conflicting dimensions, or competing vendor families degrade to request scope without
content inference. Coherent dimensions yield one identity in existing HMAC namespaces.
Absent native fields use conservative complete-first-real-user inference when legal;
empty/unreliable/incomplete/generic wrapper input remains request-scoped. Raw native
IDs are not persisted. Equal content cannot merge distinct native IDs; rewriting
content does not change an established native session.

Hermes SHALL use a distinct HMAC namespace scoped to the trusted managed connection/authorization. Equal raw native IDs under distinct connections cannot merge conversations. For a trusted registered Hermes consumer, absent or invalid native identity remains request-scoped without first-user content inference. A caller-supplied header or user-agent does not establish managed-consumer authority; session identity itself grants no model or management access. Native cache/root affinity is not Hermes conversation proof.

#### Scenario: Separate Hermes conversations when identity is missing or content matches

- **WHEN** independent managed Hermes conversations have equal prompts, equal raw IDs under different connections, or no usable native ID
- **THEN** proven scoped identities remain distinct and unproven requests remain request-scoped; equal content or cache affinity cannot merge their decisions or observations.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165296920

### Requirement: Captured content preserves actual request and response attribution

Sessions MUST present received requests and ultimately accepted responses using
proven session/turn/request/Attempt relations. Agent-local or provider-only history
not sent through HiRoute is not fabricated. An Agent turn may contain several model
requests; tool calls/results and a final answer may belong to different requests.
Repeated history retains its original accepted-model attribution instead of being
signed by the latest request's model. Namespace/group/name/call identity, parameters,
and actual results distinguish equal-named tools in different namespaces.

Body is the primary view. Canonical content kind is preserved: user-visible text is
question/answer, tool fields are tool context, provider state and reasoning events
are not final answer text. `text_delta` plus `text_finished` for one block composes
one continuous result rather than duplicate snapshots or fragmented words. Request
`accepted` after termination means response delivered, not still-processing admission
or entire Agent task completed. Missing model/usage/value/body facts remain explicit.
Recorded plan identity follows [route history](../request-routing-history/spec.md).

#### Scenario: Tool response is followed by a later final answer

- **WHEN** one turn invokes a tool and then makes another model request
- **THEN** the view MUST preserve both requests, tool context, actual models, and
  final answer without inferring that the first request completed the whole task.

### Requirement: Titles and folded context do not rewrite recorded content

Session title comes from the first readable real user input in request/message
order. Empty messages and standalone fully matched trimmed
`<environment_context>...</environment_context>` blocks are skipped; partial tags,
plain mentions, or trailing real text are not. Complete leading Claude
`<system-reminder>...</system-reminder>` blocks may be skipped to expose the remaining
question. Context-only sessions use a neutral title and update when later real input
becomes readable, including across fixed-range pagination and home activity refresh.

Complete leading Codex setup blocks, including recommended_plugins and an exact
`# AGENTS.md instructions` heading (optionally naming a path) followed by a complete
`<INSTRUCTIONS>...</INSTRUCTIONS>` block, are also presentation context. A following
complete environment block may be consumed with that setup prefix, while remaining
real text supplies the title. Mere mentions or malformed/unclosed setup cannot hide
a user question. Captured bytes remain expandable and unchanged.

Body may initially fold standalone Agent context, complete leading reminders,
consecutive system/developer content, and tool definitions at their original location.
Users can expand captured original text. Real user/assistant messages cannot be
hidden by partial-tag guesses. Incomplete pagination/search-anchor context keeps raw
text until safe interpretation is possible. Folding/title extraction changes neither
model request, search offsets, nor audit data.

### Requirement: Turn changes and request relay have different identities

Within-turn model relay and same-model Key relay MUST be separate. Adjacent-turn
comparison uses reliable session order and actual accepted models, possibly the
turn's last accepted model without dropping other request contributions. Missing
predecessor, ambiguous time/association, or fact gaps yield unknown change, not false
or true. Determine adjacency in the authorized time scope before display filtering/
pagination; hidden middle turns cannot make remaining turns adjacent. First visible
turn with no proven predecessor is not an automatic model switch.

### Requirement: Search anchors obey visibility and original UTF-8 offsets

Hits MUST locate actual readable content with original UTF-8 offsets, including
multibyte text, case-fold expansion, overlapping and cross-block matches. Deleted,
expired, inaccessible, or conflicting-run content cannot be opened from an old hit/
cursor. Index delay/failure is incomplete coverage, not confirmed no-match.
Authorization is checked again when opening body. Query changes immediately clear
old results and reject delayed responses; refresh/home links reopen the same session
without fabricated identity.

Released V2 occurrence `search` is a separate bounded scan/snapshot interface.
Pages preserve the original query/scope and request/content/index window, keep seen
gaps visible, and expose continuation while unscanned range remains even without a
hit. They do not scan all history up front to certify completeness. Newly added
requests/index publications do not enter that in-progress snapshot. CLI default
range remains fixed through continuation.

### Requirement: Desktop session search is live matching-session pagination

Protected V2 `session_search` returns one original anchor per matching session,
sorted by latest matching request then stable session ID. Limit counts sessions;
next_cursor means more matches, not directory scan windows. Each page reauthorizes.
Authenticated cursor binds reader identity/scope, normalized keyword, filters,
limit, and last sort position, not global relation generation or content/index
watermarks. Same-scope authorization renewal still requires a current grant.

New matches in unread sort range may appear. Activity moving a session before the
consumed position appears after refresh; Desktop merges by session ID. Unrelated
requests/index publication do not invalidate continuation. Scope/query/filter changes
start a fresh query. Every page and anchor rechecks cleanup/expiry/run conflict.

Literal case-insensitive substring behavior uses Unicode lowercase; live keywords
allow up to 256 Unicode characters, independently of occurrence search's 256-byte
limit. Only complete digest-verified live indexes yield hits. Missing/failed/pending
indexes inside the authorized range set `index_partial`; out-of-scope gaps remain
private. Bounded backfill is resumable without rewriting original bodies or dates.
No ready matches is complete empty only when no authorized index gap remains.
Four reader permits and the real 500 ms SQLite deadline remain; timeout is retryable
failure, never empty success. Failure clears continuation and retry starts fresh.

### Requirement: Completeness is independent of model success and preview text

Fact and body failures cannot block the model response. Retain available portions
and identify missing facts, content, or both. Session body completeness aggregates
all visible requests from the same per-request directory rules; a successful title
preview cannot hide another request's loss. Whole-request/end-event gaps and interrupted
capture count even when stored chunks are individually complete. Detail lists may
be bounded with explicit truncation, but the aggregate accounts for every gap and
large gap count cannot masquerade as storage outage. Cleanup/expiry are separate states.
Incremental tool arguments and final snapshots cannot silently disagree.

### Requirement: Request receipts close independently of Agent turns

A valid `RequestFinished` MUST build its immutable RoutingReceipt, known value, and
request terminal projection without waiting for `AgentTurnFinished`. Complete
request facts remain complete when no turn-ending event exists; real gaps remain
gaps. Function/custom named-tool identity survives receipt/restart digest checks.
Request delivery is never a substitute for Worker/task completion.

### Requirement: Worker queries use proven task/run and model relations

Task evidence MUST associate main task, frozen plan, Harness, native session, and
confirmed turn/request/Attempt without crossing equal-plan concurrent tasks.
Authorized CLI reads and Desktop task-to-session navigation use those exact relations.
Every request in a multi-model turn retains actual model/native reasoning; the last
model cannot stand for all execution. Main Agent using another provider does not
invalidate the delegated Worker evidence.

### Requirement: Quality stages preserve frozen selection and later assessment

Route stages derive from sealed actual execution identity and can span multiple
routing turns, not simply user-message counts. New user input rejudges; broken
ContextHold can also seal a decision boundary, while tool continuations stay in
that turn. A boundary without a task-ending fact is unknown, not invented completed/
interrupted. No new judgment means no newly invented external assessment. Missing,
zero, malformed, and partial scores are different.

Each stage retains selected/actual category, group/index, model/profile, plan revision,
turn range, degree probability/threshold, selection reason, and any low-score trigger/
floor/source stage. Assessment uses the previous stage's frozen rubric. Later more
complete assessment may replace that stage's latest score but cannot rewrite an
already-made selection reason. Network/quota/observation failure is not incompetence.
Mixed or unknown model identity stays separate instead of crediting one model.

### Requirement: Quality summaries and drill-down share one authorized snapshot

Plan quality defaults to the selected published version's latest seven days.
Backend aggregation covers the full authorized retained scope, using the same
snapshot as stage pages. Pagination, selected model, and detail score filters cannot
change the mean. Count a stage once using its latest complete valid score; valid
zero counts, unrated/partial do not. Versions, categories, groups, and model/profiles
remain separate. Failed candidates that did not serve the stage receive no score.
Configured candidates without execution display no records, not zero.

Join configured positions by plan revision/category/group/published index and known
reasoning, without demanding equality to today's materialized model ID. This only
supplies names/order and cannot merge differing recorded model/profile summaries.
Display published order, not performance ranking; unknown labels stay unavailable.
Draft edits cannot change historical rows/threshold interpretation. Below-floor,
meets-floor, and unrated filters use each stage's frozen floor; equality meets it,
partial is unrated but may expose its raw score/gaps in detail.

Plan and session views share stage/score/query components and reject stale-range
responses. Switching sessions resets its stage filter, while same-session refresh
retains the choice. Evidence navigation opens saved exact request IDs even beyond
the first timeline page: execution first, optional later assessment trigger second.
Return preserves plan draft/version/time/filter or original session context; deleted
body stays unavailable and cannot be rebuilt from a score explanation. Public
`observation plan-quality samples --plan-id/--session-id` exposes the same bounded
read-only facts, help/schema discovery, and cursor without content/model calls.
Competence is stage performance, not task acceptance rate or universal model ranking.

### Requirement: Expose Hermes connection state and recorded execution models

HiRoute SHALL expose the managed Hermes profile's saved connection scope/Plan/protocol and preparation/operation state through existing CLI status queries and Desktop views. Routed Hermes requests SHALL be readable through the existing public execution queries with their recorded Plan, branch and actual execution model. Saved configuration, configuration-operation state and recorded execution SHALL remain distinguishable; the current Plan or an installed adapter SHALL NOT substitute for historical execution or unobserved live conversation selection. Existing session-history attribution and query-access contracts remain authoritative.

#### Scenario: Read saved intent and routed results separately

- **WHEN** a caller inspects a managed Hermes connection and a recorded request after the Plan has changed
- **THEN** status reports the saved connection/operation facts, while execution queries report that request's recorded Plan/branch/model through existing views.

#### Scenario: No routed request has been recorded

- **WHEN** a Hermes default and adapter are prepared but no applicable request has reached HiRoute
- **THEN** CLI/Desktop report prepared/configured state and any known next action without claiming runtime loading, effective live selection or an actual execution model.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165297426

## Implementation references

- [Native identity resolver](../../../crates/gateway/src/context_hold/identity.rs) and
  [real native-session scenarios](../../../tools/e2e-harness/tests/p0_gateway_native_sessions.rs).
- [Search contract](../../../crates/observation/src/query_v2/README.md),
  [live query](../../../crates/observation/src/query_v2/session_search.rs), and
  [receipt scenarios](../../../crates/observation/src/store/tests/store_flow.rs).
- [Quality aggregation scenarios](../../../crates/observation/src/store/tests/plan_quality/summary.rs)
  and [Desktop quality identity](../../../apps/desktop/src/features/plan-quality-state.ts).
