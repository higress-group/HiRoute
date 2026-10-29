# Usage and value

## Purpose

HiRoute reports known consumption and valuation from real Attempts. Cash cost,
subscription API-equivalent value, cache-token ratio, and evidence completeness
are separate measures.

## Requirements

### Requirement: Usage follows actual accepted evidence, including failed Attempts

Request, model, session, and home queries MUST retain available input/output/cache
usage under trusted traffic classification and current authorized scope. Known
consumption is not erased by unknown price, missing body, cancellation, or a later
Attempt's success. Verified connection-probe traffic is excluded according to its
actual classified identity, including failed probes, rather than query labels.
Repeated events or alternate representations cannot double-count a request.
Missing producer metrics are unknown, not zero. Numeric overflow/unrepresentable
storage or aggregate capacity failure reports an explicit error/incompleteness,
not a truncated plausible value.

### Requirement: Aggregates use a stable scope and currency basis

Home, session, and authorized queries of the same range MUST agree on usage and
amount. Daily results declare timezone and natural-day boundaries. Different
currencies cannot be directly summed. Pending settlement, partially known amount,
and confirmed complete value remain distinct. Ordinary CLI parameter defaults and
continued pagination preserve the same evaluated time scope.

#### Scenario: One relay Attempt consumed tokens before failing

- **WHEN** a later Attempt delivers the answer
- **THEN** both Attempts' known consumption MUST be included once with their own
  model/source identity, without claiming the failed Attempt was free.

### Requirement: Price evidence is exact and frozen per request

Manual source-specific prices take precedence over defaults and survive data
updates until explicit restore-default. Equal model names across provider/endpoint/
product do not share billing conclusions. Each new request freezes its effective
basis under [price policy](../model-catalog-price/spec.md); historical value cannot
be recomputed using today's prices.

Subscription API-equivalent value is available only with sufficient usage/reference
price and MUST be labeled an estimate with inspectable source/basis. It is not a
subscription bill, cash paid, refundable amount, or demonstrated savings. Missing
price is unknown, not free.

### Requirement: Cache ratio is token-weighted over valid known samples

Input cache-hit ratio MUST be `sum(cache_read) / sum(total_input)` over the same
eligible known sample set. Producer protocol semantics are normalized before
aggregation; cache writes are not hits. It is not request hit rate, ContextHold rate,
or cost savings. Missing/inconsistent/excluded metrics, true zero, zero denominator,
and partial coverage remain distinguishable. Valid token totals are not averaged
per request or assumed from absent fields. UI and queries use the same numerator,
denominator, exclusions, and completeness.

#### Scenario: Two requests have different input sizes

- **WHEN** one small request is cached and a larger request is uncached
- **THEN** the displayed ratio MUST use total cached/input tokens, not an average
  of their per-request percentages.

### Requirement: Long-lived aggregates cannot restore expired detail

Permitted aggregate usage/value may outlive seven-day request/session detail without
body, competence samples, or reverse access to deleted sessions/runs. Whole-instance
aggregate retention cannot manufacture precise expired run/session slices. Unknown
unsettled/unrecorded history remains unknown. A query crossing the detail boundary
must expose its available scope, and new-session default views use the bounded
seven-day window without extending detail retention.

### Requirement: Rank session spending and drill down through the same cost basis

