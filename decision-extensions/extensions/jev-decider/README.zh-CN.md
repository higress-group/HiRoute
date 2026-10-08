# 官方 Jev 决策扩展

[English](README.md) · [自定义扩展 API](../../api/README.zh-CN.md)

这个可选的 Python HTTP 服务用一次 System One 供应商调用实现 HiRoute 当前 **v1**。HiRoute 也可在**模型 → 决策模型**直接接入供应商；选择自行部署的自定义扩展时才需要该服务。

扩展遵守请求传入的任务类别、程度条件和冻结评分标准，返回当前类别/程度，以及可选的上一实际阶段胜任度。阈值、模型组、可用性接力与观测由 HiRoute 负责，部署端不配置路由规则、阈值或固定分支策略。

## 安装

需要 **Python 3.12 或以上**。在 HiRoute 仓库根目录执行：

```sh
python3 -m venv "$HOME/hiroute-jev/.venv"
"$HOME/hiroute-jev/.venv/bin/pip" install ./decision-extensions/extensions/jev-decider
install -d -m 700 "$HOME/.config/hiroute"
touch "$HOME/.config/hiroute/jev-api-key"
chmod 600 "$HOME/.config/hiroute/jev-api-key"
```

使用编辑器将所选供应商的 API Key 保存到 `$HOME/.config/hiroute/jev-api-key`，内容为 UTF-8 文本。下面的配置读取这个私有文件；供应商 Key 留在扩展主机上。使用其他文件时应填写绝对路径。

## 配置并启动

选择一种供应商配置。包括百炼在内的兼容接入都使用 `OPENROUTER_*` 环境变量，没有另一套百炼 Key 或账户版本变量。

### OpenRouter Jev

```sh
export OPENROUTER_API_KEY_FILE="$HOME/.config/hiroute/jev-api-key"
export OPENROUTER_DECISIONS_URL="https://openrouter.ai/api/alpha/decisions"
export JEV_MODEL="typesafe/jev-1.13"
export JEV_REQUEST_TIMEOUT_SECONDS=10
"$HOME/hiroute-jev/.venv/bin/hiroute-jev-decider"
```

### 百炼 Token Plan

在私有文件中保存百炼 Token Plan Key，然后执行：

```sh
export OPENROUTER_API_KEY_FILE="$HOME/.config/hiroute/jev-api-key"
export OPENROUTER_DECISIONS_URL="https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone"
export JEV_MODEL="decision-model-preview"
export JEV_REQUEST_TIMEOUT_SECONDS=10
"$HOME/hiroute-jev/.venv/bin/hiroute-jev-decider"
```

Token Plan 使用统一端点，实际可用性由凭证权限和供应商调用结果决定。接入其他兼容 System One 供应商时，显式设置完整上游地址与模型。

### 环境变量

| 变量 | 默认值 / 用途 |
| --- | --- |
| `OPENROUTER_API_KEY_FILE` | 必填，私有 UTF-8 供应商 Key 文件的绝对路径 |
| `OPENROUTER_DECISIONS_URL` | `https://openrouter.ai/api/alpha/decisions` |
| `JEV_MODEL` | `typesafe/jev-1.13` |
| `JEV_REQUEST_TIMEOUT_SECONDS` | 总预算默认 2.8 秒，包含准备、排队、连接和读取；范围为 `(0,3600]` |
| `JEV_MAX_REQUEST_BYTES` | 默认 262144 字节，包括问题和 state 的完整供应商请求；不是 token 估算 |
| `JEV_MAX_CONCURRENCY` | 默认同时进行 32 个上游调用 |
| `HOST`、`PORT` | `127.0.0.1`、`8080` |
| `DECIDER_AUTH_HEADER_NAME`、`DECIDER_AUTH_HEADER_VALUE` | 可选，必须成对设置，用于 HiRoute 请求扩展的认证；自定义连接中配置相同头名与值 |

## 在 HiRoute 中接入扩展

服务启动后，`GET http://127.0.0.1:8080/health` 检查 HTTP 服务是否运行，不会调用供应商或验证 Key 权限。

在**模型 → 决策模型**添加**自定义扩展**，完整接入点为 **`http://127.0.0.1:8080/v1/decisions`**。HiRoute 连接超时应大于扩展总预算；以上示例的扩展预算为 10 秒。设置了入站认证时，两端配置相同头名与值。扩展运行在其他主机时，应使用 HiRoute 可访问的地址代替回环地址。保存并测试连接，然后在路由计划中选择它并发布。

两层 HTTP 边界使用不同请求：

| 调用方 → 接收方 | 接入点 | 请求内容 |
| --- | --- | --- |
| HiRoute → 本扩展 | `/v1/decisions` | [自定义扩展请求](../../api/README.zh-CN.md)：定义、当前输入、可见历史和评分目标 |
| 本扩展 → 决策供应商 | 配置的 `/alpha/decisions` 或 `/systemone` 地址 | System One 的 `model`、`state` 及 Choice/Score 问题 |

不要把上游供应商地址当作本扩展的 HiRoute 接入点。直接调用供应商时，应添加内置决策模型连接。

## 行为和限制

扩展在一次供应商请求中提交独立的类别、程度和可选评分问题，只读取选中类别的程度；未选中答案错误不影响选中路径。程度使用完整概率分布，胜任度使用供应商原始 score 除以二，不使用 confidence。选中程度非法时保留类别，由 HiRoute 使用该类别主力组；非法评分省略，不制造评分理由。

当前输入与传入的提示词保持完整。为满足完整请求字节预算，可以从最旧完整历史轮次开始裁剪；目标阶段内的证据被移除时标记部分评分，目标全部移除则省略评分。当前输入与问题本身超限时拒绝请求，不截断内容。供应商上下文限制仍适用，业务模型继续使用路由计划中的上下文设置。

每次只有一个上游调用，无重试，禁止跳转；上游响应最大 **64 KiB**。服务总预算包含等待并发名额。日志记录耗时、数量、状态和供应商实际返回的用量，不记录提示词或凭据。工具精选仅保留未来文档，本服务尚未实现。

## 离线验证

安装完成后，从仓库根目录执行：

```sh
cd decision-extensions/extensions/jev-decider
"$HOME/hiroute-jev/.venv/bin/python" -m unittest discover -v
```

测试通过本地 fixture 验证严格请求、答案归约、裁剪、认证、deadline 和 HTTP handler，不代表供应商判断质量或原生 Desktop 验收。HiRoute 的显式连接测试验证保存的连接及必要结果字段；实际任务质量仍需代表性的真实使用确认。
