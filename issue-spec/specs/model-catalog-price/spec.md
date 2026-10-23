# Model facts, reference scores, and price policy

## Purpose

HiRoute combines version-bound offline facts, exact source capabilities, reference
scores, and source-specific price policy without treating one as proof of another.

## Requirements

### Requirement: Offline facts are version-bound and identity remains explicit

The installed ReleaseFacts bundle MUST work without a Hub, login, or network.
Manifest identity, resource digests, and cross-references bind it to the client
version and fail closed on mismatch; the current bundle does not require an
independent signing-key release system. Sources may share a canonical model and
reference score while retaining separate provider, product, region, protocol,
capability, price, and entitlement facts.

Metadata usability MUST depend on required field completeness, not a separate
confidence threshold inferred from source category. The maintained dataset closes
provider-scoped descriptive fields deterministically to observed/retrieved fact,
explicitly marked conservative inference with per-field evidence/rationale, or
unsupported/not-applicable. Marked inference is never provider-confirmed observation.
Unresolved canonical identity, alias ownership, endpoint binding, or execution
qualification stays unresolved even when descriptive fields are complete.

A qualified endpoint's actual directory observation may grant an unknown raw
upstream ID bounded text-only execution. It MUST NOT invent a canonical model,
price, free entitlement, or cross-provider equivalence. Unsupported, not-applicable,
inferred, and unresolved states remain distinct; not-applicable is not false/unknown.
A model's maximum context and an Agent's default/adapted window are different facts.

#### Scenario: A directory contains a new unknown model

- **WHEN** a qualified endpoint returns a raw ID absent from canonical metadata
- **THEN** its bounded execution may be available, but identity, price, and optional
  capabilities MUST remain unresolved rather than copied from a similar name.

### Requirement: Scores bind exact native reasoning and one immutable snapshot

Reference scores MUST identify canonical model, actual native reasoning setting,
score version, and scale. The displayed aggregate uses five stars to one decimal
and distinguishes matched, estimated, and unknown results; coding/tool scores are
supplemental. A highest setting, provider default, or same-named setting on another
model cannot replace the selected setting. Missing scores never prevent legal use.

A batch resolves one immutable snapshot. An unavailable explicitly requested old
version fails as unavailable instead of mixing versions or falling back to latest.
Maintenance evidence includes original evaluation/discussion source, date, model,
known native configuration, and a brief judgment. Estimates are not measurements;
unknown settings are not measured settings, and reposts/likes are not product votes.
Updating reference data cannot reorder a published plan.

#### Scenario: Catalog refresh occurs during a score query

- **WHEN** a batch spans a newer score release or asks for a missing historical one
- **THEN** all results MUST use its single requested snapshot or explicitly fail;
  existing published candidate order stays unchanged.

### Requirement: Price overrides belong to the supplying identity

Users MUST be able to set input/output price per million tokens and currency for
an exact source, including custom models without default prices. Manual values
win over defaults and survive updates. Restoring default removes that override and
uses the current default or unknown. Price editing cannot change subscription or
free entitlement.

Overrides bind stable supplying identity, model, currency, and valuation basis.
Binding IDs are locators/concurrency facts, not a reason to transfer prices between
suppliers. Rename retains the override; actual supplier identity change does not.
Ordinary input and cached-input prices are separate. Missing cache price MUST NOT
be replaced by zero or the full ordinary rate. Edit context includes observed
source/Binding/override revisions; stale submissions fail. Subscription/CPA targets
have no ordinary editable-price context.

#### Scenario: Rename versus replace the supplier

- **WHEN** a connection is renamed or its actual supplying identity changes
- **THEN** rename MUST retain its override, while a different supplier cannot
  inherit the old price simply because the display name or model matches.

### Requirement: Every request freezes its own effective valuation basis

Every new logical request, including a continued Worker task, MUST acquire the
price generation effective at request start. All Attempts stay in that generation
but resolve time-dependent rules at their own applicable execution time. Frozen
basis includes exact supplier, price/override versions, currency, billing fields,
units, and time basis. Historical settlement cannot be re-priced by later data.
Plan/publication identity, run identity, and price generation remain independent.

