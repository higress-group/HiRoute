# HiRoute Jev 决策器

## 决策日志

CLI 默认以 INFO 级别向 stderr 输出 JSON 事件，可重定向到文件或由容器收集。
`service_started` 记录实际模式、模型、阈值及资源限制。每次请求生成 `decision_id`，
通过响应头 `X-Jev-Decision-Id` 返回，关联开始、完成、取消或异常事件。
这个 ID 属于决策服务，HiRoute 当前尚未持久化该响应头。

结果包含上游原始选择、rules 概率、归一化能力分及其有效性、阈值判断、最终分支、
规则原因、历史裁剪与评估边界、排队及总耗时、HTTP 状态和失败阶段。
规则原因为 `auto_choice`、`complexity_threshold`、`competence_guard` 或 `economy_eligible`。
缺失能力分仍沿用当前“不阻止省钱分支”的规则，日志会明确记录缺失。
这里记录的是规则依据，不是模型生成的推理过程；不记录正文、凭证或上游原始响应。
以库方式运行时，需要将 `jev_decider` logger 启用为 INFO。

[English](README.md) · [决策机制](../../README.zh-CN.md) · [API 与 OpenAPI](../../api/README.zh-CN.md)

这是可以自行部署的官方参考扩展：接收 HiRoute 的五字段决策请求，通过一次 OpenRouter `typesafe/jev-1.13` Decisions 调用，返回分支选择与可选的上一执行阶段胜任度评分。它是独立受信服务，HiRoute 负责执行、路由执行轮次边界、评分归属和持久化，服务负责策略、提示词与模型上下文裁剪。部署后，可见对话投影会发往 OpenRouter/TypeSafe。

## 选择一种模式

- `auto`：默认模式。一次请求包含 branch Choice 与可选 competence Score；Jev 综合新任务复杂度、历史表现及反馈选分支。支持请求允许的多个分支。
- `rules`：一次请求包含简单/复杂概率与可选 competence Score，再执行下面的公式。仅支持当前智能省钱的两个分支，不与 auto 混用或互相兜底。

**胜任度只负责阻止冒险降本，复杂度负责提供降本机会。**

```text
选择省钱分支 = P(simple) >= JEV_SIMPLE_THRESHOLD
              且（本次无合法评分 或 score >= JEV_COMPETENCE_FLOOR）
否则选择主力分支
```

默认阈值分别为 `0.80` 和 `0.50`，只是起点，不代表已校准。提高任一阈值会更保守：上一阶段低分阻止降本，高分不能抵消当前问题的复杂度；上一阶段高分且新问题简单才提供降本机会。缺评分不补为零，也不触发胜任度否决。部分评分仍参与公式，并明确 `partial`，不能当作完整验证。仅使用本次评分，不读取缓存旧分补位。

## 本地运行

需要 Python 3.12+。从仓库根目录开始：

```sh
cd decision-extensions/extensions/jev-decider
python -m venv .venv
. .venv/bin/activate
pip install .
printf '%s' 'replace-with-key' > /absolute/path/openrouter-key
chmod 600 /absolute/path/openrouter-key
OPENROUTER_API_KEY_FILE=/absolute/path/openrouter-key hiroute-jev-decider
```

也可从仓库根目录构建容器：

```sh
docker build -t hiroute-jev-decider decision-extensions/extensions/jev-decider
docker run --rm -p 127.0.0.1:8080:8080 \
  -v /absolute/path/openrouter-key:/run/secrets/openrouter-key:ro \
  -e OPENROUTER_API_KEY_FILE=/run/secrets/openrouter-key \
  -e JEV_MODE=auto hiroute-jev-decider
```

HiRoute 的智能省钱计划配置：

```json
{
  "kind": "rest",
  "endpoint": "http://127.0.0.1:8080/v1/decisions",
  "timeout_ms": 3000
}
```

`GET /health` 只检查服务，不调用模型；保存和发布计划也不会调用服务。已有部署升级时需修改计划 endpoint，当前不保留旧路径别名。容器或远程部署中的地址必须能从 HiRoute 所在环境访问。

## 配置

