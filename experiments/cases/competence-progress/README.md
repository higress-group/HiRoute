# Competence progress diagnostics

These cases exercise the current editable native competence default: supported
failure, usable partial progress and completed scoped work. They supplement the
frozen [System One paper](../../system-one-routing-paper/README.md); they do not
replace its datasets, original labels or published results.

The middle tier permits substantive useful progress before the overall outcome
is complete. A required local omission is distinct from an essential defect
that makes the delivered result unusable. Repeated uncorrected avoidable errors
and final non-delivery remain low. Completed work within the actual assessed
scope and clear acceptance without contradiction remain high. No fixed fraction
of completed requirements decides the tier.

## Inputs, expectations and provenance

[Inputs](inputs.jsonl) contain only case IDs and conversations. Separate
[labels](labels.jsonl) contain expectations, rationale and provenance. These are
diagnostic expectations under that declared policy, not independently established
task-quality measurements or population-representative samples.

| Case | Expected most likely tier | Purpose |
| --- | --- | --- |
| `DIAG_open_progress` | Middle | A substantive finding and confirmed scoped test while the wider review is open |
| `DIAG_completed_subtask` | High | Complete the explicitly requested subtask without penalizing deferred later work |
| `DIAG_bounded_wait` | No unique gold | Observe external waiting separately; no forced grade from absent error or unfinished work |
| `DIAG_repeated_error` | Low | The same avoidable error persists without recovery |
| `DIAG_false_success` | Low | Visible output and a test contradict the success claim |
| `DIAG_final_nondelivery` | Low | A promise does not supply the requested usable deliverable |
| `ISSUE344_checkpoint_7`, `ISSUE344_checkpoint_34` | Middle | Specific partial findings while the full review verdict remains pending |
| `CRITICAL_missing_signature_en`, `CRITICAL_missing_signature_zh` | Low | Four fields are present but a missing essential signature makes the result unusable |

The Issue-derived cases quote assistant notes from the public
[38-request report](https://github.com/higress-group/HiRoute/issues/344).
Their user scope and continuation are reconstructed. The original wire, full
tool arguments and tool results are unavailable, so no successful tool status,
test receipt or accepted native execution is invented. The adapter preserves
unknown execution status. Middle credit comes from the substantive findings,
not independent verification of the assistant's build-success claim. These two
checkpoints belong to one workflow, not independent trials. They are text
reconstructions, not a reproduction of its Jev 1.13 traffic or a real DSH run.

For comparison with required-but-local omissions, preserve the paper's original
`MR011_a` English/Chinese ledger examples and their middle labels. Do not relabel
a case after seeing model output. Keep original conditional and insufficient
routing labels separate from uniquely determined action accuracy. The signature
contrast is intentionally about usability, rather than a completion-percentage rule.

## Prepare requests without model calls

From the repository root, choose a new local output path:

```sh
python3 experiments/cases/competence-progress/prepare.py \
  --output artifacts/competence-progress-requests.jsonl
```

This standard-library command reads the current shared default and unchanged C4
stock questions, using the paper's lossless synthetic state adapter. It never
reads labels, credentials or provider settings, and performs no network request.
Only each row's `request` object belongs in an explicitly authorized Decisions API
call; the ID remains local bookkeeping. Output files are never overwritten.
The command does not simulate Gateway evidence admission, native execution or
the default/custom publication lifecycle.

Fresh comparisons should freeze the candidate, inputs and labels before calls;
use the same model and state in both arms; preserve raw probabilities and scalar
scores; retain invalid/missing outcomes and all transport attempts. The modal
middle grade is not a scalar forced to exactly 1 (or normalized 0.5). Report
grade accuracy separately from the eventual routing reducer, applicability and
cost. A bounded-wait case with no unique grade stays outside unique-grade accuracy.
Keep raw requests, outputs and run-specific results in ignored artifacts.

These diagnostics do not add closure-triggered grading, change thresholds or
regrade historical stages. Active and historical publications retain their
frozen rubrics; the editable default changes on the next explicit publication.
