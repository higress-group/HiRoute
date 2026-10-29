# Dataset card: mathematics history supplement

## Scope and policy

The unit is a bilingual synthetic agent checkpoint, not a completed mathematics
benchmark problem. The task is adherence to a declared routing policy:
choose `primary` (enhanced tier) when the simple-work probability `s < 0.5`, or
when the historical scalar `h < 1` and applicability probability `a >= 0.5`;
otherwise choose `economy` (standard tier). Both model versions and both question
sets use the same thresholds. A three-question request evaluates current demand,
observed historical performance and the continuing relevance of a historical
defect. The direct request embeds the same definitions and formula.

Labels use visible evidence: copied content, explicit requirements, supplied
proof steps, execution/readback reports and whether pending work requires the
affected artifact or failed constraint. They do not require solving an unsupplied
mathematical problem or predicting which executor would succeed. A request for
new, unsupplied reasoning is demanding even if a standard method is named.
A repair promise or a correct value repeated in chat is not completed artifact
repair. Shared mathematical topic alone does not establish a dependency.

The assessed stage is the full supplied stage under its then-applicable
requirements. The performance grade of a failed stage does not improve
retrospectively because later evidence repairs its consequences. Later evidence
can instead remove applicability. Earlier prior messages do not replace the
explicit assessment boundary, and newly introduced requirements are not past
execution failures.

## Construction and independence

| Panel | Construction | Representation and dependence | Review |
|---|---|---|---|
| Confirmation: 136 records, 68 per language | Separate AI author received a neutral policy/schema without evaluated prompts, earlier records or outputs. The two evaluated definition sets were fixed before inspecting these cases/labels. | 16 workflow families; 32 each healthy, unresolved, repaired and independent records, plus eight withdrawn-requirement records. Post-stage events appear in a recap within the current user message. Two repeats per cell. | Author self-audit, including bilingual and evidence checks; not a separate blinded second reviewer. |
| Targeted: 48 records, 24 per language | Study author knew earlier results and intentionally constructed dependency challenges. | Six families × four dependency states × two languages: unresolved, promise-only repair, completed user repair with full replacement content, and withdrawn/independently replaced target. | Independent anonymous AI labels before author-label access, reconciliation and evaluated calls. |
| Chronological: 44 records, 22 per language | Previously frozen, unevaluated material from a separate AI author. | 22 bilingual workflow pairs with distinct prior messages, last assessed execution and current user request. | The same independent anonymous AI review as the targeted panel. |

The chronological source's role-tagged prior messages were grouped into turns;
its assessment messages and current request were normalized without dropping
text, roles or ordering. The public request plan retains the resulting exact
native context. No context is truncated in any of these panels. Request bodies
contain model, context and questions only; labels, family/state names and other
scoring metadata remain outside the body. Identical contexts are checked across
both forms, definition sets, versions and repeats.

Review independence is procedural access separation, not an operating-system
isolation guarantee or independent human expertise. The targeted author is not
an independent authoring source. The 136-record source has author self-review,
not the 92-record blinded-review procedure. These distinctions should survive
any later reuse of the data.

## Labels and uncertainty

`datasets/*-labels.jsonl` are the scoring labels. `id`, `family_id`, `state`,
`language`, and `pair_id` join conversations, labels and request metadata. Labels
retain current demand, ordinal stage grade, applicability, action status,
action, rationale, and any conditions or evidence locations. Null means that a
unique label is not established. Original author and independent initial
judgments remain in `reviews/` and are not used as replacement gold.

All 136 confirmation and 48 targeted records are determinate. Chronological
coverage is **32/44 = 72.73%**: 20 standard-tier targets and 12 new-reasoning
targets. The 20 standard targets comprise 12 actual post-problem releases and
eight healthy bounded tasks. There are no determinate grade-zero protection
records in the chronological panel after review.