| 环境变量 | 默认值 | 作用 |
| --- | --- | --- |
| `OPENROUTER_API_KEY_FILE` | 必填 | UTF-8 key 文件的绝对路径，仅用于出站 Bearer 认证 |
| `JEV_MODE` | `auto` | `auto` 或 `rules` 二选一 |
| `JEV_MODEL` | `typesafe/jev-1.13` | OpenRouter Decisions 模型 |
| `JEV_POLICY_FILE` | 内置判断标准 | 可选自定义定义 JSON 文件的绝对路径；启动读取，修改后重启 |
| `OPENROUTER_DECISIONS_URL` | `https://openrouter.ai/api/alpha/decisions` | 仅在受控网关或测试时覆盖 |
| `JEV_REQUEST_TIMEOUT_SECONDS` | `2.8` | 包含校验、上下文、排队及上游调用的总预算，范围 `(0,3600]`；应略小于 HiRoute `timeout_ms/1000` |
| `JEV_MAX_STATE_TOKENS` | `24000` | 保守 state 预算，见下节 |
| `JEV_MAX_CONCURRENCY` | `32` | 最大同时上游请求数，排队也消耗总预算 |
| `JEV_SIMPLE_THRESHOLD` | `0.80` | rules 的简单概率门槛；auto 显式设置时拒绝启动 |
| `JEV_COMPETENCE_FLOOR` | `0.50` | rules 的胜任度下限；auto 显式设置时拒绝启动 |
| `DECIDER_AUTH_HEADER_NAME` / `DECIDER_AUTH_HEADER_VALUE` | 不设置 | 可选入站认证，两者同时设置或同时省略 |
| `HOST` / `PORT` | `127.0.0.1` / `8080` | 监听地址和端口 |

出站使用一个长生命周期连接池，遵循标准 `HTTP_PROXY/HTTPS_PROXY/NO_PROXY` 及小写变量。没有代理配置时直连；容器须显式接收需要的代理环境变量。入站认证与 OpenRouter key 分开；在 HiRoute Secret 中保存完整 header 值，例如 `Bearer ...`，HiRoute 不补认证前缀。

### 自定义简单与复杂的含义

智能省钱默认使用随 Python 包发布的 [policy.default.json](jev_decider/policy.default.json)，无需设置环境变量或复制文件。
如需自定义，复制并编辑该文件，设置 `JEV_POLICY_FILE=/absolute/path/policy.json` 后启动服务。
文件必须是 UTF-8 JSON，最多 4096 字节，恰好包含两个非空字符串：

```json
{
  "simple": "需求明确、可沿用已有实现模式的常规功能、修复、测试和文档工作。不能只因涉及多个文件就判复杂。",
  "complex": "需要定位未知根因、权衡不同设计方案、改变核心架构或处理复杂并发、状态一致性和算法交互的工作。"
}
```

定义应描述适合当前省钱/主力模型组合的工作，可附少量通用例子；不是指定某个实验题必须走哪个模型。
这只是判断口径，不代表经济模型真实成功率。先固定定义，再用独立任务校准阈值。

配置属于服务实例，所有接入该实例的计划共用；目前不是 Desktop 计划编辑字段。
对于智能省钱两个分支，`auto` 和 `rules` 都用内置或自定义定义同时替换上游 state 与 Choice criteria 的说明，
避免判断口径冲突。显式自定义文件只适用于这两个分支，不扩大允许集合；未配置文件时，auto 对其他分支集合仍使用请求提供的说明。
非法文件拒绝启动，不静默忽略或退回默认值。
修改后需重启，无热加载、策略 DSL 或额外会话存储。有效定义计入上下文预算。
`rules` 仍按原概率/胜任度公式执行；把更多工作定义为简单，不会取消低胜任度阻止降本的机制。

启动日志记录智能省钱的有效定义哈希；每次决策日志记录实际 `criteria_source`（default/policy/request）和
`criteria_sha256`，不记录定义正文。比对不同实验时应同时固定需求、可见历史、定义哈希、模型和阈值。

`decision_completed.upstream_usage` 保留上游实际报告的 `input_tokens`、`output_tokens` 和
`cost`（OpenRouter 为美元），即使随后判定答案无效也保留，便于把失败调用纳入费用。
仅记录这三个合法数值字段，不记录 usage 中的其他字段；缺失/非法用量为 `null`，缺字段仍是未知，
不会补成零。上游明确报告的零会保留。用量不改变响应合同、分支选择或重试；超时及没有用量的
拒绝/非法响应不能当作零费用。通过 `X-Jev-Decision-Id` 对应现有决策日志即可，无需新存储。

## 上下文边界

