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

Users MUST be able to set input/output price per million tokens and currency for an exact source, including custom models without default prices. Manual values SHALL win over official defaults and survive catalog updates, restart and upgrade. Restoring default SHALL remove the override and use the selected official snapshot's applicable price or unknown. Price editing SHALL NOT change subscription or free entitlement.

Official price updates SHALL apply to already embedded and already saved models as well as new models. Resolution SHALL match the actual supplying identity, product, region, model/upstream binding, currency, billing/valuation basis and applicable schedule conditions. Overrides SHALL remain bound to stable supplying identity, model, currency and valuation basis; Binding IDs are locators/concurrency facts. Rename SHALL retain an override, while a different supplier SHALL not inherit it from a reused name or locator.

Ordinary input, output, cache-read and cache-write prices SHALL remain distinct. Time windows, prompt thresholds, service tiers, unit and schedule application SHALL be preserved. A missing component, ambiguous scope or unsupported calculation rule SHALL remain unknown; it SHALL NOT be replaced by zero, the ordinary input rate, another provider's price or an invented blended rate. Explicit valid zero is a value. A display-only cost hint SHALL not become a default valuation rule. Subscription/CPA targets retain no ordinary editable-price context; an explicitly selected source-scoped API-equivalent estimate uses its existing separate price path and is never labeled subscription payment.

Price edit/default-reset context SHALL capture source, Binding, override and selected-catalog revisions. A stale submission SHALL fail without overwriting current values. A default-price generation SHALL be derived from one validated catalog and the corresponding user configuration; concurrent refresh SHALL not install an older or mixed generation.

Every new logical request, including a Worker continuation and a request through an older published Plan, SHALL capture the current effective price generation. Attempts SHALL retain that generation while resolving its applicable time conditions at their own execution time. Subsequent metadata changes SHALL not reprice settled receipts or ordinary historical amounts. A newly requested current-price comparison may use the newer price basis only with that basis disclosed; an existing retained comparison scope keeps its captured basis. Missing price coverage remains unknown and valuation failure SHALL not block the model response.

#### Scenario: Update a saved model's default prices

- **WHEN** an accepted official update changes ordinary input and cache-read rates for the exact product used by a saved model
- **THEN** a later request uses the new applicable default components and recorded provenance without requiring a client release or recreating the connection.

#### Scenario: Manual prices survive and reset to current defaults

- **WHEN** a source has a manual price, its official default changes, and the service refreshes and restarts
- **THEN** the manual price remains effective; an explicit restore-default operation selects the new scoped default or unknown.

#### Scenario: Rename versus replace the supplier

- **WHEN** a connection is renamed or its actual supplying identity changes
- **THEN** rename retains the override, while a different supplier cannot inherit the old price merely because the display name or model matches.

#### Scenario: Keep unsupported price components unknown

- **WHEN** the current catalog removes a cache-write price or provides a tier rule the installed price engine cannot evaluate
- **THEN** the affected component remains unknown with its reason; older embedded values and unrelated product rates do not fill it.

#### Scenario: Refresh during a request

- **WHEN** one request starts before a catalog price update and another starts after the new price generation is installed
- **THEN** all attempts of the first retain its captured generation, the later request uses the new generation, and settled historical amounts remain unchanged.

#### Scenario: Reject a stale restore-default confirmation

- **WHEN** the catalog or supplying identity changes after a user previews restoring default prices
- **THEN** apply reports a revision conflict without silently resetting to a different basis.

#### Scenario: Keep comparison and payment meanings

- **WHEN** a subscription user repeats an API-equivalent comparison after official reference prices change
- **THEN** the new explicitly requested estimate discloses its new price basis; retained scopes, frozen receipts and actual subscription payment retain their prior meanings.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295419945

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

