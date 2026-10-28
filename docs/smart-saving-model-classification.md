# Decision models for smart routing

[简体中文](smart-saving-model-classification.zh-CN.md)

Add a decision model or custom extension under **Models → Decision models**. Configure provider,
model, complete endpoint and credential, save and test, then select its saved revision in a route.
Saving/publishing a plan does not silently invoke the service. Credentials reuse protected input
and existing storage. Built-in providers need no separate Jev deployment.

Save-and-test checks the exact saved revision with fixed synthetic input, without reading real
conversations, executing a task or producing competence samples. Editing the connection creates
a new immutable revision; a published route keeps its selected revision until explicitly republished.
The list shows the latest revision, while an already saved historical revision remains publishable.

Difficulty routing defaults to the decision-model choice; users may explicitly choose heuristic rules or
a custom extension. Regular and Primary groups retain the existing candidate picker, ordering and
reasoning controls. For built-in connections, judgment settings start collapsed: simple probability defaults to 0.8, competence
floor to 0.5, and both degree and assessment prompts are editable.

Every new user message decides again. With built-in connections, Regular requires this call's simple probability to meet
the threshold with no applicable complete low score. Complex or fresh low competence selects primary.
Only a fresh complete score bound to the preceding actual stage, in the same category and compatible
with the published configuration and assessment criteria, can trigger protection. Missing/partial
scores do not preserve old protection; new turns do not automatically reset to regular.
Tool continuation and replay reuse the frozen decision only when the same turn is identifiable,
history continues, and the decision remains reusable. Compaction or history reconstruction that
breaks continuity or prevents reuse triggers a fresh decision. Candidate hold stays inside the newly
selected group. Availability relay never downgrades from primary or crosses task categories.

Custom branches first classify task intent, then optionally judge degree within that category. Each branch
has regular candidates and optional primary candidates. Built-in connections follow plan judgment or copy the complete
settings for independent editing. No primary means no degree question, while competence remains observable.
Category conditions, degree criteria and assessment criteria are separate inputs.

Existing plan/session views show actual group/model/profile, frozen thresholds and selection reasons
separately from latest competence. Draft edits cannot reinterpret published evidence.

Historical problem applicability is the default for newly published built-in System
One routes. There is no switch or background setting. An existing frozen route
continues unchanged until its next publication. Default assessments then resolve
silently to the current default; custom instructions and every criterion remain
byte-for-byte identical. Reset to default restores current text while preserving
thresholds. Unsupported context references fail Preview with the scope and field.
Local rules and custom extensions do not use native applicability. For built-in connections, the simple probability threshold remains
0.8 and the competence floor remains 0.5 unless configured otherwise.

The applicability protocol supplies separate `prior_context`, `assessment_stage`
and `latest_user`. Background is fixed at eight whole turns before the actual
assessed stage, preserving bounded interrupted/unknown turns and their status.
The complete stage can be longer. Wire-budget trimming marks the target partial and
omits the applicability judgment without refilling history; partial scores cannot trigger
historical protection. Current input is never silently truncated; tool-result bodies
are excluded.

For an otherwise eligible historical low-score protection opportunity, applicability
at least 0.5 retains protection and a lower value suppresses it. Unavailable answers
retain protection conservatively. Attribution, version, category, score/rubric
validity, same-turn inheritance and availability relay retain their existing rules.
This check is independent of valid current difficulty. Releasing the historical
constraint returns to the difficulty-based group, which remains Primary for complex
work. The preceding actual stage may already have used Primary; no third group exists.

The connection page retains one connectivity test with the existing contract and a
current-default synthetic example. There is no route-editor test. CLI and
acceptance diagnostics can explicitly test the full nineteen-question maximum
shape through the production transport/parser. Explicit testing may consume
quota; opening, saving and publication never probe a provider.

Plan/session quality includes counts from all retained matching decisions,
independent of stage pages and score/model filters. Current events mark all four
classifier protocols; known non-applicability decisions are counted separately and
do not make mixed scopes partial. Authenticated supported older formats also count as known earlier-publication
decisions. Unproven unmarked events, missing projections and bounded scans remain
coverage gaps: counts stay visible, while exact rates are
unavailable. Content-only deletion keeps safe evidence; full fact deletion/expiry
removes it. Every scope records its authored default/custom assessment intent before sealing. Rates describe
routing behavior and do not claim accuracy or quality improvement.
Opportunities count recorded protection actions under the rule in effect at the time;
retained scopes can mix rules, so their rates do not compare one uniform policy.

The CLI keeps `decision services list/apply/test` and the existing routing preview/apply workflow.
Editor data uses `smart.judgment`, or `branch_routing.judgment` with branch `primary_candidates` and optional
complete `judgment`. Custom extensions own their decision policy and directly return category/group within the published route. Their optional prior-stage scores are observation only; HiRoute applies no thresholds or low-score protection. Custom routes hide judgment controls and keep parked source values for switching back to a built-in connection. Current custom execution projections omit judgment settings and reject the former present-judgment shape; no old custom data adapter is provided. Built-in released Plans and Operations keep their existing support. Their form provides
examples and the single current OpenAPI. No separate Secret-management UI is needed.

See [HiRoute extension API semantics](../decision-extensions/api/README.md), [optional self-hosted Jev deployment](../decision-extensions/extensions/jev-decider/README.md),
[System One mapping](../decision-extensions/api/system-one-design.md) and [code owners](code-map/decision-foundation.md).
Earlier custom-extension shapes have no compatibility path; supported built-in data and
shared Observation history retain the registered admission/recovery readers.
Custom extensions cannot select tools.
