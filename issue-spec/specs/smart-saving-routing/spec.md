# Smart-saving routing

## Purpose

Smart saving chooses between economy and primary model groups for one task scope.
It uses the shared decision/competence policy without exposing category branches.

## Requirements

### Requirement: Two groups are ordered relay lists, not escalation ladders

The editor MUST label this mode Difficulty routing and present judgment source,
Regular (formerly Economy) group, and Primary group. Each
group has ordered availability-relay candidates; it has no independent competence
upgrade chain. New configuration favors a decision-model source but requires the
user to select a saved connection before publication. Users may explicitly choose
a custom extension or deterministic heuristic; absence of a connection cannot
silently change the configured mode.

Heuristic classification uses the existing deterministic English/Chinese policy,
generates no model-derived competence score, and hides probability/rubric controls
while retaining their parked edits. Custom extensions directly select regular or primary and hide judgment controls while preserving parked edits. Built-in connections use the published thresholds and rubric of the shared [decision policy](../decision-services/spec.md); provider-specific hidden thresholds cannot override them.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6236473660

#### Scenario: Existing route uses the Familiar threshold

- **WHEN** using a built-in decision connection, a saved smart-saving route with threshold 0.80 is opened
- **THEN** the editor MUST display Difficulty routing and Familiar without changing
  the stored mode, candidate ordering or selected decision source.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: Degree and competence thresholds are independent

These thresholds govern built-in decision connections. Custom extensions choose regular or primary directly and have no HiRoute simple threshold, competence floor or protection rule.

Default simple probability threshold is 0.8 and competence floor is 0.5, both
finite within 0..1. Economy is selected when `P(simple) >= simple_threshold` and
there is no currently applicable complete assessment strictly below the floor.
Equality meets either threshold. Otherwise primary is selected with a distinct
complex-task, low-competence, or degree-unavailable reason.

Degree instructions and competence standards are separate. The default rubric's
incompetent/partial/competent anchors map to 0/0.5/1. Primary below the floor remains
primary; there is no third group. Missing/partial scores are unrated, not zero.
Judgment settings may be collapsed with a threshold summary but invalid hidden
fields must expand on publish failure.

Native routes published with the applicability protocol use the same unchanged T=0.8 and F=0.5 defaults. Its third question
MUST only control whether an otherwise eligible historical low score applies to the
current work, using the fixed 0.5 applicability boundary and the shared
[decision policy](../decision-services/spec.md). It never tunes T/F, changes the
raw assessment anchors, merges current difficulty with historical performance or
overrides a complex current task. Research thresholds are not product defaults.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/139#issuecomment-6235629301

#### Scenario: Exactly at both thresholds

- **WHEN** using a built-in decision connection, simple probability is 0.8 and a complete applicable score is 0.5
- **THEN** Default policy MUST allow economy; equality is not incompetence.

#### Scenario: C suppression with unchanged thresholds

- **WHEN** using a built-in decision connection, simple probability meets the configured threshold, a complete eligible score is strictly below the floor and valid applicability is below 0.5
- **THEN** Economy MUST be allowed by suppressing only that historical trigger, without changing either threshold or score.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: Every new user turn can return to economy

New user input MUST receive a new judgment, potentially assessing the preceding
actual execution stage. No cross-turn one-way upgrade cursor survives. A previous
primary outcome does not prevent economy on a later qualifying turn, and old
observed low scores cannot be repeatedly reapplied. Tool continuations/replays in
one continuous turn inherit the frozen decision. A valid ContextHold MUST always prefer the
previously successful candidate inside the newly selected group, independently of
any saved follow-up field; it cannot preserve a candidate across a group change.

#### Scenario: A primary turn is followed by an economy turn

- **WHEN** a new user message qualifies for economy after an actual primary execution
- **THEN** the Gateway MUST select economy and cannot reuse the primary candidate through its hold.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/133#issuecomment-6220133330

### Requirement: Decision failure and candidate failure have different effects

