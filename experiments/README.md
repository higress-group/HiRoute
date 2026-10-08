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
