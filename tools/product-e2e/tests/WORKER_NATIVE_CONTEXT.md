# Worker native context acceptance

The core and boundary journeys use the real CLI, daemon, Gateway, installed native CLI and ACP
adapter. Only the model upstream is synthetic. It creates a private product HOME
with ordinary user/project skills, conflicting provider configuration and a
neighboring user file. It neither reads a daily account nor replaces ACP.
Qoder has a separate explicitly logged-in [acceptance entry](QODER_DELEGATION.md);
it reuses these journeys without adding Qoder installation requirements to this target.
Headless Codex/Claude cases select a non-default native configuration directory explicitly;
Pilot cases use its documented native roots beneath the selected process HOME.

## Business cases and proof levels

The [Rust target](worker_native_context.rs) invokes the
[core journey](../../../crates/daemon/tests/support/native_context_product.py) and
[boundary journey](../../../crates/daemon/tests/support/native_context_boundaries.py)
for both Codex and Claude. This is an assertion map, not a record of executed checks.

| Stable case ID | Core journey | Boundary journey or separate contract |
| --- | --- | --- |
| `worker.context.native-skills` | Native discovery, real tool execution and independent receipts from user and project skills; Claude invokes its native `Skill` tool. Conflicting settings and the user proxy trap must not divert the model route. | Boundary tools also require the native skill receipts before their heartbeat scripts start. |
| `worker.context.concurrent-routing` | Not a core outcome. | Boundary: simultaneous native tools in one HOME/config/workspace, distinct Plans, endpoints, source credentials, models and native sessions; each source rejects the other task's marker. |
| `worker.context.run-authority` | Conflicting provider settings must not override the selected route. | Fresh run token, expiry, revocation and restricted permissions are not completed by either report. Keep [profile tests](../../../crates/daemon/src/delegation/profile/tests.rs), [journal contracts](../../../crates/daemon/src/delegation/persistent_journal_tests.rs) and the separate [lifecycle journey](worker_delegation.rs). |
| `worker.context.exact-continue` | Immediate and restarted Continue retain the native session and correlated tool history after publishing a different source/model; task Plan stays frozen and replay sends no upstream request. | Boundary: temporarily remove only the exact fixture transcript; Continue must reject or fail native load without a result, model request, replacement session or changed neighboring history. |
| `worker.context.cancel-owned-work` | Not a core outcome. | Boundary: formal cancellation removes the target native tool PID and stops its heartbeat while the neighbor remains live and completes. [Local process tests](../../../crates/daemon/tests/local_worker_platform.rs) retain child-chain/root-first-exit/failure assertions; the separate lifecycle journey remains required. |
| `worker.context.retention-ownership` | User configuration, skills and neighboring files remain unchanged through cleanup. | Neither report establishes hidden/expired body refusal or physical history deletion. [Read/Continue](worker_read.rs) and [store/maintenance consumers](../../../crates/daemon/src/control/runtime/delegation_maintenance_tests.rs) own visibility and task-metadata cleanup assertions; borrowed history stays native-client owned. |

The runner independently requires two core outcomes and three boundary outcomes
(concurrency, cancellation and missing-history Continue). See the
[boundary guide](../../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md)
for its observations and cleanup ownership. The core report still marks its four
`related_contracts` as `not_executed`; boundary results do not rewrite that report.
The lifecycle/read targets, component contracts and native Desktop each need their
own selected evidence. Keep missing, duplicate, non-green and zero-selected cases
failing. Default Cargo runs ignore both installed-Agent tests, even when the rest
of the workspace is green.

## Exact-candidate execution

Use `scripts/test-plan.py --base BASE` and the configured `scripts/validation.py`
backend entry. The plan exposes `product_checks` separately because ordinary
package tests do not execute ignored real-Agent tests. Set an exact full
`HIROUTE_PRODUCT_CANDIDATE_SHA` and explicitly selected absolute file paths:

- `HIROUTE_WORKER_CODEX_BINARY`
- `HIROUTE_WORKER_CODEX_ACP_ADAPTER`
- `HIROUTE_WORKER_CLAUDE_BINARY`
- `HIROUTE_WORKER_CLAUDE_ACP_ADAPTER`
- `HIROUTE_WORKER_NODE`

The selected command is:

```sh
cargo test --locked -p hiroute-product-e2e --test worker_native_context -- --ignored --nocapture --test-threads=1
```

