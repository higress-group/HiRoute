# Claude Code 模型路由：用 Opus、Sonnet 和 Haiku 预设选择路由

继续在 Claude Code 写代码，用熟悉的模型预设进入 HiRoute 路由。一条路由里可以让更经济的模型处理简单轮次，关键决策用主力模型；要并行做一项独立检查时，可以让 Pi 负责执行。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/claude-code-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再介绍任务委派的适用场景，按我的需要选择启用。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查找订阅或 API：hiroute compute scan · compute connection options / login / prepare / preview / apply
- 创建计划和预设映射：routing preview / apply · agents connect preview / apply
- 需要任务委派时，配置执行方并核对：worker executors · worker plans · worker result · sessions timeline
界面和 CLI 共用配置和记录；由我完成授权，参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

已有 Claude 订阅时，先把账号里的模型导入 HiRoute。

1. 打开“模型 → 通用模型 → 添加模型 → **连接订阅**”。
2. 在 **Claude Code** 卡片点击“登录”，用自己的 Claude 账号在浏览器里完成授权，然后回到 HiRoute。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-subscription-login-zh.webp"><img src="/guide-assets/agent-guide-subscription-login-zh.webp" alt="HiRoute 订阅接入中的 Claude Code 登录入口" width="1200" height="900" loading="lazy" /></a>
  <figcaption>在 Claude Code 卡片点击“登录”，完成账号授权后回到 HiRoute。</figcaption>
</figure>

3. 点击 **“检查并选择模型”**，再点击 **“检查订阅”**，读取这个账号可用的模型。
4. 从账号目录勾选要使用的模型，点击 **“保存并接入”**。例如可以选 Sonnet 或 Opus，再搭配已有的经济模型组成路由。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-subscription-models-zh.webp"><img src="/guide-assets/agent-guide-claude-subscription-models-zh.webp" alt="Claude Code 订阅检查后选择 Sonnet 与 Opus 并保存接入" width="1200" height="900" loading="lazy" /></a>
  <figcaption>模型以账号返回的目录为准，勾选后保存到 HiRoute。</figcaption>
</figure>

已经在本机登录 Claude Code 时，也可以从“添加模型 → **扫描本机**”复用登录，检查订阅并保存模型。HiRoute 支持读取现有文件和 macOS 钥匙串登录。有 Anthropic API Key 时，从“新建 API 接入”选择对应服务、检查并保存即可。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“Claude 日常开发”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

打开“Agent → Claude Code → 模型路由”，展开“高级设置”，将 **Sonnet** 预设映射到刚发布的“Claude 日常开发”。也可以为 Opus 或 Haiku 预设分别选择路由，保存设置。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-claude-zh.webp"><img src="/guide-assets/agent-guide-claude-zh.webp" alt="Claude Code 的 Opus、Sonnet、Haiku 预设与 HiRoute 路由映射" width="1200" height="900" loading="lazy" /></a>
  <figcaption>将 Sonnet 预设映射到已发布的难度路由。</figcaption>
</figure>

重启 `claude`，输入 `/model` 选择映射过的预设，再发一个小任务。HiRoute 会根据这条路由在常规组和主力组之间选择模型，实际选模可在“会话 → 路由时间线”查看。

<h2 id="task-delegation">4. 了解任务委派</h2>

在 Claude Code 修复问题时，可以把“查清配置从哪里加载”或“独立审查这份 diff”交给 Pi，让当前对话继续推进主线，完成后再汇总调查结果。

想使用时，在路由的“更多设置 → 任务委派”中绑定执行 Agent，并在发起任务的主 Agent 上开启“任务路由”。执行方会按照这份计划选择模型，结果可在“Agent → 任务记录”查看。[继续阅读 Agent 智能路由教程](/zh/docs/task-routing/)
