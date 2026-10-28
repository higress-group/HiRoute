# Select the smallest sufficient validation

Run `python3 scripts/test-plan.py --base <comparison-ref>` at the repository root.
The comparison includes committed changes since the merge base, staged/unstaged edits,
and untracked files. Renames cover both old and new paths. Inspect the JSON plan before
submitting its Cargo commands through the [configured validation entry](validation-routing.md). Use `--full` for
an explicit complete short check set. A missing or invalid base is an error, not a
reason to select nothing.

Use `--since CHECKPOINT_SHA` for a separate diagnostic `iteration` plan while retaining
the top-level final plan. See [validation feedback](validation-feedback.md) for checkpoint
semantics, explicit focused/final run associations and the required agent feedback loop.

## Selection rules

| Change | Checks |
| --- | --- |
| Root README/AGENTS, docs, durable-spec Markdown only | Review links and requirement consistency; no Rust build. A behavior change still requires the task's explicit acceptance tests. |
| Repository `.gitignore` | Publication-policy regressions and contract checks. Review ignored paths and any affected producer or packaging behavior; ignore rules alone do not change tracked product sources. |
| Known repository agent instructions and shared validation skill | Link, instruction-consistency and host-discovery checks; no product build from instruction paths alone. Known skill scripts retain their own tests; unknown or product-installed skill paths retain consumer investigation. |
| `apps/website`, website/release workflows and their `.github/scripts` | Dedicated website Node tests/build, static browser checks and publication-script checks; no Desktop frontend or Rust build. Decision API/source changes keep their own broader contract scope. |
| `apps/website/data/releases.json` | Website checks plus `hiroute-host-runtime` library tests and package Clippy, because the native upgrade reader consumes this catalog; no Desktop build. |
| Release contract snapshots under `contracts/releases/`, their generator/PR publisher and release workflow | Snapshot/source/index checks and isolated Git/PR-publisher regressions; no Rust or Desktop build. Release workflow edits also retain the existing website workflow checks. Full plans include these contract-record checks. |
| Release candidate workflow and bounded packaging/build/installer scripts | CPA target mapping, candidate-to-publication, Desktop inventory, standalone installer and license regressions; no Rust product suite. Website consumers keep their dedicated workflow. Changes to the pinned CPA source or runtime code retain their broader checks. |
| Desktop frontend | Frontend build and tests; this is not native Desktop evidence. |
| CLI/client core | CLI, client core and daemon tests, plus the default production smoke once. |
| Integrations | Integrations, application, CPA bridge, storage, release-facts, daemon and CLI tests, plus smoke. |
| Storage | Storage, observation, application, daemon, client core and CLI tests, plus smoke. |
| Observation | Observation, daemon, client core and CLI tests, plus smoke. |
| Daemon | Daemon and CLI tests, plus smoke. Plans running the daemon package suite also build the candidate console required by installed management tests. |
| CPA bridge | CPA bridge, daemon, integrations and CLI tests, plus smoke. |
| Gateway/core | Both Gateway packages, protocol E2E, integrations, CPA bridge, storage, release-facts, daemon, CLI and smoke; retain the distinct no-default-features loopback. |
| Native Desktop | Frontend, CLI/client core/daemon and core smoke, plus the configured-Mac `desktop.compile` gate and affected native behavior checks; do not automatically expand to all backend packages. Hosted success does not prove native behavior. |
| Shared Worker API and domain harness/launch contracts consumed by Desktop | Preserve backend scope and add `desktop.compile`, even when no Desktop file changes. This bounded mapping does not make every shared-library change a native obligation. |
| Diagnostics implementation/settings/filtering/writer and its tests | Diagnostics package tests and Clippy, plus Pilot launcher tests; no unrelated Worker/Gateway E2E or frontend build. Add the release-profile regression when changing the build default. |
| Diagnostics event/record/identity/error contracts | Full short set by default; review cross-boundary consumers. |
| Hosted backend workflow, shared Linux setup and execution adapters (`gateway-core.yml`, `ci-setup/action.yml`, `ci-prerequisites.py`, `ci-run.py`, `ci-shards.py`) | Full hosted matrix, because these files decide what backend validation actually runs; their unit tests alone do not prove the real Actions entry path. |
| `scripts/test-plan.py`, `scripts/test-test-plan.py`, hosted adapter tests, and the operator skill `.agents/skills/hiroute-integrate/SKILL.md` | Selection and shard-coverage regression tests; no Rust or frontend build when the execution adapters themselves are unchanged. |
| HOME validation routing and local/SSH Rust runner tooling | Routing, runner and Pilot launcher Python regressions; no Rust/frontend build unless the change also affects product code. |
| Pilot CPA manifest producer/consumer, reusable-build tests or selection rules | Default local Pilot checks use fake build/launch tools. In Actions (`GITHUB_ACTIONS=true`), affected paths and full plans additionally select the explicit `test-pilot-builds.py --real-consumer` command; its single real Rust case is required and cannot pass by skipping or selecting nothing. No full Desktop acceptance is added. |
| Bounded manual local GC wrapper | Wrapper and shared local-runner Python regressions; deletion policy remains in the runner. No Rust/frontend build. |
| Bounded Gateway runtime/protocol tests, routing fixtures, Responses request helper | Package Clippy, explicit target/consumer tests and coverage gates; runtime test changes also select the protocol matrix, and the Responses helper retains the coverage gate and indirect smoke consumer. Gateway runtime includes the known `serde_json/preserve_order` combination from #94. |
| Real Worker test entries and native-context fixtures | Bounded package compilation plus separate required `product_checks`; ignored native cases are never counted as ordinary Cargo success. Shared native-context fixtures select both Codex/Claude and Qoder consumers. |
| Installed native routing adapters and Gateway wire/history owners | Preserve ordinary package/Worker consumers and add explicit actual-client `native_checks`. DSH also requires its real Web/Cordis isolated LLM main/title path; ACP alone cannot establish that path. |
| Qoder native integration, its Worker profile, and shared collaboration settings/Skill boundaries | Keep ordinary owner checks and require the Qoder main-Agent/Worker journey; model-specific Codex/Claude settings do not automatically select this gate or all older Worker journeys. |
| Shared application/domain/API, Cargo manifests/lockfile, other unclassified scripts, shared E2E harness, assets/contracts, unknown paths | Conservative full proposal until impact is understood; do not equate an unclassified path with proven cross-product impact. |