HiRoute SHALL retrieve the model feed only from a fixed HTTPS hiroute.ai location derived by the client build, using normal TLS certificate validation. The HTTPS origin and production website publication pipeline are the content trust boundary; independent feed signatures and signing keys are not required. It SHALL refuse redirects, any location supplied through IPC, CLI arguments, environment or configuration in production builds, and responses exceeding the explicit finite decoded-byte and structural limits of the supported complete-snapshot contract, and it SHALL apply a 10-second connection timeout and a 30-second total deadline. Each feed publication SHALL be the direct JSON payload and carry a schema version, a monotonically increasing sequence, an issue time and source revision. The daemon SHALL apply a publication only when all of these hold: the schema version is one the client implements, the whole publication is well formed, and its sequence is greater than the highest sequence this service has accepted. A rejected publication SHALL leave the held validated copy in effect and SHALL record a classified reason: network, size, schema, validation or older sequence. The daemon SHALL persist the last validated copy with its digest, revalidate it at startup and discard a copy that fails validation, with a reason, instead of partially using it. The daemon SHALL retain the highest accepted sequence and the digest of that publication independently of the held copy, so that removing or discarding the copy never lowers it. A publication whose sequence equals the retained highest sequence SHALL be accepted only to restore a missing held copy and only when its digest equals the retained digest and the copy remains admissible under the selected reader and data-revision floor; a lower sequence SHALL never be accepted. Retrieval and validation SHALL run in the daemon, so that every product UI host (macOS Desktop, the Windows shell and the web console), the public CLI and standalone services share one validated state, and SHALL NOT delay startup, Gateway admission or routing. The request SHALL carry no account, installation, model, credential, session or usage identifier beyond a product user agent with the client version. Automatic retrieval SHALL be enabled by default for every service, whether it runs under macOS Desktop, the Windows shell or the web console, or as a standalone service. It SHALL run after a randomized 2–10 minute startup delay and then every six hours with 30 minutes of jitter in either direction, using bounded retry backoff after failure. An explicit refresh SHALL be available in every product UI host and in the public CLI. One service-owned setting SHALL turn automatic retrieval off and back on. Every product UI host and the public CLI SHALL read and change that same setting, and it SHALL survive restart and upgrade. The first time feed content is shown in a product UI host, the host SHALL state that the content comes from hiroute.ai and how to turn retrieval off.

The highest accepted sequence and payload digest SHALL be durably checkpointed outside the removable cache directory before a new cache state is committed. A missing cache state SHALL be reconstructed from that checkpoint with no held copy and automatic retrieval off. If only the activated sequence survives without its digest, only a strictly higher sequence may be accepted. A corrupt owned cache state MAY be explicitly quarantined and reset through the confirmed removal flow only when a valid independent checkpoint preserves acceptance; status SHALL disclose the reason and whether this recovery is available. Newer-schema, unsafe or unreadable state, invalid checkpoints and corruption without a trusted checkpoint remain preserved and refuse mutation. After atomic activation failure, the previous consistent catalog remains available and further feed mutations are refused until restart reconciles durable state.

The transport, parser, private cache and client-facing query paths SHALL support the complete maintained snapshot within documented finite limits; no path may silently truncate records or provenance. The supported wire contract SHALL define its size and structural limits, and publication validation SHALL verify that its complete output fits every client declared compatible with that contract. Unsupported contracts and oversized bodies SHALL leave the last consistent usable snapshot available. Startup SHALL use local validated data without waiting for network retrieval.

Each query or authoring operation SHALL capture one effective metadata snapshot and its generation, including metadata, applicable capabilities, reference-score snapshot and default-price inputs. Refresh SHALL activate a complete validated generation atomically. A check, save, preview or publication whose captured dependency became stale SHALL return the existing revision-conflict outcome and require a fresh operation, rather than silently applying newer facts to an earlier user confirmation. Snapshot activation and price-generation rebuilding SHALL not expose a mixture of generations or reinstall an older default-price generation. Requests already holding a price generation and published execution remain governed by their frozen contracts.

Publication sequence and exact-payload digest acceptance SHALL be scoped to the build-defined wire-contract identity. A retained acceptance identity for a different versioned contract SHALL remain preserved in that namespace and SHALL NOT be compared as a sequence or digest of the current contract. Cache removal, restart, reinstall or retry of the same contract SHALL NOT reset its accepted floor or create a new namespace. Comparable full snapshots SHALL also satisfy the independent semantic data-revision floor; a prior delta format without that identity SHALL NOT supply an inferred semantic revision.

