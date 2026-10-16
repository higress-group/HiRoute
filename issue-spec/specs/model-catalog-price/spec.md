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
A frozen publication cannot revive invalid run tokens, expired deadlines, cancelled
runs, or revoked source/Key access; current Plan delegation policy governs new
Start/Continue without automatically cancelling accepted work.

#### Scenario: Relay crosses a time-price boundary

- **WHEN** two Attempts in one logical request execute on opposite sides of a
  pricing time boundary
- **THEN** they MUST share the admitted price generation but freeze their own
  applicable time rules; neither uses a newly published generation.

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

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6220387895

## Implementation references

- [Metadata records](../../../crates/domain/src/compute/model_metadata_records.rs) and
  [source price contract](../../../crates/domain/src/compute/source_price/mod.rs).
- [Price persistence scenarios](../../../crates/local-storage/src/control/prices/tests.rs).
- [Current subscription/model query](../../../crates/application/src/compute_management/query_v3.rs).
