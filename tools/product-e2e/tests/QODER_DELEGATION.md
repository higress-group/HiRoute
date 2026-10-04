# Qoder model and task routing acceptance

The [Qoder target](qoder_delegation.rs) exercises five distinct product journeys.
It is explicitly opt-in; the existing Codex/Claude target keeps its original
installation requirements. A missing CLI, login, case or result is a failed or
not-executed prerequisite, never a passing skipped case.

## Run contract

Read this table before selecting the native context or starting a fixture. The
sections below give the precise inputs, ownership guards and required outcomes.

| Journey scope | Inputs and write scope | Restore/cleanup owner and evidence limit |
| --- | --- | --- |
| Main-Agent, Worker core, auxiliary routes and cancellation | [Explicit CLI, candidate and normally logged-in context](#select-the-native-context-explicitly). HiRoute storage, workspaces and submission receipts are private; only recorded test receipt Skills and the product-installed collaboration Skill enter the chosen native config. Existing user settings are preserved. | The [Qoder context leaf](../../../crates/daemon/tests/support/qoder_native_context.py) removes only its unchanged receipt files and owned empty Skill directory; the [collaboration journey](../../../crates/daemon/tests/support/qoder_collaboration_product.py) requests production Skill Restore before service stop. Login and native history remain native-client-owned. These are headless paths, not Desktop UI proof. |
| Persisted additional model routes | A [dedicated, normally logged-in MODEL context](#persisted-additional-model-routes), separate from daily HOME. The [model settings leaf](../../../crates/daemon/tests/support/qoder_model_fixture.py) writes a synthetic settings baseline; production Apply writes its managed provider and Skill. | The [model journey](../../../crates/daemon/tests/support/qoder_model_product.py) requests production Restore while daemon/sources remain alive; the leaf restores original settings only after its baseline/ownership guard passes. Neither owner deletes login or history. Ordinary saved-settings startup and bounded Live checks are separate evidence. |

For settings preparation, follow the existing [internal bootstrap boundary](../../../docs/code-map/testing.md#what-each-layer-proves).
Mac UI acceptance uses its separately owned managed Desktop/Pilot session;
that session's owner restores through the UI, retains evidence and stops only its
owned processes. Do not run a headless fixture against that live session's settings.
Required missing outcomes fail; [cheap oracle checks](#cheap-oracle-checks) do not
start the product or establish native support.

## Product journeys

| Product journey | Executable entry and decisive observations |
| --- | --- |
| Enable task routing and delegate from the real main Agent | [Main-Agent journey](../../../crates/daemon/tests/support/qoder_collaboration_product.py): discover the installation, perform the native capability check, apply collaboration-only settings, check the actual installed user Skill, then run native Qoder. Its `Skill` tool must load the exact user directory and body; its `Bash` tool executes the trusted public `hiroute worker plans` and `worker exec`. The separate Worker source requires real user/project Skill receipts and a disk artifact before returning success. Disable uses the current public restore reference and removes only the product-owned Skill. |
| Continue a Worker in its original context | [Shared core](../../../crates/daemon/tests/support/native_context_product.py): real user/project Skill calls and shell receipts; immediate Continue; daemon restart after replacing the Plan route and current native context; same exact native session, original route and prior correlated tool history; replay sends no request. Unsupported restricted policies fail before model/tool side effects. |
| Keep native tool summaries and automatic compaction on the managed route | [Auxiliary-route journey](../../../crates/daemon/tests/support/qoder_compaction_product.py): a real public Worker runs an owned read-only Bash receipt. A synthetic high-usage response triggers Qoder’s actual tool-summary and compaction requests. Each is classified by its native request content and independently issued receipt, then the main request must consume both receipts. Exact native history must contain an automatic compaction boundary and persisted summary; Continue loads that same compacted session. |
| Run neighboring tasks and cancel only one | [Shared boundary](../../../crates/daemon/tests/support/native_context_boundaries.py): simultaneous tasks have distinct sources, models and native IDs; cancellation stops the target's real child and heartbeat while its neighbor remains live and succeeds. Temporarily withholding only the exact test session's transcript must not produce a replacement session or prompt. |
| Select additional models from persisted user settings | [Persisted-model journey](../../../crates/daemon/tests/support/qoder_model_product.py): save two Plan routes, start ordinary Qoder with each saved selector, restart the daemon and invoke again, then perform the production Live check. Rotate the connection token, preserve independent Skill/model Restore, and refuse removing a route referenced by the native default. Uses a separate writable test context. |

Settings setup/preview/apply/check use the existing `Product.cli` notation for
production internal Local Control. They are not proof of the standalone public
settings CLI. The actual public CLI acceptance here is the native main Agent
executing `worker plans`, `worker exec`, and any necessary `worker wait`/`worker result`.
An exec response may report accepted or running after a bounded wait. The native
main Agent follows the response's exact wait cursor until a terminal state, then
reads the result for that same task/run. Acceptance alone cannot prove completion;
the independently checked Worker artifact remains required.

The main-Agent fixture has independent Responses endpoints and credentials for the
main Agent and the Worker. The main Agent's model is a temporary test input, not a
HiRoute-managed Qoder model connection. No Qoder model scan, import, subscription
or catalog call is needed. Native `Skill` launch receipts alone do not prove which
file loaded: the oracle also requires Qoder's actual directory/body expansion.
The private project has a conflicting default/core model route to a separate
foreign loopback trap. Worker success requires zero requests there and unchanged
project settings. The raw-native negative preflight must first show that this
conflict actually reaches the trap without the HiRoute overlay; schema inspection
alone is not that evidence.

Model self-report, prompt echo, an uncorrelated tool result, or a successful public
result without the independent Worker artifact cannot satisfy the journey.

## Select the native context explicitly

Set these values before the managed exact-candidate run:

- `HIROUTE_PRODUCT_CANDIDATE_SHA`: the full committed candidate SHA.
- `HIROUTE_WORKER_QODER_BINARY`: an absolute path to the selected native CLI.
- `HIROUTE_QODER_CONTEXT_HOME`: an existing, explicitly chosen native HOME.
- `HIROUTE_QODER_CONFIG_DIR`: its explicitly chosen existing Qoder config root.

The context must have completed Qoder's normal login. An empty root with a BYOK
fixture can still fail native session creation; do not copy authentication or
silently fall back to the daily HOME. The Product storage/runtime/workspace remain
separate and private. The fixture never deletes the selected native root, reads
an auth file, or rewrites user provider/model settings. It adds unique receipt
Skills and removes only its recorded unchanged files. Native authentication may
refresh its own state; unchanged user settings and ownership are the assertions,
not zero writes across all HOME.

Keep login, native Provider access, collaboration checks and completed delegation
as separate observations. On the tested macOS Qoder CLI 1.1.65 installation, a
normally logged-in context reported `allow_byok=0` yet reached the controlled
Provider with its selected model and credential, and passed the real Desktop
collaboration check. Neither an account label nor this status flag alone proves
support or refusal. Use actual native/product behavior and record the selected
context and CLI version; do not generalize this observation to other accounts or
versions. A no-login result from a dedicated acceptance HOME says nothing about
the user's separately logged-in daily HOME.

HiRoute's CLI submission receipts always use the Product's explicit private
`HIROUTE_WORKER_RECEIPT_DIR`, even when native HOME is borrowed. Repeated submissions
within one journey keep that directory so replay/conflict behavior remains real;
separate acceptance runs must not reuse the daily client's journal.

The real collaboration Skill must be installed by the production settings
operation at `<QODER_CONFIG_DIR>/skills/hiroute-collaboration/SKILL.md`. There is no
same-named project/plugin replacement. A pre-existing unmanaged different file is
a real conflict; do not overwrite it to obtain green. Borrowed identical Skills
must survive disabling collaboration. Retain the selected native context for
later normal login reuse; it is not owned by `Product.close()`.

If the collaboration leaf directory is absent, the fixture explicitly creates
only that empty directory with mode `0700` before Apply. It records the directory's
identity and permissions; the product still creates the actual `SKILL.md`. After
product Restore, cleanup removes only the same unchanged, empty fixture-owned
directory. Existing user directories are neither claimed nor chmodded. A permissive
`skills` parent remains unchanged; an existing unsuitable leaf remains subject to
the product's normal access guard.

When delegation fails after installation, the fixture first requests product
Restore through `agents restore preview/apply` while the service is still running,
then retains diagnostics and stops it. Configure continues to use
`agents connect preview/apply`; both paths require the authoritative status after Apply.
Cleanup errors are reported separately and cannot replace the first business
failure. A failed Restore never authorizes direct deletion of an installed Skill;
the unchanged-empty-directory guard still applies. Partial observations such as a
proved user Skill or an accepted failed Worker run do not complete the journey.

Use [test planner](../../../scripts/test-plan.py) and the configured
[validation runner](../../../scripts/validation.py):

```sh
cargo test --locked -p hiroute-product-e2e --test qoder_delegation -- --ignored --nocapture --test-threads=1
```

For an already attested exact-candidate build, the existing Python entries accept
`REPO FULL_SHA`, with `HIROUTE_VALIDATION_PRODUCT_BIN_DIR` selecting that build.
The core selects `HIROUTE_PRODUCT_WORKER_HARNESS=qoder`; the boundary takes
`--harness qoder`. Preserve build proof and binary digests. A random local binary
path is not an exact-candidate attestation.

## Persisted additional model routes

The fifth journey additionally requires `HIROUTE_QODER_MODEL_CONTEXT_HOME` and
`HIROUTE_QODER_MODEL_CONFIG_DIR`. Both must explicitly select a dedicated,
normally logged-in test context; the config must be inside that HOME. Missing
values fail even when the four Worker journeys have a usable borrowed context.
The fifth journey rejects the caller's daily HOME. Never copy or symlink login
credentials into it. Keep this persistent login root outside the temporary Product
root; Product cleanup has no ownership of authentication or native history.

The fixture seeds a synthetic user default, a separate native provider, unknown
fields and a later unrelated edit in that controlled settings file. Production
Preview/Apply adds only its owned provider. Ordinary native `--model provider/alias`
calls read the persisted user settings with the normal settings precedence; the
launcher supplies no `--settings`, `--setting-sources`, provider or credential
override. It requires a real source request, an independently generated output
nonce and one native terminal result with `subtype=success` and `is_error=false`.
The original collaboration journey's temporary direct model source cannot prove
this behavior.

Four required outcomes keep the evidence distinct:

- `agent.models.persisted-routes`: both saved aliases reach their own production
  Gateway routes, one works after daemon restart, and the explicit current-revision
  Live check actually calls both routes. Live uses its separately bounded native
  verifier; it cannot substitute for the ordinary startup calls.
- `agent.models.credential-rotation`: the old local connection token is rejected
  at the same `/_hiroute/qoder/v1/responses` entry used by ordinary native Qoder,
  without any source request, while a new ordinary Qoder
  process reads the updated persisted credential successfully. Tokens stay in
  memory/protected native settings and never enter the report.
- `agent.models.independent-restore`: Skill Restore leaves a usable model route;
  model Restore leaves the actual user Skill intact and verifiable. A route-list
  adjustment removes the old native choice, revokes its alias and keeps the retained
  route callable by ordinary Qoder. Native/default/unknown user fields and
  unrelated later edits survive these operations.
- `agent.models.default-reference-guard`: a synthetic user selection of a managed
  default blocks both its removal and full model Restore, without changing the
  settings, Skill, current authority or token. Keeping that alias is still legal.
  The fixture changes its own simulated default back before actual Restore.

Production Restore runs while the daemon and sources are alive. The fixture then
undoes its synthetic baseline only if no managed or drifted fields remain; a
failed Restore cannot authorize direct deletion of a provider or Skill. Existing
settings bytes/permissions are restored only after that ownership check. This
JSON baseline does not claim exhaustive JSONC, hook, MCP or external-project
precedence coverage; their focused integration tests and dedicated native cases
remain separate. Budget-reduction publication rejection belongs to the real-store
contract tests, rather than another repeated native journey.

The complete Qoder target now requires five Rust tests, five scenario reports and
14 required green outcomes. Reports from the original four-journey revision retain
their original SHA and do not establish support for persisted model routing.

## Native auxiliary-route proof

The core Skill/Continue journey deliberately keeps a 32,768-token context and a
4,096-token output budget. Every Qoder source request must carry that exact output
limit, and an unexpected automatic compaction fails immediately instead of being
mistaken for another main-model tool request. Main-Agent and cancellation journeys
use 100,000-token contexts to keep their assertions focused. These reports record
the selected context budget; success at 100,000 does not establish small-window
support. The separate core gate must also pass.

The source observes requests after Gateway processing, which can cap output
tokens. Its 4,096 assertion therefore proves the upstream limit, not the raw
native request limit by itself. The native metadata projection is separately
covered by renderer/profile contracts and the pinned raw-ACP metadata-only versus
CLI-only controls; the small-window journey catches the original zero-threshold
compaction failure through actual behavior. Keep these evidence levels distinct.

`worker.context.tool-summary-route` and `worker.context.compaction-route` share
one fourth journey. Only its owned project settings enable context management and
Bash output summarization. The source declares a 100,000-token context; the first
real Responses tool reply reports 90,000 input tokens. The native client performs
the summarization and compaction itself; the fixture does not write a native
summary or invoke an internal HiRoute execution port. This leaves the original
immediate-Continue journey unchanged.

The source sees the Gateway's upstream model and source credential. Successful
auxiliary requests therefore prove traversal of the production Worker/Gateway
path; this report does not claim to capture the private run token on that leg.
The Gateway's existing per-run authorization contracts protect the frozen Plan
alias and credential. An auxiliary request using a wrong source model is rejected,
and a foreign project endpoint request makes preservation fail. Ordinary main
request success without both auxiliary requests, their receipt chain and the
actual native compaction boundary cannot make this journey green.

The Qoder 1.1.65 native signatures and exact-history witness are narrow fixture
facts. Keep the raw-native positive and isolated wrong-summary/wrong-compact
mutation evidence alongside the product report: it demonstrates that both native
purpose settings affect real requests, but does not replace this production-entry run. Other native auxiliary uses remain separately
bounded or unverified; this case does not claim universal model routing coverage.

## What remains separate

The six Worker outcome definitions and retention/read/credential consumer gates
remain in [Worker native acceptance](WORKER_NATIVE_CONTEXT.md). These journeys do
not prove every native hook, plugin, auxiliary model or permission mode. The
Worker's supported tool set excludes unproven native nested-Agent tools. Strong
conflicting *user* provider/purpose/hook cases require a normally logged-in test
context whose synthetic settings are owned by the test; never change daily
settings to manufacture that negative case.

The [Qoder leaf](../../../crates/daemon/tests/support/qoder_native_context.py)
reads the task's binding only in the current Product's owned metadata. Its
`history_directory` function isolates the Qoder 1.1.65 short-workspace filename
assumption used to locate one exact test session. Production retains opaque
native history and confirms exact `session/load`; it does not consume that
filename algorithm. The missing-history case restores the original test file,
refuses to overwrite an unexpected replacement, and never searches daily history
by message contents. Borrowed native history is not fixture cleanup material.

A newly cancelled task need not have a continuation binding. For its history
preservation assertion only, `cancelled_history` reads the exact task/run from this
Product's own runtime database in read-only mode, verifies cancellation and complete
cleanup, and uses that recorded native ID with the same bounded history formula.
This registered current-storage fixture reader neither searches another task nor
grants Continue. The successful neighbor and all Continue cases still require the
real `native-session-binding.json` witness.

macOS acceptance must additionally use the real managed Desktop/Pilot entry with
the explicitly logged-in test HOME: install/select the single CLI, bind Qoder to a
Plan, enable collaboration, observe a completed task and cancel real native work.
A Python `prepared` response or API setup is not Desktop acceptance. Record
platform, product/fixture revisions and every selected case's outcome separately.

The main-Agent and persisted-model journeys share
[Agent product support](../../../crates/daemon/tests/support/agent_product_support.py)
for process deadlines/private output and authoritative settings transactions. Qoder
leaves retain launch arguments, native settings ownership and result/receipt checks.
The same upstream counter rejects false credential-revocation proofs without
per-journey handler replacement. A new ecosystem should reuse these mechanics;
it still needs independent assertions for its native contract.

## Cheap oracle checks

```sh
python3 scripts/test-agent-product-support.py
python3 scripts/test-qoder-product.py
python3 scripts/test-native-context-product.py
python3 scripts/test-native-context-boundaries.py
```

These checks reject missing explicit context, shadow user Skills, wrong role
endpoints, fabricated/uncorrelated completion, changed borrowed files and missing
disk artifacts. They also retain the existing SSE/ownership/boundary assertions.
They start neither a native Agent nor HiRoute and cannot establish product support.