Mixed changes take the union. The tool proposes checks from paths; it cannot infer the
semantics of every edit. Review the diff and consumers before starting costly checks:

- **Docs only:** links and consistency. Instructions do not count as implementation evidence.
- **Local implementation:** affected behavior, package tests and actual consumers. A shared
  library is not automatically a shared protocol. Logging defaults/filters do not require
  unrelated routing or Worker E2E. Tests that assert an explicit logging threshold remain.
- **Cross-boundary contract/dependency:** public API shape, serialized event/config shape,
  authentication, routing semantics, process lifetime or dependency changes require their
  consumers and explicit gates; use the full set when several boundaries are affected.
- **Unclear impact:** investigate callers and tests first. If impact remains unknown, retain
  the full set. Do not narrow merely because a test is slow or failing.

For an evidence-based adjustment, record changed behavior, affected consumers, selected
commands and why omitted checks cannot observe this change. No extra user permission is
needed for this routine decision. CI still uses the conservative generated plan; add a
bounded classification with regressions for recurring known cases instead of repeatedly
ignoring it. An implementation change inside a normally focused file can still require
`--full` or additional consumer/native checks when its actual semantics cross a boundary.
Embedded Markdown and product Skills are not automatically docs-only.
The mapping is implemented in `scripts/test-plan.py` and regression-tested by
`python3 scripts/test-test-plan.py`.

For focused observation-library checks on many-core hosts, explicitly use
`-- --test-threads=8`, matching the existing workspace DAG's per-library bound.
Each isolated store has its own query workers and a real 1 s query deadline;
starting one test per host CPU can create dozens of concurrent SQLite stores and
interrupt otherwise bounded reads. Keep deadline/cancellation tests and their
production limits intact. Diagnose suspected scheduling failures with the same
candidate, feature graph and full selected library suite at both thread settings;
one passing isolated test does not establish contention. Run process targets
separately with their documented one-thread setting.
The final workspace DAG runs the complete Observation library at eight threads after
the isolated CPA library and before preparation/other target processes. This isolates
external workbench load without changing query limits, fixtures, assertions or coverage.
Its target records retain start/end times; historical runs without these timestamps
cannot establish precise query-phase resource contention from a later passing rerun.