Gateway uses nonblocking read-only price snapshots; Observation settles
asynchronously. SQLite or valuation failure cannot block the model response.
Known failure-attempt usage still counts. Missing usage or pricing remains unknown,
not zero. Subscription API-equivalent value is labeled an estimate, never payment.
The captured generation MUST include the verified mapping from a source's stable
management model identity through the verified source/model binding. A pinned
publication's execution ID may predate credential or availability revisions;
those revisions MUST NOT detach an unchanged source/model from its current price. Equal names are not
mapping evidence; conflicting mappings leave execution pricing unknown.
When an ordinary billing quote lacks known input/output rates, an explicitly
configured source-specific API-equivalent quote with those rates MAY supply the
recorded estimate instead. Its valuation kind MUST remain API-equivalent. A usable
verified billing quote takes precedence. Missing cache rates or usage still produce
an explicit partial/unknown amount; reference pricing cannot fabricate usage.
A frozen publication cannot revive invalid run tokens, expired deadlines, cancelled
runs, or revoked source/Key access; current Plan delegation policy governs new
Start/Continue without automatically cancelling accepted work.

#### Scenario: Relay crosses a time-price boundary

- **WHEN** two Attempts in one logical request execute on opposite sides of a
  pricing time boundary
- **THEN** they MUST share the admitted price generation but freeze their own
  applicable time rules; neither uses a newly published generation.

#### Scenario: A custom model has distinct management and execution identities

- **WHEN** a user configures prices for a custom model through its stable source binding
- **THEN** new requests use the exact generation-bound execution mapping, while
  different suppliers, changed source/model bindings, currencies and valuation kinds cannot
  inherit the quote. The frozen evidence retains the executed model identity.

#### Scenario: A subscription has a reference price but no billing quote

- **WHEN** an exact source has configured API-equivalent input/output rates and
  ordinary billing rates are unavailable
- **THEN** new recorded amounts use the frozen API-equivalent estimate with that
  label and actual coverage; edits affect future requests without repricing old records.

### Requirement: Management reports safe current source and price facts

The model-management snapshot MUST return trusted connection identity, billing
class, availability evaluated within the same query, closed masked reasons, and
available price context for each model. Missing facts are unknown/empty. Connector
ownership alone is not subscription classification; one cooling Key does not make
a Binding unavailable while another is usable.

Saved subscription mode is distinct from runtime execution readiness and survives
missing credentials or unpublished state. Unavailable subscriptions retain their
actual saved mode and repair path. Reads do not change source state, route identity,
price revisions, or publication, and restart preserves the committed choice.

#### Scenario: Price or health facts cannot be read

- **WHEN** one source's runtime or effective price lookup fails
- **THEN** its projection MUST show the specific unknown/unavailable state without
  hiding healthy sources or inventing an editable subscription price.

### Requirement: Compare routed usage using an explicit API-equivalent price basis

HiRoute SHALL support an explicitly selected API-equivalent cost view for recorded routed usage, including subscription/Coding Plan traffic, against an explicitly identified reference model and currency. It SHALL reuse source-scoped current API-reference prices or user-provided revisioned prices, expose the chosen price/time basis, routed estimate, reference estimate and signed estimated savings, and include observed retry usage. This current-price comparison SHALL NOT rewrite immutable historical receipts or be represented as a subscription bill or realized cash savings. Missing or incompatible usage/price facts SHALL remain unknown or partial, with counted coverage and no fabricated savings. Correct supply classification SHALL distinguish free, subscription, paid and unknown independently from the selected estimate basis. The common management Skill SHALL guide price completion, comparison and evidence-based optimization through installed public CLI capabilities.
 A recent-summary path SHALL default to a disclosed rolling seven-day scope and resolve the selected Plan's sole primary candidate as the exact reference when no explicit override is supplied; nonunique or unavailable references/currencies SHALL require a bounded explicit choice. Saved applicable rates SHALL be reused. The summary SHALL expose a signed saving and percentage for a complete paired set, or clearly labeled compared-subset amounts/difference/percentage and missing/pending/excluded counts when incomplete. A zero reference denominator leaves percentage unavailable. It SHALL disclose unmeasured decision-service or other overhead and SHALL NOT label an incomplete-overhead comparison as net total savings. Existing retention boundaries apply and expired detail cannot be reconstructed for a new reference calculation. The comparison SHALL evaluate the complete retained in-scope population at observed workload scale within the existing query limits, or return a classified retryable failure; it SHALL NOT fail solely because the scope exceeds a fixed row count, and SHALL NOT present a partially evaluated population as complete. Current-price estimates SHALL be labeled distinctly from API-equivalent amounts frozen at request time. When scoped to a verified continuation conversation, the summary SHALL use the same session-history membership as conversation and ranking views, counting each in-scope request/attempt once; unproven or expired members remain coverage gaps. Related summary, ranking and monetary drill-down SHALL expose a reusable analysis scope fixing the relation watermark, resolved bounds, usage/evaluation revisions and price basis, so live usage settlement or price changes cannot silently alter the compared population or arithmetic. A fresh analysis can include later evidence; deleted or withdrawn used evidence requires an explicit refresh, while routine retention is accounted for in the upfront scope. A new analysis scope SHALL disclose requested and retention-safe effective bounds and its lifetime before pagination. Within that lifetime, routine retention cleanup SHALL NOT change its selected population, grouping, coverage or monetary totals. Any clipped interval SHALL be explicit; complete results apply only to the effective interval, never an omitted part of the requested period. This SHALL NOT extend original evidence retention. Explicit deletion, proof withdrawal and authorization changes still apply, while token expiry has a distinct refresh action. A normal reopen with already aligned retained projections SHALL reuse them without introducing reconciliation pending. Only views requiring known missing or stale retained projections may report catch-up pending, with scoped progress and a supported retry action; ready reads and valid scopes SHALL remain usable under current source/visibility checks. Interrupted catch-up SHALL resume without restarting for unrelated aware traffic, and stalled work SHALL report its reason rather than remain indefinitely plain pending. Missing historical evidence is an explicit gap, not unfinished reconciliation.

