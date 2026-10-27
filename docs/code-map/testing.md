# Tests as product documentation

Native acceptance uses [isolated scopes](../../scripts/native-agent-scope.py) and
the [macOS session supervisor](../../scripts/native-agent-supervisor.py). Its live
anchor preserves ownership after a client exits, and its private lease closes on
guardian death. Native exit and cleanup outcomes remain separate. It covers the
anchored session; detached sessions are not an inferred cleanup authority.
An optional private command pipe dispatches native SIGINT through the live anchor;
its EOF revokes the session. Dispatch is separate from native Stop acknowledgement
and Gateway cancellation evidence. Interrupting the guardian requests cleanup only.

[Code map](README.md) · [Desktop scenarios](../../apps/desktop/tests/README.md) ·
[Agent/Worker scenarios](worker-context.md#representative-scenarios)

[Execution locations](../validation-routing.md) owns caller HOME routing,
per-origin SSH workbenches, shared compiler capacity and separate candidate/driver
revision identities. A runner setup failure is not a product test result. Ordinary local and SSH runs
save results and [private evidence](../../scripts/validation-report.py) before
reclaiming their managed checkout/TMPDIR, including failed or blocked runs.
Explicit retention covers diagnosis and consumers that still need build artifacts;
[local lifecycle regressions](../../scripts/test-local-rust.py) and
[SSH worker regressions](../../scripts/test-remote-rust.py) protect both paths.
[Scheduler preparation regressions](../../scripts/test-validation-schedule.py)
verify complete smoke receipt export without preserving executable snapshots;
the shared exporter keeps Pilot's existing target-artifact consumer paths.
The [console artifact owner](../../scripts/console-assets.py) checks selected SSH
installed-package prerequisites before Rust execution and verifies the same-candidate
frontend from an execution-host staging directory under the runner's checkout lease.
Standard file tools supply that directory; the runner does not transport frontend bytes.
The [SSH checkout owner](../../scripts/remote-rust.py) shares retained-state admission
with recovery-command selection; [continuation regressions](../../scripts/test-console-assets.py)
cover repeated source advances, preparation blocks and execution of the generated retry.
Windows development execution is owned by [windows-dev.py](../../scripts/windows-dev.py)
and its [native executor](../../scripts/windows-dev-host.py), reached through
`validation.py windows`. HOME configuration owns machine/session/client locations;
the project owns artifact admission, exact-case execution and revision-bound evidence.
Its four temporary-directory native artifact ACL/recovery cases can execute without
replacing an existing installation; installed-product cases still require a fresh profile.
Reusable Desktop Pilot build admission belongs to
[pilot-builds.py](../../scripts/pilot-builds.py); its
[lifecycle regressions](../../scripts/test-pilot-builds.py) cover manifest-content
identity, frozen snapshots and checks before reuse/start using fake build/launch tools.
The separate `--real-consumer` case is required by the affected CI selection and
checks the production Rust build-script and embedding declaration; local default
checks never compile or execute those temporary Rust programs.
The optional native App entry belongs to
[pilot-native-app.py](../../scripts/pilot-native-app.py), with
[fixture process regressions](../../scripts/test-pilot-native-app.py) for ordinary
reopen, profile retention, artifact refusal and lease retirement. Actual macOS
LaunchServices/native CU and daemon/CLI acceptance remain separate from those
local entry tests; follow the [App lifecycle guide](../desktop-tauri-pilot.md#managed-native-app-opening-and-reopening).
[Orphaned data preparation](../../scripts/windows-dev-profile.py) preserves complete
backups after explicit user authorization; it does not establish a product scenario.
[Native Desktop session actions](../../scripts/windows-desktop-session.py) admit
the installed NSIS member and collect actual Windows UI Automation/screenshot
evidence. Public CLI readback is distinguished from a UI action, and a cross-build
replacement is distinguished from a newer-version update.
The [Windows uninstall restore plan](../../crates/cli/src/service/restore_plan.rs)
keeps Codex root collaboration separate from its model-profile projection; its
portable unit cases cover owner selection and refusal, while the actual NSIS
uninstall remains the native evidence for restoring Agent files.
The [native registration publication regression](../../crates/platform/src/windows/shell_registration.rs)
uses a unique disposable registry key to reproduce inherited system-file permissions,
verify private publication and refuse changed retained bytes or a failed export.
It verifies a generic recovery boundary; it does not establish a supported
historical Windows release or a complete ordinary-user restoration journey.
The [standalone journal cases](../../crates/host-runtime/src/standalone_install/journal/tests.rs)
also prove that cutover facts stop rollback before manager effects, forward
reapplication registers only the current definition and preserves the original
backups, and changed native entries are refused before any manager call.
Current-installation failure pages and finite installer waits have bounded
component and production-style checks. Development cross-build simulations are
not supported historical Windows upgrade evidence. The general complete-source
recovery design remains; extending the development historical matrix is separate.

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

Native reasoning replay is covered by the accepted-response
[boundary regression](../../crates/gateway/src/agent_turn_history/boundary/reasoning_tests.rs),
the [plaintext content and cancelled-turn recovery cases](../../crates/gateway/src/agent_turn_history/boundary/plaintext_content_tests.rs),
the [encrypted replay cases](../../crates/gateway/src/agent_turn_history/boundary/encrypted_tests.rs),
the [unsigned Messages tool-stage cases](../../crates/gateway/src/agent_turn_history/boundary/messages_tests.rs)
and [current-window turn/continuation cases](../../crates/gateway/src/core_runtime/decision_history/window_event_tests.rs).
[Stage closure cases](../../crates/gateway/src/agent_turn_history/stage_closure_tests.rs)
distinguish captured membership, provider outcomes and closure proof; the
[native retry listener case](../../crates/gateway/src/core_runtime/response_diagnostics_tests/native_retry.rs)
exercises upstream failure, same-activation retry, subsequent classification,
model-group switching and assessment membership through the real Gateway;
[retry boundary cases](../../crates/gateway/src/agent_turn_history/native_retry_tests.rs)
reject ambiguous siblings, changed input/setup, capture gaps and accepted output. The
[request JSON observation cases](../../crates/gateway/src/observation/content/request_json_replay_tests.rs)
verify Replay expansion, integrity and bounded storage without leaking locators.
The observation [timeline cases](../../crates/observation/src/store/tests/plan_quality/routing_timeline.rs)
verify that explicit maintenance-purpose evidence excludes a user turn without
removing the request, while unmarked retained records remain visible. These
component assertions do not replace the real DSH Web/HiRoute acceptance.
[Quality snapshot cases](../../crates/observation/src/query_v2/plan_quality_snapshot_tests.rs)
exercise concurrent production ingestion, deletion fences and strict pagination
without making ordinary appends invalidate a fresh summary/detail read.
The [native encrypted listener cases](../../tools/e2e-harness/tests/p0_gateway_runtime/native_encrypted_continuation.rs)
and [cross-protocol continuation cases](../../tools/e2e-harness/tests/p0_gateway_runtime/cross_protocol_encrypted_continuation.rs)
exercise accepted-event delivery, repeated tool requests, complete assessment membership
and retained-content policy through the production Gateway listener. Protocol-owned
[projection tests](../../crates/gateway/src/adapters/response/client_projection_tests.rs)
distinguish intentionally omitted content from deferred client output.

Start with the representative cases in the [capability map](README.md). A file
named `product`, a golden containing `green`, or a zero process exit does not prove
execution. In `tools/product-e2e`, [transaction recovery](../../tools/product-e2e/tests/transaction_recovery.rs)
contains expected-red composition, [adapter contracts](../../tools/product-e2e/tests/gateway_adapter_contract.rs)
are component evidence, and [oracle tests](../../tools/product-e2e/tests/product_oracle.rs)
validate the oracle. The real installed management loop is in
[publication_process.rs](../../crates/daemon/tests/publication_process.rs).
Its [https-feed lifecycle](../../crates/daemon/tests/support/model_feed_product.py)
checks registered-source admission, published sessions, new Workers and exact Continue
through withdrawal, cache removal, restart and later embedded absorption. The daemon's
`integration-test-hooks` feature supplies only the external feed/inventory fixtures;
Gateway calls reuse the existing TLS E2E dial map. The Gateway's [registered-feed TLS case](../../crates/gateway/src/test_control/registered_feed_tls.rs) connects its production transport directly to that same external OpenSSL fixture, exposing handshake failures before the installed lifecycle. Normal builds have no feed fixture
trust root or transport override. The test does not establish live production signing.

## Reuse the fixture at the owning boundary

| Need | Reuse / preserve |
| --- | --- |
| Prepare bounded control smoke prerequisites | [Control storage preparation](../../tools/product-e2e/src/smoke/control_preparation.rs) runs the same verified daemon on fresh storage before ordinary control/CLI watchdogs; [clock boundary tests](../../tools/product-e2e/src/smoke/control_preparation_tests.rs) cover admission and timing, while [smoke CLI](../../tools/product-e2e/tests/smoke_cli.rs) checks actual initialization and business steps |
| Diagnose a real Gateway listener assertion | [RuntimeFixture](../../tools/e2e-harness/tests/runtime_support/mod.rs): use the diagnostics-enabled REST-classified variant when boot/request evidence is needed; the Gateway's [test-control feature](../../crates/gateway/Cargo.toml) binds the separate SUT to the shipped daemon's JSON map-order feature. Unexpected panics retain the private fixture root for [failure diagnosis](../diagnostic-troubleshooting.md#gateway-listener-fixture-failures) |
| Check provider-state cleanup authority | [ContextHold cleanup](../../tools/e2e-harness/tests/p0_gateway_context_hold/cleanup.rs): free-first successful/failed cleanup and bounded prefix reuse; a classified contextual history cannot authorize deleting opaque state, while later accepted execution permits ordinary routing recovery |
| Prepare a daemon's current sources, grants and plans | [Publication Product](../../crates/daemon/tests/support/publication_product.py): its `Product.cli` is internal Local Control notation, not the standalone CLI; protected input belongs to that fixture's daemon |
| Save an explicit HTTP model source or correct its target | [Custom HTTP journey](../../crates/daemon/tests/support/custom_http_product.py): real control/save/restart/recheck with synthetic literal/hostname targets and no directory; [connection form browser checks](../../apps/desktop/tests/v3/visual/connection-flow-active.mjs) separately cover plaintext notices, localized errors and retained input |
| Configure a real native Agent | [Agent product support](../../crates/daemon/tests/support/agent_product_support.py): shared Preview → Apply → status and bounded process capture; native argv/ownership/result interpretation remain in the leaf |
| Prove saved model configuration | [Additional-model journey](../../crates/daemon/tests/support/additional_model_product.py), [Windows Pi/DSH journey](../../crates/daemon/tests/support/windows_user_models.rs), [Windows Qoder account journey](../../crates/daemon/tests/support/windows_qoder_user_models.rs): ordinary native startup reads saved settings without provider/endpoint/credential overrides; `agent-user-models` and protected `account-qoder-user-models` remain separate from Worker profiles |
| Prove ordinary plugin activation and optional history | [Native history journey](../../crates/daemon/tests/support/additional_native_history_product.py): Pi/OpenCode auto-discovery, real multi-turn decision history, missing-plugin output, capture gaps, Gateway restart and restored native startup; Worker pure-mode success cannot replace these assertions |
| Observe native routing and rejected attempts | [NativeContextUpstream](../../crates/daemon/tests/support/native_context_fixture.py): count every model POST, including rejected endpoints/models/credentials; keep the ledger for positive route/history assertions |
| Prove exact Continue, concurrent routes and cancellation | [Native context](../../crates/daemon/tests/support/native_context_product.py), [boundary journey](../../crates/daemon/tests/support/native_context_boundaries.py), [boundary oracle guide](../../crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md) |
| Prove restoration of old durable state | [Frozen producer fixtures](../../crates/local-storage/src/migrations/fixtures/README.md), [crash-boundary tests](../../crates/local-storage/src/migrations/upgrade_batch_recovery_tests.rs): old bytes through the current consumer, not a new serializer pretending to be history |
| Upgrade connected Agents from a released package | [Released grant upgrade](../../crates/daemon/tests/support/released_grant_upgrade.py): pass the repository and checksum-pinned v0.3.1 Linux archive, with current binaries in `HIROUTE_VALIDATION_PRODUCT_BIN_DIR`; released transactions create both connections, current startup preserves grants/native files, and real requests plus public session queries prove attribution across restart. Native probes and the upstream are bounded fixtures. |
| Recover interrupted Agent settings before the service receipt | [Publication process](../../crates/daemon/tests/publication_process.rs) runs [Agent recovery](../../crates/daemon/tests/support/publication_agent_recovery.py) at `after_target` and `after_active`; startup completes the journal before Apply replay, authenticated requests acquire the Agent, user native files remain byte-identical, and the recovered managed profile is stable across restart. |
| Drain before upgrade replacement | [Gateway upgrade journey](../../crates/daemon/tests/support/upgrade_requests.py) pauses admission and observes Status until both call/task counts are zero before replacing the process. [Harness protocol checks](../../scripts/test-upgrade-drain-product.py) retain response-tail, task-tail, deadline and unexpected-resume failures; receiving an HTTP response alone is not a server-side drain acknowledgement. |
| Keep session reads responsive during discovery, catalog and body reads | [Product-session transport barriers](../../crates/product-session/src/commands/agent_snapshot/tests.rs) prove observation reads and cursor updates proceed before a delayed Agent scan, trial choice/model read, catalog read or body read completes. Live hidden clients may read; revocation during transport rejects publication. Narrow trial reads retain the same replaced Session/epoch, changed revisions/publication/recovery/authority and cancellation checks. The old locked path is a negative control. |
| Reuse verified Agent history | [History cache regressions](../../crates/local-storage/src/control/agent_history_cache_tests.rs) cover large cold/warm histories, selected-row identity and strict versus best-effort reads. [Snapshot reuse](../../crates/local-storage/src/control/agent_history_epoch_tests.rs) checks local/schema/external invalidation, commits during an established snapshot, rollback/savepoints and failed caller reads. Native response latency remains a separate measurement. |
| Reuse completed provenance without caching authority | [Completed decoder checks](../../crates/local-storage/src/control/completed_operation_cache_tests.rs) cover repeated decode, mutable returned copies, external payload damage, exact current receipts, reopen and savepoint rollback. [Publication marker recovery](../../crates/daemon/src/control/runtime/effects/tests/publication_validation.rs) checks warm reuse plus changed identities/content and legacy normalization. |
| Reconstruct a full-content publication once | [Cold reconstruction regressions](../../crates/domain/src/operation/routing_content_tests.rs) count authentication through the real stored-input reader, retain the separate control join and reject changed publication bytes. Startup and publish latency remain native product measurements. |
| Locate recovery material while the service is down | [Shared presentation](../../apps/desktop/tests/storage-recovery.test.mjs), [production-style geometry](../../apps/desktop/tests/v3/visual/storage-recovery-scenarios.mjs), [offline CLI formatter](../../crates/cli/src/service.rs): verified batch and source location, platform folder route and guide names; unsafe/absent locations remain unverified. Native opening/restoration needs its own installed evidence. |
| Preserve native probe failure classification | [Pi probe tests](../../crates/integrations/src/agents/pi_runtime_tests.rs), [immutable launcher](../../crates/integrations/tests/fixtures/pi-node-launcher.sh): mutable test bodies are interpreter inputs; executable availability and version/output failures have separate assertions |
| Keep Claude authentication checks inside owned roots | [Native probe](../../crates/integrations/src/agents/native_claude_ingress_probe.rs), [launch environment regression](../../crates/integrations/src/agents/native_claude_probe_environment_tests.rs): the actual child receives owned HOME/configuration/workspace/temporary selectors; ambient overrides, unauthenticated completion and failed native exit cannot prove authentication |
| Exercise actual Desktop controls | [Browser scenario map](../../apps/desktop/tests/README.md) and native acceptance: select stable scenario IDs and require every declared outcome |

The publication fixture leases its listener address across daemon restarts; do
not substitute a released `bind(0)` probe. Its protected-input channel cannot be
attached to an existing Desktop. Prepare that instance first, then use real UI
mutations. Main-Agent delegation must separately invoke the actual public CLI.
The isolated subscription startup test uses the [listener port lease](../../crates/daemon/src/test_support/listener_port.rs)
outside the host's outbound ephemeral range; the parent keeps that lease through
the isolated child's Gateway shutdown. On Unix it shares the publication fixture's
per-user address-lock namespace across worktrees and runner temporary directories.

The [Gateway runtime fixture](../../tools/e2e-harness/tests/runtime_support/listeners.rs)
holds its product and optional control listeners together during preparation,
then releases both immediately before starting the child. Releasing a `bind(0)`
probe before choosing the other address can assign both roles the same port.

The [shared product session](../../crates/product-session/src) owns confirmation,
exact-digest authority and command orchestration tests. Its permit-drop and
broker-abort cases verify that cancelled callers can retry, while stale permits
cannot cancel a newer prompt or another context. The
[console composition cases](../../crates/daemon/src/console/tests.rs) exercise the
real role-all dispatcher while preserving the external Local Control Released
gate. [Authentication cases](../../crates/daemon/src/console/auth/tests.rs) cover
launch replay, revocation, hash-only storage and rate limits.
The [listener lifecycle cases](../../crates/daemon/src/console/listener_tests.rs)
verify off/failed minting without binds, actual network origins, the pre-established
shell loopback path and desired configuration pending restart. Unix installer
regressions cover explicit exposure validation, omitted-option preservation and
configuration rollback. Network launch authentication tests cover exact origin/port,
remote local-code refusal, expiry, replay and credential redaction.
The [console transport cases](../../crates/daemon/src/console/transport/tests.rs)
exercise independent peer admission, stalled-write permit release, progressing
responses, bounded idle headers and confirmations on aged keep-alive connections
through the production Hyper connection owner and shared confirmation broker.
Browser scenarios
run through both host transports; their production CSS screenshots include the
console and Windows startup/update pages, without establishing native effects.
The [Home startup scenarios](../../apps/desktop/tests/v3/browser/home-startup-scenarios.mjs)
exercise delayed snapshot and startup-status results through the production
[Home reader](../../apps/desktop/src/product/use-desktop-home.ts); stale replies
cannot replace newer service evidence or clear the current failure.

The [Rust installation journal](../../crates/host-runtime/src/standalone_install/journal/tests.rs)
covers current-schema interruption recovery. Package tests verify x64 and ARM64
inventories and reject a target/executable mismatch. There is no released Windows
Python producer to preserve. The
[frozen release reader](../../crates/host-runtime/tests/release_catalog_compatibility.rs)
proves that the V2 projection remains consumable by supported macOS Desktop.
Windows shell origin/handoff tests live in
[the shell library](../../apps/shell/src-tauri/src/lib.rs); installed NSIS, WebView,
Task and upgrade effects require the separate
[native acceptance inventory](../../scripts/windows-desktop-cases.json), bound to
the actual standard-user OS, candidate and installer hash.
The [shell transaction regression](../../apps/shell/src-tauri/src/runtime/install_transaction.rs)
uses real Windows file sharing to verify backup handles close before hash admission;
the native installer candidate build runs it before exporting the measured bundle.
The [bounded NSIS journey](../../crates/daemon/tests/windows_desktop.rs) uses the
verified development bundle and configured existing standard session to check
silent defaults, repeat install, console/Gateway readiness and uninstall with
data retained. It verifies the installed shell against the NSIS member image hash
after install, repeat and passive update. Its result does not establish WebView, Agent restore or upgrade
recovery acceptance; see the [execution entry](../validation-routing.md#windows-iteration-and-release-validation).
The [Windows package inspector](../../scripts/package-windows-shell.py) extracts the
actual NSIS shell member with 7-Zip for its SHA-256 and CRT audit. The native case
compares that measurement with installed bytes; the workspace executable is not
the artifact identity source.

Keep borrowed user resources, writable native acceptance configuration, fresh
product storage and per-run materials separate. Each product instance needs a
private runtime/storage namespace. Worker task submissions recover with the original
key and request, with title/detail lookup only as an aid. Agent collaboration
configuration separately freezes credential-free Preview/Apply bytes in the
[CLI receipt owner](../../crates/cli/src/collaboration/receipt.rs); Windows publishes
them without replacement and checks native ownership, link count and ancestors
before replay. Its [native regressions](../../crates/cli/src/collaboration/windows_receipt_tests.rs)
do not substitute for an installed Agent invoking the public CLI.
[CLI submission regressions](../../crates/cli/tests/worker_submission.rs) cover denied local
writes and uncertain responses; durable admission and native Main journeys remain separate.
Expose the selected CLI and Pi's selected Node consistently
to main-Agent checks and Worker launch. Never repair permissions on pre-existing
user directories to satisfy a fixture; cleanup verifies the owned object's
identity and preserves foreign files.

For a native run, read the ecosystem's [acceptance entry](worker-context.md#ecosystem-differences)
for explicit inputs, writable roots, required outcomes and cleanup. Report a
preparation/cleanup failure separately while retaining the first product failure.
Process exit, successful setup or the model's success text cannot replace a
Gateway receipt and an independent tool artifact.
The [native preparation and recovery contract](../test-selection.md#native-preparation-and-recovery-evidence)
keeps actual preparation before model admission, cancellation causality, uncertain
Apply recovery and independent cleanup results explicit.

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

[Codex subscription replay](../../crates/gateway/src/adapters/tests/codex_replay.rs)
checks endpoint-specific output-status normalization while preserving the original
history and public API fields, including nullable reasoning states and Replay.
[Completed commentary history](../../tools/e2e-harness/tests/p0_gateway_runtime/decision_branches/responses_chat_history.rs)
exercises candidate-local [history projection](../../crates/gateway/src/adapters/request/history.rs),
independent live rendering, model-group changes, concurrent requests and restart
assessment through the listener. [Projection diagnostics](../../crates/diagnostics/src/event/request_projection.rs)
describe preflight or materialization without claiming provider execution.
[Response lifecycle tests](../../crates/gateway/src/adapters/response/client_stream_tests.rs)
check response-scoped synthetic IDs across incremental, buffered and terminal output.
Protocol changes reuse saved-route and native-context journeys across Responses
and Messages. Assert actual request paths, trusted receipt identity, rotation and
independent restore. A removed-route witness uses its previously working protocol
and proves a typed authorization rejection plus zero upstream requests; an unknown
HTTP path or old restore fixture does not prove route revocation or native execution.

## Select and report validation

Use [HiRoute validation](../../.agents/skills/hiroute-validation/SKILL.md) for the
current selection, execution, failure or handoff operation. Its routes lead to
the owning guides; branch convergence remains a separate integration responsibility.
Hosted prerequisite installation belongs to
[`ci-prerequisites.py`](../../scripts/ci-prerequisites.py), invoked by the shared
[`ci-setup` action](../../.github/actions/ci-setup/action.yml).
[`test-ci-setup.py`](../../scripts/test-ci-setup.py) covers Ubuntu source selection
and bounded failure handling; the real hosted matrix verifies package installation.

Hosted shard inventory and harness policy belong to
[ci-shards.py](../../scripts/ci-shards.py); [ci-run.py](../../scripts/ci-run.py)
captures launch concurrency settings for the [shared report](../../scripts/validation-report.py).
See [the hosted guide](../github-actions-validation.md#rust-test-concurrency) for
the explicit process-target exception and unknown effective concurrency.
The [host adapter](../../scripts/validation-host.py) derives admitted local/Desktop
run help from the [local runner parser](../../scripts/local-rust.py), while retaining
configured repository/jobs and Desktop origin-fetch admission.

1. Run `python3 scripts/test-plan.py --base <exact-base>`; use `--integration` for
   branch integration. Review affected consumers and explain semantic adjustments.
   Observation port changes select the [current boundary golden check](../../tools/product-e2e/tests/local_observation.rs)
   early; retained historical observations remain separate compatibility inputs.
2. Follow the selected commands and [CI definitions](../github-actions-validation.md).
   Use `scripts/validation.py` for configured hosts, exact committed Rust candidates,
   feature plan, phase and related runs. Compile affected native consumers early;
   featureless Desktop checks and zero-selected tests do not satisfy that gate.
3. Diagnose focused failures, freeze after affected checks pass, then complete the
   selected final checks once. Missing required cases remain unexecuted, not green.
4. Record revision, command, platform, cases, result and evidence location. Preserve
   reused evidence at its original SHA; assess the three slowest measured modules
   once without automatically refactoring the frozen candidate.

Final backend DAG admission and preparation are owned by
[validation-schedule.py](../../scripts/validation-schedule.py), with
[regressions](../../scripts/test-validation-schedule.py). CPA bridge process fixtures
run in an isolated lane before other listeners; Gateway runtime consumers wait for
prepared product roles. [Smoke preparation](../../tools/product-e2e/src/smoke/preparation.rs)
and [Gateway preparation](../../tools/e2e-harness/src/p0/production/candidate.rs) bind
the exact receipts consumed by admission; same-byte caches from another recipe do
not replace them. Failed admission retains independent results and missing coverage.

Local/SSH cache admission in [remote-rust.py](../../scripts/remote-rust.py) connects/starts
sccache under the existing capacity lease and probes effective scratch with a
unique cache-miss compilation; [local lifecycle coverage](../../scripts/test-local-rust.py)
includes isolated real-cache cleanup and obsolete-server refusal.

Ordinary local/SSH totals and module reports share Cargo-target attribution in
[validation-report.py](../../scripts/validation-report.py). The
[runner regressions](../../scripts/test-remote-rust.py) exercise worker results and
compact waits with nested child summaries, failed and unfinished parents, and multiple targets.

Actual native Gateway obligations stay separate from backend and Worker success
in [test selection](../test-selection.md#explicit-native-agent-product-gates).
The shared [native command scope](../../scripts/native-agent-scope.py) prepares
independent native roots and contains the complete acceptance driver; its exact-PID
observer belongs to the trusted controller. Client drivers retain business oracles,
port ownership and cleanup. [Offline regressions](../../scripts/test-native-agent-scope.py)
do not establish native execution or DSH Web/Cordis scope coverage.
The [control-shell startup fixture](../../tools/product-e2e/tests/control_shell_support/startup.rs)
retains bounded private failure evidence before temporary-root cleanup; its
[regressions](../../tools/product-e2e/tests/control_shell_support/startup_tests.rs)
cover timeout, early exit, normal readiness and incomplete diagnostics. Follow the
[diagnostic guide](../diagnostic-troubleshooting.md) for retention and interpretation.
The [product read boundary](../../crates/product-session/src/commands/read_context.rs)
reports post-transport Home rejections; its [command tests](../../crates/product-session/src/commands/read_context/tests.rs)
and [closed event checks](../../crates/diagnostics/tests/product_read.rs) distinguish
actual final failures from successful control calls without logging payloads.

Shared private filesystem behavior has one [platform contract suite](../../crates/platform/src/private_fs/tests.rs): exclusive creation, protected handle reads, permission preservation and identity-bound cleanup. Storage and Worker tests keep domain errors, recovery and lifecycle assertions; the native Windows cases additionally exercise ACLs and held handles.

The [Windows iteration route](../validation-routing.md#windows-iteration-and-release-validation)
uses managed MSVC cross-compilation and affected execution on the configured Windows
development machine. The [release workflow](../../.github/workflows/build-release.yml)
requires the reusable native Windows matrix before package builds; PR/main pushes
do not start that expensive matrix.
The [Windows CI entry](../../scripts/windows-headless-ci.ps1) requires nonempty
native inventories for primitives, shared libraries, installed foreground and
Task business paths, retained low-level SCM tests, and fixed upstream clients.
The Windows 11 ARM install job builds a native ARM64 package and invokes the Rust
transaction; the other Windows 11 jobs retain their explicit x64 replay boundary. The
[canonical native link configuration](../../scripts/build-release.py) also owns
the Debug checks' static CRT; [captured-source inputs](../../scripts/windows-native-artifacts.py)
bind its CMake toolchain, and the [PE import gate](../../scripts/windows-native-runtime-audit.py)
rejects redistributable VC runtime dependencies before replay.
The
[fixture producer](../../scripts/windows-agent-fixtures.py) binds exact npm
integrities and official Qoder archive hashes; the
[feedback adapter](../../scripts/windows-ci-report.py) retains the original SHA
and native verdict. A build or client initialize is not evidence of tool execution,
provider login, Windows 11, or another processor architecture.

[Native scope dispositions](../../scripts/windows-native-scopes.ps1) own the
complete background recipe names and the hosted runtime/deferred partition.
Hosted producer and replay retain every compiler artifact and exact nonempty
inventory while deferring the three logged-owner Task bodies. The feedback adapter
checks zero execution, null exit and actual capture/list facts against the original
verified manifest; missing compilation or a failed executable gate stays red.
Its successful hosted assessment remains `not_assessed` for background acceptance,
with no full-scope or complete-candidate claim. Use default `full` for actual
logged-owner acceptance; that mode keeps every original runtime gate. See
[hosted evidence limits](../github-actions-validation.md) and the adapter's
[regressions](../../scripts/test-windows-ci-report.py) before changing this boundary.

The [installed management journey](../../crates/daemon/tests/support/windows_management_business.rs)
keeps Host collaboration receipts, native defaults and model authority separate.
Its foreground doctor/logs negatives do not replace the
[real Task doctor/logs helper](../../crates/daemon/tests/support/windows_management_service.rs).
The [standard-owner Task journey](../../crates/daemon/tests/support/windows_background_standard.rs)
executes as its own actual logged-on standard user, with no administrator group
or alternate token. It requires an absent installation and SID-derived Task,
then verifies native identity, Control/Gateway recovery, public service restart,
packaged login lifecycle, recovery and guarded uninstall. The
[Task action supervisor](../../crates/daemon/src/control/windows_task.rs) keeps
the registered arguments and persisted definition unchanged while owning a
same-user business child and its complete Job. The real crash case must observe
a new authenticated business process, and a requested stop must cancel recovery;
Scheduler action completion or restart settings alone do not prove that behavior.
The forced-stop case stalls the real protected-input reader and checks the held
daemon's native exit time. The lifecycle client must retain its kernel peer proof
when the stop acknowledgement times out, then mark manager stop intent before
terminating that held process; malformed replies still fail closed.
The
[shared authorization checks](../../crates/daemon/tests/support/windows_subscription_authorization.rs)
cover public missing/wrong-kind/stale Result/Release rejection; they do not prove
valid provider authorization.
WTS account lookup retains actual SID validity and equality independently of
the API's input buffer capacity; follow this fixture for identity prerequisites.
The self-reexecuted owner fixture starts its closed result frame on a new line,
so the parent can require an exact prefix independently of libtest's pretty
caption; a successful child exit alone does not establish a received result.
Owner command envelopes share the public Control response bound, so real
diagnostic log tails are admissible on repeated runs; other fixture
metadata and stderr retain their smaller capture bounds.
Its policy toggles do not establish console closure, RDP reconnection or a login
trigger; those need external session evidence.
The [Task upgrade journey](../../crates/daemon/tests/support/windows_background_upgrade.rs)
shares its original recovery, rollback and business-state assertions between the
alternate-token gate and a separate actual-standard-owner gate. Its
[process and cleanup boundary](../../crates/daemon/tests/support/windows_background_upgrade_transport.rs)
uses ordinary process creation for the latter and refuses cleanup without the
exact installation record. Replays retain prior saved Sources and use fresh
intent identities; native fixture reuse requires the exact restored settings and
pinned executable, with no authenticated Codex profile. The untouched settings
leave the native default model to Codex so Restore needs no account catalog.
Same-OS package qualification does not establish the
original full Windows 11 baseline or execution of either upgrade case.
The [console owner](../../crates/platform/src/windows/console.rs) keeps the GUI
daemon detached for background launch and preserves inherited pipes when explicit
foreground use attaches the parent console. Its [native primitive cases](../../crates/platform/tests/windows_console.rs)
use isolated consoles; the [public foreground cases](../../crates/daemon/tests/support/windows_foreground_console.rs)
require the actual CLI to wait for natural daemon drainage after Ctrl-C and
Ctrl-Break. Their test-owned installation does not replace complete-package or
logged-owner Task acceptance.
The [complete-package foreground CPA journey](../../crates/daemon/tests/support/windows_packaged_foreground.rs)
executes before the required logged-owner Task, so missing WTS prerequisites cannot
hide stock CPA startup, public PKCE cancellation/expiry or launch-snapshot failures.
Its marker is test-owned; installer, Task and authorized outbound model traffic
remain separate required evidence.

The [native Source upstream](../../crates/daemon/tests/support/windows_headless_business_upstream.rs)
owns the independent model endpoint and bounded wire receipts; public command
and settings assertions stay in the business journey. Its accepted TCP sockets
and the [rejected package proxy observer](../../crates/daemon/tests/support/windows_packaged_proxy_observation.rs)
explicitly clear nonblocking mode before bounded reads. Real delayed header/body
and post-rejection peer EOF regressions cover the inherited Winsock mode without
extending deadlines or weakening request validation.

The [Worker dependency selection fixture](../../crates/daemon/tests/support/windows_worker_dependency_selection.rs)
binds the locked npm fixture directory before daemon startup, then submits actual
discovery candidates only when their held FileIDs match the pinned CLI and Node.
The transaction keeps its canonical tuple and digest contract; same-byte foreign
files and duplicate candidates cannot replace an admitted dependency.
The public journey rejects a noncanonical direct LocalControl request, then
proves ordinary CLI input and canonical replay share one Operation and revision.
[CLI native input tests](../../crates/cli/src/worker/selection_windows_tests.rs)
also preserve completed canonical replay after file deletion. The
[Pi locator regressions](../../crates/integrations/src/agents/pi_native/windows_tests.rs)
cover original npm/package paths and reparse rejection before canonicalization.
[Worker terminal facts](../../crates/daemon/tests/support/windows_worker_failure_diagnostics.rs)
publish typed result states and bounded, independently observed oracle counters;
they retain neither arbitrary result text nor a guessed per-run failure cause.
The [cancel publication fixture](../../crates/daemon/tests/support/windows_worker_cancel.rs)
waits for the complete nonce receipt within the original deadline; file existence
alone does not prove that the native writer has published its contents.
The shared model oracle follows the declared shell timeout units. OpenCode's
Windows shell receipt uses the runner's pinned Node with encoded paths, so the
same cancellation check works with PowerShell, cmd and Git Bash.
Its residual binding reader distinguishes incomplete publication from fatal native
identity errors. The [residual oracle](../../crates/daemon/tests/support/windows_worker_residual_oracle.rs)
keeps correlated tool errors observable, with fixed stage/error facts and no native
body. These diagnostics preserve the original cancellation and recovery assertions. The
system PowerShell parser probe retains its original timeout while recording actual
entry, parse and completion stages through bounded private captures; a forced Job
cleanup never turns that timeout into successful parsing.
The [readonly control](../../crates/integrations/src/agents/collaboration_probe/readonly_control.rs)
keeps pipeline entry and language-mode observations separate from actual write
attempt and access-denial receipts; diagnostic output cannot satisfy the permission gate. Its Windows controls prove
actual writable and ACL-denied behavior in both system PowerShell and pwsh;
FullLanguage retains File.WriteAllText, while ConstrainedLanguage uses the native
filesystem cmdlet and an independently nonce-bound typed denial.
The [background state fixture](../../crates/daemon/tests/support/windows_background_state.rs)
preserves persistent state after uninstall; the Lua journey requires an absent state
at its start, while the upgrade journey retains its existing-state recovery contract.
For Qoder Skill receipt mismatches, follow the
[retained Windows Skill binding](../../crates/integrations/src/agents/qoder_probe_skill_diagnostic.rs)
and its HTTP challenge owner. Windows admits a directory spelling only through its
held native directory/file identity, captured permissions and content checks before and
after a same-candidate user-array receipt with exact body. Linux retains the literal
path rule. Test-only closed observations distinguish these dimensions without
publishing paths or bodies; diagnostic facts alone cannot authorize a match.
The [output-read controls](../../crates/integrations/src/agents/qoder_probe_output_diagnostic_tests.rs)
distinguish the live file-size guard from a failed final read after the native
scope stops. Their closed observations retain actual file length, I/O kind and
sampling failure without exposing output. A size-related error caption alone
does not prove that the file exceeded its limit or that account execution passed.
The [owned capture reader](../../crates/integrations/src/agents/native_probe_process.rs)
reads the retained file only after its native process scope stops, preserving
exclusive Windows sharing and the exact byte limit. Its native causal control
uses the production private-file creator; borrowed files keep their path reader.
Unix process controls also prove that an exited leader skips termination grace,
a running leader retains it, and a TERM-ignoring descendant cannot remain active.
The [recovery-read transaction tests](../../crates/local-storage/src/control/current_dependencies_tests.rs)
prove exact current-journal exclusion without redundant decoding, retain other
parked operations, and reject stale current facts, indexed-step damage and missing
membership coverage. Publication process tests retain activation/recovery behavior.
The [trial Agent browser controls](../../apps/desktop/tests/v3/visual/trial-agent-read-checks.mjs)
also cover an external Codex root-to-profile change: stale Apply cannot overwrite
the new target, explicit refresh discovers it and replaces stale authority, and
only a subsequent user confirmation maps the refreshed context.
The [actual Git Bash control](../../crates/integrations/src/agents/qoder_probe_windows_cli.rs)
executes the selected candidate CLI, including literal special-character drive paths,
and retains exact output equality. Its required native recipe selects an explicit
Git for Windows image through a validation-only input. The held installation and
MSYS runtime imports determine its distribution; a compiler triplet alone cannot
admit another shell. WSL or an absent shell cannot supply a passing result.
The [Qoder tool fixture](../../crates/daemon/src/delegation/installation/native_tools_probe.rs)
and [saved-model Skill oracle](../../crates/daemon/tests/support/windows_qoder_user_models_skill.rs)
use the same Bash execution contract with separate causal controls. Their
[native path binding](../../crates/daemon/tests/support/windows_qoder_native_paths.rs)
retains ordinary and proven canonical spellings, permits the stable CLI's
existing hardlink, and releases Skill leases before production restoration.
These fixtures do not substitute for actual account-backed tool execution.
The [native tool caller](../../crates/daemon/src/delegation/installation/windows_tools.rs)
also rechecks the same file effects after each New/Continue execution has stopped
its owned process scope. A correlated receipt observed before exit cannot replace
that final-state check; the shell fixture includes a same-identity late-write control.

The same native entry's `-InventoryOnly` exports static scope recipes through its
actual required-test producer without Cargo, package preparation or client launch.
Protected Qoder acceptance uses that exact source inventory, including library and
installed integration-test targets; a missing saved-model recipe cannot be accepted
as complete account coverage. Its borrowed official profile retains native identity
without reading authentication bytes, and controlled upstreams prove saved routing
and tool execution separately from provider-account model inference.
Use the exact old checkout's export to compare upgrade baseline cases, filters and
harness arguments; the export itself records no runtime execution. Workstation
replay provenance binds its actual OS and original manifest/package hashes to the
producer run, so another build of the same source is not interchangeable evidence.

Code-map and local ownership-guide edits require link/owner review and the selected
contract checks, not new model calls. Runtime-embedded Markdown and shipped Skills
can change behavior and need their actual consumers. Instructions and compilation
remain distinct from executed business evidence.

Repository agent policy under `.agents/` and `issue-spec/project/` is separate from
the product-installed collaboration Skill. The selector maps known project skill
scripts to their own tests; unknown skill code and runtime-embedded documents
still require consumer investigation. Repository ignore rules select publication-policy
regressions; optional ignored skills are not public-checkout test dependencies.
See [instruction ownership](../../.agents/README.md)
for pinned generation and actual host discovery, including the shared validation skill.

Complete-model-data regressions cover [canonical wire identity](../../crates/integrations/src/model_feed/validation.rs),
[manual versus scoped defaults](../../crates/integrations/src/compute/defaults.rs),
[source/default commit and restart](../../crates/local-storage/src/control/compute/management/tests/defaults.rs),
[defaults, metadata pagination and removal dialogs](../../apps/desktop/tests/v3/visual/model-metadata-active.mjs),
[price activation races](../../crates/daemon/src/control/runtime/prices/tests.rs), and
[slow shared-session reads](../../crates/product-session/src/commands/model_feed/tests.rs).
The [producer regressions](../../tools/release-facts/src/feed/tests.rs) reject
uncommitted and committed same-revision corrections and rollback during bootstrap;
the [service reopen scenario](../../crates/daemon/src/model_feed/tests.rs) preserves
settings, acceptance and a captured old snapshot across an embedded v2 data upgrade.
Directory scenarios use real generated inferred and observed records, including
snapshot-bound provenance reads and retry; IPC is mocked, not native acceptance.
The installed HTTPS lifecycle uses complete v2 publications, exact documented cache components,
frozen main/Worker/Continue execution, removal, restart and embedded absorption.

Current dependency startup and journal partition regressions live in
[`current_dependencies_tests.rs`](../../crates/local-storage/src/control/current_dependencies_tests.rs)
and [`current_storage/tests.rs`](../../crates/local-storage/src/migrations/current_storage/tests.rs):
retained-history growth, active/LKG damage, missing membership/coverage, rollback,
pending terminal finalization and payload substitution. These component cases do
not establish native startup timing or same-data Debug/Release completion.

The `plan_content_process.py` production process suite interrupts before and after the local terminal commit: the latter already has the exact Plan head/version while remaining durably pending until the real finalizer finishes. `control::read_stamp` rejects missing revision guards and observes committed writes from either connection; the daemon routing lock test proves native read-grant registration remains independent of the routing-input fence.

Explicit native-target discovery tests in `control::agent_history_cache_tests`
count only current target-owner reads as history grows, keep pending targets
strict, reject missing coverage/current owners, and preserve inline legacy journal
bytes during one-time target-reference initialization. Superseded unrelated
corruption remains visible to the strict history reader without blocking discovery.

[Artifact dependency regressions](../../crates/local-storage/src/control/artifact_dependency_tests.rs)
use real protected files and journal transactions to distinguish superseded damage
from current or unfinished recovery damage. They cover missing coverage, exact
admission-owner CAS, successful retirement, rollback retention and failure between
owner update and terminal commit. Legacy marker tests authenticate V2/V3 backups on
individual access as well as explicit full audit. These are storage assertions,
not a substitute for process interruption/restart or native latency acceptance.

Current model consumers use the same bounded owner set: the daemon
[`settings_entry_model_budget_tests.rs`](../../crates/daemon/src/control/runtime/settings_entry_model_budget_tests.rs)
publication-admission scenario and
[`settings_profile_tests.rs`](../../crates/daemon/src/control/runtime/settings_profile_tests.rs)
active Codex access scenario retain replaced journal damage and missing optional
completed-service rows without bypassing current budget/native-file checks.
The storage dependency cases separately reject a missing required grant step.
Historical Restore is exercised after restoring the exact damaged fixture bytes.

[Legacy Plan completion discovery](../../crates/local-storage/src/control/legacy_finalization_tests.rs)
exercises the actual initializer over retained inline journals. Exact committed heads
and published content avoid reopening finalization; missing/different heads, prepared
content and foreign owners retain recovery, while malformed published content is
rejected. Journal bytes remain unchanged. These storage cases do not replace a
released-package upgrade or native performance measurement.

Current Agent readback coverage: `agent.readback.active-and-exact` and `desktop.operation.hidden-summaries-defer` exercise scoped saves and deferred hidden reads. The storage `agent_history_cache_tests` and `agent_history_epoch_tests` retain full closure corruption, snapshot/rollback and bounded-projection assertions; `commands/agent_snapshot/tests.rs` applies the existing authority fences to exact-context reads.

Selected Agent refresh and settings admission: the production adapter scenarios in
[`settings_entry_native_routing_tests.rs`](../../crates/daemon/src/control/runtime/settings_entry_native_routing_tests.rs)
prove two independent DSH composition checks (Preview and locked Apply), exact-context
selection, rejection of mismatched targets and absence of neighboring CLI probes.
[`settings_tail.rs`](../../crates/application/src/operations/tests/settings_tail.rs)
checks authorization/revision before preparation, replay without rebuilding and
separate Restore authority. Shared transport tests in
[`agent_snapshot/tests.rs`](../../crates/product-session/src/commands/agent_snapshot/tests.rs)
retain all publication/authority/lifetime fences for selected reads.
