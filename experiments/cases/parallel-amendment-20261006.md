# Parallel execution amendment — 2026-10-06

The user requested two separate `gpt-6.1-sol / max` executors after preparation
and initial preflight. This amendment supersedes only the cross-experiment
serialization in the [common protocol](decision-routing-20261006.md) and the
instruction to finish [usage reconciliation](usage-reconciliation/README.md)
before [writer/reviewer](writer-reviewer/README.md). Preserve the original
protocol commit and file hashes; record this amendment's commit separately.

One executor owns usage reconciliation (`F-1, S-1, M-1, M-2, S-2, F-2`). The
existing executor owns all twelve writing pipelines in their original order.
Each executor runs at most one native turn at a time, with no parallel arms
within an experiment. Current turns finish and retain their original evidence
before ownership changes. Do not restart completed cells to hide the change.

Task materials, prompts, published plan revisions, model profiles, decision
revision, grading, failure policy and per-turn/replicate limits remain frozen.
Reuse completed preflight evidence at its actual revision. The shared maximum
of sixteen preflight turns and the original 17:53 CST session deadline remain
in force; splitting executors does not multiply either allowance. Record the
ownership handoff, already executed cells, remaining allowance and actual time
when concurrent execution begins.

Native projects, HOME directories and OS boundaries remain disjoint. Partition
controller output and summary ownership. Attribute every receipt to the exact
published plan, alias, revision and native conversation/prompt evidence; a
global database row added during a turn may belong to the other experiment.
Exclude other-plan rows before extracting content, and fail on mismatches
within the expected plan. Lock each execution lane and any shared preflight
counter to prevent duplicate turns or an allowance race. Neither executor may
modify the other's subjects, runtime configuration or evidence. Shared product
configuration and repairs remain coordinated by the parent.

The user assigned CU configuration and experiment preparation to the parent.
The parent owns the ordinary Desktop, verifies the existing published plans
and performs the session-observation UI checks. The two executors perform native
experiments and read-only evidence collection without operating that shared UI
or changing product configuration. No executor polls for the other's state.

Both experiments share the HiRoute daemon and Bailian Token Plan. Save actual
start/end timestamps and overlap intervals, including retries and failures.
Report concurrency, throttling and queueing when observed. Token counts and
delivery quality remain the primary descriptive outcomes. Elapsed times are
observations under this shared load, not an isolated speed comparison or a
causal model-effort performance claim. Keep any earlier serial cells identified
separately. Do not silently discard them or attribute a latency difference to
routing alone.

Each executor proactively reports completion or a reproducible product issue.
The parent waits without polling, fixes product defects, then hands affected
work back. A shared runtime restart requires both executors to finish or safely
stop their current turns and acknowledge a pause before replacement. All paid
and partial work remains in the report. Writing outputs still go first to the
parent as a blind bundle, with the model/effort mapping held separately.

The initial `F-1` stage completed before this amendment, but its grader failed
before assertions because copying an immutable snapshot preserved read-only
directory permissions. Recovery grades a byte-identical, writable private copy
using the unchanged frozen grader and OS isolation. Preserve the original
snapshot and failed grading attempt. Restore continuation metadata from the
recorded native events; do not replay the model turn. Its lost process exit code
and precise duration remain unknown. Reference-positive and stub-negative
controls verify the corrected preparation separately from this subject result.
