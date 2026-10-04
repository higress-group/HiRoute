# Decision service behavior tests

Run `python -m unittest -v` from the extension directory with its declared
dependencies available. Tests use the real HTTP handler and a loopback upstream;
they neither load a real key nor call a paid provider.

Start with the capability classes in [test_server.py](test_server.py):

| Capability | Owning tests |
| --- | --- |
| Select an allowed branch, including a third natural-language branch; apply the existing binary rules policy | `BranchSelectionTests` |
| Assess prior execution separately from the new choice; preserve whole turns and mark missing evidence | `HistoryAssessmentTests` |
| Explain decisions and retain reported costs, including invalid decisions; never invent missing usage | `DecisionEvidenceTests` |
| Enforce authentication, input/output bounds, one-call time budget, concurrency, proxy routing and health isolation | `ServiceBoundaryTests` |
| Load bounded configuration and fail invalid policy files without silently replacing them | `SettingsTests` |

[decision_cases.py](decision_cases.py) contains independent request, upstream
answer and expected-output examples. One HTTP test runs those examples for binary
auto, prior-segment assessment, the rules competence guard and generic auto
selection. It checks the full upstream payload, exact prompts, response and one
authenticated call. Four former success tests are represented by these four
capabilities, rather than four copies of the launcher and response assertions.
The generic case actually selects the third branch; it does not merely check
that the third definition was forwarded. A fifth example retains the distinct
binary-plus-extra-branch boundary: default smart-saving descriptions must not
replace an extended request's definitions.

[test_decision.py](test_decision.py) reuses those same examples at the pure
module boundary, without credentials, HTTP or an event loop. Run it with
`python -m unittest tests.test_decision -v`; no third-party dependency is needed.
The HTTP invocation remains necessary to prove production wiring, single-call
behavior and authentication. It shares expectations instead of maintaining a
second set of almost identical fixtures.

Literal prompts are a model-facing behavior contract, not a snapshot of source
layout. Expectations never import producer prompts or calculate the expected
branch through the production algorithm. Prompt changes require a deliberate
contract update; moving Python functions does not.

Keep the distinct threshold, partial-history, invalid optional Score, unknown
branch, timeout, authentication and paid-failure-usage assertions. They cannot
be replaced by a single successful branch-selection example. These tests prove
the extension contract; HiRoute routing, publication, cancellation and actual
business-model selection belong to the production Gateway tests linked in the
[extension README](../README.md#opt-in-hiroute--jev-smoke).