#### Scenario: Estimate a Coding Plan combination

- **WHEN** a user chooses API-equivalent value with prices for economy and primary/reference models
- **THEN** the view compares recorded routed usage with the selected reference under one disclosed currency and price basis, while actual subscription payment remains independent and unknown unless separately evidenced.

#### Scenario: Supply reference prices

- **WHEN** a source/model lacks collected API prices
- **THEN** the caller can use the existing revisioned source-specific manual price path and explicitly rerun the estimate; this does not change entitlement or historic settlement.

#### Scenario: Keep missing evidence honest

- **WHEN** required usage, price, identity, retention or terminal facts are missing
- **THEN** the report counts coverage and leaves incomplete comparisons unknown rather than treating them as free or claiming a full saving.

#### Scenario: Include retries and regressions

- **WHEN** a routed request uses multiple attempts or costs more than the reference
- **THEN** all observed attempt usage contributes and a valid negative difference remains negative.

#### Scenario: Keep historical meaning

- **WHEN** the user later edits API-equivalent prices
- **THEN** a new selected current-price estimate may change, but stored receipts and ordinary historical value queries retain their original meanings.

#### Scenario: Ask for recent savings without internal identifiers

- **WHEN** the user selects a smart-saving Plan with one primary candidate and asks for recent savings
- **THEN** the installed summary resolves and names that exact reference, fixes the default seven-day range and currency/price basis, and reports routed/reference amounts and signed saving/percentage without manual Binding lookup.

#### Scenario: Report the known paired subset

- **WHEN** some requests are priceable on both sides while others lack usage, price, terminal or retention evidence
- **THEN** full-period totals remain incomplete, but the same paired subset has inspectable routed/reference subtotals and signed difference with compared/missing counts; those values are not labeled the whole period.

#### Scenario: Keep percentage and overhead honest

- **WHEN** the reference cost is zero, routed cost is larger, or decision-service charges are unmeasured
- **THEN** percentage is unavailable for zero denominator, a valid loss remains negative and missing overhead is disclosed rather than silently included in a net-savings claim.

#### Scenario: Reuse rates without rewriting history

- **WHEN** the user completes missing API-equivalent rates through the existing source-specific override flow and asks again
- **THEN** the next current-price summary uses those exact saved rates and shows its evaluation basis; ordinary frozen historical amounts and actual subscription payments retain their meaning.

#### Scenario: Summarize a high-volume week

- **WHEN** the selected seven days contain far more retained requests and attempts than one result page
- **THEN** the summary evaluates the whole retained scope under the existing query deadline and reports its coverage, or returns a classified retryable failure with a supported narrowing action; scope size alone never produces a generic outage or a partial total labeled complete.

#### Scenario: Distinguish current-price and recorded estimates

- **WHEN** the same requests have frozen API-equivalent amounts and the user asks for a current-price comparison
- **THEN** each figure names its basis and price time, and neither is presented as the other.

#### Scenario: Reconcile a summary during live activity

- **WHEN** additional compression links, usage settlement or price edits arrive between summary, ranking and request drill-down
- **THEN** the reused analysis scope preserves its relation watermark and monetary evidence/price basis; later additions are excluded without invalidating it merely for arriving, and retained covered totals reconcile.