An invalid degree selects primary with a judgment-failed reason, not an invented
probability. Whole decision-call failure uses heuristic fallback; source cancellation,
deadline, and input-integrity failure terminate the request. Candidate unavailability
relays economy candidates, then permitted primary candidates, then fails. Direct
primary selection cannot downgrade to economy. Relay cannot manufacture competence,
retry committed output, or add another fallback list.

#### Scenario: Economy is rate limited

- **WHEN** all economy candidates exhaust their permitted Key/availability relay
- **THEN** the configured primary group MAY serve the request, recorded as
  availability relay without a synthetic low score or rewritten degree judgment.

### Requirement: Presentation distinguishes selection from later assessment

Plan quality and session stage views MUST preserve actual economy/primary group,
model, native reasoning, position, opening probability/threshold, protection target,
and any subsequent availability relay. Later assessment belongs to the earlier
stage and its frozen rubric. Current drafts cannot reinterpret history; aggregation,
coverage, and drill-down follow [session history](../session-history/spec.md).

### Requirement: Trust levels express a routing choice, separately from evidence

Built-in difficulty routing and two-group categories MUST offer Trial 0.90, Familiar 0.80,
Trusted 0.65 and Hands-off 0.50 from one shared level definition. The existing 0.80
threshold displays Familiar. A nonmatching value remains an exact Custom value;
reading or opening an editor MUST NOT round it. Heuristic routing and custom extensions offer no trust
levels. The stored mode remains `smart_saving`, the editor keeps its `economy` candidate field,
and the execution group remains `regular`. Existing Economy references name this same group.

Changing a level changes only the simple-probability threshold. It MUST NOT alter
competence floors, permissions, tool approvals or sandbox settings. The editor
states the active revision's evidence separately and explicitly identifies zero
samples. Keeping a level is a normal choice, not incomplete onboarding.

The safety-net explanation follows the selected judgment settings. With System One native judgment, a complete Regular score below the competence floor protects the next turn when the earlier problem still matters; unavailable applicability conservatively retains protection, while a fixed or unrelated problem allows simple tasks to remain with Regular. Custom extensions own their decision policy; HiRoute does not apply low-score protection to their decisions. Published routing follows its frozen settings; editor explanations describe the next publication.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235185303

#### Scenario: Custom threshold and adjacent suggestions

- **WHEN** using a built-in decision connection, a route has threshold 0.72
- **THEN** it displays Custom 0.72 until an explicit edit; an upward suggestion is
  0.65 and a downward suggestion is 0.80, without silently replacing the value.

#### Scenario: Native safety-net explanation

- **WHEN** using a built-in decision connection, the selected decision connection is System One
- **THEN** the editor explains that relevant low-score history protects the next turn, unavailable applicability retains protection, and fixed or unrelated history allows simple tasks to remain with Regular.

#### Scenario: Custom extension has no HiRoute safety net

- **WHEN** the selected decision connection is a custom extension
- **THEN** the editor explains direct extension group selection and observation-only scores, with no HiRoute threshold or low-score protection controls.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: Threshold replay previews recorded stages

While a user chooses a trust level or threshold for a published difficulty-routing Plan or two-group branch, HiRoute SHALL show a replay preview computed from that Plan's retained recorded stages in a disclosed scope (published revision and time window, defaulting to the active revision's last seven days). For a candidate threshold t, a stage SHALL count as regular when its recorded decision-model simple probability is at least t and it was not selected for low-competence protection, and as primary otherwise. Stages selected for low-competence protection SHALL be counted as primary and reported separately, because their protection depends on earlier scores. Stages without a recorded model probability, including heuristic, degree-unavailable, decision-fallback, single-group and availability-relay selections, SHALL be excluded and counted by reason. The preview SHALL show regular and primary counts and shares among included stages, the excluded counts, the recorded share at the threshold actually in force, and a label that it is an estimate from history rather than a prediction. The preview SHALL state that it estimates how recorded work would have been allocated and does not establish competence on stages that the regular group did not handle. The preview SHALL be computed by the backend over the complete in-scope population or return a classified retryable failure; it SHALL NOT call a model, rerun a judgment or change the Plan. The same facts SHALL be available through the public observation CLI with help and schema discovery.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235187673

