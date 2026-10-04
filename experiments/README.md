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

## Verify published evidence without model calls

From the repository root:

```sh
python3 experiments/reproduce.py verify
python3 experiments/reproduce.py report
python3 -m unittest discover -s experiments/tests
```

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
scoring change as post hoc. Keep expensive live runs opt-in and CI entirely offline.

Use an allowlist to export public evidence. Do not commit account credentials,
subscription state, request authorization headers, private reasoning traces, or
unrelated workstation paths. No experiment reads a user's original authentication
file or acquires a second refresh-token owner.