#### Scenario: Keep recent savings stable through housekeeping

- **WHEN** a requested seven-day summary and its drill-down are reused while routine retention runs
- **THEN** the result discloses its upfront effective interval and omitted bounds and preserves covered amounts under the same valid scope; it never claims full requested-week savings from a clipped interval.

#### Scenario: Reuse aligned history after an ordinary restart

- **WHEN** HiRoute reopens already aligned retained data without an intervening unaware writer or interrupted catch-up
- **THEN** new analysis reads reuse the saved projections without a new reconciliation-pending period; actual source gaps, unsettled usage and turn incompleteness keep their distinct meanings.

#### Scenario: Expose genuine catch-up without blocking ready reads

- **WHEN** retained data needed by one view is missing from derived projections after compatible writer mutations
- **THEN** only the affected view reports scoped progress and retry guidance; ready reads continue, interrupted work resumes, and stalled or unavailable work has an explicit reason instead of endless plain pending.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6220387895

### Requirement: The website model feed is validated over HTTPS before use

HiRoute SHALL retrieve the model feed only from a fixed HTTPS hiroute.ai location derived by the client build, using normal TLS certificate validation. The HTTPS origin and production website publication pipeline are the content trust boundary; independent feed signatures and signing keys are not required. It SHALL refuse redirects, any location supplied through IPC, CLI arguments, environment or configuration in production builds, and responses larger than 2 MiB, and it SHALL apply a 10-second connection timeout and a 30-second total deadline. Each feed publication SHALL be the direct JSON payload and carry a schema version, a monotonically increasing sequence, an issue time and source revision. The daemon SHALL apply a publication only when all of these hold: the schema version is one the client implements, the whole publication is well formed, and its sequence is greater than the highest sequence this service has accepted. A rejected publication SHALL leave the held validated copy in effect and SHALL record a classified reason: network, size, schema, validation or older sequence. The daemon SHALL persist the last validated copy with its digest, revalidate it at startup and discard a copy that fails validation, with a reason, instead of partially using it. The daemon SHALL retain the highest accepted sequence and the digest of that publication independently of the held copy, so that removing or discarding the copy never lowers it. A publication whose sequence equals the retained highest sequence SHALL be accepted only to restore a missing held copy and only when its digest equals the retained digest; a lower sequence SHALL never be accepted. Retrieval and validation SHALL run in the daemon, so that every product UI host (macOS Desktop, the Windows shell and the web console), the public CLI and standalone services share one validated state, and SHALL NOT delay startup, Gateway admission or routing. The request SHALL carry no account, installation, model, credential, session or usage identifier beyond a product user agent with the client version. Automatic retrieval SHALL be enabled by default for every service, whether it runs under macOS Desktop, the Windows shell or the web console, or as a standalone service. It SHALL run after a randomized 2–10 minute startup delay and then every six hours with 30 minutes of jitter in either direction, using bounded retry backoff after failure. An explicit refresh SHALL be available in every product UI host and in the public CLI. One service-owned setting SHALL turn automatic retrieval off and back on. Every product UI host and the public CLI SHALL read and change that same setting, and it SHALL survive restart and upgrade. The first time feed content is shown in a product UI host, the host SHALL state that the content comes from hiroute.ai and how to turn retrieval off.

The highest accepted sequence and payload digest SHALL be durably checkpointed outside the removable cache directory before a new cache state is committed. A missing cache state SHALL be reconstructed from that checkpoint with no held copy and automatic retrieval off. If only the activated sequence survives without its digest, only a strictly higher sequence may be accepted. A corrupt owned cache state MAY be explicitly quarantined and reset through the confirmed removal flow only when a valid independent checkpoint preserves acceptance; status SHALL disclose the reason and whether this recovery is available. Newer-schema, unsafe or unreadable state, invalid checkpoints and corruption without a trusted checkpoint remain preserved and refuse mutation. After atomic activation failure, the previous consistent catalog remains available and further feed mutations are refused until restart reconciles durable state.

#### Scenario: Invalid publication

- **WHEN** the HTTPS publication fails schema or content validation
- **THEN** the publication is not applied, the held validated copy stays in effect, and feed status reports the classified validation failure.

#### Scenario: Older or repeated publication

- **WHEN** the service holds a verified copy and the served sequence is lower than or equal to the highest accepted sequence
- **THEN** a lower sequence is rejected, equal sequence with the same payload digest is already current, and equal sequence with different content is rejected; restoration of a missing copy follows the retained-digest rule above.

