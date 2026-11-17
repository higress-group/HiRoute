# 组合本地模型与前沿模型，让决策也在本地完成

用本地 Qwen3.8-27B 承担常规工作，云端前沿模型处理复杂轮次，再由本地部署的 D3 决策模型判断难度和上一阶段的表现。HiRoute 把这三个角色连接到同一条路由，供 OpenCode 或其他支持的 Agent 选用。

本文基于 HiRoute 现有接口和上游文档说明一种可行的接入方案。HiRoute 尚未对完整的 Qwen + D3 组合做端到端实测；决策质量、延迟和硬件容量仍需在所选部署环境中评估。

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/local-model-routing/
按我的目标研究或配置本地 Qwen3.8-27B 执行模型、本地 vLLM Semantic Router D3 决策接口和云端主力模型。区分协议可行性与真实推理证据。复用已有服务；只有我要求部署且目标机器容量足够时，才下载模型权重或启动推理服务。
CLI 参考：https://hiroute.ai/zh/docs/cli/
请使用内置管理技能：https://hiroute.ai/zh/docs/#management-skill
- 查看执行模型、添加 API：compute list · compute connection prepare / test / preview / apply
- 查看和配置 System One 决策接入：decision connections options / list / apply / test
- 配置按难度分流并接入 Agent：routing preview / apply · agents connect preview / apply
- 复盘模型切换和费用：sessions timeline · prices effective · prices override preview / apply
界面与 CLI 共用配置和记录。先读取现状，参数以本机 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。从 HiRoute 服务所在机器解析接口地址，采用服务实际提供的模型 ID、认证方式和上下文上限。不要把连接测试通过表述为决策质量已验证。
```

</details>
</aside>

## 三个角色分别做什么

| 角色 | 示例 | HiRoute 中的接入方式 |
| --- | --- | --- |
| 常规模型执行 | 本地 `Qwen/Qwen3.8-27B`，Ollama 提供的 ID 为 `qwen3.8:27b` | 通用模型，OpenAI Chat Completions |
| 主力模型执行 | 已接入 API 或订阅中的前沿模型 | 通用模型，兼容 Agent 的请求需求 |
| 决策 | 本地 `vllm-sr/d3` | 决策模型，System One |

Qwen 的官方模型 ID 是 [Qwen/Qwen3.8-27B](https://huggingface.co/Qwen/Qwen3.8-27B)。[D3](https://huggingface.co/vllm-sr/d3) 来自 vLLM Semantic Router 社区，是基于 Qwen3.8-27B 的独立决策权重，以概率回答指定类型的问题。它不执行 Agent 的工具，也不生成任务回答。

两个本地模型可以部署在同一台容量足够的机器上，也可以分别运行在自己管理的机器上。内存需要同时容纳模型、执行上下文缓存、决策输入和运行时；下载大小不等于运行内存需求。完整 D3 也是 27B 级模型；资源有限时可评估较小的 [d3-mini](https://huggingface.co/vllm-sr/d3-mini)，同时比较其准确性与资源开销。本例不建议在小内存 Omarchy 虚拟机里同时运行两个大模型。

## 1. 把本地 Qwen 接为执行模型

在推理机器上按运行时的安装和模型指引提供 [Ollama `qwen3.8:27b`](https://ollama.com/library/qwen3.8:27b) 服务。[Ollama 的 OpenAI 兼容文档](https://docs.ollama.com/api/openai-compatibility) 说明了本地聊天接口。

在 HiRoute 打开 **模型 → 通用模型 → 添加模型 → 新建 API 接入**，再选择 **自定义 API**。如果 Ollama 与 HiRoute 服务在同一台机器上，可填写：

| 字段 | 值 |
| --- | --- |
| API 基础地址 | `http://127.0.0.1:11434/v1` |
| 协议 | OpenAI Chat Completions |
| 模型 ID | `qwen3.8:27b` |
| API Key | 忽略认证的本地服务可填 `ollama`；若代理要求认证，填写实际配置的 Key |

读取模型列表，选中模型并保存。按服务实际能力核对上下文和工具能力；一次文本回答成功不代表 Agent 的工具调用已验证。通过同一页面或 **连接订阅** 添加云端主力模型。完整操作见 [接入模型](/zh/docs/models/)。

`127.0.0.1` 指 **HiRoute 服务所在机器**。如果 HiRoute 在 Omarchy 中运行，而推理服务在宿主机或其他局域网机器上，应填写那个服务可达的地址。局域网接口应使用私有、带认证的连接。用浏览器打开远程 Web UI 不会改变 HiRoute 发请求的位置。

## 2. 把 D3 接为本地决策模型