The publication and data-revision acceptance floors SHALL survive cache removal and restart. A corrupt or unavailable held copy SHALL not be partially used. In-flight retrieval invalidated by disable/removal SHALL not restore that copy or reverse the setting. Conditional retrieval SHALL fetch full content when no valid held copy exists. Interrupted activation SHALL recover a complete old or new generation under the existing checkpoint ordering, with an explicit retry/recovery state when necessary.

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

- **WHEN** the service has accepted publication 43, which withdraws a model absent from the embedded fallback that publication 42 listed, and the user removes the held copy, restarts the service and is then served the otherwise valid publication 42
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

#### Scenario: Complete catalog exceeds the old delta-feed size

- **WHEN** a complete maintained snapshot exceeds the former 2 MiB delta-feed bound but fits the supported complete-snapshot contract
- **THEN** the website check, download, parser, cache, restart and product queries preserve all records and provenance without truncation.

#### Scenario: Refuse content beyond supported bounds

- **WHEN** a response exceeds its supported decoded-byte or structural limit
- **THEN** retrieval/validation fails with a bounded size or validation reason and leaves the last consistent usable snapshot available.

#### Scenario: Refresh overlaps a saved preview

- **WHEN** the active catalog changes after a user reviews a connection or Plan preview and before applying it
- **THEN** the apply reports the stale dependency and requires a new preview; it cannot commit facts the user did not review.

#### Scenario: Refresh overlaps a model query and price rebuild

- **WHEN** a query or default-price rebuild begins with one generation and a newer generation activates before it finishes
- **THEN** the query remains internally consistent or reports a revision conflict, and the old builder cannot install defaults over the newer generation.

#### Scenario: Disable while retrieval is in flight

- **WHEN** a user disables retrieval or removes the held copy before an existing fetch completes
- **THEN** the late result cannot undo the setting or repopulate the removed copy.

#### Scenario: Keep versioned publication acceptance identities distinct

- **WHEN** a client moves to a supported versioned full-snapshot contract while retaining acceptance evidence from a different wire contract
- **THEN** the old evidence remains preserved under its original contract identity, valid current-contract publications are checked only against that contract's sequence/digest and the verified semantic data floor, and removal or restart cannot reset an already accepted current-contract floor.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295416952

### Requirement: Feed publications are reviewed, validated and published independently

Official model snapshots SHALL be deterministic projections of the existing maintained metadata sources and independent reference-score inputs. The embedded fallback and website publication SHALL derive from the same reviewed data revision and semantic content, with explicitly versioned wire projections where supported clients require them. Maintainers SHALL not create a second hand-edited website catalog. A compatible client's ordinary model metadata, capabilities and default-price updates SHALL require a data publication rather than a new client release.

Sources SHALL change through reviewed repository pull requests. Validation SHALL run in CI for each change and before publication and SHALL reject malformed schemas, duplicate identities, broken metadata references, forbidden executable authority, lost required provenance or inference markers, invalid featured links, oversized complete output, inconsistent same-revision content and data/publication rollback. Dates of first inclusion SHALL remain stable; publication time SHALL not refresh untouched evidence dates.

Generation SHALL include all maintained descriptive records, including existing embedded models and records without a supported executable binding. Descriptive availability and executable qualification SHALL be evaluated separately. Unsupported execution SHALL not cause omission of a valid descriptive record or rejection of the complete publication. Any claimed executable compatibility/minimum client SHALL be checked against the exact recorded supported release registry and adapters; an unestablished version SHALL remain unclaimed. Incorrect executable claims or malformed references SHALL fail validation.

Only the production model-feed deployment SHALL publish the exact validated artifact. It SHALL run independently of the static website build after reviewed metadata, publication or recorded release changes reach main, using the same production environment and serialized deployment boundary as the website. It SHALL upload only the versioned feed object; ordinary website uploads SHALL refuse a model-feed prefix. PR jobs SHALL validate without publishing or requiring production credentials. The fixed HTTPS origin and production pipeline establish trust; the direct JSON needs no independent signature or signing-key lifecycle. Deployment SHALL retrieve and validate the currently served publication before replacement. A genuine HTTP 404 permits first publication; redirects, unsuccessful retrieval or invalid served content stop publication and preserve the served copy. Identical content may be a no-op; changed publication bytes require a strictly greater sequence, and changed semantic model data require a strictly greater data revision. Deployment SHALL record publication sequence, data revision, digests and source revision.

