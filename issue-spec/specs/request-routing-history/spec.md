# Recorded route identity

## Purpose

A historical request identifies the route that actually served it. Current defaults,
plan names, and surviving content cannot substitute for execution evidence.

## Requirements

### Requirement: Composite stage quality is independent of model attribution

An actually captured stage MAY use several models or execution groups. When all
accepted executions share a frozen Plan revision, branch and assessment rubric,
the next normal classification SHALL retain an overall stage quality question
bound to the immutable actual request membership. This assessment SHALL contain
no single-model or profile identity and SHALL NOT contribute to a single-model
capability average or competence upgrade. A missing common rubric withdraws this
optional assessment, without declaring missing captured output or disabling
classification of later input. Custom extensions retain their own rubric contract.

Capture completeness, source uncertainty, association uncertainty and executor
attribution SHALL remain separate facts. A successful retry may extend a new stage
revision; it SHALL NOT rewrite an earlier failed revision or an existing assessment's
membership. A later clean single-model stage SHALL again support its normal
single-model quality evaluation.

#### Scenario: Relay, failure and exact retry precede a new question

- **WHEN** one task accepts work from a Regular model and a Primary relay, then
  encounters a temporary upstream failure and completes after an exact native retry
- **THEN** the failed snapshot remains unchanged, actual accepted output and failed
  request membership are retained, and the next question can assess the joint
  result under its common rubric without fabricating single-model credit.

### Requirement: Excerpts use the recorded input position

New requests SHALL record a bounded original canonical message position when
current-input selection is established, independently of optional retained history.
This position identifies the first selected input message, not the complete input
batch or human authorship. Source uncertainty SHALL remain explicit. Invalid or
unavailable selection SHALL retain that uncertainty rather than choose a late
User-role envelope heuristically.

Catalog readers SHALL opt in to position metadata. Existing calls and cursor
bindings without that option SHALL retain their original shape. Evidence lookup
SHALL remain bounded to the authorized retained request, authenticate its original
carrier and digest, and honor deletion. Missing legacy evidence SHALL be shown as
unrecorded while the original conversation remains readable. Ordinary startup and
session reads SHALL NOT scan all historical facts to backfill positions.

#### Scenario: Native context follows the actual input

- **WHEN** a client appends runtime context or a time envelope after its selected
  input, or a historical record has no retained selected-input position
- **THEN** the excerpt selects only the recorded position or declares it unrecorded;
  original context is neither deleted nor presented as the new question.

### Requirement: Initial choice and actual relay are displayed separately

The stage-start explanation SHALL use its recorded selection group. The accepted
model and any request relay SHALL remain separately visible. Missing selection
evidence SHALL be explicit; the final executor or probability alone SHALL NOT
reconstruct an initial group.

### Requirement: Request summaries preserve admitted routing identity safely

Each ObservedRequestSummary MUST return routing-context state and, when proven,
plan ID/revision plus the safe display name frozen at admission. Execution facts,
a separate safe projection, or an exact run relation may establish that identity.
Conflicting evidence yields conflicted state with null references/name; absent or
unsupported historical evidence is explicitly unavailable. A proven route with no
historical name may expose the route while declaring the name unavailable. Current
Agent/default/session plan cannot fill a missing historical field.

The safe routing projection is retained separately from sensitive original facts
and receipt content. Deleting/expiring those carriers cannot remove safe admitted
identity from new records. Public Facts queries cannot expose the private projection
or raw receipts to reconstruct missing display data.

#### Scenario: Two plans serve requests in one session

- **WHEN** the same session contains requests served by different plan revisions
- **THEN** every row MUST retain its own recorded context, including the original
  safe name after a later rename.

#### Scenario: Content is removed or evidence disagrees

- **WHEN** sensitive receipt content is cleaned up, or receipt and run-link identity
  conflict
- **THEN** independent safe identity MUST remain available when consistent, while
  conflict remains explicit rather than resolved using today's default plan.

### Requirement: Explain recorded model transitions through the management CLI

HiRoute SHALL expose an ordered read-only model-transition view for ordinary supported Agent conversations and existing Worker-linked execution, with actual accepted models/reasoning, time, frozen Plan/publication, recorded reasons, completeness and exact receipt/content anchors. Adjacency, relay separation and unknown-change handling follow session-history's "Turn changes and request relay have different identities". Compaction continuity follows "Verified native compaction links preserve session identity": proven native compression links allow continuous analysis with member identities preserved; an unproven boundary remains separate with an unknown predecessor. A continuation link does not itself establish adjacent user turns. Existing stage records supply frozen decision evidence, but a stage boundary is not automatically a model or reasoning change; compare actual accepted models and explicit reasoning settings. A whole-profile digest difference alone cannot prove a reasoning-only change. Initial selection, between-turn model change, compression continuation boundary, within-request fallback, same-model credential relay, reasoning-only change, unchanged and unknown remain distinguishable. A proven compression boundary SHALL expose the predecessor's last accepted and successor's first accepted model/reasoning where recorded, state whether the model changed and mark turn_context_reset; it SHALL NOT assert user-turn adjacency or count as between_turn_change. Missing endpoints remain unknown or pending. Reasons use recorded decision source/probability/threshold/group, applicable competence and actual failure/continuity/configuration facts; never rerun judgment or guess from current settings. Loss of transient turn context SHALL disclose the reset without hiding a proven change or inventing prior decision context. Unfinished turns expose closed receipts with comparison pending. No routing-policy change, Worker requirement or model inference is introduced. Interleaved or overlapping compression endpoints SHALL make model_changed unknown even when the recorded models differ. Verified ancestor proof SHALL remain usable for normal spaced native requests within its declared cache lifetime, independently of fresh current-state evidence. Observations SHALL preserve their actual order/age: replayed older proof cannot override newer state, and fresh verified proof can restore a quarantined relation when it is again valid.

