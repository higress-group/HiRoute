# HiRoute black-box E2E

[简体中文](README.zh-CN.md)

This directory holds environment-independent product-path contracts. It sits above unit,
component, and `gateway-core` lifecycle tests and verifies user-visible behavior across a
real TCP boundary.

```text
E2E HTTP client
    -> external HiRoute SUT
       -> stock CPA Claude (one source)
          -> Anthropic-native mock
       -> stock CPA Codex (one source)
          -> Responses-native mock
```

`hiroute-e2e` does not depend on `hiroute-poc` or the `gateway-core` Rust crate. A future
replacement of the POC binary by the production daemon/Desktop backend does not require a
scenario rewrite as long as the external CLI and HTTP product contracts remain stable.

## Current coverage

[`scenarios/core-routing.json`](scenarios/core-routing.json) contains seventeen ordered
steps:

| Downstream protocol | Task | Real native mock | Expected source |
| --- | --- | --- | --- |
| Responses | simple | Anthropic `/v1/messages` | Claude-compatible |
| Responses | complex | Codex `/responses` | Codex subscription |
| Messages | simple | Anthropic `/v1/messages` | Claude-compatible |
| Messages | complex | Codex `/responses` | Codex subscription |
| Responses / Messages | large Agent envelope + simple user | Anthropic `/v1/messages` | Claude-compatible |
| Responses / Messages | tool continuation | Codex `/responses` | original Turn affinity hit |
| Responses | simple + Claude 500 | Claude `/v1/messages`, then Codex `/responses` | precommit fallback to Codex |
| Responses | function output after fallback | Codex `/responses`, no Claude Attempt | written-back Turn affinity hit |
| Responses | simple full-history append after complex | Codex `/responses` | same Session prefix remains on Codex |
| Responses | keep first two user items, replace later history with summary | Anthropic `/v1/messages` | rebuilt history reselects Claude |
| Messages | complex full-history append after simple | Anthropic `/v1/messages` | same Session prefix remains on Claude |
| Messages | edit system while history text is unchanged | Codex `/responses` | changed instructions reselect Codex |
| Responses | non-empty `previous_response_id` | no native Attempt | explicit 400 before Planner |

Every step also asserts:

1. downstream status, SSE content type, and protocol terminal event;
2. `x-hiroute-source`;
3. ordered Attempts actually observed by the native mock, including native path/model,
   response-head status, and use of that source's unique upstream authentication;
4. whether a Messages/Responses protocol conversion actually occurred;
5. each Attempt's routing receipt, including protocol, source, class, affinity, disposition,
   and status;
6. accepted-source writeback after fallback, followed by a Codex affinity hit for function
   output without probing Claude again;
7. source stability for a full visible-history append in one stable Session, and reselection
   after history reconstruction or instruction changes;
8. early rejection of unsupported Responses server-side continuation before Planner or
   native upstream, with no new ledger or receipt entry.

The current implementation proves only these basic paths, sources, and limited terminal
events. It does **not** prove full protocol-semantic fidelity. At present,
`protocol_conversion=true` means only that client and native-mock protocol names differ.
The native ledger checks only the top-level `messages`/`input` shape; it does not yet compare
Vision, tool schema/choice/call/result/ID, native reasoning fields, usage/finish reason,
provider state, or the complete SSE event sequence. Do not describe the current seventeen
steps as passed protocol-conversion conformance.

The formal P0 target is defined by the Agent integration and model protocol-conversion
contract and split into:

- `E2E-P028`: resolve AgentIntegrationProfile from exact Codex/Claude Code installations and
  valid configuration, select the unique Responses/Messages entry, apply and roll back
  configuration diffs, and execute a real cross-protocol Turn;
- `E2E-D011`: semantic fidelity for the six ingress/upstream combinations across Responses,
  Chat Completions, and Messages;
- `E2E-D012`: isolated ingress effort and exact AgentPlan reasoning configuration rendered
  natively for each target and again after fallback;
- `E2E-D013`: tool IDs/continuation, provider-state affinity, normalized streaming events,
  in-stream errors, and zero postcommit handoff.

These items must begin as failing tests with bounded schemas, a canonical Model IR corpus,
request/SSE goldens, and a native semantic ledger, then be implemented in the production
Adapter. HTTP 200, final text, a target path, or merely different protocol names are not
sufficient evidence.

The native mock ledger is the primary source-selection evidence. A response model or a
Gateway self-reported header is only independent supporting evidence. Native mocks assert
exact authentication without writing credentials to the ledger or result artifact.

## Environment contract

