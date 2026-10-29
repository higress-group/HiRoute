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
| Claude Code | Coherent `X-Claude-Code-Session-Id` / Messages JSON session inside `metadata.user_id`, plus the optional actual `X-Claude-Code-Agent-Id` |
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

Claude session header and Messages metadata use the same released session
namespace and SHALL agree when both are present. Duplicate session claims inside
the metadata JSON are invalid, even when equal. An actual native agent ID adds a
separate scope dimension for children or teammates. Missing agent identity keeps
the supplied session with source/association uncertainty; prompt IDs, request
classes and notification text SHALL NOT invent that missing dimension.

Hermes SHALL use a distinct HMAC namespace scoped to the trusted managed connection/authorization. Equal raw native IDs under distinct connections cannot merge conversations. For a trusted registered Hermes consumer, absent or invalid native identity remains request-scoped without first-user content inference. A caller-supplied header or user-agent does not establish managed-consumer authority; session identity itself grants no model or management access. Native cache/root affinity is not Hermes conversation proof.

#### Scenario: Separate Hermes conversations when identity is missing or content matches

- **WHEN** independent managed Hermes conversations have equal prompts, equal raw IDs under different connections, or no usable native ID
- **THEN** proven scoped identities remain distinct and unproven requests remain request-scoped; equal content or cache affinity cannot merge their decisions or observations.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/119#issuecomment-6165296920

### Requirement: Managed Agent attribution uses persistent authenticated grant ownership

New observation writes SHALL associate a session with an Agent only through the exact authenticated workspace, grant ID and generation mapped to a server-resolved Agent connection. The product SHALL persist that mapping from a verified connection transaction and matching publication before admitting its traffic. It SHALL remain available across reconfiguration, grant rotation, native-setting restoration and restart, so retained sessions keep their recorded Agent. Registration and receipt rebuilding SHALL NOT backfill old observations without attribution evidence. User-Agent strings, shared Plans, model aliases and client-supplied session IDs SHALL NOT establish Agent ownership.

Startup attribution recovery SHALL register only verifiable ownership. Missing proof for a managed Agent grant in the current publication SHALL leave new observations unlinked and produce bounded diagnostics without blocking startup or journal recovery. A current authenticated service-completion receipt may prove ownership while the transaction's native-file tail remains pending; the exact journal and grant proofs still apply. After journal recovery completes the operation, publication restoration SHALL register the now-verifiable ownership for new sessions. Unverifiable historical connection operations SHALL be skipped with bounded diagnostics. Recovery SHALL NOT backfill unlinked historical sessions. Publication activation SHALL retain strict transaction and grant validation.

A session receiving mapped grants from different Agents SHALL retain an attribution conflict and display as unlinked. Repeated or later matching evidence cannot clear the conflict. Missing mappings remain unknown. Full fact deletion or retention expiry SHALL erase the session attribution evidence with those facts; content-only deletion preserves it. Worker run relations retain their existing verified authority and semantics.

#### Scenario: Reconfiguration and rotation preserve old attribution

- **WHEN** a configured Agent sends a request, its grant rotates or its native settings are restored, and the observation store reopens
- **THEN** the retained session keeps its original Agent and new mapped grants can associate new sessions without consulting the old native files.

#### Scenario: Conflicting and missing grant ownership

- **WHEN** two Agents' authenticated grants appear in one session, or a request has no registered mapping
- **THEN** conflicting ownership remains unlinked across later requests and reopening, while absent evidence is not guessed from other request metadata.

#### Scenario: Released connections survive startup recovery

- **WHEN** a data directory created by supported v0.3.1 with Claude Code and Codex connected starts the current build, including an unrelated historical operation that cannot be verified
- **THEN** startup succeeds, new sessions use the verified current grants to identify their Agents, and unlinked old sessions remain unlinked; missing current-grant proof produces bounded diagnostics and leaves observations unlinked without blocking startup.

#### Scenario: Agent settings interrupted before service completion

- **WHEN** Agent settings activation is interrupted after the target installation or active-publication commit but before its service-completion receipt is saved
- **THEN** startup remains available to complete journal recovery, missing proof does not fabricate attribution, and subsequent restoration associates new sessions once ownership is verified; accepted managed settings complete exactly, unrelated user native files remain unchanged, and previously unlinked observations remain unlinked.

#### Scenario: Committed service with a pending native-file tail

- **WHEN** the service publication is committed but an independent user edit leaves the Agent's native-file tail pending
- **THEN** startup accepts only the exact authenticated current-service receipt and journal as grant provenance, preserves the user file and pending-tail state, and does not export the unfinished connection grant; without that proof startup still permits journal recovery while attribution remains unknown.

#### Scenario: Old observations and Worker relations remain unchanged

- **WHEN** grant ownership is first registered, historical receipts are rebuilt, or a verified Worker run emits new observations
- **THEN** old sessions are not retrospectively assigned an Agent and Worker request/run associations retain their existing meaning.

Source Issue:
- https://github.com/higress-group/HiRoute/issues/182

Source SPEC comment:
- https://github.com/higress-group/HiRoute/issues/182#issuecomment-6292812078

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

#### Scenario: Native reasoning replay and maintenance calls

- **WHEN** a native client replays an accepted bounded plaintext reasoning summary or native
  `content` with one `reasoning_text` part, followed by visible output or a tool call
- **THEN** exact accepted reasoning/output evidence may establish the response edge;
  empty summary/text placeholders do not manufacture a history rewrite. Changed
  text, simultaneous populated summary/content carriers, extra parts, opaque
  provider state and unsupported fields remain unproven, and
  reasoning is never imported into competence-assessment content.
- **AND** bounded comparison retains the same meaning when request content moves
  from inline strings into request-owned Replay backing, including after earlier
  fields consume the aggregate inline allowance. A matching tool continuation and
  its accepted final answer retain one exact frozen request membership for the
  next input's assessment.
- **AND** recognized native title/compaction requests retain immutable receipts
  and monetary accounting while their explicit auxiliary marker excludes them
  from user-turn projection. Historical records without that marker remain
  unclassified; missing purpose evidence cannot hide an ordinary request.

### Requirement: Titles and folded context do not rewrite recorded content

Session title comes from the first readable real user input in request/message
order. Empty messages and standalone fully matched trimmed
`<environment_context>...</environment_context>` blocks are skipped; partial tags,
plain mentions, or trailing real text are not. Complete leading Claude
`<system-reminder>...</system-reminder>` blocks may be skipped to expose the remaining
question. Context-only sessions use a neutral title and update when later real input
becomes readable, including across fixed-range pagination and home activity refresh.

