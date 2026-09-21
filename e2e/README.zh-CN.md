# HiRoute black-box E2E

[English](README.md)

这个目录保存环境无关的产品路径合同。它位于 unit、component 和 `gateway-core` lifecycle 测试之上，从真实 TCP 边界验证用户最终看到的行为。

```text
E2E HTTP client
    -> external HiRoute SUT
       -> stock CPA Claude (one source)
          -> Anthropic-native mock
       -> stock CPA Codex (one source)
          -> Responses-native mock
```

`hiroute-e2e` 不依赖 `hiroute-poc` 或 `gateway-core` Rust crate。将来 POC binary 被正式 daemon / Desktop backend 替换时，只要维持外部 CLI 与 HTTP 产品合同，scenario 不需要重写。

## 当前覆盖

[`scenarios/core-routing.json`](scenarios/core-routing.json) 包含十七个顺序步骤：

| 下游协议 | 任务 | 真实 native mock | 预期来源 |
| --- | --- | --- | --- |
| Responses | simple | Anthropic `/v1/messages` | Claude-compatible |
| Responses | complex | Codex `/responses` | Codex subscription |
| Messages | simple | Anthropic `/v1/messages` | Claude-compatible |
| Messages | complex | Codex `/responses` | Codex subscription |
| Responses / Messages | large Agent envelope + simple user | Anthropic `/v1/messages` | Claude-compatible |
| Responses / Messages | tool continuation | Codex `/responses` | 原 Turn affinity hit |
| Responses | simple + Claude 500 | Claude `/v1/messages` 后 Codex `/responses` | precommit fallback 到 Codex |
| Responses | fallback 后 function output | Codex `/responses`，无 Claude Attempt | 回写后的 Turn affinity hit |
| Responses | complex 后追加 simple 完整历史 | Codex `/responses` | 同 Session 前缀保持 Codex |
| Responses | 保留前两条 user、替换后续摘要 | Anthropic `/v1/messages` | 完整历史重建后重新选择 Claude |
| Messages | simple 后追加 complex 完整历史 | Anthropic `/v1/messages` | 同 Session 前缀保持 Claude |
| Messages | 编辑 system、历史正文不变 | Codex `/responses` | 指令变化后重新选择 Codex |
| Responses | 非空 `previous_response_id` | 无 native Attempt | Planner 前返回明确 400 错误码 |

每一步同时断言：

1. 下游状态、SSE content type 与协议终止事件；
2. `x-hiroute-source`；
3. native mock 实际收到的有序 Attempt、native path、native model、response-head status，以及 CPA 使用了该 source 独有的上游鉴权；
4. 是否确实发生 Messages / Responses 协议转换；
5. 每次 Attempt 对应的 routing receipt，其 protocol、source、class、affinity、disposition 与 status；
6. fallback 后接受来源会回写 Turn affinity，function output 续轮直接命中 Codex，不再探测 Claude；
7. 同一稳定 Session 的完整可见历史 append 跨 simple/complex 分支保持实际成功来源，历史重建或指令编辑则重新选路；
8. 不支持的 Responses 服务端续链在 Planner 和 native 上游前拒绝，ledger 与 receipt 均不增加。

上述当前实现只证明基础路径、来源与有限终止事件，**不能**证明协议语义已经保真。当前
`protocol_conversion=true` 仅表示客户端协议与 native mock 协议名称不同；native ledger 只检查
`messages`/`input` 顶层形状，尚未逐项比较 Vision、Tool schema/choice/call/result/ID、
reasoning 原生字段、usage/finish reason、Provider state 和完整 SSE 事件序列。因此不能把当前 17-step suite
表述成正式协议转换 conformance 已通过。

正式 P0 目标由
[`Agent 接入与模型协议转换合同`](../design/product-contract/protocol-conversion-contract.md)定义，并拆成：