选择支持 D3 且提供 **System One** 的运行时构建。[vLLM Semantic Router Engine 快速开始](https://github.com/vllm-project/semantic-router/blob/main/website/docs/model-runtime/get-started/quickstart.md) 说明了安装、发布通道要求和模型服务方式。其 macOS 容器使用 CPU；本文的大模型示例应选择支持的 GPU 部署环境。在容量足够的推理机器上安装适用构建后，可用 Engine 模式提供该模型：

```sh
vllm-sr serve vllm-sr/d3 --engine
```

首次启动会下载所需镜像和模型。以服务实际打印的地址为准；下面采用文档中的 Engine 监听端口 `8899`。通过 `/v1/systemone/models` 核对已发布的模型 ID。Engine 模式提供决策 API，执行路由及其策略仍由 HiRoute 管理。

也可以用支持 D3 的 vLLM 构建直接提供 `/v1/systemone`，遵循其 [D3 支持与上下文限制说明](https://docs.vllm.ai/en/latest/models/structured_decisions_models/#decision-30)。选择前先核对已安装版本的支持情况；普通聊天接口不足以承担这个角色。

在 **模型 → 决策模型** 添加决策模型，选择 **独立配置 → 其他兼容接入**，填写：

| 字段 | 值 |
| --- | --- |
| 名称 | `Local D3` |
| 模型 ID | `vllm-sr/d3`，或服务实际发布的完整 ID |
| 完整地址 | `http://127.0.0.1:8899/v1/systemone` |
| 协议 | System One |
| API Key | 接口的实际 Key；若本地接口允许无认证访问，可用 `local` 等非机密占位值满足 HiRoute 的必填字段 |

填写包含 `/v1/systemone` 的完整决策地址，不要填执行模型的 `/v1` 基础地址。HiRoute 的决策上下文设置应匹配服务实际支持的输入预算，并为问题留出空间。更长的上下文可能增加内存和决策耗时。不要让服务静默截断输入：丢失上一阶段内容会破坏评分依据。

保存后测试这个已保存的接入。本方案预计无需自定义扩展，依据是双方协议相符：HiRoute 发送 `state` 和具名 `questions`，消费 System One 定义的类型化 `answers`、选项概率和评分。连接测试检查这段交换，不代表 D3 对实际任务的判断质量已经得到验证。

## 3. 发布路由，在 OpenCode 中选用

1. 打开 **智能路由 → 新建智能路由**，命名为“本地 Qwen + 前沿模型”，选择 **按难度分流**。
2. 在 **常规模型** 添加本地 Qwen，在 **主力模型** 添加云端模型。
3. 在 **决策方式** 选择已保存的 **Local D3** 接入。评估自己的任务时先保留初始信任度，并在判断设置中核对难度与胜任度标准。
4. 启用路由。打开 **Agent → OpenCode → 模型路由**，选中该路由，选择兼容协议并保存。
5. 重启 OpenCode，在 `/models` 中选用该路由。这些控件的图示见 [OpenCode 接入教程](/zh/docs/opencode-model-routing/#configure-routing)；将图示模型和启发式决策方式替换为上述配置。

每个新用户轮次中，HiRoute 可以把常规工作交给常规模型，把难点交给主力模型。对上一实际执行阶段的新鲜、完整低评分，在此前问题与本轮任务相关时，也可以触发主力接手。低分不一定导致切换。同轮工具续接复用仍有效的决策，HiRoute 不会在已经开始流式输出的回答中途切换模型。详见 [按难度分流](/zh/docs/model-routing/#difficulty-routing)。

## 4. 在所选部署环境中评估

具备推理资源后，可用一个包含工具调用和后续追问的小型真实任务评估。在 **会话 → 路由时间线** 查看实际执行模型、难度结果、上一阶段评分和每次分组选择的原因，核对切换是否符合任务，以及本地模型是否完成所需工作。不要把示意截图或连接测试通过当作端到端演示。

同时检查失败行为。整次难度决策失败或超时时，HiRoute 回退到启发式规则，不保证一定选择云端。可用决策中若难度判断不可用，则选择主力。执行候选必须满足请求能力要求；常规模型候选不可用时可以接力到主力。依赖这层兜底时，需要保持云端凭据和额度可用。

本地决策让 **决策推理** 留在自己管理的基础设施上。路由到云端的轮次仍会把执行所需上下文发送给提供方。若要求所有执行都留在本地，应只配置本地执行候选；本文的混合路由不保证这一策略。

在 **编辑价格** 中给云端候选填写实际 API 价格。本地服务没有 API token 费用，不代表硬件和电力成本为零。增加本地模型承担的工作量前，应一起比较任务完成情况、决策额外耗时和云端用量。[路由复盘指南](/zh/docs/optimize-model-routing/) 说明了怎样使用这些记录。
