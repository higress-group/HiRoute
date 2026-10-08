# Tests as product documentation

[Code map](README.md) · [Desktop scenarios](../../apps/desktop/tests/README.md) ·
[Agent/Worker scenarios](worker-context.md#representative-scenarios)

Choose a user promise, then the layer that can observe its failure. Keep tests
beside that owner and preserve independent assertions. A focused filesystem
conflict test can document a product guarantee more precisely than another broad
happy-path UI journey.

## What each layer proves

| Layer | Appropriate assertion | Evidence limit |
| --- | --- | --- |
| Pure state / policy | Intent, thresholds, branch isolation and request construction | No service dispatch or production wiring |
| Component with real filesystem/database/controlled service | Ownership, rollback, authorization, cancellation and recovery | Not every production entry or OS |
| Desktop React + mock IPC | Controls, edits, pending/error feedback and emitted commands | No Tauri effects, native clipboard or actual daemon mutation |
| Desktop command authority / native compilation | [ACL check](../../apps/desktop/tests/ui-consistency.test.mjs), configured Mac `desktop-pilot --all-targets` consumer compilation | An allowed/compiled command is not a WebView invocation or native scenario |
| Real CLI/daemon or Gateway listener | Published plans, actual upstream attempts and durable effects | Controlled upstreams do not establish live-model answer quality |
| Native Desktop / installed Agent / live provider | The selected real integration path and independently observed outcome | Bound to its exact environment, revision and executed scenarios |
| Frozen compatibility / expected-red / oracle fixture | A historical reader, intended rejection or test oracle | Not current product completion |

Start with the representative cases in the [capability map](README.md). A file
named `product`, a golden containing `green`, or a zero process exit does not prove
execution. In `tools/product-e2e`, [transaction recovery](../../tools/product-e2e/tests/transaction_recovery.rs)
contains expected-red composition, [adapter contracts](../../tools/product-e2e/tests/gateway_adapter_contract.rs)
are component evidence, and [oracle tests](../../tools/product-e2e/tests/product_oracle.rs)
validate the oracle. The real installed management loop is in
[publication_process.rs](../../crates/daemon/tests/publication_process.rs).

## Reuse the fixture at the owning boundary

| Need | Reuse / preserve |
| --- | --- |
| Prepare a daemon's current sources, grants and plans | [Publication Product](../../crates/daemon/tests/support/publication_product.py): its `Product.cli` is internal Local Control notation, not the standalone CLI; protected input belongs to that fixture's daemon |
| Configure a real native Agent | [Agent product support](../../crates/daemon/tests/support/agent_product_support.py): shared Preview → Apply → status and bounded process capture; native argv/ownership/result interpretation remain in the leaf |
| Prove saved model configuration | [Additional-model journey](../../crates/daemon/tests/support/additional_model_product.py): ordinary native startup reads saved settings, without injected provider/endpoint/credential overrides |
| Observe native routing and rejected attempts | [NativeContextUpstream](../../crates/daemon/tests/support/native_context_fixture.py): count every model POST, including rejected endpoints/models/credentials; keep the ledger for positive route/history assertions |
| Prove exact Continue, concurrent routes and cancellation | [Native context](../../crates/daemon/tests/support/native_context_product.py), [boundary journey](../../crates/daemon/tests/support/native_context_boundaries.py), [boundary oracle guide](../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md) |
| Prove restoration of old durable state | [Frozen producer fixtures](../../crates/local-storage/src/migrations/fixtures/README.md), [crash-boundary tests](../../crates/local-storage/src/migrations/upgrade_batch_recovery_tests.rs): old bytes through the current consumer, not a new serializer pretending to be history |
| Exercise actual Desktop controls | [Browser scenario map](../../apps/desktop/tests/README.md) and native acceptance: select stable scenario IDs and require every declared outcome |

The publication fixture leases its listener address across daemon restarts; do
not substitute a released `bind(0)` probe. Its protected-input channel cannot be
attached to an existing Desktop. Prepare that instance first, then use real UI
mutations. Main-Agent delegation must separately invoke the actual public CLI.

Keep borrowed user resources, writable native acceptance configuration, fresh
product storage and per-run materials separate. Each product instance needs a
private `HIROUTE_WORKER_RECEIPT_DIR`, propagated to public CLI subprocesses and
stable for retries. Expose the selected CLI and Pi's selected Node consistently
to main-Agent checks and Worker launch. Never repair permissions on pre-existing
user directories to satisfy a fixture; cleanup verifies the owned object's
identity and preserves foreign files.

For a native run, read the ecosystem's [acceptance entry](worker-context.md#ecosystem-differences)
for explicit inputs, writable roots, required outcomes and cleanup. Report a
preparation/cleanup failure separately while retaining the first product failure.
Process exit, successful setup or the model's success text cannot replace a
Gateway receipt and an independent tool artifact.

## Keep assertions independent of implementation layout

Build legal current state through current producers; compute expected outcomes
independently. A historical journal fixture must retain its original producer
bytes, including relevant feature combinations such as JSON ordering. Register
recovery readers and removal conditions in the
[compatibility registry](../../contracts/compatibility-support.v1.json).

Require the capability under test rather than a fixed total number of Agents.
Keep inventory completeness at the registry boundary: [registry](../../crates/integrations/src/agents/registry.rs),
[bundle source](../../assets/agent-profiles/current/profile-seed.json) and
[bundle producer](../../assets/release-facts/current/prepare-bundle.py) must agree.
Changed deterministic generators run twice; the second run must leave zero diff.

Use shared enable/adjust/disable and blocked-action recovery scenarios from the
[Agent feature guide](../../apps/desktop/src/features/agents/README.md). Native
protocol differences alone do not justify an extra verification control. Browser
fixtures, including standalone editors, select stable IDs rather than positional
slices or translated labels; require nonempty selection and all expected outcomes.

Before consolidating tests, list their unique failure conditions. Preserve crash,
stale-preview, cancellation, foreign-file, authorization, timeout and compatibility
cases even when happy paths overlap. For a broken gate, demonstrate that a realistic
bad case fails. For managed native overrides, retain a positive control showing
that the conflicting native setting would otherwise be consumed.

Protocol changes reuse saved-route and native-context journeys across Responses
and Messages. Assert actual request paths, trusted receipt identity, rotation and
independent restore. A removed-route witness uses its previously working protocol
and proves a typed authorization rejection plus zero upstream requests; an unknown
HTTP path or old restore fixture does not prove route revocation or native execution.

## Select and report validation

1. Run `python3 scripts/test-plan.py --base <exact-base>`; use `--integration` for
   branch integration. Review affected consumers and explain semantic adjustments.
2. Follow the selected commands and [CI definitions](../github-actions-validation.md).
   Use `scripts/validation.py` for configured hosts, exact committed Rust candidates,
   feature plan, phase and related runs. Compile affected native consumers early;
   featureless Desktop checks and zero-selected tests do not satisfy that gate.
3. Diagnose focused failures, freeze after affected checks pass, then complete the
   selected final checks once. Missing required cases remain unexecuted, not green.
4. Record revision, command, platform, cases, result and evidence location. Preserve
   reused evidence at its original SHA; assess the three slowest measured modules
   once without automatically refactoring the frozen candidate.

Code-map and local ownership-guide edits require link/owner review and the selected
contract checks, not new model calls. Runtime-embedded Markdown and shipped Skills
can change behavior and need their actual consumers. Instructions and compilation
remain distinct from executed business evidence.