Before a compatible supported client release is recorded, production publication for that contract SHALL be skipped while ordinary website delivery remains possible. The compatible release's embedded metadata and parser limits SHALL be validated before publication activation. An incompatible wire change used by an already supported release SHALL use a versioned endpoint and intentional legacy support recorded in the existing compatibility register. Older supported readers retain their supported contract and rollback protections; unreleased experimental formats create no compatibility promise. Persisted user choices and supported cache/configuration inputs SHALL retain their meaning through upgrade or receive explicit versioned recovery, never a silent reset.

Source generation and publication SHALL remain reproducible from the recorded reviewed source revision, including squash-stable provenance. A deployment or validation failure SHALL leave the prior website snapshot served and clients' last usable snapshots intact.

#### Scenario: Publish a correction without a client release

- **WHEN** a reviewed metadata change corrects an already embedded model's capability or price within the supported wire and adapter contracts
- **THEN** deterministic generation and production deployment publish the new data revision, and compatible clients can adopt it without upgrading their binary.

#### Scenario: Keep models waiting for execution support visible

- **WHEN** a maintained model has valid descriptive metadata but no recorded supported release can execute its binding
- **THEN** publication includes the descriptive record with the limitation, without inventing executable eligibility or a minimum version.

#### Scenario: Reject understated executable compatibility

- **WHEN** a record claims executable support for a client whose exact released registry or adapter cannot resolve its binding
- **THEN** validation rejects the claim and identifies the incompatible dependency; it does not repair it by downloading execution authority.

#### Scenario: Preserve the served publication on failure

- **WHEN** the deployment cannot retrieve/validate the currently served snapshot or validate the proposed complete snapshot
- **THEN** no replacement is uploaded and the existing served snapshot is retained.

#### Scenario: Prevent semantic rollback with a larger sequence

- **WHEN** a changed publication increases its sequence but reuses a data revision for different semantic content or supplies an older data revision
- **THEN** publication validation rejects it; an intentional correction must use a new data revision.

#### Scenario: Guard embedded semantic identity before publication activation

- **WHEN** reviewed embedded data changes before any compatible supported release has enabled website publication
- **THEN** validation still rejects a lower data revision or different semantic content at the same revision against HEAD and the supplied PR/push baseline, for uncommitted and committed changes; a greater data revision with valid content passes while the publication remains skipped.

#### Scenario: Validate a pull request

- **WHEN** a pull-request workflow checks metadata and generated publication artifacts
- **THEN** it validates deterministic completeness, compatibility and bounds without publishing or requiring production deployment credentials.

#### Scenario: Activate a compatible first publication

- **WHEN** no compatible supported release has been recorded, then a compatible release is recorded and a valid data publication is reviewed
- **THEN** the first state skips feed publication; the later production deployment can publish for that recorded contract without assuming a successful website build already published data.

#### Scenario: Preserve released readers during a contract change

- **WHEN** a future full-catalog wire shape is incompatible with a supported released reader
- **THEN** the new contract uses a versioned endpoint, the supported old contract remains available according to policy, and both derive from the reviewed source without erasing client acceptance state.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295423043

### Requirement: Official model snapshots contain the complete maintained public metadata

HiRoute SHALL publish a complete versioned snapshot of its maintained public model metadata through its fixed hiroute.ai HTTPS interface. The snapshot SHALL include already embedded models and provider-scoped records without a canonical match or executable binding. Publication and descriptive queries SHALL NOT filter out a maintained model solely because no supported client can execute it. Completeness means the maintained catalog, not a claim to cover every model offered worldwide.

The snapshot SHALL preserve canonical identity and aliases, provider/product/region/upstream/protocol scope, endpoint capability overrides, context and output limits, modalities, tool/streaming/reasoning facts, lifecycle and replacement relationships, public connection hints, documented pricing schedules, provenance, per-field inference markers, completeness and usage applicability. Unknown, conditional, conflicting, unsupported and not-applicable states SHALL retain their meanings. Metadata presence or inferred completeness SHALL NOT establish account access, canonical equivalence or executable protocol fidelity. Documented costs SHALL preserve currency, units, cache components, tiers, time windows and application conditions; display-only cost hints SHALL NOT become valuation rules.

