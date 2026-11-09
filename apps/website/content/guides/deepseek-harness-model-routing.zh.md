# DeepSeek Harness 模型路由：给 DSH 接入多模型与执行 Agent

继续用 DeepSeek Harness 的工作流，为简单问题选更快的模型，为复杂分析保留主力。HiRoute 可以给 DSH Web 或 Desktop 添加路由，也可以让其他主 Agent 把任务交给 DSH 执行。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/deepseek-harness-model-routing/
请按我使用的 DSH 入口复用模型、配置分流和委派。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查看 DSH 与可导入模型：hiroute quickstart · compute scan · compute connection prepare
- 发布计划并接入正确入口：routing preview / apply · agents connect preview / apply
- 检查执行器与实际模型：worker executors · worker result · sessions timeline
界面和 CLI 共用配置和记录；区分 Web 与 Desktop 目标，以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

## 1. 复用 DSH 已配置的模型或添加 API

对于已有 DSH Web/CLI 配置，在“模型 → 通用模型 → 添加模型 → 扫描本机”中找受支持的静态 provider。它需要有明确的模型声明；检查来源后保存要用的模型。

如果没有可导入的接入，可以在“新建 API 接入”添加 DeepSeek API，或复用已经保存在 HiRoute 的 [Qoder、WorkBuddy 或其他来源](/zh/docs/agent-integrations/)。先选一个用于快速分析的模型，再选一个用于长链推理的主力模型。

## 2. 为 Web 或 Desktop 添加难度路由

在“智能路由”新建“DSH 分析”，选择“按难度分流”。让常规组读代码、提取日志，让主力组判断根因。先用启发式规则试用；如果希望记录阶段评分，选择[决策模型](/zh/docs/model-routing/#difficulty-routing)。发布后打开“Agent → 模型路由”。

按你实际使用的版本选 **DeepSeek Harness Web** 或 **DeepSeek Harness Desktop**。两者分别保存。添加已发布的路由，重启所选客户端后从模型选择器选中。用一个小任务确认“会话”显示的是这条路由。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-dsh-zh.webp"><img src="/guide-assets/agent-guide-dsh-zh.webp" alt="DeepSeek Harness Web 的 HiRoute 模型路由对话框" width="1200" height="900" loading="lazy" /></a>
  <figcaption>给实际使用的 DSH 入口添加路由。Web 和 Desktop 各有自己的设置。</figcaption>
</figure>

## 3. 把修复交给 Codex，或让 DSH 执行分析

想从 DSH 发起委派时，用支持任务路由的 Web/CLI 入口。复用你的模型新建“Codex 局部修复”，在“更多设置 → 任务委派”中绑定 **Codex CLI**，填写用途、检查依赖并发布。然后在 DSH [开启任务路由](/zh/docs/task-routing/#delegation-skill)，要求它把一个边界明确的修复交给该计划。

也可以从 Codex、Hermes 等主 Agent 委派给 DSH：在独立计划里选 **DeepSeek Harness** 为执行 Agent，并准备相应 CLI 依赖。任务执行方与结果在“Agent → 任务记录”查看，实际模型在会话的路由时间线查看。

下一步：[从 Codex 发起 DSH 任务](/zh/docs/codex-model-routing/) · [给 Hermes 添加 Codex 执行计划](/zh/docs/hermes-model-routing/)
