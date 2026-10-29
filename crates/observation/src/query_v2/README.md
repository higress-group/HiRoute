# Session content search

Management reads share `ObservationReaderContext::QUERY_BUDGET` (one second).
V2 admission and deadline-aware composition carry one absolute deadline; nested contexts only
shorten it. Four reader permits, the 100 ms SQLite busy cap, row/byte limits and
cancellation remain independent bounds. Gateway and maintenance budgets are separate.

Current `hiroute.observation.query/v2` retains the optional
`include_routing_observations` flag for facts and plan-quality reads. Omitted/false
preserves released strict fields, authorization encoding and cursor bindings.
Opt-in adds bounded routing evidence; it does not add a read permission, SQL table
or query version.

Request/timeline views may include `native_auxiliary:true` only when a retained
original route-decision carrier authenticates that native purpose. The optional
field is absent for unclassified requests, including legacy, deleted, conflicting
or out-of-budget evidence. Absence does not establish human origin. This view
adds no persisted state or safe-fact field. Purpose lookup keeps the existing
reader scope, transaction and cursor boundaries, checks at most two indexed
route rows per returned request, reads no more than 64 KiB per carrier and 1 MiB
across a page, and degrades optional purpose evidence to unknown at its bounds.
It does not remove requests, receipts or monetary accounting. Session preview
titles skip only the affirmative marker; direct request and hit reads remain
available under their existing content authorization. Title-only absence and
loading never establish content deletion.

Statistics cover all retained matching decisions in the authorized scope,
independently of stage pagination and model/score filters. Each current classifier
decision records C, native legacy, custom-extension v1 or local rules. Known non-C
counts stay separate from C denominators. Authenticated supported pre-applicability envelopes also count as known non-C. The existing bounded projection refresh verifies each original carrier/digest and retains only safe format, routing and time evidence. Marker omission alone proves nothing. Unproven historical/current records, unknown scope, missing or invalid projections and bounded scans withhold rates while preserving counts. Already-deleted carriers cannot be reconstructed.
Content-only deletion retains safe evidence; full fact deletion/expiry removes it
through existing retention, without a new table, migration, view or permission.
See [the contract](../../../../issue-spec/specs/session-history/spec.md) and
[all-decision query](routing_observations.rs). The remaining sections describe
independent session search.

Plan-quality reasoning reads filter known unrelated events through the existing
safe catalog before opening sensitive carriers. Missing catalog entries retain
the original payload lookup; deleted carriers cannot supply a reasoning profile.
An additive request/digest/sequence index covers event identity reads and locates
routing evidence by its exact digest. Opening an existing store creates the index
without changing its schema marker, immutable events or serialized contracts.
The shared query deadline and retention/deletion filters still apply.

Quality summaries count a score as covering the current stage only when its
retained membership is complete and covered by the frozen assessment. Explicit
clipping or partial capture defeats that coverage even if all retained IDs were
assessed before the stage grew. The original score and assessment membership
remain stored unchanged. Missing legacy membership retains its existing reader
behavior; source or association uncertainty alone does not exclude a complete
stage. [Store-ingest regressions](../store/tests/plan_quality/history_membership.rs)
cover the 128-member boundary and reopening without a later successful assessment.

Routing summaries stream through the covering safe-route index in unique
request/digest order, so both the decision limit and the reader's safe-byte
limit can stop work before scanning the remaining matching routes. A truncated
summary reports partial coverage and withholds population rates. This internal
scan order does not change historical stage or timeline pagination.

`session_search` is the Desktop conversation-search intent on the existing protected
observation operation. It uses the existing search filters and returns one
`ObservationTextAnchorV2` per matching session, newest matching request first,
then session ID ascending. Within that request it chooses the first matching
content occurrence/block. `limit` counts sessions, and `next_cursor` means there
are more matching sessions, never more directory windows to scan.

Each page is a live query under a fresh authorized reader. The authenticated
cursor binds the reader's identity/scope, normalized keyword, filters, limit and
last matching session's sort position. It does not pin request/content/index
watermarks or the global relation generation. Unrelated requests and newly
published indexes therefore cannot invalidate continuation. Changing query or
reader scope requires a new search. Authorization generation renewal in the same
scope is allowed only after the current grant is validated again.

Pages are not a historical snapshot. New matches before an already consumed sort
position, or sessions moved before it by new activity, appear after refresh.
The Desktop merges by session ID. Deletion/expiry and run-link conflict are checked
in every page and again for returned anchors after its short read transaction.
Opening a body independently checks current content permission and visibility.

Literal, case-insensitive substring semantics use the same per-character Unicode
lowercase mapping for keyword and stored blocks. The rebuildable SQLite keyword
lookup indexes unique one-, two- and three-character grams, encoded as ASCII
tokens. A folded keyword of at most three characters is a single complete gram,
so its lookup is exact. Longer keywords intersect their trigrams and verify the
folded substring before selecting a content occurrence. Chinese, punctuation and
short words need no word tokenizer. Anchors retain original UTF-8 byte offsets, including lowercase
expansion and overlapping/cross-block matches.
The live intent accepts up to 256 Unicode characters; it does not impose the
released occurrence intent's 256-byte limit on Chinese input.

