# HiRoute experiments

Public, self-contained case studies for model routing. Each case keeps its task,
configuration, evaluation rules, results, and provenance together. Python 3.11+ is
required for the standard-library tools. Nothing imports or links to repository-private
material. English is used for operator instructions; the research task's original
Chinese prompts are deliberately preserved for reproducibility.

| Case | Question | Entry |
| --- | --- | --- |
| [Research cost and quality](cases/research-cost-quality/README.md) | Can economical bulk research plus strong critical reasoning meet a delivery standard at lower cost? | `python3 experiments/reproduce.py report` |
| [Unattended engineering](cases/unattended-engineering/README.md) | Can a native coding agent automatically upgrade at a context boundary and complete independent acceptance? | Prepare a pinned HTTPX task, then verify its implementation |
| [Usage reconciliation](cases/usage-reconciliation/README.md) | Does built-in smart saving complete a changing data-audit workflow with less model usage? | [Completed local results and limits](results/decision-routing-20261006/README.md); independent staged verifier |
| [Writer and reviewer](cases/writer-reviewer/README.md) | Does routing drafting to Qwen and review to GLM improve an article over either model alone? | [Completed blinded assessment](results/decision-routing-20261006/README.md); all six conditions retained |
| [Competence progress](cases/competence-progress/README.md) | Does the editable competence default distinguish useful partial progress, essential failure and completed scoped work? | Offline request preparation; separate diagnostic labels and report-derived reconstructions |

The October 6 result report includes all registered outcomes, partial runs,
actual routing deviations and later product repairs. Its compact result record
is in this repository; the article/review package and raw observation evidence
remain local and have not been published as a Release asset. The following
download commands still apply only to the earlier October 4 research experiment.

## Verify published evidence without model calls

The 69 bulky JSON records are distributed as a [Release attachment](https://github.com/higress-group/HiRoute/releases/tag/experiment-evidence-2026-10-04),
instead of source files. The repository keeps readable summaries, tasks, policies,
test fixtures, the model's patch and the original 106-file checksum manifest.

Download the 394 KB archive once, then verify offline from the repository root:

```sh
python3 experiments/reproduce.py fetch
python3 experiments/reproduce.py verify
python3 experiments/reproduce.py report
python3 -m unittest discover -s experiments/tests
```

`fetch` checks the pinned archive SHA-256 in [evidence-manifest.json](evidence-manifest.json).
`verify` and `report` restore the original JSON paths from the local archive and check
the original file hashes. Downloaded and restored files are ignored by Git; a changed
local evidence file is rejected, never overwritten. For inspection without scoring,
use `python3 experiments/reproduce.py unpack`. The archive can also be downloaded
manually to `experiments/evidence-2026-10-04.zip` before running the offline commands.

Verification checks original artifact hashes, all 3,240 cards and nine memos,
recorded finding counts, critical-memo findings, per-attempt accounting, Jev cost,
whole-delivery gates, and conservative paired savings. It replays the published
unblinded review judgments; it does **not** independently establish semantic truth.
The case overview reports correctness and critical-decision quality; those metrics
are distinct from whole-delivery acceptance. All original acceptance outcomes remain
in the evidence and offline scorer. Fresh live runs are separate from these historical
results and cannot overwrite them.

## Read the article

[English](../news/2026-10-04-astra-qwen.en.md) ·
[简体中文](../news/2026-10-04-astra-qwen.zh-CN.md)

The accepted-pair chart can be rebuilt with Python and `matplotlib==3.11.2`:

```sh
python3 experiments/render_figures.py
```

It writes the canonical SVGs in `news/assets/`. Website preparation copies these
assets; the Markdown and news index remain the source of truth.

## Recalculate cache-cost sensitivity

For the cache-cost article's separate post-hoc sensitivity analysis, run:

```sh
python3 experiments/reproduce.py fetch
python3 experiments/cache_cost_analysis.py
```

The calculator verifies the frozen evidence before using its prices, fixed FX,
all-attempt usage and recorded delivery bounds. It preserves all three paired
acceptance outcomes and computes two idealized all-input-cached references with
each run's observed outputs held fixed: mixed routing, to bound how much any cache
loss can change its savings, and all-Astra, for the price-cut comparison. Prefix-loss examples are conditional
calculations, not measured cache-hit rates. This analysis adds no model calls and
changes neither the historical evidence nor its acceptance rules. Read the article
in [English](../news/2026-10-14-routing-cache-cost.en.md) or
[简体中文](../news/2026-10-14-routing-cache-cost.zh-CN.md).

Observed per-turn request counts are reported separately from the conditional
cache calculations. Writing counts use the tracked October 6 structured results
(including interrupted turns separately); HTTPX counts use the frozen October 4
evidence, distinguishing the whole native turn from its final stable Astra segment.
The calculator includes the writing source hash and keeps decision reuse distinct
from cache hits.

It also reports a hypothetical uniform Astra price-cut sensitivity: the whole-task
break-even price for each pair, with Qwen prices, usage and outcomes held as
recorded. The article's
cache-cost figures are rebuilt with the other figures by
`python3 experiments/render_figures.py`.

## Research manuscripts

[System One routing for long-running agents](system-one-routing-paper/README.md)
contains a community-review manuscript, bilingual datasets, exact study questions,
curated model responses and an offline evaluator for policy-conditioned routing
and historical protection. Follow its [reproduction guide](system-one-routing-paper/REPRODUCE.md);
the earlier case-study commands above do not cover that manuscript.

## Add an experiment

Create `cases/<name>/` with a README, task inputs, parameter/provenance record,
and independent verification entry. Record the protocol before paid execution:
comparison groups, order, acceptance gates, retry/stop rules, pricing and uncertainty.
Keep subject execution separate from grading. Retain failures and unknown cost;
unknown does not mean zero. Use fresh output directories, and label any subsequent
scoring change as post hoc. Keep bulky frozen records in a versioned Release asset
with pinned checksums, and retain readable results and reproduction code here.
Keep paid live runs opt-in; CI only downloads the pinned evidence and verifies offline.

Use an allowlist to export public evidence. Do not commit account credentials,
subscription state, request authorization headers, private reasoning traces, or
unrelated workstation paths. No experiment reads a user's original authentication
file or acquires a second refresh-token owner.