Custom-extension decisions MUST be excluded by their recorded protocol, independently of any probability field. They cannot enter threshold replay or threshold suggestions. A current custom-extension Plan exposes no threshold policy from parked draft values.

#### Scenario: Preview at a lower threshold

- **WHEN** using a built-in decision connection, a Plan has 100 in-scope stages with recorded probabilities, 10 low-competence protections and 20 heuristic stages, and the user previews a lower threshold
- **THEN** the preview counts the 10 protections as primary, excludes the 20 heuristic stages with their reason, and reports the regular share among the remaining included stages at the new threshold.

#### Scenario: Equality meets the threshold

- **WHEN** using a built-in decision connection, a recorded stage probability equals the candidate threshold
- **THEN** the stage counts as regular, consistent with the runtime rule that equality meets the threshold.

#### Scenario: No usable history

- **WHEN** using a built-in decision connection, the scope contains no stage with a recorded decision-model probability
- **THEN** the preview states that no recorded evidence is available and shows no estimated shares instead of zero.

#### Scenario: Higher level from lower-level history

- **WHEN** using a built-in decision connection, a Plan's history was recorded at the Trial level and the user previews Hands-off
- **THEN** the preview shows the estimated regular share and states that the stages it would newly send to the regular group were handled by the primary group and carry no regular-group competence evidence.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: Suggestions require complete, revision-scoped evidence

Suggestions MUST use the full retained active published revision, not a sample or
just the displayed time window. At least 30 complete-scored Regular stages are
required in the evaluated direction. A competence rate below 70% suggests the
adjacent more conservative level first, without an upward coverage gate. At the
most conservative boundary, Return to previous routing is offered only when a
currently mapped Agent preset has a verified recoverable configuration; otherwise
the suggestion is to change the Regular model.

An upward suggestion requires competence at least 85%, scoring coverage at least
60%, and complete scores from at least three sessions. At Hands-off, qualifying
evidence may suggest division. Rates use the counts defined in
[session history](../session-history/spec.md). Custom thresholds use the nearest
strict neighbour; a missing neighbour cannot cause an invented numeric jump.

A model-update announcement for a Regular candidate scopes upward/division evidence
to stages beginning after that announcement day in UTC. Older evidence stays
visible and still informs downward evaluation. Announcement dates are editorial
boundaries, not provider deployment/version proof. Removing an announcement does
not manufacture a version identity. A failed or oversized read is retryable and
MUST NOT appear as no suggestion. Home, navigation and Plan-list indicators use
the same suggestion result. Applying a suggestion opens an ordinary editable
change; it never silently publishes.

Work deferred by the bounded suggestion-read budget MUST be disclosed as pending,
separately from actual unavailable/oversized reads and a complete empty result.
The visible view SHALL continue pending reads automatically with bounded backoff,
retaining the normal budget, current-evidence checks and actionable real errors.
Completed work may be reused only under the existing reader, revision, visibility
and evidence bindings. Hidden clients and inactive suggestion subscriptions SHALL
NOT continue polling.

#### Scenario: High rate with incomplete coverage or only old-version evidence

- **WHEN** historical competence is high but coverage is below 60%, fewer than
  three sessions contribute complete scores, or a current update has no qualifying
  post-announcement scores
- **THEN** the UI retains history and explains the missing evidence, with no upward
  or division suggestion; observed low scores and protections remain visible.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6235188590

### Requirement: User-facing names describe difficulty routing

