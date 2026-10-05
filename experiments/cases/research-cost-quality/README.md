# Research delivery: cost and quality

This is a purpose-designed engineering research case, not a customer production
engagement or a held-out population benchmark. Six messaging systems each require
60 source-verification cards; a seventh component requires a Kafka/SQL consistency
memo. Sources, task bytes, model settings, and quality gates were fixed before the
three paired mixed/all-Astra repeats. Three all-Qwen repeats were registered afterward
without changing task or rubric. Evaluators knew the model identities.

## Quality at a glance

All nine deliveries are included below: three repeats per mode, with 1,080 research
cards and three critical memos per mode. The percentages describe card correctness;
they are not whole-delivery acceptance rates.

| Quality measure | HiRoute mixed | All Astra | All Qwen |
| --- | --- | --- | --- |
| Card correctness, including citation support (original rubric) | **98.61%** (1,065/1,080) | **99.72%** (1,077/1,080) | **98.80%** (1,067/1,080) |
| Content/verdict correctness (supplementary analysis) | **99.17%** (1,071/1,080) | **99.72%** (1,077/1,080) | **99.07%** (1,070/1,080) |
| Critical memos without a material error | **3/3** | **3/3** | **1/3** |

Content/verdict correctness separates citation-support-only findings from substantive
errors. This analysis was introduced after observing the results and is exploratory;
it does not replace the original rubric or excuse outstanding citation defects. The
same classification is applied to all three modes. The complete analysis and finding
classifications are in `results/2026-10-04/posthoc.json` in the
[evidence archive](https://github.com/higress-group/HiRoute/releases/tag/experiment-evidence-2026-10-04).

## Correctness in every repetition

These are the original scores, including citation support, without dropping any repeat.

| Repeat | HiRoute mixed | All Astra | All Qwen |
| --- | --- | --- | --- |
| 1 | 97.78% (352/360) | 99.72% (359/360) | 98.89% (356/360) |
| 2 | 99.17% (357/360) | 99.72% (359/360) | 98.89% (356/360) |
| 3 | 98.89% (356/360) | 99.72% (359/360) | 98.61% (355/360)* |

A wrong verdict, unsupported factual statement, missing necessary qualification, or
unsupported citation counts once per card. Every card and complete memo was read against
the frozen sources; finite JSON checks alone are insufficient. The first mixed score uses
the corrected 352 count after a citation-support omission was found and checked
symmetrically in all nine deliveries. Model answers were not repaired.

The original whole-delivery rule remains complete coverage of 360 unique cards, at least
353 correct, and an entirely correct critical memo. A high percentage alone does not
establish that every delivery met this rule. The archived `results/2026-10-04/deliveries.json`
and offline scorer preserve the original per-delivery and all-three-pairs acceptance
results; this presentation changes neither the threshold nor those outcomes.

*Qwen repeat 3 includes one prospectively registered transport recovery after an HTTP 502.
Its original execution-completeness gate remains failed. The failed request's usage/cost
is unknown, not zero. The completed delivery can be reviewed, but its all-attempt total
cannot be presented as completely known. The critical memo had not run before recovery;
no completed answer was rerun to improve a semantic score.*

## Complete-task cost

All three pairs' spending is retained. The article's savings claim uses pairs 2 and 3,
which met the original whole-delivery rule in both modes. Pair 1's spending is shown for
accounting completeness and is not counted as accepted-delivery savings.

| Pair | Mixed USD bounds | All-Astra USD bounds | Conservative reduction for a qualified pair |
| --- | --- | --- | --- |
| 1 | 0.135848–0.144745 | 2.083912–2.155990 | — |
| 2 | 0.137909–0.146806 | 1.838882–1.842160 | **92.01%** |
| 3 | 0.162341–0.177957 | 2.067170–2.132528 | **91.39%** |

These are frozen **API-equivalent USD**, not actual subscription charges. Savings use
`1 - mixed_upper / strong_lower`, rounded down for publication. All subject attempts,
structural retries, and reported Jev costs are included. External evaluator labor and
case development are outside the per-delivery comparison; the overall spend record is
retained in the archived `results/2026-10-04/evidence.json`. No faster-completion claim.

## Inspect and recalculate

Download the [frozen evidence Release](https://github.com/higress-group/HiRoute/releases/tag/experiment-evidence-2026-10-04)
with `python3 experiments/reproduce.py fetch`, then run
`python3 experiments/reproduce.py unpack`. Paths below are relative to this case:

- `results/2026-10-04/deliveries.json`: all 63 answer artifacts, findings, usage and cost.
- `results/2026-10-04/evidence.json`: complete historical method and evidence projection.
- `inputs.json`: task/source identities and exact model settings.
- [Original quality policy](quality-policy.json) and [Jev semantic criteria](jev-policy.json).
- `results/2026-10-04/posthoc.json`: post-hoc layered analysis separates citation-only
  findings from content/verdict errors uniformly. It yields content-usability counts
  3/3, 3/3, 1/3, but never replaces the original strict gate. It was introduced after
  observing outcomes and is exploratory, with threshold sensitivity retained.

Run `python3 experiments/reproduce.py verify` or `report` from the repository root.
The scorer includes all outcomes, even when an article highlights accepted pairs.
Three repeats of one selected case do not establish statistical equivalence or a
universal model ranking.

## Prepare the exact sources (no paid calls)

Official source text is retrieved from its publishers, not relicensed as HiRoute code.
URLs, timestamps, source identities, and hashes are in `inputs.json`. The original
normalization and Kafka-section selection are included in `prepare_sources.py`:

```sh
python3 experiments/reproduce.py fetch
python3 experiments/reproduce.py unpack
python3 experiments/cases/research-cost-quality/prepare_sources.py --output /tmp/hiroute-research-inputs
```

Every subject input must match the registered hash. If a website changes, the tool
stops; do not silently adopt new text or reuse old source-line judgments. When you
have the original raw snapshots, supply a directory containing `<source-id>.raw`:

```sh
python3 experiments/cases/research-cost-quality/prepare_sources.py \
  --raw-directory /path/to/registered-raw-sources --output /tmp/hiroute-research-inputs
```

Historical upstream availability is not guaranteed. A changed-source experiment
needs a new case registration and renewed semantic review. This limitation does not
affect offline replay of the published result artifacts.

## Execute fresh deliveries (paid model calls)

Use your own normally configured HiRoute sources. Configure three plans: fixed Astra,
fixed Qwen, and smart-saving with Qwen economy/Astra primary. Export a separate HiRoute
client TOML profile for each plan. Configure `gpt-6-astra` reasoning `medium`,
`qwen3.8-flash` reasoning `xhigh`, 131072 context tokens, and 65536 maximum output tokens.
For mixed, use the supplied Jev criteria, simple threshold 0.8 and competence floor 0.5.
The current task/claims/source metadata are visible to Jev; the corpus is in the preceding
user message, available to the subject. Do not add product-name or answer-key overrides.

```sh
python3 experiments/cases/research-cost-quality/run.py \
  --sources /tmp/hiroute-research-inputs --profile /path/to/mixed.hiroute.toml \
  --group mixed --output /tmp/hiroute-fresh-p01-mixed
```

The supplied profile selects the actual plan; `--group` is a declared experimental
label, not a routing override or proof. Repeat with the corresponding profiles in the
registered order: mixed/Astra, Astra/mixed, mixed/Astra, then three Qwen arms. Register
this fresh schedule and policy before starting. The runner performs at most two
structural attempts and stops on transport/incomplete-usage failures. It does not retry
semantic mistakes. It preserves output, terminal usage, input hashes, and failure status
without writing authorization headers or private reasoning streams.

After each run, export HiRoute immutable receipts and Jev usage for its recorded
`reproduction-*` session IDs. Confirm the actual models and all attempt usage, reconcile
against terminal usage, and count routing costs. Fresh outputs remain **ungraded and
cost-incomplete** until that reconciliation and a full source-grounded review finish.
Missing cost stays unknown. This portable execution client does not claim to recreate
the original workstation isolation or automatically repeat the analyst's judgment.