Session title scans skip only requests with affirmative retained native auxiliary
purpose evidence, including when those requests precede the first main request or
fill a timeline page. The same selection supplies the preview's model and route.
Raw request lists, direct request excerpts, search hits, receipts and costs retain
auxiliary calls. Missing, legacy, deleted or conflicting purpose evidence remains
unclassified and cannot hide a request or prove human origin. Auxiliary-only
sessions use a neutral title; this is not evidence of content cleanup.

Request excerpts show a pending state before visibility-triggered reads and while
loading. Empty or failed reads are unavailable, not deleted. A cleared-content
label requires explicit deleted content evidence with no unresolved catalog gap.
These presentation states do not claim that a particular historical request lost
its body or that all session content is complete.

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
Four reader permits and the real 1 s SQLite deadline remain; timeout is retryable
failure, never empty success. Failure clears continuation and retry starts fresh.

#### Scenario: Search repeated native conversation history

- **WHEN** retained native traffic contains many repeated content occurrences and
  large request metadata, while authorized indexes are ready
- **THEN** live and scoped conversation search check source readiness within the
  existing query budget without reading every occurrence's metadata; genuinely
  missing authorized indexes remain partial rather than complete empty results.

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

Extension stages MUST retain the extension identity and optional bounded reason without a fabricated probability, threshold or floor. Optional valid scores remain stage observations and enter ordinary score averages. Original supported envelope bytes and canonical digests remain unchanged on reopen; historical schema digests cannot authenticate new extension fields.

#### Scenario: Extension stage is assessed later

- **WHEN** an extension-selected stage later receives a valid complete score
- **THEN** the recorded score MUST remain on that actual stage without inventing a HiRoute floor or rewriting its selection.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: Management observation reads share one bounded budget

SQL-backed management observation reads SHALL have a default query budget of
one second. V2 entry admission, deadline-aware analysis control/price reads and
storage queries share the same absolute deadline; a nested reader or later SQL
statement SHALL NOT restart it. The query deadline bounds SQLite work and
existing deadline-aware composition, not all legacy or trust metadata operations
outside those owners. It is not a hard end-to-end latency guarantee.
An explicitly shorter deadline or cancellation remains effective. Successful
reads return immediately rather than waiting for the budget to expire.

The four-reader limit, at-most-100-ms SQLite lock wait, existing row/byte limits,
authorization, deletion and cursor rules remain. Timeout SHALL interrupt the
read and retain its existing error meaning, never return empty-success evidence.
This budget does not change Gateway execution, background ingestion, capacity
maintenance or automatic retry policy.

#### Scenario: A composed reader cannot extend its caller's deadline

- **WHEN** a bounded observation read creates a nested reader or starts another SQL statement
- **THEN** it retains the original deadline or an explicitly earlier one, even when the default budget is longer.
- **AND** cancellation and deadline exhaustion stop active SQL without taking the Observation writer.

### Requirement: Quality summaries and drill-down share one authorized snapshot

Plan quality defaults to the selected published version's latest seven days.
Backend aggregation covers the full authorized retained scope, using the same
snapshot as stage pages. Pagination, selected model, and detail score filters cannot
change the mean. Count a stage once using its latest complete valid score; valid
zero counts, unrated/partial do not. Versions, categories, groups, and model/profiles
remain separate. Failed candidates that did not serve the stage receive no score.
Configured candidates without execution display no records, not zero.

A fresh page SHALL return its consistent authorized read snapshot when ordinary
ingestion commits concurrently; new records enter a later refresh. Visibility
changes, including content or fact deletion, SHALL invalidate the in-flight read.
Continuation cursors retain strict population-generation and reader/query bindings;
a concurrent population change SHALL NOT silently mix summary and detail pages.

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

Plan-quality reads opting into `include_routing_observations=true` SHALL expose
bounded-read failures through the existing `hiroute.observation.read-failure/v1`
details. Timeout and reader exhaustion remain retryable failures with their
distinct reasons and bounded retry hints, never empty success. This does not
increase the read deadline, reader count or automatic retries. Omitted/false
retains the released strict failure shape.

Current observation v2 callers may explicitly request `read_failure_details=true`
to receive the same typed bounded-read failures, including Home sessions and usage.
Omitted/false retains each view's released failure shape and canonical request
identity. The option participates in the exact protected request digest; it grants
no access and changes neither reader limits nor data semantics. Home shares the
bounded observation queue, retries only reader-busy failures at most twice after
the initial attempt, and cancels obsolete queued/retry work when refreshed or
unmounted. Genuine failures and exhausted retries retain the last loaded data and
an explicit per-domain Retry action; unavailable reads never become empty success.

Extension scores without a floor MUST NOT be classified as below-floor or meets-floor and MUST NOT be relabelled unscored solely because no floor exists. Their valid complete scores still contribute to mean scores and scoring coverage.

#### Scenario: Read routing evidence beside unrelated retained facts

- **WHEN** a Plan-quality read includes routing observations and the workspace retains many unrelated safe facts
- **THEN** routing-decision selection does not scan every unrelated safe body. Its result, completeness rules and existing read deadline stay unchanged. Reopening an existing store may add a rebuildable index without changing retained bytes or the schema marker; malformed retained JSON remains preserved and the affected read fails rather than claiming complete evidence.

#### Scenario: Stop routing statistics at the retained-read limits

- **WHEN** matching retained routing decisions exceed the existing decision or safe-byte read limit
- **THEN** routing statistics stop consuming matching rows at that limit without first filtering, joining or sorting the entire matching population. The result marks its counts as partial and withholds exact-population rates. Historical stage and timeline ordering and pagination remain unchanged; the internal scan order of a truncated statistic does not define conversation chronology.

#### Scenario: Read stage selection beside unrelated event payloads

- **WHEN** a Plan-quality stage page resolves its recorded route selection beside other event kinds in the same request
- **THEN** the reader uses the retained event's covered identity and excludes known unrelated safe projections before opening sensitive payloads. A missing safe projection retains the original payload lookup, while deleted payloads remain unavailable. Small decision fields use rebuildable query indexes so cold reads do not repeatedly open unrelated content pages. Index creation preserves retained bytes and the schema marker; malformed payloads still fail the affected read. Selection, ordering, pagination and full-scope summaries retain their meaning within the existing read deadline and reader admission limits.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

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

### Requirement: Verified native compaction links preserve session identity