User-facing Desktop text, website guides, README files and user documentation SHALL call the smart-saving mode "Difficulty routing" (按难度分流), For built-in decisions, it is described as giving simple tasks to the regular model and giving complex tasks, or turns requiring low-score protection, to the primary model. With System One native judgment, a complete Regular score below the competence floor protects the next turn when the earlier problem still matters; unavailable applicability conservatively retains protection, while a fixed or unrelated problem allows simple tasks to remain with Regular. Custom extensions directly select the group using their own policy; HiRoute does not apply low-score protection to their decisions. Published routing follows its frozen settings; editor explanations describe the next publication. They SHALL call its groups "Regular" (常规) and "Primary" (主力), the same names that two-group custom branches use. Names SHALL NOT present the mode or a group as a saving, because a regular model can cost more than the primary model. Desktop cost statements remain labelled estimates on the disclosed basis. README files, the website and user documentation MAY present lower cost and longer-lasting subscriptions as the benefit of placing a cheaper model in the Regular group, provided they state that condition and never promise savings unconditionally. The new-routing dialog SHALL keep the four modes and SHALL offer "Try a new model" as a template of difficulty routing. Further templates MAY be added only as prefilled difficulty-routing drafts without new semantics. Display-name changes SHALL leave stored identifiers, CLI arguments, JSON values, schemas, machine-readable output and routing semantics unchanged, including `smart_saving`, `smart` and the stored economy group. Human-readable CLI help MAY show the display name beside the identifier.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6236473660

#### Scenario: Existing Plan after upgrade

- **WHEN** display names change and a user opens a Plan saved as `smart_saving` before that naming change
- **THEN** Desktop shows Difficulty routing with Regular and Primary groups. The display-name change MUST NOT alter stored content, digest or runtime behavior; separately specified extension protocol changes retain their own compatibility contract.

#### Scenario: CLI compatibility

- **WHEN** only the display names change and a script uses the existing smart-saving CLI arguments and JSON output
- **THEN** The display-name change MUST leave command arguments and machine-readable output unchanged; human-readable help may use the new display name. Extension wire evolution follows its explicit API contract.

#### Scenario: Expensive regular model

- **WHEN** a user places a model that costs more than the primary model in the regular group
- **THEN** no Desktop text calls the mode or the group a saving, and the model performance view shows any positive cost difference as additional cost.

#### Scenario: Conditional cost benefit in introductory material

- **WHEN** a README or website introduces difficulty routing and its cost benefit
- **THEN** it uses the Difficulty routing, Regular and Primary names and may explain that placing a cheaper model in Regular lowers cost or makes subscriptions last longer, without promising savings for every configuration.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/150#issuecomment-6246370790

### Requirement: The trust view distinguishes released historical protection

The Plan model-performance protection card MUST show how many historical low-score protections were released because the previous problem was fixed or unrelated. Release removes that historical constraint; the current difficulty still determines the base group, and complex work may remain Primary. This count describes retained routing decisions and remains separate from the trust report's protected-stage count. A numeric count is permitted only when routing-observation coverage is complete and its Plan, published-revision selection, effective time interval and branch scopes match the protection card's trust report. Model and score filters must not relabel a Plan-wide count as a filtered-model count. Cohorts may be aggregated only within the matching branch scopes. A complete matching recorded cohort with no releases shows zero. Missing observations, incomplete coverage or any unproven scope alignment MUST show an unavailable value and a plain-language reason; unknown evidence MUST NOT become zero. The observation disclosure remains available with its own scope when a count cannot be placed beside protections. The release explanation uses capability language and MUST NOT display internal protocol labels, digests or question identifiers. No route, assessment, suggestion threshold or stored record changes through this view.

The view MUST offer last-24-hours, default last-seven-days and all-retained scopes. For every period, the stage and routing-decision reads MUST use the same captured explicit interval with its lower bound inside the retention boundary and a bounded read margin. Reads taken at different instants MUST NOT make an otherwise complete matching release count unavailable at the normal seven-day edge. Actual effective-interval mismatches still show unavailable. Changing the displayed period does not narrow the full active-revision evidence used for suggestions.

The routing release aggregate SHALL remain the result of the base read paired with the trust report. Later detail score/model filters and pagination SHALL NOT replace it with a separately clipped aggregate, even after the bounded read margin has elapsed. A grouping, period or revision change, explicit refresh or retry SHALL capture a fresh interval and reread both sources together. Actual scope mismatches remain unavailable.

Every Plan-performance entry SHALL use the same publication selection for both reads. A loaded publication revision pins both reads to that revision. An entry without it SHALL resolve the active revision before reading current-version quality; unresolved active selection SHALL NOT silently mean all retained versions. Explicit historical and all-version selections retain their own scopes. Responses from an earlier revision or refresh SHALL NOT populate the replacement scope, including its base routing aggregate.

