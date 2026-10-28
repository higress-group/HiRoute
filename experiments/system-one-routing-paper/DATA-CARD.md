# Dataset card

The mathematical dependency-state studies in Sections 5.8–5.9 are documented in
the [supplementary package](supplements/mathematics-history/README.md). They add
three separately reported cohorts and do not replace the labels, main tables or
latency conditions described below.

Research snapshot: 18 October 2026. Public packaging and latency update:
19 October 2026. Original decision-quality datasets, labels and responses are
preserved; the latency update does not change their experimental conditions.

Appendix C's latency table uses ten frozen engineering contexts, five per
language, shared across all three backends and both formats. Selection was at
request-size quantiles of Jev 1.14's three-question requests, before inspecting
these timings. The Jev timings were already recorded on 19 October; Qwen was
then measured with the same frozen inputs and original request parameters.
All calls used one request in flight and a fresh connection. For each call,
subtract its measured connection phase before taking the median or nearest-rank
p95. The metric still includes request transport, service processing and response
handling; these ten-call cells do not establish stable tail latency or universal
model speed rankings.

The [Jev timing records](diagnostics/connection-timing.json) retain 88 calls:
40 in the original study transport condition used by the table, 40 in a separate
transport control and eight excluded warmups. The [Qwen timing records](diagnostics/qwen-connection-timing.json)
retain 22 calls: 20 table observations and two excluded warmups. All Qwen calls
returned the expected model and valid answers, without retries; they consumed
43,972 Token Plan tokens. Numeric records retain model identity, request hashes,
call order and durations, while omitting credentials, private configuration and
response contents. Original 256-call end-to-end timing observations remain in the
saved-response archives and offline report; they are not relabeled as
setup-excluded measurements.

## Intended use

Evaluate whether a model-routing mechanism follows a declared contextual policy
at an agent checkpoint: assess current demand, retain protection for an applicable
past failure, and release protection after correction or loss of dependency.
Agent, multi-model gateway and decision-interface developers are the intended
users. The evaluation method can be applied to another explicit policy; these
labels do not define universal task difficulty or executor capability.

The generic action track evaluates standard-tier/enhanced-tier selections from any mechanism.
Frozen action identifiers remain `economy` for the standard tier and `primary` for
the enhanced tier, matching the original prompts, labels and evaluator.
The typed track additionally evaluates component answers and error propagation.
Only determinate actions enter headline accuracy. A boundary record is not an
opportunity to count either answer correct.

## Released panels

| Dataset | Records | Workflows | English / Chinese | Determinate / boundary / insufficient |
|---|---:|---:|---:|---|
| Engineering primary | 104 | 26 | 52 / 52 | 86 / 16 / 2 |
| Engineering paraphrases, separate | 24 | Paired with original workflows | 12 / 12 | 18 / 4 / 2 |
| Mathematics | 64 | 16 | 32 / 32 | 50 / 12 / 2 |
| Development V3, historical | 384 | 48 | 192 / 192 | 312 / 48 / 24 |
| Direct-interface developer probes | 8 | Diagnostic cases, not a blind panel | 4 / 4 | 6 determinate / 2 unforced |

Engineering's 104 primary records comprise 96 original records and eight records
from a fixed coverage supplement created before evaluated outputs. The supplement
is marked `construction_cohort=coverage_supplement`; the paraphrases use
`evaluation_panel=expression`. Never merge the paraphrases into the primary score.

Engineering primary has 46 standard-tier and 40 enhanced-tier determinate records. Mathematics
has 28 standard-tier and 22 enhanced-tier records. These constructed proportions do not estimate the
prevalence of decisions in production traffic.

## Record schema

Inputs and labels are separate JSONL files joined by `id`. Inputs contain
`conversation.prior_turns`, `assessment_stage`, and `current_request`. All text,
roles, order and original evidence remain available. Mathematical input rows also
retain their original family, state and language metadata.

The execution adapter turns these records into `prior_context`,
`assessment_stage`, `latest_user`, and `history_partial`. Prior background is
bounded at eight complete supplied turns; raw tool-result bodies are absent.
No semantic summary, goal extractor or model identity is invented for the stage.
Exact sent request bodies are in each compressed execution request archive.

Labels retain:

- `current_demand`: `routine`, `reasoning`, or `underdetermined` under the stated policy.
- `stage_grade`: unique 0/1/2, or null with `allowed_grades` when not unique.
- `applicability`: `applicable`, `not_applicable`, or `underdetermined`.
- `action_status`: `determinate`, `conditional`, or `insufficient`.
- `action`, `allowed_actions`, and `action_condition` without deriving gold from model probabilities.
- Family, language, pairing, panel, cohort and mode metadata, exact evidence spans,
  rationale, and the original adjudication fields where present.

