# Preparation evidence and validation scope

These records describe preparation only, before any formal paid model result.
The [common protocol](decision-routing-20261006.md) is the authority for execution.

- Audit reference passes all six cumulative stages: 3, 6, 9, 12, 16 and 21 test
  cases selected. The stub fails at stages 1 and 6. Five deliberately defective
  references fail on unknown-as-zero, oldest revision, last retry status, partial
  transactional update, and missing-cache-as-zero behavior.
- A real inherited Mac sandbox separately passes the reference's 21 final cases
  and rejects the stub. It protects the live subject while grading a copy.
- Direct and symlink reads of the hidden reference and credential files are
  denied. Local listener connection succeeds; external and other-local-port
  connections are denied, including from a child process. The tested SBPL uses
  `(remote ip "localhost:55976")`; numeric IP syntax was rejected during setup
  and was corrected before any subject execution.
- Writing structural verification accepts a synthetic valid artifact set and
  rejects malformed review JSON, a too-short final and a symlink final. These
  checks say nothing about prose quality; that requires the parent assessment.
- The initial audit report incorrectly subtracted unittest subtest failures from
  parent test counts. It was repaired to count distinct failed cases before
  freezing; the original diagnostic output is retained, not presented as a grade.
- Contract convergence: 20 checker unit tests pass; the repository scan is green
  (24 subjects, 23 registered legacy supports).

Local raw evidence lives in the task-owned `decision-routing-20261006/preparation`
directory. It is not copied to subjects or included as a public account snapshot.
The executor must bind the same files' hashes to the final committed protocol,
re-probe each actual subject HOME boundary, and record the exact running product
source separately. No generated subject or live quality outcome exists yet.

## Selection adjustment

`scripts/test-plan.py --base 1ad66b18174edfa3bc89762acbdf63c23741975c` proposes full
Rust/frontend checks because the new private experiment helper is an unclassified
path. The actual diff contains only experiment documentation, tasks, independent
Python verifiers and a small caller of the unchanged grading boundary. It changes
no product Rust/TypeScript, production contract, dependency, manifest or generator.
Rust/frontend rebuilds cannot exercise these inputs and are omitted. The selected
checks are the reference/negative controls, actual OS boundary and isolated grader,
writing structural controls, Python syntax/JSON/local-link checks, diff whitespace
and the two contract-convergence checks. Real product behavior is tested by the
explicit live experiment and session-observation gates, not by these preparations.

Preparation timing is small (control batch roughly seconds, isolated grader about
one second, contract scan about seconds). Exact per-module timing was not collected;
there is insufficient evidence for a meaningful slowest-three optimization.
Preserve all assertions and defer harness optimization. Do not rebuild or refactor
the already accepted product to speed up preparation.
