# Validation timing and missed-regression feedback

This tooling produces diagnostics for the current development agent. It does not certify product
acceptance, waive final checks, or start a background agent.

## Select iteration checks separately

```sh
python3 scripts/test-plan.py --base origin/main --since CHECKPOINT_SHA
```

The top-level plan remains the final obligation against the PR/base merge base. The
`iteration` member selects changes since the explicit checkpoint, including staged,
unstaged, untracked and both sides of renamed paths. The checkpoint must resolve to a
commit that is an ancestor of HEAD. It is a diagnostic comparison, **not** proof of prior
validation. An empty iteration does not discharge the final plan. `--since` cannot be
used with hosted CI execution/output; CI always uses the final plan.

The first bounded mappings cover Gateway runtime/protocol tests (including their
coverage-manifest check and the protocol consumer of runtime changes), routing-plan test fixtures, and the Responses request helper
(including its oracle target and indirect smoke consumer). The oracle's ignored external-SUT
case remains ignored; the current-checkout smoke supplies the production-path check.
Shared harness/process helpers, dependencies,
unclassified production code and unknown paths retain conservative expansion. Mixed
changes take the union; already covered targets and smoke are not executed twice.
Bounded target plans retain one package Clippy gate (`--all-targets --all-features`),
unless that package is already checked by the selected group/full plan.
Known process targets use one test-harness thread; pure coverage/fixture targets keep
default parallelism. Package selection can change resolved dependency features even
when both commands say `--all-features`; preserve the failing build context when needed.
The focused `p0_gateway_runtime` target and its Clippy gate explicitly enable
`serde_json/preserve_order`, matching the known feature-sensitive fixture context from
#94's full backend run. This covers that specific combination, not equality of the
entire workspace feature graph. Keep resolved-feature evidence when diagnosing a new
combination; do not assume all future failures share this cause.

