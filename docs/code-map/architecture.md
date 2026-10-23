# Architecture and ownership

[Code map](README.md) · [Agent/Worker map](worker-context.md) ·
[Decision map](decision-foundation.md) · [Test map](testing.md)

## Web console and Windows host

`crates/daemon/src/console` owns the separate HTTP listener, origin/asset admission,
credential stores and per-client product contexts. Service startup exclusively
binds configured sockets, including the local companion in network mode; launch
minting selects live state and never enables listening. Linux installer options
and `console init` persist scope/port in the host-runtime listener store. A wildcard
bind requires a concrete advertised IP for its one-use URL. Launch codes bind that
origin and actual port; the Windows shell explicitly requests the local endpoint.
Request deadlines cover each request independently of connection age; incomplete
headers and bodies have their own bounds. Shared confirmation permits and broker
prompts release on caller cancellation, without approving or affecting another context.
The console transport reserves separate 64-connection pools for loopback and network
peers. Socket writes close after 30 seconds without byte progress; successful writes
reset that deadline, and operation/confirmation waits do not arm it.
Its private in-process Client
Core transport shares Local Control budgets, operation admission and upgrade
drain; external Local Control retains its released-command gate. The frontend
`apps/desktop/src/host` selects the Tauri, browser or Windows-shell transport and
host capabilities. Browser product requests use explicit Bearer/context headers;
native host actions stay in their host adapter.

[`apps/shell`](../../apps/shell/README.md) owns the Windows Tauri window, exact-origin
host IPC, startup/reconnect, tray/login preference and NSIS hooks. The
[Rust standalone transaction](../../crates/host-runtime/src/standalone_install)
owns architecture-matched Task installation and current-schema journal recovery; the shell's separate
inventory/finisher owns its program files and update outcome. Neither owns business
transactions or exposes Local Control to WebView.
The native [current-package repair](../../crates/host-runtime/src/standalone_install/windows/repair.rs)
matches the complete accepted package and owned Task before reapplying a prepared
core journal forward. Generic rollback refuses possible storage writes before any
manager effect. NSIS supplies the candidate to early shell recovery, preserving
the original batch instead of forcing an older-program rollback. The
[release catalog](../../apps/website/src/lib/releases.mjs) emits V3 and the registered
Windows-free V2 projection for supported macOS readers. Public Windows artifacts
are x64 NSIS installers only; headless x64/ARM64 archives stay internal. The
installer and tests invoke `__windows-install` / `__windows-uninstall`, with no
Python installer or offered SCM installation entry.

## Shared layers

| Responsibility | Production owner | Boundary |
| --- | --- | --- |
| Business types and invariants | [Domain](../../crates/domain/src) | Typed intent, identity, immutable plans and operation checkpoints |
| Local control contract | [Application API](../../crates/application-api/src), [client-core](../../crates/client-core/src) | Shared requests/responses for CLI and Desktop; generated schemas come from typed producers |
| Business workflows | [Application](../../crates/application/src) | Planning and authorization through ports; external effects are supplied by the daemon |
| Composition and process lifecycle | [Daemon control](../../crates/daemon/src/control/runtime), [delegation](../../crates/daemon/src/delegation) | Concrete adapters, accepted operations, Worker execution and recovery |
| Operating-system primitives | [Platform](../../crates/platform/README.md) | Cross-platform private filesystem creation/reads and owned cleanup; native handles, ACLs, file identity and local byte streams. Business authority and lifecycle remain with callers |
| Durable state and external effects | [Local Storage](../../crates/local-storage/src), [Integrations](../../crates/integrations/src) | Store current state; read or conditionally change external Agent configuration through its native leaf |
| Request execution | [Gateway](../../crates/gateway/README.md), [gateway-core](../../crates/gateway-core/src) | Resolve native credential carriers against published authority, plan attempts, adapt protocols and enforce execution budgets; [listener regressions](../../tools/e2e-harness/tests/p0_gateway_runtime/bearer_auth.rs) cover precedence and revocation |
| Business evidence | [Observation](../../crates/observation/src), [authorized queries](../../crates/application/src/observation_query) | Execution facts, managed content, session queries and valuation; [diagnostics](../../crates/diagnostics/src) remain separate |
| Product presentation | [Desktop product shell](../../apps/desktop/src/product), [native host](../../apps/desktop/src-tauri/src), [CLI](../../crates/cli/src) | The native host projects safe DTOs; frontend state does not grant authority or establish durable success |

