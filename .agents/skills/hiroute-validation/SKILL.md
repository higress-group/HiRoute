---
name: hiroute-validation
description: Select, execute and diagnose HiRoute validation, retire owned inactive build artifacts, and report revision-bound results. Use for validation work in ordinary changes or integration; not research-only tasks or automatic branch integration.
---

# HiRoute validation

Use the current task, checkout and authorized checks in Codex or Claude. This skill
uses repository scripts and the caller's HOME configuration; it adds no delegation,
publication, merge or phase authority. Branch convergence belongs to
[HiRoute integration](../hiroute-integrate/SKILL.md).

## Read for the current operation

Read the relevant guide or section as needed. A routine edit does not require
loading every execution, diagnosis and retention guide.

| Operation | Read next |
| --- | --- |
| Select checks and consumers | [Test selection](../../../docs/test-selection.md), using the [test map](../../../docs/code-map/testing.md) to find the owning assertion. |
| Execute backend, Desktop or Pilot checks | [Validation routing](../../../docs/validation-routing.md#commands); follow only the selected local/SSH runner or [Desktop Pilot](../../../docs/desktop-tauri-pilot.md) guide. Hosted CI has its [own entry](../../../docs/github-actions-validation.md). |
| Iterate on Windows behavior | [Cross-compilation and development-machine execution](../../../docs/validation-routing.md#windows-iteration-and-release-validation), including `validation.py windows` and HOME machine configuration. Reserve the hosted matrix for release candidates or an explicitly needed manual diagnosis. |
| Diagnose a failure | The run's result, report and relevant logs, then [failure repair](../../../docs/validation-feedback.md#required-agent-feedback-loop). For product-path failures, also use [diagnostic troubleshooting](../../../docs/diagnostic-troubleshooting.md#diagnose-failures-before-rerunning). |
| Retire build outputs | [Owner cleanup](../../../docs/local-rust-validation.md#owner-cleanup-during-buildfixretest) or the selected remote runner's cleanup flow. |
| Prepare evidence and timing feedback | [Report interpretation](../../../docs/validation-feedback.md#read-the-report) and its required feedback loop. |

## Select and execute

- Record the actual comparison ref and candidate. Run
  `python3 scripts/test-plan.py --base BASE_REF`, inspect its changed paths, consumers and explicit native/product
  gates, and record justified scope adjustments. Docs/tooling use their own checks;
  product smoke covers affected business paths. Expand for cross-boundary contracts,
  dependencies or unresolved impact, rather than location or test duration alone.
- For resource/performance changes, apply the [serviceability rule](../../../AGENTS.md).
  Select real production-entry cases for the affected pressure, long/concurrent request,
  fallback, cancellation and cleanup paths. Verify valid requests beyond internal memory
  targets complete with exact content and affected history/attribution/recovery intact.
  Compare exact-revision before/after resident memory, CPU through asynchronous drain,
  and complete local foreground time including fallback; separate external waiting.
  Preserve configured request limits, timeouts and retries. Report cold/warm workloads
  and sample limits; an admission-accounting bound is not a whole-process RSS bound.
- Run selected Python/Node checks directly. Use `scripts/validation.py` and the
  caller's configured host for Cargo and Desktop/Pilot execution. Final evidence
  uses an exact committed candidate locally and a pushed candidate remotely.
  Before an authorized validation push, apply [public PR checks](../hiroute-public-pr/SKILL.md).
- Backend checks exclude Desktop; required Desktop compilation and native scenarios
  use the configured capable environment. Report Windows and other platform coverage
  from actual execution. Missing capabilities remain evidence gaps; do not install
  Desktop dependencies as an incidental backend repair. For affected Rust files,
  preserve the [existing size gate](../../../scripts/check-rust-file-size.sh).
- Give managed runs a feature-specific `--plan`, a `focused` or `final` `--phase`, and explicit
  `--related-run` associations. Keep execution and report interpretation separate.
  Development/test defaults and isolated Pilot logging levels are documented in
  the diagnostic guide; verify both roles' applied levels when diagnosing product failures.
- For queued SSH checks, use
  `python3 scripts/validation.py backend wait RUN_ID --timeout 50` instead of frequent status snapshots. Resume the same live shell
  session when available; on timeout, give a progress update and wait again.
  On failure, inspect its report and relevant logs. A wait summary is not a scenario verdict.
- For Windows development execution, use `validation.py windows` for verified artifact
  staging, exact-case execution in the configured existing login session, bounded waits,
  evidence collection and owned-task retirement. Keep host/account/session/client paths
  in HOME. Do not recreate one-off SSH or scheduled-task scripts, manufacture a login,
  override product request deadlines, or expand a selected case to the hosted matrix.
  A failed test may leave native configuration requiring its established recovery path;
  runner `finish` removes only its stopped task and lock, never user configuration.

## Repair and finish

- Preserve Cargo fail-fast by default. Use `--no-fail-fast` only for explicitly scoped
  batch diagnosis. An early stop leaves observed failures and unexecuted checks;
  it does not establish a complete failure set. Classify failures and batch related fixes.
- After a final failure, inspect the report before retrying. Repair the applicable
  assertion, selection, build-context or harness gap and demonstrate that the
  regression catches the old failure. Isolated success alone does not prove noise.
  Rerun affected checks and explicit gates, without restarting the full set after every edit.
- Once affected checks pass, freeze the candidate and complete the selected final
  checks once. Every selected check and gate needs passing evidence. Run affected
  deterministic generators twice; the second run must produce zero diff.
- Report source SHA, command, run/evidence, features/platform, executed cases and
  remaining obligations. Separate process exit from business-scenario outcomes;
  missing or zero-selected required cases fail. Expected-red and fixture checks
  do not establish product completion. Reused evidence retains its original SHA
  and an explanation of the intervening change. Docs/tooling follow-ups do not
  restart unaffected Rust validation.
- At feature handoff, assess the existing report's slowest three modules once:
  optimize, defer or insufficient evidence, with observed cost, smallest change
  and preserved assertions. Timing is advisory and does not automatically refactor
  a frozen candidate or start another full run.
- Before another local build and at handoff, retire only owned inactive outputs
  that are no longer needed, using the documented tools and locks. Save unique
  evidence first; record retained run IDs, purpose and end condition in the Issue/PR.
  Cleanup is housekeeping, not an additional validation gate or background task.