- `E2E-P028`：从 exact Codex/Claude Code 安装版本和有效配置解析 AgentIntegrationProfile，分别选择
  Responses/Messages 唯一入口，执行配置 diff/apply/rollback 与真实跨协议 Turn；
- `E2E-D011`：Responses/Messages 入站到 Responses/Chat Completions/Messages 上游的六路径语义保真；
- `E2E-D012`：入站 effort 隔离、AgentPlan exact reasoning configuration 的逐目标协议原生渲染与 fallback 重渲染；
- `E2E-D013`：Tool ID/续轮、Provider state 亲和、统一流式事件、流中错误和 postcommit 零接力。

这四项必须先以有限 schema、canonical Model IR corpus、request/SSE golden 和 native semantic ledger 建成红灯
测试，再实现正式 Adapter。只看 HTTP 200、最终文本、目标 path 或协议名称不同均不构成通过证据。

来源判定以 native mock ledger 为主证据，不信任 response model 或 Gateway 自报 header；后两者只是相互独立的旁证。Native mock 只断言鉴权是否完全匹配，不把任何 credential 写入 ledger 或结果 artifact。

## 环境合同

Scenario 只描述产品行为，禁止 executable path、shell command、端口、延时、任意响应体和凭据。v2 的成功步骤只允许 1–2 次 source-native Attempt，以及有限的 response-head status：`200/429/500/502/503/504/529`；最终 Attempt 必须是 `200 Accept`。入口预校验步骤只允许固定 `400 + error code + 0 Attempt`，用于断言请求没有进入 Planner/native 上游。请求头只允许有界的 `session-id`/`thread-id`，不能注入认证或环境编排。`#[serde(deny_unknown_fields)]`、语义校验与 JSON Schema 共同防止环境编排逐渐渗入用例。

[`profiles/local-process.json`](profiles/local-process.json) 只允许两个固定引用：

```text
HIROUTE_E2E_CPA_BIN  -> stock CLIProxyAPI executable
HIROUTE_E2E_SUT_BIN  -> external HiRoute executable
```

Runner 每次使用动态端口与 0700 TempDir，生成随机 capability，启动两个 single-source CPA、两个 native mock 与一个 SUT；所有 secret-bearing 文件和进程日志为 0600。每个子进程都有独立、启动时为空的 0700 work directory，并同时作为 `current_dir`、`HOME`、XDG 与 `TMPDIR`，因此 stock CPA 不会向上搜索并读取仓库 `.env`。子进程从 `env_clear()` 的最小环境启动，不继承开发者 shell 中的真实 Provider token；外部 HTTP proxy 被指向 loopback blackhole，native source 只允许 loopback。进程 readiness 使用连接探测，不依赖固定 sleep；退出有界并由 drop 做二次 kill 保证。

结果 artifact 只保留产品证据，不包含：

- 请求或模型响应正文；
- capability / CPA / native API key；
- 端口、临时路径或 executable path；
- command argv 或原始进程日志。

## 使用

验证合同（不要求已有二进制）：

```sh
cargo run -p hiroute-e2e -- validate \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json
```

运行完整黑盒链路：

```sh
HIROUTE_E2E_CPA_BIN=/absolute/path/to/CLIProxyAPI \
HIROUTE_E2E_SUT_BIN=/absolute/path/to/hiroute-poc \
cargo run -p hiroute-e2e -- run \
  --scenario e2e/scenarios/core-routing.json \
  --profile e2e/profiles/local-process.json \
  --result /tmp/hiroute-e2e-result.json
```

Runner 每次分配动态端口和私有临时目录，启动两个 native mock、两个隔离 stock CPA 和一个外部 HiRoute SUT。它用真实 TCP 验证四个协议/来源象限、两种 Agent envelope 反例、Responses/Messages 工具续轮的 affinity hit、context hold/重建失效、Responses 服务端续链早拒绝，以及 `Claude 500 -> Codex Accept -> function output 直接命中 Codex` 的完整链路。

建议的质量门禁：

