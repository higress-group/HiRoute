# DeepSeek Harness 模型路由：给 DSH 接入多模型与执行 Agent

继续用 DeepSeek Harness 的工作流，为简单问题选更快的模型，为复杂分析保留主力。HiRoute 可以给 DSH Web 或 Desktop 添加路由，也可以让其他主 Agent 把任务交给 DSH 执行。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/deepseek-harness-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再介绍任务委派的适用场景，按我的需要选择启用。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 查看 DSH 与可导入模型：hiroute quickstart · compute scan · compute connection prepare
- 发布计划并接入正确入口：routing preview / apply · agents connect preview / apply
- 检查执行器与实际模型：worker executors · worker result · sessions timeline
界面和 CLI 共用配置和记录；区分 Web 与 Desktop 目标，以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

打开“模型 → 通用模型 → 添加模型 → **扫描本机**”，查找 DSH Web/CLI 中已有的静态 provider。选择包含明确模型声明的接入，检查后保存要用的模型。

还没配置模型时，在“新建 API 接入”选择 DeepSeek 或自定义 API，填写地址与 Key，检查并保存。你也可以把已接入的订阅模型放进同一条路由。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“DSH 分析”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

在“Agent”中选择实际使用的 **DeepSeek Harness Web** 或 **DeepSeek Harness Desktop**，打开“模型路由”，勾选“DSH 分析”并保存。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-dsh-zh.webp"><img src="/guide-assets/agent-guide-dsh-zh.webp" alt="DeepSeek Harness Web 的模型路由设置" width="1200" height="900" loading="lazy" /></a>
  <figcaption>为日常使用的 DSH 入口添加已发布的路由。</figcaption>
</figure>

重启所选客户端，从模型选择器选中这条路由，再试一次日志分析或代码阅读。打开 HiRoute“会话 → 路由时间线”查看实际模型。Web 和 Desktop 的路由配置分别保存。

<h2 id="task-delegation">4. 了解任务委派</h2>

在 DSH Web/CLI 中分析问题时，可以把一项边界清楚的修复交给 Codex CLI。反过来，也可以从 Codex 或 Hermes 把根因调查交给 DSH，再让主 Agent 结合结果继续工作。

想使用时，在路由的“更多设置 → 任务委派”中绑定执行 Agent，并在发起任务的主 Agent 上开启“任务路由”。执行方会按照这份计划选择模型，结果可在“Agent → 任务记录”查看。[继续阅读 Agent 智能路由教程](/zh/docs/task-routing/)