#### Scenario: Explain a proven economy-to-primary change

- **WHEN** two proven adjacent turns of the same ordinary managed Agent conversation were served by different accepted models
- **THEN** the CLI identifies the actual from/to models and linked messages/requests and explains the recorded selection facts, including probability/threshold or applicable competence evidence when available.

#### Scenario: Keep compaction inside its proven conversation

- **WHEN** the native Agent compacts context but continues with the same valid scoped session identity
- **THEN** the view retains the same conversation association and the actual decision boundary without creating a new identity from rewritten text or treating summary text as a new cause.

#### Scenario: Follow a proven native compression continuation

- **WHEN** native compaction issues a new session ID and the managed adapter supplies valid same-scope compression lineage
- **THEN** opt-in conversation and routing views follow the proven continuation while retaining each physical session and receipt; actual turn adjacency is established separately from request/turn evidence.

#### Scenario: Explain a change after routing context was lost

- **WHEN** a proven session resumes after its transient routing turn context expired or the service restarted, and the next turn is served by a different model
- **THEN** the view reports the actual change from persisted request order and the recorded new selection, and states that the selection had no prior turn context instead of hiding the change or inventing continuity.

#### Scenario: Distinguish selection from relay

- **WHEN** one request relays to another model or credential, or the first visible request has no proven predecessor
- **THEN** the view describes that actual event separately from an adjacent-turn model change and labels initial/unknown predecessor evidence honestly.

#### Scenario: Do not infer absent causes

- **WHEN** historical reason/body facts are missing or a Plan publication differs between requests
- **THEN** available execution is still shown, absent causes remain unknown and a revision difference alone is not asserted as the cause of a model switch.

#### Scenario: Preserve true adjacency and identity

- **WHEN** the user filters to switched/primary turns or follows another page
- **THEN** the view applies session-history adjacency and verified-continuation rules before display filters/pages; hidden middle turns, unsupported relations and missing order never manufacture a switch.

#### Scenario: Keep an unproven rotation separate

- **WHEN** a native session rotates but lineage support or a required link is absent, conflicting or outside the managed scope
- **THEN** valid model use continues; physical sessions remain readable separately with unknown continuation/predecessor instead of guessed association.

#### Scenario: Do not turn a stage boundary into a model switch

- **WHEN** context reset, branch/Plan changes or mixed execution create a new stage but the proven compared models and reasoning settings are unchanged
- **THEN** the view exposes the applicable reset/context or mixed evidence and does not report a model or reasoning switch merely because the stage ID changed.

#### Scenario: Separate reasoning from other profile changes

- **WHEN** a protocol profile digest changes while explicit recorded reasoning settings do not, or reasoning evidence is missing
- **THEN** the digest difference is configuration context; it cannot establish a reasoning-only change, and unavailable reasoning evidence stays unknown.

#### Scenario: Explain compaction within a user turn

- **WHEN** a verified rotating compaction occurs between model/tool calls within one user turn, with accepted evidence on both sides
- **THEN** the view emits a continuation_boundary with physical member/request anchors, actual from/to models and explicit reasoning, model-change status and turn_context_reset; it is separate from between_turn_change, whether the model stayed the same or changed.

#### Scenario: Keep an unfinished compression endpoint honest

- **WHEN** a proven compression link exists but an accepted predecessor or successor endpoint has not been retained or closed
- **THEN** the boundary keeps its relation and available anchors with unknown or pending endpoint/comparison evidence; no served model, switch or turn adjacency is invented.

#### Scenario: Carry verified lineage on normally spaced requests

- **WHEN** a real rotation has completed native verification and the next warm managed request occurs after ordinary model-generation or tool time
- **THEN** the cached valid ancestry is carried despite the spacing; unavailable current state remains unknown, older proof cannot undo newer state and later fresh valid verification can restore quarantined links.

#### Scenario: Keep interleaved compression endpoints unknown

- **WHEN** accepted predecessor work overlaps or occurs after the successor endpoint, or their causal order is unproven
- **THEN** the boundary retains actual endpoint evidence and turn_context_reset but reports model_changed unknown, without inventing an ordered model switch.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/125#issuecomment-6229104803

## Implementation references

- [Authorized relation and routing projection](../../../crates/observation/src/query_v2/relation.rs).
- [Observation query contract](../../../crates/observation/src/query_v2/README.md).