HiRoute SHALL provide bounded backend-ranked session and request spending queries for a selected interval, scope, currency and disclosed recorded or API-equivalent valuation basis, with stable ordering and continuation. Ranking and drill-down SHALL aggregate actual recorded attempt usage and matching amounts before page limits and SHALL reuse existing valuation/price evidence without reading every conversation body. Results SHALL expose known amounts, usage/model breakdown, request/attempt counts, covered/missing/pending/excluded populations and exact evidence references. Unknown amounts SHALL NOT become zero or participate as fully known totals; incomplete results SHALL be labeled highest known costs rather than a definitive full-population ranking. Session/request details and covered aggregates SHALL reconcile under one reusable analysis scope with a disclosed relation watermark, fixed interval, monetary evidence revisions and price basis. Retried/failed attempts with known consumption count once, and retained aggregate data cannot recreate expired session/request detail. A recorded basis SHALL rank one currency and one recorded valuation kind; when the admitted scope contains several kinds or currencies without a selection, the query SHALL return bounded choices instead of summing different measures. An API-equivalent ranking SHALL be labeled a current-price estimate, distinct from amounts frozen at request time. Calendar-day scopes such as yesterday SHALL use one disclosed local timezone and absolute half-open bounds shared by ranking and drill-down. Ranking SHALL cover the complete retained scope at observed workload scale within the existing query limits or return a classified retryable failure, never a fixed-row-count outage or a partial evaluation labeled complete. In the opt-in session ranking, proven compression-continuation members form one conversation row under session-history's "Verified native compaction links preserve session identity". Rows retain member identities, disclose partial linkage and reconcile with member/request drill-down; an unproven boundary is not silently grouped. Ordinary legacy outputs remain unchanged. Pagination and related drill-down SHALL accept that scope. Later relation/evidence additions SHALL NOT silently regroup or revalue it, or invalidate it merely by arriving. Genuinely withdrawn or deleted used proof/evidence SHALL produce explicit stale/refresh, while current visibility is always enforced. Routine retention is accounted for in the upfront effective interval instead of silently changing totals or repeatedly invalidating valid scopes. A new analysis scope SHALL disclose requested and retention-safe effective bounds and its lifetime before pagination. Within that lifetime, routine retention cleanup SHALL NOT change its selected population, grouping, coverage or monetary totals. Any clipped interval SHALL be explicit; complete results apply only to the effective interval, never an omitted part of the requested period. This SHALL NOT extend original evidence retention. Explicit deletion, proof withdrawal and authorization changes still apply, while token expiry has a distinct refresh action. Derived values SHALL be checked against current retained source facts and the actual deletion class, including after compatible released-writer mutations and reopen; fully deleted or archived requests cannot be charged through stale auxiliary rows, while content-only deletion preserves retained monetary facts. A normal reopen with already aligned retained projections SHALL reuse them without introducing reconciliation pending. Only views requiring known missing or stale retained projections may report catch-up pending, with scoped progress and a supported retry action; ready reads and valid scopes SHALL remain usable under current source/visibility checks. Interrupted catch-up SHALL resume without restarting for unrelated aware traffic, and stalled work SHALL report its reason rather than remain indefinitely plain pending. Missing historical evidence is an explicit gap, not unfinished reconciliation.

#### Scenario: Find expensive sessions and their requests

- **WHEN** the user asks for the most expensive sessions in a selected day and opens the highest known-cost one
- **THEN** the backend returns correctly ordered bounded sessions, then its expensive requests with the same range/currency/basis and reconciling covered amounts plus actual model/usage/evidence references.

#### Scenario: Rank before taking a page

- **WHEN** an expensive session lies outside the first chronological page and ties or concurrent activity exist
- **THEN** the cost query ranks the full admitted scope before LIMIT, orders ties deterministically and binds continuation to its evaluated scope/price basis instead of sorting only a client-downloaded sample.

#### Scenario: Handle missing price and retry usage

- **WHEN** one session has missing monetary facts and another contains a consuming failed attempt followed by success
- **THEN** the incomplete session is reported in coverage rather than counted as free, all known attempt consumption is included once and the displayed ranking clearly states its known-cost coverage.

#### Scenario: Keep costs readable without body access

- **WHEN** message content or text indexes are missing or pending while monetary/usage facts remain available
- **THEN** cost summary/ranking and same-basis request drill-down remain available, with unavailable body and unknown monetary components independently described.

#### Scenario: Preserve price, currency and retention meaning