HiRoute SHALL retain the physical identity and exact request/receipt attribution of each native session while exposing a verified compression-continuation chain as one navigable conversation in opt-in analysis views. A managed native adapter SHALL obtain explicit compression lineage from its selected native profile through the native read-only interface; generic root affinity, equal content, timing and arbitrary parent headers are not proof. HiRoute SHALL admit a relation only within the same authenticated managed consumer, workspace and existing native-identity namespace. Only scoped HMAC identities and provenance are retained, never raw native IDs. Branches, delegates, explicit resets and unrelated sessions SHALL remain independent. Missing, malformed, conflicting, cyclic, ambiguous, truncated or unavailable lineage SHALL disclose the unproven boundary and SHALL NOT merge it, invent an origin or block otherwise valid model access. Provenance and retained visibility SHALL be rechecked on queries, including across restart; old records without evidence remain separate. The declared chain is navigable only through proven links and never rewrites physical sessions, immutable receipts, Worker identity, ContextHold or routing history. A verified session relation alone does not prove adjacent user turns. Conversation, search and cost results SHALL disclose and reuse one relation watermark across pagination and related drill-down, preserve member references, apply time/Plan/visibility filters to each request, and count each request/attempt once. Later relation additions SHALL NOT silently regroup a pinned result or invalidate it merely by advancing the live watermark. A used relation that is genuinely withdrawn or deleted SHALL cause explicit stale/refresh rather than silent regrouping. Scope creation SHALL select evidence stable through its declared lifetime so ordinary expiry does not invalidate an otherwise valid scope; upfront clipping and token expiry remain explicit. Current authorization and visibility always apply. Retained relation evidence SHALL survive reopening without making released physical-session/fact readers reject otherwise supported data. Missing or expired members stay explicit; legacy unflagged calls retain their physical-session meanings. Compatible released-writer mutations SHALL be reconciled from current retained source facts when HiRoute reopens: fully deleted or archived sources cannot remain charged, linked or recoverable through auxiliary projections, and retained newly ingested facts cannot be omitted indefinitely. Content-only deletion SHALL preserve its existing retained-fact/monetary meaning. Historical archives SHALL NOT recreate deleted detail. A normal reopen with already aligned retained projections SHALL reuse them without introducing reconciliation pending. Only views requiring known missing or stale retained projections may report catch-up pending, with scoped progress and a supported retry action; ready reads and valid scopes SHALL remain usable under current source/visibility checks. Interrupted catch-up SHALL resume without restarting for unrelated aware traffic, and stalled work SHALL report its reason rather than remain indefinitely plain pending. Missing historical evidence is an explicit gap, not unfinished reconciliation. Full facts-and-content deletion of a middle member SHALL withdraw its incident continuation edges and split the surviving chain at an explicit gap; queries SHALL neither traverse the deleted member nor synthesize a shortcut. Re-observing deleted historical lineage SHALL NOT undo deletion. Content-only deletion retains existing fact/grouping meaning, and genuinely new post-cutoff evidence follows existing revival rules without restoring deleted history.

#### Scenario: Read a verified compaction continuation

- **WHEN** a managed native request has valid compression ancestry in the same authenticated identity scope
- **THEN** analysis follows only proven compression edges, preserves physical identities and original receipts, and can navigate the retained conversation across the boundary.

#### Scenario: Keep forks and unproven ancestry independent

- **WHEN** lineage is absent, conflicting, unsupported, ambiguous, cyclic, incomplete, or describes a reset, branch, delegate or another authorization scope
- **THEN** no unproven edge is used; ordinary model access continues and the result discloses separate or partially linked members with the missing boundary.

#### Scenario: Read and value a chain once

- **WHEN** conversation/search/cost views select the same proven continuation chain
- **THEN** they reuse one disclosed relation watermark across pages and drill-down, preserve visible member references and resolved time bounds, and count each request and consuming attempt once; repeated captured input is not new usage.

#### Scenario: Preserve history through reopen and visibility changes

- **WHEN** HiRoute restarts, content expires or is deleted, old records lack lineage, or a relation changes during pagination
- **THEN** retained proven relations remain queryable subject to current visibility; unavailable members and unproven old relations stay explicit, and continuation cannot silently change its grouping or rewrite receipts.

#### Scenario: Analyze while new continuations arrive

- **WHEN** a ranking or conversation result is paged and opened in another supported view while later compression relations are admitted, including from the querying Agent itself
- **THEN** the caller can reuse the original relation watermark; additions after it neither rebucket its rows nor invalidate its continuation, while a fresh analysis may include them.

#### Scenario: Withdraw invalid pinned proof

- **WHEN** a previously used relation is genuinely contradicted or explicitly deleted during a valid scope
- **THEN** the result reports stale proof and a refresh action; routine expiry was excluded by the upfront retained scope and unrelated changes do not invalidate it.

#### Scenario: Preserve compatible released readers

- **WHEN** verified continuation evidence is retained alongside otherwise supported physical-session and request facts
- **THEN** its presence alone does not make a released physical-session/fact reader reject the page or storage; existing supported upgrade and complete-backup recovery contracts still apply.

#### Scenario: Keep retained analysis stable during ordinary expiry

- **WHEN** new analysis near the retention boundary is paged while normal expiry runs
- **THEN** requested versus effective bounds, clipping and lifetime were disclosed at creation and the selected proof/population remain stable through that lifetime, without extending retention or exposing deleted data.

#### Scenario: Reconcile after compatible released-writer mutations

- **WHEN** a compatible released writer deletes facts, expires requests or ingests retained traffic while unaware of auxiliary tables, then the current reader reopens
- **THEN** queries use live sources and precise deletion scope, purging stale associations and deriving missing retained inputs without duplicate charges, archived reverse lookup or resurrection; content-only deletion retains its lawful monetary facts.

#### Scenario: Reopen an already aligned store promptly

- **WHEN** HiRoute restarts over compatible retained data whose derived projections are already aligned
- **THEN** it reuses them immediately rather than putting all analysis views into reconciliation pending; missing historical lineage still remains a gap.

#### Scenario: Report and resume only real projection catch-up

- **WHEN** a compatible writer leaves retained inputs missing or stale, or catch-up is interrupted
- **THEN** affected reads expose scoped progress and retry guidance while ready reads remain usable; catch-up resumes rather than restarting for unrelated current writes, and stalled work exposes its reason.

#### Scenario: Delete the middle of a continuation

- **WHEN** A to B to C is a proven continuation and B is fully deleted
- **THEN** the adjacent A to B and B to C edges are withdrawn, A and C remain separately readable with a gap, no A to C shortcut or transit through B is invented, and old lineage cannot re-create the deleted edges; content-only deletion instead preserves grouping and retained money.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6229104803

### Requirement: C observations cover all retained decisions

Routing evidence MUST reuse existing route events, bounded safe-field storage
and authorized query entrances, with no new table, SQL migration, view or permission.
Every current routed classifier decision records its selected protocol (native
legacy, applicability C v1, custom extension v1 or local rules), independently of
inherited/fallback source; project that marker into safe facts. Known non-C counts
are separate and never enter C denominators. An authenticated supported pre-applicability envelope proves a known non-C decision without identifying its classifier family. Retain only its bounded format evidence in the existing safe projection, with routing/time identity, after verifying the original digest through the supported reader. Missing markers alone remain unknown; never infer protocol from Plan identity or rewrite authenticated facts. Missing, invalid, unsupported or already-deleted source evidence cannot be invented.

