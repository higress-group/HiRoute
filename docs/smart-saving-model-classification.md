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

Smart saving defaults to the decision-model choice; users may explicitly choose heuristic rules or
a custom extension. Economy and primary groups retain the existing candidate picker, ordering and
reasoning controls. Judgment settings start collapsed: simple probability defaults to 0.8, competence
floor to 0.5, and both degree and assessment prompts are editable.

Every new user message decides again. Regular/economy requires this call's simple probability to meet
the threshold with no applicable complete low score. Complex or fresh low competence selects primary.
Only a fresh complete score bound to the preceding actual stage, in the same category and compatible
with the published configuration and assessment criteria, can trigger protection. Missing/partial
scores do not preserve old protection; new turns do not automatically reset to regular.
Tool continuation and replay reuse the frozen decision only when the same turn is identifiable,
history continues, and the decision remains reusable. Compaction or history reconstruction that
breaks continuity or prevents reuse triggers a fresh decision. Candidate hold stays inside the newly
selected group. Availability relay never downgrades from primary or crosses task categories.

Custom branches first classify task intent, then optionally judge degree within that category. Each branch
has regular candidates and optional primary candidates, and follows plan judgment or copies the complete
settings for independent editing. No primary means no degree question, while competence remains observable.
Category conditions, degree criteria and assessment criteria are separate inputs.

Existing plan/session views show actual group/model/profile, frozen thresholds and selection reasons
separately from latest competence. Draft edits cannot reinterpret published evidence.

The CLI keeps `decision services list/apply/test` and the existing routing preview/apply workflow.
Editor data uses `smart.judgment`, or `branch_routing.judgment` with branch `primary_candidates` and optional
complete `judgment`. Custom extensions follow the supplied definition and rubric; their form provides
examples and the single current OpenAPI. No separate Secret-management UI is needed.

See [HiRoute extension API semantics](../decision-extensions/api/README.md), [optional self-hosted Jev deployment](../decision-extensions/extensions/jev-decider/README.md),
[System One mapping](../decision-extensions/api/system-one-design.md) and [code owners](code-map/decision-foundation.md).
Development v1 is replaced in place using fresh data, without migration or old-format compatibility.
Tool selection is documentation-only.