```sh
cargo fmt --package hiroute-e2e -- --check
cargo check -p hiroute-e2e
cargo clippy -p hiroute-e2e --all-targets -- -D warnings
cargo test -p hiroute-e2e
```

## 用 E2E 驱动后续功能

新增能力时遵循“产品合同先红、实现后绿”：

1. 用现有字段可表达时，先添加一个 `ScenarioStep`，运行确认因未实现行为而失败。
2. response-head fallback 直接使用 v2 的有限 `attempts`；新能力需要首包断连、语义 SSE failure、更多 step 类型或新的证据时，再升级 Rust contract 与 `schema_version`，同步更新 `e2e/schema/`。
3. 不在 scenario 中加入 shell、路径、sleep、固定端口或 secret；编排能力属于 runner/profile。
4. 对路由能力优先增加 native ledger 断言；对协议能力同时增加原生事件序列和统一语义断言。
5. 实现产品代码，直到本地与 CI 的同一 scenario 通过。
6. 结果 artifact 通过敏感信息扫描后再作为评审证据保留。

推荐下一批 scenario：

- 两个 Turn 交错，affinity 不串线；
- 429/502/503/504/529 response-head 矩阵、首包断连与首个语义 SSE error 的 precommit fallback；
- commit 后禁止重放和总 Attempt 数上限；
- SSE 任意字节切片、长流、客户端取消与内存上界；
- multimodal、`count_tokens`、`compact` 和 usage/cost evidence；
- Codex CLI / Claude Code 的 real-local smoke profile。

当前 v2 native mock 对成功响应返回固定文本 SSE，并支持有限 response-head failure；`ScenarioStep` 只支持 HTTP Turn。不要用任意 shell 字段绕过这个边界；需要上述能力时，应把 stream failure 或 `AgentTurn` 建成新的有限枚举并升级 schema。

## CI 分层

当前公共 CI 在 Linux、macOS、Windows 上编译 harness、运行其 mock/unit tests，并通过 CLI
静态校验 checked-in scenario 与 profile；这条门禁不需要外部二进制、真实账号或凭据。

完整 10-step 进程链路当前仍需要单独提供 stock CPA 与 HiRoute SUT，按上面的 `run` 命令显式执行。
它不会被静态 `validate` 冒充已经在公共 CI 跑通。正式 SUT 可由代码仓库直接构建后，再分层扩展：

- `e2e-core-routing`：构建 stock CPA + SUT，执行当前 10-step hermetic suite；
- `e2e-product`：按 Proposal 增加 fallback、tool、cancel、memory 等套件；
- `e2e-real-local`：显式人工/nightly 触发，使用真实本地 Agent 和授权，不进入公共 CI，也不替代 hermetic suite。

## P0 Gateway exact Oracle

最终 `gateway-isolated` 合同由 `schema/p0-production-manifest.json` 封口。Manifest 锁定一个真实
Responses → Responses production smoke、profile、scenario 和 launcher/readiness/collector/result schema；
artifact SHA-256 对 canonical JSON 计算，任何未重新封口的语义漂移都会在启动前失败。旧
`p0-gateway-manifest.json`、corpus/golden 和 fixture adversary 仍可作为历史负向输入，但不再被最终 profile
加载，也不能证明发布完成。

Runner 只用正常的 `hirouted --role gateway --listen ... --publication ... --credentials ...` 路径；最终合同中
不存在 `--fixture`、`--expect-status`、`expected_red` 或 `skip`。每次运行生成私有 publication/credential
输入、随机 client/provider capability、不可预测 freshness challenge、动态 loopback listener 和 native
Provider mock。Profile 固定 Gateway 产品源码 revision；runner 在 binary 所属 checkout 内检查源码树和构建输入相对该
revision 无 tracked/untracked 漂移，再用当前 target/toolchain 执行带随机 build nonce 的 `cargo rustc --locked
--all-features` 可信重建。Launcher evidence 固定这次重建的 revision/source tree/build-input digest、target、toolchain、
build command、artifact path 和实际 executable SHA-256，因此不同 checkout/平台的合法构建无需共享本地 debug hash，
而 wrapper、非 Cargo artifact 和伪造构建输入不能被当作 SUT。
`/_hiroute/ready` 除精确回报 publication revision/digest 与 executable digest 外，runner 还会在 probe 前后
通过 OS socket-owner proof 要求动态 listener 都由同一存活 child PID 持有。调用方还可用
`HIROUTE_E2E_SUT_REVISION` 增加外部 revision 断言。

