# Research delivery: cost and quality

This is a purpose-designed engineering research case, not a customer production
engagement or a held-out population benchmark. Six messaging systems each require
60 source-verification cards; a seventh component requires a Kafka/SQL consistency
memo. Sources, task bytes, model settings, and quality gates were fixed before the
three paired mixed/all-Astra repeats. Three all-Qwen repeats were registered afterward
without changing task or rubric. Evaluators knew the model identities.

## Complete original results

| Repeat | Mixed cards | Astra cards | Qwen cards | Critical memo: mixed / Astra / Qwen | Strict whole delivery: mixed / Astra / Qwen |
| --- | --- | --- | --- | --- | --- |
| 1 | 352/360 | 359/360 | 356/360 | pass / pass / fail | fail / pass / fail |
| 2 | 357/360 | 359/360 | 356/360 | pass / pass / fail | pass / pass / fail |
| 3 | 356/360 | 359/360 | 355/360 | pass / pass / pass | pass / pass / pass* |

Whole delivery requires all 360 unique cards, at least 353 correct, and an entirely
correct critical memo. A wrong verdict, unsupported factual statement, missing necessary
qualification, or unsupported citation counts once per card. Every card and complete memo
was read against the frozen sources; finite JSON checks alone are insufficient.

**Original strict whole-delivery counts: mixed 2/3, Astra 3/3, Qwen 1/3.**
The first mixed result was corrected from 353 to 352 after a citation-support omission
was found and checked symmetrically in all nine deliveries. The original goal requiring
all three paired deliveries to pass was not met. Model answers were not repaired.

*Qwen repeat 3 includes one prospectively registered transport recovery after an HTTP 502.
Its original execution-completeness gate remains failed. The failed request's usage/cost
is unknown, not zero. The completed delivery can be reviewed, but its all-attempt total
cannot be presented as completely known. The critical memo had not run before recovery;
no completed answer was rerun to improve a semantic score.*

| Pair | Mixed USD bounds | All-Astra USD bounds | Conservative reduction | Original pair accepted |
| --- | --- | --- | --- | --- |
| 1 | 0.135848–0.144745 | 2.083912–2.155990 | 93.05% | No; attempt expense, not accepted-delivery savings |
| 2 | 0.137909–0.146806 | 1.838882–1.842160 | 92.01% | Yes |
| 3 | 0.162341–0.177957 | 2.067170–2.132528 | 91.39% | Yes |

These are frozen **API-equivalent USD**, not actual subscription charges. Savings use
`1 - mixed_upper / strong_lower`, rounded down for publication. All subject attempts,
structural retries, and reported Jev costs are included. External evaluator labor and
case development are outside the per-delivery comparison; the overall spend record is
retained in [evidence.json](results/2026-10-04/evidence.json). No faster-completion claim.

## Inspect and recalculate

- [All 63 answer artifacts, findings, usage and cost](results/2026-10-04/deliveries.json).
- [Complete historical method and evidence projection](results/2026-10-04/evidence.json).
- [Task/source identities and exact model settings](inputs.json).
- [Original quality policy](quality-policy.json) and [Jev semantic criteria](jev-policy.json).
- [Post-hoc layered analysis](results/2026-10-04/posthoc.json): separates citation-only
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