For branch integration, add `--integration` to expose known boundary checks before the
final plan, including when dependencies select full workspace. Add `--collect-failures`
only for explicit bounded diagnosis. With `--since`, this view follows the iteration diff.
See [integration preflight](test-selection.md#integration-preflight); the existing reports
and handoff remain the evidence record, with no automatic skip or reuse of final checks.

## Link runs belonging to this feature

Use a distinct `--plan` ID for this feature/convergence session and retain the returned
run IDs. These options pass through the existing HOME routing:

```sh
python3 scripts/validation.py backend run --ref refs/heads/codex/my-change \
  --plan my-change-round1 --phase focused -- \
  cargo test --locked -p hiroute-e2e --all-features \
  --test p0_gateway_runtime -- --test-threads=1

python3 scripts/validation.py backend run --ref refs/heads/codex/my-change \
  --plan my-change-round1 --phase final --related-run FOCUSED_RUN_ID -- \
  cargo test --locked --workspace --exclude hiroute-desktop --all-features

python3 scripts/validation.py backend status FINAL_RUN_ID
```

Repeat `--related-run` to associate more records, including a previous final run if a
rerun is necessary. Referenced runs must have a saved report, have executed, and belong
to the same plan in that execution host's run store. There is no global latest-success
lookup, automatic cross-machine import, or automatic cross-revision evidence reuse.
Without explicit associations the report says no comparison evidence was supplied.
Different SHAs remain labeled with their original SHA and require intervening-change
analysis; they cannot prove a same-candidate escape.

Default plans and local/SSH runners preserve Cargo's fail-fast policy: the first failed
test executable ends the Cargo test command after that executable finishes. Structured
feedback is currently emitted at command completion, so silently continuing through
later long targets would delay the agent's consumable result. Runners preserve an
explicit `--no-fail-fast` when the caller chooses batch diagnosis within a declared scope;
that report still arrives at command completion. This is not live failure feedback.
Phase and scope do not change this default. See [failure policy and completeness](test-selection.md#failure-policy-and-completeness)
for collection limits, including incomplete execution and multi-command plans.

The SSH runner automatically reserves its existing exclusive slots for workspace tests. This does not
serialize threads created inside every workspace test; selected process targets retain
their explicit `--test-threads=1`. This stage does not replace workspace scheduling.

After a failed run, record observed failures and unexecuted or unknown coverage; an early
stop cannot establish a complete failure set. Choose explicit collection only when the
diagnosis needs it, batch related fixes and run affected checks. Remaining checks retain
their final obligations; final success requires passing evidence for every selected check
and explicit gate. Do not automatically restart full to fill the failure report.

CI's `ci-run.py` accepts the same three feedback options. Its related IDs are sibling
run directory names under `artifacts/ci/`; the generated CI plan marks checks as final.
CI uploads those reports with its existing artifacts. Backend success does not replace
Mac Desktop/Pilot or real-account scenario evidence.
When a PR selects only tooling/frontend/docs, the selection job executes the selected
Python checks and uploads their reports; a skipped backend job does not skip tool tests.

## Read the report

Each completed/failed runner invocation saves `validation-report.json` and
`validation-report.md` beside `result.json` and `command.log`. The result/status includes
`validation_report`: report paths, slowest three observed modules, findings and follow-up.
Reports survive checkout cleanup. A report error stays visible without overwriting the
original process/scenario verdict. Reports are snapshots at Cargo completion; later
Pilot scenario verdicts remain authoritative in the runner's current result/session.

The module unit is a Cargo target/test binary, not an individual Rust function. The
parser uses a target's terminal summary, checks its declared test count, and does not
add nested child summaries to the parent's time/counts. Missing/ambiguous identities,
unfinished parents, missing timing and truncated output remain incomplete or unknown.
Different binaries with the same integration-target name keep separate timing rows;
ambiguous names and nested child output cannot establish cross-run case coverage. Binary hashes may change
with build features, so only unique logical target identities are compared across runs.
Unattributed summaries cannot establish module coverage. A successful Cargo process is
not a Product scenario green verdict, and zero/ignored cases are not coverage evidence.

Ordinary local/SSH run results and compact SSH waits use the same complete Cargo-target
counts as the module report. Nested child summaries stay in the command log for
diagnosis, without adding independent cases. An unfinished parent stays incomplete
even when a child's selected count matches its own; a later successful target cannot
erase an earlier failure. Process exit is retained separately from these test counts.

Outer compile events, nested/later build events, execution times, queue/setup/checkout
timings and total time use separate fields. They must not be blindly added together.
Unknown time is not zero. Smoke's existing `build_ms`, `execution_ms` and
`unclassified_ms` are copied from explicitly logged reports or this command's
`target/smoke/*/report.json`, checked against the checkout, SHA and time window; only
timing/verdict fields are copied. A historical report without stage timestamps cannot
magically recover queue or exact compile overhead.

For `workspace-dag`, the workbench records `queue_seconds`, `execution_seconds`
and `end_to_end_seconds` with one host's monotonic clock. The execution interval
starts at the capacity lease and ends after the report is published; phase durations
overlap. `performance_goal_met` reports the 300-second target independently of the
test and product verdict. A passing same-SHA repeat does not prove a different-SHA
incremental build meets that target.

For retained runs, regenerate a report without running Cargo:

```sh
python3 scripts/validation-report.py /path/to/runs/RUN_ID
```

This writes only the two derived report files and prints their summary. It does not
rewrite the original run result or its verdict. For historical runs without plan/phase
metadata it provides timing diagnostics, not linked focused/full proof.

## Required agent feedback loop

After a final failure, the active development agent reads `findings`, related original
results and logs before retrying. `focused_full_gap` is a diagnostic prompt, not an
automatic claim that a unit test is missing or that contention caused the failure.

| Cause | Repair during the current feature iteration |
| --- | --- |
| Missing assertion | Add a regression at the lowest layer that can really reproduce the old failure. |
| Existing test was not selected | Fix the owner/consumer mapping and its selection regression. |
| Only a component combination fails | Add a bounded boundary/integration check to the relevant focused plan. |
| Features, platform or build context differ | Repair hidden fixture assumptions or add the needed build combination. Matching requested flags do not prove equal resolved features. |
| Process/parallelism/environment issue | Repair deterministic readiness, isolation or scheduling; preserve the assertion. |
| Insufficient evidence | Continue focused diagnosis; do not fabricate a regression or call the case covered. |

Demonstrate that the regression catches the old failure and passes with the fix.
Do not duplicate an already adequate test. Batch related repairs, rerun affected checks
and explicit gates, and retain final obligations; a report must not launch another full
run automatically. Repeated identical final commands in explicitly linked same-SHA
records produce a diagnostic reminder to explain the repeat.

At feature handoff, assess the report's Top 3 once: **optimize**, **defer**, or
**insufficient evidence**, with the observed cost, smallest change, expected benefit and
uncertainty, and unique assertions to preserve. One run can identify a hotspot but
cannot establish a performance regression. Performance feedback is a recommendation;
it does not automatically refactor a frozen, validated candidate. Future prevention is
stored in committed tests/mappings/fixtures, not the agent's conversation memory.