Provider 证据保存完整原生 JSON payload 和精确连接计数。Collector 分别读取 Lifecycle、ExecutionFact、
ConversationContent 和 content-free OTel JSONL，要求四套独立 producer/epoch/stream identity、从 1 开始的
连续 sequence、每条 record 的 exact channel schema digest 与拒绝未知字段的强类型 product envelope、唯一
event ID、同一 request correlation、零 gap/loss 和各自 terminal。Lifecycle 的 `request_finished` 必须
最后出现；ExecutionFact 的 `request_finished` 后只允许独立的 `usage_and_cache`；ConversationContent 必须
只含 request/response 两个方向并分别形成 `begin … finish`，绝不把 `request_finished` 解释成正文 barrier。结果中的十项
exact evidence 唯一地投影为四个有序 checkpoint；缺失、孤立、损坏、hard-coded、wrong-ready、wrong payload
或 incomplete terminal 均 fail closed。

结果 schema 只允许 scenario `green|red`，并把 `process_exit.test_process_code` 与 `scenario_state` 分开记录。
CLI 只在真实 production evidence 推出 `green` 时返回 0；证据不匹配会写出 `red` 结果并返回非零，启动、
provenance、readiness 或 collector 合同失败也返回非零且不能形成发布证明。完整结果含 synthetic challenge
和 canonical Content evidence，因此以 0600 写入指定路径；stdout 只输出无正文摘要。

验证、重新封口并运行真实 production smoke：

```sh
cargo build -p hiroute-gateway --bin hirouted --all-features
cargo run -p hiroute-e2e -- validate \
  --schema e2e/schema --scenario e2e/scenarios/p0-gateway.json

# 只在有意修改任一已锁定 artifact 后运行；连续运行两次，第二次必须零 diff。
cargo run -p hiroute-e2e -- seal-production --e2e-root e2e

HIROUTE_E2E_SUT_BIN="$PWD/target/debug/hirouted" \
cargo run -p hiroute-e2e -- run \
  --profile e2e/profiles/gateway-isolated.json \
  --scenario e2e/scenarios/p0-gateway.json \
  --result target/e2e/p0-gateway-result.json
```

本 PROCESS 只冻结这一个 production smoke 和稳定扩展点。九条协议路径、fault/resource/privacy 全矩阵继续由
PROCESS-22009 在其独立 ownership 中补齐；不得把这里的一例绿色外推成完整矩阵完成。

`matrix/p0-gateway-coverage.json` 是 PROCESS-22009 的独立 exact coverage index，但不是完成状态的
自报输入。冻结 typed registry 定义完整 required row 集；每个 row 只引用 registry receipt，而 receipt
固定真实 `hirouted` + native Provider 执行模式、测试 target、精确 symbol、完整 source SHA-256 和
assertion identity。`seal-coverage` 只把该 registry 投影为 JSON：

```sh
cargo run -p hiroute-e2e -- seal-coverage --e2e-root e2e
```

`p0_gateway_matrix_coverage` 同时校验 schema、完整 registry projection、source digest、symbol/test
绑定与 assertion identity；缺失 row 或伪造 execution 均失败。index 不包含 `skip`、`expected_red` 或人工
`passed` 状态。个人版 P0 listener 的 transport 行仅是 loopback H1；release H2 benchmark 与 Enterprise
TLS listener 不属于此 matrix。
