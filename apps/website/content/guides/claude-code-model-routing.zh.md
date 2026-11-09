# Claude Code 模型路由：用 Opus、Sonnet 和 Haiku 预设选择路由

继续在 Claude Code 写代码，用熟悉的模型预设进入 HiRoute 路由。一条路由里可以让更经济的模型处理简单轮次，关键决策用主力模型；要并行做一项独立检查时，可以让 Pi 负责执行。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/claude-code-model-routing/
请帮我复用 Claude 模型并给选定的预设映射一条难度路由。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查找订阅或 API：hiroute compute scan · compute connection options / prepare
- 创建计划和预设映射：routing preview / apply · agents connect preview / apply
- 配置执行方并核对：worker executors · worker plans · worker result · sessions timeline
界面和 CLI 共用配置和记录；由我完成授权，参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

## 1. 接入 Claude 订阅或 Anthropic API

从“模型 → 通用模型 → 添加模型”进入。“扫描本机”可以识别受支持的 Claude Code 订阅或静态 API 配置；如果检测到订阅，按提示完成检查。也可在“连接订阅”独立登录，或在“新建 API 接入”直接连接 Anthropic API。

选两个适合你任务的模型。例如让 Haiku 做常规的代码阅读，Sonnet 或 Opus 做需要多步推理的修复。具体以账号目录为准。关于 Claude Code 的接口范围，参见 [Anthropic 的网关说明](https://code.claude.com/docs/en/llm-gateway)。

## 2. 让 Sonnet 预设进入一条难度路由

到“智能路由”新建“Claude 日常开发”，选“按难度分流”，将两个模型分别放进常规组和主力组。选择“启发式规则”，或用[决策模型](/zh/docs/model-routing/#difficulty-routing)加入阶段评分，发布计划。

在“Agent → Claude Code → 模型路由”展开“高级设置”，将 Sonnet 预设映射到这条路由。Opus、Sonnet、Haiku 可以各自保留原生映射，也可以映射到你选的路由。重新启动 `claude`，在 `/model` 中选已映射的预设，发一个小任务。在 HiRoute 的“会话”查看真正执行的模型。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-zh.webp"><img src="/guide-assets/agent-guide-claude-zh.webp" alt="HiRoute 中 Claude Code 的 Opus、Sonnet 与 Haiku 预设路由映射" width="1200" height="900" loading="lazy" /></a>
  <figcaption>从一个预设开始，其余预设可以继续使用原来的模型。</figcaption>
</figure>

## 3. 把独立调查交给 Pi

复用已接入的模型，创建“Pi 代码调查”。常规组负责找文件和证据，主力组负责归纳跨模块原因。在“更多设置 → 任务委派”选择 **Pi**，写明用途、检查 Pi 的 CLI 与 Node 环境，然后发布。使用该任务所需的兼容协议和模型能力。

在 Claude Code [开启任务路由](/zh/docs/task-routing/#delegation-skill)，试一句：“使用 HiRoute，把配置加载路径的调查委派给 Pi 代码调查，返回入口文件和证据链接。”完成后在“Agent → 任务记录”核对 Pi 的结果。即使 Claude Code 自身保留原生模型，你也可以使用这条委派计划。

下一步：[给 Pi 增加模型路由](/zh/docs/pi-model-routing/) · [查看委派任务的完整步骤](/zh/docs/task-routing/)
