# Product smoke and evidence

## Purpose

Smoke tools execute a bounded selected product journey and report what it actually
proved. Scenario success, process exit, fixture correctness, and missing environment
are separate results.

## Requirements

### Requirement: Every scenario declares and uses its production entry

A scenario MUST identify domain/requirement, bounded operations, prerequisites,
entry, assertions, and evidence. CLI uses actual hiroute/hirouted and public control;
shared-client actions use real Client Core/Application. A controlled upstream may
supply deterministic protocol/fault conditions. Standalone Gateway may use declared
publication/credential inputs but proves execution-plane scope only. Claims of source
import, publication, Agent configuration, or restore cannot bypass those production
operations by writing SQLite/Secrets/native files/publication directly. Synthetic
initial fixture setup is not a successful product operation.

Default bounded regression includes nonempty actual controlled Gateway requests
and local-control read/discovery/safe-rejection scenarios with independent results.
A safe negative rejection can be green while its positive feature remains unproved.
Missing production composition is explicit; an internal-seam substitute cannot be
relabeled production success.

### Requirement: Selection is complete and result classes are explicit

Each run records requested selection, required/expected identities, actual hits,
and per-case terminal result. Zero selection, missing required case, duplicate ID,
or missing terminal fails the requested validation. Process exit remains separate
from green, expected_red, red, and not-executed. Registered expected_red means the
known capability/gap, never delivery completion. Missing account/platform/dependency
is not-executed with reason, not green or executed expected_red. Required red/missing/
expected-red capability prevents acceptance even if a fixture test legitimately exits
zero. Existing runner result contracts cannot be weakened to obtain green.

Ordinary Rust validation results, compact waits, and module reports MUST count
independent cases from complete Cargo-target terminal summaries, excluding nested
child-process summaries. Child assertions and diagnostic output MUST remain available.
Unattributed output, ambiguous target identity, or an unfinished parent MUST NOT
establish successful test execution, even if a child's test count matches its parent.
Every completed target contributes its own passing, failing and ignored counts;
a later successful target MUST NOT erase an earlier failed target. A nonzero command
exit MUST fail the run independently of otherwise successful test summaries.

#### Scenario: Nested subprocesses finish inside a successful parent target

- **WHEN** a Cargo target completes its selected parent tests after printing nested child summaries
- **THEN** run results, compact waits and module reports expose the parent target counts, retaining the child output without adding independent coverage.

#### Scenario: A child finishes but its parent target is interrupted

- **WHEN** the final observed child summary has no completed parent summary, including when both selected the same number of tests
- **THEN** validation remains incomplete and the child summary cannot establish parent success or independent parent counts.

#### Scenario: Installed-package prerequisites fail before expensive execution

When managed SSH backend selection includes installed-package acceptance requiring
the console, the runner MUST verify the candidate's frontend source revision and
output hashes before starting Rust execution. Missing or mismatched assets block
setup with actionable preparation instructions; they are not product failures or
successful acceptance. The caller supplies prepared assets in a separate execution-host
staging directory; the runner verifies them and places them in the retained candidate
only under its checkout lease. Retained assets are rechecked before use. Selections
that exclude the installed scenarios MUST NOT require a frontend build. Real installed
management and Quickstart assertions remain required after preparation; artifact verification alone does not
establish their outcomes.

#### Scenario: Resume a retained checkout after a recoverable preparation block

When a retained integration checkout has advanced through one or more candidate
revisions, a missing or mismatched console input, or failed cache admission before
the selected command starts, MUST leave its exact latest stopped run recoverable.
The fixed checkout owner controls retention and cleanup; current resource state
and the latest run control recovery. A prior owner's SHA or verdict MUST NOT stand
in for the current candidate. Recovery instructions and admission MUST use the same
explicit preparation-state policy and preserve the selected host/workbench,
candidate, command, timeout, schedule, plan, phase and executed related-run links.

After preparation, the generated command MUST create a new run in the same retained
checkout and recheck its lease, stopped worker, latest-state identity, clean source,
origin, plan, driver/helpers, descendant revision and required artifact provenance.
Unknown or executed blockers, stale borrowers and invalid resource state MUST NOT
gain preparation-recovery admission. All earlier results retain their source and
verdict; an unexecuted blocked run MUST remain ineligible as related execution
evidence. The selected tests MUST actually execute after recovery; valid artifacts
alone do not establish acceptance. Fresh blocked `on-failure` owners retain their
exact-candidate reuse path; `never` retention requires a new checkout.

After an executed, non-timeout failure, an otherwise eligible retained checkout
MUST remain reusable for the exact same candidate, even without a plan or when
its original `on-failure` owner had a recoverable preparation block. Reuse MUST
recheck the latest resource state and preserve earlier results; an unchanged
owner result alone MUST NOT establish admission.

