# Tests as product documentation

[Code map](README.md) · [Desktop scenarios](../../apps/desktop/tests/README.md)

Organize navigation around user capabilities; keep tests physically beside the
layer that can exercise each invariant precisely. Product-oriented does not mean
turning every test into a slow UI journey. A small deterministic test of restoring
a user file after a conflict documents a product promise better than a repeated
happy-path end-to-end test.

## What each layer proves

| Layer | Appropriate assertion | Evidence limit |
| --- | --- | --- |
| Pure state / policy | User intent produces the right request; admitted branches, thresholds and scores behave as specified | Does not prove dispatch or a running service |
| Component with filesystem / database / controlled service | Conflict, rollback, ownership, authorization, timeout and recovery at a real boundary | Does not prove all production wiring |
| Desktop React + mock IPC | Visible controls, edits, pending/error feedback and commands sent by a user journey | Does not prove Tauri, daemon effects or native clipboard |
| Desktop [native command authority check](../../apps/desktop/tests/ui-consistency.test.mjs) | Registered bundled commands, generated ACL entries and explicit main-window permissions agree | Does not prove an actual WebView invocation; compilation and mock IPC alone can miss a denied command |
| Desktop Rust consumer compilation with `desktop-pilot --all-targets` through the managed Mac configuration | Shared contract changes compile through the Tauri bridge, protected Session and test consumers; Pilot includes `desktop-runtime` and supplies the configured ACL manifest | Does not execute tests or prove native behavior; featureless checks omit the bridge |
| Real CLI/daemon or Gateway listener with controlled upstream | Production composition, published plans, actual upstream attempts and durable effects | Does not prove a live model's quality or every OS |
| Native Desktop / real Agent harness / live provider | The selected real integration path | Environment-specific; record platform and revision |
| Frozen compatibility or expected-red fixture | A historical reader/rejection rule, or an intentionally incomplete composition | Not evidence of current product completion |

