# Hermes Agent 模型路由：复用 profile，让 Codex 执行编码

继续在 Hermes 聊天和协调工作。接入当前 profile 的可用模型，简单请求用常规组，需要深入推理的对话用主力；需要实现一项独立的代码改动时，再交给 Codex CLI。

[下载 HiRoute](/zh/download/) · [查看其他 Agent 教程](/zh/docs/agent-integrations/)

<aside class="agent-guide" id="agent-cli" aria-label="给 Agent 的操作指引">
<div class="agent-guide-heading"><strong>让 Agent 参考本页执行</strong></div>
<details class="agent-guide-preview">
<summary>查看给 Agent 的内容</summary>

```text
阅读 https://hiroute.ai/zh/docs/hermes-model-routing/
请帮我按本页完成获取模型、配置智能路由和接入 Agent，再用本页的轻量 Pi 场景说明如何用更少的主力模型 token 完成独立任务。按我的需要启用委派。CLI 参考：https://hiroute.ai/zh/docs/cli/
使用产品内置管理技能：https://hiroute.ai/zh/docs/#management-skill
需要委派时使用内置委派技能：https://hiroute.ai/zh/docs/task-routing/#delegation-skill
- 确认 profile 和可用接入：hiroute quickstart --hermes-root PATH · compute scan · compute connection prepare
- 发布路由并设置 Hermes 主模型：routing preview / apply · agents connect preview / apply
- 需要任务委派时，配置执行方并核对：worker executors · worker plans · worker result
界面和 CLI 共用配置和记录。先确认所选配置目录，参数以 --help 和 hiroute schema show --command-id COMMAND_ID --include-schema 为准。
```

</details>
</aside>

[1. 扫描或登录，获取模型](#get-models) · [2. 配置智能路由](#configure-routing) · [3. 接入 Agent](#connect-agent) · [4. 了解任务委派](#task-delegation)

<h2 id="get-models">1. 扫描或登录，获取模型</h2>

先在“Agent → Hermes”确认日常使用的 profile 和配置目录。打开“模型 → 通用模型 → 添加模型 → **扫描本机**”，需要时指定同一个 Hermes 配置目录，检查并保存发现的 API 模型。

要补充模型时，从“新建 API 接入”填写服务地址与 Key，检查并保存，也可以选择已有的订阅或本地模型。

<h2 id="configure-routing">2. 配置智能路由</h2>

1. 打开“**智能路由 → 新建智能路由**”，命名为“Hermes 日常对话”，选择 **“按难度分流”**。
2. 在“**常规模型**”组点击“添加模型”，选择第一步接入的快速或经济模型，处理读文件、找定义、整理资料等常规工作。
3. 在“**主力模型**”组添加适合复杂分析、方案判断或多文件修改的模型。两组可以使用不同订阅、API 或本地模型，按各自账号可用的模型组合。
4. “判断方式”先选 **“启发式规则”**，然后点击 **“启用”**。发布后，这条路由就能在下一步的 Agent 接入设置里选到。

<figure class="guide-shot-panel">
  <a href="/guide-assets/agent-guide-difficulty-zh.webp"><img src="/guide-assets/agent-guide-difficulty-zh.webp" alt="HiRoute 按难度分流配置中的判断方式、常规模型组和主力模型组" width="1200" height="900" loading="lazy" /></a>
  <figcaption>常规组负责日常步骤，主力组处理复杂轮次。图中的模型可替换为你第一步接入的模型。</figcaption>
</figure>

想根据执行表现进一步调整时，可在同一位置选择决策模型，使用阶段评分和低分保护。[了解难度路由的判断方式](/zh/docs/model-routing/#difficulty-routing)

<h2 id="connect-agent">3. 接入 Agent</h2>

打开“Agent → Hermes → 模型路由”，确认配置目录，把“Hermes 日常对话”设为该 profile 的默认主模型，选择协议并保存。

<figure class="guide-shot-dialog">
  <a href="/guide-assets/agent-guide-hermes-zh.webp"><img src="/guide-assets/agent-guide-hermes-zh.webp" alt="Hermes profile 的配置目录、默认路由与协议" width="1200" height="900" loading="lazy" /></a>
  <figcaption>确认要使用的 Hermes profile，再把已发布路由设为默认主模型。</figcaption>
</figure>

按页面提示重新加载 Hermes，再开一个新会话。试着让它整理一段资料，随后到 HiRoute“会话 → 路由时间线”查看实际模型。

<h2 id="task-delegation">4. 了解任务委派</h2>

让 Hermes 整理项目周报时，可以把几份发布记录和工作笔记交给轻量的 Pi，先提取变更、待办和对应来源。Pi 用经济模型完成逐份阅读，Hermes 只需读简报，再判断哪些变化值得写进周报。

创建“Pi 周报素材”固定模型路由，在“更多设置 → 任务委派”中开启委派并选 Pi，填写用途并启用。回到当前 Hermes profile 的“任务路由”开启委派。

> 使用 HiRoute，把这几份笔记交给“Pi 周报素材”。按变更、待办和出处汇总，每类最多三项。

取回简报后让 Hermes 完成汇总。遇到要实现的独立改动，可以改选绑定 Codex CLI 的计划。[配置 Pi 轻量委派](/zh/docs/task-routing/#lightweight-pi)