CLI JSON request sources (`--request-file` / `--request-stdin`) share the bounded
[strict input reader](../../crates/cli/src/request_input.rs). Command codecs retain
typed intent, selector and byte-limit ownership; see the [CLI input guide](../../crates/cli/INPUT.md)
and [production argv regressions](../../crates/cli/tests/request_input.rs).
Connection-test help and native/registered templates come from
[Application API](../../crates/application-api/src/command_schemas.rs); the
[installed-example regression](../../crates/cli/tests/connection_test_examples.rs)
executes the displayed commands with those request documents. The
[real-process recipe regression](../../crates/daemon/tests/support/connection_test_examples_product.py)
checks their current-catalog results and protected-input cleanup through CLI/daemon.

[Quickstart and Preview convenience inputs](../../crates/cli/src/quickstart.rs) compose existing
queries and transactions. They use [Application authoring options](../../crates/application/src/routing/editor_options.rs)
for both initial and reopened editors, also consumed by Desktop. Exact Apply files are ordinary
user-selected requests, not another journal or authority. [Offline schemas](../../crates/application-api/src/command_schemas.rs)
come from shared wire DTOs, including [discovery](../../crates/application-api/src/agent_discovery.rs)
and [settings Preview](../../crates/application-api/src/agent_settings_preview.rs); Application
still owns discovery/planning and the daemon supplies adapters.

CLI/Desktop → client-core → daemon control → Application → storage/native adapters
is the control path. Published plans and grants feed Gateway execution. Worker
admission freezes a plan and run authority before the native client calls Gateway.
Observation records what actually ran and supplies authorized reads and continuation.

`LocalControlAdapter` remains a broad assembly facade. Follow its focused runtime
module before extending it; a local feature does not require a facade-wide rewrite.

## Model sources and native configuration

Custom API [target normalization](../../crates/integrations/src/model_connections/target.rs)
is pure for HTTP and HTTPS, retaining the configured hostname until bounded request
transport resolves it. Explicit LAN/remote HTTP uses the ordinary check/save flow.
The [durable target](../../crates/domain/src/compute/management/mod.rs) and
[user-configured Gateway target](../../crates/domain/src/routing/gateway_execution.rs)
admit the same explicit transports while retaining exact credential destinations
and the separate registered/managed-connector scopes.
[Control error translation](../../crates/daemon/src/control/runtime/model_connections/error.rs)
retains closed target reasons in the existing error envelope; the Desktop form owns
localized corrections and plaintext notices. The [real-process regression](../../crates/daemon/tests/support/custom_http_product.py)
covers omitted-directory save/restart/recheck and safe error propagation.

Branch execution keys distinguish reasoning selections. The derived Gateway candidate
also retains its original source binding for current credential membership and exact
leases; routing, retry and context identities continue using the execution key.

CLI secret entry uses the shared local [protected input socket](../../crates/daemon/src/control/standalone.rs)
in both standalone and Desktop-owned `role=all` processes. The [CLI input owner](../../crates/cli/src/host_commands.rs)
reads inherited descriptors; ordinary control requests never carry secret bytes.
All current hosts and the CLI resolve its logical `hiroute/input.sock` endpoint through host-runtime.
Windows maps it to a private named pipe bound to the held runtime directory and account SID;
it does not create a filesystem socket. Follow [native ownership](../../crates/platform/README.md).
The name must fit any runtime root that supports `hiroute/control.sock`, including macOS.