Eight chronological records are conditional: their local defects support rubric
grade one and applicability, but an ordinal/modal grade of one does not uniquely
locate the continuous scalar relative to the **known strict threshold `h < 1`**.
Four have insufficient visible evidence. These 12 records receive no exact-action
accuracy credit, including when the model's answer seems conservative. Their
predicted-action distributions are reported separately. The original author's
`assessment_grade` field in this cohort was a failure-evidence code
(0: no failure, 1: uncertain, 2: visible failure), not the performance rubric.
The code was reconciled semantically, never converted as an ordinal score.

Component agreement has its own known-label denominator. In the chronological
panel, demand is evaluable on 44 records, grade and applicability on 40 each;
this differs from the 32-record action denominator. Component correctness does
not imply correct routing, nor does correct routing establish all components.

## Estimation and reproducibility

Each planned call remains in its appropriate denominator. A missing, malformed,
wrong-version or unsuccessful response is an action error for a determinate
record; it is never silently removed. All published calls are valid and each has
reported usage. The replay preserves the direct native choice, including ties,
and uses normalized probability distributions with the pinned parser's tolerance.
Three-question routing uses the **returned historical scalar**, not a modal grade
or a scalar recomputed from displayed probabilities. Modal grades are used only
for component agreement. A separate sensitivity recomputes the ordinal
expectation and reports scalar differences, threshold crossings and action changes.

Principal accuracy averages the within-family accuracy equally over families.
Paired differences are three-question minus matched direct, computed within each
family before averaging. Bootstrap draws resample complete families with
replacement, retaining translations, states and repeats together: 10,000 draws,
with sorted draw indices 249 and 9749 as the interval endpoints. Confirmation uses
seed 20261029 and original label-file family order. Both extension panels use
seed 2026102813 and sorted family IDs. These conventions preserve the source
analyses exactly. Chronological action intervals use its 16 determinate families,
not all 22 families. The targeted six-family intervals are descriptive and cannot
establish population generalization. A zero-width interval at ceiling is not
proof of zero uncertainty outside these families.

Protection means bounded work with a determinate enhanced-tier label;
`regular_targets` means all determinate standard-tier targets, including healthy
work. `post_problem_release` narrows that latter group to repaired,
withdrawn/superseded or independent-input states. `healthy_regular` is the remaining
standard-tier group. `new_reasoning` is the current-demand enhanced branch.
The exact state lists are in `replay.py`. This prevents interpreting all ordinary
standard-tier cases as evidence of recovery from a past defect.

The replay recomputes main counts, record/family accuracy, paired family intervals,
components, protection/release and reasoning groups, unforced action distributions,
scalar sensitivity, prompt-character counts, and token/cost/validity accounting
from published conversations, labels, exact requests and saved answer objects.
It uses the existing public parser; it does not call a model or import private
modules. The 1,024 checked references were extracted from the existing source
aggregates, not created from this new replay. The supplemental scalar calculation
uses the native parser's `stage_grade` probability key; this diagnostic does not
change parsing, thresholds, labels or main action scores.

## Limits and disclosure

These are constructed policy tests, not samples of production workload frequency,
executor capability, downstream mathematical correctness or realized savings.
Dependencies and repairs are explicit and sometimes include complete replacement
content. The definition comparison does not isolate length from wording or
interface effects. Two versions of one model family do not provide independent
architectural replications. Translations, variants and repeats are not independent
new workflows. No post-response prompt, threshold or label tuning is represented
in these evaluated panels. No expert-human validation is claimed.

The three panels remain separate from each other and from the original main
mathematics sample. In particular, the targeted improvements must not be pooled
with chronological ceiling performance to claim a larger general benefit.
Existing and concise protocols have different direct baselines; their respective
differences cannot establish that the concise rewrite itself improved the
three-question mechanism.

Semantic requests and complete answer objects retain exact evaluated Jev versions,
probabilities, scores/choices, confidence, available legends, token usage and
reported USD costs. Access namespaces, remote identifiers, timestamps, host
configuration and execution logs are excluded. Construction and review sources
are described by AI roles, with their identities undisclosed. Consequently the
package reproduces numerical results but not hosted-service timing, billing
conditions, model generation or the full annotation process. Source hashes are
provenance claims; the public manifest verifies the transformed files available
here, not the inaccessible source bytes.