### Requirement: Evidence binds actual revision, artifact, role, and path

Report MUST include checked source revision, build/executable identity, platform,
scenario/contract version, actual entry/upstream, step outcomes, and safe correlation.
Equal program filenames cannot equate builds or process roles. Reused evidence
retains its original revision with unchanged-input explanation, never relabeled as
new execution. A candidate incompatible with a frozen oracle/profile must obtain
current source/build/artifact proof rather than lower trust checks. Real process,
listener/protocol and independent outcome evidence are required; static goldens/
fixture self-report/model success text are not actual user content or effects.

Reusable native Desktop build identity MUST bind the exact CPA manifest bytes
captured and validated at acquisition, resolving the default from the exact
candidate or an explicit package-relative source. Compilation MUST consume a private
build-owned snapshot of that same captured buffer. Equal bytes MUST permit reuse
across source paths and default/explicit selection when all other build inputs
match. Original source paths MUST NOT be retained in recipes, lease state or public
evidence. Reuse and startup MUST verify the snapshot and existing artifact
attestations; later source modification or deletion MUST NOT invalidate an acquired
build or lease. A fresh acquisition MUST inspect its current input and reject
missing, unreadable, symbolic-link, special-file or invalid UTF-8 JSON object input,
including non-finite JSON constants, before starting the managed build. A hardlinked
regular source is admissible by its captured contents.

A missing essential publish/call step or substituted canned body MUST fail the
relevant oracle. Evidence-generation integrity is verified before/after launch;
large artifact verification time is runner overhead, not consumed from product-step
budgets. Build and scenario time are recorded separately.

Build preparation MUST bind each selected product role to its exact build receipt
and receipt digest, including the recipe/environment identity verified by its
producer. Different valid recipes MAY retain identical executable bytes in the
same checkout. Unrelated receipts MUST NOT make the selected preparation ambiguous
or substitute a different runtime environment. Admission still verifies the source,
receipt and executable digests, checkout path containment and consistent selected
environment. Missing, changed or conflicting selected evidence MUST fail admission.

#### Scenario: Continue focused smoke with a full workspace preparation

- **WHEN** focused smoke and full workspace preparation use one candidate checkout
  with different build environments and identical resulting executable bytes
- **THEN** each preparation selects its own exact product and Gateway receipts and
  runtime environment without removing or rewriting the other receipts
- **AND** a missing, corrupt, escaping or conflicting selected receipt remains a
  failure even when another valid receipt has the same executable digest.

#### Scenario: A changed CPA source selects a new build without invalidating an old lease

- **WHEN** an explicit CPA manifest is selected for a reusable Desktop Pilot build and its bytes later change at the same path
- **THEN** a fresh acquisition has a different build identity, while the old lease remains usable with its verified frozen snapshot even if the source is subsequently deleted.

#### Scenario: Equivalent CPA sources reuse the same frozen build

- **WHEN** different paths or default/explicit selections resolve to equal manifest bytes and all other build inputs match
- **THEN** acquisition reuses the same verified build without retaining the original source path.

### Requirement: Validation tracks have independent evidence limits

| Track | Can establish | Cannot substitute for |
| --- | --- | --- |
| Unit/component | Its state/policy/owned boundary | Installed product/native journey |
| Real listener with controlled provider/emulator | Actual declared runtime path and deterministic faults | Native Agent, real-account quality, Desktop/install |
| Explicit real account/native Agent | Selected installed client and authorized model/tool path | Other clients/platforms or synthetic fault coverage |
| Native Desktop | UI through Client Core/backend and observed business effect | Merely browser fixtures or compile success |
| Installed package | Exact candidate works after production installation | Other architecture/platform or live provider quality |
| Frozen/expected-red/oracle fixture | Historical reader or intended rejection/assertion | Current production feature completion |

These may combine but remain labeled. Controlled faults cover 429/disconnect safely;
normal real-account calls are not used to force rate limits. Missing Windows stays
unverified regardless of cross-platform source/compile contracts.

### Requirement: Native acceptance isolates resources and restores via the product

Codex/Claude acceptance uses controlled effective config/launch/env/cwd, private test
storage/runtime, and explicit account authorization rather than normal user's daily
configuration/tasks. Other Harnesses use their documented borrowed-resource versus
writable-configuration boundaries. If required isolation/ownership cannot be proven,
stop before changes/model calls; do not test by exploring daily settings. Version
labels are diagnostic, not a new software allowlist.