C detail records full-content/bundle identity, authored default/custom assessment standard, provider/model,
target/completeness/eligibility, applicability dispatch/state/reason/probability,
P/T, s/F, fixed 0.5 applicability boundary, action and bounded byte/turn counts.
Selection and actual execution/relay remain separate. Safe metadata includes no
prompt, conversation, tool-result, credential or free-form diagnostic text.

Existing observation query versions and the optional include_routing_observations flag MUST remain. Omitted/false preserves released strict request/result fields, nested selections, errors, authorization encoding and cursor bindings. True exposes bounded facts detail and plan-quality summaries through the same protected operations, with the flag bound to opt-in cursors. Do not add a query version or separate legacy-v2 business path. A stage-opening sample is not an all-decision denominator.

Summaries MUST cover all matching authorized retained route decisions independently
of page, model/score/grade filters, scoped by Plan/session, time, configured
thresholds, protocol/content/standard and provider/model. Exclude inherited records
as new dispatches and deduplicate originating event identity. Report retained and
covered scopes, missing projections, old unknown markers and scan limits. Mixed current C/non-C scopes, including proven supported pre-applicability records, can report complete coverage and C rates; unproven old/current records or incomplete evidence withholds exact-population rates while preserving known counts.
Deleted data is outside retained scope and its missing population cannot be invented.

Q3 unavailable/requested uses actual dispatch as denominator. No-target/partial
omissions and inherited decisions are separate. Eligible complete finite low history and a recorded Applied, RetainedUnavailable or Suppressed action define protection opportunities, independently of simple probability. Suppressed/opportunities and suppressed/fresh C decisions describe recorded historical-constraint releases, not necessarily actual group changes or avoided Primary execution. Old complex/not_needed observations remain outside that opportunity population. Required-question-missing
violations retain protection and count separately as internal anomalies, excluded
from provider-unavailable/requested totals. Zero denominators are unavailable.
These rates describe behavior, not ground-truth accuracy or task quality.
Content-only deletion preserves safe evidence under existing retention; facts-and-
content deletion/expiry removes it. Existing reader authorization, run conflicts,
idempotency and deletion boundaries remain effective.

Statistics use protection actions recorded under the rules active at the time. A retained scope may contain records from different rule versions; these counts and rates do not establish a uniform-policy effectiveness comparison. Do not invent a rule version from dates or Plan identity, relabel old events, or backfill hypothetical protection opportunities.

Prior source:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235111513

#### Scenario: All decisions, not open stages

- **WHEN** several C decisions occur inside one stage and a stage-score filter or page is selected
- **THEN** Counts MUST include all matching retained decision events while keeping inherited and omitted cases out of the requested denominator.

#### Scenario: Content deletion

- **WHEN** only captured content is deleted and safe facts remain within retention
- **THEN** Safe C detail/counts MUST remain queryable without restoring body content; full fact deletion removes them.

#### Scenario: Released read

- **WHEN** a facts or plan-quality request omits the new opt-in flag
- **THEN** The response MUST preserve its prior field set, including nested stage selection.

#### Scenario: Proven pre-applicability format

- **WHEN** supported pre-applicability decisions with authenticated format evidence share a scope with current applicability decisions
- **THEN** Known non-C counts MUST include only the proven older decisions, outside applicability denominators; complete scopes retain rates before and after content-only deletion and reopen.

#### Scenario: Missing evidence stays partial

- **WHEN** a current marker is lost or a source/projection is missing, invalid, unsupported or already deleted without retained proof
- **THEN** Coverage MUST remain partial and exact-population rates unavailable; Plan identity and marker omission cannot replace evidence.

#### Scenario: Recorded opportunities across difficulty and rule history

- **WHEN** one retained scope contains simple Applied, complex Applied, complex Suppressed, RetainedUnavailable, old complex NotNeeded and an inherited duplicate
- **THEN** the complete opportunity count is four and the suppressed count is one; inherited/old NotNeeded are excluded and the UI discloses historical-rule comparability limits.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/226#issuecomment-6445348771

### Requirement: Trial evidence discloses scoring coverage and execution scope

Trust reports MUST scope by Plan, revision, time and optional category. Rows retain
execution group, exact nullable model/reasoning identity and attribution; optional
executor grouping distinguishes the main Agent, delegated Harness and unknown.
Mixed identity cannot be credited to a single model. Complete and partial scores
are classified first. A stage without a score is awaiting assessment only when no
later turn exists and its Worker task is not terminal; other unscored stages stay
unscored. Coverage is complete scores divided by executions; competence is complete
scores meeting their frozen floor divided by complete scores that have a real frozen floor. Partial, awaiting
and unscored stages enter coverage's denominator, not competence's denominator. Complete extension scores count as scored coverage and may contribute to averages, but have no floor and enter neither competence numerator nor denominator. A pure extension population has no competence rate.

Reports disclose protection triggers, availability relay, recorded request duration
and output rate where available, contributing sessions and retained excerpts.
Exact-row drill-down preserves null identity and optional executor filters while
leaving the full-scope quality summary unchanged. Rolling aliases, dynamic routes
and unknown identities state that evidence may mix provider versions. Announcements
split earlier/day/later evidence without certifying an executed version.

Executor task outcomes are a separate population: tasks first submitted within the
selected Plan/revision/time scope, using the latest run state, including tasks with
no execution stage. Missing latest-run evidence is unknown. Continued tasks are
identified independently. Completion is neither competence nor delivery acceptance;
completion without a complete score remains unscored. Different executors are not
ranked because their task populations differ. Unavailable cost comparisons are
explicitly unavailable, never zero or a claimed saving.

The cost comparison SHALL select requests by the disclosed Plan revision, category and Regular group through retained stage identity and reuse the existing per-attempt API-equivalent calculation. Missing/nonunique Primary or saved currency and genuinely dirty/missing request projections make it unavailable until ready; known out-of-scope membership cannot block the selected scope. A retained authenticated native maintenance purpose also rules a request out of user-stage scope without removing its generic costs or receipts. Missing, deleted, ambiguous or invalid purpose evidence does not prove exclusion. Purpose verification has per-query byte and lookup budgets; unchecked evidence stays unresolved and in the full denominator. These budgets do not truncate the request population. Partial history alone does not invalidate proven membership; request-level usage and price completeness still apply. Closed mixed/unknown/conflicting membership is an explicit gap. A request without closed-turn membership remains pending while its physical session has retained activity within five minutes; after five minutes without activity it becomes unresolved evidence, without asserting task completion. Late facts reclassify the request on the next read, including after reopen. Unresolved normal requests in the Plan/time window that cannot be ruled out of the selected scope remain in its conservative denominator without an invented revision/group. They count once as missing and excluded (excluded stage gaps are a subset of missing), prevent complete coverage and full-scope amounts/difference, and allow only paired complete-request subtotals. No compared evidence is unknown, never zero. Decision traffic remains outside this population; generic comparison and settlement meanings stay unchanged. The view SHALL disclose the conservative denominator and overlapping missing/excluded counts, and SHALL NOT imply cash savings.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235188590

