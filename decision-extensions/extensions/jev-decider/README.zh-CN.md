# 官方 Jev 决策扩展

[English](README.md) · [自定义扩展 API](../../api/README.zh-CN.md)

Jev 扩展是一个可自行部署的 Python HTTP 服务。它接收 HiRoute 的决策请求，将问题发送给兼容 System One API 的供应商，再按 [HiRoute 扩展 API v1](../../api/README.zh-CN.md) 返回结果。每次决策只调用一次上游。

如果只想直接使用支持的决策供应商，可以在 **模型 → 决策模型** 中添加内置连接，无需部署本服务。需要自己管理决策服务或修改供应商集成时，可以使用这份参考实现。

扩展按每次请求中的任务条件和评分标准进行判断：`categorical` 返回任务类别，`ordinal` 返回复杂度概率，`assessment` 可选地评价上一执行阶段。HiRoute 再应用阈值、选择模型组并记录执行结果。这些路由设置在 HiRoute 中配置，不写进扩展的部署参数。

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

选择下面一种供应商配置。环境变量沿用 `OPENROUTER_*` 名称；接入百炼或其他兼容供应商时，也使用这些变量填写相应的地址和 API Key 文件路径。

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
| `JEV_REQUEST_TIMEOUT_SECONDS` | 请求总超时默认 2.8 秒，包含准备、排队、连接和读取；范围为 `(0,3600]` |
| `JEV_MAX_REQUEST_BYTES` | 发往供应商的完整请求最多 262144 字节，包括问题与 `state`；单位是字节，不是 token |
| `JEV_MAX_CONCURRENCY` | 默认同时进行 32 个上游调用 |
| `HOST`、`PORT` | `127.0.0.1`、`8080` |
| `DECIDER_AUTH_HEADER_NAME`、`DECIDER_AUTH_HEADER_VALUE` | 可选，必须成对设置，用于 HiRoute 请求扩展的认证；自定义连接中配置相同头名与值 |

## 在 HiRoute 中接入扩展

服务启动后，`GET http://127.0.0.1:8080/health` 检查 HTTP 服务是否运行，不会调用供应商或验证 Key 权限。

在**模型 → 决策模型**添加**自定义扩展**，完整接入点为 **`http://127.0.0.1:8080/v1/decisions`**。HiRoute 连接超时应大于扩展的请求总超时；以上启动命令将扩展超时设为 10 秒。设置了入站认证时，两端配置相同头名与值。扩展运行在其他主机时，应使用 HiRoute 可访问的地址代替回环地址。保存并测试连接，然后在路由计划中选择它并发布。

这里有两个不同的请求地址：一个供 HiRoute 调用扩展，另一个供扩展调用供应商。两者使用的 JSON 格式也不同：

| 调用方 → 接收方 | 接入点 | 请求内容 |
| --- | --- | --- |
| HiRoute → 本扩展 | `/v1/decisions` | [自定义扩展请求](../../api/README.zh-CN.md)：定义、当前输入、可见历史和评分目标 |
| 本扩展 → 决策供应商 | 配置的 `/alpha/decisions` 或 `/systemone` 地址 | System One 的 `model`、`state` 及 Choice/Score 问题 |

不要把上游供应商地址当作本扩展的 HiRoute 接入点。直接调用供应商时，应添加内置决策模型连接。

## 行为和限制

扩展将类别选择、复杂度判断和历史评分作为独立问题，一次发送给供应商。返回结果后，只读取选中类别的复杂度；其他类别的答案有误，不影响已经选中的结果。

复杂度使用各档位的完整概率分布。历史评分使用 System One 返回的 `score`：上游以 0、1、2 对应三个评分标准，扩展将分数除以 2，转换为 HiRoute 的 `[0,1]` 范围；`confidence` 不能代替胜任度。选中类别的复杂度答案无效时，仍保留类别，由 HiRoute 选择该类别的主力组。无效评分会被省略，扩展也不会自行生成评分理由。

当前输入与传入的提示词保持完整。为满足供应商请求的字节上限，可以从最旧完整历史轮次开始裁剪；目标阶段内的证据被移除时标记部分评分，目标全部移除则省略评分。当前输入与问题本身超限时拒绝请求，不截断内容。供应商上下文限制仍适用，执行模型仍使用路由计划中的上下文设置。

每次只有一个上游调用，不重试、不跟随重定向；上游响应最大 **64 KiB**。等待空闲并发名额的时间也计入请求总超时。日志记录耗时、数量、状态和供应商实际返回的用量，不记录提示词或凭据。`subset` 工具精选尚未实现。

## 离线验证

安装完成后，从仓库根目录执行：

```sh
cd decision-extensions/extensions/jev-decider
"$HOME/hiroute-jev/.venv/bin/python" -m unittest discover -v
```

这些测试使用本地模拟供应商，检查请求校验、供应商结果转换、历史裁剪、认证、超时和 HTTP 接口，不会验证真实供应商的判断质量。部署后，再在 HiRoute 中保存并测试连接，最后用代表性任务观察实际效果。
