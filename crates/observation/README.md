# Local observation

`hiroute-observation` stores and queries execution facts and managed conversation content. It does not make routing decisions or use ordinary logs as a source of execution truth.

## Responsibilities

- `writer`, `receipt`, and `store`: bounded fact ingestion and durable local storage.
- `query` and `query_v2`: request, session, task, and turn views with explicit unknown or incomplete associations.
- `content`, `managed_text`, and `text_index`: content references, bounded reads, search, and deletion boundaries.
- `valuation` and `value`: usage and monetary facts with explicit missing-price and incomplete-data states.
- `maintenance`: storage maintenance and retention work.

Observation gaps must remain visible without changing the model request's execution result. Historical identity and price facts must not be inferred from current configuration. Deleted content remains unavailable through search, old references, and replay.

`StageAssessmentRecordedV1` retains joint quality under one frozen native rubric
without assigning a model or profile. It uses the existing nullable stage projection
and immutable history membership. Complete joint assessments count as scored stages;
single-model capability averages exclude them. The earlier model assessment format
and authenticated historical fact readers retain their meaning.

Catalog queries may opt in to `include_input_selection`. The request-local reader
authenticates at most 128 metadata facts and a 64 KiB original carrier; missing,
ambiguous, corrupt or deleted evidence yields no position. Existing calls omit this
field and retain their cursor binding. No startup backfill or session-history scan
is needed. Position evidence does not prove human authorship or complete input-batch
coverage, and never changes stored conversation content.

Managed content implementation details are described in
[managed_text/README.md](src/managed_text/README.md). The executable storage, query,
deletion and retention expectations are covered by this crate's tests and the product E2E
scenarios under `e2e/product`.

Focused package checks use `cargo test -p hiroute-observation`.

## Compact storage and capacity retention

The current physical layout is format 5. Only the three repeated protocol
carriers share a stream descriptor; content metadata and frozen feedback use
independent checksummed level-1 zstd records with an identity fallback. Content
objects keep their existing digest and file representation. No segment-file
format or small-object database is introduced.

`store/upgrade` admits only fresh/current writers and classifies supported old
formats for sealing. Production startup preserves the complete original group,
keys and matching old package, seals the old Observation directory by a
same-filesystem rename, then initializes a fresh format-5 generation privately
before publishing it. It does not scan, copy, hash or convert old history.
Original DB/WAL/SHM, object and staging files remain together and unchanged;
current history/statistics begin at cutover. Current-format data and policy are
never reset on ordinary restart. Interrupted namespace publication uses the same
original recovery batch; unknown partial conversion generations remain preserved
with actionable whole-group recovery. See
[release recovery](../../docs/release-contracts.md#upgrade-and-manual-recovery).

`store/capacity` uses the existing maintenance owner. Its optional positive byte
limit is installation-wide and defaults to 20 GB (20,000,000,000 bytes).
An explicitly saved disable is retained across restart; seven-day age retention
continues independently. Accounting counts allocated active
Observation DB/WAL/SHM and content files, excluding protected source generations
and recovery packages. Initial file inventory is incremental and then maintained
by owned file mutations. Unix accounting reads path metadata without opening live
SQLite files, preserving their process locks. Checks are at least 10 seconds apart from completion,
with a cooperative 100 ms SQL/filesystem budget and backoff after failed or
ineffective work. SQLite checkpoint/fsync can overrun that cooperative budget;
the next wait is also at least 100 times the actual elapsed pass cost, so a slow
unit cannot turn into continuous CPU work. Oldest eligible completed requests
retire toward 90% of the limit. Durable request/blob progress, shared-reference
rechecks and incremental page release keep logical expiry separate from actual
physical release. Busy,
unreclaimable or protected data may keep usage above this soft threshold.
Page release consumes each bounded incremental-vacuum result and yields to pinned
readers before selecting more requests. Request cleanup starts from existing
request indices so unrelated workspace history does not consume its SQL budget.
Turn cleanup has a workspace/turn request lookup index, including on current-store
reopen. A large WAL still receives a nonwaiting checkpoint attempt: its physical
high-water mark is not evidence of remaining frames or a reason to block forever.
An initial session-list read retries one visibility change within its original
query deadline. Existing pagination cursors still report stale visibility.

The ignored `store::field_tests` acceptance entry accepts only an explicitly
marked private stopped copy. It exercises current-format open/reopen and real
maintenance cadence; it never authorizes mutation of a live source.
