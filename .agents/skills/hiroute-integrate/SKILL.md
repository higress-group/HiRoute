---
name: hiroute-integrate
description: Integrate HiRoute task branches and converge their validation before handoff or an authorized merge. Use for combining branches or resuming integration after upstream updates; not ordinary feature implementation or standalone test runs.
---

# HiRoute integration

Own the combined candidate and cross-branch convergence. Use
[HiRoute validation](../hiroute-validation/SKILL.md) for check selection, configured
execution, failure repair and evidence; run repository commands from its root.

## Establish the candidate

Record the exact target baseline and requested source tips after fetching. Inspect the
checkout before branching or merging; preserve unrelated work. Resolve conflicts against
current requirements and both branches' consumers. Review each new upstream diff before
accepting it; an upstream update creates a new candidate, not evidence for the old one.
Use the configured code-host workflow for merges only within the user's authorization.
When the combined diff affects released data/configuration, recovery or public CLI/Gateway
behavior, use [compatibility maintenance](../hiroute-compatibility/SKILL.md) to check the
combined source-to-current path; individually passing branch tests may miss migration conflicts.

## Expose cheap integration failures early

For an SSH backend integration with repeated checks, retain the first managed
checkout with `--retention always` and a nonempty plan. Continue from the last
stopped run using `--continue-checkout LAST_RUN_ID`; keep its checkout owner for
final cleanup. This preserves Cargo artifacts across descendant candidate SHAs,
not earlier test verdicts. See [retained integration checkouts](../../../docs/validation-routing.md#continue-an-integration-checkout-and-schedule-the-final-workspace)
for refusal conditions and cleanup ownership.

Run `python3 scripts/test-plan.py --base BASE_SHA --integration` after combining changes.
The top-level plan retains final obligations. `integration.commands` puts selected Python
gates and known coverage/protocol/runtime checks first, even when a lockfile selects full.
Inspect its paths and reasons: an empty early target list does not prove integration coverage.
Add missing consumers based on the diff; preserve required native/product acceptance cases.
The planner only prints commands. Execute preflight checks as focused runs through
the shared validation workflow; they do not waive the final plan.

Run the coverage gate before protocol/runtime checks. If generated data is stale, review
the source change and let the convergence owner use the validation generator protocol.
A checksum refresh is not evidence that a changed assertion still represents the contract.
For affected Desktop journeys, resolve the required capable environment before a costly build.

When UI branches overlap, converge on the [shared interaction guidelines](../../../docs/desktop-interactions.md)
and the combined production pattern. Do not resolve a conflict by retaining two
competing controls or layering incompatible styles. Use UI-quality checks on the
affected combined flows, including draft/confirmation guards and comparative evidence.

## Use platform builds and the development machine during iteration

For Windows changes, follow [Windows iteration](../../../docs/validation-routing.md#windows-iteration-and-release-validation):
run affected Linux checks, cross-compile the affected Windows production and test
targets on the configured Linux workbench, then execute those exact artifacts on
the configured Windows development machine. Retain the warm managed checkout
across repairs. Compile affected macOS consumers through the configured Mac when
shared contracts or Unix paths change; a Linux or Windows pass does not cover them.

Select real Windows cases by the changed boundary: CLI/Gateway, native files and
permissions, Worker/Agent, or installed background/upgrade. Do not start the hosted
Windows matrix for each commit. The release-candidate workflow owns that expensive
gate; its manual entry remains available for a CI-specific failure or unavailable
development capability. Neither cross-compilation nor hosted compile-only cases
replace required native business execution. Record any missing OS, architecture,
interactive-user or account capability explicitly before declaring convergence.

## Diagnose once within the affected boundary

Use the validation failure-repair route for reports, original evidence and affected checks.
When multiple related failures may remain, use `--integration --collect-failures`
to propose bounded Cargo collection, review its scope, and explicitly run it.
It does not continue later commands or collect every workspace failure.
Do not add product smoke to diagnosis unless a product path or an unresolved consumer needs it.
If early coverage fails, repair it before continuing the behavior command on the new candidate.

Batch related integration repairs. Use `--since CHECKPOINT_SHA --integration` to obtain
an iteration preflight while retaining the original base plan and its Cargo package context.
Keep original failures and linked run IDs. Preserve the failing package/feature context;
requested flags alone do not prove the focused and final commands resolve the same feature graph.

## Freeze and finish

Freeze the integrated source heads and use the shared validation workflow to finish
the selected final checks on the combined candidate. For a final full locked backend
test, use the documented [workspace DAG schedule](../../../docs/validation-routing.md#continue-an-integration-checkout-and-schedule-the-final-workspace).
Preserve early diagnostic evidence and final obligations; avoid running expensive
smoke in both phases without a concrete unresolved reason.

Record the target base, integrated source SHAs, shared-file resolution and remaining
production-wiring gaps in the Issue/PR, alongside the validation handoff. Individual
branch passes do not prove the combined production path. Merge/push only when
authorized, then verify the remote target contains the intended sources.