The main rule uses simple probability below 0.5 or historical scalar below 1
with applicability probability at least 0.5. It retains supplied continuous
scalars. A modal grade of one does not uniquely determine whether that strict
scalar boundary triggers. The default production simple threshold in the pinned
test is 0.8; this is separate from the paper configuration.

## Construction and review

The original engineering cohort and paraphrases were authored by one AI source;
a second sequential AI source authored the coverage supplement. Mathematics used
one AI source for both languages. An independent AI review role first assigned
anonymous labels without author answers or model results, froze them, then
reconciled after revealing the mapping. Source diversity is limited. Procedural
access separation is not an operating-system isolation guarantee.

Public snapshots of author labels, first labels, author notes and the labeling
brief are under `reviews/`. They are historical states, not replacement gold.
The final pre-call files remain in `datasets/`. Mathematics also includes the
AI-reviewed labels before human adjudication and the component-label change ledger.

Two people each reviewed a 96-record V3 subset with independent AI assistance
and personal review: action/status agreement was 92/96, kappa 0.932; both endorsed
all 78 originally determinate actions. This did not review the later independent
engineering panel. Optional naturalness ratings were not completed.

Two people each reviewed all 64 mathematical records with their own AI assistance;
neither had mathematical research experience. Action/status agreement was 60/64,
kappa 0.902; both endorsed all 50 original determinate actions. Naturalness-category
agreement was 37/64. The earlier expert-review prerequisite was explicitly amended
before calls; the preserved collection snapshot predates that amendment.

Human evidence is released as anonymous aggregates and disclosed disagreements.
Raw submissions, reviewer identities, invitation links and assistant histories
remain private. Therefore these public files reproduce the reported aggregates
as records, but cannot independently regenerate every human vote. Construction
and AI-review model identities are not disclosed; model-specific bias auditing
and exact regeneration are limited.

## Provenance and disclosure transformations

The dataset records are parsed-object-identical to the frozen local sources.
JSON whitespace is normalized. The five primary question files remain byte-identical.
The public `MANIFEST.json` binds all packaged files; `provenance/source-hashes.json`
separately records logical source identifiers and original hashes. Public curated
files and original frozen files are not claimed to have identical byte hashes.

Response ledgers retain request starts and results, planned IDs, model versions,
raw answers/probabilities/scalars, LLM completion text including invalid output,
usage charges, token accounting, status, attempts and client timing. Remote
response IDs, service-provider metadata, creation timestamps inside responses and
system fingerprints are removed. Outer request timestamps remain. No distribution,
continuous score, chosen category, completion text or failure outcome is repaired.

The hosted decision access service is unnamed. Evaluated Jev/Qwen versions,
proxy-versus-Token-Plan transport conditions and execution parameters are retained.
See `execution/backends.json`, exact request archives and the manuscript's cost
appendix. API charges are historical observations, not current price guarantees;
subscription tokens are not converted to a comparable Qwen dollar charge.

## Split, dependence and limitations

Keep workflow families together when resampling or creating new splits. State
pairs, translations, paraphrases and repeats are dependent. V3 is development-era
material, not an independent blind test. Engineering independent construction
preceded its evaluated outputs, but the matched comparison reused the already-seen
dataset. The direct-expression supplement is also on seen engineering inputs.
The small direct probes are interface development, not a general accuracy estimate.

Contexts are short: independent engineering has zero or one prior turn, mathematics
none. The data do not test the eight-turn truncation boundary or long unattended
execution. The router checks visible contextual evidence under a policy; it is
not asked to independently solve hidden engineering or mathematical problems.

This is synthetic policy conformance, not measured executor utility, real traffic
accuracy or realized task savings. False protection, current-demand errors, and
correlated historical-judgment errors remain visible in the released returns.

## Labels and versions

Original labels are retained, including the disputed MR010_b action exclusion.
`sensitivities/mr010.json` is a separate post-hoc interpretation. The replay reports
both original and candidate populations without overwriting either input file.
New relabeling, prompts or samples should receive distinct versions and results.

Unless a file carries another notice, this release follows the repository's
[Apache License 2.0](../../LICENSE). Citation metadata will be finalized with the
paper authors before arXiv submission; no paper DOI or arXiv identifier is invented.