A Scenario describes product behavior only. It cannot contain executable paths, shell
commands, ports, delays, arbitrary response bodies, or credentials. A v2 success step allows
one or two source-native Attempts and the bounded response-head statuses
`200/429/500/502/503/504/529`; the final Attempt must be `200 Accept`. Entry-validation steps
allow only a fixed `400 + error code + 0 Attempt`, proving that the request did not reach the
Planner/native upstream. Headers are limited to bounded `session-id`/`thread-id` values and
cannot inject authentication or environment orchestration. `#[serde(deny_unknown_fields)]`,
semantic validation, and JSON Schema prevent orchestration from leaking into cases.

[`profiles/local-process.json`](profiles/local-process.json) permits only two fixed
references:

```text
HIROUTE_E2E_CPA_BIN  -> stock CLIProxyAPI executable
HIROUTE_E2E_SUT_BIN  -> external HiRoute executable
```

Each run uses dynamic ports and a 0700 TempDir, generates random capabilities, and starts two
single-source CPAs, two native mocks, and one SUT. Secret-bearing files and process logs are
0600. Every subprocess starts from a separate empty 0700 work directory used as `current_dir`,
`HOME`, XDG, and `TMPDIR`, so stock CPA cannot search upward into a repository `.env`.
Subprocesses start after `env_clear()` with a minimal environment and do not inherit real
provider tokens. External HTTP proxies point to a loopback black hole and native sources are
loopback-only. Readiness uses connection probes, not fixed sleeps; shutdown is bounded and a
drop guard performs a second kill if necessary.

The result artifact contains product evidence only, never:

- request or model-response bodies;
- capabilities or CPA/native API keys;
- ports, temporary paths, or executable paths;
- command argv or raw process logs.

## Usage

Validate the contract without prebuilt binaries:

```sh
cargo run -p hiroute-e2e -- validate \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json
```

Run the complete black-box path:

```sh
HIROUTE_E2E_CPA_BIN=/absolute/path/to/CLIProxyAPI \
HIROUTE_E2E_SUT_BIN=/absolute/path/to/hiroute-poc \
cargo run -p hiroute-e2e -- run \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json \
  --result /tmp/hiroute-e2e-result.json
```

The runner allocates dynamic ports and a private temporary directory, then starts two native
mocks, two isolated stock CPAs, and an external HiRoute SUT. Across real TCP it verifies the
four protocol/source quadrants, two Agent-envelope counterexamples, Responses/Messages tool
continuation affinity, ContextHold/rebuild invalidation, early rejection of Responses
server-side continuation, and the full
`Claude 500 -> Codex Accept -> function output hits Codex directly` path.

Recommended gates:

```sh
cargo fmt --package hiroute-e2e -- --check
cargo check -p hiroute-e2e
cargo clippy -p hiroute-e2e --all-targets -- -D warnings
cargo test -p hiroute-e2e
```

## Driving later features with E2E

Follow product-contract-first-red, implementation-green:

1. If current fields express the behavior, add a `ScenarioStep` and run it to establish a
   real not-yet-implemented failure.
2. Use the v2 bounded `attempts` for response-head fallback. Upgrade the Rust contract and
   `schema_version` with `e2e/schema/` only when a feature requires first-chunk disconnect,
   semantic SSE failure, another step type, or new evidence.
3. Never put shell, paths, sleep, fixed ports, or secrets in a scenario; orchestration belongs
   to runner/profile.
4. Prefer native-ledger assertions for routing behavior and add native-event sequence plus
   normalized-semantic assertions for protocol behavior.
5. Implement product code until the same local and CI scenario is green.
6. Scan the result artifact for sensitive data before retaining it as review evidence.

Good next scenarios include interleaved Turns without affinity leakage; the response-head and
first-semantic-SSE-error precommit fallback matrix; zero replay after commit and total Attempt
limits; arbitrary SSE byte slicing, long streams, cancellation, and memory bounds;
multimodal, `count_tokens`, `compact`, and usage/cost evidence; and real-local Codex CLI /
Claude Code smoke profiles.

The v2 native mock currently returns fixed text SSE for success and supports bounded
response-head failure. `ScenarioStep` handles only an HTTP Turn. Do not evade this boundary
with arbitrary shell fields; model a stream failure or `AgentTurn` as a new bounded enum and
upgrade the schema when needed.

## CI layers

Public CI compiles the harness on Linux, macOS, and Windows, runs mock/unit tests, and
statically validates checked-in scenarios and profiles through the CLI. This gate needs no
external binary, account, or credential.

The complete process path still requires an explicit stock CPA and HiRoute SUT via the `run`
command above. Static `validate` never represents that path as executed. After building a
production SUT from the repository, expand in layers:

- `e2e-core-routing`: build stock CPA + SUT and run the current hermetic suite;
- `e2e-product`: add fallback, tool, cancellation, memory, and related suites;
- `e2e-real-local`: manual/nightly only, using real local Agents and explicit authorization;
  it is not public CI and does not replace the hermetic suite.

## P0 Gateway exact Oracle

`schema/p0-production-manifest.json` seals the final `gateway-isolated` contract. The manifest
locks one real Responses → Responses production smoke, its profile/scenario, and the
launcher/readiness/collector/result schemas. SHA-256 values cover canonical JSON, so an
unsealed semantic change fails before startup. Old manifests, corpora, goldens, and fixture
adversaries remain historical negative inputs and cannot prove release completion.

The runner uses only the normal
`hirouted --role gateway --listen ... --publication ... --credentials ...` path. The final
contract has no `--fixture`, `--expect-status`, `expected_red`, or `skip`. Each run creates
private publication/credential input, random client/provider capabilities, an unpredictable
freshness challenge, a dynamic loopback listener, and a native provider mock. The profile
pins the Gateway product source revision. Within the binary's checkout, the runner verifies
that source and build inputs have no tracked or untracked drift from that revision, then
performs a trusted `cargo rustc --locked --all-features` rebuild with a random build nonce and
the checkout's current target/toolchain. Launcher evidence binds revision, source-tree and
build-input digests, target, toolchain, command, artifact path, and executable SHA-256.
Legitimate builds need not share a local debug hash, while wrappers and forged build inputs
cannot qualify as the SUT. Before and after readiness probing, OS socket-owner proof requires
the dynamic listener to belong to the same live child PID. `HIROUTE_E2E_SUT_REVISION` adds an
optional external revision assertion.

Provider evidence retains the complete native JSON payload and exact connection count. The
collector independently reads Lifecycle, ExecutionFact, ConversationContent, and
content-free OTel JSONL. Each must have its own producer/epoch/stream identity, contiguous
sequence beginning at 1, exact channel schema digest, strongly typed product envelope that
rejects unknown fields, unique event IDs, common request correlation, no gaps/loss, and its
own terminal. Lifecycle `request_finished` is last; after ExecutionFact `request_finished`,
only independent `usage_and_cache` is allowed. ConversationContent has only request and
response directions, each with `begin … finish`; `request_finished` is never treated as a
content barrier. Ten exact result facts project uniquely to four ordered checkpoints. Missing,
orphaned, damaged, hard-coded, wrong-ready, wrong-payload, or incomplete-terminal evidence
fails closed.

The result schema allows only scenario `green|red` and separates
`process_exit.test_process_code` from `scenario_state`. The CLI returns zero only when real
production evidence establishes green. A mismatch writes a red result and exits non-zero;
startup, provenance, readiness, or collector-contract failure is also non-zero and cannot
become release proof. The full result contains a synthetic challenge and canonical content
evidence, so it is written mode 0600; stdout contains only a body-free summary.

Validate, reseal intentionally, and run the real production smoke:

```sh
cargo build -p hiroute-gateway --bin hirouted --all-features
cargo run -p hiroute-e2e -- validate \
  --schema e2e/schema --scenario e2e/scenarios/p0-gateway.json

# Run only after intentionally changing a locked artifact. Run twice; the second run must
# produce no diff.
cargo run -p hiroute-e2e -- seal-production --e2e-root e2e

HIROUTE_E2E_SUT_BIN="$PWD/target/debug/hirouted" \
cargo run -p hiroute-e2e -- run \
  --profile e2e/profiles/gateway-isolated.json \
  --scenario e2e/scenarios/p0-gateway.json \
  --result target/e2e/p0-gateway-result.json
```

This process freezes only one production smoke and stable extension points. The broader
protocol, fault, resource, and privacy matrix remains owned by its dedicated convergence
work; this one green scenario must not be extrapolated to matrix completion.

`matrix/p0-gateway-coverage.json` is an exact independent coverage index, not a self-reported
completion flag. A frozen typed registry defines every required row. Each row references a
registry receipt that fixes real `hirouted` + native-provider execution mode, test target,
exact symbol, complete source SHA-256, and assertion identity. `seal-coverage` only projects
that registry:

```sh
cargo run -p hiroute-e2e -- seal-coverage --e2e-root e2e
```

`p0_gateway_matrix_coverage` verifies schema, complete registry projection, source digest,
symbol/test binding, and assertion identity. Missing rows or forged execution fail. The index
contains no `skip`, `expected_red`, or manual `passed` state. The personal P0 listener's
transport row is loopback H1 only; release H2 benchmarks and Enterprise TLS listeners are
outside this matrix.
