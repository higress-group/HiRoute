# Writing and reviewing through custom branches

Protocol v1, 2026-10-06. No results yet. Follow the
[common execution contract](../decision-routing-20261006.md).

## Question and six conditions

Does assigning drafting/revision to Qwen 3.8 Max and editorial review to GLM 5.3
produce a better final explanatory article than assigning both roles to either
model alone? How does the answer change between the middle and highest native
reasoning levels? This evaluates one specific editorial workflow, not every
genre or the models' ability to do original reporting.

| Code | Draft and revision | Reviews | Native effort |
| --- | --- | --- | --- |
| QM | qwen3.8-max | qwen3.8-max | medium |
| GM | glm-5.3 | glm-5.3 | high |
| XM | qwen3.8-max | glm-5.3 | medium / high |
| QH | qwen3.8-max | qwen3.8-max | xhigh |
| GH | glm-5.3 | glm-5.3 | max |
| XH | qwen3.8-max | glm-5.3 | xhigh / max |

Two independent replicates per condition. Frozen execution order:
`QM-1, XH-1, GM-1, QH-1, XM-1, GH-1, GH-2, XM-2, QH-2, GM-2, XH-2, QM-2`.
Balanced reversal reduces simple order effects; two samples do not establish
statistical significance. Do not change this order after reading output quality.

Create six published custom-branch plans with identical branch conditions:

- **Writing**: “The current user asks to draft, rewrite or revise an article.
  Produce or improve the article itself, including a revision based on feedback.”
- **Review**: “The current user asks to review, fact-check or evaluate an existing
  article and provide editorial feedback or a publication verdict, without
  rewriting the article.”

Writing is the decision-failure default; any fallback is reported separately.
Both branches use no upgrade candidates. This keeps the assigned model constant
for the role. Set each branch's competence prompt to judge the task it performed
(article grounded in supplied facts / accurate actionable review, respectively),
with the same 0.5 floor. Keep scores as product observations only; a low score
does not add an editorial round. Single-model controls use the same two-branch
plan/decision overhead, with both branches assigned to that model and effort.

## Human reference and reconstruction

The reference is Jordana Cepelewicz's October 5, 2026 Quanta essay
[Is AI the End of Math As We Know It?](https://www.quantamagazine.org/is-ai-the-end-of-math-as-we-know-it-20261005/),
selected on October 6 before generating candidate articles. Quanta's
[editorial policy](https://www.quantamagazine.org/ai-editorial-policy/) says it
does not use generative AI to write or edit articles. The essay combines reported
scenes, a conceptual argument and contrasting views, making it a useful editorial
benchmark. It is not an experimentally controlled human arm.

Reconstruction separates public facts from finished storytelling. The subjects
receive [TASK.md](subject/TASK.md) and [MATERIALS.md](subject/MATERIALS.md): short
original research notes from seven relevant sources, with attribution and limits.
They do not receive the original essay, its title/outline/metaphors, or grading
notes. The small reported-scene note is attributed to Quanta; the subjects may
not pretend to have attended or interviewed anyone. The brief asks for new English
prose so language does not become an additional comparison factor.

The human reporter had interviews, firsthand experience, editorial help and a
different length/brief. Compare explanatory and narrative craft qualitatively
against that reference, and use the same frozen rubric for AI-arm comparisons.
Do not interpret closeness to the reference as proof of matching original
reporting. Pretraining contamination is not measurable here; no runtime browsing
or original-text access is allowed.

## Four fixed turns

Release [prompts.json](prompts.json) one turn at a time. Same conversation, source
packet and permissions across all arms: draft → review → revision → final review.
The reviewer sees the writer's draft and materials; the reviser sees the review.
The final review cannot modify the final article. Snapshot every artifact after
its stage. No user corrections and no extra quality-driven rounds. Tool access
is limited to local file reading/writing and ordinary checking within the project.

## Independent evaluation

The parent assesses all 12 draft/final pairs before opening the blinded mapping,
using [rubric.md](rubric.md). Record dimension scores, cited text locations,
factual violations, review usefulness and revision gains. Review models' scores
or verdicts are not independent truth. Show every outcome; summarize mixed versus
each single-model arm within each effort band, and middle versus highest within
each assignment. Compare effect sizes and concrete defects, not just averages.

Structural checks (`verify.py`) validate the required artifacts and basic review
schema; they do not claim to measure prose quality. A valid final article can
still fail factual/editorial gates. The parent later compares the human original's
strengths and weaknesses, without requiring its distinctive metaphors or sequence.