- **WHEN** a price generation/currency changes or requested detail predates retention
- **THEN** the caller starts an appropriately scoped comparison or receives explicit missing/retention coverage; the query does not mix currencies, silently revalue historical recorded costs or invent expired detail.

#### Scenario: Choose between different measures

- **WHEN** the selected scope contains requests whose recorded values are of different valuation kinds or currencies
- **THEN** the ranking asks for one currency/kind or the API-equivalent basis through bounded choices and never adds different measures into one cost.

#### Scenario: Rank yesterday in the user's local day

- **WHEN** the user asks for yesterday's most expensive sessions
- **THEN** the query resolves one local calendar day, discloses its timezone and absolute bounds, and the request drill-down uses exactly the same bounds.

#### Scenario: Rank a high-volume week

- **WHEN** the selected scope contains far more requests and attempts than one result page
- **THEN** the backend ranks the complete retained scope within the existing query deadline, or returns a classified retryable failure with a supported narrowing action; scope size alone never yields a generic outage.

#### Scenario: Rank a proven compression conversation once

- **WHEN** one retained conversation spans several physical sessions with verified compression links
- **THEN** the opt-in session ranking groups its in-scope requests once and reconciles chain, member and request totals under the same basis; unproven sessions remain separate with explicit coverage.

#### Scenario: Page and drill down while the querying Agent compacts

- **WHEN** the analyzing Agent is itself in scope and records another compaction, request or settlement after the first ranking page
- **THEN** continuation and conversation/member/request drill-down reuse the returned relation watermark and monetary basis with identical covered totals; new rows/links wait for a fresh analysis and do not by themselves invalidate the existing cursor.

#### Scenario: Page a default ranking during retention

- **WHEN** a default retained ranking is created near the oldest retained millisecond and maintenance runs before its next page and request drill-down
- **THEN** requested/effective bounds and clipping were disclosed and covered totals/order remain identical throughout its valid lifetime; unrelated expiry does not globally invalidate it.

#### Scenario: Re-upgrade after a compatible writer changes history

- **WHEN** a compatible released writer deletes a session, expires requests and records new retained traffic before current HiRoute reopens
- **THEN** fully deleted/archived requests and stale reverse links are absent, new live traffic is derived once, content-only deletions preserve applicable costs and no detail is reconstructed from archives.

#### Scenario: Reuse aligned history after an ordinary restart

- **WHEN** HiRoute reopens already aligned retained data without an intervening unaware writer or interrupted catch-up
- **THEN** new analysis reads reuse the saved projections without a new reconciliation-pending period; actual source gaps, unsettled usage and turn incompleteness keep their distinct meanings.

#### Scenario: Expose genuine catch-up without blocking ready reads

- **WHEN** retained data needed by one view is missing from derived projections after compatible writer mutations
- **THEN** only the affected view reports scoped progress and retry guidance; ready reads continue, interrupted work resumes, and stalled or unavailable work has an explicit reason instead of endless plain pending.

#### Scenario: Do not rank through a fully deleted middle member

- **WHEN** a ranked continuation A to B to C loses B through full deletion
- **THEN** a fresh ranking separates surviving components with a linkage gap and excludes B's deleted details/amounts; a prior scope using those edges is stale, never silently regrouped. Content-only deletion keeps retained costs and grouping.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6229105150

### Requirement: Cost comparison presents exact amounts or complete bounded ranges

A same-usage cost comparison SHALL show each available metric as exact, otherwise a valid full-scope bound, otherwise unknown. A known routed estimate remains visible when the reference is unknown. Bounds must cover the same population and maintain routed-minus-reference consistency; partial paired subtotals cannot stand in for totals. Retention/overflow or inconsistent counts remain explicit, not false complete values. Main card, evidence tiles and timeline SHALL preserve nonzero small amounts, currency, signs and unknown contributions using consistent precision; bound rounding cannot narrow the interval. True zero, missing value and unavailable comparison are distinct. Essential estimate/same-usage limitations remain visible, while detailed calculations are accessible. No display action changes prices, usage, retained records or calls a model. Range endpoints retain separate signs and use an unambiguous localized separator, including Chinese 至, so two negative endpoints are not read as subtraction.

