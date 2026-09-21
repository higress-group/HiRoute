# E2E case sharding

[English](SHARDING.md)

通用 `hiroute-e2e` 场景可选地在 `case_shards` 中声明完整分片。每个分片列出
自己必须一起执行的步骤；所有步骤必须恰好属于一个分片。没有该声明的场景不能
用 `--case` 切分，避免把隐含顺序、会话或回退状态误判为独立。

先查看已声明分片：

```sh
cargo run --locked -p hiroute-e2e -- validate \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json
```

为一个分片运行独立的黑盒进程时，给每次运行唯一结果文件：

```sh
cargo run --locked -p hiroute-e2e -- run \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json \
  --case responses-complex-continuation \
  --result target/e2e/responses-complex-continuation.json
```

每次 `run` 都创建自己的临时目录、native mock、CPA、SUT 和随机 loopback
端口；不同 case 只有在独立 checkout 中运行时才可以并发。结果会包含
`selected_case`，不能将多个 case 报告合并为一条完整场景的 green 证明。完整
场景仍需运行一次，以验证默认的全步骤顺序。

P0 Gateway 生产 Oracle 是一条密封的单一场景，`--case` 会被拒绝。它的独立
组件与协议测试可按 Rust integration-test target 或精确测试名分片，但最终生产
Oracle 和 seal/汇总必须保持单片、串行。