Every non-empty change plan also runs the contract-convergence unit tests and
`python3 scripts/check-contract-convergence.py`. These are repository contract checks, not Rust
evidence, so docs-only and frontend-only changes retain their existing `rust: false`
classification.

## Explicit native Agent product gates

`product_checks` records required opt-in journeys separately from ordinary `commands`.
The selector uses one entry per test target and bounded fixture-consumer mappings. Review
these obligations even when ordinary package tests pass: an ignored target with zero
executed cases is not product evidence. Missing installations or required environment
leave the gate failed or unexecuted, never green. The selector retains the gate when
those environment variables are absent; the product runner enforces their validity.

The Qoder entry selects all five tests in `qoder_delegation`: the installed user Skill
calling the public HiRoute CLI to delegate real work, native Worker skills with exact
Continue of the frozen task, independent concurrent routes with cancellation of
only owned work, native tool summarization/automatic compaction on the frozen route, and ordinary
startup using persisted additional model routes through the production Gateway.
The fourth journey requires both `worker.context.tool-summary-route` and
`worker.context.compaction-route`, including Continue from the persisted native summary.
Its `harnesses` value is `["qoder"]`; Codex/Claude evidence does not
replace this entry. Run the printed command through the configured validation runner:

```sh
cargo test --locked -p hiroute-product-e2e --test qoder_delegation -- --ignored --nocapture --test-threads=1
```

The [Qoder journey guide](../tools/product-e2e/tests/QODER_DELEGATION.md) defines setup.
It requires the exact full `HIROUTE_PRODUCT_CANDIDATE_SHA`, an absolute installed
`HIROUTE_WORKER_QODER_BINARY`, and explicit existing `HIROUTE_QODER_CONTEXT_HOME` and
`HIROUTE_QODER_CONFIG_DIR` directories with normal native login. Do not copy authentication
material or fall back to an unrelated daily context. No adapter or Node installation is
required for Qoder. The fifth journey additionally requires `HIROUTE_QODER_MODEL_CONTEXT_HOME` and
`HIROUTE_QODER_MODEL_CONFIG_DIR`: a dedicated, normally logged-in acceptance context
whose settings may be changed and restored. Borrowed daily context for a read-only
Worker journey does not authorize writing persistent model providers. No transient
provider or credential override can substitute for this fifth journey.
Record all five journey outcomes and their revision, not only the
Cargo exit code. Python fixture tests check the oracle, not the installed Agent journey.

The OpenCode entry selects all seven tests in `opencode_delegation`: saved ordinary
Responses/Messages routes and independent restore, the installed user Skill calling
the public HiRoute CLI, native Worker tools with exact Continue for both protocols,
concurrent routing/cancellation and missing-history refusal, stale-safe static API
import, and automatic native compaction plus auxiliary calls on the frozen route.
The [OpenCode journey guide](../tools/product-e2e/tests/OPENCODE_INTEGRATION.md)
defines the selected official installation, isolated context and per-case evidence.
Run every required case; a successful build or an ignored test does not replace it.

Shared `native_context_*` fixtures retain their registered Codex/Claude, Qoder,
Pi, DeepSeek Harness and OpenCode native consumers;
`worker_product_support` and shared delegation/runtime setup retain all known Worker
consumers. Qoder-only fixtures and collaboration settings select the Qoder gate without
adding the old Codex/Claude lifecycle/read journeys. Registered journey guides and files
under `docs/` remain documentation-only; changing executable fixtures is different.