The runner builds the current checkout's CLI/daemon and executes both Harnesses.
For a previously attested exact-candidate build, the Python entry can be called
with `REPO FULL_SHA`; `HIROUTE_VALIDATION_PRODUCT_BIN_DIR` selects that build's
directory. The core entry selects one Harness with `HIROUTE_PRODUCT_WORKER_HARNESS`;
the boundary entry requires `--harness codex` or `--harness claude`. Record the
original build proof and binary digests; pointing at arbitrary binaries is not
an acceptable shortcut. Do not run Agent work during a tooling-only review.

## Cheap preflight contracts

These checks prepare trustworthy inputs and verdicts before starting an Agent.
They do not satisfy the real-client or Desktop business cases.

| Contract | Smallest entry and useful assertions |
| --- | --- |
| Native SSE wire | [Fixture tests](../../../scripts/test-native-context-product.py): `test_responses_native_wire_has_typed_ordered_terminal_frames` and `test_messages_native_wire_has_typed_terminal_frames` make real loopback HTTP requests, checking JSON event `type`, Responses sequence numbers, tool and text terminal frames. |
| Owned directory permissions | The same entry's `test_fixture_new_skill_ancestors_are_private_under_group_writable_umask` and `test_fixture_never_chmods_an_existing_native_parent` check each newly created ancestor and preserve existing parent permissions. |
| Native context budget | `test_native_sources_declare_a_budget_the_selected_client_can_admit` checks [save_context_source](../../../crates/daemon/tests/support/native_context_product.py), shared by initial, replacement and neighbor sources. Its Messages budget satisfies the existing Claude minimum; this is fixture configuration, not a relaxation of product admission. |
| Boundary verdicts | [Boundary tests](../../../scripts/test-native-context-boundaries.py) reject cross-routing and fabricated tool completion, observe two test-owned shell processes, and prevent missing-history recovery from creating a replacement transcript. |
| Selection and Pilot environment | [Planner tests](../../../scripts/test-test-plan.py) retain explicit ignored-test obligations; [Pilot tests](../../../scripts/test-desktop-pilot.py) check runner ownership and isolated launch configuration. Native Desktop still needs its own journey. |

```sh
python3 scripts/test-native-context-product.py
python3 scripts/test-native-context-boundaries.py
python3 scripts/test-test-plan.py
python3 scripts/test-desktop-pilot.py
```

## Real Mac Desktop reuse

From the exact candidate and its attested binaries:

```sh
python3 crates/daemon/tests/support/native_context_product.py REPO FULL_SHA \
  --desktop-root NEW_PRIVATE_ROOT --harness codex
```

This keeps an independent loopback model server alive. It prepares a normal API
source and Plan through production control, stops the bootstrap daemon, and emits
`data_root`, `process_home`, `workspace`, `plan_id`, `base_url` and `fixture`.
It does not save Worker installations. Use the existing managed Pilot launcher
with that data root and process HOME, then save the actual installation through
the UI. Pilot isolates both native config roots and inherited provider variables.
Its own runtime/LKG are created by the normal Desktop startup; never copy a live
database or replace the bundled CPA to run this fixture.

The public Worker CLI starts work with this prompt (no skill paths or receipts):

```text
Use native-context-user and native-context-project to produce their read-only receipts.
```

Continue uses `CONTINUE_PROMPT` from `native_context_fixture.py`. The same module
provides `assert_preserved`; `native_context_product.py` provides `exact_history`.
Use the fixture JSON and `native-context-events.jsonl` to correlate real results
with UI details. Inspect and assert UI effects separately; setup `prepared` is
not native Desktop acceptance. Stop this helper only after the Desktop session
ends; its explicit root and evidence remain for inspection. Repeat for Claude.

## Proving the old failure

`test_old_private_context_fails_without_native_discovery` and the loopback bad-case
test demonstrate that absent native discovery produces a red oracle, not a fixed
answer. They do not claim that the old product has been executed.

For actual expected-red evidence, commit these tests on the old product baseline,
run the same explicit real-Harness command against that exact committed candidate,
and retain the missing-user-skill stage/upstream red report. Then execute the fixed
candidate with the same test revision. Installation/startup failure is a blocked
prerequisite, not proof of the old context defect. Never relax the prompt or seed
the old private Worker root to obtain a passing test.