#### Scenario: Completed Worker without an assessment

- **WHEN** a Worker succeeds but its model stage has no complete assessment
- **THEN** task completion and unscored execution are reported separately; neither
  supplies competence evidence for raising trust.

#### Scenario: Interrupted and late trial cost evidence

- **WHEN** a report contains partial capture, mixed membership and an unclosed turn beyond the inactivity bound, then receives a late closing fact
- **THEN** it discloses conservative missing/excluded counts and paired subtotals, reclassifies late evidence without duplication and keeps genuinely dirty request projections unavailable until catch-up.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: The routing timeline re-emits late evidence without duplicates

The tool-continuation count SHALL use the recorded `InheritedToolContinuation` decision reason. `ContextHoldApplied` records candidate affinity, which may also apply to a freshly classified user message, and SHALL NOT count as tool-continuation evidence. Task-root inheritance or a retained decision without an explicit tool-continuation reason SHALL NOT invent that evidence. Pending, closed and reopened timelines SHALL apply the same rule without rewriting retained execution facts.

Session detail MUST show actual agent-turn selection, boundary, group, model/reasoning, probability/threshold, continuations, relays, assessments and request usage/cost beside retained conversation. New AgentTurnFinished writes SHALL record an optional opening cause, preserved across tool continuation and completion: continuing message history with an appended user message means UserMessage, independently of whether the prior decision can be inherited; fewer canonical messages than an existing comparable checkpoint means HistoryShortened; an equal or greater count with a changed prefix means HistoryRewrite; absent checkpoints (first entry, expiry, eviction, contention or disabled tracking) mean NoComparableHistory; other boundaries, including a Plan change without an appended user message, mean Other. System/developer instructions are excluded from the message count. Changed instructions or model/tool setup invalidate exact execution inheritance even when conversation text is unchanged; that invalidation alone does not establish compaction.

Explicit causes override legacy hints. UserMessage displays New message. HistoryShortened displays Context rebuilt and is attributed to compaction by the owner-selected step-count heuristic. A step is one canonical request message, not a token, tool or timeline turn. Details disclose that a shorter rewind or shared-scope background/subagent request can also satisfy this heuristic. HistoryRewrite and Other remain unknown. NoComparableHistory displays New message only for the first session turn. Without the field, legacy inference displays New message for the first session turn or a fresh opening decision (not inherited) with ContextHoldApplied or ContextHoldInvalidated. A retained tail with a known later ordinal is not the first session turn. Invalidation alone never proves compaction. Unknown headings omit the badge and details disclose the gap. Supported old bytes and authenticated digests remain readable unchanged; unreleased experimental digests are not admitted.

Opening causes SHALL distinguish the observed request-input boundary from accepted-response association. Reused branches SHALL also require actual current input after the prior request before treating a native activation-identifier change as a new input. An exact tool continuation with no appended current input SHALL NOT be reclassified, closed or assessed merely because that identifier changes. On an independent branch, a fully matched prior request prefix and an actual current input entirely after that request may establish UserMessage without a complete intervening response. Comparable candidate boundaries must agree; a newer ingress retry, overlapping input window or explicitly unavailable current input cannot be overridden by an older matching prefix. A reused native identifier SHALL NOT hide an actually appended current input. Explicit continuation evidence selects the saved input by making the current input unavailable; the identifier alone does not establish that meaning. Missing digests or a checkpoint position not measured before concurrent advancement mean Other, not proven HistoryRewrite. A changed comparable request prefix retains HistoryRewrite. This evidence grants no predecessor reuse, affinity, completion or stronger assessment association; the interrupted or unknown prior stage and its frozen partial target remain unchanged. The independent branch captures its own actual execution and starts its own assessment segment.

The view SHALL disclose that client background messages can satisfy the new-message rule, so continuity does not prove human authorship. Aliases, tools, native activation/window identifiers and cache presence SHALL NOT establish native provenance. Concurrent execution membership follows the independent branch contract below; an unknown main/child relationship remains explicitly uncertain. The legend states that an eligible exact continuation keeps its model and new messages or rebuilt context trigger fresh judgment.

The opening request supplies stable display identity before completion. Late assessments and valuations update that turn. When late authoritative request membership absorbs provisional rows into a completed turn, Desktop SHALL remove the absorbed rows by their exact opening-request identities, transfer their selection/focus to the owning turn, and reconcile overview, request and turn counts. Names, timing or model similarity SHALL NOT infer membership. Incremental cursors MUST be authenticated and bound to reader, session, page size and retained visibility; pagination freezes fact, valuation and link positions until its changed page is consumed. Reconnect deduplicates equal content digests. Cleared or expired scope invalidates the cursor. Unknown usage/cost remains unknown; per-model currency and valuation basis are preserved. A per-model sum SHALL retain available known amounts when another contribution is unknown, disclose that it is a known subtotal and retain the missing contribution. No known contribution means amount unknown, not zero. Frozen API-equivalent amounts SHALL be labeled estimates in summary and request detail.

Reads retain shared authorization, a 1 s storage deadline, and explicit scope errors beyond 10,000 requests, 50,000 facts or a 1 MiB response. Deadline exhaustion is a transient read timeout, not evidence that the retained population exceeds a capacity limit. Cache locks cover only take/return, never database reads, projection or serialization. Visible Desktop timelines refresh every two seconds; hidden views stop polling and catch up when shown. Transient failures retain the last successful view, disclose that it is awaiting refresh, and retry with bounded backoff while visible. Actual capacity failures require explicit Retry. Busy readers honor the bounded retry hint while retaining content. CLI --follow emits NDJSON through the same query, preserves terminal errors, remains interruptible while busy and retries at most three consecutive transient unavailable failures. Live and retained readers use recorded endpoint/start-time membership and the same retained transition classifier; unrelated timestamp ties stay unassigned.

Selected-session summary metadata and visible unresolved excerpts SHALL refresh independently of the turn-fact digest. Refreshing an existing session SHALL preserve selection, loaded list pages and scroll position; a stale response from an earlier selection SHALL NOT replace the current session. Transient content failures and pending content SHALL NOT be described as proven deletion. Content cleanup invalidates visible excerpts and later reads SHALL respect current visibility instead of restoring cleared content.

