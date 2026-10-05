# Pi product acceptance

[Product test map](../../../docs/code-map/testing.md) · [Worker owners](../../../docs/code-map/worker-context.md)

Pi reuses the product's settings, publication, Worker lifecycle and Gateway.
Its native leaves implement official npm SDK discovery, effective static API
sources, resource loading and one exact task transcript. There is no subscription
or OAuth import, separate user verification action, or native extension/MCP claim.

## Run contract

Use the Linux or Mac test environment described in the
[product test map](../../../docs/code-map/testing.md).
[Hosted CI](../../../docs/github-actions-validation.md) does not execute these
installed native clients. Select an exact committed
candidate and an explicitly installed official `@earendil-works/pi-coding-agent`
CLI with Node >=22.19. The reproducible reference fixture uses 1.0.2; compatibility
runs may select another exact release, including 1.0.1, without changing the journeys.
This acceptance pin is not production admission: the
[operation-specific contract](../../../docs/code-map/worker-context.md#pi-compatibility-is-a-capability-contract)
checks actual required interfaces and local behavior. The SDK must be exported by
the selected CLI's own package; an unrelated global SDK is not a substitute.
Set absolute `HIROUTE_WORKER_PI_BINARY`, `HIROUTE_WORKER_NODE` and full
`HIROUTE_PRODUCT_CANDIDATE_SHA`. Missing inputs fail instead of skipping silently.

```sh
cargo test --locked -p hiroute-product-e2e --test pi_delegation -- \
  --ignored --nocapture --test-threads=1
```

This starts production `hirouted`, public `hiroute`, Gateway and official Pi.
Only the model upstreams are deterministic. The fixture creates a private native
HOME/config/project for the scenario; production Workers borrow the calling
instance's effective HOME and `PI_CODING_AGENT_DIR`, never a replacement task HOME.
User, project and already installed package Skills retain their native precedence.

## Product outcomes and their witnesses

| User outcome | Shared journey / Pi difference | Required witness |
| --- | --- | --- |
| Run and Continue the original delegated task after restart and Plan replacement | [Native context](../../../crates/daemon/tests/support/native_context_product.py); [Pi resources/history](../../../crates/daemon/tests/support/pi_native_context.py) | Actual user/project/package Skill read and shell receipts; exact native session, old frozen route, no replay POST; 16,384/4,096 context/output boundary |
| Concurrent routes stay separate; cancellation stops only its native child | [Native boundaries](../../../crates/daemon/tests/support/native_context_boundaries.py) | Two actual route/tool starts, owned child stop and surviving neighbor; distinct native IDs |
| Missing or corrupt history never starts a replacement conversation | Same boundary journey | Missing file, truncated JSON, valid JSON missing message content, unsupported session version; zero model POSTs and no native repair for each independent task |
| Saved additional model routes work without CLI provider/credential overrides | [Additional models](../../../crates/daemon/tests/support/additional_model_product.py), [native file/result leaf](../../../crates/daemon/tests/support/additional_model_fixture.py) | Actual ordinary Pi output/session, restart, credential rotation, removed-route refusal, preserved defaults/unknown fields, independent model/Skill restoration |
| A main Agent reads its installed user collaboration Skill and delegates | [Collaboration](../../../crates/daemon/tests/support/collaboration_product.py), [role oracle](../../../crates/daemon/tests/support/collaboration_fixture.py) | Actual installed Skill body, public plans/exec/wait/result, distinct main/Worker sources, successful Worker output and independent disk artifact; owned Skill restoration |
| Native automatic compaction and Continue retain the frozen route | [Native compaction](../../../crates/daemon/tests/support/native_compaction_product.py) | Actual tool receipt in native summary request, independently returned summary in task transcript and subsequent prompt, exact same session, no foreign route |
| Import an effective static provider/model and use the saved source in a route | [Static discovery](../../../crates/daemon/tests/support/pi_discovery_product.py) | BOM/JSONC configuration, model endpoint override, saved credential precedence; public scan/prepare/save/restart with unchanged native files; stale source save refused; zero model POSTs during passive import, then exactly one ordinary native Pi request through the saved source and published Plan; restore owned settings |

Ordinary tests compile these ignored targets without executing them. The test
planner registers all six required native tests explicitly. Cheap shared oracle
checks are in `scripts/test-agent-product-support.py`,
`scripts/test-qoder-product.py`, `scripts/test-native-context-product.py` and
`scripts/test-native-context-boundaries.py`; they cannot prove native support.
Keep security/ownership assertions separate when a product journey cannot cover
that failure branch precisely.

## Ownership and evidence limits

The Worker uses an empty in-memory credential store so native authentication
helpers cannot execute during runtime initialization. Installed package resources
are resolved offline; missing packages are not installed. Extensions are disabled.
Native model/default/auth files are borrowed and must remain byte-for-byte intact.
Only the task's `native-pi.jsonl` is owned. Validate its current v3 shape before
native open, because the SDK can otherwise skip malformed rows or migrate history.
SDK release and transcript format compatibility are separate. Missing native open
only blocks Continue; an unknown SDK writer or existing transcript format blocks
Worker execution/restoration without changing unrelated model or task-route setup.
Cheap local interface/ownership regressions are in
`crates/integrations/src/agents/pi_sdk_contract.test.mjs`; synthetic release labels
prove admission behavior, not actual future-release compatibility.

Main-Agent model settings own only one provider member in `models.json` and the
local grant inside that private file. Pi's separate user `settings.json` default
is a transaction dependency checked again at activation/retry. Project defaults
are native project configuration outside this global default ownership contract.
A helper or OAuth key can be reported but never imported by executing it.

The stored-source journey has three required outcomes: passive import, stale
source refusal, and saved-source route use. The first two make no model request.
The last publishes the imported binding, saves an additional Pi route and invokes
ordinary Pi without endpoint or credential overrides; an independent source
receipt proves the one resulting model request. This guards the whole saved
source → capability qualification → Plan → native settings path, rather than
treating a successful scan or save as usable routing.

The native journeys prove routing, tools, context and restoration using
deterministic sources. They do not prove arbitrary
live-provider behavior, native OAuth, restrictions not implemented by the SDK
bridge, Windows, or every plugin/extension. Record platform, candidate SHA,
fixture revision, selected cases and verdicts together.

macOS additionally requires the current real Desktop/Pilot entry: scan/import a
source, select Pi CLI and Node, publish a Pi Worker Plan, enable/adjust/disable
model and task routes through the page, invoke the saved route, inspect a task and
Continue/cancel actual native work. API setup and a successful toast cannot replace
those UI actions. Continue is a public Worker CLI action followed by observing
the updated task/run in Desktop; Desktop has no separate Continue control for any
ecosystem. Cancellation must use Desktop's task detail and confirmation, followed
by real child-stop and live-neighbor witnesses. Leave the validated client open
only when handing it to the user.