#### Scenario: Offline start

- **WHEN** the service starts without network access while it holds a verified copy
- **THEN** the copy is re-verified and used, startup is not delayed by retrieval, and status shows the time of the last successful retrieval.

#### Scenario: Newer schema

- **WHEN** a publication declares a schema version the client does not implement
- **THEN** the client keeps the embedded catalog and the held copy, and status states that a newer HiRoute is needed to read feed updates.

#### Scenario: Redirect

- **WHEN** the feed location answers with a redirect
- **THEN** the redirect is not followed and no request is sent to the redirect target.

#### Scenario: Default on a standalone service

- **WHEN** a standalone Linux service starts after an upgrade and no product UI is open
- **THEN** it retrieves the feed automatically after the bounded startup delay, and the public CLI status shows the result and the setting that turns retrieval off.

#### Scenario: One setting across hosts

- **WHEN** automatic retrieval is turned off in the web console of a service
- **THEN** the public CLI and any other product UI host of that service report retrieval as off, and no automatic request is made until it is turned back on from any of them.

#### Scenario: No rollback after removal

- **WHEN** the service has accepted publication 43, which withdraws a model that publication 42 listed, and the user removes the held copy, restarts the service and is then served the otherwise valid publication 42
- **THEN** publication 42 is rejected as an older sequence, the withdrawn model stays unavailable, and serving publication 43 with the retained digest, or a newer publication, restores a held copy.

#### Scenario: Conflicting content at the retained sequence

- **WHEN** after the held copy was removed, a valid publication with the retained highest sequence but a different digest is served
- **THEN** it is rejected with a validation reason, no held copy is restored, and the retained sequence and digest are unchanged.

#### Scenario: Interrupted cache replacement

- **WHEN** the service stops while replacing its held publication
- **THEN** restart retains the highest independently committed acceptance checkpoint. If cache state trails that checkpoint, it discards held/ETag references, uses embedded facts and leaves automatic retrieval off. A blob alone never advances acceptance or becomes a held copy.

#### Scenario: Restore after conditional retrieval

- **WHEN** a cached publication previously had an HTTP ETag and its held copy is removed or discarded
- **THEN** the next refresh obtains full verified content before reporting restoration; an HTTP not-modified response alone cannot restore a missing copy.

#### Scenario: Removing the entire cache cannot reset acceptance

- **WHEN** an accepted publication is checkpointed and the cache directory is removed before restart
- **THEN** embedded facts are used with automatic retrieval off, the highest accepted sequence/digest survive, older publications are rejected and only an identical same-sequence restoration or a higher sequence is accepted

#### Scenario: Confirmed corrupt-cache reset

- **WHEN** a user confirms the removal preview for corrupt owned state with a valid independent checkpoint
- **THEN** the damaged file is preserved, cache state is recreated without lowering acceptance, and published Plans and runs remain usable

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/142#issuecomment-6236433837

### Requirement: Feed records add facts only for models absent from the embedded catalog

A validated model feed MAY contain model, model endpoint capability, offer and price-rate records that use the same record contracts as the embedded model data, plus featured entries. It SHALL NOT contain connector, endpoint profile, connection option, free-offer or rating records, and a publication that contains one is invalid as a whole. Each record SHALL declare the minimum client version able to use it and its inclusion date. A model record SHALL carry the same identity-stability fact as embedded model metadata: provider-stable, rolling alias, dynamic route or unknown. Provider-scoped fields SHALL follow the maintained closure rule: an observed or retrieved fact, a marked conservative inference with per-field evidence, or unsupported/not applicable. For any model configuration identity, upstream model binding or price-rate key that the embedded catalog contains, the embedded facts SHALL win. A conflicting feed record SHALL be ignored and counted in feed status. A feed default price SHALL be presented with its origin and inclusion date; a manual price override SHALL continue to win, and restoring the default SHALL use the current embedded or feed default, or unknown. A featured entry SHALL declare whether it announces a new model or an update of a model that HiRoute already describes, SHALL reference a model identity from the embedded catalog or the same publication, and SHALL carry a localized plain-text headline and note, an inclusion date and optional links whose targets are restricted to https://hiroute.ai/. An entry whose reference does not resolve in the client SHALL NOT be shown. Featured entries SHALL NOT affect qualification, candidate order or routing. An update entry SHALL be presented as an announcement together with the referenced model's current fact origin. It SHALL NOT create a second model identity or imply that the feed changed that model's facts, which remain owned by the embedded catalog and later client releases. Feed sequence, source revision and inclusion date identify the facts HiRoute received; they SHALL NOT be presented as identifying the model version that a provider executes. A model without a feed record keeps the existing bounded text-only handling of an unknown upstream ID.