Displayed assessment ranges SHALL resolve exact target Agent-turn identities through the same chronological display coordinates as the overview and lanes. Agent-turn identity is distinct from the opening-request identity used for stable timeline selection. Producer-local ordinals SHALL NOT stand in for displayed turn numbers. A target outside the loaded or retained range, or with ambiguous identity, SHALL be disclosed without inventing a number. Plan names and execution model identifiers SHALL be labeled by their distinct meaning; a friendly model name requires an authoritative mapping, and the exact execution identifier remains available.

Timeline transition summaries SHALL distinguish a completed turn comparison from intermediate request comparisons. A complete authoritative comparison may subsume a redundant nonfinal unknown comparison only when both identify the same known predecessor and the same previous/current model and reasoning identities and change facts. Different endpoints, different models, pending or partial evidence, within-request fallback and credential relays remain distinguishable. A missing request-level comparison SHALL NOT be presented as a missing final turn comparison.

#### Scenario: A bounded timeline read times out and recovers

- **WHEN** a timeline within capacity limits temporarily exhausts its read deadline
- **THEN** the response identifies a retryable timeout, Desktop retains its last successful view and retries while visible, and recovery resumes updates without manual navigation.
- **AND** a real request, fact or response-size limit still produces a capacity failure rather than an unbounded read.

#### Scenario: Summary changes while a timeline is selected

- **WHEN** later requests update a selected session's last activity or content completeness
- **THEN** its header and existing list entry refresh without resetting selected turns, list pagination or scroll position, and an older response cannot overwrite a later selection.

#### Scenario: Excerpt becomes readable without another turn fact

- **WHEN** a visible excerpt's content becomes readable after an empty or failed read while its turn digest remains unchanged
- **THEN** a bounded recheck displays the content; inactive or hidden views stop rechecking and resume when visible.
- **AND** cleanup removes the excerpt and a stale in-flight read cannot restore it.

#### Scenario: Confirmed cleanup is distinct from content awaiting capture

- **WHEN** successful cleanup or current session metadata confirms deleted or expired content, including when request counts, activity timestamps and turn digests remain unchanged
- **THEN** visible titles and excerpts discard cached text, show cleared content and stop body retries, including after reopening the session. An empty catalog alone is not proof of deletion.
- **AND** a later request may make new session content readable without restoring content from requests already confirmed cleared.

#### Scenario: Assessment after a producer epoch changes

- **WHEN** a producer restarts its local ordinal sequence and a later displayed turn has an assessment
- **THEN** the assessment uses its exact target identities to name the displayed turns, or explicitly reports a target outside the available range.

#### Scenario: A multi-request turn gains a complete comparison

- **WHEN** a completed turn has an authoritative complete comparison and an earlier redundant unknown comparison of the same known predecessor and model identities
- **THEN** the turn summary presents the complete comparison while retaining request records and all distinct request-level changes or evidence gaps.

#### Scenario: Assessment arrives during pagination

- **WHEN** an earlier turn receives a score after the first page was delivered
- **THEN** the remaining frozen page is delivered first, followed by the updated
  earlier turn once; reconnect cannot duplicate its unchanged projection.

#### Scenario: Completion absorbs a provisional request row

- **WHEN** a later completed turn retains its opening-request identity and includes
  a request previously displayed as a provisional row
- **THEN** the live view removes that provisional row, preserves selection on the
  completed turn, and shows each request and Agent turn once without a failed read.

#### Scenario: Focus survives a provisional turn's completion

- **WHEN** the focused overview mark or lane button belongs to a provisional turn
  absorbed into a completed turn
- **THEN** actual keyboard focus moves to the completed turn's corresponding control,
  with selection and visible keyboard focus retained.
- **AND** completion leaves focus unchanged when it is outside the absorbed controls.

#### Scenario: Some model amounts are known

- **WHEN** a model has known recorded amounts and another request has unknown value
- **THEN** the card shows the known subtotal with an unknown-part disclosure,
  preserving currency and valuation kind; an entirely unknown model stays unknown.

#### Scenario: Fresh decision on continuing history

- **WHEN** an old turn without an opening-cause field has a fresh opening decision recording ContextHoldApplied or ContextHoldInvalidated
- **THEN** the turn shows a new-message boundary; a group change is never labelled as compaction.

#### Scenario: A new input follows an interrupted tool stream

- **WHEN** the old request prefix is unchanged, the client replays delivered text but omits an unfinished tool call, and an actual new input follows the complete old request
- **THEN** its independent admission records a new-message boundary while the old stage remains interrupted and partial; no old response edge, decision inheritance or affinity is invented.
- **AND** subsequent complete execution and assessment use only the new branch's actual members, including when both branches execute on the same model and group.

#### Scenario: A changed native identifier does not end a tool stage

- **WHEN** a request exactly replays a captured tool call and its result, its validated input window still selects the prior input, and only its native activation identifier changes
- **THEN** the request continues the same stage and decision without an intermediate closure or assessment.
- **AND** a later actual new input may open a new stage and assess the prior stage's complete, frozen request membership.

#### Scenario: A repeated native identifier does not hide a new input

- **WHEN** a request has a matched prior request prefix and a validated current-input window entirely after that prefix, even though its native activation identifier is unchanged
- **THEN** it opens a new input stage and requests a fresh decision under the configured new-input policy.
- **AND** only a separately proven response association permits reuse of the prior stage's frozen assessment membership; explicit continuation evidence with no current input does not satisfy this rule.

#### Scenario: A missing comparison is not a changed prefix

- **WHEN** a bounded digest is unavailable or a concurrent checkpoint advances beyond measured positions
- **THEN** the independent opening remains unknown rather than claiming a history rewrite; routing and capture continue with the existing association limits.

#### Scenario: Unknown and retained-tail boundaries

- **WHEN** a later turn lacks continuity evidence or only inherits a decision, including a retained tail with a known later ordinal
- **THEN** its boundary remains unknown, its heading omits the badge and its details disclose the missing evidence.

#### Scenario: Background message satisfies the same rule

- **WHEN** the client appends a background message carrying fresh-decision and continuing-history evidence
- **THEN** the new-message boundary applies with an explicit disclosure that the gateway does not establish human authorship.

#### Scenario: Shorter history uses the compaction heuristic

- **WHEN** the current canonical message-step count is smaller than an existing comparable checkpoint, including a compacted replacement or shorter rewind
- **THEN** the recorded HistoryShortened cause displays Context rebuilt with compaction attribution and its limitation; equal-length or growing prefix edits and unavailable checkpoints remain unknown after the first session turn.

#### Scenario: System instructions change without rewriting messages

- **WHEN** the system instructions change while messages still extend the prior checkpoint
- **THEN** message history remains continuing even if the hold preference is invalidated; an appended user message can start a new-message turn.

#### Scenario: Recorded cause overrides legacy inference