`native_required` remains the Desktop compilation flag for changes under
`apps/desktop/src-tauri/` and the known shared Worker contracts consumed by that
entry. `native_checks` includes the separate `desktop.compile` command template
through the configured Mac, enabling `desktop-pilot`
(which includes `desktop-runtime`) to match the managed frontend/Pilot ACL configuration,
and compiling all targets, including test consumers. Run it early after shared
contracts integrate; [native compilation](validation-routing.md#native-consumer-compilation)
explains setup and evidence reuse. These commands stay outside backend `commands`;
hosted backend success does not execute them. A docs-only `--since` iteration does
not remove the top-level final obligation.

Gateway wire/history owners also select actual native Gateway obligations for
Codex, Claude, Qoder, Pi, DSH, OpenCode and Hermes. Installed adapter changes select
their native client and existing Worker consumers. Each native obligation names
saved-route use, same-session followup, tools, auxiliary calls and independent
Restore. DSH additionally names actual Web/Cordis isolated LLM main/title calls.
The printed acceptance-command placeholder must be replaced with the affected
native journey, retaining its real business oracles and exact candidate identity.
An empty test selection, preparation, `--version`, an SDK fixture or ACP alone
does not pass those obligations. Full backend mode and the integration view retain
these checks; neither runs them or waives an unexecuted case.

Use `scripts/native-agent-scope.py` to prepare a fresh root outside daily HOME for
an authorized native command. Pass its absolute entry and exact read-only runtime
dependencies with repeated `--read-only` arguments. Preparation creates independent
HOME, native configuration, XDG, session, project, temporary and runtime directories
without launching a process. `--execute` starts the entire driver and descendants
under macOS file/signal containment, in an independent process session. It captures
stdout/stderr privately before even driver imports or guard initialization. The
driver still owns distinct ports, bounded process cleanup and its business verdicts.
The entry inherits no provider credentials, proxy or client overrides. Populate
only the owned native configuration through the journey's normal product path.
Use `--expose node=/absolute/selected/node` or `--expose rg=/absolute/selected/rg`
when a driver needs those installed tools on its private PATH; ambient PATH is
not inherited, and each exposed entry adds only its exact read-only file.
Keep daily configuration/session directories outside dependency exceptions.

On macOS, wrap native launches with `scripts/native-agent-supervisor.py` to retain
a live session anchor through descendant cleanup. For native Stop, optionally pass
`--native-control-fd FD` with a private read-only pipe above the standard descriptors.
Keep its writer only in the controller: byte `I` requests SIGINT to the anchor's
still-live native child; closing it revokes the session. It is not inherited by the
native process and does not replace stdin/stdout. The dispatch event proves signal
delivery was requested, not a native acknowledgement or an interrupted HTTP result.
Signals to the guardian mean cleanup, not native Stop. Check those facts separately.

Daily guards retain their original observations and every later delta. On a host
being used concurrently, an unexplained change to a preexisting file or process
remains `changed_unattributed`; it cannot establish which process caused it. A new
isolated execution also takes an explicitly separate immediate before/after file
observation, without replacing or relabeling the original record. Admission still
requires the immutable launcher, exact owned-process authority and actual kernel
denials of out-of-scope writes, daily configuration reads and unrelated signals.
Violating those enforced boundaries stops execution. Global process survival and
file changes remain separately reported, never silently waived or marked unchanged.
If an existing controller needs exact baseline process
identity, `ExactProcessObserver` in that module reads only requested PIDs through
macOS libproc on the trusted controller. It launches no process and grants no
signal authority. Do not send `/bin/ps` back into a sandboxed worker. Missing or
inaccessible identity leaves the guard red; it cannot identify an unknown writer.
`Product.close` retains original output frames for both temporary and explicit
roots, and cleanup/export failures cannot replace the original business failure.

The mapping is intentionally bounded, not automatic dependency discovery. Review
new or moved contract consumers and extend it where justified. Neither field is an
umbrella for native Agent execution: `native_required: false` cannot waive
`product_checks`, and a successful headless Agent journey cannot replace required
Desktop acceptance.

### Native preparation and recovery evidence

Before enabling real model requests, exercise the actual entry, interpreter,
imports and preparation under the same containment as the business run. Keep
physical model admission closed. A compile-only check or a mocked preparation
sequence cannot prove this boundary. Complete-state migration follows final
HOME/XDG selection and precedes any native command that can initialize those
directories, including version discovery. Preserve the source state; compare
migration bytes separately from the complete configuration baseline established
before Apply. Controller-only inputs stay on the controller; the worker verifies
its permitted exact-byte copies without widening filesystem access.

Native state includes its lookup relationships, such as a project directory's
encoded working-directory key, not just journal bytes. Prove that the real client
loads the original session in the final scope. Preserve historical fields and
native error rows; a resume proof may exclude a row only when the native replay
contract excludes it, without deleting it from the journal. SQLite read-only
connections can still update shared-memory coordination files. Inspect immutable
evidence from a stopped client only after copying its complete database state;
an active database requires a consistent snapshot, not piecemeal file copying.

Keep cancellation observations separate: requesting Stop, receiving its
acknowledgement, observing native interruption, actual HTTP closure, provider
completion and cleanup are different events. Assert their required causal order,
not one client's incidental total ordering. A client may close HTTP before its
Stop acknowledgement, or only after consuming another genuine stream event.
Cleanup-induced closure does not prove native cancellation. Join a reader if it
was started; an uncreated reader needs an explicit not-started observation.
Reasoning, tool and text progress are distinct from terminal or unknown events;
successful forwarding alone does not prove native consumption.

Require only acknowledgements the selected native interface actually provides.
A quiet CLI signal handler need not print an interruption message. Persist the
owned signal action, observed process result and transport events before checking
them; a missing exit code or event timestamp cannot be reconstructed as fact.

Separate execution evidence from model answer quality. Prove a tool with its
actual call/result, exit or artifact effect and the subsequent request carrying
that result. A model misquoting a nonce in its final answer does not erase a
successful tool execution. Record that answer error separately; do not replace
the execution proof with an exact final-text echo. Default-model acceptance needs
the actual saved model, coherent protocol completion and native receipt of the
real response. These checks do not establish answer correctness or semantic
scoring accuracy; an empty, truncated, substituted or unaccepted response still
fails the execution gate.

Prepare recovery actions and their baseline dependencies before the first
potentially committed mutation, including native model selection. Recovery uses
its own bounded steps even when the business deadline has expired. An Apply
acknowledgement is not the configuration commit. After an uncertain Apply outcome,
query actual production state before Restore; an unavailable query
is unknown, not proof that nothing changed. Record the first business failure,
Restore, restored-default inference and owned cleanup independently. A later
cleanup error must neither replace the first failure nor disappear behind a
green report. Require the controller and worker exit results as well as each
business gate. Business failure stops its subsequent requests; an independently
authorized default-inference reserve remains subject to successful complete
Restore and the unchanged physical-call ledger.

## Integration preflight

ContextHold, decision-history and runtime selection changes select the ContextHold
listener target before final workspace execution. Its cleanup journey distinguishes
the free-first recovery contract from classified causal branches; a shared session
or contextual score target alone cannot authorize provider-history deletion. Direct
ContextHold test changes retain the protocol/coverage consumers, serialized process
execution and the known `serde_json/preserve_order` feature context.

Use `python3 scripts/test-plan.py --base BASE_SHA --integration` after combining task branches.
The added `integration` view prints early checks without modifying the top-level final commands
or running anything. Selected Python gates precede the coverage-manifest target; the protocol
and runtime targets follow in one bounded Cargo command. Native/frontend final requirements
remain visible at the top level. See the [integration skill](../.agents/skills/hiroute-integrate/SKILL.md)
for branch convergence and evidence bookkeeping.

Known early consumers include Gateway adapters/model IR, runtime/protocol test changes,
the Responses helper and the coverage producer/manifest/schema. This is a small supported
mapping, not comprehensive dependency discovery. Unknown changes still retain conservative
final expansion; no early targets is not evidence that a branch is safe. Runtime changes
select the protocol matrix as well as their coverage gate because protocol semantics can
invalidate cross-ingress assertions while runtime tests pass.

Command descriptor changes select both the CLI `contracts` and Product E2E `product_oracle`
targets early. Regenerate both `contracts/cli/` and `e2e/product/schema/`: they share the
command descriptor digest, so updating only the CLI outputs leaves the product contract stale.
Observation domain ports select the Product E2E `local_observation` target early to
check the current boundary golden against the canonical port digests. Retained legacy
observations keep their original digests and remain separate compatibility evidence.
Gateway observation ports and their current Oracle/JSON schema consumers also select
the exact producer-to-consumer pin regression early. Legacy sealed pins remain unchanged.

For a full/group plan, early tests preserve the final Cargo package scope, adding the E2E
owner if needed, and restrict execution with explicit `--test` targets. Focused runtime
checks preserve the known `serde_json/preserve_order` combination. This reduces late failures;
it does not promise a cheap cold build or complete feature equivalence in every context.

After an observed failure, `--integration --collect-failures` explicitly adds `--no-fail-fast`
to the bounded protocol/runtime command only. Review the proposed scope before submitting
through `validation.py`. Coverage runs first; repair a failed coverage gate before continuing.
Collection does not continue later commands, include expensive product smoke, or establish a
complete workspace failure set. Final commands retain their default fail-fast policy.

Combine `--since CHECKPOINT_SHA --integration` to base early checks on the iteration diff;
the top-level obligations and Cargo package context remain against BASE_SHA, so narrowing
the changed paths does not silently narrow dependency features. Both integration options are planning-only
and unavailable with hosted CI execution/output. Existing CI continues to execute the final
plan and its selection regressions, with no second smoke invocation.

Preflight success does not waive final obligations or automatically reuse evidence. Keep
command, SHA, run ID, package/features/platform, outcome and remaining checks together in the
existing task handoff. Small early checks may repeat inside final workspace validation;
reserve costly product runs for affected acceptance paths and unresolved diagnosis. Preserve
original evidence and explain intervening changes before reusing unchanged-code results.

When expansion selects the complete locked backend workspace test, the SSH workbench
can run that exact command with `--schedule workspace-dag`. The executor checks the
compiled target list and each target's full case-name set, runs doctests separately,
and requires the three real smoke cases through the existing `smoke_cli` target.
Preparation is build evidence only. Its published binaries are verified by role;
ordinary tests cannot silently build replacements. A failing target stops admission
of later targets by default, so its unexecuted set remains an obligation. Use an
explicit bounded diagnostic scope before a new full final attempt. The older
`workspace-smoke` schedule is an explicit rollback and does not inherit a failed
DAG run's evidence.

## Failure policy and completeness

Validation phase, selected scope and failure policy are separate choices. A group/full
plan or `--phase final` does not automatically enable failure collection.

| Purpose | Failure policy | Evidence requirement |
| --- | --- | --- |
| Routine iteration | Default Cargo fail-fast | Classify observed failures; an early stop leaves the failure set incomplete and remaining checks pending. |
| Explicit batch diagnosis | Caller adds `--no-fail-fast` for a declared scope | Collect failures across that Cargo command's selected test executables; structured feedback still arrives only at command completion. |
| Final candidate validation | Default Cargo fail-fast | Any failed or unexecuted required check prevents final success; all selected checks and explicit gates need passing evidence. |

`--no-fail-fast` continues other selected test executables after a test failure. It does
not continue later validation commands: `test-plan.py --run-ci` returns after the first
failed command. Build failures, timeouts, interruption or missing output can leave even
an explicit collection incomplete. Claim a complete failure set only for a declared
scope whose required tests finished with recorded outcomes; unknown coverage is not
completion or passing evidence.

After failure, record observed failures and remaining or unknown coverage. Decide whether
batch diagnosis needs an explicit collection run, preserving the candidate SHA and required
build context. Select its scope from the diagnostic need; do not automatically restart full
just to fill a report. Batch related fixes and keep unexecuted checks as final obligations.

## Iteration, merge and specialist checks

Use three explicit validation phases:

1. **Diagnostic:** inspect the current run's observed failures and execution limits.
   Classify branch-caused failures, exact-base failures and external blockers before
   editing. Use explicit batch collection when needed under the policy above; a default
   fail-fast run need not exhaust its selected scope before diagnosis or fixes begin.
2. **Focused:** batch related fixes, then run only affected package/downstream checks,
   deterministic generators and explicit task gates. Intermediate pushed revisions are
   diagnostic candidates, not final evidence.
3. **Final:** freeze the candidate and complete the selected checks once, pushing first
   only when managed Rust validation requires it. A focused scope stays focused at final
   validation; “final” does not mean full workspace. If a check fails, classify observed
   failures, decide whether explicit collection is needed, batch fixes and validate the
   affected next candidate. Remaining checks still need passing evidence before final
   success. Docs/tooling-only follow-ups run their own checks; preserve unchanged Rust
   evidence under its original SHA.

This sequencing does not waive required assertions or explicit task acceptance gates. Do not
skip a selected command or unique assertion, relabel evidence from an earlier SHA, or rerun an
unaffected generator merely because an unrelated test-only edit changed the candidate SHA.

- PR CI computes the affected plan against the PR base. Affected plans remain linear;
  complete plans run every backend target exactly once across coverage-checked hosted
  shards, including the E2E packages. The separate Gateway
  diagnostics workflow is manual/reusable; it no longer duplicates these tests on
  every PR. No-default-features transport testing remains distinct.
- Minute-scale stability, high connection counts and fixed-machine performance are
  specialist checks: run them when changing resource lifetime, buffering, transport,
  concurrency or performance-sensitive behavior, or when explicitly required for release.
- Real Agent/account, native Desktop and installation tests retain their own environment
  requirements. A synthetic peer, valid JSON contract or missing dependency is not a
  substitute for a successful production journey.

## Examples of sufficient scope

A Debug default change in diagnostics needs package tests that cover actual event filtering,
startup, persisted values, damaged settings and explicit overrides, plus Pilot launcher
checks and the release-profile default regression. Expand to Desktop/native validation only
when its entry, UI or lifecycle behavior changes. Do not add unrelated model requests,
Worker lifecycles, Gateway protocols or frontend builds solely because diagnostics is shared.

Selection rule edits need `python3 scripts/test-test-plan.py`, a generated plan for the
actual changed paths, and regressions for focused, mixed, unknown and explicit-full cases.
Review the CI selection job too: it already runs these regressions before deciding whether
to start Rust/frontend jobs. No Rust build is needed to validate this Python mapping.

If a run is stopped by the user, preserve its evidence and mark it incomplete regardless
of partial passing counts. Do not restart or replace it with another broad run.

## Keep these quality boundaries

Do not remove the only assertion for authentication/credentials, preview zero side effects,
Apply idempotency/crash recovery, precommit fallback/postcommit no replay, protocol/tool IDs,
context isolation, Worker cancel/continue, content retention or bounded resource cleanup.
Move an assertion to its owning component or a surviving core journey before removing its
old test. A duplicate launcher can be removed; a unique failure condition cannot.

The quick smoke currently contains three real cases: Gateway controlled response,
CLI/control discovery, and invalid release rejection. It runs once in `smoke_cli`; all
scope/step/report assertions remain there. Explicit case selection is still available for
diagnosis, while selection correctness uses cheap `validate` tests. The quick set does not
claim full model-save, Agent restore, Worker lifecycle, record-cleanup or Desktop coverage.
Those behaviors continue to rely on their owning package tests and required product checks.

Full and Gateway selections also retain the complete core-routing definition check:

```sh
cargo test --locked -p hiroute-e2e --test case_shards \
  core_routing_validation_covers_the_complete_unsharded_scenario -- --exact --nocapture
```

Submit this through the configured managed backend runner. The existing
[case-selection CLI tests](../tools/e2e-harness/tests/case_shards.rs) invoke the candidate's
`validate` command with the full scenario and local-process profile, without `--case`.
The 17 steps and 10 declared shards are contract counts, not executed business steps;
this check does not start CPA/SUT or replace production smoke. The Cargo test entry
is accepted by both managed runners without admitting arbitrary `cargo run` commands.

Historical sealed production smoke is explicitly ignored unless requested with its SUT.
When explicitly selected, missing SUT fails. Current-candidate smoke remains the normal
quick entry; it cannot be replaced by a historical result or a fixture success.

## Diagnostic logs

Development/test runtimes default to Debug when no saved level exists. Product smoke
uses the isolated Pilot Debug override. Before retrying a product-path failure, follow
[diagnostic troubleshooting](diagnostic-troubleshooting.md) to verify applied levels,
inspect both roles and distinguish business failure from missing diagnostic output.
Tests without a diagnostics runtime still use their assertion and runner output.

## Build and evidence rules

For Rust execution use [HOME-configured routing](validation-routing.md); it selects
the existing SSH workbench or local-rust runner on the execution host. `test-plan.py --run-ci` is restricted to Actions and
uses the independent [CI adapter](github-actions-validation.md). Selection changes no SSH
configuration, checkout retention, capacity locks, or target directory rules.

The first reduction removes repeated executions and unused binaries. Trusted build
attestation, per-run random challenge, output identity, private directories and cleanup
remain unchanged. Do not bypass these checks using an arbitrary cached binary. Further
Build reuse is available only when explicitly retaining and leasing the same managed
backend checkout, as described in [validation routing](validation-routing.md#reuse-one-retained-backend-checkout-for-product-smoke).
The smoke runner verifies a per-candidate recipe and staged binary digest for each
package. Gateway and Control each retain their own `hirouted` binary; a changed or
corrupt receipt fails instead of silently rebuilding. Separate checkouts still share
compiler objects only through sccache.

Report command exit and case states separately, with exact revision, expected/actual cases,
and failure location. Zero selected required tests, unavailable required environments, or
missing scenario results cannot produce a green verdict. Preserve old evidence under its
original SHA; never relabel it after retry.