| User step | Owners to follow | Representative guard |
| --- | --- | --- |
| Add an API connection and choose its models | [Connection directory](../../apps/desktop/src/features/model-connections/ConnectionDirectory.tsx), [form and draft ownership](../../apps/desktop/src/features/model-connections/README.md), [registered check](../../crates/daemon/src/control/runtime/model_connections/registered.rs) | The executable catalog supplies the complete directory. A source name survives check/save; per-product credentials stay in the dialog and protected input. Released public registered probes may omit the name and use the authenticated option label; the current Desktop check requires an explicit label. |
| Discover a native source without executing it | [Native model connections](../../crates/integrations/src/model_connections), [Pi reader](../../crates/integrations/src/agents/pi_sources.rs), [DSH reader](../../crates/integrations/src/agents/dsh_sources.rs), [OpenCode reader](../../crates/integrations/src/agents/opencode_sources.rs), [Hermes reader](../../crates/integrations/src/agents/hermes_sources.rs) | Shared [point grouping](../../crates/integrations/src/agents/native_points.rs) keeps scope/provider/address/protocol/authentication distinct; static scan never executes helpers |
| Display, prepare and save the selected source | [Safe shared product projection](../../crates/product-session/src/commands/model_connection_web.rs), [V2 points/catalog qualification](../../crates/daemon/src/control/runtime/model_connections/points.rs), [daemon preparation](../../crates/daemon/src/control/runtime/model_connections/pi_discovered.rs) | Exact native/catalog provenance and explicit model selection remain visible; privileged locators stay private; stale facts cannot expand an accepted selection; legacy one-model handles retain their meaning |
| Rename a connection, append models or remove saved configuration | [Shared lifecycle planner](../../crates/application/src/compute_management/mutations/lifecycle.rs), [transactional reference guards](../../crates/local-storage/src/control/compute/management/references.rs), [Desktop lifecycle dialog](../../apps/desktop/src/features/models/ConnectionLifecycleDialog.tsx) | Templates do not own credentials. Explicit append preserves saved members; enabled/disabled routes and retained recovery versions block removal. Removing the last model requires explicit connection deletion; owned secrets follow the same recoverable operation. |
| Save Key order or use current Keys with an older plan | [Single-save edits](../../apps/desktop/src/features/models/model-draft.ts), [request admission](../../crates/gateway/src/core_runtime/credentials.rs), [current pool authority](../../crates/local-storage/src/control/compute/management/runtime_pool.rs), [safe Key masks](../../crates/daemon/src/control/runtime/model_connections/key_masks.rs) | A request owns its admitted Key IDs and order; later exact leases enforce current source/Key eligibility and generation. Admission retains closed permission denials in the existing request summary. Shared Plan content stays frozen. Masks are a separate authenticated query; saved plaintext never reaches Desktop. |
| Connect a native Codex or Claude subscription | [Source discovery](../../crates/integrations/src/agents/subscription_sources.rs), [credential adapters](../../crates/cpa-bridge/src), [shared lifecycle](../../crates/daemon/src/control/runtime/subscriptions.rs), [provider runtimes](../../crates/cpa-bridge/src/runtime_set.rs) | Native clients retain refresh authority; only access leases enter isolated CPA instances. Claude Keychain interaction is limited to an explicit check; discovery reads metadata only. Codex borrowing reads file storage only and rejects a configured non-file store even if stale auth.json remains. |
| Locate a slow subscription validation condition | [Stock discovery/pin loop](../../crates/cpa-bridge/src/accounts/stock.rs), [pin facts](../../crates/diagnostics/src/event/cpa_control.rs), [CPA discovery facts](../../crates/diagnostics/src/event/cpa_model_discovery.rs), [troubleshooting](../diagnostic-troubleshooting.md) | [Condition regressions and isolated live probe](../../crates/cpa-bridge/src/accounts/stock/diagnostic_tests.rs) distinguish unmet pins; [private metadata admission](../../crates/cpa-bridge/src/accounts/stock/model_discovery.rs) observes upstream phase, reason and timing from the [pinned CPA](../../vendor/cpa/README.md), retaining the first failure without logging account identities. |
| Sign in independently to a Codex or Claude subscription | [Shared login control](../../crates/application/src/control_plane/subscription_login.rs), [protected lifecycle](../../crates/daemon/src/control/runtime/subscriptions/login.rs), [private sessions](../../crates/cpa-bridge/src/managed_sessions.rs), [Desktop sign-in](../../apps/desktop/src/features/subscriptions/SubscriptionSignIn.tsx) | CPA receives browser callbacks on temporary loopback listeners; an unavailable listener uses explicit protected manual input. Login is separate from Check/Save and routing. Daemon maintenance owns pending completion/expiry, and saved revisions own runtime selection. [The pinned CPA adapter](../../vendor/cpa/README.md) owns callback admission, authorization and refresh. |
| Lease a subscription credential without blocking Gateway workers | [Async adapter](../../crates/daemon/src/gateway_ports/credential/cpa.rs), [request budget](../../crates/cpa-bridge/src/request_context.rs), [admission and observations](../../crates/cpa-bridge/src/runtime/admission.rs), [Claude identity resolution](../../crates/cpa-bridge/src/claude_profile.rs) | Bounded provider executors own compatibility I/O. Short admission updates revoke before lifecycle work; identity lookup runs outside the lifecycle owner. Cancellation and saved revisions guard later publication. Follow [concurrency regressions](../../crates/cpa-bridge/src/runtime/tests/request_io.rs) for races with disable, replacement and delayed I/O. |
| Inspect saved subscription mode and repair sign-in | [V3 query](../../crates/application/src/compute_management/query_v3.rs), [mode and repair copy](../../apps/desktop/src/features/models/subscription-copy.ts) | Mode derives from the committed source selection and survives missing credentials. Desktop consumes V3; published V2 queries retain their strict response shape. Existing release-version matching still applies; an external service without V3 requires an update. |
| Configure a subscription connection proxy | [Device scan](../../apps/desktop/src/product/ModelManagementPage.tsx), [inline proxy editor](../../apps/desktop/src/features/SubscriptionProxySettings.tsx), [native apply](../../apps/desktop/src-tauri/src/bridge/subscription_proxy.rs) | Scan retains results while editing; cancel restores its actions, and explicit save/restart returns to a fresh scan. Settings has no proxy entry; host policy remains shared by all subscriptions. |
| Make a saved source usable by a plan | [Capability qualification](../../crates/application/src/compute_management/compilation.rs), [candidate materialization](../../crates/daemon/src/control/runtime/candidate_execution.rs), [compiler](../../crates/application/src/compiler) | Native-observed facts retain source-local authority and budgets; [unavailable candidate reasons](../../crates/daemon/src/control/runtime/routing_candidates.rs) are read-only and never execution facts. Upstream model values use the shared opaque-ID rule; internal references remain strict |
| Distinguish a missing subscription candidate from a runtime outage | [Exact candidate projection](../../crates/daemon/src/control/runtime/compute_routing.rs), [source authorization](../../crates/daemon/src/control/runtime/source_authorization.rs), [provider discovery](../../crates/cpa-bridge/src/runtime_set.rs) | Resolve the requested provider from the trusted connection option. Its discovery error remains Unavailable; a best-effort aggregate list cannot establish that a selected source is absent. |
| Configure models and collaboration independently | [Shared settings transaction](../../crates/application/src/agent_connection/settings), [typed native facts](../../crates/daemon/src/control/runtime/settings_facts), [native provider guide](../../crates/integrations/src/agents/additional_native/README.md) | Native defaults, model grants, installed Skills and conditional restore have distinct owners |
| Project native reasoning settings | [Wire types](../../crates/domain/src/routing/gateway_execution.rs), [daemon renderer](../../crates/daemon/src/control/runtime/candidate_protocol_profiles.rs), [Gateway reader](../../crates/gateway/src/profiles/reasoning.rs) and [wire normalization](../../crates/gateway/src/adapters/request/reasoning.rs) | Keep one field/path/toggle definition; [materializer → compiler → Gateway tests](../../crates/daemon/src/control/runtime/candidate_execution_tests.rs) cover accepted aliases |