服务接收完整、非空的 `latest_user` 和保留的 `visible_conversation`。一次决策可能因为没有可继承分支、追加了用户消息，或 ContextHold 不再有候选保持而开始；服务不识别压缩。因此 `latest_user` 可以与上一执行轮次相同，也可以是客户端生成的摘要/继续消息，这种形状本身不是用户反馈。每个 visible 项是已封存的路由执行轮次；仅因进入下一决策边界而封存的项可以是 `unknown`，但仍可能包含可评分的实际推进。

服务不要求 HiRoute 截断或重试。超预算时移除最老的完整执行轮次，保留最近的完整历史；不截断当前输入、分支定义或固定问题。固定部分仍放不下则返回 413。删减了评分目标就标记 `partial: true`；没有目标内容剩余时省略 Score 和 assessment。

Jev 窗口以 token 计，不是 KB。本扩展为避免额外 tokenizer 依赖，将每个 UTF-8 字节保守计为一个 token，并为问题和输出预留空间。因此默认 state 最多 24,000 UTF-8 字节，不代表字节数等于实际模型 token 数；调整预算前应测量自己的模型与输入。

HiRoute 投影不含系统/开发者指令、reasoning、模型/计划身份或工具输入输出，工具仅有名称、顺序及粗粒度状态。服务不记录正文或凭据。

## 协议与扩展位置

`POST /v1/decisions` 接收 `branches/latest_user/visible_conversation/history_partial/assessment_from`，返回 `branch_id` 和可选 `assessment {score, partial, reason?}`。完整示例及 OAS 见 [API](../../api/README.zh-CN.md)。

评分是胜任度，不是置信度；Jev 的 `0..2` Score 除以二映射到 `[0,1]`。Jev 不提供文字 reason，本实现直接省略，不追加生成调用。无合法 Score 时仍可保留合法分支；模式必需输出非法则失败，不补调。

生产入口仍为 `jev_decider.server:main` → `create_app()` → `POST /v1/decisions`。职责按以下入口分开：

- [settings.py](jev_decider/settings.py)：环境配置、受限凭据/策略文件读取及校验。
- [protocol.py](jev_decider/protocol.py)：严格 JSON 和现有五字段请求校验。
- [decision.py](jev_decider/decision.py)：无 I/O 的策略优先级、整轮裁剪、Jev 问题构造、Choice/Score 解释及实际用量提取。
- [server.py](jev_decider/server.py)：HTTP、认证、总超时、并发、响应上限、错误映射、日志及生命周期。原公开导入保留为同一实现的转导出。

修改现有策略从 `decision.py` 的 `questions()` 和 `decision_response()` 开始；提示、阈值、覆盖优先级和裁剪变化都影响行为。保持 HiRoute 请求/响应合同，不增加第二个评分调用。本次职责拆分没有实现内置服务、多 provider 或工具选择。

后续应区分通用推断、用途策略、provider 传输和调用方执行授权。自然语言条件由模型评估；稳定分支 ID、允许集合、发布版本及默认回退由调用方政策拥有，合法分支不直接授予执行能力。现有二分 policy 覆盖及 rules 阈值保持不变；上一段 assessment 与本次选择分开。完整阅读入口见 [English architecture map](README.md#code-and-responsibility-map)。

## 测试

在扩展目录运行：

```sh
python -m unittest -v
```

测试使用真实 HTTP handler 和受控上游，不读真实 key、不访问付费供应商。覆盖 auto/rules 单次调用、首轮无评分、公式边界、非法输出、裁剪、认证、代理、上游失败、超时及并发。

按用户能力定位断言见 [English test map](tests/README.md)。HTTP 和纯模块共用独立预期的输入输出案例；纯模块合同也可仅使用 Python 标准库运行：`python -m unittest tests.test_decision -v`。

需要真实冒烟时，先启动持有真实 key 的本地服务，再从仓库根目录按仓库验证路由要求显式运行：

```sh
HIROUTE_LIVE_CLASSIFIER_ENDPOINT=http://127.0.0.1:8080/v1/decisions \
  cargo test -p hiroute-e2e --test p0_gateway_runtime \
  live_hiroute_to_jev_smoke_selects_and_records_an_assessment \
  -- --ignored --exact --nocapture
```

此测试会产生两次付费 Jev 请求，不自动重试。它通过真实 HiRoute listener 和 REST transport、受控业务 provider 验证选择与下一轮评分事实；本地规则回退或缺评分都不能算通过。它不证明真实业务模型质量，不放入默认 CI。不要将 OpenRouter key 放入 HiRoute 计划或命令行。