Reference scores SHALL be delivered with their independent immutable snapshot identity, exact model/native-reasoning association, evidence and measured/estimated/unknown distinctions. A score batch SHALL capture one snapshot; an explicitly requested unavailable score snapshot SHALL fail as unavailable. Missing ratings SHALL NOT prevent otherwise legal use, and a refresh SHALL NOT reorder a published Plan.

Executable interpretation SHALL remain bounded by installed registry/adapters and exact source qualification. Public endpoint/authentication descriptions may support editable prefill, but the snapshot SHALL NOT install connectors or executable code, change registered destinations/headers/authentication/credential authority, or grant free/subscription entitlement. A well-formed descriptive record with unsupported execution requirements remains queryable with an accurate limitation. Malformed records, duplicate scoped identities or broken references within the snapshot invalidate the complete publication.

Featured entries SHALL remain optional plain localized new-model or model-update announcements referencing a resolvable model, with stable first-inclusion dates and optional links restricted to https://hiroute.ai/. Unresolved entries SHALL be hidden without creating placeholder models. Recommendations SHALL come only from the selected validated official feed, including its retained cache. If no feed is selected or its featured list is empty, recommendations SHALL remain empty; clients SHALL NOT substitute bundled announcements or catalog entries. The embedded metadata fallback remains available independently. Announcement membership and directory order SHALL NOT grant availability or affect routing. An announced update and a metadata revision SHALL NOT establish the provider's actually executed model version or reattribute earlier observations.

#### Scenario: Publish facts for existing and descriptive-only models

- **WHEN** a complete source contains an embedded model with corrected limits, a provider-scoped model without a canonical match and a model needing an unimplemented adapter
- **THEN** all three remain queryable with their scopes and facts; the existing model's new default is available and execution limitations remain explicit for the others.

#### Scenario: Preserve endpoint scope

- **WHEN** the same canonical model or upstream name has different limits and prices on two products or regions
- **THEN** the publication retains separate scoped bindings and schedules; one product's update does not change the other product's defaults.

#### Scenario: Preserve missing and inferred information

- **WHEN** a record has marked inferred tool support, an entitlement-dependent limit and a display-only cost hint
- **THEN** queries retain those distinctions and provenance; they do not present provider-verified capability, an invented numeric limit or an executable price.

#### Scenario: Show inference and its sources in the directory

- **WHEN** the directory shows inferred capability or token-limit fields alongside an observed model record
- **THEN** inferred fields carry visible localized markers; expandable source and rule details read the same captured snapshot, observed fields do not acquire inference markers, and stale detail reads require reloading the directory rather than mixing snapshots.

#### Scenario: Reject executable authority in metadata

- **WHEN** a publication attempts to install an adapter or mutate registered authentication, credential destination or free entitlement
- **THEN** the publication is rejected, and the previous validated snapshot and connection authority remain in effect.

#### Scenario: Update reference scores consistently

- **WHEN** a metadata refresh supplies a new reference-score snapshot while a batch query is in progress
- **THEN** the batch resolves entirely against its captured score snapshot; a later latest query can see the new snapshot, and published candidate order remains unchanged.

#### Scenario: Keep announcements independent

- **WHEN** an announcement refers to a model in the effective catalog and names a provider release behind a rolling alias
- **THEN** it shows the announcement date and actual fact origin without creating another model identity or claiming the earlier session evidence validates that provider release.

#### Scenario: An absent or cleared announcement list stays empty

- **WHEN** no validated feed is selected or a newer publication clears its featured list, while the effective catalog still contains previously recommended models
- **THEN** Home and trial recommendations remain empty, and users may still choose models from the complete effective catalog without any bundled recommendation being restored.

#### Scenario: Reject an incomplete or inconsistent publication

- **WHEN** the downloaded publication contains a malformed record, duplicate scoped identity or a missing referenced metadata entity
- **THEN** the whole publication is rejected rather than partially installed or completed from stale embedded records.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295407404

### Requirement: Effective model defaults preserve user intent across official data updates

HiRoute SHALL distinguish explicit user configuration from fields that follow official defaults. For editable fields and source-scoped prices, explicit user values SHALL take precedence over the selected official snapshot. The current compatible validated website snapshot, including its retained cache, SHALL supply defaults for existing and new models. The embedded catalog SHALL provide the fallback when no usable downloaded snapshot exists. Matching SHALL use stable scoped identities rather than display names; provider- or endpoint-specific facts SHALL take precedence over a general model fact within their declared scope.