Native configuration precedence belongs to each reader. For example, Pi provider
`baseUrl` overrides inherited catalog addresses, while provider `api` defaults apply
only to explicit models; [filesystem tests](../../crates/integrations/src/agents/filesystem_tests/pi.rs)
cover that distinction. DSH module replacement and credential-store precedence
are located in the [native composition guide](../../crates/integrations/src/agents/additional_native/README.md#dsh-static-composition).
Do not move either rule into a generic provider merge.

[Model settings status](../../crates/daemon/src/control/runtime/settings_status.rs)
joins owned native configuration with the current Gateway endpoint as well as
grant/publication evidence. Its opt-in [effective-state projection](../../crates/daemon/src/control/runtime/agent_effective_state.rs)
uses existing Observation queries and authenticated receipts with the same current Plan/publication/grant identity.
It does not infer loading from saved files, persist new state, restart Agents or send inference requests.
[CLI listener status](../../crates/cli/src/host_commands.rs)
uses that read and, for unconfigured/restored targets, the
[released V1 connection join](../../crates/daemon/src/control/runtime/agent_connection_discovery.rs)
to request explicit reconfiguration; listener operation history alone
does not establish Agent convergence. Port selection belongs to the shared
[listener store](../../crates/host-runtime/src/gateway_listener.rs), including Desktop.

Follow discovery through an ordinary request using the saved source. The
[Pi](../../tools/product-e2e/tests/PI_INTEGRATION.md),
[DSH](../../tools/product-e2e/tests/DSH_INTEGRATION.md) and
[OpenCode](../../tools/product-e2e/tests/OPENCODE_INTEGRATION.md) acceptance entries cover
Scan → Prepare → Save → restart → native use, including stale-source rejection.

The shared product-session layer in `crates/product-session` owns Preview composition, confirmation permits, retry identity and protected-input lifetime. Desktop supplies its inherited-channel `HostAuthority` adapter in `apps/desktop/src-tauri/src/bootstrap/authority.rs`; native startup, updater and window authority stay with the Desktop host.
[Native recovery](../../apps/desktop/src-tauri/src/bootstrap/recovery.rs) completes
only the exact outstanding acknowledgement on the existing inherited channel,
then transfers the running child to a fresh product session. It never restarts the
daemon or pauses work. Old confirmations and authority remain invalid; malformed
or closed channels fail closed. Status separates reachability from protected authority.

## State and lifecycle boundaries

| Boundary | Invariant | Read next |
| --- | --- | --- |
| Agent settings | Configure/Edit seals service state before the file tail. Ordinary Disable conditionally restores files before withdrawing grants; conflicts preserve foreign edits and require the appropriate fresh operation or exact retry. | [Transaction](../../crates/application/src/agent_connection/settings/transaction.rs), [ordering tests](../../crates/application/src/operations/tests/settings_tail.rs) |
| Native target registration | An unused discovered target is not access authority or a global startup dependency. Actual access and accepted-operation recovery retain path identity and safe-write checks; OS I/O determines accessibility. | [Registration tests](../../crates/local-storage/src/agents/native_registration_tests.rs), [startup journey](../../crates/daemon/tests/publication_process.rs) |
| Operation journal | Domain checkpoints and stored journals agree before publication or native effects; immutable plan bytes and generation/identity guards remain authoritative. JSON member ordering is distinct from a content change. | [Checkpoint](../../crates/domain/src/operation/journal.rs), [verification](../../crates/local-storage/src/control/journal.rs), [transaction guards](../../crates/local-storage/src/control/transaction_v2_tests.rs) |
| Managed credential rollback | Authenticate the exact compensated Secret before converging source references under the same writer. Forward generations and source/workspace revisions reject stale authority; the atomic receipt makes restart idempotent. | [Coordinator](../../crates/application/src/operations/compute_compensation.rs), [Secret proof](../../crates/local-storage/src/secrets/compensation.rs), [Control receipt](../../crates/local-storage/src/control/compute/management/compensation.rs) |
| Reference refresh | A checkpoint copies verified installed routing authority at the next revision through the normal publication writer. Preview pins revisions and content; disabled routes, drafts and held versions remain deletion blockers. | [Strict checkpoint](../../crates/domain/src/operation/publication_checkpoint.rs), [shared entry](../../crates/application/src/control_plane/publication_checkpoint.rs), [installed-state facts](../../crates/daemon/src/control/runtime/routing.rs) |
| Profile deletion | Untouched proof binds the current store instance and exact marker; a stage directory alone cannot prove deletion never began. | [Profile lifecycle](../../crates/daemon/src/control/runtime/settings_profile_tests.rs) |
| Worker/content lifetime | Result completion, process stop, body availability and permission to Continue are separate facts; cleanup requires exact ownership. | [Worker map](worker-context.md#state-and-lifetime), [managed-text contract](../../crates/observation/src/managed_text/README.md) |
| Recovery and compatibility | Current producers have one ingestion contract. Startup presenters prefer the current attempt's typed facts over an older writer's report; the Windows shell binds manager-start failures to a diagnostic boot/sequence watermark captured before startup. Registered legacy revoke tails retain their own ordering; frozen fixtures alone do not establish production upgrade support. | [Startup admission](../../crates/local-storage/src/migrations/startup_format.rs), [upgrade recovery action](../../crates/local-storage/src/migrations/startup_recovery.rs), [host failure facts](../../crates/host-runtime/src/storage_recovery.rs), [daemon failure projection](../../crates/daemon/src/control/startup_recovery.rs), [legacy-tail tests](../../crates/daemon/src/control/runtime/settings_profile_legacy_tests.rs), [compatibility registry](../../contracts/compatibility-support.v1.json) |

## Observation and conversation boundaries

| Concern | Owner | What callers must preserve |
| --- | --- | --- |
| Native Agent session correlation | [Gateway identity resolver](../../crates/gateway/src/context_hold/identity.rs), [native input guide](../../crates/gateway/README.md#native-session-correlation) | One identity feeds ContextHold, decision history and observation; native IDs precede content inference, malformed or conflicting claims remain request-scoped |
| Configured Agent attribution | [Publication join](../../crates/daemon/src/control/runtime/grant_attribution.rs), [persistent grant/session projection](../../crates/observation/src/store/agent_attribution.rs) | New authenticated writes resolve the exact grant generation to its server-owned Agent; rotation/restoration preserve prior attribution, conflicting Agents stay unlinked, and old receipt rebuilds do not backfill identity. Startup uses [verified historical candidates](../../crates/local-storage/src/control/agent_read.rs) best-effort; missing current proof stays unlinked without blocking journal recovery, and restoration registers completed ownership. Activation stays strict |
| Named tool decision facts and request receipts | [Strict Gateway adapter](../../crates/integrations/src/gateway/mod.rs), [Product tool choice](../../crates/domain/src/observation/execution.rs), [receipt writer](../../crates/observation/src/receipt/mod.rs) | Preserve callable kind and name; Product's implicit Function encoding preserves historical digests. Each completed HTTP request closes independently of an unfinished Agent turn; [boundary regressions](../../crates/integrations/src/gateway/tests.rs) cover all tool choices |
| Lifecycle acceptance of empty response end-stream frames | [Transport acceptance capture](../../crates/gateway/src/observation/content.rs), [Product lifecycle validation](../../crates/domain/src/observation/lifecycle.rs), [bounded receiver](../../crates/observation/src/writer/lifecycle.rs) | Successful zero-byte end-stream writes are real operational events with a zero byte count; identity remains strict. [Receiver regressions](../../crates/observation/src/writer/lifecycle_tests.rs) preserve continuous ACK through RequestFinished |
| Capture completeness | [Content completeness](../../crates/observation/src/content/completeness.rs), [session query](../../crates/observation/src/query_v2/sessions.rs) | Aggregate every required request/direction; a finished stream or readable preview does not prove the session is complete |
| Windows service logs | [CLI log reader](../../crates/cli/src/service/windows/logs.rs), [held diagnostic directories](../../crates/diagnostics/src/files/windows.rs) | Read only the daemon writer's current and newest rotated log through existing held directories; settings, keys, reparse points and hardlinks cannot supply log content |
| Streaming memory | [Capture](../../crates/gateway/src/observation/provider/capture.rs), [pending delivery](../../crates/gateway/src/observation/provider/capture/tracker.rs) | Decoder, projector and delivery tracker share the stream budget; queue capacity is not capture evidence. [First-error regressions](../../crates/gateway/src/observation/provider/capture/diagnostic_tests.rs) bind closed capture diagnostics to their original attempt without retaining payloads. |
| Reported token usage | [Response driver](../../crates/gateway/src/runtime/driver/response.rs), [completion facts](../../crates/gateway/src/observation/request/completion.rs), [capture tracker](../../crates/gateway/src/observation/provider/capture/tracker.rs) | Nonstream completion retains known usage before request termination; capture records content without duplicating it. Follow the [Gateway guide](../../crates/gateway/README.md#response-failure-evidence) for streaming ownership. |
| Upstream fault attribution | [Wire projection](../../crates/gateway/src/observation/provider/wire_diagnostic.rs), [attempt association](../../crates/gateway/src/observation/request/diagnostic.rs), [decision calls](../../crates/gateway/src/core_runtime/classification/diagnostic.rs) | Actual bounded model/control fields, negotiated HTTP protocol, closed error codes, hashed request IDs and separate client headers/body commit facts must preserve the real outcome without prompts or provider error text |
| Cross-request source health | [Runtime state](../../crates/gateway/src/runtime/state.rs), [production policy](../../crates/gateway/src/runtime/driver/materialization.rs), [listener regressions](../../tools/e2e-harness/tests/p0_gateway_runtime/auxiliary_failure_isolation.rs) | Production Agent grants and verified Workers keep soft Binding failures request-local so background errors cannot suppress main calls on the same target; exact Credential/quota authority stays shared and the kernel/provider default retains shared backoff |
| Source disabled before an upstream attempt | [Exact permission gate](../../crates/local-storage/src/secrets/runtime.rs), [product preparation reply](../../crates/gateway/src/runtime/driver/selection/preparation.rs), [Gateway error guide](../../crates/gateway/README.md#candidate-preparation-errors) | Preserve the closed lease denial in the request fact and local JSON response; a frozen route does not grant current source/Key permission, and rejected preparation remains zero attempts |
| Inspect CPA authentication recovery | [Private response metadata](../../crates/gateway/src/observation/provider/cpa_execution.rs), [pinned CPA adapter](../../vendor/cpa/README.md) | Logical attempts and actual HTTP sends differ; only verified CPA targets can supply recovery counters. Missing evidence remains unknown and does not create token usage. |
| Same-user CLI analysis | [Read admission](../../crates/application/src/observation_query/control/v2.rs), [scoped analysis](../../crates/observation/src/analysis/read.rs), [CLI codec](../../crates/cli/src/observation.rs), [installed journey](../../crates/daemon/tests/support/quickstart_analysis.py) | Conversation, switch/reason, expense ranking and savings paths share retained scopes; control/price composition and indexed content reads keep one 500 ms budget. Supplied grants, current deletion/visibility and legacy calls remain checked. |
| Recorded request estimates | [Captured price snapshot](../../crates/application/src/prices/snapshot.rs), [source/execution mapping](../../crates/daemon/src/control/runtime/prices.rs), [Gateway freeze](../../crates/daemon/src/gateway_ports/pricing.rs), [installed price regression](../../crates/daemon/tests/support/source_pricing_product.py) | Resolve exact source identity in one pinned generation; a configured API-equivalent estimate can fill unavailable billing prices while preserving its label, partial usage and frozen history. |
| Current API-equivalent comparison | [Analysis price capture](../../crates/application/src/observation_query/control/v2/analysis.rs), [numeric comparison](../../crates/observation/src/analysis/comparison.rs), [legacy adapter](../../crates/observation/src/analysis/legacy.rs) | Source-matched attempt inputs and immutable scoped prices use the same per-attempt arithmetic as the explicit-reference report. Unknown coverage is not zero; recorded valuation remains unchanged. |
| Native compression and actual transitions | [Hermes adapter](../../assets/agents/hermes/__init__.py), [private carrier](../../crates/domain/src/observation/native_continuation.rs), [relationship owner](../../crates/observation/src/analysis/native.rs), [transition reader](../../crates/observation/src/analysis/routing.rs) | Authenticated bounded ancestry does not change physical session identity or prove adjacent user turns. Shared member resolution and current source/deletion guards apply to content and monetary readers. |
| Analysis upgrades and deleted-history revival | [Schema/SQL triggers](../../crates/observation/src/analysis/schema.rs), [native creation proof](../../crates/observation/src/analysis/native_origin.rs), [joint deletion regression](../../crates/observation/src/store/tests/analysis_delete_tests.rs) | Core-table triggers also execute for supported released writers; replace dependent triggers/tables atomically. Full deletion cuts historical edges. Fresh native creation evidence can prove a new child after the cut; unknown origins remain gaps. Strict retained native-event bodies exclude the optional creation extension. |
| Search and gaps | [Live session search](../../crates/observation/src/query_v2/session_search.rs), [query contract](../../crates/observation/src/query_v2/README.md), [keyword lookup](../../crates/observation/src/text_index/lookup.rs) | Fresh authorized pages count matching sessions; bounded background backfill and explicit gaps distinguish pending indexes from complete empty results. The released occurrence-search interface remains separate. |
| Business relations and visibility | [Relation owner](../../crates/observation/src/query_v2/relation.rs), [managed content](../../crates/observation/src/managed_text/README.md) | `query_v2` also writes relations; every content read checks current authorization and visibility |
| Native tool-history continuation | [Responses ingress](../../crates/gateway/src/adapters/responses_ingress.rs), [real continuation journey](../../tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs) | Preserve model-generated empty tool names in native history without relaxing declarations/call IDs; unsafe conversion, ContextHold and reasoning cleanup stay disabled for that history |
| Decision and competence attribution | [Decision observation map](decision-foundation.md#observation-and-public-consumers) | Attribute scores to the stage that actually ran, with its frozen category/group/candidate and rubric |

## Changing boundaries

Compact Observation format admission and fresh-generation initialization belong to
[`store/upgrade`](../../crates/observation/src/store/upgrade.rs) and the
[`store` factory](../../crates/observation/src/store/mod.rs); complete original-source
preservation remains in the existing
[startup coordinator](../../crates/local-storage/src/migrations/observation_source.rs).
The [sealed-source publisher](../../crates/local-storage/src/backup/source_snapshot/seal.rs)
uses same-filesystem directory publication without traversing historical contents.
The optional capacity threshold uses the
[existing maintenance lifecycle](../../crates/observation/src/maintenance.rs) and
[bounded retirement owner](../../crates/observation/src/store/capacity.rs).
Representative scenarios cover
[format admission](../../crates/observation/src/store/upgrade/tests.rs),
[interrupted sealing](../../crates/local-storage/src/backup/source_snapshot/seal/tests.rs),
[production startup/restart](../../crates/daemon/src/control/runtime/history_cutover_tests.rs),
and [shared survivors and rollback](../../crates/observation/src/store/capacity/tests.rs).
Operating and recovery details are in the
[Observation guide](../../crates/observation/README.md#compact-storage-and-capacity-retention).

Windows process defaults and private handle validation belong to
[native security](../../crates/platform/src/windows/security.rs), with suspended-child
initialization in the [Job launcher](../../crates/platform/src/windows/job.rs).
The [CPA private-object leaf](../../crates/cpa-bridge/src/windows_private.rs) uses those
primitives for managed files. The [borrowed native reader](../../crates/cpa-bridge/src/borrowed_native_windows.rs)
retains external file identity and permission facts through access-lease admission;
it preserves readable native ACLs instead of adopting managed-file permissions.
Follow these owners for elevated-account defaults or third-party file creation.
The [Windows Apps registration exporter](../../crates/platform/src/windows/shell_registration.rs)
admits a controlled `reg.exe` export under a held private temporary directory;
the existing child Job gives that process private user-SID defaults. Its explicit
environment binds both temporary-directory variables to the same private stage,
so standard-user export never depends on the Windows-directory scratch fallback. It
then publishes complete bytes with protected private permissions. A repeated
preparation must match the retained export; changed material is preserved and
refused. Unpublished system output never occupies a final recovery filename.
Native client gates select the installed, pinned producer through
[Windows acceptance](../../scripts/windows-headless-ci.ps1). Ordinary Agent
recipes and official Qoder account recipes have separate inventories; the latter
borrow an explicitly prepared private profile and never obtain account authority
from pull-request environment secrets.
Stopped Windows source snapshots preserve native permissions through
[streamed backup copying](../../crates/local-storage/src/backup/source_snapshot.rs)
and the [durable stage owner](../../crates/platform/src/durable.rs); the
[native descriptor owner](../../crates/platform/src/windows/file_security.rs)
restores creation-recomputed DACL control on the unpublished held stage before
exact permission verification. Native artifact
activation uses a sealed-file transition; it does not reopen read-only payloads
for data writes or clear a target's attributes before replacement.
Windows Agent configuration and restoration reads share the
[artifact snapshot reader](../../crates/local-storage/src/control/native_permissions.rs).
Its [read lease](../../crates/platform/src/windows/read_lease.rs) retains all
reparse-free ancestors and the regular file while bounded bytes, native identity
and permissions are checked. Native targets keep their existing size limits and
borrowed ACLs; private restore files additionally require their private owner.
Worker New and Continue compare ordinary and verbatim directory spellings through
the existing [native workspace identity](../../crates/daemon/src/delegation/profile/native_context.rs).
The [managed profile](../../crates/daemon/src/delegation/installation.rs) and
[retained root reader](../../crates/daemon/src/delegation/profile/materials.rs)
still require the recorded marker, volume/file identity and complete ancestry.

Keep a change with the owner that can enforce it. Extend a native leaf for a real
ecosystem difference; reuse settings, admission, lifecycle and observation owners.
Built-in decision models and custom branches are implemented in the
[decision map](decision-foundation.md); tool selection remains protocol-only.

Before extracting or consolidating code, identify the user scenario, state owner
and unique failure assertions. Keep recovery, cancellation, authorization and
foreign-file preservation tests. Assign shared contracts and generated registries
to one convergence owner. Use the [test map](testing.md) to select evidence; neither
file size nor test churn alone justifies another abstraction or a broad rewrite.

Trial cost reads use the internal admitted `TrustCostScopeV1`, joining retained analysis stage identity to Plan/revision/category/group before calling the shared per-attempt comparison engine. [Evidence-gap scenarios](../../crates/observation/src/store/tests/trust_cost_tests.rs) distinguish partial history from conflicting membership, pending projections from unresolved turns, and conservative coverage from paired subtotals. They preserve the general value query and frozen settlement contracts. The [Desktop timeline](../../apps/desktop/src/features/RoutingTimeline.tsx) reconciles absorbed provisional rows using returned request membership; [focused presentation checks](../../apps/desktop/tests/v3/visual/new-model-trial-active.mjs) cover selection, actual focus transfer without stealing focus, and partial monetary subtotals. Live timeline transition presentation adapts the same endpoint/time membership and classifier used by retained analysis; cache mutexes cover only taking and returning snapshots, never SQL or projection. [Real reader exhaustion](../../crates/daemon/src/control/runtime_timeline_retry_tests.rs) and [CLI follow](../../crates/cli/tests/routing_timeline_follow.rs) verify the structured failure boundary and retry recovery.

### Complete official data and saved default intent

The [feed service](../../crates/daemon/src/model_feed) owns complete snapshot selection,
semantic/publication floors and retrieval. [Admission](../../crates/integrations/src/model_feed)
qualifies installed execution without discarding descriptive records. The shared
[capability resolver](../../crates/integrations/src/compute/defaults.rs) joins saved intent
and exact current endpoint/product facts; [price projection](../../crates/daemon/src/control/runtime/prices/official.rs)
uses the same scope with the existing valuation engine. [Source preference persistence](../../crates/local-storage/src/control/compute/management/defaults.rs)
shares source revisions, effect activation and compensation. The [metadata query owner](../../crates/daemon/src/control/runtime/model_feed/metadata.rs)
and [removal preview](../../crates/daemon/src/control/runtime/model_feed/removal.rs) provide bounded
complete reads. The [Desktop directory](../../apps/desktop/src/features/model-feed/ModelMetadataDirectory.tsx)
keeps field inference visible and loads source/rule details against that same captured
snapshot. The [feed producer](../../tools/release-facts/src/feed.rs) checks embedded
semantic identity against HEAD and the supplied baseline before release gating.
Hosts use product-session; published execution remains in its existing owner.