- **WHEN** a turn contains an explicit opening cause and an old-style ledger hint suggests another cause
- **THEN** the explicit cause wins, remains stable through tool continuation and reopening, and absent legacy fields retain their previous canonical bytes.

#### Scenario: New user message after a Plan revision

- **WHEN** messages continue the prior history and append a user message after the Plan changes or without an inheritable prior decision
- **THEN** the turn records UserMessage and displays New message while obtaining a fresh decision; a Plan change without an appended user message remains Other.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235190304

### Requirement: Model performance compares a model across executing Agents

For a model and reasoning configuration in scope, the model performance view SHALL be able to group its stage evidence by executing Agent. The executing Agent is the main Agent of a connected session, or the executor of a delegated task such as DeepSeek Harness, and each group SHALL name the Plan that produced it. Each group SHALL show stages, scored and unrated counts, competence rate and observed duration on the same definitions as the rest of the view, and SHALL drill down to its sessions or tasks. For a Plan with task delegation enabled, the view SHALL also show delegated task outcomes in scope: submitted, completed, failed, cancelled and continued. These SHALL use the existing task projections. Task outcomes SHALL be shown separately from competence: a completed task without a complete valid score SHALL count as unscored, not as competent, and task completion SHALL NOT be presented as delivery acceptance. An executing Agent's name does not identify its installed version, and evidence from one executing Agent SHALL NOT be presented as evidence for another. Single-group custom branches SHALL show their recorded assessments under these definitions even though they do not judge difficulty. The view SHALL state that the groups handled different tasks, and SHALL NOT rank Agents or harnesses. Unknown executing identity SHALL remain a separate group.

#### Scenario: Same model in two Agents

- **WHEN** DeepSeek V4.1 Flash served stages in Claude Code through a difficulty-routing Plan and in DeepSeek Harness through a delegation Plan
- **THEN** the view shows one group per executing Agent with its Plan, rates and counts, together with the note that the tasks differ, and opening a group lists its sessions or tasks.

#### Scenario: Delegated task outcomes

- **WHEN** a delegation Plan's scope contains eleven tasks, of which ten completed and one was cancelled
- **THEN** the view shows those outcome counts from the task projections, and selecting cancelled opens that task.

#### Scenario: Unknown executor

- **WHEN** some stages lack a recorded executing identity
- **THEN** those stages appear in a separate unknown group and are not merged into either Agent.

#### Scenario: Completed but unscored task

- **WHEN** a delegated task completed and its only stage has no complete valid score
- **THEN** the view counts the task as completed and its stage as unscored, and the competence rate does not change.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6236472910

### Requirement: Routing timelines explain historical protection at the opening selection

The session routing-timeline read and public CLI follow output MUST expose the opening selection's recorded historical-protection outcome, previous stage score and competence floor when available, and applicability probability when valid. The result distinguishes applied protection, released protection, protection retained because judgment was unavailable, and protection not needed. Unconsidered or unrecorded evidence remains explicit; it MUST NOT imply a release or zero probability. The lane marks released protection distinctly from an ordinary unprotected selection. Release removes only the historical constraint: simple work can continue with Regular, while complex work remains Primary. Protection may follow an eligible previous Regular or Primary stage; a previous Primary does not imply a third tier or a Regular-to-Primary transition. Selected-turn details explain the current task's simple probability and threshold, the previous stage score and floor, and whether the earlier problem still matters, ending with the actual recorded group. The previous score comes from the opening decision, not the selected turn's later assessment. Continuations inherit the opening selection and are not additional protection decisions. Existing selection, read and follow meanings remain unchanged. This view does not alter routing, stored facts or assessment semantics. Labels use plain capability names and exclude internal protocol labels, digests and question identifiers.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6253018980

#### Scenario: Protection applied

- **WHEN** an opening selection records simple probability 0.94 at threshold 0.90, previous score 0.30 below floor 0.50, applicability probability 0.99 and applied protection
- **THEN** the timeline marks protection and explains that the problem still matters and this turn went to Primary

#### Scenario: Protection released

- **WHEN** the same simple and low-score inputs have applicability probability 0.05 and recorded released protection
- **THEN** the lane marks released protection and explains that the problem no longer matters and Regular continued

#### Scenario: Judgment unavailable

- **WHEN** the record retains protection because applicability judgment was unavailable
- **THEN** the timeline explains the unavailable judgment and retained Primary protection without inventing a probability

#### Scenario: No protection and missing evidence differ

- **WHEN** a record says protection was not needed, or historical-protection evidence is absent or unconsidered
- **THEN** the view distinguishes those states and never calls missing evidence released protection

#### Scenario: Complex work after protection release

- **WHEN** P(simple)=0.80 at T=0.90, eligible previous score=0.30 below F=0.50, applicability=0.05 and recorded Suppressed
- **THEN** the lane marks released protection while explaining the actual Primary/complex_task selection; it does not claim Regular continued.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/226#issuecomment-6445348771

### Requirement: Long routing timelines provide a navigable overview

Above the routing lanes, session detail MUST provide a compact overview with one chronological mark per retained agent turn. Marks use the same Regular and Primary colors as the lanes and identify recorded low scores or failures, protection, and released protection. An outline shows the visible lane interval and follows scrolling, resizing and newly received turns. Activating a mark scrolls its turn into the lane viewport and selects it without changing routing or calling a model. The overview stays within the supported small window and has one Tab stop with roving focus across its chronologically ordered marks. Arrow keys move focus, Home and End reach the first and last mark, and Enter or Space selects the focused turn. Each mark retains a readable accessible label. Tab leaves the overview without visiting every mark. Chronological keyboard access through the lanes remains available; the overview is not the sole accessible path to a turn. Unknown groups and unavailable evidence remain distinct. Existing live-arrival handling, explicit-selection retention and reduced-motion preferences still apply.

#### Scenario: Forty-turn overview

- **WHEN** a session has forty retained Regular and Primary turns with low-score, failure, protection and release evidence
- **THEN** forty chronological marks reflect that evidence and the visible-interval outline matches the lanes

#### Scenario: Navigate and resize

- **WHEN** a user activates an offscreen mark and resizes to the smallest supported window
- **THEN** the selected turn becomes visible, details match it, the outline follows the viewport and the page has no horizontal overflow

#### Scenario: Keyboard and live append

- **WHEN** turns arrive while the user navigates overview marks or lane controls by keyboard
- **THEN** the overview keeps one Tab stop and its readable labels, arrow navigation and activation work, and chronological lane focus and live-follow or explicit-selection behavior remain available as matching marks are added

#### Scenario: Scroll beneath fixed lane labels

- **WHEN** lane cards move behind the fixed label column during horizontal scrolling
- **THEN** the entire fixed column occludes those cards visually and for pointer hit testing, while keyboard focus remains visible and the focused turn can be brought into view.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6253019758

### Requirement: Routing model labels preserve execution identity