Embedded and downloaded official snapshots SHALL carry comparable monotonic data revisions bound to immutable semantic content digests, independently of website publication sequence and client version. On upgrade, an older cached data revision SHALL NOT replace a newer compatible embedded revision. At equal data revision their semantic content SHALL agree; conflicting content SHALL be rejected. A validated newer cache SHALL remain effective across an upgrade whose embedded data is older. A later downloaded update SHALL satisfy both publication acceptance rules and the installed/retained data-revision floor; a higher publication sequence alone SHALL NOT authorize older model data. An intentional correction is a new data revision. Publication transport identity and semantic data identity SHALL not be conflated.

The selected complete snapshot SHALL define its default record set. Missing records, explicit withdrawals and unknown fields in that snapshot SHALL NOT be filled from an older snapshot. An unsupported executable binding SHALL remain unsupported rather than acquiring an older binding implicitly. User overrides remain saved independently, but do not grant access to a withdrawn registered model or override an unsupported adapter.

Explicit user values, including valid false or zero values and deliberately unknown optional prices, SHALL survive refresh, restart and upgrade. Fields still following defaults SHALL track the selected snapshot. Restoring default SHALL remove the corresponding override and immediately resolve the current scoped default or unknown; it SHALL NOT copy a default into permanent user authority. Existing saved provenance SHALL govern conversion: a value without sufficient evidence of catalog-following intent SHALL be preserved until the user explicitly chooses to follow defaults. Equality with an old default is not proof of that intent.

Network failure or disabled automatic retrieval SHALL retain the usable selected cache. If removal, corruption or incompatibility leaves no usable downloaded copy, embedded fallback SHALL be reported with its actual revision, freshness and reason, without claiming current website coverage or changing saved overrides and frozen publications. That explicit fallback SHALL NOT reset the retained publication/data acceptance floor. Withdrawal and absence apply to the currently selected snapshot; selecting a disclosed embedded fallback is not a successful adoption of a lower website publication.

#### Scenario: Refresh an embedded model with a manual override

- **WHEN** a newer official snapshot changes a model's tool support and context limit while the user has explicitly set only its context limit
- **THEN** the catalog-following tool value updates, the explicit context limit survives, and each field retains its effective origin.

#### Scenario: Restore a default after refresh

- **WHEN** a user restores an overridden capability or price after the official default changed or became unknown
- **THEN** the override is removed and the current default or unknown is used, including after restart and a later update.

#### Scenario: Do not infer intent from equal values

- **WHEN** a retained user-declared or ambiguous capability happens to equal the old embedded default
- **THEN** the value remains preserved during upgrade and refresh; it does not silently become catalog-following.

#### Scenario: Do not backfill a selected snapshot

- **WHEN** the current complete downloaded snapshot omits an old embedded model or marks its cache-input price unknown
- **THEN** the old model/default price is not reintroduced from embedded data while that snapshot is selected.

#### Scenario: Compare cache and embedded revisions on upgrade

- **WHEN** an upgrade embeds a newer data revision than the cached snapshot, or embeds an older revision than a still-compatible cache
- **THEN** the higher compatible data revision supplies defaults in each case; saved user intent and the acceptance floor survive.

#### Scenario: Reject a publication carrying older data

- **WHEN** a served publication has a higher publication sequence but a data revision below the installed or retained acceptance floor
- **THEN** it is not selected and its older facts cannot replace current defaults; the last consistent usable state remains available.

#### Scenario: Disclose embedded fallback

- **WHEN** there is no usable downloaded snapshot after cache removal or corruption
- **THEN** embedded data is shown with its own revision and the fallback reason; user overrides, frozen publications and acceptance floors are retained.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/191#issuecomment-6295415513

## Implementation references

- [Metadata records](../../../crates/domain/src/compute/model_metadata_records.rs) and
  [source price contract](../../../crates/domain/src/compute/source_price/mod.rs).
- [Price persistence scenarios](../../../crates/local-storage/src/control/prices/tests.rs).
- [Current subscription/model query](../../../crates/application/src/compute_management/query_v3.rs).