Only digest-verified, live published sources with a complete keyword lookup can
produce hits. Existing released blocks are backfilled in bounded maintenance
cycles without rewriting requests, bodies, dates or identities. Interrupted
backfills remain unpublished and resume idempotently. Block deletion removes its
lookup entries in the same transaction. Missing/failed/pending authorized indexes
set `index_partial`; no ready match is a complete empty result only when that flag
is false. Four reader permits and the existing 1 s SQLite deadline remain;
exceeding the query budget is a failure/retry, not a misleading empty result or
an exposed scan-window cursor.
The SQL scans source identities through their index and tests the shared keyword
block-rowid set before checking source publication or reading folded text.
A rebuildable complete-content covering index supplies workspace/digest, content
kind, media type, request identity and the implicit occurrence rowid. It chooses
the first eligible matching content per request, then checks request authority
and filters once before ranking sessions. Both sparse matches and common short
keywords avoid loading every content occurrence or folded body. Returned anchors
always verify the exact folded substring and load original offset maps. Current
reads use returned content identities. A partial covering index on ready-source
identities keeps publication checks in metadata; it still requires the complete
lookup publication marker. Gap detection merges the ordered covering live-source
and ready-source indexes with `EXCEPT`, materializing only unready digests before
opening occurrence metadata under the reader's filters. It avoids both large
temporary complete-source sets and large metadata pages for repeated history;
missing indexes or incomplete publication still report authorized gaps.
Scale regressions include distinct source digests and common short keywords;
repeating many occurrences of a single blob does not cover these query costs.
The maintenance indexer limits metadata-only scanning separately from body input.
After a stable complete pass it sleeps until the existing source revision or its
writer connection changes; analysis-scope writes do not restart that scan. A
change behind an in-progress cursor requires another pass before declaring idle.
Unchanged analysis checkpoints perform no writes during ordinary idle maintenance.
Fresh analysis scopes and price captures share the existing capture writer's
admission within the caller's original deadline, so sustained native capture does
not repeatedly beat them to SQLite's shorter busy slice. Existing scope reads
retain their concurrent read transactions; content reads release the writer first.
Idle physical blob collection also merges covering identities on a separate read
connection. Only selected unreferenced candidates take the writer, and current
references and paths are rechecked before deleting a file. The batch bound stays
in the candidate merge, including a large orphan backlog.
Terminal content acknowledgement and missing-blob checks use the existing request
index to select their request/direction/fork before sorting content IDs. An ordering
index must not make these capture writes scan unrelated retained history while
analysis waits for the writer.

The released `search` intent retains occurrence pagination and its V2 response
shape. It is a distinct supported interface, not the Desktop session-list query.

Desktop shows searching, actual matching sessions, complete empty, partial index,
or failure with retry. A failure withdraws continuation and retry starts a new
query. Query changes invalidate delayed responses immediately and clear old
results so new keywords cannot act on a previous cursor.
Search results stay in the list until the user opens a matching conversation.
At the minimum window size an unselected search uses the available pane, and a
continuation failure keeps its retry action visible above the retained results.

Regressions belong beside `session_search.rs` and `text_index/lookup.rs`. Browser
scenarios exercise the production Sessions component; native acceptance must
type the keyword in Desktop, find a known body after a large nonmatching corpus,
continue across unrelated Worker activity and open the original anchor. Browser
fixtures alone do not establish authorization or product completion.

## Scoped analysis consumers

The opt-in analysis views live in [`../analysis`](../analysis/): `read` composes the existing bounded
content and stage owners, `scope` pins relation/numeric/price watermarks, `members` resolves physical
members for both content and money, and `comparison` shares source matching and per-attempt rounding
with the legacy explicit-reference report. Numeric inputs and native relations use additive auxiliary
tables, not new execution-fact/receipt variants or changes to shared storage admission markers.

Application composition supplies one 1 s deadline, including nonblocking control/price reads.
The existing reader semaphore and SQLite progress handler remain authoritative. Analysis scopes are
signed, expire within one hour and disclose retention-safe effective bounds; ordinary cleanup removes
expired membership/price leases. Used-source deletion and withdrawn proofs invalidate affected scopes;
append-only unrelated traffic does not. Released cursors retain their original validation rules.

Content selection hydrates only the selected page and bounds public text/tool snippets; references
lead to the existing V2 content pager. Provider state/private reasoning is not conversation content.
Recorded stages explain routing but never create user-turn adjacency. Verified native compression is
a continuation boundary with reset turn context. Requests lacking reliable lineage/turn evidence
remain separate or unknown rather than acquiring inferred links.
