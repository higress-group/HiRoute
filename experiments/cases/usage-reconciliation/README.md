# Usage reconciliation with built-in smart saving

Protocol v1, 2026-10-06. No results yet. Follow the
[common execution contract](../decision-routing-20261006.md).

## Question and comparison

Can an automatically routed native agent deliver a correct usage-audit utility
through a changing workflow, while using fewer expensive-model tokens than a
fixed strong model? The task mixes straightforward file/summary work with event
reconciliation and transactional updates. This is synthetic data engineering,
not financial advice or a reconstruction of an actual customer's bill.

| Code | Published plan | Native reasoning |
| --- | --- | --- |
| F | Fixed qwen3.8-flash | xhigh |
| M | Fixed qwen3.8-max | xhigh |
| S | Smart saving: qwen3.8-flash → qwen3.8-max | xhigh for both |

Two fresh native sessions per arm. Frozen order: `F-1, S-1, M-1, M-2, S-2, F-2`.
Run this case before the writing case. Freeze the same actual endpoint/source
and model profiles. The smart plan uses the built-in decision service, the product's
default simple/complex branch prompts, default competence prompt and a 0.5 floor.
Its economical branch upgrades to the main branch's Max model. Record the exact
rendered defaults in the configuration evidence before the run. Do not change
criteria or thresholds to force a favorable routing trace.

## Six-stage task

Subjects start with [TASK.md](subject/TASK.md), the stub, and the small public
example only. Release [stages.json](stages.json) one stage at a time into the same
conversation. Every arm gets exactly the same text. The requirements accumulate:

1. Summarize known and unknown input/output/cache usage from clean attempts.
2. Validate data and offer a JSONL CLI with stable output/error behavior.
3. Reconcile out-of-order revisions, duplicates and conflicting latest records.
4. Add deterministic team/model grouping and an optional team view.
5. Count logical requests across retries and account for uncached input correctly.
6. Support transactional incremental updates and document the resulting contract.

All required semantics are in the released prompts. No future requirements or
private tests are provided early. The later stages intentionally need more
reasoning, but this does not dictate the correct router decision: if Flash solves
them, that is a valid outcome. Do not equate a Max selection with improved quality.

After each stage, snapshot the complete subject directory. Grade that snapshot
with only tests for requirements released so far, outside the live subject's OS
boundary. Do not feed hidden test names, counts, pass/fail or solutions back to
the agent; the next stage is the next fixed user request, not coaching. The agent
can run its own public tests. Report stage-first-pass and final-retained capability.

## Independent verifier

`verify.py` loads `grading/test_audit.py` against a supplied project snapshot and
stage number. The grader uses hand-computed examples and metamorphic checks;
it does not import `grading/reference.py`. The reference is a positive control
only and is never copied to a subject. Run baseline-negative and reference-positive
controls before model execution. Use the repository's grading OS isolation for
untrusted generated code and immutable snapshot copies.

```sh
python3 experiments/cases/usage-reconciliation/verify.py --project SNAPSHOT --stage 6
```

The executable contract covers empty inputs, unknown versus zero, cache counts,
invalid rows, CLI stdin/files and errors, out-of-order records, ignored obsolete
conflicts, conflicts at the winning revision, grouping/order, logical retry
outcomes, cross-team rejection, filtered totals, generator input, transactional
rollback, caller mutation and chunk/order invariance on valid update prefixes.
Invalid events remain errors even if they would later be superseded.

Whole-delivery acceptance requires all final-stage tests and a nonempty README
explaining revision, unknown-usage and retry semantics. Report each failed
requirement, not only aggregate assertion percentages. Timeouts, missing outputs
and zero selected tests cannot pass. The final report includes intermediate and
final grades, actual routes/upgrades/feedback, all tokens and elapsed time.

No cash-savings claim is predeclared. Compare total tokens and high-model usage;
their meanings differ. If all-Flash passes every gate, explicitly include it in
the conclusion. A failure or absence of natural upgrades does not authorize new
probes designed to create a success story.
