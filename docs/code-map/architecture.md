# Architecture and ownership

[Code map](README.md) · [Test map](testing.md)

## Control and execution paths

The CLI and Desktop share `client-core`; the Desktop host projects safe DTOs to
the WebView. Requests enter the daemon's control service and the Application
workflows. Application owns business planning and authorization through ports;
the daemon assembles concrete adapters. `domain` owns typed business invariants,
`application-api` owns control DTOs, `local-storage` owns durable state, and
`integrations` owns effects against external Agent configurations and services.

This is an ownership map, not a claim that every existing dependency is clean.
`LocalControlAdapter` still assembles many responsibilities. Follow its focused
runtime modules before extending the central facade; do not start a broad facade
rewrite as a prerequisite for a local feature.

Published plans and grants feed the Gateway. Protocol adapters normalize ingress;
planning and classification select from the admitted publication; `gateway-core`
owns provider-neutral execution, attempt and output budgets; the runtime driver
resolves and contacts upstreams. Preserve authentication, pinned publication,
source cancellation and total deadlines across a decision call. A model's branch
choice is untrusted input and must pass the allowed-branch check before execution.

Observation contains business evidence, managed content, queries and valuation.
It is not all optional telemetry: AgentTurn output capture and Worker continuation
use it. `query_v2/relation.rs` also maintains relations; the directory name does
not imply a read-only boundary. Diagnostic logging is a separate concern and must
not determine business success.

## State and lifecycle rules to preserve

| Boundary | Authority / invariant | Read next |
| --- | --- | --- |
| Agent configuration | Configure/Edit publishes service state and a receipt before the file tail. Ordinary Disable conditionally restores files before withdrawing service/grants. A conflict preserves the user file and original grant; repair requires a fresh preview/operation. | [Settings transaction](../../crates/application/src/agent_connection/settings/transaction.rs), [ordering tests](../../crates/application/src/operations/tests/settings_tail.rs) |
| Native target registration | Enumerating an unused Agent target is not access authority or a global startup dependency. Actual access, durable markers and accepted-operation recovery retain strict path and permission checks. | [Artifact store](../../crates/local-storage/src/control/mod.rs), [permission boundaries](../../crates/local-storage/src/agents/native_registration_tests.rs), [real startup entry](../../crates/daemon/tests/publication_process.rs) |
| Historical Agent revoke tails | Already sealed service-first revoke tails have a separate recovery reader. Do not unify this path with ordinary Disable by treating their ordering as interchangeable. | [Legacy recovery tests](../../crates/daemon/src/control/runtime/settings_profile_legacy_tests.rs), [compatibility registry](../../contracts/compatibility-support.v1.json) |
| Committed Operation journal | The domain checkpoint and stored journal must agree in one database snapshot before publication or a native effect. Preserve immutable Plan bytes and generation/identity guards. Historical mutable JSON objects may differ only in member order; content changes must still fail. | [Domain checkpoint](../../crates/domain/src/operation/journal.rs), [storage verification](../../crates/local-storage/src/control/journal.rs), [Plan publication consumer](../../crates/local-storage/src/control/plans/publication.rs), [Skill activation consumer](../../crates/daemon/src/control/runtime/effects/tests/skill_activation.rs), [transaction guards](../../crates/local-storage/src/control/transaction_v2_tests.rs), [original upgrade fixtures](../../crates/local-storage/src/migrations/fixtures/README.md) |
| Fresh profile deletion | Untouched proof binds the current store instance and exact marker. A stage directory alone is not proof that deletion never started; unknown state after reopen fails closed. | [Profile lifecycle tests](../../crates/daemon/src/control/runtime/settings_profile_tests.rs) |
| Worker lifecycle | Result completion, OS process stop, body availability and permission to Continue are separate facts. Finalization and cleanup require the exact ownership lease/root identity. | [Finalization](../../crates/daemon/src/delegation/finalization.rs), [launcher](../../crates/daemon/src/delegation/local_worker/README.md) |
| Managed content | Every read checks current authorization and visibility. Hidden/deleted/expired content cannot be restored by cached references; native cleanup is acknowledged only after success. | [Managed-text contract](../../crates/observation/src/managed_text/README.md) |
| Storage and compatibility | Current producers have one ingestion contract. Frozen historical fixtures do not imply production upgrade support. | [Startup format admission](../../crates/local-storage/src/migrations/startup_format.rs), [compatibility registry](../../contracts/compatibility-support.v1.json) |

## Extension boundaries

**Agent ecosystems.** Treat discovery, reading configuration, importing model
sources, configuring a connection, restoring owned changes, launching a client
and running a Worker as separate capabilities. Recognizing an installation does
not authorize writes or imply Worker support. Start a new ecosystem in its native
adapter and the [Agent feature](../../apps/desktop/src/features/agents/README.md);
reuse the existing Application transaction and confirmation contracts. Verify
real configuration precedence and restoration semantics before generalizing.

Qoder illustrates why these capabilities stay separate. Its main-Agent selection
adds explicit Plan routes without importing native models or taking over the
native default. Its collaboration Skill remains independent, and Worker routing
uses transient frozen-task overrides rather than the persistent model provider.
The [shared selection](../../crates/domain/src/agents/settings.rs) defines user
intent; the [native budget policy](../../crates/domain/src/agents/qoder_model.rs)
is shared by persistent models and Workers. Do not duplicate that policy in a
renderer or turn a successful Skill check into model verification. Native live
checks must bind the original client session to the Gateway's receipt authority.
The [Gateway compiler boundary](../../crates/gateway/src/publication/trust.rs)
derives expected execution trust from the same verified compilation as request
authorization. A Plan projection's digest is not the executable alias digest;
do not reconstruct that identity in an Agent adapter or relax receipt equality.
Its persistent provider's fixed bearer header uses an explicit local model-grant
entry; ordinary Responses and transient Worker authority keep their existing
authentication channels. See [Gateway ingress authentication](../../crates/gateway/src/core_runtime/inbound_auth_tests.rs)
and [real listener authority](../../tools/e2e-harness/tests/p0_gateway_request_authority.rs)
before changing a native provider's endpoint.
The [native provider guide](../../crates/integrations/src/agents/qoder_native/README.md)
maps JSONC ownership, protected credential delivery and conditional restoration;
the [persisted-model journey](../../crates/daemon/tests/support/qoder_model_product.py)
documents the normal startup path independently of Worker overrides.
The guide also maps backend facts, protected file execution, retry and Live checks.
Installed native budgets constrain both admission and Install of later Plan updates;
a parked native file tail keeps the original Operation's Control ownership until
its exact retry succeeds. Keep these guards with the shared transaction rather
than treating native configuration as an independent file write.

**Decisions.** The external Decision API can return an allowed branch ID, but the
current product authoring, materialization and execution policies still contain
binary smart-saving assumptions. See the [Gateway map](../../crates/gateway/README.md)
for those locations. Jev currently remains an external reference service. Its
pure request preparation, scoring and validation can be studied independently of
its HTTP server; extraction does not establish an in-process runtime or a new
provider protocol.

Built-in Jev, configurable System One providers, natural-language custom branches
and tool-set selection are separate future feature work. The
[decision foundation](decision-foundation.md) records verified provider differences,
consumer ownership and the tool-selection exploration. Define
default branch, overlap, no-match, failure and re-selection behavior before
changing the authoring/publication contract. Preserve selected versus executed
branch attribution and ownership of the previous stage's assessment. Do not add
a rules DSL or a generic plugin SDK in anticipation of these tasks.