The shared [publication fixture's `Product.cli`](../../crates/daemon/tests/support/publication_product.py)
is command notation for internal Local Control preparation, not the standalone CLI.
Its protected-input/grant channel belongs to the daemon that the fixture starts;
it cannot be attached to an already running Desktop. Prepare sources before handing
that instance to Desktop, then perform protected user mutations through the real UI.
Native main-Agent delegation separately invokes the actual public Worker CLI.

Start from the representative cases in the [capability map](README.md). In
`tools/product-e2e`, inspect the declared proof level: `transaction_recovery.rs`
includes expected-red composition, `gateway_adapter_contract.rs` is component
coverage, and `product_oracle.rs` tests the oracle itself. A file named `product`
or a zero exit status alone is not acceptance evidence. Likewise, a test that
reads `green` from a scenario/golden file validates the recorded contract, not
that the journey ran. Compute, routing and observation fixture checks link to
the real `installed_standalone_cli_completes_the_headless_management_loop` in
[publication_process.rs](../../crates/daemon/tests/publication_process.rs); keep
those evidence levels distinct.

## Keep useful assertions, reduce accidental coupling

Name a scenario by the user's initial state, action and observable result. Arrange
current state through the current producer or a legal fixture builder; keep the
expected outcome independent of that builder. Freeze legacy bytes separately and
register their owner and removal condition in the compatibility registry.
For durable JSON, pair independent historical producer bytes with recovery through
the current production consumer. Preserve the failing package/feature selection:
package-level and workspace-level `--all-features` can resolve different dependency
features, including JSON object ordering. A current-serializer golden cannot prove
that an older committed journal remains recoverable.

Generic discovery and CLI journeys should require the capabilities they exercise
and unique identities, rather than freeze the total number of registered Agents.
Keep exact inventory completeness checks at the registry/catalog boundary. Adding
an ecosystem should extend its own product assertions without forcing unrelated
process, privacy or configuration journeys to change their expected list length.
For Agent settings, start from the shared enable/adjust/disable and blocked-action
recovery journeys in the [feature guide](../../apps/desktop/src/features/agents/README.md).
Compare an added ecosystem's interactions with existing ones before turning a new
button or status label into an expected result. Assert saved intent, independent
facets and recoverable failures; retain UI detail assertions only where the
durable product contract requires that detail.

A saved integration must be tested through normal startup reading that saved
configuration. A fixture that injects an endpoint, provider, credential or settings
override can prove a transient launch path but cannot prove persistent connection
setup. Keep configuration writers on an explicitly selected acceptance context;
borrowed daily login for a read-only Worker journey is not authority to write its
model providers. Reuse the same production control and upstream oracle, with a
small native launch leaf for each distinct startup contract.

For a new ecosystem, reuse [Agent product support](../../crates/daemon/tests/support/agent_product_support.py)
for product settings Preview → Apply → current-status reads and bounded native CLI
execution with private output capture. Keep native argv, configuration ownership,
terminal-result parsing and independent business assertions in the ecosystem leaf.
The process helper proves neither a model request nor a successful task by itself.
[NativeContextUpstream](../../crates/daemon/tests/support/native_context_fixture.py)
owns the Responses/Messages transport and counts every attempt, including rejected
credentials; new journeys should not replace its HTTP handler to add observation.

Before copying a native journey, identify which shared path already owns setup,
source routing, exact Continue, cancellation and cleanup. Add a new native leaf
only for an actual protocol/configuration difference. Keep native-history layout
witnesses isolated and version-specific; do not share production code that computes
the same expected answer with its test oracle. Register a new helper's actual
consumers in the test planner, including its cheap fault/ownership checks.

A conflict fixture also needs a positive control: establish that the native client
would consume its provider, hook or auxiliary-model setting before claiming the
managed launch defeats that setting. Keep this expensive native proof distinct
from exhaustive deterministic merge, drift and restore tests. Observing a process
exit or a model's own success text never replaces the Gateway receipt and exact
route/authorization assertions.

When exercising a real store, keep its fresh storage directory separate from
fixture executables and native workspaces. Production correctly refuses to
initialize over unrelated files; a fixture must not weaken that guard to finish
setup. Borrowed user context, product storage and temporary run materials also
have different cleanup owners—represent those roots explicitly in the fixture.
Reusing native HOME for login and Skills must not reuse HiRoute test receipts.
Give each product instance a private `HIROUTE_WORKER_RECEIPT_DIR` and propagate it
to public CLI subprocesses, including those launched by the main Agent. Otherwise
an old receipt can replay a different daemon's operation for the same working
directory and idempotency key. Keep that namespace stable within the instance for
retry assertions, and retain the production revision/idempotency guards.
When a native acceptance fixture needs a writable Skill target, it may create a
new empty leaf with explicit permissions and recorded ownership; the production
operation must still install and restore the actual Skill. Never normalize a
pre-existing user's directories to make a test pass. Cleanup checks the recorded
directory identity, permissions and emptiness before removing it. Separate
private-HOME negative cases prove that unsafe targets remain rejected and cannot
block unrelated startup merely by being registered.

Before starting a native journey, locate its explicit inputs, borrowed/writable
roots, cleanup owners and required outcomes in the acceptance entry guide; the
[Qoder run contract](../../tools/product-e2e/tests/QODER_DELEGATION.md#run-contract)
is one example. If preparation or cleanup fails, report that stage and retain the
first product failure. A fixture check, successful setup or process exit cannot
stand in for the declared business outcomes. Scope UI readiness checks to the
current feature and expected target; a matching list on another mounted page
does not establish that the Agent list has loaded.

Prefer behavior over source layout: rendering and request assertions survive a
component move; regexes for a local variable name do not. Keep source checks only
for intentional static policies (for example forbidden credentials or public
contract tokens). Select browser scenarios by stable IDs/capabilities, not array
positions or translated display names, and require both a nonempty selection and
completion of every required scenario.

Before deleting or consolidating a test, identify its unique failure condition and
assertion. Keep crash/reopen, cancellation, foreign-file conflict, stale preview,
authorization, timeout and compatibility cases even when happy paths overlap.
Two layers may intentionally protect the same promise against different failures.
Do not replace these checks with constants, snapshots of implementation structure,
or expected values computed by the production algorithm under test.

For a behavior-preserving refactor, commit representative behavior tests before
moving production code. For a faulty gate, demonstrate that a realistic bad case
fails, then fix the gate. A passing synthetic test of the gate must not substitute
for running it against the actual production source.

## Select and report validation

Storage upgrade recovery is a product promise, separate from native model-call
verification. The [fixed crash-boundary scenarios](../../crates/local-storage/src/migrations/upgrade_batch_recovery_tests.rs)
exercise missing original backups, interrupted prior batches and real intermediate
SQL commits through daemon startup. They reuse frozen producer journals, preserve
keys and backup ownership, and pin historical versions rather than generating an
interruption by upgrading straight to `LATEST_SCHEMA_VERSION`. A new migration
must review the whole accepted source-to-target chain and these restart boundaries.

1. Run `python3 scripts/test-plan.py --base <exact-base>`; add `--integration` when
   combining branches. Explain any semantic scope adjustment using changed
   behavior and affected consumers.
2. Follow the selected plan and [CI check definitions](../github-actions-validation.md). Use the configured backend,
   frontend and native environments. Validate committed Rust candidates through
   `scripts/validation.py` with the feature plan, phase and related run IDs.
   Follow shared contracts through existing serialized fixtures and
   [client transport tests](../../crates/client-core/tests/transport.rs), as well as
   CLI and Desktop consumers, even when those files have no diff. Keep exact wire
   and completeness checks (for example, every supported Worker harness); update
   obsolete fixtures instead of weakening response validation. Run the plan's
   `native_checks` early; enabling the correct feature and compiling existing
   consumers catches type drift without adding per-field tests.
3. Diagnose focused failures before retrying. Freeze the candidate after focused
   success; complete the selected final checks once. Required cases left
   unexecuted remain pending, not green.
4. Record exact revision, command, selected cases, run/report location, platform,
   outcome and evidence limits. Preserve unchanged-code evidence at its original
   SHA. Review the slowest three measured modules once: record the measured cost,
   smallest useful improvement and assertions to preserve, or explain deferral.

Review test churn by cause, not just count: product changes, new regressions,
fixture/schema repairs, source-layout assertions and harness repairs need
different remedies. Line count, coverage percentage and test count are signals
for review; none is an automatic cleanup target.

When changing scenario selection, inspect every importing fixture page as well as
the main runner. Standalone editor pages must request stable scenario IDs and
required IDs too; positional slices can silently select different cases after
another feature adds scenarios. Keep their retained assertions visible in the
fixture result, even when the main suite exercises the same components.