A protection opportunity is a non-inherited valid C observation with an eligible complete finite low score and recorded Applied, RetainedUnavailable or Suppressed action, independently of simple probability. Suppressed counts only actual suppression in those opportunities. Zero ratio denominators remain unavailable. Old complex/not_needed records do not become opportunities. Threshold replay preserves recorded protected stages as Primary without applying the current algorithm to old unprotected records.

Statistics use protection actions recorded under the rules active at the time. A retained scope may contain records from different rule versions; these counts and rates do not establish a uniform-policy effectiveness comparison. Do not invent a rule version from dates or Plan identity, relabel old events, or backfill hypothetical protection opportunities.

Related source specifications:
- https://github.com/higress-group/HiRoute/issues/140#issuecomment-6253018308

#### Scenario: Session entry has no loaded publication revision

- **WHEN** the session timeline opens Plan performance without a loaded revision and the active publication has complete matching observations with zero releases
- **THEN** the entry resolves that publication, reads its exact quality scope and displays zero; a failed active-revision read stays unavailable and retry can recover without recreating the view or broadening its scope.

#### Scenario: Publication and retained-version changes preserve pairing

- **WHEN** the loaded or resolved active revision changes, the user selects a historical revision or all retained versions, or an earlier read finishes after refresh
- **THEN** quality and trust evidence use the selected common revision scope and interval; earlier responses cannot replace the new scope, and genuine revision mismatches remain unavailable.

#### Scenario: Inspect every period inside the retention boundary

- **WHEN** the user opens the default last-seven-days view or selects all retained records or the last 24 hours, and the two complete evidence reads occur at different instants
- **THEN** both use the same explicit retained interval and the protection card shows the aligned release count; detail navigation preserves the pair and grouping captures a fresh paired interval.

#### Scenario: Aligned complete evidence

- **WHEN** a trust report and complete routing observations match Plan, revision, time and branches and matching cohorts contain five released protections
- **THEN** the protection card shows five releases separately from its protected-stage count and explains that the earlier problem no longer constrained routing while the current difficulty still selected the actual group

#### Scenario: Incomplete or mismatched scope

- **WHEN** observations have partial coverage or a different Plan, revision, interval or branch scope
- **THEN** the protection card shows an unavailable value with the corresponding reason, while the observation disclosure retains its actual scope

#### Scenario: Missing and zero evidence differ

- **WHEN** there are no recorded historical-protection observations, or a complete matching recorded cohort has zero releases
- **THEN** missing evidence is unavailable with a reason, whereas the complete matching recorded cohort shows zero

#### Scenario: Filter and paginate after the read margin expires

- **WHEN** the default seven-day or all-retained view remains open for more than one minute, then the user selects low-score stages and loads another page
- **THEN** the release count stays bound to the original paired base read; grouping afterward refreshes both sources with one new interval.

#### Scenario: Actual opportunity population

- **WHEN** the scope contains simple Applied, complex Applied, complex Suppressed, RetainedUnavailable, old complex NotNeeded and an inherited duplicate
- **THEN** opportunities=4 and suppressed=1 with ratio 1/4 when coverage is complete; old and inherited entries are excluded.

#### Scenario: Scope remains paired

- **WHEN** detail filters/pagination occur after a paired retained read, or the period/revision changes
- **THEN** detail navigation preserves the pair; scope changes reread both; unproven alignment remains unavailable.

#### Scenario: Replay preserves old facts

- **WHEN** new complex protection and an old complex/not_needed stage are replayed at another threshold
- **THEN** the protected stage remains Primary and the old stage is not rewritten or relabelled protected.

Source SPEC comments:
- https://github.com/higress-group/HiRoute/issues/226#issuecomment-6440838906

## Implementation references

- [Two-group policy](../../../crates/gateway/src/core_runtime/classification/group_policy.rs)
  and [policy scenarios](../../../crates/gateway/src/core_runtime/classification/group_policy_tests.rs).
- [Real multi-turn listener scenarios](../../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches.rs).
