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

### Requirement: Evidence binds actual revision, artifact, role, and path

Report MUST include checked source revision, build/executable identity, platform,
scenario/contract version, actual entry/upstream, step outcomes, and safe correlation.
Equal program filenames cannot equate builds or process roles. Reused evidence
retains its original revision with unchanged-input explanation, never relabeled as
new execution. A candidate incompatible with a frozen oracle/profile must obtain
current source/build/artifact proof rather than lower trust checks. Real process,
listener/protocol and independent outcome evidence are required; static goldens/
fixture self-report/model success text are not actual user content or effects.

A missing essential publish/call step or substituted canned body MUST fail the
relevant oracle. Evidence-generation integrity is verified before/after launch;
large artifact verification time is runner overhead, not consumed from product-step
budgets. Build and scenario time are recorded separately.

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

## Implementation references

- [Product control journey](../../../tools/product-e2e/tests/control_shell.rs),
  [oracle](../../../tools/product-e2e/tests/product_oracle.rs), and
  [controlled Gateway runtime](../../../tools/e2e-harness/tests/p0_gateway_runtime.rs).
- [Evidence/fixture map](../../../docs/code-map/testing.md) and
  [validation selection](../../../docs/test-selection.md).