Configuration/restore assertions call real Preview/Apply/restore with retained
product service and recovery evidence. Copying a backup file or deleting a test root
is not product restore. Failure/cancel stops owned processes and cleans temporary
resources after verified restore. User same-field changes create conflict and retain
safe recovery state with explicit repair, never forced overwrite or success. Abnormal
process exit cannot automatically overwrite native settings. Real account/native
journeys remain explicit opt-in, outside default no-account smoke.

### Requirement: Scenario owners reuse shared runner contracts

Domains own their minimal product operations, success/failure assertions, and fixes;
runner owns bounded selection/execution/evidence/reporting. At least two distinct
domains use the common contract without rewriting a central dispatcher or introducing
an unnecessary dynamic registry. Descriptors do not provide arbitrary shell/URL/
Secret access. New behavior requires a bounded explicit contract. Shared manifests,
lockfiles, and generated registries have one convergence owner, not competing writes.
Affected deterministic generators run twice, with no second-pass diff.

### Requirement: Failures retain safe evidence and bounded cleanup

Reports expose failed stage, closed error, missing conditions, and rerun selector
without Keys/OAuth/full Agent config/daily body/recoverable secret paths. Necessary
raw evidence stays in private run artifacts with no automatic upload, distinct from
[typed safe diagnostics](../runtime-diagnostic-logging/spec.md). Isolated smoke uses
Debug by default or its explicit level, verifying applied client/engine roles without
changing user settings or success gates.

Startup/readiness/steps/cancel/exit are bounded; timeout cannot leave runaway children
or a success report without terminal result. Focused reruns require nonzero selected
cases and do not force unrelated product matrices. Cargo output stays in each
worktree's normal target, and artifact cleanup preserves active use and unique
evidence. A leak scan failure blocks acceptance. Concrete measured timing belongs
to the runner/run record, not an unmeasured duration promise here.

If preparation fails after independent targets execute, the run MUST retain their
attributable terminal results and counts, the preparation failure, and explicit
unexecuted target/doctest coverage. Completed independent targets cannot establish
complete validation when dependent targets were not admitted.

### Requirement: Native Pilot App reopening preserves its isolated owner

An explicitly requested macOS native Pilot App entry MUST belong to one registered
open reusable-build lease and one private session. Its original/deployed Desktop,
daemon and CLI identities, candidate, launch guard, bundle metadata and isolation
directories MUST be verified before every initial or ordinary operating-system
launch. The entry MUST require the managed build's attested non-persistent WebView
configuration; ordinary production builds retain their existing launch behavior.

Initial launch and ordinary reopening MUST use the same explicit test data root,
private runtime/temporary directories, working directory and isolated HOME/native
Agent configuration locations. Ambient authentication environment MUST NOT be
persisted in the entry or borrowed from the operating-system launcher. Native
reopening MUST NOT fall through to the daily/default product profile. WebView state
remains non-persistent while the test profile's product state survives reopening.

Concurrent ordinary openings MUST NOT create a second untracked instance. Each
new process generation MUST record its PID, native start identity and exact
executable before business actions. Each App's deployed executables MUST preserve
attested source bytes using distinct file identities, so concurrent Apps cannot
redirect one another's native executable path. Observation MUST NOT launch a process.
Ordinary application quit or explicit Pilot stop leaves the open lease's entry
reopenable. Lease finish MUST revoke the entry before process cleanup and refuse
subsequent openings; cleanup MUST affect only verified owned processes and deployment
files, retaining evidence and refusing unknown/reused identities or changed artifacts.

#### Scenario: Native acceptance across ordinary reopen

- **WHEN** an exact-candidate App is physically selected/opened, its real daemon
  and CLI are verified, and the application is quit then opened through its
  ordinary native entry
- **THEN** the new recorded process uses the same isolated profile without a
  duplicate/default instance; replaced launch artifacts are refused and owner
  retirement stops only that instance and prevents reopening
- **AND** local fixture-process or IPC-only success cannot establish this native
  scenario; unavailable LaunchServices/native CU leaves it unexecuted

## Implementation references

- [Product control journey](../../../tools/product-e2e/tests/control_shell.rs),
  [oracle](../../../tools/product-e2e/tests/product_oracle.rs), and
  [controlled Gateway runtime](../../../tools/e2e-harness/tests/p0_gateway_runtime.rs).
- [Desktop Pilot reusable build owner](../../../scripts/pilot-builds.py) and
  [lifecycle regressions](../../../scripts/test-pilot-builds.py).
- [Evidence/fixture map](../../../docs/code-map/testing.md) and
  [validation selection](../../../docs/test-selection.md).
- [Smoke preparation](../../../tools/product-e2e/src/smoke/preparation.rs),
  [Gateway preparation](../../../tools/e2e-harness/src/p0/production/candidate.rs), and
  [DAG admission and regressions](../../../scripts/test-validation-schedule.py).