#### Scenario: A full bound exists without exact reference

- **WHEN** routed=2764, reference bounds 92400..115200 and difference bounds -112436..-89636 micros cover the full scope despite compared=0
- **THEN** the main result and tile show routed $0.0028, reference $0.0924 to $0.1152 and outward difference −$0.1125 to −$0.0896 (Chinese uses 至), not a contradictory Unknown; signs and bounds remain unchanged.

#### Scenario: Exact, tiny and zero amounts

- **WHEN** exact and bound data coexist, or a value is zero/one micro/a small nonzero amount
- **THEN** exact has priority without duplicate ranges; true zero is zero and nonzero never rounds to displayed zero.

#### Scenario: Incomplete or inconsistent evidence

- **WHEN** bounds are missing/inverted/unsafe/inconsistent, counts conflict, or retention/overflow exists
- **THEN** the affected totals remain unknown with their real reason; known routed is preserved except where total scope itself is invalid, and subsets are labelled subsets.

#### Scenario: Unavailable reference conditions

- **WHEN** Primary, currency or price basis is not uniquely available
- **THEN** the specific unavailable condition remains visible; no inferred currency, zero usage or new comparison population is invented.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/229#issuecomment-6440856870

### Requirement: Evidence counts disclose their actual routing cohorts

The evidence view SHALL label the selected display scope separately from suggestion evidence. Header stage/scored/unrated counts follow the display filters. Step-up evidence remains the active-revision, per-category Regular complete-scoring cohort within retention and its applicable model-update cutoff, independent of the display time window; labels remain visible even when the suggestion is keep. When complete authoritative evidence proves no announcement applies, use retention-only wording, not a fabricated update date. If the published per-category identities or applicability cannot be established, the model-update basis SHALL remain unconfirmed; incomplete or refreshed editor options cannot prove applicability or absence. The reported effective cohort start remains visible. Required thresholds and cohorts remain unchanged; noncurrent/all-version actions retain existing restrictions. Counts, original model identity, unscored last stages and protection/release distinctions are not changed to appear consistent. Keep a compact visible step-up sample count and explicit active-revision/category/Regular scope; place retention and model-update cutoff details in an accessible disclosure. Do not refer to the header as the scope above or hide the difference between display counts and suggestion counts.

#### Scenario: Different legitimate counts

- **WHEN** the header has four scored stages but the active Regular step-up cohort has two
- **THEN** both values remain unchanged and their different scopes are explicitly stated.

#### Scenario: Period and announcement scope

- **WHEN** the user changes seven days to all, another branch has an announcement, or retention is later than the applicable announcement
- **THEN** suggestion population keeps its actual scope and labels do not attribute another branch or retention date to a model update.

#### Scenario: Incomplete or refreshed editor options

- **WHEN** a published Regular binding is absent from current editor options or its current model identity differs from the published identity while the report contains model-update announcements
- **THEN** the view describes the model-update basis as unconfirmed and displays the reported effective cohort start without asserting applicability or absence from those options.

#### Scenario: Proven absence of model updates

- **WHEN** the complete report contains no model-update announcements
- **THEN** the view identifies retention as the bound on the step-up scope without inventing an announcement date.

#### Scenario: No later assessment or historical view

- **WHEN** the final stage is unrated or a historical/all-revision view is open
- **THEN** unrated evidence and active-revision-only action restrictions remain visible; no score or scope is fabricated.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/229#issuecomment-6440858265

## Implementation references

- [Value projection](../../../crates/observation/src/value/mod.rs),
  [usage archive](../../../crates/observation/src/valuation/usage_archive.rs), and
  [value contract scenarios](../../../crates/observation/src/store/tests/value_contract.rs).
- [Source price types](../../../crates/domain/src/compute/source_price/mod.rs).