Desktop routing timelines SHALL prefer a readable current configuration name only through an unambiguous exact configuration-ID match. Original configuration/native identities remain inspectable and determine routing, pricing and scoring independently of display names. Unknown/deleted/conflicting names fall back without guessing by prefix, group or similar display name. Actual per-request identity and mixed execution remain visible; a last model cannot represent a mixed turn. Label lookup is bounded to the current authorized page lifecycle/explicit invalidation, does not block timeline/content on failure and is not repeated for polling, turn selection or pagination. Equal display names remain visibly distinguishable using proven current source labels; ambiguous/missing source evidence is explicit and distinct configuration identifiers remain accessible and visible when needed. A current source label is not proof of the historical executing Binding. Qualify a lane label as Latest only when multiple known model identities appear in the loaded/displayed lane; pagination does not prove the unseen history used one model. Mixed and unknown execution remain explicit.

#### Scenario: Readable exact identity

- **WHEN** a retained request has an exact current configuration with a display name
- **THEN** the name is shown with original identity available in details and no execution or price reinterpretation.

#### Scenario: Conflicting and mixed identities

- **WHEN** same-name configurations differ or a turn relays across models
- **THEN** no name-based merge occurs; request identities and mixed execution remain distinct.

#### Scenario: Label read fails

- **WHEN** candidate lookup fails or returns after the page/connection changes
- **THEN** facts remain readable using fallback and stale names do not cross the lifecycle boundary.

#### Scenario: One visible model, several models and duplicate names

- **WHEN** a loaded lane first contains one known model, later contains another, or different exact configurations share a name
- **THEN** a single model needs no Latest prefix; multiple known models qualify the latest identity within the displayed scope; duplicate names show proven current source or explicit unconfirmed/multiple-source text plus a distinct configuration identifier without guessing the executed Binding.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/227#issuecomment-6440843097

### Requirement: Timeline presentation distinguishes pending facts from missing evidence

An explicitly requested timeline presentation SHALL distinguish proven pending, resolved, mixed and unavailable group state from independent execution attribution. Pending requires validated opening admission and absence of route/execution/terminal/conflicting evidence; running alone is insufficient. Terminal attribution and actual request identities retain their original authority. Existing callers without opt-in retain response shape and cursor semantics; rich and legacy reads cannot corrupt each other's cache or miss updates. Live transitions preserve one turn, request membership, focus and selection. Content loading, running pending, terminal incomplete, error and privacy-cleared states remain distinct; missing usage never becomes zero.

#### Scenario: Opening progresses

- **WHEN** validated admission is followed by route, attempts, finish and a late score
- **THEN** the same turn transitions from neutral pending to its actual lane and late evidence updates without duplicates or lost focus.

#### Scenario: Running is not proof

- **WHEN** a running turn already has mixed/conflicting execution evidence or a terminal turn lacks group
- **THEN** it is mixed/unavailable with the real gap rather than pending.

#### Scenario: Legacy and rich readers interleave

- **WHEN** absent/false and true presentation queries use caches and exchange watermarks
- **THEN** old output remains unchanged; shape-crossing watermarks are stale and each reader sees its correct updates.

#### Scenario: Content is cleared during a read

- **WHEN** a late body result arrives after privacy invalidation
- **THEN** cleared content stays hidden; loading/error/incomplete remain distinguishable.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/227#issuecomment-6440844648

### Requirement: Timeline detail explains business state without hiding evidence

Timeline primary explanations SHALL use one low-score-protection and competence-floor vocabulary, readable reasoning settings and concise fresh-decision text. Raw boundary/protection evidence remains available; unknown or rebuilt boundaries retain visible distinction. A missing prior assessment target is not a claim that the first visible row is the session's first turn. A released protection does not promise Regular, and special unavailable/partial reasons remain truthful. The timeline overview retains one roving keyboard stop and focus through provisional-turn absorption.

#### Scenario: Retained tail has no target

- **WHEN** the first visible turn is a retained tail without an assessment target
- **THEN** the UI states no prior stage is available, not that the session is beginning.

#### Scenario: Protection and reasoning

- **WHEN** the reason is low_competence with provider-default reasoning, or suppression on complex work
- **THEN** one protection name and a readable default label appear; actual group/reason/raw scores remain inspectable.

#### Scenario: Keyboard transition

- **WHEN** a focused provisional turn is absorbed into a completed turn
- **THEN** focus and selection move to its corresponding completed control; unrelated focus remains unchanged.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/227#issuecomment-6440846153

## Implementation references

- [Native identity resolver](../../../crates/gateway/src/context_hold/identity.rs) and
  [real native-session scenarios](../../../tools/e2e-harness/tests/p0_gateway_native_sessions.rs).
- [Search contract](../../../crates/observation/src/query_v2/README.md),
  [live query](../../../crates/observation/src/query_v2/session_search.rs), and
  [receipt scenarios](../../../crates/observation/src/store/tests/store_flow.rs).
- [Quality aggregation scenarios](../../../crates/observation/src/store/tests/plan_quality/summary.rs)
  and [Desktop quality identity](../../../apps/desktop/src/features/plan-quality-state.ts).

### Requirement: New routing stages retain exact execution membership

Current AgentTurnFinished and BranchAssessmentRecorded facts SHALL carry optional
versioned `history_membership`: version 1, opaque execution branch ID, immutable
revision, up to 128 exact Gateway request IDs, an explicit clipped flag, relation
kind and separate source/association/capture uncertainty. Relation kinds are root,
exact_prefix, ambiguous_prefix and contextual. Producer/native request IDs cannot
replace Gateway IDs. Current writes use the single current envelope descriptor;
a registered previous descriptor remains read-only and cannot authenticate the new
field. Old authenticated bytes/digests remain unchanged.

For facts with membership, analysis, timeline, quality and cost consumers SHALL use
those members, never an inclusive ordinal or first/last-time interval across
concurrent branches. Missing/clipped membership is explicit incomplete evidence.
Later stage extensions cannot broaden an earlier assessment's frozen membership.
Facts without membership retain their supported historical reader semantics.
Rebuildable projections MAY be refreshed through a versioned migration; original
facts, sensitive carriers and their digests SHALL NOT be reset or rewritten.
Matched ordinary reopen SHALL not repeatedly rebuild current projections.

#### Scenario: Interleaved branches and a late stage extension

- **WHEN** A1, B1 and A2 interleave in one session and an assessment was frozen before A2
- **THEN** A contains only A1/A2, B only B1, and the earlier assessment does not claim to assess A2, including after the observation store reopens.

#### Scenario: Previous producer envelope is retained

- **WHEN** a supported pre-membership envelope is read after upgrading
- **THEN** its original digest and bytes still authenticate, historical projections remain readable, and adding new membership to that old descriptor is rejected.