#### Scenario: Forbidden record kind

- **WHEN** a publication contains a free offer or a rating record
- **THEN** the whole publication is rejected as invalid and the held validated copy stays in effect.

#### Scenario: Embedded identity wins

- **WHEN** the feed carries capabilities for a model configuration identity that the embedded catalog also defines
- **THEN** the embedded facts are used, the feed record is ignored, and feed status counts one ignored record.

#### Scenario: Bundled announcements without a held feed

- **WHEN** the feed supplies no featured entries and the client has maintained editorial announcements
- **THEN** Home and quick start show only announcements whose model identities resolve in the service catalog, retain each model's real fact origin and do not label the bundled announcements as retrieved from the website. Directory order alone never makes a model new. An announced provider update does not establish the version served by another connection or reattribute existing observations.

#### Scenario: Price override survives a feed update

- **WHEN** a user overrode the default price of a feed-supplied model and a later publication changes that default
- **THEN** the override remains in effect; restoring the default afterwards uses the new feed default.

#### Scenario: Unresolved featured entry

- **WHEN** a featured entry references a model that neither the embedded catalog nor the publication defines for this client
- **THEN** the entry is not shown, and no placeholder model is created.

#### Scenario: Announced update of an embedded model

- **WHEN** a publication features an update of an embedded model whose upstream ID is unchanged
- **THEN** the entry is shown as an announcement with the model's embedded origin, no second model identity is created, the embedded facts are used unchanged, and earlier competence evidence is not presented as validation of the announced version.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/142#issuecomment-6236435011

### Requirement: Feed publications are reviewed, validated and published by the website deployment

Model feed sources SHALL be maintained in this repository and changed only through reviewed pull requests. Validation SHALL run in CI for every change and again before publication. It SHALL reject schema violations, forbidden record kinds, unclosed provider-scoped fields or missing evidence, references that resolve in no supported release's embedded registry, a declared minimum client version lower than the earliest supported release whose registry and adapters resolve the record, featured links outside https://hiroute.ai/, and a sequence that is not greater than the last published sequence. Only the production website deployment SHALL publish the feed. It SHALL stage the exact validated JSON payload without a signed envelope or signing secret. If retrieval or validation of the proposed or currently served publication fails, the previously published feed SHALL remain in place and no replacement SHALL be uploaded. Each publication SHALL be reproducible from its reviewed source revision, and the deployment SHALL record the published sequence, digest and source revision. A genuine HTTP 404 permits first publication; redirects and other unsuccessful responses SHALL stop publication.

Before a feed-capable supported client release is recorded, the generated payload SHALL remain null and deployment SHALL skip publication. Bootstrap checks still validate maintained publication and editorial inputs. Generation SHALL use committed first-inclusion dates, the exact supported release catalogs, and the earliest release admitting each model together with its executable capability and offer. PR validation SHALL compare the publication to its exact base, and deployment SHALL compare it to the validated served publication; identical content may be a no-op, while changed content requires a higher sequence.

#### Scenario: Unresolvable reference

- **WHEN** a pull request adds a record bound to an endpoint profile that no supported release's registry contains
- **THEN** CI fails and names the record and the unresolved reference.

#### Scenario: Understated minimum version

- **WHEN** a record requires an adapter revision first shipped in 0.3.3 but declares 0.3.1 as its minimum client version
- **THEN** validation fails and states the earliest compatible version.

#### Scenario: Invalid served publication

- **WHEN** production deployment cannot retrieve and validate the currently served publication
- **THEN** the previously published feed remains served, and no new feed bytes are uploaded.

#### Scenario: Pull-request build

- **WHEN** a pull-request build validates feed changes
- **THEN** validation results are reported, and the build validates the payload without deployment credentials and publishes nothing.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/142#issuecomment-6236438296

## Implementation references

- [Metadata records](../../../crates/domain/src/compute/model_metadata_records.rs) and
  [source price contract](../../../crates/domain/src/compute/source_price/mod.rs).
- [Price persistence scenarios](../../../crates/local-storage/src/control/prices/tests.rs).
- [Current subscription/model query](../../../crates/application/src/compute_management/query_v3.rs).
