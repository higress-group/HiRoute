# OpenCode 多模型路由：复用 API 配置，在 /models 中选用

已有 OpenCode provider？把支持的接入复用到 HiRoute，让快速模型读代码、强模型做复杂分析。发布后在 OpenCode 的 `/models` 选用，原有模型仍在。需要第二种执行工具时，可以把独立检查交给 Pi。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/opencode-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再介绍任务委派的适用场景，按我的需要选择启用。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 发现和保存 API 来源：hiroute compute scan · compute connection prepare / preview / apply
- 创建路由并加入 OpenCode：routing preview / apply · agents connect preview / apply
- 核对模型与另一执行方：sessions timeline · worker plans · worker result
界面和 CLI 共用配置和记录。先读取现状，参数按 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 确认。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

打开“模型 → 通用模型 → 添加模型 → **扫描本机**”，选择发现的 OpenCode 静态 provider、模型与 API Key 来源，检查并保存。

OpenCode 的 `/models` 也可能显示动态发现或 OAuth 插件提供的模型。如果扫描未找到来源，且该服务提供 API 地址与 Key，可从“新建 API 接入”填写并检查。你也可以在“连接订阅”导入 Codex 或 Claude Code 模型，或复用已接入的模型。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“OpenCode 代码审查”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

打开“Agent → OpenCode → 模型路由”，勾选“OpenCode 代码审查”，按提示选择兼容协议并保存。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-opencode-zh.webp"><img src="/guide-assets/agent-guide-opencode-zh.webp" alt="OpenCode 中新增 HiRoute 路由的配置" width="1200" height="900" loading="lazy" /></a>
  <figcaption>路由会加入 OpenCode 的模型列表，在 /models 中按需选用。</figcaption>
</figure>

重启 OpenCode，输入 `/models` 选中新增路由。先让它总结一个小改动，再看 HiRoute“会话 → 路由时间线”中的模型选择。

<h2 id="task-delegation">4. 了解任务委派</h2>

准备提交代码时，可以让 Pi 独立找测试缺口，同时让 OpenCode 继续修复。需要 OpenCode 的执行工具时，也可以让其他主 Agent 把一项任务交给它。

想使用时，在路由的“更多设置 → 任务委派”中绑定执行 Agent，并在发起任务的主 Agent 上开启“任务路由”。执行方会按照这份计划选择模型，结果可在“Agent → 任务记录”查看。[继续阅读 Agent 智能路由教程](/zh/docs/task-routing/)
