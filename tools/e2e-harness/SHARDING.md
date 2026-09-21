# E2E case sharding

[简体中文](SHARDING.zh-CN.md)

A general `hiroute-e2e` scenario may declare complete shards in `case_shards`. Each shard
lists the steps that must execute together, and every step must belong to exactly one shard.
A scenario without that declaration cannot be split with `--case`; this prevents hidden
ordering, session, or fallback state from being misrepresented as independent.

Inspect declared shards first:

```sh
cargo run --locked -p hiroute-e2e -- validate \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json
```

Give each isolated black-box process a unique result file:

```sh
cargo run --locked -p hiroute-e2e -- run \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json \
  --case responses-complex-continuation \
  --result target/e2e/responses-complex-continuation.json
```

Each `run` creates its own temporary directory, native mock, CPA, SUT, and random loopback
port. Different cases may run in parallel only from independent checkouts. The result records
`selected_case`; results from several cases cannot be combined into one whole-scenario green
claim. Run the complete scenario once to validate its default full-step ordering.

The P0 Gateway production Oracle is one sealed scenario and rejects `--case`. Its component
and protocol tests may be sharded by Rust integration-test target or exact test name, but the
final production Oracle and seal/aggregation remain single-shard and serial.
